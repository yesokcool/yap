// Accounts: a name + a password (8–128 characters), and per-device tokens.
// Accounts from before 2026-10-11 may still have a 6-digit PIN, which signs
// in until they choose a password; nothing new can be set as a PIN.
//
// The real protection against guessing is the sign-in lockout in Hub (few
// tries, growing waits); the hash means a leaked database doesn't hand out
// passwords. Device tokens are 256-bit random secrets; only their SHA-256 is
// stored.

import { b64urlDecode as unb64, b64urlEncode as b64, hex } from './encoding.ts';
import COMMON_PASSWORDS from './common-passwords.ts';
import { WORDS } from './words.ts';

// Current scheme: PBKDF2-SHA256, 100k iterations (the Workers maximum), over
// the secret *plus a server secret* (PIN_PEPPER). The pepper isn't in the
// database or backups, so a leaked backup can't be brute-forced offline.
// Older hashes ("pbkdf2-sha256", 20k, no pepper) still verify and are
// upgraded on the next successful sign-in.
const ITERATIONS = 100_000;
const SCHEME = 'pbkdf2-sha256-pepper';
/** An old 6-digit PIN (still signs in; never set any more). */
export const PIN_RE = /^\d{6}$/;
/** Length in characters (code points, so an emoji is one). */
const chars = (s: string) => [...s].length;
/** A password: 8–128 characters. Sign-up, Change password and resets only make these. */
export const isPassword = (s: string) => chars(s) >= 8 && chars(s) <= 128;
/** What sign-in accepts: a password, or an old PIN. */
export const isSecret = (s: string) => PIN_RE.test(s) || isPassword(s);
/** Why a new password isn't allowed (bad_pin), or null. */
export const passwordRule = (s: string): string | null =>
  chars(s) < 8 ? 'Use a password of at least 8 characters.' : chars(s) > 128 ? 'Use a password of at most 128 characters.' : null;

/** What a temporary password from a reset looks like: "maple-river-stone-42". */
export const TEMP_RE = /^[a-z]+-[a-z]+-[a-z]+-\d{2}$/;

const TOO_EASY = 'That password is too easy to guess.';
let common: Set<string> | null = null;
const isCommon = (s: string) => (common ??= new Set(COMMON_PASSWORDS.split('\n'))).has(s);

/**
 * Keyboard rows and columns (US, German, French), counting and the alphabet.
 * A password made only of runs along these, either way, is guessable.
 */
const RUNS = [
  '01234567890', '02468', '13579', 'abcdefghijklmnopqrstuvwxyz',
  '`1234567890-=', 'qwertyuiop[]\\', "asdfghjkl;'", 'zxcvbnm,./',
  'qwertzuiop', 'yxcvbnm', 'azertyuiop', 'qsdfghjklm', 'wxcvbn',
  '1qaz2wsx3edc4rfv5tgb6yhn7ujm8ik,9ol.0p;/', 'zaq1xsw2cde3vfr4bgt5nhy6mju7,ki8.lo9/;p0',
  '1q2w3e4r5t6y7u8i9o0p', 'q1w2e3r4t5y6u7i8o9p0', '789456123', '147258369',
].flatMap((r) => [r, [...r].reverse().join('')]);
const isRun = (s: string) => RUNS.some((r) => r.includes(s));

/**
 * Is `s` nothing but runs: each piece at least 3 long ("qwerty", "abc",
 * "aaa"), or 2 counting digits ("qwerty12")?
 */
function onlyRuns(s: string): boolean {
  const ok = (piece: string) => piece.length >= 3 ? isRun(piece) || /^(.)\1+$/.test(piece) : piece.length === 2 && /^\d\d$/.test(piece) && isRun(piece);
  const reach = new Array<boolean>(s.length + 1).fill(false);
  reach[0] = true;
  for (let i = 0; i < s.length; i++) {
    if (!reach[i]) continue;
    for (let j = i + 2; j <= s.length; j++) {
      const piece = s.slice(i, j);
      if (ok(piece)) reach[j] = true;
      // Longer pieces only fit if this one does (a run's pieces are runs), except "12" → "123".
      else if (piece.length >= 3) break;
    }
  }
  return reach[s.length];
}

/** Letters and digits only, lowercased: "Ada Lovelace!" → "adalovelace". */
const squash = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]/gu, '');

/**
 * "cleo", "cleo2024", "2024cleo", "cleocleo", "Cleo-12345", "lovelace1815"
 * for Ada Lovelace: your name (or a part of it) with little else around it.
 */
function mostlyName(s: string, name: string): boolean {
  const core = squash(s);
  const parts = [squash(name), ...name.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter((w) => w.length >= 3)]
    .filter(Boolean)
    .sort((x, y) => y.length - x.length);
  let rest = core;
  for (const part of parts) rest = rest.split(part).join('');
  return rest !== core && (rest.length <= 3 || /^\d+$/.test(rest) || onlyRuns(rest));
}

/**
 * Why a new password is too easy to guess (for a stranger or for someone who
 * knows you), or null: one of the most common passwords (even with digits or
 * symbols around it), your own name, one thing repeated, keyboard rows and
 * counting.
 */
export function weakPassword(secret: string, name = ''): string | null {
  const s = secret.toLowerCase();
  const core = s.replace(/^[^\p{L}]+|[^\p{L}]+$/gu, ''); // "!!sunshine2024" → "sunshine"
  if (isCommon(s) || isCommon(core)) return `${TOO_EASY} It’s one of the most common passwords.`;
  if (name && mostlyName(s, name)) return `${TOO_EASY} It’s mostly your name.`;
  if (/^(.{1,4})\1+$/su.test(s)) return `${TOO_EASY} It’s the same thing over and over.`;
  if (onlyRuns(s)) return `${TOO_EASY} It’s keys in a row or counting.`;
  return null;
}

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

/**
 * A temporary password for an owner/mod reset: three short words and two
 * digits ("maple-river-stone-42"), easy to read out and to type on a phone.
 */
export function tempPassword(): string {
  for (;;) {
    const r = crypto.getRandomValues(new Uint32Array(4));
    const words = [r[0], r[1], r[2]].map((n) => WORDS[n % WORDS.length]);
    if (new Set(words).size < 3) continue;
    const password = `${words.join('-')}-${String(r[3] % 100).padStart(2, '0')}`;
    if (!weakPassword(password)) return password;
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
