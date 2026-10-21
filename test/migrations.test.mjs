// The chat database's migrations (worker/schema.ts), run in Node's SQLite
// against databases left at every older version, with data in them. No
// server needed.
//   node --experimental-strip-types --no-warnings test/migrations.test.mjs
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { MIGRATIONS, eraseDeletedPosts, migrate } from '../worker/schema.ts';

/** Just enough of Cloudflare's SqlStorage (ctx.storage.sql) over node:sqlite. */
function sqlStorage() {
  const db = new DatabaseSync(':memory:');
  const cursor = (rows) => ({
    toArray: () => rows,
    one: () => {
      if (rows.length !== 1) throw new Error(`expected exactly one row, got ${rows.length}`);
      return rows[0];
    },
    [Symbol.iterator]: () => rows[Symbol.iterator](),
  });
  return {
    exec(query, ...bindings) {
      // Several statements in one string (the migrations do that): run them all.
      if (!bindings.length && /;\s*\S/.test(query.trim())) {
        db.exec(query);
        return cursor([]);
      }
      return cursor(db.prepare(query).all(...bindings));
    },
  };
}
const LATEST = MIGRATIONS.length;
const atVersion = (v) => {
  const sql = sqlStorage();
  migrate(sql, v);
  return sql;
};
const schemaOf = (sql) => sql.exec("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name").toArray().map((r) => ({ ...r }));
const version = (sql) => Number(sql.exec("SELECT value FROM meta WHERE key = 'schema'").one().value);
const rows = (sql, query, ...args) => sql.exec(query, ...args).toArray().map((r) => ({ ...r }));

const cases = [];
const t = (name, fn) => cases.push([name, fn]);

t('a brand-new database gets every migration and its seeds', () => {
  const sql = atVersion(LATEST);
  assert.equal(version(sql), LATEST);
  assert.deepEqual(rows(sql, 'SELECT id, kind FROM channels ORDER BY kind, position').map((c) => `${c.id}:${c.kind}`), ['general:text', 'lounge:voice', 'gaming:voice']);
  assert.deepEqual(rows(sql, 'SELECT id, name, url, view FROM links'), [{ id: 'board', name: 'Board', url: '', view: 'board' }], 'the board link, with no address of its own');
  migrate(sql);
  assert.equal(rows(sql, 'SELECT * FROM links').length, 1, 'migrating again does nothing');
});

t('upgrading from every older version ends at exactly the schema a new database gets', () => {
  const fresh = schemaOf(atVersion(LATEST));
  for (let v = 0; v < LATEST; v++) {
    const sql = atVersion(v);
    migrate(sql);
    assert.equal(version(sql), LATEST, `from v${v}`);
    assert.deepEqual(schemaOf(sql), fresh, `from v${v}`);
  }
});

t('a database from the first version: its people become members, every message and channel survives', () => {
  const sql = atVersion(1);
  sql.exec("INSERT INTO users (id, name, color, created_at, last_seen) VALUES ('u1', 'Ada', '#5b6cff', 1, 2)");
  sql.exec("INSERT INTO channels (id, name, kind, position, created_at) VALUES ('c1', 'old stuff', 'text', 1, 1)");
  for (let i = 0; i < 5; i++) sql.exec('INSERT INTO messages (id, channel_id, user_id, body, ts) VALUES (?, ?, ?, ?, ?)', `m${i}`, i % 2 ? 'c1' : 'general', 'u1', `hello ${i}`, 100 + i);
  migrate(sql);
  assert.deepEqual(rows(sql, 'SELECT id, name, role, ntfy_mode, pin_hash FROM users'), [{ id: 'u1', name: 'Ada', role: 'member', ntfy_mode: 'off', pin_hash: null }]);
  assert.deepEqual(rows(sql, 'SELECT id, body, gif, image, extra, reply_to, deleted_at FROM messages ORDER BY ts').map((m) => [m.id, m.body, m.gif, m.image, m.extra, m.reply_to, m.deleted_at]),
    [0, 1, 2, 3, 4].map((i) => [`m${i}`, `hello ${i}`, null, null, null, null, null]));
  assert.deepEqual(rows(sql, "SELECT id FROM channels WHERE archived_at IS NULL ORDER BY id").map((c) => c.id), ['c1', 'gaming', 'general', 'lounge']);
  assert.equal(rows(sql, "SELECT * FROM links WHERE view = 'board'").length, 1, 'the board link was seeded on the way');
});

t('posts deleted before v15 are erased by it (text, GIF, picture, edits, reactions); live ones are untouched', () => {
  const sql = atVersion(14);
  const pic = (id) => JSON.stringify({ id, width: 10, height: 10 });
  const gif = JSON.stringify({ provider: 'klipy', slug: 'x', title: 'x', width: 1, height: 1, mp4: 'https://static.klipy.com/x.mp4' });
  sql.exec('INSERT INTO messages (id, channel_id, user_id, body, ts, gif, image, deleted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)', 'gone', 'general', 'u1', 'secret plans', 1, gif, pic('a'.repeat(32)), 5);
  sql.exec('INSERT INTO messages (id, channel_id, user_id, body, ts, gif, image) VALUES (?, ?, ?, ?, ?, ?, ?)', 'kept', 'general', 'u1', 'hi all', 2, null, pic('b'.repeat(32)));
  for (const id of ['gone', 'kept']) {
    sql.exec("INSERT INTO message_edits (message_id, body, edited_at) VALUES (?, 'older words', 3)", id);
    sql.exec("INSERT INTO reactions (message_id, user_id, emoji, ts) VALUES (?, 'u2', '👍', 4)", id);
  }
  migrate(sql);
  const [gone, kept] = rows(sql, 'SELECT id, body, gif, image, deleted_at FROM messages ORDER BY ts');
  assert.deepEqual(gone, { id: 'gone', body: '', gif: null, image: null, deleted_at: 5 }, 'a tombstone is all that stays');
  assert.deepEqual(kept, { id: 'kept', body: 'hi all', gif: null, image: pic('b'.repeat(32)), deleted_at: null });
  assert.deepEqual(rows(sql, 'SELECT message_id FROM message_edits').map((r) => r.message_id), ['kept']);
  assert.deepEqual(rows(sql, 'SELECT message_id FROM reactions').map((r) => r.message_id), ['kept']);
  assert.deepEqual(rows(sql, 'SELECT id FROM media_trash').map((r) => r.id), ['a'.repeat(32)], "the deleted post's picture goes to the trash (emptyTrash erases it)");
});

t('a restore of a backup from before v15 erases its deleted posts too (eraseDeletedPosts)', () => {
  const sql = atVersion(LATEST);
  // As importBackup inserts them: rows exactly as an old backup had them.
  const doodle = 'd'.repeat(32);
  sql.exec('INSERT INTO messages (id, channel_id, user_id, body, ts, image, deleted_at, pinned_at, pinned_by, extra) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
    'gone', 'general', 'u1', 'secret plans', 1, JSON.stringify({ id: 'a'.repeat(32), width: 1, height: 1 }), 5, 3, 'u1', JSON.stringify({ poll: { options: ['x', 'y'] } }));
  sql.exec("INSERT INTO messages (id, channel_id, user_id, body, ts, pinned_at, pinned_by) VALUES ('kept', 'general', 'u1', 'hi all', 2, 4, 'u1')");
  for (const id of ['gone', 'kept']) {
    sql.exec("INSERT INTO message_edits (message_id, body, edited_at) VALUES (?, 'older words', 3)", id);
    sql.exec("INSERT INTO reactions (message_id, user_id, emoji, ts) VALUES (?, 'u2', '👍', 4)", id);
    sql.exec('INSERT INTO reactions (message_id, user_id, emoji, ts) VALUES (?, ?, ?, 5)', id, 'u3', `<d:${id === 'gone' ? doodle : 'e'.repeat(32)}>`);
    sql.exec("INSERT INTO poll_votes (message_id, user_id, option, ts) VALUES (?, 'u2', 0, 4)", id);
  }
  eraseDeletedPosts(sql);
  eraseDeletedPosts(sql); // idempotent: runs after every page
  assert.deepEqual(rows(sql, 'SELECT id, body, gif, image, pinned_at, pinned_by, extra FROM messages ORDER BY ts'), [
    { id: 'gone', body: '', gif: null, image: null, pinned_at: null, pinned_by: null, extra: null },
    { id: 'kept', body: 'hi all', gif: null, image: null, pinned_at: 4, pinned_by: 'u1', extra: null },
  ]);
  for (const table of ['message_edits', 'reactions', 'poll_votes']) {
    assert.deepEqual([...new Set(rows(sql, `SELECT message_id FROM ${table}`).map((r) => r.message_id))], ['kept'], table);
  }
  assert.deepEqual(rows(sql, 'SELECT id FROM media_trash ORDER BY id').map((r) => r.id), ['a'.repeat(32), doodle], 'its picture and doodle are trashed');
});

t('the board link is seeded once, by v10: upgrading from v10 or later never adds another', () => {
  for (const v of [9, 10, 12, 17]) {
    const sql = atVersion(v);
    if (v >= 10) sql.exec("UPDATE links SET name = 'Our board' WHERE id = 'board'"); // renamed since
    migrate(sql);
    assert.deepEqual(rows(sql, 'SELECT id, name FROM links'), [{ id: 'board', name: v >= 10 ? 'Our board' : 'Board' }], `from v${v}`);
  }
});

t('the busiest message reads walk an index instead of every row (rows read are a free-plan budget)', () => {
  const sql = atVersion(LATEST);
  const plan = (query, ...args) => rows(sql, `EXPLAIN QUERY PLAN ${query}`, ...args).map((r) => r.detail).join(' | ');
  // The same queries as Hub.nextTs, hub-backup.ts › exportPage and hub-messages.ts › onHistory, answerRepeat.
  const reads = {
    'the newest message': plan('SELECT MAX(ts) AS ts FROM messages'),
    "a backup's page": plan('SELECT id FROM messages WHERE ts > ? ORDER BY ts LIMIT ?', 0, 5000),
    'a history page far back': plan(`SELECT id FROM messages WHERE channel_id = ? AND deleted_at IS NULL AND (ts, id) < (?, ?)
      ORDER BY ts DESC, id DESC LIMIT ?`, 'general', 5, 'm5', 51),
    'a send that came again': plan('SELECT id FROM messages WHERE user_id = ? AND nonce = ? AND channel_id = ? LIMIT 20', 'u1', 'n1', 'general'),
  };
  assert.match(reads['the newest message'], /USING COVERING INDEX messages_by_ts/);
  assert.match(reads["a backup's page"], /USING INDEX messages_by_ts \(ts>\?\)/);
  assert.match(reads['a history page far back'], /USING INDEX messages_by_channel \(channel_id=\? AND \(ts,id\)<\(\?,\?\)\)/);
  assert.match(reads['a send that came again'], /USING INDEX messages_by_nonce \(user_id=\? AND nonce=\?\)/);
  for (const [what, detail] of Object.entries(reads)) assert.doesNotMatch(detail, /SCAN messages|TEMP B-TREE/, what);
});

let failed = 0;
for (const [name, fn] of cases) {
  try {
    fn();
    console.log(`✓ ${name}`);
  } catch (err) {
    failed++;
    console.log(`✗ ${name}\n    ${err.message}`);
  }
}
console.log(`${cases.length - failed}/${cases.length} passed`);
process.exit(failed ? 1 : 0);
