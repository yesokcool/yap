// What clients send, checked: anything that ends up stored or relayed goes
// through one of these (or throws a ClientError). Pure functions with no
// Durable Object state, so test/worker.test.mjs runs them in Node (hence the
// `.ts` on imports).
import { LIMITS, type Channel, type CursorAnchor, type Gif, type ImageRef, type Stroke, type VaultWrap } from './protocol.ts';

/** A refusal the client sees as `error { code, message }`. */
export class ClientError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

export function cleanName(value: unknown, max: number): string {
  if (typeof value !== 'string') return '';
  // Strip control characters, collapse whitespace, cap length.
  return value.replace(/[\p{Cc}\p{Cf}]/gu, '').replace(/\s+/g, ' ').trim().slice(0, max);
}

export const COLOR_RE = /^#[0-9a-fA-F]{6}$/;
const STROKE_ID_RE = /^[A-Za-z0-9_-]{6,40}$/;
/** A stored file's id: the first 128 bits of its SHA-256, in hex. */
export const MEDIA_ID_RE = /^[a-f0-9]{32}$/;
// Only KLIPY's media CDN. Anything else could be a tracking pixel that logs
// every friend's IP the moment the message renders.
const KLIPY_MEDIA_RE = /^https:\/\/static\d*\.klipy\.com\/[^\s"'<>\\]{1,400}$/;

export const ID_RE = /^[A-Za-z0-9_-]{1,64}$/;
/** Session ids are random UUIDs. */
export const SID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export const YOUTUBE_ID_RE = /^[A-Za-z0-9_-]{11}$/;
export const EMOJI_NAME_RE = /^[A-Za-z0-9_]{2,32}$/;
export const CUSTOM_EMOJI_RE = /^<:[A-Za-z0-9_]{2,32}:[a-f0-9]{32}>$/;

const UNICODE_EMOJI_RE = /^(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|\p{Emoji_Modifier}|\p{Emoji_Component}|\u200d|\ufe0f){1,16}$/u;
export const isEmoji = (s: unknown): s is string =>
  typeof s === 'string' && s.length <= 64 && (CUSTOM_EMOJI_RE.test(s) || (UNICODE_EMOJI_RE.test(s) && /\p{Extended_Pictographic}|\p{Regional_Indicator}/u.test(s)));

/** Everyone's Personal space is the channel `p_<uid>` (no row in `channels`). */
export const personalId = (uid: string) => `p_${uid}`;
export const isPersonal = (id: unknown): id is string => typeof id === 'string' && id.startsWith('p_');

/** Encrypted notes ("e1:…", Personal space only) may be longer: base64 + IV + tag + JSON. */
export const ENCRYPTED_NOTE_RE = /^e1:[A-Za-z0-9_-]+$/;
export const bodyLimit = (channel: Channel | undefined, body: string) =>
  channel?.kind === 'personal' && ENCRYPTED_NOTE_RE.test(body) ? LIMITS.bodyChars * 6 : LIMITS.bodyChars;
export const tooLong = (channel: Channel | undefined, body: string) => new ClientError('too_long', bodyLimit(channel, body) > LIMITS.bodyChars
  ? 'That note is too long to keep encrypted. Try splitting it in two.'
  : `Messages can be up to ${LIMITS.bodyChars.toLocaleString('en-US')} characters.`);

const B64URL = (bytes: number) => new RegExp(`^[A-Za-z0-9_-]{${Math.ceil((bytes * 4) / 3)}}$`);
export function cleanVaultWrap(value: unknown): VaultWrap | null {
  if (typeof value !== 'object' || value === null) return null;
  const v = value as Record<string, unknown>;
  const iter = Number(v.iter);
  if (v.v !== 1 || !(iter >= 100_000 && iter <= 10_000_000)) return null;
  if (typeof v.salt !== 'string' || !B64URL(16).test(v.salt)) return null;
  if (typeof v.iv !== 'string' || !B64URL(12).test(v.iv)) return null;
  if (typeof v.ct !== 'string' || !B64URL(48).test(v.ct)) return null;
  return { v: 1, iter, salt: v.salt, iv: v.iv, ct: v.ct };
}

/** Validate a client-supplied GIF; throws on anything off. */
export function cleanGif(value: unknown): Gif {
  const bad = (why: string) => new ClientError('bad_gif', `GIF rejected: ${why}.`);
  if (typeof value !== 'object' || value === null) throw bad('not an object');
  const g = value as Record<string, unknown>;
  if (g.provider !== 'klipy') throw bad('unknown provider');
  if (typeof g.slug !== 'string' || !/^[A-Za-z0-9_-]{1,200}$/.test(g.slug)) throw bad('bad slug');
  const dim = (v: unknown) => (Number.isInteger(v) && (v as number) >= 1 && (v as number) <= 4096 ? (v as number) : 0);
  const width = dim(g.width);
  const height = dim(g.height);
  if (!width || !height) throw bad('bad size');
  const gif: Gif = { provider: 'klipy', slug: g.slug, title: cleanName(g.title, 200), width, height };
  for (const key of ['mp4', 'webm', 'webp', 'gif', 'still'] as const) {
    const url = g[key];
    if (url === undefined) continue;
    if (typeof url !== 'string' || !KLIPY_MEDIA_RE.test(url)) throw bad(`${key} must be a KLIPY media URL`);
    gif[key] = url;
  }
  if (!gif.mp4 && !gif.webm && !gif.webp && !gif.gif) throw bad('no media');
  return gif;
}

export function cleanStroke(value: unknown, maxNumbers: number): Omit<Stroke, 'uid'> | null {
  if (typeof value !== 'object' || value === null) return null;
  const v = value as Record<string, unknown>;
  if (typeof v.sk !== 'string' || !STROKE_ID_RE.test(v.sk)) return null;
  if (typeof v.c !== 'string' || !COLOR_RE.test(v.c)) return null;
  const w = Math.round(Number(v.w));
  if (!(w >= 1 && w <= 400)) return null;
  if (!Array.isArray(v.p) || v.p.length < 2 || v.p.length > maxNumbers || v.p.length % 2) return null;
  const p = v.p.map((n) => Math.round(Number(n)));
  if (p.some((n) => !(n >= -500 && n <= 10500))) return null;
  const stroke: Omit<Stroke, 'uid'> = { id: v.sk, c: v.c.toLowerCase(), w, e: v.e === true, p };
  if (v.l !== undefined && v.l !== 0) {
    if (!Number.isInteger(v.l) || (v.l as number) < 0 || (v.l as number) >= LIMITS.drawLayers) return null;
    stroke.l = v.l as number;
  }
  if (v.k === 'poly') stroke.k = 'poly';
  if (v.k === 'text') {
    const text = typeof v.text === 'string' ? v.text.replace(/[\u0000-\u001f\u007f]/g, ' ').trim().slice(0, 200) : '';
    if (!text || p.length !== 2) return null;
    Object.assign(stroke, { k: 'text', text });
  }
  return stroke;
}

/** A picture we stored: a content id and sane dimensions. */
export function cleanImage(value: unknown): ImageRef | null {
  if (typeof value !== 'object' || value === null) return null;
  const v = value as Record<string, unknown>;
  const width = Math.round(Number(v.width));
  const height = Math.round(Number(v.height));
  if (typeof v.id !== 'string' || !MEDIA_ID_RE.test(v.id)) return null;
  if (!(width >= 1 && width <= 8000 && height >= 1 && height <= 8000)) return null;
  return { id: v.id, width, height };
}

export function cleanAnchor(value: unknown): CursorAnchor | null {
  if (typeof value !== 'object' || value === null) return null;
  const v = value as Record<string, unknown>;
  const k = v.k === 'card' || v.k === 'lane' || v.k === 'page' ? v.k : null;
  const x = Number(v.x);
  const y = Number(v.y);
  if (!k || !Number.isFinite(x) || !Number.isFinite(y)) return null;
  const anchor: CursorAnchor = { k, x: clamp(x, -0.5, 1.5), y: clamp(y, -0.5, 1.5) };
  if (k !== 'page') {
    if (typeof v.id !== 'string' || !/^[A-Za-z0-9_-]{1,64}$/.test(v.id)) return null;
    anchor.id = v.id;
  }
  return anchor;
}

export function clamp(n: number, lo: number, hi: number) {
  return Math.min(hi, Math.max(lo, n));
}
