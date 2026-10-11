// End-to-end test of signed click-to-update (macOS/Linux), against a local
// static server and a throwaway Ed25519 key:
//   - a tampered file, a wrong-key signature and a size lie are refused and the
//     executable is left alone;
//   - a good update is downloaded, verified, swapped in and relaunched with the
//     same arguments (the "new version" is a tiny script that records its args);
//   - leftovers (yap.old, temp files) are cleaned on the next start;
//   - a build without an update key never asks the server;
//   - a release build ignores the YAP_UPDATE_URL / YAP_UPDATE_PUBKEY overrides.
//
//   node desktop/tests/update-e2e.mjs        (needs target/debug/yap; target/release/yap optional)
//   (CARGO_TARGET_DIR moves target/; YAP_BIN / YAP_RELEASE_BIN name the binaries)
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const TARGET = process.env.CARGO_TARGET_DIR ?? path.join(here, '../target');
const DEBUG = process.env.YAP_BIN ?? path.join(TARGET, 'debug/yap');
const RELEASE = process.env.YAP_RELEASE_BIN ?? path.join(TARGET, 'release/yap');
const PORT = Number(process.env.YAP_UPDATE_PORT ?? 8796);
const OUT = fs.mkdtempSync(path.join(os.tmpdir(), 'yap-update-'));
const platform = { darwin: 'macos', linux: 'linux', win32: 'windows' }[process.platform] + '-' + { arm64: 'aarch64', x64: 'x86_64' }[process.arch];

const keyPair = () => {
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519');
  const raw = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
  return { pub: raw.toString('base64'), privateKey };
};
const good = keyPair();
const other = keyPair();
const sha256 = (b) => crypto.createHash('sha256').update(b).digest('hex');
const sign = (b, k) => crypto.sign(null, b, k.privateKey).toString('base64');

// The "new release": records that it ran, and with which arguments.
const ranFile = path.join(OUT, 'ran.txt');
const newBin = Buffer.from(`#!/bin/sh\necho "NEW VERSION RAN $@" > "${ranFile}"\n`);

let manifest = {};
let manifestRequests = 0;
const server = http.createServer((req, res) => {
  if (req.url === '/latest.json') {
    manifestRequests++;
    res.writeHead(200, { 'Content-Type': 'application/json' });
    return res.end(JSON.stringify(manifest));
  }
  if (req.url === '/files/yap-new') {
    res.writeHead(200, { 'Content-Type': 'application/octet-stream' });
    return res.end(served);
  }
  res.writeHead(404);
  res.end();
});
let served = newBin;
await new Promise((r) => server.listen(PORT, r));

function entry(bytes, { key = good, size = bytes.length } = {}) {
  return { path: '/files/yap-new', size, sha256: sha256(bytes), sig: sign(bytes, key) };
}
function setRelease(e) {
  manifest = { version: '99.0.0', date: '2026-10-05', notes: ['Test release'], files: { [platform]: e } };
}
function freshApp(name) {
  const dir = path.join(OUT, name);
  fs.mkdirSync(dir, { recursive: true });
  const exe = path.join(dir, 'yap');
  fs.copyFileSync(DEBUG, exe);
  fs.chmodSync(exe, 0o755);
  return exe;
}
// Async on purpose: the HTTP server above lives in this same process.
function run(exe, args, env = {}, overrides = true) {
  const base = overrides ? { YAP_UPDATE_URL: `http://localhost:${PORT}/latest.json`, YAP_UPDATE_PUBKEY: good.pub } : {};
  return new Promise((resolve) => {
    const c = spawn(exe, args, { env: { ...process.env, ...base, ...env } });
    let out = '';
    c.stdout.on('data', (d) => (out += d));
    c.stderr.on('data', (d) => (out += d));
    c.on('exit', () => resolve(out));
  });
}

const results = [];
async function test(name, fn) {
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(['✗', name, err]);
  }
}
const original = sha256(fs.readFileSync(DEBUG));
const args = ['--update-install', '--server', 'http://localhost:1'];

await test('check reports the newer signed release', async () => {
  setRelease(entry(newBin));
  const out = await run(freshApp('check'), ['--update-check', '--server', 'http://localhost:1']);
  assert.match(out, new RegExp(`UPDATE available 99.0.0 \\(${newBin.length} bytes\\)`));
});

await test('tampered file is refused, exe untouched', async () => {
  const exe = freshApp('tampered');
  setRelease(entry(newBin));
  served = Buffer.from(newBin.toString().replace('NEW VERSION', 'EVIL VERSIO')); // same size, different bytes
  const out = await run(exe, args);
  assert.match(out, /UPDATE failed: Update rejected: SHA-256/);
  assert.equal(sha256(fs.readFileSync(exe)), original);
  served = newBin;
});

await test('signature from another key is refused, exe untouched', async () => {
  const exe = freshApp('wrongkey');
  setRelease(entry(newBin, { key: other }));
  const out = await run(exe, args);
  assert.match(out, /signature doesn't match/);
  assert.equal(sha256(fs.readFileSync(exe)), original);
});

await test('size mismatch is refused', async () => {
  const exe = freshApp('size');
  setRelease(entry(newBin, { size: newBin.length + 5 }));
  const out = await run(exe, args);
  assert.match(out, /size/);
  assert.equal(sha256(fs.readFileSync(exe)), original);
});

await test('good update: verified, swapped in, relaunched with the same args', async () => {
  const exe = freshApp('good');
  setRelease(entry(newBin));
  fs.rmSync(ranFile, { force: true });
  const out = await run(exe, args);
  assert.match(out, /UPDATE installed 99.0.0/);
  for (let i = 0; i < 50 && !fs.existsSync(ranFile); i++) await new Promise((r) => setTimeout(r, 100));
  assert.equal(fs.readFileSync(ranFile, 'utf8').trim(), `NEW VERSION RAN ${args.join(' ')}`);
  assert.equal(sha256(fs.readFileSync(exe)), sha256(newBin), 'the executable now is the new release');
  assert.ok((fs.statSync(exe).mode & 0o111) !== 0, 'still executable');
});

await test('leftovers are cleaned up on the next start', async () => {
  const exe = freshApp('cleanup');
  fs.writeFileSync(`${exe}.old`, 'old');
  fs.writeFileSync(path.join(path.dirname(exe), '.yap.update-abcd.tmp'), 'tmp');
  await run(exe, ['--update-check', '--server', 'http://localhost:1'], { YAP_UPDATE_URL: 'http://localhost:1/none.json' });
  assert.ok(!fs.existsSync(`${exe}.old`), 'yap.old removed');
  assert.ok(!fs.existsSync(path.join(path.dirname(exe), '.yap.update-abcd.tmp')), 'temp file removed');
});

await test('without an update key there are no checks', async () => {
  setRelease(entry(newBin));
  const before = manifestRequests;
  const out = await run(freshApp('nokey'), ['--update-check', '--server', 'http://localhost:1'], { YAP_UPDATE_URL: `http://localhost:${PORT}/latest.json`, YAP_UPDATE_PUBKEY: '' }, false);
  if (/self-update on/.test(out)) {
    // This debug build has a key built in (YAP_UPDATE_PUBKEY at build time): it checks.
    assert.match(out, /UPDATE available 99\.0\.0/);
  } else {
    assert.match(out, /UPDATE off/);
    assert.equal(manifestRequests, before, 'the server was never asked');
  }
});

if (fs.existsSync(RELEASE)) {
  await test('release builds ignore the test overrides', async () => {
    const out = await run(RELEASE, ['--update-check', '--server', 'http://localhost:1']);
    // Built with a key: it checks its own server, not the override; without one: no checks.
    assert.match(out, /checking http:\/\/localhost:1\/downloads\/latest\.json|UPDATE off/);
    assert.doesNotMatch(out, /UPDATE available/);
  });
}

server.close();
for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.message ?? err}`);
}
const failed = results.filter((r) => r[0] === '✗').length;
console.log(`${results.length - failed}/${results.length} passed (${OUT})`);
process.exit(failed ? 1 : 0);
