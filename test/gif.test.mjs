// The GIF encoder (web/js/gif.js) checked against an independent decoder
// written here from the GIF89a spec. Run: node test/gif.test.mjs
import assert from 'node:assert/strict';
import { encodeGif } from '../web/js/gif.js';

const results = [];
async function test(name, fn) {
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(['✗', name, err]);
  }
}

/** → { width, height, palette: [r,g,b][], frames: Uint8Array[] (indexes), delays, loops } */
function decode(bytes) {
  let p = 0;
  const u8 = () => bytes[p++];
  const u16 = () => bytes[p++] | (bytes[p++] << 8);
  const str = (n) => String.fromCharCode(...bytes.subarray(p, (p += n)));
  assert.equal(str(6), 'GIF89a');
  const width = u16();
  const height = u16();
  const flags = u8();
  u8();
  u8();
  const palette = [];
  if (flags & 0x80) for (let i = 0; i < 1 << ((flags & 7) + 1); i++) palette.push([u8(), u8(), u8()]);
  const frames = [];
  const delays = [];
  let loops = false;
  for (;;) {
    const b = u8();
    if (b === 0x3b) break;
    if (b === 0x21) {
      const label = u8();
      if (label === 0xf9) {
        u8();
        u8();
        delays.push(u16());
        u8();
        u8();
        continue;
      }
      if (label === 0xff && str(u8()) === 'NETSCAPE2.0') loops = true;
      for (let n = u8(); n; n = u8()) p += n;
      continue;
    }
    assert.equal(b, 0x2c, 'image descriptor');
    u16();
    u16();
    const w = u16();
    const h = u16();
    assert.equal(u8() & 0x80, 0, 'no local color table');
    const minCode = u8();
    const data = [];
    for (let n = u8(); n; n = u8()) for (let i = 0; i < n; i++) data.push(u8());
    frames.push(lzwDecode(data, minCode, w * h));
  }
  return { width, height, palette, frames, delays, loops };
}

function lzwDecode(data, minCode, count) {
  const clear = 1 << minCode;
  const end = clear + 1;
  const out = new Uint8Array(count);
  let n = 0;
  let size = minCode + 1;
  let dict = [];
  const reset = () => {
    dict = [];
    for (let i = 0; i < clear; i++) dict[i] = [i];
    dict[clear] = [];
    dict[end] = null;
    size = minCode + 1;
  };
  reset();
  let bitPos = 0;
  const read = () => {
    let code = 0;
    for (let i = 0; i < size; i++, bitPos++) if (data[bitPos >> 3] & (1 << (bitPos & 7))) code |= 1 << i;
    return code;
  };
  let prev = null;
  for (;;) {
    const code = read();
    if (code === clear) {
      reset();
      prev = null;
      continue;
    }
    if (code === end) break;
    let entry;
    if (code < dict.length && dict[code]) entry = dict[code];
    else if (code === dict.length && prev) entry = [...prev, prev[0]];
    else throw new Error(`bad code ${code} (dict ${dict.length})`);
    for (const v of entry) out[n++] = v;
    if (prev) {
      dict.push([...prev, entry[0]]);
      if (dict.length === 1 << size && size < 12) size++;
    }
    prev = entry;
  }
  assert.equal(n, count, 'pixel count');
  return out;
}

const rgbaFrom = (w, h, pick) => {
  const f = new Uint8ClampedArray(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const [r, g, b] = pick(x, y);
      const i = (y * w + x) * 4;
      f.set([r, g, b, 255], i);
    }
  }
  return f;
};
const colorAt = (gif, frame, x, y) => gif.palette[gif.frames[frame][y * gif.width + x]];

await test('a drawing-like page round-trips exactly, 3 frames, looping, 140 ms', async () => {
  const pens = [[255, 255, 255], [0, 0, 0], [229, 72, 77], [62, 99, 221]];
  const frames = [0, 1, 2].map((f) => rgbaFrom(640, 400, (x, y) => pens[(x + f * 3) % 97 < 6 ? 1 : (y + f) % 53 < 3 ? 2 : (x * y) % 211 === 0 ? 3 : 0]));
  const gif = decode(encodeGif(frames, 640, 400, 140));
  assert.deepEqual([gif.width, gif.height, gif.frames.length, gif.loops], [640, 400, 3, true]);
  assert.deepEqual(gif.delays, [14, 14, 14]);
  for (let f = 0; f < 3; f++) {
    for (let y = 0; y < 400; y += 7) for (let x = 0; x < 640; x += 5) {
      const i = (y * 640 + x) * 4;
      assert.deepEqual(colorAt(gif, f, x, y), [...frames[f].subarray(i, i + 3)], `frame ${f} at ${x},${y}`);
    }
  }
});

await test('noisy images (dictionary resets past 4096 codes) and one-color images', async () => {
  let seed = 7;
  const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) % 200);
  const noisy = rgbaFrom(300, 200, () => [rand(), rand() % 4, 9]);
  const gif = decode(encodeGif([noisy], 300, 200, 100));
  for (let i = 0; i < 300 * 200; i += 13) assert.deepEqual(gif.palette[gif.frames[0][i]], [...noisy.subarray(i * 4, i * 4 + 3)]);
  assert.equal(gif.loops, false, 'a single frame needs no loop block');
  const blank = decode(encodeGif([rgbaFrom(10, 10, () => [255, 255, 255])], 10, 10, 100));
  assert.ok(blank.frames[0].every((v) => v === blank.frames[0][0]));
});

await test('more than 256 colors snap to the nearest kept color', async () => {
  const many = rgbaFrom(64, 64, (x, y) => [x * 4, y * 4, 128]);
  const gif = decode(encodeGif([many], 64, 64, 100));
  assert.ok(gif.palette.length <= 256);
  for (const [x, y] of [[33, 20], [5, 60], [63, 63], [40, 9]]) {
    const [r, g, b] = colorAt(gif, 0, x, y);
    assert.ok(Math.abs(r - x * 4) <= 26 && Math.abs(g - y * 4) <= 26 && Math.abs(b - 128) <= 26, `close enough at ${x},${y}: ${r},${g},${b}`);
  }
});

for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.message ?? err}`);
}
const failed = results.filter((r) => r[0] === '✗').length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
