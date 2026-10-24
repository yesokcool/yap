// What a stored picture is, from its own bytes: its type (never trusting the
// filename or Content-Type) and its size in pixels and frames, and whether
// that's within LIMITS. Pure functions, so test/worker.test.mjs runs them in
// Node (hence the `.ts` on the import).
import { LIMITS } from './protocol.ts';

/** Trust the bytes, not the filename or Content-Type. No SVG (it can carry script). */
export function sniffImage(b: Uint8Array): string | null {
  const ascii = (from: number, to: number) => String.fromCharCode(...b.slice(from, to));
  if (b[0] === 0x89 && ascii(1, 4) === 'PNG') return 'image/png';
  if (ascii(0, 4) === 'GIF8') return 'image/gif';
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return 'image/jpeg';
  if (ascii(0, 4) === 'RIFF' && ascii(8, 12) === 'WEBP') return 'image/webp';
  return null;
}

/**
 * Why these picture bytes are refused, or null: a size we can't read, or one
 * past LIMITS (picturePx, pictureMegapixels; animatedPx and framePixels for an
 * animation; `maxSide`, e.g. LIMITS.iconPx for emojis and avatars). A few
 * hundred bytes can declare a 20000-pixel GIF that crashes an app the moment
 * it's drawn (security review, 2026-10-24).
 */
export function pictureProblem(b: Uint8Array, mime: string, maxSide: number = LIMITS.picturePx): string | null {
  const size = pictureSize(b, mime);
  if (!size || !size.width || !size.height) return 'That picture looks broken.';
  const { width, height, frames } = size;
  const side = Math.max(width, height);
  if (frames > 1 && (side > LIMITS.animatedPx || width * height * frames > LIMITS.framePixels)) {
    return `Animated pictures can be up to ${LIMITS.animatedPx} pixels a side, with fewer frames the bigger they are.`;
  }
  if (side > LIMITS.picturePx || width * height > LIMITS.pictureMegapixels * 1_000_000) {
    return `Pictures can be up to ${LIMITS.picturePx} pixels a side (${LIMITS.pictureMegapixels} megapixels).`;
  }
  if (side > maxSide) return `This one can be up to ${maxSide} pixels a side.`;
  return null;
}

/**
 * A picture's size in pixels and how many frames it has, read from its own
 * headers (PNG with APNG's frame count, GIF, JPEG, WebP), or null when they
 * can't be read. For a GIF, the canvas is as big as its biggest frame reaches.
 */
export function pictureSize(b: Uint8Array, mime: string): { width: number; height: number; frames: number } | null {
  const view = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const ascii = (from: number) => String.fromCharCode(...b.subarray(from, from + 4));
  if (mime === 'image/png') {
    if (b.length < 24) return null;
    let frames = 1;
    // An animated PNG says how many frames it has (acTL) before its first image data.
    for (let at = 8; at + 12 <= b.length;) {
      const length = view.getUint32(at);
      const type = ascii(at + 4);
      if (type === 'acTL' && at + 12 <= b.length) {
        frames = Math.max(1, view.getUint32(at + 8));
        break;
      }
      if (type === 'IDAT') break;
      at += 12 + length;
    }
    return { width: view.getUint32(16), height: view.getUint32(20), frames };
  }
  if (mime === 'image/gif') {
    if (b.length < 13) return null;
    let width = view.getUint16(6, true);
    let height = view.getUint16(8, true);
    let at = 13;
    if (b[10] & 0x80) at += 3 << ((b[10] & 7) + 1); // global colour table
    const skipSubBlocks = () => {
      while (at < b.length && b[at] !== 0) at += b[at] + 1;
      at++;
    };
    let frames = 0;
    while (at < b.length) {
      if (b[at] === 0x21) {
        // An extension: its introducer and label, then its sub-blocks.
        at += 2;
        skipSubBlocks();
      } else if (b[at] === 0x2c && at + 10 <= b.length) {
        // An image (one frame): left, top, width, height, flags.
        width = Math.max(width, view.getUint16(at + 1, true) + view.getUint16(at + 5, true));
        height = Math.max(height, view.getUint16(at + 3, true) + view.getUint16(at + 7, true));
        const flags = b[at + 9];
        at += 10;
        if (flags & 0x80) at += 3 << ((flags & 7) + 1); // local colour table
        at++; // LZW minimum code size
        skipSubBlocks();
        frames++;
      } else break; // the trailer (0x3b), or something we don't read
    }
    return { width, height, frames: Math.max(1, frames) };
  }
  if (mime === 'image/jpeg') {
    for (let at = 2; at + 9 <= b.length;) {
      if (b[at] !== 0xff) return null;
      const marker = b[at + 1];
      if (marker === 0xff) {
        at++; // fill
        continue;
      }
      if (marker === 0x01 || marker === 0xd8 || (marker >= 0xd0 && marker <= 0xd7)) {
        at += 2; // markers without a length
        continue;
      }
      // A start-of-frame (any kind but DHT/JPG/DAC, which share the range): length, precision, height, width.
      if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
        return { width: view.getUint16(at + 7), height: view.getUint16(at + 5), frames: 1 };
      }
      if (marker === 0xd9 || marker === 0xda) return null; // the end, or image data, before any size
      at += 2 + view.getUint16(at + 2);
    }
    return null;
  }
  if (mime === 'image/webp') {
    let width = 0;
    let height = 0;
    let frames = 0;
    for (let at = 12; at + 8 <= b.length;) {
      const type = ascii(at);
      const size = view.getUint32(at + 4, true);
      const body = at + 8;
      if (type === 'VP8X' && body + 10 <= b.length) {
        // The canvas: 24-bit width - 1 and height - 1 after the flags.
        width = 1 + (b[body + 4] | (b[body + 5] << 8) | (b[body + 6] << 16));
        height = 1 + (b[body + 7] | (b[body + 8] << 8) | (b[body + 9] << 16));
      } else if (type === 'VP8 ' && !width && body + 10 <= b.length) {
        width = view.getUint16(body + 6, true) & 0x3fff;
        height = view.getUint16(body + 8, true) & 0x3fff;
      } else if (type === 'VP8L' && !width && body + 5 <= b.length) {
        const bits = view.getUint32(body + 1, true);
        width = (bits & 0x3fff) + 1;
        height = ((bits >>> 14) & 0x3fff) + 1;
      } else if (type === 'ANMF') frames++;
      at = body + size + (size & 1);
    }
    return width && height ? { width, height, frames: Math.max(1, frames) } : null;
  }
  return null;
}
