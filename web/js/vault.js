/*
 * Encrypted Personal space, done in the browser (PROTOCOL.md › Encrypted
 * Personal space). The server only ever sees the wrapped key and ciphertext.
 *
 *  space key K   random AES-GCM-256, made once; each unlocked device keeps a
 *                non-extractable copy in IndexedDB
 *  wrap          PBKDF2-SHA256(passcode, salt, 600k) → KEK; AES-GCM(KEK, K)
 *                with AAD "yap:wrap:v1" — stored on the server
 *  note          "e1:" + b64url(iv ‖ AES-GCM(K, JSON {text, gif?, image?}, AAD "yap:note:v1"))
 *  picture       iv ‖ AES-GCM(K, bytes, AAD "yap:blob:v1"), uploaded as opaque bytes
 *
 * No library: WebCrypto only. Nothing here touches the DOM, so tests run it in Node.
 */

export const ITERATIONS = 600_000;
const enc = new TextEncoder();
const dec = new TextDecoder();
const AAD = { wrap: enc.encode('yap:wrap:v1'), note: enc.encode('yap:note:v1'), blob: enc.encode('yap:blob:v1') };

export const b64u = (bytes) => {
  let s = '';
  for (const b of new Uint8Array(bytes)) s += String.fromCharCode(b);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
};
export const unb64u = (text) => Uint8Array.from(atob(text.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((text.length + 3) % 4)), (c) => c.charCodeAt(0));

async function passcodeKey(passcode, salt, iterations) {
  const base = await crypto.subtle.importKey('raw', enc.encode(passcode.normalize('NFKC')), 'PBKDF2', false, ['deriveKey']);
  return crypto.subtle.deriveKey({ name: 'PBKDF2', hash: 'SHA-256', salt, iterations }, base, { name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
}

const importSpaceKey = (raw) => crypto.subtle.importKey('raw', raw, 'AES-GCM', false, ['encrypt', 'decrypt']);

async function wrapRaw(raw, passcode, iterations = ITERATIONS) {
  const salt = crypto.getRandomValues(new Uint8Array(16));
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const kek = await passcodeKey(passcode, salt, iterations);
  const ct = await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: AAD.wrap }, kek, raw);
  return { v: 1, iter: iterations, salt: b64u(salt), iv: b64u(iv), ct: b64u(ct) };
}

/** A brand-new space key, wrapped with `passcode`. */
export async function createVault(passcode, iterations = ITERATIONS) {
  const raw = crypto.getRandomValues(new Uint8Array(32));
  const wrap = await wrapRaw(raw, passcode, iterations);
  const key = await importSpaceKey(raw);
  raw.fill(0);
  return { wrap, key };
}

/** Unwrap with the passcode. Throws (`WrongPasscode`) if it doesn't fit. */
export async function openVault(wrap, passcode) {
  const raw = await unwrapRaw(wrap, passcode);
  const key = await importSpaceKey(raw);
  raw.fill(0);
  return key;
}

async function unwrapRaw(wrap, passcode) {
  const kek = await passcodeKey(passcode, unb64u(wrap.salt), wrap.iter);
  try {
    return new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv: unb64u(wrap.iv), additionalData: AAD.wrap }, kek, unb64u(wrap.ct)));
  } catch {
    throw Object.assign(new Error('That passcode doesn’t open it.'), { name: 'WrongPasscode' });
  }
}

/** Same key, new passcode (the notes don't change). */
export async function rewrapVault(wrap, oldPasscode, newPasscode) {
  const raw = await unwrapRaw(wrap, oldPasscode);
  const next = await wrapRaw(raw, newPasscode, Math.max(wrap.iter, ITERATIONS));
  raw.fill(0);
  return next;
}

export const isEncryptedNote = (body) => typeof body === 'string' && /^e1:[A-Za-z0-9_-]+$/.test(body);

/**
 * Sealed notes as `personal.seal` batches that each fit one WebSocket frame
 * (the server refuses frames over 384 KB, and batches of more than 50): at
 * most `maxBytes` of JSON and `maxNotes` notes per batch. A note too big for
 * any frame comes back in `tooBig`.
 * @param {{ id: string, body: string }[]} notes
 */
export function sealBatches(notes, { maxBytes = 300_000, maxNotes = 50 } = {}) {
  const batches = [];
  const tooBig = [];
  let batch = [];
  let bytes = 0;
  for (const note of notes) {
    const size = JSON.stringify(note).length + 1; // ASCII: ids and base64url bodies
    if (size > maxBytes) {
      tooBig.push(note);
      continue;
    }
    if (batch.length && (bytes + size > maxBytes || batch.length >= maxNotes)) {
      batches.push(batch);
      batch = [];
      bytes = 0;
    }
    batch.push(note);
    bytes += size;
  }
  if (batch.length) batches.push(batch);
  return { batches, tooBig };
}

export async function encryptNote(key, payload) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: AAD.note }, key, enc.encode(JSON.stringify(payload))));
  const out = new Uint8Array(12 + ct.length);
  out.set(iv);
  out.set(ct, 12);
  return `e1:${b64u(out)}`;
}

/** → {text, gif?, image?}; throws if this key can't open it. */
export async function decryptNote(key, body) {
  const bytes = unb64u(body.slice(3));
  const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: bytes.subarray(0, 12), additionalData: AAD.note }, key, bytes.subarray(12));
  return JSON.parse(dec.decode(plain));
}

export async function encryptBlob(key, data) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv, additionalData: AAD.blob }, key, data));
  const out = new Uint8Array(12 + ct.length);
  out.set(iv);
  out.set(ct, 12);
  return out;
}

export async function decryptBlob(key, data) {
  const bytes = new Uint8Array(data);
  return new Uint8Array(await crypto.subtle.decrypt({ name: 'AES-GCM', iv: bytes.subarray(0, 12), additionalData: AAD.blob }, key, bytes.subarray(12)));
}

// ── This device's copy of the key (IndexedDB; the key can't be read back out) ──

function idb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open('yap-vault', 1);
    req.onupgradeneeded = () => req.result.createObjectStore('keys');
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function tx(mode, run) {
  const db = await idb();
  return new Promise((resolve, reject) => {
    const t = db.transaction('keys', mode);
    const req = run(t.objectStore('keys'));
    t.oncomplete = () => resolve(req?.result);
    t.onerror = () => reject(t.error);
  });
}

/** `{key, ct}`: ct = which wrap it came from, so a reset elsewhere is noticed. */
export const rememberKey = (uid, key, ct) => tx('readwrite', (s) => s.put({ key, ct }, uid)).catch(() => {});
export const recallKey = (uid) => tx('readonly', (s) => s.get(uid)).catch(() => null);
export const forgetKey = (uid) => tx('readwrite', (s) => s.delete(uid)).catch(() => {});
