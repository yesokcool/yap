// Accounts: a name + 6-digit PIN, and per-device tokens.
//
// A PIN is only 1,000,000 guesses, so the real protection is the login
// lockout in Hub (few tries, growing waits); the hash just means a leaked
// database doesn't hand out PINs in plain text. Device tokens are 256-bit
// random secrets; only their SHA-256 is stored.

import { b64urlDecode as unb64, b64urlEncode as b64, hex } from './encoding.ts';

// Current scheme: PBKDF2-SHA256, 100k iterations (the Workers maximum), over
// the PIN *plus a server secret* (PIN_PEPPER). The pepper isn't in the
// database or backups, so a leaked backup can't be brute-forced offline.
// Older hashes ("pbkdf2-sha256", 20k, no pepper) still verify and are
// upgraded on the next successful sign-in.
const ITERATIONS = 100_000;
const SCHEME = 'pbkdf2-sha256-pepper';
export const PIN_RE = /^\d{6}$/;
/** Instead of a PIN, a password: 10–128 characters. */
const isPassword = (s: string) => s.length >= 10 && s.length <= 128;
export const isSecret = (s: string) => PIN_RE.test(s) || isPassword(s);

/**
 * Why a PIN/password is too easy to guess (for someone who knows you), or
 * null. Six digits are only safe because of the lockout, so the guessable
 * ones are refused: repeats, runs, patterns, common PINs and dates.
 */
export function weakSecret(secret: string, name = ''): string | null {
  if (isPassword(secret) && !PIN_RE.test(secret)) {
    const s = secret.toLowerCase();
    if (/^(.)\1+$/.test(s) || /^(?:0123456789|1234567890|9876543210|qwertyuiop|asdfghjkl)/.test(s)) return 'That password is too easy to guess.';
    if (name && s.includes(name.toLowerCase())) return 'Don’t put your name in your password.';
    if (/^(password|letmein|iloveyou|yapyapyap)/.test(s)) return 'That password is too easy to guess.';
    return null;
  }
  if (!PIN_RE.test(secret)) return null;
  const d = [...secret].map(Number);
  const steps = d.slice(1).map((x, i) => (x - d[i] + 10) % 10);
  if (steps.every((s) => s === 0)) return 'All the same digit is the first thing anyone tries.';
  if (steps.every((s) => s === 1) || steps.every((s) => s === 9)) return 'Counting up or down is the first thing anyone tries.';
  if (secret.slice(0, 2).repeat(3) === secret || secret.slice(0, 3).repeat(2) === secret) return 'Repeating patterns are easy to guess.';
  if (secret.slice(0, 3) === [...secret.slice(3)].reverse().join('')) return 'Mirrored PINs are easy to guess.';
  if (/^(?:(\d)\1){3}$/.test(secret)) return 'Doubled digits are easy to guess.';
  if (COMMON_PINS.has(secret)) return 'That’s one of the most common PINs.';
  const day = (x: string) => Number(x) >= 1 && Number(x) <= 31;
  const month = (x: string) => Number(x) >= 1 && Number(x) <= 12;
  const [a, b, c] = [secret.slice(0, 2), secret.slice(2, 4), secret.slice(4, 6)];
  if ((day(a) && month(b)) || (month(a) && day(b)) || (month(b) && day(c))) return 'That looks like a date (a birthday?) — a friend could guess it.';
  return null;
}
const COMMON_PINS = new Set(['159753', '147258', '258369', '963852', '741852', '852456', '159357', '753951', '246810', '135790', '135791', '102030', '100200', '420420', '696969', '520520', '123654', '147852', '258456', '369258', '789456', '456123', '321654', '112233', '332211', '111222', '000111', '999000', '090909']);


async function derive(pin: string, salt: Uint8Array, iterations: number): Promise<Uint8Array> {
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(pin), 'PBKDF2', false, ['deriveBits']);
  const bits = await crypto.subtle.deriveBits({ name: 'PBKDF2', hash: 'SHA-256', salt, iterations }, key, 256);
  return new Uint8Array(bits);
}

export async function hashPin(pin: string, pepper: string): Promise<string> {
  const salt = crypto.getRandomValues(new Uint8Array(16));
  return `${SCHEME}$${ITERATIONS}$${b64(salt)}$${b64(await derive(`${pepper}\u0000${pin}`, salt, ITERATIONS))}`;
}

/** True when a stored hash should be replaced with the current scheme. */
export const needsRehash = (stored: string | null) => Boolean(stored) && !stored!.startsWith(`${SCHEME}$${ITERATIONS}$`);

export async function verifyPin(pin: string, stored: string | null, pepper: string): Promise<boolean> {
  if (!stored || !isSecret(pin)) return false;
  const [scheme, iter, salt, hash] = stored.split('$');
  if (scheme !== SCHEME && scheme !== 'pbkdf2-sha256') return false;
  const secret = scheme === SCHEME ? `${pepper}\u0000${pin}` : pin;
  const actual = await derive(secret, unb64(salt), Number(iter));
  const expected = unb64(hash);
  return actual.byteLength === expected.byteLength && crypto.subtle.timingSafeEqual(actual, expected);
}

export function newToken(): string {
  return b64(crypto.getRandomValues(new Uint8Array(32)));
}

export async function tokenHash(token: string): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(token)));
  return hex(digest);
}

/** A random 6-digit PIN (for owner/mod resets). */
export function randomPin(): string {
  for (;;) {
    const n = crypto.getRandomValues(new Uint32Array(1))[0] % 1_000_000;
    const pin = String(n).padStart(6, '0');
    if (!weakSecret(pin)) return pin;
  }
}

/** What the Apple app says it is in `hello.client` ("macos" from older Mac builds; older iPad builds said "ios"). */
const APPLE_APPS = new Map([['ios', 'Yap for iPhone'], ['ipad', 'Yap for iPad'], ['mac', 'Yap for Mac'], ['macos', 'Yap for Mac']]);

/**
 * "Chrome on Windows", "Safari on iPhone", "Desktop app"… from the client's
 * `hello.client` and its User-Agent. Any other client value is fine: the
 * label then comes from the User-Agent alone.
 */
export function deviceLabel(ua: string, client: string): string {
  if (client === 'desktop') return /Windows/i.test(ua) ? 'Desktop app (Windows)' : /Mac/i.test(ua) ? 'Desktop app (Mac)' : /Linux/i.test(ua) ? 'Desktop app (Linux)' : 'Desktop app';
  const apple = APPLE_APPS.get(client);
  if (apple) return apple;
  // An Apple app under a name we don't know yet: its networking stack gives it away.
  if (/CFNetwork|Darwin/.test(ua)) return 'Yap for iPhone';
  const browser = /Edg\//.test(ua) ? 'Edge' : /Firefox\//.test(ua) ? 'Firefox' : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : 'A browser';
  const os = /iPhone/.test(ua) ? 'iPhone' : /iPad/.test(ua) ? 'iPad' : /Android/.test(ua) ? 'Android' : /Mac OS X/.test(ua) ? 'Mac' : /Windows/.test(ua) ? 'Windows' : /Linux/.test(ua) ? 'Linux' : '';
  return os ? `${browser} on ${os}` : browser;
}
