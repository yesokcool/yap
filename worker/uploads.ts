// Bytes in the Media store: the upload routes (/api/emoji, /api/image,
// /api/avatar, /api/drawing, /api/blob, /api/sound, /api/doodle) and serving
// them back (/emoji/<id>, /media/<id>). Every upload is checked by its bytes
// and stored under its content id. The router (index.ts) checks the method
// and the invite key first.
import { LIMITS } from './protocol';
import { checkWav } from './extras';
import { hex } from './encoding';
import { hub, media } from './objects';
import { EMOJI_NAME_RE } from './validate';

/** Pictures and drawings. Each is one SQLite row in the Media object, and rows cap at 2 MB. */
export const MAX_PICTURE = 1_900_000;

export const fail = (status: number, code: string, message: string) => Response.json({ ok: false, code, message }, { status });

export async function uploadEmoji(request: Request, url: URL, env: Env, asAdmin = false): Promise<Response> {
  const name = url.searchParams.get('name') ?? '';
  const sid = url.searchParams.get('sid') ?? '';
  const ut = url.searchParams.get('ut') ?? '';
  if (!EMOJI_NAME_RE.test(name)) return fail(400, 'bad_name', 'Emoji names are 2–32 letters, numbers or underscores.');
  // A live session first: nothing is stored for anyone else (waiting room, a leaked link).
  if (!asAdmin && !(await hub(env).isUploader(sid, ut))) return fail(401, 'no_session', 'Connect first, then upload.');
  const stored = await storeImage(request, env, LIMITS.emojiBytes);
  if (stored instanceof Response) return stored;
  const { id, mime } = stored;
  const result = await hub(env).addEmoji({ id, name, mime, sid, ut, asAdmin });
  return result.ok ? Response.json(result) : fail(result.status, result.code, result.message);
}

// Your avatar / a saved drawing / a picture for a message: POST image
// bytes (same rules as emojis). Only from a signed-in session, checked
// before anything is stored.
export async function uploadImage(request: Request, url: URL, env: Env): Promise<Response> {
  const sid = url.searchParams.get('sid') ?? '';
  const ut = url.searchParams.get('ut') ?? '';
  if (!(await hub(env).canUpload(sid, ut))) return fail(403, 'no_session', 'Connect first (or slow down a little).');
  const stored = await storeImage(request, env, url.pathname === '/api/avatar' ? LIMITS.emojiBytes : MAX_PICTURE);
  if (stored instanceof Response) return stored;
  if (url.pathname === '/api/image') return Response.json({ ok: true, ...stored });
  const result = url.pathname === '/api/avatar'
    ? await hub(env).setAvatar({ sid, ut, media: stored.id })
    : await hub(env).addDrawing({ sid, ut, media: stored.id });
  return Response.json(result, { status: result.ok ? 200 : 400 });
}

// Encrypted bytes (Personal space pictures): stored as-is, opaque to us.
export async function uploadBlob(request: Request, url: URL, env: Env): Promise<Response> {
  if (!(await hub(env).canUpload(url.searchParams.get('sid') ?? '', url.searchParams.get('ut') ?? ''))) return fail(403, 'no_session', 'Connect first (or slow down a little).');
  if (Number(request.headers.get('Content-Length') ?? 0) > MAX_PICTURE + 28) return fail(413, 'too_big', 'Too big.');
  const data = await request.arrayBuffer();
  // + 28: an encrypted picture is the original plus its IV and tag.
  if (data.byteLength > MAX_PICTURE + 28 || data.byteLength < 29) return fail(413, 'too_big', 'Too big.');
  const id = await contentId(data);
  await media(env).put(id, 'application/octet-stream', data);
  return Response.json({ ok: true, id });
}

// A soundboard clip or your own join chime: a small WAV the client made
// (PROTOCOL.md › Extras › Soundboard). Loudness is measured here.
export async function uploadSound(request: Request, url: URL, env: Env): Promise<Response> {
  const sid = url.searchParams.get('sid') ?? '';
  const ut = url.searchParams.get('ut') ?? '';
  if (!(await hub(env).canUpload(sid, ut))) return fail(403, 'no_session', 'Connect first (or slow down a little).');
  if (Number(request.headers.get('Content-Length') ?? 0) > LIMITS.soundBytes) return fail(413, 'too_big', 'Clips max out at 480 KB.');
  const data = await request.arrayBuffer();
  if (data.byteLength > LIMITS.soundBytes) return fail(413, 'too_big', 'Clips max out at 480 KB.');
  const kind = url.searchParams.get('kind') === 'chime' ? 'chime' : 'board';
  const clip = checkWav(new Uint8Array(data), kind === 'chime' ? LIMITS.chimeMs : LIMITS.soundMs);
  if (typeof clip === 'string') return fail(415, 'bad_sound', clip);
  const id = await contentId(data);
  await media(env).put(id, 'audio/wav', data);
  const result = await hub(env).addSound({ sid, ut, kind, name: url.searchParams.get('name') ?? '', emoji: url.searchParams.get('emoji') ?? '', media: id, ...clip });
  return Response.json(result, { status: result.ok ? 200 : 400 });
}

// A doodle reaction: a tiny PNG/GIF (≤ 64 KB, ≤ 256 × 256).
export async function uploadDoodle(request: Request, url: URL, env: Env): Promise<Response> {
  if (!(await hub(env).canUpload(url.searchParams.get('sid') ?? '', url.searchParams.get('ut') ?? ''))) return fail(403, 'no_session', 'Connect first (or slow down a little).');
  if (Number(request.headers.get('Content-Length') ?? 0) > LIMITS.doodleBytes) return fail(413, 'too_big', 'Doodles max out at 64 KB.');
  const data = await request.arrayBuffer();
  if (data.byteLength > LIMITS.doodleBytes) return fail(413, 'too_big', 'Doodles max out at 64 KB.');
  const bytes = new Uint8Array(data);
  const mime = sniffImage(bytes);
  const size = mime === 'image/png' || mime === 'image/gif' ? imageSize(bytes) : null;
  if (!size) return fail(415, 'bad_type', 'Doodles are PNG or GIF.');
  if (size.width > LIMITS.doodlePx || size.height > LIMITS.doodlePx) return fail(400, 'too_big', `Doodles are at most ${LIMITS.doodlePx} × ${LIMITS.doodlePx}.`);
  const id = await contentId(data);
  await media(env).put(id, mime!, data);
  return Response.json({ ok: true, id });
}

/** Validate image bytes (sniffed, size-capped) and store them content-addressed in Media. */
async function storeImage(request: Request, env: Env, maxBytes: number): Promise<{ id: string; mime: string } | Response> {
  const tooBig = fail(413, 'too_big', `Images max out at ${Math.round(maxBytes / 1024)} KB.`);
  if (Number(request.headers.get('Content-Length') ?? 0) > maxBytes) return tooBig;
  const data = await request.arrayBuffer();
  if (data.byteLength > maxBytes) return tooBig;
  const mime = sniffImage(new Uint8Array(data));
  if (!mime) return fail(415, 'bad_type', 'Use a PNG, GIF, WebP or JPEG.');
  const id = await contentId(data);
  await media(env).put(id, mime, data);
  return { id, mime };
}

/** Media ids are the first 128 bits of the bytes' SHA-256, in hex. */
export async function contentId(data: ArrayBuffer): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', data));
  return hex(digest.slice(0, 16));
}

/** PNG / GIF pixel size from the header. */
function imageSize(b: Uint8Array): { width: number; height: number } | null {
  const view = new DataView(b.buffer, b.byteOffset, b.byteLength);
  if (b.length >= 24 && b[0] === 0x89) return { width: view.getUint32(16), height: view.getUint32(20) };
  if (b.length >= 10 && b[0] === 0x47) return { width: view.getUint16(6, true), height: view.getUint16(8, true) };
  return null;
}

/**
 * Content-addressed bytes (GET or HEAD). The bytes behind an id never change,
 * but most can be erased: deleting a post erases its picture. So only custom
 * emojis (asked for as /emoji/<id>, and really an emoji) are cached for a
 * year; everything else at most a day, whichever path it's asked for by.
 */
export async function serveBlob(request: Request, id: string, env: Env, emojiPath: boolean): Promise<Response> {
  const head = request.method === 'HEAD';
  if (!head && request.method !== 'GET') return new Response('GET only.', { status: 405, headers: { Allow: 'GET, HEAD' } });
  const cache = caches.default;
  const cached = head ? undefined : await cache.match(request);
  if (cached) return cached;
  const blob = await media(env).get(id);
  if (!blob) return new Response(head ? null : 'Not found.', { status: 404 });
  const forever = emojiPath && (await hub(env).isEmoji(id));
  const headers = {
    'Content-Type': blob.mime,
    'Cache-Control': forever ? 'public, max-age=31536000, immutable' : 'public, max-age=86400',
    'X-Content-Type-Options': 'nosniff',
    'Content-Security-Policy': "default-src 'none'",
  };
  if (head) return new Response(null, { headers: { ...headers, 'Content-Length': String(blob.data.byteLength) } });
  const response = new Response(blob.data, { headers });
  await cache.put(request, response.clone());
  return response;
}

/** Trust the bytes, not the filename or Content-Type. No SVG (it can carry script). */
export function sniffImage(b: Uint8Array): string | null {
  const ascii = (from: number, to: number) => String.fromCharCode(...b.slice(from, to));
  if (b[0] === 0x89 && ascii(1, 4) === 'PNG') return 'image/png';
  if (ascii(0, 4) === 'GIF8') return 'image/gif';
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return 'image/jpeg';
  if (ascii(0, 4) === 'RIFF' && ascii(8, 12) === 'WEBP') return 'image/webp';
  return null;
}
