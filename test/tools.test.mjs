// tools/backup.mjs and tools/restore.mjs against a fake Yap admin API (no
// server needed): a file erased mid-backup, keeping only the files someone
// still needs, and restoring only the files a backup uses.
//   node test/tools.test.mjs
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'yap-tools-'));
const idOf = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex').slice(0, 32);
const PNG = Buffer.from([0x89, 0x50, 0x4e, 0x47]);
/** A stored file: content-addressed like the real thing. */
const file = (label, mime = 'image/png') => {
  const bytes = Buffer.concat([mime === 'image/png' ? PNG : Buffer.alloc(0), Buffer.from(label)]);
  return { id: idOf(bytes), mime, bytes };
};

// ── A fake Yap admin API ──────────────────────────────────────────────────
const server = {
  tables: {}, // emojis, users, reactions, drawings, sounds…
  messages: [],
  blobs: new Map(), // id → file
  erasedMidway: new Set(), // listed, but gone by the time it's fetched
  uploads: [],
  phases: [],
};
const fake = http.createServer((req, res) => {
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => {
    const body = Buffer.concat(chunks);
    const url = new URL(req.url, 'http://x');
    const json = (value, status = 200) => res.writeHead(status, { 'Content-Type': 'application/json' }).end(JSON.stringify(value));
    if (req.headers.authorization !== 'Bearer test-admin') return json({ ok: false }, 401);
    if (url.pathname === '/api/admin/export') return json({ schema: 18, channels: [], ...server.tables, messages: url.searchParams.get('after') === '0' ? server.messages : [], next: null });
    if (url.pathname === '/api/admin/stats') return json({ hub: { databaseBytes: 1, messages: server.messages.length }, media: { databaseBytes: 1 } });
    if (url.pathname === '/api/admin/media') return json([...server.blobs.values()].map((b) => ({ id: b.id, mime: b.mime, size: b.bytes.length })));
    const one = url.pathname.match(/^\/api\/admin\/media\/([a-f0-9]{32})$/);
    if (one) {
      const b = server.blobs.get(one[1]);
      if (!b || server.erasedMidway.has(b.id)) return res.writeHead(404).end('Gone.');
      return res.writeHead(200, { 'Content-Type': b.mime }).end(b.bytes);
    }
    if (url.pathname === '/api/admin/media-put') {
      server.uploads.push(idOf(body));
      return json({ ok: true, id: idOf(body) });
    }
    if (url.pathname === '/api/admin/import') {
      const part = JSON.parse(body);
      server.phases.push(part.phase);
      return json({ ok: true, count: part.messages?.length ?? 0 });
    }
    return json({ ok: false }, 404);
  });
});
await new Promise((resolve) => fake.listen(0, '127.0.0.1', resolve));
const BASE = `http://127.0.0.1:${fake.address().port}`;
/** Run a tool against the fake server; returns its output (throws if it fails). */
const tool = async (name, args = [], env = {}) => {
  const { stdout, stderr } = await promisify(execFile)(process.execPath, [path.join(ROOT, 'tools', name), ...args], {
    env: { ...process.env, YAP_URL: BASE, YAP_ADMIN_KEY: 'test-admin', ...env },
  });
  return stdout + stderr;
};
const backupDir = path.join(tmp, 'backups');
const saved = () => fs.readdirSync(path.join(backupDir, 'media')).map((f) => f.split('.')[0]).sort();
const ids = (...files) => files.map((f) => f.id).sort();

const results = [];
async function test(name, fn) {
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(['✗', name, err]);
  }
}

const emoji = file('party');
const picture = file('a picture in a post');
const sealed = file('sealed bytes', 'application/octet-stream');
const orphan = file('uploaded, never sent');
const ghost = file('erased while the backup ran');

await test('backup: every file the server lists; one erased in between is skipped, not a failure', async () => {
  server.tables = { emojis: [{ id: emoji.id, name: 'party' }], users: [], reactions: [], drawings: [], sounds: [] };
  server.messages = [{ id: 'm1', channelId: 'general', uid: 'u1', body: 'look', ts: 1, image: { id: picture.id, width: 1, height: 1 }, deletedAt: null }];
  for (const b of [emoji, picture, sealed, orphan, ghost]) server.blobs.set(b.id, b);
  server.erasedMidway.add(ghost.id);
  const out = await tool('backup.mjs', [], { YAP_BACKUP_DIR: backupDir });
  assert.match(out, /backup ok .*1 erased meanwhile/);
  assert.deepEqual(saved(), ids(emoji, picture, sealed, orphan));
  assert.ok(fs.existsSync(path.join(backupDir, 'media', `${sealed.id}.bin`)), 'sealed pictures are kept as .bin');
});

await test('backup: a file stays while the server or a kept snapshot needs it, and goes after', async () => {
  // An older snapshot still shows the picture (restoring it needs the file).
  fs.writeFileSync(path.join(backupDir, 'yap-2000-01-01.json.gz'), zlib.gzipSync(JSON.stringify({ format: 'yap-backup', messages: server.messages, emojis: [] })));
  // Today the post is deleted (its picture erased), and the sealed picture's owner deleted it.
  server.messages = [{ id: 'm1', channelId: 'general', uid: 'u1', body: '', ts: 1, deletedAt: 2 }];
  server.blobs.delete(picture.id);
  server.blobs.delete(sealed.id);
  server.blobs.delete(ghost.id);
  await tool('backup.mjs', [], { YAP_BACKUP_DIR: backupDir });
  assert.deepEqual(saved(), ids(emoji, picture, orphan), 'the picture stays (an older snapshot needs it), the sealed one goes');
  fs.unlinkSync(path.join(backupDir, 'yap-2000-01-01.json.gz'));
  const out = await tool('backup.mjs', [], { YAP_BACKUP_DIR: backupDir });
  assert.match(out, /1 no longer needed removed/);
  assert.deepEqual(saved(), ids(emoji, orphan), 'nothing needs the picture any more');
});

await test('restore: the files the backup uses, and sealed pictures; nothing else', async () => {
  const dir = path.join(tmp, 'restore');
  fs.mkdirSync(path.join(dir, 'media'), { recursive: true });
  const used = { emoji: file('e'), avatar: file('a'), postPicture: file('p'), doodle: file('d'), drawing: file('w'), clip: file('RIFF clip', 'audio/wav') };
  const deletedPicture = file('a deleted post’s picture');
  const unused = file('nobody uses this');
  const sealedNote = file('encrypted', 'application/octet-stream');
  const ext = { 'image/png': 'png', 'audio/wav': 'wav', 'application/octet-stream': 'bin' };
  for (const b of [...Object.values(used), deletedPicture, unused, sealedNote]) fs.writeFileSync(path.join(dir, 'media', `${b.id}.${ext[b.mime]}`), b.bytes);
  const backup = {
    format: 'yap-backup', schema: 18,
    emojis: [{ id: used.emoji.id, name: 'e', deleted_at: 5 }], // deleted emojis too: old messages still show them
    users: [{ id: 'u1', avatar: used.avatar.id }],
    messages: [
      { id: 'live', body: 'hi', ts: 1, image: { id: used.postPicture.id, width: 1, height: 1 }, deletedAt: null },
      { id: 'gone', body: '', ts: 2, image: { id: deletedPicture.id, width: 1, height: 1 }, deletedAt: 3 },
    ],
    reactions: [{ message_id: 'live', user_id: 'u2', emoji: `<d:${used.doodle.id}>` }, { message_id: 'gone', user_id: 'u2', emoji: `<d:${unused.id}>` }],
    drawings: [{ id: 'dr', media_id: used.drawing.id }],
    sounds: [{ id: 's', media_id: used.clip.id }],
  };
  const backupFile = path.join(dir, 'yap-2026-01-01.json.gz');
  fs.writeFileSync(backupFile, zlib.gzipSync(JSON.stringify(backup)));
  server.uploads = [];
  server.phases = [];
  const out = await tool('restore.mjs', [backupFile, '--to', BASE]);
  assert.deepEqual(server.uploads.sort(), ids(...Object.values(used), sealedNote));
  assert.match(out, /files: 7 \(2 this backup doesn't use left out\)/);
  assert.deepEqual(server.phases, ['check', 'begin', 'messages', 'finish']);
});

fake.close();
fs.rmSync(tmp, { recursive: true, force: true });
for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.stack ?? err}`);
}
const failed = results.filter((r) => r[0] === '✗').length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
