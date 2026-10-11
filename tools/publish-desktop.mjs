#!/usr/bin/env node
/*
 * Publishes the desktop app's binaries (from desktop/target/dist/, where
 * desktop/scripts/build-cross.sh leaves them; else dist/; or --from <dir>) for download and for the
 * apps' click-to-update (PROTOCOL.md › Updates):
 *   web/downloads/yap-<version>-<platform>[.exe]   the files, signed
 *   web/downloads/latest.json                      what the apps check
 * Each file is signed (Ed25519, raw bytes). The apps verify against the
 * public key built into them (desktop/README.md › Self-update), so nothing
 * unsigned can ever install.
 *
 * The signing key (base64 of the PKCS#8 DER private key, or a PEM block):
 *   - env YAP_UPDATE_SIGNING_KEY, else
 *   - the macOS Keychain item with the service name `yap-update-signing-key`.
 * It is never printed. A binary that doesn't contain the matching public key
 * (built without it, or with another one) is refused: its users could never
 * install the update.
 *
 *   node tools/publish-desktop.mjs [--note "…"]...   then: npm run deploy
 *   node tools/publish-desktop.mjs --local           same files, no tag (try it on a dev server)
 *   node tools/publish-desktop.mjs --from <dir>      the binaries are somewhere else
 *
 * Notes default to the `Changelog:` lines of commits touching desktop/ since
 * the last `desktop-v*` tag. Tags `desktop-v<version>` when done.
 * web/downloads/ is gitignored: deploy from the machine that has it, or the
 * downloads vanish from the site.
 */
import { createHash, createPrivateKey, createPublicKey, sign, verify } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { ROOT, changelogLines, fail, git, keychain } from './lib.mjs';

const root = ROOT;
const out = join(root, 'web/downloads');
const PLATFORMS = {
  'windows-x86_64': { file: 'yap-windows-x86_64.exe', ext: '.exe' },
  'linux-x86_64': { file: 'yap-linux-x86_64', ext: '' },
  'macos-aarch64': { file: 'yap-macos-aarch64', ext: '' },
};
const local = process.argv.includes('--local');
// The build's own output first, so a stale copy in dist/ can't be published by mistake.
const fromArg = process.argv[process.argv.indexOf('--from') + 1];
const dist = process.argv.includes('--from') && fromArg
  ? resolve(fromArg)
  : [join(root, 'desktop/target/dist'), join(root, 'dist')].find((d) => existsSync(d)) ?? join(root, 'desktop/target/dist');
const shown = relative(root, dist) || dist;
const MAX_ASSET = 25 * 1024 * 1024; // Workers static assets: 25 MiB per file
const KEEP_VERSIONS = 2; // the previous one stays, so a download in flight never 404s
const KEY_ENV = 'YAP_UPDATE_SIGNING_KEY';
const KEYCHAIN_SERVICE = 'yap-update-signing-key';

const die = (msg) => fail(`publish-desktop: ${msg}`);

/** The signing key's text and where it came from. Nothing here prints it. */
function signingKeyText() {
  const fromEnv = process.env[KEY_ENV]?.trim();
  if (fromEnv) return { text: fromEnv, source: `env ${KEY_ENV}` };
  const fromKeychain = keychain(KEYCHAIN_SERVICE);
  if (fromKeychain) return { text: fromKeychain, source: `Keychain ${KEYCHAIN_SERVICE}` };
  die(`no signing key. Set ${KEY_ENV} (base64 of the PKCS#8 DER Ed25519 private key, or a PEM block)${process.platform === 'darwin' ? `, or keep it in the Keychain as service "${KEYCHAIN_SERVICE}"` : ''}. desktop/README.md › Self-update explains how to make one.`);
}

function loadSigningKey() {
  const { text, source } = signingKeyText();
  let privateKey;
  try {
    privateKey = text.startsWith('-----BEGIN')
      ? createPrivateKey(text)
      : createPrivateKey({ key: Buffer.from(text, 'base64'), format: 'der', type: 'pkcs8' });
  } catch {
    die(`the signing key from ${source} isn't a readable private key (expected base64 PKCS#8 DER or PEM).`);
  }
  if (privateKey.asymmetricKeyType !== 'ed25519') die(`the signing key from ${source} is ${privateKey.asymmetricKeyType}, not Ed25519.`);
  const publicKey = createPublicKey(privateKey);
  // Raw 32-byte public key, base64: what the apps are built with (YAP_UPDATE_PUBKEY).
  const publicB64 = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64');
  return { privateKey, publicKey, publicB64, source };
}

const version = readFileSync(join(root, 'desktop/Cargo.toml'), 'utf8').match(/^version\s*=\s*"([^"]+)"/m)?.[1];
if (!version) die('no version in desktop/Cargo.toml');
if (!local && git('tag', '-l', `desktop-v${version}`)) die(`desktop-v${version} is already published; bump desktop/Cargo.toml first`);

const notes = process.argv.flatMap((a, i, all) => (a === '--note' && all[i + 1] ? [all[i + 1]] : []));
if (!notes.length) {
  const last = git('tag', '-l', 'desktop-v*', '--sort=-v:refname').split('\n')[0];
  const log = git('log', '--format=%B%x00', ...(last ? [`${last}..HEAD`] : []), '--', 'desktop');
  for (const line of changelogLines(log)) if (!notes.includes(line)) notes.push(line);
}

const { privateKey, publicKey, publicB64, source } = loadSigningKey();
console.log(`signing with the key from ${source}; public key ${publicB64}; binaries from ${shown}/`);

// Check every binary before writing anything.
const ready = [];
for (const [platform, { file, ext }] of Object.entries(PLATFORMS)) {
  const src = join(dist, file);
  if (!existsSync(src)) die(`missing ${shown}/${file} (desktop/scripts/build-cross.sh all)`);
  const bytes = readFileSync(src);
  if (bytes.length > MAX_ASSET) die(`${shown}/${file} is ${bytes.length} bytes, over the 25 MiB asset limit`);
  if (!bytes.includes(Buffer.from(publicB64, 'ascii'))) {
    die(`${shown}/${file} wasn't built with this signing key's public key, so installed apps would refuse the update. Rebuild with YAP_UPDATE_PUBKEY=${publicB64} (or put it in desktop/update-pubkey.txt), or sign with the key it was built for.`);
  }
  const sig = sign(null, bytes, privateKey);
  if (!verify(null, bytes, publicKey, sig)) die(`the signature of ${shown}/${file} doesn't verify`);
  ready.push({ platform, name: `yap-${version}-${platform}${ext}`, bytes, sig });
}

mkdirSync(out, { recursive: true });
const files = {};
for (const { platform, name, bytes, sig } of ready) {
  writeFileSync(join(out, name), bytes);
  files[platform] = { path: `/downloads/${name}`, size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex'), sig: sig.toString('base64') };
  console.log(`${platform.padEnd(15)} ${name} ${(bytes.length / 1e6).toFixed(1)} MB`);
}

const manifest = { version, date: new Date().toISOString().slice(0, 10), notes, files };
writeFileSync(join(out, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`);

// Keep this version and the one before; older binaries go.
const versionOf = (name) => name.match(/^yap-(\d+\.\d+\.\d+)-/)?.[1];
const cmp = (a, b) => a.split('.').map(Number).reduce((r, n, i) => r || n - Number(b.split('.')[i]), 0);
const keep = [...new Set(readdirSync(out).map(versionOf).filter(Boolean))].sort(cmp).slice(-KEEP_VERSIONS);
for (const name of readdirSync(out)) {
  if (versionOf(name) && !keep.includes(versionOf(name))) {
    rmSync(join(out, name));
    console.log(`removed ${name}`);
  }
}

if (!local) git('tag', '-a', `desktop-v${version}`, '-m', `Desktop ${version}`);
const total = readdirSync(out).reduce((n, f) => n + statSync(join(out, f)).size, 0);
console.log(`\nlatest.json → ${version} (${notes.length} notes)${local ? ', NOT tagged (--local)' : `, tagged desktop-v${version}`}; web/downloads/ is ${(total / 1e6).toFixed(1)} MB.\n${local ? 'Local try-out only: run without --local to publish.' : 'Now: npm run deploy'}`);
