// Encrypted Personal space: the desktop's Rust crypto (src/vault.rs) and the
// web's (web/js/vault.js) must open each other's notes, pictures and wraps.
// Both directions, at the real 600k PBKDF2 iterations.
//
//   node desktop/tests/vault-crosscheck.mjs      (runs `cargo test --release vault::`)
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createVault, openVault, encryptNote, decryptNote, encryptBlob, decryptBlob, b64u, unb64u } from '../../web/js/vault.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'yap-vault-'));
const cargo = process.env.CARGO ?? path.join(os.homedir(), '.cargo/bin/cargo');
const rust = (env) => execFileSync(cargo, ['test', '--release', '--quiet', 'vault::tests::', '--', '--nocapture'], { cwd: path.join(here, '..'), env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'inherit'] }).toString();

// Web → Rust.
const passcode = 'purple tiger river 42 ﬁ';
let t = performance.now();
const { wrap, key } = await createVault(passcode);
const webCreateMs = Math.round(performance.now() - t);
const text = 'from the web: wifi at the cabin is on the fridge 🧊';
const note = await encryptNote(key, { text, image: { id: 'b'.repeat(32), width: 10, height: 20, mime: 'image/webp' } });
const plainBlob = crypto.getRandomValues(new Uint8Array(4000));
const blob = await encryptBlob(key, plainBlob);
const webFile = path.join(dir, 'web.json');
fs.writeFileSync(webFile, JSON.stringify({ wrap, passcode, note, text, blob: b64u(blob), plainBlob: b64u(plainBlob) }));

// Rust → web (and Rust opens the web's fixture in the same run).
const rustFile = path.join(dir, 'rust.json');
rust({ YAP_VAULT_FIXTURE: webFile, YAP_VAULT_WRITE: rustFile });
const r = JSON.parse(fs.readFileSync(rustFile, 'utf8'));
assert.equal(r.wrap.iter, 600000);
t = performance.now();
const k2 = await openVault(r.wrap, r.passcode);
const webUnlockMs = Math.round(performance.now() - t);
const n = await decryptNote(k2, r.note);
assert.equal(n.text, r.text);
assert.equal(n.gif.slug, 'cat');
assert.equal(n.image.mime, 'image/jpeg');
assert.deepEqual(await decryptBlob(k2, unb64u(r.blob)), unb64u(r.plainBlob));
await assert.rejects(openVault(r.wrap, 'not the passcode'));

console.log(`✓ web → Rust: Rust opened the web's wrap, note and picture`);
console.log(`✓ Rust → web: vault.js opened Rust's wrap (NFKC passcode), note (+GIF, picture ref) and picture`);
console.log(`PBKDF2 600k: Rust create ${r.createMs} ms, unlock ${r.unlockMs} ms (release); web create ${webCreateMs} ms, unlock ${webUnlockMs} ms (Node)`);
