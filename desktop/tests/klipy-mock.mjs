// A tiny stand-in for KLIPY's API, for tests (point the app at it with
// YAP_KLIPY_API=http://localhost:<port>/api/v1).
//
//   node desktop/tests/klipy-mock.mjs <port> [local|klipy]
//
// `local`: media URLs point back here (PNG tiles; for screenshots).
// `klipy`: media URLs look like KLIPY's CDN (https://static.klipy.com/…), so the
//          Yap server's allowlist accepts a send (the images themselves 404).
// Logs every request as "REQ <method> <path>" and shares as "SHARE <slug> <body>".
import http from 'node:http';
import zlib from 'node:zlib';

const port = Number(process.argv[2] ?? 8794);
const mediaMode = process.argv[3] ?? 'local';
const COLORS = [[255, 126, 182], [78, 230, 197], [122, 167, 255], [255, 209, 102], [255, 93, 108], [142, 78, 198]];
const SIZES = [[498, 280], [498, 498], [360, 480], [480, 270]];

function png(w, h, [r, g, b]) {
  const rows = [];
  for (let y = 0; y < h; y++) {
    const row = Buffer.alloc(1 + w * 4);
    for (let x = 0; x < w; x++) {
      const stripe = ((x + y) >> 4) % 2 ? 1 : 0.82;
      row.set([r * stripe, g * stripe, b * stripe, 255], 1 + x * 4);
    }
    rows.push(row);
  }
  const chunk = (t, d) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(d.length);
    const td = Buffer.concat([Buffer.from(t), d]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(zlib.crc32(td) >>> 0);
    return Buffer.concat([len, td, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr.set([8, 6, 0, 0, 0], 8);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(Buffer.concat(rows))), chunk('IEND', Buffer.alloc(0))]);
}

function item(q, i) {
  const [w, h] = SIZES[i % SIZES.length];
  const base = mediaMode === 'local' ? `http://localhost:${port}/media` : 'https://static.klipy.com/test';
  const file = (kind) => ({ url: `${base}/${q}-${i}.${kind === 'jpg' && mediaMode === 'local' ? 'png' : kind}`, width: w, height: h, size: 1000 });
  const size = { gif: file('gif'), webp: file('webp'), jpg: file('jpg'), mp4: file('mp4'), webm: file('webm') };
  return { id: i, slug: `${q}-${i}`, title: `${q} ${i}`, type: 'gif', file: { hd: size, md: size, sm: size, xs: size } };
}

http
  .createServer((req, res) => {
    const url = new URL(req.url, `http://localhost:${port}`);
    console.log(`REQ ${req.method} ${url.pathname}${url.search}`);
    if (url.pathname.startsWith('/media/')) {
      const i = Number(url.pathname.match(/-(\d+)\./)?.[1] ?? 0);
      const [w, h] = SIZES[i % SIZES.length];
      res.writeHead(200, { 'Content-Type': 'image/png' });
      return res.end(png(Math.round(w / 3), Math.round(h / 3), COLORS[i % COLORS.length]));
    }
    if (req.method === 'POST') {
      let body = '';
      req.on('data', (d) => (body += d));
      req.on('end', () => {
        console.log(`SHARE ${url.pathname.split('/').pop()} ${body}`);
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ result: true }));
      });
      return;
    }
    const q = url.searchParams.get('q') ?? (url.pathname.includes('/recent/') ? 'recent' : 'trending');
    const page = Number(url.searchParams.get('page') ?? 1);
    const items = Array.from({ length: 12 }, (_, i) => item(q, (page - 1) * 12 + i));
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ result: true, data: { data: items, current_page: page, per_page: 12, has_next: page < 2 } }));
  })
  .listen(port, () => console.log(`klipy mock on ${port} (${mediaMode} media)`));
