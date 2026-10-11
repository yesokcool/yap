/**
 * Web Push from the Worker, no libraries: payload encryption per RFC 8291
 * (aes128gcm, RFC 8188) and VAPID (RFC 8292) with WebCrypto.
 *
 * The VAPID private key is the Worker secret VAPID_PRIVATE_JWK (an EC P-256
 * JWK, made by tools/make-vapid.mjs). Its public half is what browsers subscribe
 * with (served in /api/config as `push.publicKey`).
 */

import { b64urlDecode, b64urlEncode } from './encoding.ts';

interface PushSubscriptionKeys {
  endpoint: string;
  p256dh: string; // base64url, the browser's ECDH public key (65 bytes)
  auth: string; // base64url, 16-byte auth secret
}

const enc = new TextEncoder();

const concat = (...parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let i = 0;
  for (const p of parts) {
    out.set(p, i);
    i += p.length;
  }
  return out;
};

async function hkdf(salt: Uint8Array, ikm: Uint8Array, info: Uint8Array, length: number): Promise<Uint8Array> {
  const key = await crypto.subtle.importKey('raw', ikm, 'HKDF', false, ['deriveBits']);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: 'HKDF', hash: 'SHA-256', salt, info }, key, length * 8));
}

/** The VAPID public key (uncompressed point, base64url) for a private JWK. */
export function vapidPublicKey(jwk: JsonWebKey): string {
  return b64urlEncode(concat(new Uint8Array([4]), b64urlDecode(jwk.x!), b64urlDecode(jwk.y!)));
}

/**
 * Encrypt `payload` for one subscription (RFC 8291). `salt` and `serverKeys`
 * are only passed in by tests; normally they're fresh per message.
 */
export async function encryptPayload(
  sub: PushSubscriptionKeys,
  payload: Uint8Array,
  opts: { salt?: Uint8Array; serverKeys?: CryptoKeyPair } = {},
): Promise<Uint8Array> {
  const uaPublic = b64urlDecode(sub.p256dh);
  const authSecret = b64urlDecode(sub.auth);
  const serverKeys = opts.serverKeys ?? ((await crypto.subtle.generateKey({ name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits'])) as CryptoKeyPair);
  const asPublic = new Uint8Array((await crypto.subtle.exportKey('raw', serverKeys.publicKey)) as ArrayBuffer);
  const uaKey = await crypto.subtle.importKey('raw', uaPublic, { name: 'ECDH', namedCurve: 'P-256' }, false, []);
  // Standard WebCrypto spelling (`public`); Cloudflare's typings call it `$public`.
  const ecdh = { name: 'ECDH', public: uaKey } as unknown as SubtleCryptoDeriveKeyAlgorithm;
  const shared = new Uint8Array(await crypto.subtle.deriveBits(ecdh, serverKeys.privateKey, 256));
  // IKM = HKDF(auth secret, ECDH secret, "WebPush: info" ‖ 0 ‖ ua_public ‖ as_public)
  const ikm = await hkdf(authSecret, shared, concat(enc.encode('WebPush: info\0'), uaPublic, asPublic), 32);
  const salt = opts.salt ?? crypto.getRandomValues(new Uint8Array(16));
  const cek = await hkdf(salt, ikm, enc.encode('Content-Encoding: aes128gcm\0'), 16);
  const nonce = await hkdf(salt, ikm, enc.encode('Content-Encoding: nonce\0'), 12);
  // One record: the payload plus the 0x02 "last record" delimiter.
  const key = await crypto.subtle.importKey('raw', cek, 'AES-GCM', false, ['encrypt']);
  const sealed = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv: nonce }, key, concat(payload, new Uint8Array([2]))));
  // Header: salt (16) ‖ record size (4, big-endian) ‖ key id length (1) ‖ key id (as_public).
  const header = new Uint8Array(16 + 4 + 1);
  header.set(salt, 0);
  new DataView(header.buffer).setUint32(16, 4096);
  header[20] = asPublic.length;
  return concat(header, asPublic, sealed);
}

/** A VAPID JWT (ES256) for the push service at `endpoint`. */
async function vapidJwt(endpoint: string, jwk: JsonWebKey, subject: string): Promise<string> {
  const header = b64urlEncode(enc.encode(JSON.stringify({ typ: 'JWT', alg: 'ES256' })));
  const claims = b64urlEncode(enc.encode(JSON.stringify({ aud: new URL(endpoint).origin, exp: Math.floor(Date.now() / 1000) + 12 * 3600, sub: subject })));
  const key = await crypto.subtle.importKey('jwk', { ...jwk, key_ops: ['sign'] }, { name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign']);
  const signature = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key, enc.encode(`${header}.${claims}`)));
  return `${header}.${claims}.${b64urlEncode(signature)}`;
}

/**
 * Send one push. Returns the push service's HTTP status: 201 = sent;
 * 404/410 = the subscription is gone (delete it).
 */
export async function sendWebPush(sub: PushSubscriptionKeys, message: object, jwkText: string, subject: string): Promise<number> {
  const jwk = JSON.parse(jwkText) as JsonWebKey;
  const body = await encryptPayload(sub, enc.encode(JSON.stringify(message)));
  const res = await fetch(sub.endpoint, {
    method: 'POST',
    headers: {
      'Content-Encoding': 'aes128gcm',
      'Content-Type': 'application/octet-stream',
      TTL: '86400',
      Urgency: 'high',
      Authorization: `vapid t=${await vapidJwt(sub.endpoint, jwk, subject)}, k=${vapidPublicKey(jwk)}`,
    },
    body,
  });
  return res.status;
}
