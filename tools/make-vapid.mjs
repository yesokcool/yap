#!/usr/bin/env node
// A new key for browser push notifications (Web Push / VAPID): an EC P-256
// private key as JWK, printed for `wrangler secret put` to read:
//   node tools/make-vapid.mjs | npx wrangler secret put VAPID_PRIVATE_JWK
// Keep a copy somewhere safe. A new key means everyone turns pushes on again.
const { privateKey } = await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign', 'verify']);
const { kty, crv, x, y, d } = await crypto.subtle.exportKey('jwk', privateKey);
process.stdout.write(JSON.stringify({ kty, crv, x, y, d }));
