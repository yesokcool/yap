#!/usr/bin/env node
// Pull a full Yap backup: the chat database (channels, people, messages,
// deleted ones as empty tombstones, emojis, reactions, drawings, …; not
// When's good or browser push subscriptions: PROTOCOL.md › Backups) into a
// dated .json.gz, plus every stored file (emojis, pictures, avatars, drawings,
// doodles, clips, sealed pictures) into a content-addressed media folder.
// Safe to run any time; files are only downloaded once. Keeps the newest 60
// daily snapshots, and only the files the server or one of them still uses
// (a deleted post's picture goes once no kept snapshot shows it).
//
//   node tools/backup.mjs                 # server + admin key: see tools/instance.mjs
//   YAP_URL=… YAP_ADMIN_KEY=… YAP_BACKUP_DIR=… node tools/backup.mjs
//   node tools/backup.mjs --check         # only say which server and folder (install-backup.sh)
//
// Restore paths: PROTOCOL.md › Backups.
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { adminKey, serverUrl } from './instance.mjs';
import { ROOT, fail, orFail, referencedMedia } from './lib.mjs';

const BASE = orFail(serverUrl);
const DIR = process.env.YAP_BACKUP_DIR ?? path.join(ROOT, 'backups');
const KEEP = 60;
const EXT = { 'image/png': 'png', 'image/gif': 'gif', 'image/webp': 'webp', 'image/jpeg': 'jpg', 'audio/wav': 'wav' };

async function get(pathname, key) {
  const res = await fetch(`${BASE}${pathname}`, { headers: { Authorization: `Bearer ${key}` } });
  if (!res.ok) throw new Error(`${pathname}: HTTP ${res.status} ${await res.text()}`);
  return res;
}

async function main() {
  const key = adminKey();
  if (process.argv.includes('--check')) return console.log(`backs up ${BASE} into ${DIR}`);
  const started = Date.now();
  const media = path.join(DIR, 'media');
  fs.mkdirSync(media, { recursive: true });

  // Chat data, paged.
  const backup = { format: 'yap-backup', version: 1, server: BASE, exportedAt: new Date().toISOString(), messages: [] };
  let after = 0;
  for (;;) {
    const page = await (await get(`/api/admin/export?after=${after}`, key)).json();
    // First page: every table but messages (channels, users, emojis, reactions, drawings, …).
    if (after === 0) for (const [k, v] of Object.entries(page)) if (k !== 'messages' && k !== 'next' && v !== undefined) backup[k] = v;
    backup.messages.push(...page.messages);
    if (page.next === null) break;
    after = page.next;
  }
  const stats = await (await get('/api/admin/stats', key)).json();
  backup.stats = stats;

  const stamp = new Date().toISOString().slice(0, 10);
  const file = path.join(DIR, `yap-${stamp}.json.gz`);
  fs.writeFileSync(`${file}.tmp`, zlib.gzipSync(JSON.stringify(backup)));
  fs.renameSync(`${file}.tmp`, file);

  // Stored files (emojis, pictures, avatars, drawings, doodles, clips, sealed
  // pictures): content-addressed, so only new ones are fetched. Through the
  // admin route, which nothing caches (a picture may be erased tomorrow).
  let fetched = 0;
  let gone = 0;
  const listed = await (await get('/api/admin/media', key)).json();
  for (const blob of listed) {
    const out = path.join(media, `${blob.id}.${EXT[blob.mime] ?? 'bin'}`);
    if (fs.existsSync(out)) continue;
    const res = await fetch(`${BASE}/api/admin/media/${blob.id}`, { headers: { Authorization: `Bearer ${key}` } });
    if (res.status === 404) {
      gone++; // erased (its post deleted) since the list was made
      continue;
    }
    if (!res.ok) throw new Error(`media ${blob.id}: HTTP ${res.status}`);
    fs.writeFileSync(`${out}.tmp`, Buffer.from(await res.arrayBuffer()));
    fs.renameSync(`${out}.tmp`, out);
    fetched++;
  }

  // Keep the newest KEEP snapshots.
  const snapshots = fs.readdirSync(DIR).filter((f) => /^yap-\d{4}-\d{2}-\d{2}\.json\.gz$/.test(f)).sort();
  for (const old of snapshots.slice(0, Math.max(0, snapshots.length - KEEP))) fs.unlinkSync(path.join(DIR, old));

  // And only the files someone still needs: the server's, or a kept snapshot's
  // (to restore it). A deleted post's picture, or a sealed picture its owner
  // deleted, goes once nothing refers to it.
  let pruned = 0;
  const onServer = new Set(listed.map((b) => b.id));
  const unlisted = fs.readdirSync(media).filter((f) => !onServer.has(f.split('.')[0]));
  if (unlisted.length) {
    const needed = new Set();
    for (const snapshot of snapshots.slice(-KEEP)) {
      for (const id of referencedMedia(JSON.parse(zlib.gunzipSync(fs.readFileSync(path.join(DIR, snapshot)))))) needed.add(id);
    }
    for (const f of unlisted) {
      if (needed.has(f.split('.')[0]) && !f.endsWith('.tmp')) continue;
      fs.unlinkSync(path.join(media, f));
      pruned++;
    }
  }

  const mb = (n) => `${(n / 1e6).toFixed(2)} MB`;
  console.log(
    `${new Date().toISOString()} backup ok → ${path.relative(ROOT, file)} ` +
      `(${backup.messages.length} messages, ${backup.emojis.length} emojis, +${fetched} files${gone ? `, ${gone} erased meanwhile` : ''}${pruned ? `, ${pruned} no longer needed removed` : ''}; ` +
      `chat db ${mb(stats.hub.databaseBytes)} / 1 GB, media db ${mb(stats.media.databaseBytes)} / 1 GB; ${Date.now() - started} ms)`,
  );
  // Heads-up well before a database fills (Yap also warns the owner/mods in the app).
  for (const [name, bytes] of [['chat', stats.hub.databaseBytes], ['media', stats.media.databaseBytes]]) {
    if (bytes > 0.8 * 1024 ** 3) console.log(`${new Date().toISOString()} WARNING: the ${name} database is ${mb(bytes)} — over 80% of its 1 GB.`);
  }
}

main().catch((err) => fail(`${new Date().toISOString()} backup FAILED: ${err.message}`));
