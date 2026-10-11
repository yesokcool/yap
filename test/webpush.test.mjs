// Web Push encryption checked against an independent decryptor written
// straight from RFC 8291 / RFC 8188 with node:crypto, plus the RFC's own
// example vector. Run: node --experimental-strip-types test/webpush.test.mjs
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import { encryptPayload, vapidPublicKey } from '../worker/webpush.ts';
import { b64urlDecode, b64urlEncode } from '../worker/encoding.ts';

const results = [];
async function test(name, fn) {
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(['✗', name, err]);
  }
}

/** What a browser does with a push message (RFC 8291 §3.4, receiver side). */
function decrypt(body, uaPrivate, uaPublic, authSecret) {
  const salt = body.subarray(0, 16);
  const idlen = body[20];
  const asPublic = body.subarray(21, 21 + idlen);
  const ciphertext = body.subarray(21 + idlen);
  const ecdh = crypto.createECDH('prime256v1');
  ecdh.setPrivateKey(uaPrivate);
  const shared = ecdh.computeSecret(asPublic);
  const ikm = Buffer.from(crypto.hkdfSync('sha256', shared, authSecret, Buffer.concat([Buffer.from('WebPush: info\0'), uaPublic, asPublic]), 32));
  const cek = Buffer.from(crypto.hkdfSync('sha256', ikm, salt, Buffer.from('Content-Encoding: aes128gcm\0'), 16));
  const nonce = Buffer.from(crypto.hkdfSync('sha256', ikm, salt, Buffer.from('Content-Encoding: nonce\0'), 12));
  const decipher = crypto.createDecipheriv('aes-128-gcm', cek, nonce);
  decipher.setAuthTag(ciphertext.subarray(ciphertext.length - 16));
  const plain = Buffer.concat([decipher.update(ciphertext.subarray(0, ciphertext.length - 16)), decipher.final()]);
  assert.equal(plain[plain.length - 1], 2, 'last-record delimiter');
  return plain.subarray(0, plain.length - 1).toString();
}

await test('round trip: a browser can read what we encrypt', async () => {
  const ua = crypto.createECDH('prime256v1');
  ua.generateKeys();
  const auth = crypto.randomBytes(16);
  const sub = { endpoint: 'https://push.example/x', p256dh: b64urlEncode(ua.getPublicKey()), auth: b64urlEncode(auth) };
  const message = JSON.stringify({ title: 'Ada mentioned you', body: 'hop on 🎮' });
  const body = Buffer.from(await encryptPayload(sub, new TextEncoder().encode(message)));
  assert.equal(new DataView(body.buffer, body.byteOffset).getUint32(16), 4096, 'record size');
  assert.equal(decrypt(body, ua.getPrivateKey(), ua.getPublicKey(), auth), message);
});

await test('RFC 8291 §5 example: same inputs give the RFC’s exact bytes', async () => {
  // Keys and secrets from RFC 8291 section 5.
  const uaPublic = 'BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4';
  const authSecret = 'BTBZMqHH6r4Tts7J_aSIgg';
  const asPrivate = 'yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw';
  const asPublic = 'BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8';
  const salt = 'DGv6ra1nlYgDCS1FRnbzlw';
  const expected = 'DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN';
  const pub = b64urlDecode(asPublic);
  const serverKeys = {
    privateKey: await crypto.webcrypto.subtle.importKey('jwk', {
      kty: 'EC', crv: 'P-256', d: asPrivate, x: b64urlEncode(pub.subarray(1, 33)), y: b64urlEncode(pub.subarray(33)),
    }, { name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits']),
    publicKey: await crypto.webcrypto.subtle.importKey('raw', pub, { name: 'ECDH', namedCurve: 'P-256' }, true, []),
  };
  const body = await encryptPayload(
    { endpoint: 'https://push.example/x', p256dh: uaPublic, auth: authSecret },
    new TextEncoder().encode('When I grow up, I want to be a watermelon'),
    { salt: b64urlDecode(salt), serverKeys },
  );
  assert.equal(b64urlEncode(body), expected);
});

await test('VAPID public key comes from the JWK', async () => {
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ec', { namedCurve: 'P-256' });
  const jwk = privateKey.export({ format: 'jwk' });
  const raw = publicKey.export({ format: 'der', type: 'spki' }).subarray(-65);
  assert.equal(vapidPublicKey(jwk), b64urlEncode(raw));
});

for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.message ?? err}`);
}
const failed = results.filter((r) => r[0] === '✗').length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
