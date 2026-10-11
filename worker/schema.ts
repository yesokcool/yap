// The chat database's schema (the Hub's SQLite): every migration, in order.
// Kept apart from hub.ts so Node can run them for test/migrations.test.mjs
// (hence the `.ts` on imports). CLAUDE.md › Guardrails: append-only.
import type { ChannelKind } from './protocol.ts';

// Append-only. Never edit a shipped migration; add a new one.
export const MIGRATIONS: ((sql: SqlStorage) => void)[] = [
  (sql) => {
    sql.exec(`
      CREATE TABLE channels (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        kind TEXT NOT NULL,
        position INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        archived_at INTEGER
      );
      CREATE TABLE users (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        color TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        last_seen INTEGER NOT NULL
      );
      CREATE TABLE messages (
        id TEXT PRIMARY KEY,
        channel_id TEXT NOT NULL,
        user_id TEXT NOT NULL,
        body TEXT NOT NULL,
        ts INTEGER NOT NULL,
        deleted_at INTEGER
      );
      CREATE INDEX messages_by_channel ON messages (channel_id, ts, id);
    `);
    const now = Date.now();
    const seed: [string, string, ChannelKind, number][] = [
      ['general', 'general', 'text', 0],
      ['lounge', 'Lounge', 'voice', 0],
      ['gaming', 'Gaming', 'voice', 1],
    ];
    for (const [id, name, kind, position] of seed) {
      sql.exec('INSERT INTO channels (id, name, kind, position, created_at) VALUES (?, ?, ?, ?, ?)', id, name, kind, position, now);
    }
  },
  // v2 (2026-10-05): GIF messages. JSON-encoded Gif, NULL for plain text.
  (sql) => {
    sql.exec('ALTER TABLE messages ADD COLUMN gif TEXT');
  },
  // v3 (2026-10-05): custom emojis. Image bytes live in the Media object.
  (sql) => {
    sql.exec(`
      CREATE TABLE emojis (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        mime TEXT NOT NULL,
        user_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        deleted_at INTEGER
      );
      CREATE INDEX emojis_by_name ON emojis (name);
    `);
  },
  // v4 (2026-10-05): merged accounts. `alias` (a duplicate user id) is read as
  // `uid` everywhere. Nothing is rewritten, so deleting a row un-merges.
  (sql) => {
    sql.exec(`
      CREATE TABLE user_aliases (
        alias TEXT PRIMARY KEY,
        uid TEXT NOT NULL,
        created_at INTEGER NOT NULL
      );
    `);
  },
  // v5 (2026-10-05): roles + waiting room. Everyone already here is a member.
  (sql) => {
    sql.exec("ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'member'");
  },
  // v6 (2026-10-05): real accounts — name + 6-digit PIN, per-device tokens.
  (sql) => {
    sql.exec(`
      ALTER TABLE users ADD COLUMN pin_hash TEXT;
      CREATE TABLE devices (
        token_hash TEXT PRIMARY KEY,
        user_id TEXT NOT NULL,
        client TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        last_seen INTEGER NOT NULL
      );
      CREATE INDEX devices_by_user ON devices (user_id);
      CREATE TABLE login_failures (
        name TEXT PRIMARY KEY,
        failures INTEGER NOT NULL,
        locked_until INTEGER NOT NULL
      );
    `);
  },
  // v7 (2026-10-05): avatars, phone pushes (ntfy), the shared drawing board.
  (sql) => {
    sql.exec(`
      ALTER TABLE users ADD COLUMN avatar TEXT;
      ALTER TABLE users ADD COLUMN ntfy_topic TEXT;
      ALTER TABLE users ADD COLUMN ntfy_mode TEXT NOT NULL DEFAULT 'off';
      CREATE TABLE strokes (
        id TEXT PRIMARY KEY,
        canvas INTEGER NOT NULL,
        user_id TEXT NOT NULL,
        data TEXT NOT NULL,
        ts INTEGER NOT NULL,
        deleted_at INTEGER
      );
      CREATE INDEX strokes_by_canvas ON strokes (canvas, ts);
      CREATE TABLE drawings (
        id TEXT PRIMARY KEY,
        media_id TEXT NOT NULL,
        user_id TEXT NOT NULL,
        ts INTEGER NOT NULL
      );
    `);
  },
  // v8 (2026-10-05): reactions; Space look + last position per person.
  (sql) => {
    sql.exec(`
      CREATE TABLE reactions (
        message_id TEXT NOT NULL,
        user_id TEXT NOT NULL,
        emoji TEXT NOT NULL,
        ts INTEGER NOT NULL,
        PRIMARY KEY (message_id, user_id, emoji)
      );
      ALTER TABLE users ADD COLUMN look TEXT;
      ALTER TABLE users ADD COLUMN space_x REAL;
      ALTER TABLE users ADD COLUMN space_y REAL;
      ALTER TABLE users ADD COLUMN space_d REAL;
    `);
  },
  // v9 (2026-10-05): edit your own messages. Every earlier version is kept in
  // message_edits (oldest first), so an edit never loses text.
  (sql) => {
    sql.exec(`
      ALTER TABLE messages ADD COLUMN edited_at INTEGER;
      CREATE TABLE message_edits (
        message_id TEXT NOT NULL,
        body TEXT NOT NULL,
        edited_at INTEGER NOT NULL
      );
      CREATE INDEX message_edits_by_message ON message_edits (message_id);
    `);
  },
  // v10 (2026-10-05): personal invite links, a moderation log, sidebar links
  // managed in the app, and per-channel voice quality.
  (sql) => {
    sql.exec(`
      CREATE TABLE invites (
        id TEXT PRIMARY KEY,
        key_hash TEXT NOT NULL,
        label TEXT NOT NULL,
        created_by TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        revoked_at INTEGER,
        uses INTEGER NOT NULL DEFAULT 0,
        last_used_at INTEGER
      );
      CREATE INDEX invites_by_hash ON invites (key_hash);
      ALTER TABLE users ADD COLUMN invite_id TEXT;
      CREATE TABLE mod_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts INTEGER NOT NULL,
        actor TEXT,
        action TEXT NOT NULL,
        target TEXT,
        detail TEXT
      );
      CREATE TABLE links (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        url TEXT NOT NULL,
        view TEXT,
        description TEXT,
        position INTEGER NOT NULL,
        created_by TEXT,
        created_at INTEGER NOT NULL,
        archived_at INTEGER
      );
      ALTER TABLE channels ADD COLUMN bitrate INTEGER;
    `);
    // The board's sidebar link (it only shows when the server has a board).
    // Edited after shipping, once, on 2026-10-06: it used to seed the first
    // server's own board name and address. Servers that already ran it keep
    // their row; the address now comes from links() when it's empty.
    sql.exec(
      'INSERT INTO links (id, name, url, view, description, position, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)',
      'board', 'Board', '', 'board', null, 0, Date.now(),
    );
  },
  // v11 (2026-10-05): pictures in messages. JSON ImageRef, NULL otherwise.
  (sql) => {
    sql.exec('ALTER TABLE messages ADD COLUMN image TEXT');
  },
  // v12 (2026-10-05): device token scope. NULL = full account; 'listen' = an
  // OBS link that can only listen to voice.
  (sql) => {
    sql.exec('ALTER TABLE devices ADD COLUMN scope TEXT');
  },
  // v13 (2026-10-05): browser push subscriptions (Web Push), one per browser.
  (sql) => {
    sql.exec(`
      CREATE TABLE push_subs (
        endpoint TEXT PRIMARY KEY,
        user_id TEXT NOT NULL,
        p256dh TEXT NOT NULL,
        auth TEXT NOT NULL,
        created_at INTEGER NOT NULL
      );
      CREATE INDEX push_subs_by_user ON push_subs (user_id);
    `);
  },
  // v14 (2026-10-05): encrypted Personal space — your space key, wrapped with
  // your passcode in the browser. The server can't unwrap it.
  (sql) => {
    sql.exec('ALTER TABLE users ADD COLUMN space_wrap TEXT');
  },
  // v15 (2026-10-05): replies; When's good; deleted posts are erased, not
  // hidden ("dont save deleted posts"). Posts deleted before today
  // are erased here too; their pictures go to media_trash, which emptyTrash()
  // removes from the Media store once nothing else uses them.
  (sql) => {
    sql.exec(`
      ALTER TABLE messages ADD COLUMN reply_to TEXT;
      CREATE TABLE when_slots (
        user_id TEXT PRIMARY KEY,
        slots TEXT NOT NULL,
        updated_at INTEGER NOT NULL
      );
      CREATE TABLE media_trash (id TEXT PRIMARY KEY);
      INSERT OR IGNORE INTO media_trash (id)
        SELECT json_extract(image, '$.id') FROM messages WHERE deleted_at IS NOT NULL AND image IS NOT NULL;
      DELETE FROM message_edits WHERE message_id IN (SELECT id FROM messages WHERE deleted_at IS NOT NULL);
      DELETE FROM reactions WHERE message_id IN (SELECT id FROM messages WHERE deleted_at IS NOT NULL);
      UPDATE messages SET body = '', gif = NULL, image = NULL WHERE deleted_at IS NOT NULL;
    `);
  },
  // v16 (2026-10-05): account protection — PIN or password, a weak-PIN flag
  // (learned at sign-in, when the server briefly sees it), device names.
  (sql) => {
    sql.exec(`
      ALTER TABLE users ADD COLUMN secret_kind TEXT;
      ALTER TABLE users ADD COLUMN weak_secret INTEGER;
      ALTER TABLE devices ADD COLUMN label TEXT;
    `);
  },
  // v17 (2026-10-05): pinned messages.
  (sql) => {
    sql.exec(`
      ALTER TABLE messages ADD COLUMN pinned_at INTEGER;
      ALTER TABLE messages ADD COLUMN pinned_by TEXT;
      CREATE INDEX messages_pinned ON messages (channel_id, pinned_at) WHERE pinned_at IS NOT NULL;
    `);
  },
  // v18 (2026-10-06): the extras (PROTOCOL.md › Extras) — away messages, join
  // chimes, quiet hours; polls and server-rolled dice (messages.extra, JSON);
  // the soundboard; hang reminders.
  (sql) => {
    sql.exec(`
      ALTER TABLE users ADD COLUMN away TEXT;
      ALTER TABLE users ADD COLUMN chime TEXT;
      ALTER TABLE users ADD COLUMN quiet TEXT;
      ALTER TABLE messages ADD COLUMN extra TEXT;
      CREATE TABLE poll_votes (
        message_id TEXT NOT NULL,
        user_id TEXT NOT NULL,
        option INTEGER NOT NULL,
        ts INTEGER NOT NULL,
        PRIMARY KEY (message_id, user_id)
      );
      CREATE TABLE sounds (
        id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        name TEXT NOT NULL,
        emoji TEXT,
        media_id TEXT NOT NULL,
        gain REAL NOT NULL,
        ms INTEGER NOT NULL,
        user_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        deleted_at INTEGER
      );
      CREATE TABLE hangs (
        id TEXT PRIMARY KEY,
        at INTEGER NOT NULL,
        title TEXT NOT NULL,
        channel_id TEXT,
        user_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        notified_at INTEGER,
        cancelled_at INTEGER
      );
      CREATE INDEX hangs_by_at ON hangs (at);
    `);
  },
];

/**
 * Bring the database up to the latest schema (or only up to version `to`,
 * for tests). The current version lives in meta 'schema'.
 */
export function migrate(sql: SqlStorage, to = MIGRATIONS.length) {
  runMigrations(sql, MIGRATIONS, to);
}

/** Run `migrations` from the version in meta 'schema' up to `to`. The Media object's own list runs through this too. */
export function runMigrations(sql: SqlStorage, migrations: ((sql: SqlStorage) => void)[], to = migrations.length) {
  sql.exec('CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)');
  const row = sql.exec<{ value: string }>("SELECT value FROM meta WHERE key = 'schema'").toArray()[0];
  let version = row ? Number(row.value) : 0;
  while (version < to) {
    migrations[version](sql);
    version++;
    sql.exec("INSERT INTO meta (key, value) VALUES ('schema', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", String(version));
  }
}

/**
 * A deleted post is an empty row and nothing more (v15 above; "dont save
 * deleted posts"): no text, GIF, picture, poll, pin, edits, reactions or
 * votes. A backup taken before v15 still has them, so a restore runs this
 * after importing messages. Their pictures and doodle reactions go to
 * media_trash, which the Hub's emptyTrash() erases once nothing else uses them.
 */
export function eraseDeletedPosts(sql: SqlStorage) {
  const deleted = 'SELECT id FROM messages WHERE deleted_at IS NOT NULL';
  sql.exec(`
    INSERT OR IGNORE INTO media_trash (id)
      SELECT json_extract(image, '$.id') FROM messages WHERE deleted_at IS NOT NULL AND image IS NOT NULL;
    INSERT OR IGNORE INTO media_trash (id)
      SELECT substr(emoji, 4, 32) FROM reactions WHERE emoji LIKE '<d:%' AND message_id IN (${deleted});
    DELETE FROM message_edits WHERE message_id IN (${deleted});
    DELETE FROM reactions WHERE message_id IN (${deleted});
    DELETE FROM poll_votes WHERE message_id IN (${deleted});
    UPDATE messages SET body = '', gif = NULL, image = NULL, pinned_at = NULL, pinned_by = NULL, extra = NULL
      WHERE deleted_at IS NOT NULL AND (body != '' OR gif IS NOT NULL OR image IS NOT NULL OR pinned_at IS NOT NULL OR extra IS NOT NULL);
  `);
}
