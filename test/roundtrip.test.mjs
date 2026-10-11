// Backup → restore round trip, with the real tools (tools/backup.mjs,
// tools/restore.mjs). npm run test:all runs both halves:
//   seed:   against a server the other suites filled: one more account and
//           message, then a backup into <dir>
//   verify: against a brand-new, EMPTY server: restore <dir>'s backup, back
//           the restored server up again, and check both backups hold exactly
//           the same rows and media; then the account signs in with its password.
//   YAP_URL=… YAP_KEY=testkey YAP_ADMIN_KEY=testadmin node test/roundtrip.test.mjs seed|verify <dir>
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { referencedMedia } from '../tools/lib.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const BASE = process.env.YAP_URL ?? 'http://localhost:8787';
const KEY = process.env.YAP_KEY ?? '';
const ADMIN = process.env.YAP_ADMIN_KEY ?? '';
const [mode, dir] = process.argv.slice(2);
if (!['seed', 'verify'].includes(mode) || !dir || !ADMIN) {
  console.error('usage: YAP_URL=… YAP_KEY=… YAP_ADMIN_KEY=… node test/roundtrip.test.mjs seed|verify <dir>');
  process.exit(2);
}
const admin = (p, init = {}) => fetch(`${BASE}${p}`, { ...init, headers: { ...init.headers, Authorization: `Bearer ${ADMIN}` } });
const rid = () => Math.random().toString(36).slice(2, 8);
// The made-up password every suite signs up with.
const PASSWORD = 'correct-horse-582039';

/** One WebSocket client: send, and wait for the first message matching a test. */
async function connect() {
  const ws = new WebSocket(`${BASE.replace(/^http/, 'ws')}/ws`, ['yap', `key.${KEY}`]);
  const inbox = [];
  const waiters = [];
  ws.addEventListener('message', (e) => {
    if (e.data === 'pong') return;
    const msg = JSON.parse(e.data);
    const i = waiters.findIndex((w) => w.pred(msg));
    if (i >= 0) waiters.splice(i, 1)[0].resolve(msg);
    else inbox.push(msg);
  });
  await new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve, { once: true });
    ws.addEventListener('error', () => reject(new Error('socket failed')), { once: true });
  });
  return {
    send: (m) => ws.send(JSON.stringify(m)),
    next: (pred, ms = 5000) => {
      const i = inbox.findIndex(pred);
      if (i >= 0) return Promise.resolve(inbox.splice(i, 1)[0]);
      return new Promise((resolve, reject) => {
        const w = { pred, resolve };
        waiters.push(w);
        setTimeout(() => {
          if (waiters.includes(w)) {
            waiters.splice(waiters.indexOf(w), 1);
            reject(new Error(`timed out (inbox: ${inbox.map((m) => m.t).join(', ')})`));
          }
        }, ms);
      });
    },
    close: () => ws.close(),
  };
}

/** Run one of tools/*.mjs against this server; throws with its output if it fails. */
function tool(name, args, env) {
  try {
    return execFileSync(process.execPath, [path.join(ROOT, 'tools', name), ...args], {
      encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, YAP_URL: BASE, YAP_ADMIN_KEY: ADMIN, ...env },
    });
  } catch (err) {
    throw new Error(`${name} failed:\n${err.stdout ?? ''}${err.stderr ?? ''}`);
  }
}

const snapshotIn = (d) => {
  const file = fs.readdirSync(d).filter((f) => /^yap-\d{4}-\d{2}-\d{2}\.json\.gz$/.test(f)).sort().at(-1);
  if (!file) throw new Error(`no backup in ${d}`);
  return path.join(d, file);
};
const readSnapshot = (file) => JSON.parse(zlib.gunzipSync(fs.readFileSync(file)));
/** media/<id>.<ext> → { id: bytes } */
const mediaIn = (d) => Object.fromEntries(
  (fs.existsSync(path.join(d, 'media')) ? fs.readdirSync(path.join(d, 'media')) : []).map((f) => [f.split('.')[0], fs.readFileSync(path.join(d, 'media', f))]),
);

const results = [];
async function test(name, fn) {
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(['✗', name, err]);
  }
}

if (mode === 'seed') {
  await test('seed: a member with a password and a message of their own, then a full backup', async () => {
    const name = `Roundtrip ${rid()}`;
    const c = await connect();
    c.send({ t: 'hello', v: 1, client: 'test', signup: { name, pin: PASSWORD, color: '#336699' } });
    const first = await c.next((m) => m.t === 'pending' || m.t === 'ready' || m.t === 'error');
    assert.notEqual(first.t, 'error', first.message);
    if (first.t === 'pending') {
      await admin(`/api/admin/role?uid=${first.uid}&role=member`, { method: 'POST' });
      await c.next((m) => m.t === 'ready');
    }
    const body = `round trip ${rid()} ✓`;
    c.send({ t: 'send', channelId: 'general', body, nonce: 'rt' });
    const message = (await c.next((m) => m.t === 'message' && m.nonce === 'rt')).message;
    c.close();
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, 'account.json'), JSON.stringify({ name, pin: PASSWORD, uid: first.uid, message }));
    tool('backup.mjs', [], { YAP_BACKUP_DIR: dir });
    const backup = readSnapshot(snapshotIn(dir));
    assert.equal(backup.format, 'yap-backup');
    assert.ok(backup.messages.some((m) => m.id === message.id && m.body === body), 'the new message is in it');
    const listed = await (await admin('/api/admin/media')).json();
    assert.deepEqual(Object.keys(mediaIn(dir)).sort(), listed.map((b) => b.id).sort(), 'every stored blob was downloaded');
  });
}

if (mode === 'verify') {
  const account = JSON.parse(fs.readFileSync(path.join(dir, 'account.json'), 'utf8'));
  const after = path.join(dir, 'after');

  await test('restore into an empty server, back it up again: the same rows, the same media; an old backup’s deleted post stays erased', async () => {
    assert.equal((await admin('/api/admin/import', { method: 'POST', body: JSON.stringify({ phase: 'check' }) })).status, 200, 'the server starts out empty');
    const file = snapshotIn(dir);
    const before = readSnapshot(file);
    // The same backup, dressed up as one from before deleted posts were erased
    // (schema 14): one deleted post still has its words, picture, poll, pin,
    // an earlier version, a reaction and a vote. None of it may come back.
    const old = {
      id: `old-${rid()}`, channelId: 'general', uid: account.uid, body: 'a secret from long ago', ts: 1,
      image: { id: 'c0ffee'.padEnd(32, '0'), width: 4, height: 4 }, pinnedAt: 3, deletedAt: 2, pinnedBy: account.uid,
      extra: JSON.stringify({ poll: { options: ['yes', 'no'] } }),
    };
    const legacy = {
      ...before, schema: 14, messages: [old, ...before.messages],
      edits: [...before.edits, { message_id: old.id, body: 'an older secret', edited_at: 2 }],
      reactions: [...before.reactions, { message_id: old.id, user_id: account.uid, emoji: '👍', ts: 3 }],
      pollVotes: [...before.pollVotes, { message_id: old.id, user_id: account.uid, option: 0, ts: 3 }],
    };
    const legacyFile = path.join(dir, 'legacy', path.basename(file));
    fs.mkdirSync(path.dirname(legacyFile), { recursive: true });
    fs.writeFileSync(legacyFile, zlib.gzipSync(JSON.stringify(legacy)));
    tool('restore.mjs', [legacyFile, '--to', BASE, '--media', path.join(dir, 'media')]);
    tool('backup.mjs', [], { YAP_BACKUP_DIR: after });
    const again = readSnapshot(snapshotIn(after));
    assert.ok(!/secret from long ago|an older secret/.test(JSON.stringify(again)), 'no trace of the deleted post');
    // Everything the export holds must come back exactly (the old post as a bare tombstone); only the envelope differs.
    const expected = { ...before, messages: [{ id: old.id, channelId: 'general', uid: account.uid, body: '', ts: 1, deletedAt: 2 }, ...before.messages] };
    const ENVELOPE = new Set(['format', 'version', 'server', 'exportedAt', 'stats']);
    for (const key of new Set([...Object.keys(expected), ...Object.keys(again)])) {
      if (ENVELOPE.has(key)) continue;
      const [x, y] = [expected[key], again[key]];
      if (Array.isArray(x) && Array.isArray(y)) {
        const i = x.findIndex((row, n) => JSON.stringify(row) !== JSON.stringify(y[n]));
        assert.ok(i < 0 && x.length === y.length, `${key}: ${x.length} → ${y.length} rows${i >= 0 ? `; first difference at ${i}:\n  ${JSON.stringify(x[i])}\n  ${JSON.stringify(y[i])}` : ''}`);
      } else {
        assert.deepEqual(y, x, key);
      }
    }
    const rows = Object.values(before).filter(Array.isArray).reduce((n, list) => n + list.length, 0);
    console.log(`    (${rows} rows, ${before.messages.length} of them messages; ${Object.keys(mediaIn(dir)).length} blobs)`);
    // The files: exactly the ones the backup refers to, plus sealed pictures (.bin), byte for byte.
    const [mediaBefore, mediaAfter] = [mediaIn(dir), mediaIn(after)];
    const needed = referencedMedia(before);
    const sealed = new Set(fs.readdirSync(path.join(dir, 'media')).filter((f) => f.endsWith('.bin')).map((f) => f.split('.')[0]));
    const kept = Object.keys(mediaBefore).filter((id) => needed.has(id) || sealed.has(id)).sort();
    assert.ok([...needed].every((id) => id in mediaBefore), 'the backup has every file it refers to');
    assert.deepEqual(Object.keys(mediaAfter).sort(), kept, 'the files the backup uses come back, and only those');
    for (const id of kept) assert.ok(mediaBefore[id].equals(mediaAfter[id]), `file ${id} is byte for byte the same`);
    console.log(`    (${kept.length} of ${Object.keys(mediaBefore).length} files restored; the rest nothing uses)`);
  });

  await test('the restored account signs in with its name and password and finds its message', async () => {
    const c = await connect();
    c.send({ t: 'hello', v: 1, client: 'test', login: { name: account.name, pin: account.pin } });
    const ready = await c.next((m) => m.t === 'ready' || m.t === 'error');
    assert.equal(ready.t, 'ready', ready.message);
    assert.equal(ready.uid, account.uid);
    c.send({ t: 'history', channelId: 'general', limit: 20 });
    const history = await c.next((m) => m.t === 'history');
    assert.deepEqual(history.messages.find((m) => m.id === account.message.id)?.body, account.message.body);
    c.close();
  });
}

for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.stack ?? err}`);
}
const failed = results.filter((r) => r[0] === '✗').length;
console.log(`\n${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
