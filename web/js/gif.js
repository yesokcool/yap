/*
 * A tiny animated-GIF encoder (GIF89a, LZW), no library. Used by Draw's
 * "GIF" button to archive the wiggling page. Input frames are opaque RGBA;
 * colors are collected into one global palette. A drawing has a handful of
 * pen colors; anything past 256 colors keeps its 40 most used ones plus an
 * even 6×6×6 color cube, and every pixel snaps to the nearest.
 */

/**
 * @param {Uint8ClampedArray[]} frames  RGBA, width*height*4 each, opaque
 * @param {number} width
 * @param {number} height
 * @param {number} delayMs              per frame
 * @returns {Uint8Array} the .gif bytes
 */
export function encodeGif(frames, width, height, delayMs) {
  const { palette, indexed } = quantize(frames);
  let bits = 1;
  while (1 << bits < palette.length) bits++;
  const size = 1 << bits;
  const out = new Bytes();
  out.str('GIF89a');
  out.u16(width);
  out.u16(height);
  out.u8(0x80 | 0x70 | (bits - 1)); // global color table, 8-bit color resolution
  out.u8(0); // background index
  out.u8(0); // aspect
  for (let i = 0; i < size; i++) {
    const c = palette[i] ?? 0;
    out.u8((c >> 16) & 255);
    out.u8((c >> 8) & 255);
    out.u8(c & 255);
  }
  if (frames.length > 1) {
    // Loop forever (the NETSCAPE2.0 application extension).
    out.u8(0x21); out.u8(0xff); out.u8(11); out.str('NETSCAPE2.0');
    out.u8(3); out.u8(1); out.u16(0); out.u8(0);
  }
  const delay = Math.max(2, Math.round(delayMs / 10));
  for (const pixels of indexed) {
    out.u8(0x21); out.u8(0xf9); out.u8(4); out.u8(0x04); out.u16(delay); out.u8(0); out.u8(0); // graphic control
    out.u8(0x2c); out.u16(0); out.u16(0); out.u16(width); out.u16(height); out.u8(0); // image descriptor
    const minCode = Math.max(2, bits);
    out.u8(minCode);
    const data = lzw(pixels, minCode);
    for (let i = 0; i < data.length; i += 255) {
      const chunk = data.subarray(i, i + 255);
      out.u8(chunk.length);
      out.bytes(chunk);
    }
    out.u8(0);
  }
  out.u8(0x3b);
  return out.done();
}

/** One palette for all frames; pixels become indexes into it. */
function quantize(frames) {
  const counts = new Map();
  for (const f of frames) {
    for (let i = 0; i < f.length; i += 4) {
      const c = (f[i] << 16) | (f[i + 1] << 8) | f[i + 2];
      counts.set(c, (counts.get(c) ?? 0) + 1);
    }
  }
  let palette = [...counts.keys()].sort((a, b) => counts.get(b) - counts.get(a));
  if (palette.length > 256) {
    const cube = [];
    for (let r = 0; r < 6; r++) for (let g = 0; g < 6; g++) for (let b = 0; b < 6; b++) cube.push((r * 51 << 16) | (g * 51 << 8) | (b * 51));
    palette = [...new Set([...palette.slice(0, 40), ...cube])].slice(0, 256);
  }
  const index = new Map(palette.map((c, i) => [c, i]));
  const nearest = (c) => {
    let best = 0;
    let bestD = Infinity;
    for (let i = 0; i < palette.length; i++) {
      const p = palette[i];
      const d = (((p >> 16) & 255) - ((c >> 16) & 255)) ** 2 + (((p >> 8) & 255) - ((c >> 8) & 255)) ** 2 + ((p & 255) - (c & 255)) ** 2;
      if (d < bestD) [best, bestD] = [i, d];
    }
    index.set(c, best);
    return best;
  };
  const indexed = frames.map((f) => {
    const px = new Uint8Array(f.length / 4);
    for (let i = 0, j = 0; i < f.length; i += 4, j++) {
      const c = (f[i] << 16) | (f[i + 1] << 8) | f[i + 2];
      px[j] = index.get(c) ?? nearest(c);
    }
    return px;
  });
  return { palette, indexed };
}

/** GIF's variable-width LZW (codes ≤ 12 bits, LSB-first). */
function lzw(pixels, minCode) {
  const clear = 1 << minCode;
  const end = clear + 1;
  const out = new Bytes();
  let cur = 0;
  let shift = 0;
  let codeSize = minCode + 1;
  let next = end + 1;
  let table = new Map();
  const emit = (code) => {
    cur |= code << shift;
    shift += codeSize;
    while (shift >= 8) {
      out.u8(cur & 255);
      cur >>>= 8;
      shift -= 8;
    }
  };
  emit(clear);
  let prefix = pixels[0];
  for (let i = 1; i < pixels.length; i++) {
    const k = pixels[i];
    const key = (prefix << 8) | k;
    const code = table.get(key);
    if (code !== undefined) {
      prefix = code;
      continue;
    }
    emit(prefix);
    if (next === 4096) {
      emit(clear);
      table = new Map();
      next = end + 1;
      codeSize = minCode + 1;
    } else {
      if (next >= 1 << codeSize) codeSize++;
      table.set(key, next++);
    }
    prefix = k;
  }
  emit(prefix);
  emit(end);
  if (shift > 0) out.u8(cur & 255);
  return out.done();
}

class Bytes {
  buf = new Uint8Array(1 << 16);
  n = 0;
  #grow(extra) {
    if (this.n + extra <= this.buf.length) return;
    const bigger = new Uint8Array(Math.max(this.buf.length * 2, this.n + extra));
    bigger.set(this.buf);
    this.buf = bigger;
  }
  u8(v) {
    this.#grow(1);
    this.buf[this.n++] = v;
  }
  u16(v) {
    this.u8(v & 255);
    this.u8((v >> 8) & 255);
  }
  str(s) {
    for (const ch of s) this.u8(ch.charCodeAt(0));
  }
  bytes(b) {
    this.#grow(b.length);
    this.buf.set(b, this.n);
    this.n += b.length;
  }
  done() {
    return this.buf.slice(0, this.n);
  }
}
