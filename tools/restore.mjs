#!/usr/bin/env node
// Disaster recovery: rebuild a Yap server from a backup (tools/backup.mjs
// output) — e.g. on a fresh Cloudflare account. Only works into an EMPTY
// server (no users, no messages): the server refuses anything else, so this
// can't overwrite real data. Files go first (only those this backup refers
// to, plus sealed pictures, which can't be told apart), then the tables,
// then the messages in pages.
//
//   node tools/restore.mjs backups/yap-2026-10-05.json.gz --to https://new.example.com
//     [--media backups/media]     # default: the "media" folder next to the file
//   Admin key: YAP_ADMIN_KEY, or the Keychain item "yap-admin-key".
//
// There is deliberately no default --to: say which server you mean.
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { adminKey } from './instance.mjs';
import { fail, referencedMedia, usage } from './lib.mjs';

const args = process.argv.slice(2);
const option = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const file = args.find((a, i) => !a.startsWith('--') && !['--to', '--media'].includes(args[i - 1]));
const base = option('--to')?.replace(/\/+$/, '');
if (!file || !base) usage('node tools/restore.mjs <yap-YYYY-MM-DD.json.gz> --to <server url> [--media <dir>]');
const mediaDir = option('--media') ?? path.join(path.dirname(file), 'media');
const PAGE = 1000;

async function post(pathname, body, key, type = 'application/json') {
  let res;
  for (let attempt = 0; ; attempt++) {
    res = await fetch(`${base}${pathname}`, {
      method: 'POST',
      headers: { Authorization: `Bearer ${key}`, 'Content-Type': type },
      body: type === 'application/json' ? JSON.stringify(body) : body,
    });
    if (res.status !== 429 || attempt >= 5) break;
    await new Promise((r) => setTimeout(r, 1000 * Number(res.headers.get('Retry-After') ?? 5)));
  }
  const json = await res.json().catch(() => ({}));
  if (!res.ok || json.ok === false) throw new Error(`${pathname}: ${json.message ?? `HTTP ${res.status}`}`);
  return json;
}

async function main() {
  const key = adminKey();
  const backup = JSON.parse(zlib.gunzipSync(fs.readFileSync(file)));
  if (backup.format !== 'yap-backup') throw new Error(`${file} isn't a Yap backup.`);
  console.log(`Restoring ${path.basename(file)} (schema ${backup.schema}, exported ${backup.exportedAt}) into ${base}`);

  // 0. Refuse up front unless the server is empty (nothing gets uploaded otherwise).
  await post('/api/admin/import', { phase: 'check' }, key);

  // 1. Files (emojis, avatars, pictures, drawings, doodles, clips), content-addressed:
  // same bytes → same id. Only the ones this backup refers to: the media folder
  // also holds deleted posts' pictures that older snapshots still need. Sealed
  // pictures (.bin) all go: their ids are inside encrypted notes.
  const needed = referencedMedia(backup);
  let images = 0;
  let skipped = 0;
  if (fs.existsSync(mediaDir)) {
    for (const name of fs.readdirSync(mediaDir)) {
      const id = name.split('.')[0];
      if (!/^[a-f0-9]{32}$/.test(id)) continue;
      if (!needed.has(id) && !name.endsWith('.bin')) {
        skipped++;
        continue;
      }
      const stored = await post('/api/admin/media-put', fs.readFileSync(path.join(mediaDir, name)), key, 'application/octet-stream');
      if (stored.id !== id) console.warn(`  ${name}: stored as ${stored.id} (file changed since the backup?)`);
      images++;
    }
  } else {
    console.warn(`  No media folder at ${mediaDir}: emojis and pictures won't show.`);
  }
  console.log(`  files: ${images}${skipped ? ` (${skipped} this backup doesn't use left out)` : ''}`);

  // 2. Every table but messages (refused unless the server is empty).
  const { messages, ...rest } = backup;
  const begun = await post('/api/admin/import', { phase: 'begin', data: rest }, key);
  console.log(`  rows: ${begun.count}`);

  // 3. Messages, a page at a time.
  let restored = 0;
  for (let i = 0; i < messages.length; i += PAGE) {
    restored += (await post('/api/admin/import', { phase: 'messages', messages: messages.slice(i, i + PAGE) }, key)).count;
  }
  console.log(`  messages: ${restored} of ${messages.length}`);

  await post('/api/admin/import', { phase: 'finish' }, key);
  console.log('Done. Everyone signs in with their usual name and PIN (the server needs the same PIN_PEPPER secret as before).');
}

main().catch((err) => fail(`restore FAILED: ${err.message}`));
