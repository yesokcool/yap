// The Worker's pure rules, no server needed: accounts (worker/auth.ts), what
// clients send (worker/validate.ts). (Mentions: web/js/mentions.js, in test/unit.test.mjs.)
//   node --experimental-strip-types --no-warnings test/worker.test.mjs
import assert from 'node:assert/strict';
import nodeCrypto from 'node:crypto';
import { deviceLabel, hashPin, isSecret, needsRehash, randomPin, verifyPin, weakSecret } from '../worker/auth.ts';
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
await test('guessable PINs are refused: repeats, runs, patterns, mirrors, common ones, dates', () => {
  for (const pin of ['111111', '000000', '123456', '654321', '890123', '121212', '123123', '123321', '112233', '159753', '251299', '010203', '311299']) {
    assert.ok(weakSecret(pin), pin);
  }
  for (const pin of ['582039', '482916', '246802', '730964']) assert.equal(weakSecret(pin), null, pin);
  assert.equal(weakSecret('purple tiger river 7'), null, 'a decent password');
  assert.ok(weakSecret('aaaaaaaaaaaa'));
  assert.ok(weakSecret('password1234'));
  assert.ok(weakSecret('my name is samantha!', 'Samantha'), 'no name in the password');
});

await test('a secret is 6 digits or a 10–128 character password', () => {
  for (const ok of ['582039', 'purple tiger river 7', 'x'.repeat(128)]) assert.equal(isSecret(ok), true, ok);
  for (const bad of ['58203', '5820391', 'short pw', 'x'.repeat(129), '']) assert.equal(isSecret(bad), false, bad);
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

await test('temporary PINs are 6 digits and never guessable', () => {
  for (let i = 0; i < 300; i++) {
    const pin = randomPin();
    assert.match(pin, /^\d{6}$/);
    assert.equal(weakSecret(pin), null, pin);
  }
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
