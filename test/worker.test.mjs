// The Worker's pure rules, no server needed: accounts (worker/auth.ts), what
// clients send (worker/validate.ts). (Mentions: web/js/mentions.js, in test/unit.test.mjs.)
//   node --experimental-strip-types --no-warnings test/worker.test.mjs
import assert from 'node:assert/strict';
import nodeCrypto from 'node:crypto';
import fs from 'node:fs';
import { TEMP_RE, deviceLabel, hashPin, isPassword, isSecret, needsRehash, passwordRule, tempPassword, verifyPin, weakPassword } from '../worker/auth.ts';
import COMMON_PASSWORDS from '../worker/common-passwords.ts';
import { WORDS } from '../worker/words.ts';
import { clamp, cleanAnchor, cleanGif, cleanImage, cleanStroke, cleanVaultWrap } from '../worker/validate.ts';

// Workers have crypto.subtle.timingSafeEqual (a Cloudflare extra); Node doesn't.
crypto.subtle.timingSafeEqual ??= (a, b) => nodeCrypto.timingSafeEqual(Buffer.from(a), Buffer.from(b));

const results = [];
async function test(name, fn) {
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(['✗', name, err]);
  }
}
const codeOf = (fn) => {
  try {
    fn();
  } catch (err) {
    return err.code ?? err.message;
  }
  return null;
};

// ── Accounts ──────────────────────────────────────────────────────────────
await test('a new password is 8–128 characters (a 6-digit PIN isn’t one)', () => {
  for (const ok of ['correct-horse-582039', 'abcd efg', 'x'.repeat(128), '🦊🦊🦊🦊🦊🦊🦊🦊']) assert.equal(passwordRule(ok), null, ok);
  for (const short of ['582039', 'short', '1234567', '']) assert.equal(passwordRule(short), 'Use a password of at least 8 characters.', short);
  assert.match(passwordRule('x'.repeat(129)), /at most 128/);
  assert.equal(isPassword('🦊'.repeat(8)), true, 'characters, not UTF-16 units');
});

await test('sign-in takes a password or an old 6-digit PIN', () => {
  for (const ok of ['582039', 'correct-horse-582039', 'purple tiger river 7', 'x'.repeat(128)]) assert.equal(isSecret(ok), true, ok);
  for (const bad of ['58203', '5820391', 'short', 'x'.repeat(129), '']) assert.equal(isSecret(bad), false, bad);
});

await test('guessable passwords are refused: common ones, your name, one thing repeated, keyboard and number runs', () => {
  const common = ['password', 'Password1', 'iloveyou', 'sunshine2024!', 'football', '1q2w3e4r', 'trustno1', 'superman', 'baseball1'];
  const name = [['cleo2024', 'Cleo'], ['Cleo-12345', 'Cleo'], ['2009cleo', 'Cleo'], ['cleocleo', 'Cleo'], ['ADALOVELACE', 'Ada Lovelace'], ['lovelace1815', 'Ada Lovelace'], ['wrenna!!!', 'Wrenna'], ['ab12wrenna', 'Wrenna ab12'], ['xx-cleo-x', 'Cleo']];
  const repeated = ['aaaaaaaa', '!!!!!!!!!!', 'abababab', 'xyzxyzxyz', '12121212'];
  const runs = ['12345678', '87654321', '0123456789', 'qwertyuiop', 'asdfghjkl', 'qwerty12', 'qwerty123456', 'abcdefgh', 'zyxwvuts', '1qaz2wsx3edc', 'qwertzuiop', 'azertyuiop', '13579135', 'abc123def456'];
  for (const pw of [...common, ...repeated, ...runs]) assert.match(weakPassword(pw) ?? '', /^That password is too easy to guess\./, pw);
  for (const [pw, who] of name) assert.match(weakPassword(pw, who) ?? '', /mostly your name/, `${pw} for ${who}`);
  const fine = ['correct-horse-582039', 'purple tiger river 7', 'maple-river-stone-42', 'my dog ate the pasta', 'Cleo plays the banjo', 'always-be-kind-71', 'deer in the meadow', 'stonewall jackson x'];
  for (const pw of fine) assert.equal(weakPassword(pw, 'Cleo'), null, pw);
  assert.equal(weakPassword('always sunny 42', 'Al'), null, 'a short name inside a word is fine');
});

await test('the common-password list is bundled, small and lowercase', () => {
  const size = fs.statSync(new URL('../worker/common-passwords.ts', import.meta.url)).size;
  assert.ok(size <= 100_000, `${size} bytes`);
  const list = COMMON_PASSWORDS.split('\n');
  assert.ok(list.length > 10_000, `${list.length} passwords`);
  assert.equal(list[0], 'password', 'most common first');
  assert.ok(list.every((pw) => pw.length >= 8 && pw === pw.toLowerCase()));
});

await test('PIN hashes: right PIN + same pepper verifies; a wrong PIN or another pepper never does', async () => {
  const stored = await hashPin('582039', 'pepper');
  assert.match(stored, /^pbkdf2-sha256-pepper\$100000\$[\w-]{22}\$[\w-]{43}$/);
  assert.equal(await verifyPin('582039', stored, 'pepper'), true);
  assert.equal(await verifyPin('582038', stored, 'pepper'), false);
  assert.equal(await verifyPin('582039', stored, 'other pepper'), false, 'a backup without the pepper can’t be checked offline');
  assert.notEqual(await hashPin('582039', 'pepper'), stored, 'salted');
  assert.equal(needsRehash(stored), false);
  assert.equal(await verifyPin('582039', null, 'pepper'), false);
  assert.equal(await verifyPin('not a secret', stored, 'pepper'), false);
});

await test('old unpeppered hashes still verify, and are flagged for an upgrade', async () => {
  const salt = nodeCrypto.randomBytes(16);
  const hash = nodeCrypto.pbkdf2Sync('582039', salt, 20_000, 32, 'sha256');
  const legacy = `pbkdf2-sha256$20000$${salt.toString('base64url')}$${hash.toString('base64url')}`;
  assert.equal(await verifyPin('582039', legacy, 'any pepper'), true);
  assert.equal(await verifyPin('111111', legacy, 'any pepper'), false);
  assert.equal(needsRehash(legacy), true);
  assert.equal(await verifyPin('582039', 'md5$1$abc$def', 'pepper'), false, 'unknown schemes never verify');
});

await test('temporary passwords: three different short words and two digits, never guessable', () => {
  assert.ok(WORDS.length >= 300 && WORDS.every((w) => /^[a-z]{3,6}$/.test(w)), 'short lowercase words');
  assert.equal(new Set(WORDS).size, WORDS.length, 'no word twice');
  const seen = new Set();
  for (let i = 0; i < 300; i++) {
    const pw = tempPassword();
    assert.match(pw, TEMP_RE);
    assert.equal(new Set(pw.split('-').slice(0, 3)).size, 3, pw);
    assert.equal(weakPassword(pw), null, pw);
    assert.equal(passwordRule(pw), null, pw);
    seen.add(pw);
  }
  assert.ok(seen.size > 295, 'random');
});

await test('device names from the app or the browser', () => {
  const ua = {
    chromeWin: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36',
    safariPhone: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
    edge: 'Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 Edg/130.0',
    firefoxLinux: 'Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0',
    apple: 'Yap/1 CFNetwork/1568 Darwin/24.0.0',
  };
  assert.equal(deviceLabel(ua.chromeWin, 'web'), 'Chrome on Windows');
  assert.equal(deviceLabel(ua.safariPhone, 'web'), 'Safari on iPhone');
  assert.equal(deviceLabel(ua.edge, 'web'), 'Edge on Windows');
  assert.equal(deviceLabel(ua.firefoxLinux, 'web'), 'Firefox on Linux');
  assert.equal(deviceLabel('', 'web'), 'A browser');
  assert.equal(deviceLabel('yap-desktop (Windows)', 'desktop'), 'Desktop app (Windows)');
  assert.equal(deviceLabel('', 'desktop'), 'Desktop app');
  assert.equal(deviceLabel(ua.apple, 'ios'), 'Yap for iPhone');
  assert.equal(deviceLabel(ua.apple, 'macos'), 'Yap for Mac');
  assert.equal(deviceLabel(ua.apple, 'mac'), 'Yap for Mac', 'what the Mac app sends');
  assert.equal(deviceLabel(ua.apple, 'ipad'), 'Yap for iPad', 'what the iPad app sends');
  assert.equal(deviceLabel('', 'ipad'), 'Yap for iPad');
  assert.equal(deviceLabel('yap-desktop (Mac)', 'desktop'), 'Desktop app (Mac)');
  // Client values the server doesn't know: the User-Agent decides, nothing breaks.
  assert.equal(deviceLabel(ua.safariPhone, 'some-new-thing'), 'Safari on iPhone');
  assert.equal(deviceLabel('', 'some-new-thing'), 'A browser');
  assert.equal(deviceLabel(ua.apple, 'watch'), 'Yap for iPhone', 'an Apple app under a new name');
  assert.equal(deviceLabel(ua.chromeWin, 'constructor'), 'Chrome on Windows', 'no surprises from object keys');
});

// ── What clients send ─────────────────────────────────────────────────────
const GIF = {
  provider: 'klipy', slug: 'hello-hi-662', title: 'Hello', width: 498, height: 280,
  mp4: 'https://static.klipy.com/ii/abc/a.mp4', webp: 'https://static.klipy.com/ii/abc/a.webp', still: 'https://static1.klipy.com/ii/abc/a.jpg',
};

await test('GIFs: only KLIPY’s CDN, sane sizes, a clean title; anything else is bad_gif', () => {
  assert.deepEqual(cleanGif({ ...GIF, extra: 'dropped' }), GIF);
  assert.equal(cleanGif({ ...GIF, title: ' Hi\u0000 there ' }).title, 'Hi there');
  for (const bad of [
    null, 'gif', { ...GIF, provider: 'giphy' }, { ...GIF, slug: '../x' }, { ...GIF, width: 0 }, { ...GIF, height: 5000 },
    { ...GIF, mp4: 'https://evil.example/a.mp4' }, { ...GIF, webp: 'http://static.klipy.com/a.webp' }, { ...GIF, still: 'https://static.klipy.com.evil.example/a.jpg' },
    { ...GIF, gif: 'https://static.klipy.com/a" onerror="x.gif' }, { provider: 'klipy', slug: 'x', title: '', width: 1, height: 1 },
  ]) assert.equal(codeOf(() => cleanGif(bad)), 'bad_gif', JSON.stringify(bad));
});

await test('drawing strokes: an id, a colour, a width, pairs of numbers in range; layers 0–2; text has one point', () => {
  const ok = { sk: 'stroke_123', c: '#FF0000', w: 12.4, e: false, p: [1, 2, 3.6, 4] };
  assert.deepEqual(cleanStroke(ok, 100), { id: 'stroke_123', c: '#ff0000', w: 12, e: false, p: [1, 2, 4, 4] });
  assert.equal(cleanStroke({ ...ok, l: 2 }, 100).l, 2);
  assert.equal('l' in cleanStroke({ ...ok, l: 0 }, 100), false, 'layer 0 is the default and not written');
  assert.deepEqual(cleanStroke({ ...ok, p: [5, 5], k: 'text', text: ' hi\u0001 <b> ' }, 100), { id: 'stroke_123', c: '#ff0000', w: 12, e: false, p: [5, 5], k: 'text', text: 'hi  <b>' });
  for (const bad of [
    { ...ok, sk: 'x' }, { ...ok, c: 'red' }, { ...ok, w: 0 }, { ...ok, w: 401 }, { ...ok, p: [1] }, { ...ok, p: [1, 2, 3] },
    { ...ok, p: [1, 99999] }, { ...ok, p: Array(102).fill(1) }, { ...ok, l: 3 }, { ...ok, l: 1.5 }, { ...ok, k: 'text', text: 'two points' },
    { ...ok, p: [1, 2], k: 'text', text: '   ' },
  ]) assert.equal(cleanStroke(bad, 100), null, JSON.stringify(bad));
});

await test('pictures: a content id and sane dimensions', () => {
  const id = 'a'.repeat(32);
  assert.deepEqual(cleanImage({ id, width: 640.4, height: 480 }), { id, width: 640, height: 480 });
  for (const bad of [null, { id: 'nothex', width: 1, height: 1 }, { id, width: 0, height: 1 }, { id, width: 1, height: 8001 }, { id: id.toUpperCase(), width: 1, height: 1 }]) {
    assert.equal(cleanImage(bad), null, JSON.stringify(bad));
  }
});

await test('board pointers: anchored to a card or a lane (by a safe id) or the page; kept near the anchor', () => {
  assert.deepEqual(cleanAnchor({ k: 'card', id: 'g0_0', x: 0.5, y: 0.25 }), { k: 'card', id: 'g0_0', x: 0.5, y: 0.25 });
  assert.deepEqual(cleanAnchor({ k: 'lane', id: 'l0', x: 9, y: -9, extra: 1 }), { k: 'lane', id: 'l0', x: 1.5, y: -0.5 });
  assert.deepEqual(cleanAnchor({ k: 'page', id: 'ignored', x: 0.1, y: 0.2 }), { k: 'page', x: 0.1, y: 0.2 });
  for (const bad of [null, { k: 'card', id: '<img onerror=x>', x: 0, y: 0 }, { k: 'card', x: 0, y: 0 }, { k: 'mouse', x: 0, y: 0 }, { k: 'page', x: 'a', y: 0 }]) {
    assert.equal(cleanAnchor(bad), null, JSON.stringify(bad));
  }
  assert.equal(clamp(5, 0, 3), 3);
  assert.equal(clamp(-1, 0, 3), 0);
});

await test('the encrypted Personal space key: only the expected shape is stored', () => {
  const b64 = (n) => Buffer.alloc(n, 7).toString('base64url');
  const wrap = { v: 1, iter: 600000, salt: b64(16), iv: b64(12), ct: b64(48) };
  assert.deepEqual(cleanVaultWrap({ ...wrap, note: 'dropped' }), wrap);
  for (const bad of [null, { ...wrap, v: 2 }, { ...wrap, iter: 1000 }, { ...wrap, iter: 1e8 }, { ...wrap, salt: 'short' }, { ...wrap, iv: b64(16) }, { ...wrap, ct: `${b64(48)}=` }]) {
    assert.equal(cleanVaultWrap(bad), null, JSON.stringify(bad));
  }
});

for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.stack ?? err}`);
}
const failed = results.filter((r) => r[0] === '✗').length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
