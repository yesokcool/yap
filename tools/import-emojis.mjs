#!/usr/bin/env node
// Bulk-import a folder of images as custom emojis (macOS; uses `sips`).
//   node tools/import-emojis.mjs ~/Pictures/emojis [--dry-run]
// Names come from filenames ("PartyTime.png" → party_time); a name
// that's taken (the server says so) gets _2, _3… Static images over 128 px are
// scaled down; SVGs are rasterized (the server never accepts SVG). Server and
// admin key: tools/instance.mjs.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { adminKey, serverUrl } from './instance.mjs';
import { ROOT, fail, orFail, usage } from './lib.mjs';

const dir = process.argv[2];
const dryRun = process.argv.includes('--dry-run');
if (!dir) usage('node tools/import-emojis.mjs <folder> [--dry-run]');
const BASE = orFail(serverUrl);
const KEY = orFail(adminKey);
const auth = { Authorization: `Bearer ${KEY}` };
const MAX_BYTES = 512 * 1024;
const SIDE = 128;
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'yap-emoji-'));

// Standard emoji shortcodes: a custom emoji named like one would shadow it.
const STANDARD = new Set(
  JSON.parse(fs.readFileSync(path.join(ROOT, 'web/emoji-data.json'))).emojis.flatMap((e) => e[2].split(' ')),
);

function nameFor(file) {
  const stem = path.basename(file, path.extname(file));
  const numbered = stem.match(/^(\d+) - \w+$/); // "042 - XkQ7z" (random code) → e042
  if (numbered) return `e${numbered[1]}`;
  const base = stem
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2') // camelCase → camel_case
    .replace(/([A-Z])([A-Z][a-z])/g, '$1_$2') // DayIKilled → Day_I_Killed
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, 32)
    .replace(/_+$/, '');
  const name = base.length >= 2 ? base : `emoji_${base}`;
  return STANDARD.has(name) ? `${name.slice(0, 30)}_c` : name;
}

function dimensions(file) {
  const out = execFileSync('sips', ['-g', 'pixelWidth', '-g', 'pixelHeight', file]).toString();
  return [Number(out.match(/pixelWidth: (\d+)/)?.[1]), Number(out.match(/pixelHeight: (\d+)/)?.[1])];
}

/** Returns a path to upload: the original, or a scaled/converted PNG. */
function prepare(file) {
  const ext = path.extname(file).toLowerCase();
  if (ext === '.gif' || ext === '.webp') {
    if (fs.statSync(file).size > MAX_BYTES) throw new Error('animated image over 512 KB (would lose animation if scaled)');
    return file;
  }
  const [w, h] = dimensions(file);
  if (ext === '.png' && Math.max(w, h) <= SIDE && fs.statSync(file).size <= MAX_BYTES) return file;
  const out = path.join(tmp, `${path.basename(file, ext)}.png`);
  const args = Math.max(w, h) > SIDE ? ['-Z', String(SIDE)] : [];
  execFileSync('sips', [...args, '-s', 'format', 'png', file, '--out', out], { stdio: 'ignore' });
  if (fs.statSync(out).size > MAX_BYTES) throw new Error('still over 512 KB after scaling');
  return out;
}

/** POST one emoji (retrying the server's own hiccups) → its JSON answer. */
async function post(name, bytes) {
  let res;
  for (let attempt = 0; attempt < 3; attempt++) {
    res = await fetch(`${BASE}/api/admin/emoji?name=${encodeURIComponent(name)}`, { method: 'POST', headers: auth, body: bytes });
    if (res.status < 500) break;
    await new Promise((r) => setTimeout(r, 2000));
  }
  if (res.status === 401) fail('The server refused the admin key (YAP_ADMIN_KEY, or the Keychain item; tools/instance.mjs).');
  return res.json().catch(() => ({ message: `HTTP ${res.status}` }));
}

// Names this run gave out; the server knows the rest (it answers name_taken).
const taken = new Set();
const files = fs.readdirSync(dir).filter((f) => !f.startsWith('.')).sort((a, b) => a.localeCompare(b));
const report = { added: [], duplicate: [], skipped: [], failed: [] };

for (const f of files) {
  const file = path.join(dir, f);
  if (!/\.(png|jpe?g|gif|webp|svg)$/i.test(f)) {
    report.skipped.push(`${f} (not an image format browsers show)`);
    continue;
  }
  const base = nameFor(f);
  let n = 1;
  let name = base;
  const next = () => (name = `${base.slice(0, 29)}_${++n}`);
  while (taken.has(name)) next();
  try {
    const upload = prepare(file);
    if (dryRun) {
      report.added.push(`:${name}: ← ${f}`);
      taken.add(name);
      continue;
    }
    let body = await post(name, fs.readFileSync(upload));
    while (body.code === 'name_taken' && n < 50) {
      while (taken.has(next()));
      body = await post(name, fs.readFileSync(upload));
    }
    if (body.ok) {
      report.added.push(`:${name}: ← ${f}`);
      taken.add(name);
    } else if (body.code === 'duplicate') {
      report.duplicate.push(`${f} (${body.message})`);
    } else {
      report.failed.push(`${f}: ${body.message}`);
    }
  } catch (err) {
    report.failed.push(`${f}: ${err.message}`);
  }
}

fs.rmSync(tmp, { recursive: true, force: true });
for (const [kind, items] of Object.entries(report)) {
  console.log(`${kind}: ${items.length}`);
  for (const item of items) console.log(`  ${item}`);
}
if (dryRun) console.log('(A dry run asks the server nothing: names already taken there get _2, _3… for real.)');
process.exit(report.failed.length ? 1 : 0);
