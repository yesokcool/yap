/*
 * The shared drawing board. Everyone on the "draw" view sees strokes as
 * they're drawn (draw.seg pieces), finished strokes are stored on the server
 * (draw.stroke), and "Save" keeps a PNG in the gallery.
 *
 * Board space is 10000 × 6250 (16:10); coordinates and widths travel in
 * those units so every screen size draws the same picture.
 *
 * Three layers (PROTOCOL.md › Draw layers), each its own transparent
 * canvas over the paper (CSS): that layer's committed strokes, plus any
 * strokes being drawn on it right now. An eraser only removes ink from its
 * own layer. Which layer you draw on, and hiding one, are yours alone.
 */
import { encodeGif } from './gif.js';
import { esc } from './format.js';
import { icon } from './icons.js';
import { placeNear } from './place.js';

const BOARD_W = 10000;
const BOARD_H = 6250;
/**
 * The page is drawn at a small fixed size and blown up with hard pixel edges
 * (CSS `image-rendering: pixelated`), WigglyPaint-style; saves use it as is.
 */
export const PIX_W = 640;
export const PIX_H = 400;
const PAPER = '#ffffff';
const SEND_EVERY_MS = 40;
const LAYERS = 3;
const LAYER_NAMES = ['bottom', 'middle', 'top'];
const THUMB_W = 40;
const THUMB_H = 25;
/** Phones: one row of tools, colors/sizes/layers in a palette, the actions under the page. */
const NARROW = matchMedia('(max-width: 720px)');
/** A stroke's layer (absent = 0, the bottom). */
const layerOf = (s) => s.l ?? 0;
// Paint's colour box: dark row on top, bright row below.
const COLORS = [
  '#000000', '#808080', '#800000', '#808000', '#008000', '#008080', '#000080', '#800080', '#808040', '#004040', '#0080ff', '#004080', '#8000ff', '#804000',
  '#ffffff', '#c0c0c0', '#ff0000', '#ffff00', '#00ff00', '#00ffff', '#0000ff', '#ff00ff', '#ffff80', '#00ff80', '#80ffff', '#8080ff', '#ff0080', '#ff8040',
];
const SIZES = [{ w: 14, label: 'S' }, { w: 32, label: 'M' }, { w: 70, label: 'L' }, { w: 160, label: 'XL' }];
const TOOLS = [['pen', '✎', 'Pen (P)'], ['line', '╱', 'Line (L)'], ['rect', '▭', 'Box (R)'], ['ellipse', '◯', 'Circle (O)'], ['text', 'A', 'Text (T)']];
const TEXT_FONT = "'Comic Sans MS', 'Chalkboard SE', 'Comic Neue', 'Marker Felt', cursive";
/** Text size in board units for a brush width. */
const textSize = (w) => Math.max(110, w * 8);
/** The finished stroke for a shape dragged from (x0, y0) to (x1, y1). */
function shapeStroke(tool, x0, y0, x1, y1) {
  if (tool === 'line') return { k: 'poly', p: [x0, y0, x1, y1] };
  if (tool === 'rect') return { k: 'poly', p: [x0, y0, x1, y0, x1, y1, x0, y1, x0, y0] };
  // Ellipse: enough points to look round; smoothed like a freehand stroke.
  const cx = (x0 + x1) / 2;
  const cy = (y0 + y1) / 2;
  const rx = Math.abs(x1 - x0) / 2;
  const ry = Math.abs(y1 - y0) / 2;
  const n = Math.max(24, Math.min(96, Math.round((rx + ry) / 40)));
  const p = [];
  for (let i = 0; i <= n + 1; i++) {
    const a = (i / n) * Math.PI * 2;
    p.push(Math.round(cx + rx * Math.cos(a)), Math.round(cy + ry * Math.sin(a)));
  }
  return { p };
}

const id = () => crypto.randomUUID().replace(/-/g, '').slice(0, 20);

/**
 * A canvas that crisp() reads back: kept in memory rather than on the GPU,
 * where every getImageData would wait for the graphics card.
 */
function readable(width, height, canvas = document.createElement('canvas')) {
  Object.assign(canvas, { width, height });
  canvas.getContext('2d', { willReadFrequently: true });
  return canvas;
}

/** Hard pixel edges: canvas anti-aliases every line, so snap alpha to on/off. */
function crisp(ctx) {
  const { width, height } = ctx.canvas;
  const img = ctx.getImageData(0, 0, width, height);
  const d = img.data;
  for (let i = 3; i < d.length; i += 4) d[i] = d[i] >= 110 ? 255 : 0;
  ctx.putImageData(img, 0, 0);
}

// ── Wiggle ("line boil", à la WigglyPaint) ──────────────────────────────────
// Every stroke is drawn three times with a little hand-wobble each time, and
// the three frames cycle ~7×/s. Deterministic (by position), so everyone sees
// the same wiggle. Saved PNGs are drawn without it.
const WIGGLE_FRAMES = 3;
const WIGGLE_MS = 140;
const hash = (n) => {
  n = (n ^ 61) ^ (n >>> 16);
  n = (n + (n << 3)) | 0;
  n ^= n >>> 4;
  n = Math.imul(n, 0x27d4eb2d);
  n ^= n >>> 15;
  return (n >>> 0) / 4294967295;
};
/**
 * Smooth value noise over board positions: the same spot always moves the
 * same way in a given frame, whichever stroke is there — so an eraser stays
 * exactly on top of the ink it removed while everything wiggles.
 */
const CELL = 220; // board units between noise lattice points
const AMP = 22; // max displacement, board units (board is 10000 wide)
function field(x, y, frame, axis) {
  const gx = Math.floor(x / CELL);
  const gy = Math.floor(y / CELL);
  const tx = x / CELL - gx;
  const ty = y / CELL - gy;
  const at = (i, j) => hash((i * 73856093) ^ (j * 19349663) ^ (frame * 83492791) ^ (axis * 2654435761)) - 0.5;
  const sx = tx * tx * (3 - 2 * tx);
  const sy = ty * ty * (3 - 2 * ty);
  const top = at(gx, gy) + (at(gx + 1, gy) - at(gx, gy)) * sx;
  const bottom = at(gx, gy + 1) + (at(gx + 1, gy + 1) - at(gx, gy + 1)) * sx;
  return (top + (bottom - top) * sy) * 2 * AMP;
}
function wobble(p, _strokeId, frame) {
  const out = new Array(p.length);
  for (let i = 0; i < p.length; i += 2) {
    out[i] = p[i] + field(p[i], p[i + 1], frame, 0);
    out[i + 1] = p[i + 1] + field(p[i], p[i + 1], frame, 1);
  }
  return out;
}

export class DrawBoard {
  /**
   * @param {HTMLElement} root
   * @param {{ send: (msg: object) => void, upload: (blob: Blob) => Promise<void>, who: (uid: string) => {name: string, color: string} | null,
   *           whoSid: (sid: string) => {name: string, color: string} | null, me: () => string, toast: (t: string) => void }} hooks
   */
  constructor(root, hooks) {
    this.root = root;
    this.hooks = hooks;
    this.strokes = []; // committed, in order
    this.live = new Map(); // key (sid:sk or 'me') -> { c, w, e, p, at, who }
    this.mine = []; // my stroke ids, for undo
    this.gallery = [];
    this.color = COLORS[0];
    this.size = SIZES[1].w;
    this.eraser = false;
    this.tool = 'pen';
    this.layer = 0; // the one you draw on
    this.hidden = new Set(); // layers you've hidden (just for you)
    this.drawing = null;
    this.raf = 0;
    this.wiggle = (() => { try { return localStorage.getItem('yap.wiggle') !== '0'; } catch { return true; } })();
    this.frame = 0;
    // The committed page: one canvas per layer per wiggle frame.
    this.frames = Array.from({ length: LAYERS }, () => Array.from({ length: WIGGLE_FRAMES }, () => readable(PIX_W, PIX_H)));

    root.innerHTML = `
      <div class="draw-tools" role="toolbar" aria-label="Drawing tools">
        <button class="btn small draw-brush" type="button" data-tool="palette" aria-haspopup="dialog" aria-expanded="false" title="Color, size and layer" aria-label="Color, size and layer"><i class="draw-brush-dot"></i><span class="draw-brush-layer"></span></button>
        <div class="draw-colors">${COLORS.map((c) => `<button class="swatch-btn" data-color="${c}" style="--c:${c}" aria-label="Color ${c}"></button>`).join('')}
          <label class="swatch-btn custom" title="Any color"><input type="color" value="#3e63dd"></label></div>
        <div class="draw-sizes">${SIZES.map((s) => `<button class="size-btn" data-size="${s.w}" title="Brush ${s.label}"><i style="--s:${Math.max(4, Math.min(22, s.w / 7))}px"></i></button>`).join('')}</div>
        <div class="draw-shapes" role="radiogroup" aria-label="Tool">${TOOLS.map(([t, glyph, title]) => `<button class="shape-btn" data-shape="${t}" title="${title}" aria-label="${title}">${glyph}</button>`).join('')}</div>
        <button class="btn small" data-tool="eraser" aria-pressed="false" title="Eraser (E)">Eraser</button>
        <button class="btn small" data-tool="wiggle" aria-pressed="true" title="Wobbly lines, like WigglyPaint">Wiggle</button>
        <div class="draw-layers" role="group" aria-label="Layers">${Array.from({ length: LAYERS }, (_, l) => `
          <span class="layer-chip" data-chip="${l}">
            <button class="layer-pick" data-layer="${l}" title="Layer ${l + 1} (${LAYER_NAMES[l]}) — press ${l + 1}" aria-label="Draw on layer ${l + 1} (${LAYER_NAMES[l]})"><canvas class="layer-thumb" width="${THUMB_W}" height="${THUMB_H}"></canvas><span class="layer-num">${l + 1}</span></button>
            <button class="layer-eye" data-eye="${l}"></button>
          </span>`).join('')}
        </div>
        <span class="spacer"></span>
        <div class="draw-actions">
          <button class="btn small" data-tool="undo" title="Undo your last stroke (⌘Z)">Undo</button>
          <button class="btn small" data-tool="save" title="Download a PNG and keep it in the gallery">Save</button>
          <button class="btn small" data-tool="gif" title="Download an animated GIF of the page (it wiggles) and keep it in the gallery">GIF</button>
          <button class="btn small" data-tool="share" title="Post this drawing in the chat">Share</button>
          <button class="btn small danger" data-tool="clear" title="Clear a layer or the whole page for everyone (saves this one first)">Clear</button>
        </div>
      </div>
      <div class="popover draw-palette" role="dialog" aria-label="Color, size and layer" hidden>
        <div class="pop-title"><span>Color, size and layer</span></div>
        <p class="draw-palette-label muted">Layer <small>— the eye hides one just for you</small></p>
      </div>
      <div class="popover draw-clear-pop" role="dialog" aria-label="Clear" hidden></div>
      <div class="draw-stage">
        <div class="draw-paper">
          ${Array.from({ length: LAYERS }, (_, l) => `<canvas class="draw-layer" data-layer-canvas="${l}"></canvas>`).join('')}
          <div class="draw-tags"></div>
          <input class="draw-text" maxlength="200" placeholder="Type, then Enter" hidden>
        </div>
      </div>
      <div class="draw-gallery" aria-label="Saved drawings"></div>`;
    this.paper = root.querySelector('.draw-paper');
    this.sheets = [...root.querySelectorAll('.draw-layer')];
    for (const c of this.sheets) readable(PIX_W, PIX_H, c);
    this.thumbs = [...root.querySelectorAll('.layer-thumb')];
    this.clearPop = root.querySelector('.draw-clear-pop');
    this.palette = root.querySelector('.draw-palette');
    this.tags = root.querySelector('.draw-tags');
    this.galleryEl = root.querySelector('.draw-gallery');
    this.#bindTools();
    this.#bindPointer();
    this.#layout();
    NARROW.addEventListener('change', () => this.#layout());
    const resize = new ResizeObserver(() => this.#resize());
    resize.observe(root.querySelector('.draw-stage'));
    resize.observe(root);
    addEventListener('keydown', (e) => {
      if (root.hidden || /^(INPUT|TEXTAREA)$/.test(e.target.tagName)) return;
      if ((e.metaKey || e.ctrlKey) && e.key === 'z') {
        e.preventDefault();
        this.undo();
      } else if (e.metaKey || e.ctrlKey || e.altKey) return; // the browser's and Yap's own shortcuts
      else if (e.key === 'e') this.#setEraser(!this.eraser);
      else if (e.key === 'b' || e.key === 'p') this.#setTool('pen');
      else if (e.key === 'l') this.#setTool('line');
      else if (e.key === 'r') this.#setTool('rect');
      else if (e.key === 'o') this.#setTool('ellipse');
      else if (e.key === 't') this.#setTool('text');
      else if (/^[1-9]$/.test(e.key) && Number(e.key) <= LAYERS) this.#setLayer(Number(e.key) - 1);
      else if (e.key === 'Escape') {
        this.#closeClear();
        this.#closePalette();
      }
    });
    addEventListener('pointerdown', (e) => {
      if (!this.clearPop.hidden && !e.target.closest('.draw-clear-pop, [data-tool=clear]')) this.#closeClear();
      if (!this.palette.hidden && !e.target.closest('.draw-palette, [data-tool=palette]')) this.#closePalette();
    });
    this.#syncTools();
    setInterval(() => {
      if (!this.wiggle || this.root.hidden || document.hidden) return;
      this.frame = (this.frame + 1) % WIGGLE_FRAMES;
      this.#paint();
    }, WIGGLE_MS);
  }

  open() {
    this.hooks.send({ t: 'draw.state' });
    this.#resize();
  }

  // ── Server events ────────────────────────────────────────────────────────

  onState({ strokes, gallery }) {
    this.strokes = strokes;
    this.mine = strokes.filter((s) => s.uid === this.hooks.me()).map((s) => s.id);
    this.#dropLive();
    this.gallery = gallery;
    this.#renderGallery();
    this.#redrawCommitted();
  }

  onSeg({ sid, sk, c, w, e, p, l }) {
    const key = `${sid}:${sk}`;
    const entry = this.live.get(key) ?? { sk, c, w, e, l: l ?? 0, p: [], who: this.hooks.whoSid(sid) };
    entry.p.push(...p);
    entry.at = Date.now();
    this.live.set(key, entry);
    this.#scheduleLive();
  }

  onStroke({ stroke }) {
    for (const [key] of this.live) if (key.endsWith(`:${stroke.id}`)) this.live.delete(key);
    if (this.strokes.some((s) => s.id === stroke.id)) return;
    this.strokes.push(stroke);
    this.#commit(stroke);
    this.#scheduleLive();
  }

  onUndo({ id: strokeId }) {
    this.strokes = this.strokes.filter((s) => s.id !== strokeId);
    this.mine = this.mine.filter((x) => x !== strokeId);
    this.#redrawCommitted();
  }

  onClear({ by }) {
    this.strokes = [];
    this.mine = [];
    this.#dropLive();
    this.#redrawCommitted();
    this.#scheduleLive();
    const who = this.hooks.who(by);
    if (by !== this.hooks.me()) this.hooks.toast(`${who?.name ?? 'Someone'} started a fresh page (the old one is in the gallery).`);
  }

  /** One layer emptied for everyone. */
  onWipe({ l, by }) {
    this.strokes = this.strokes.filter((s) => layerOf(s) !== l);
    const left = new Set(this.strokes.map((s) => s.id));
    this.mine = this.mine.filter((x) => left.has(x));
    this.#redrawCommitted();
    const who = this.hooks.who(by);
    if (by !== this.hooks.me()) this.hooks.toast(`${who?.name ?? 'Someone'} cleared layer ${l + 1} (the page is in the gallery).`);
  }

  /** Forget other people's strokes in progress (a fresh page or state); the one you're drawing stays. */
  #dropLive() {
    for (const key of this.live.keys()) if (key !== 'me') this.live.delete(key);
  }

  onDrawing({ drawing }) {
    if (!this.gallery.some((d) => d.id === drawing.id)) this.gallery.unshift(drawing);
    this.#renderGallery();
  }

  // ── Actions ──────────────────────────────────────────────────────────────

  undo() {
    const last = this.mine.pop();
    if (last) this.hooks.send({ t: 'draw.undo', id: last });
  }

  /**
   * Save what you see (hidden layers left out); `every` = all layers (Clear's archive).
   * Resolves to the gallery entry, or null when it couldn't be kept (the upload said why).
   */
  async save({ download = true, gif = false, every = false } = {}) {
    const layers = this.#shown(every);
    const blob = gif ? this.#gif(layers) : await this.#png(layers);
    if (download) {
      const a = Object.assign(document.createElement('a'), {
        href: URL.createObjectURL(blob),
        download: `yap-drawing-${new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-')}.${gif ? 'gif' : 'png'}`,
      });
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 5000);
    }
    return this.hooks.upload(blob);
  }

  /** The layers that show: yours (not hidden), or `every` one. */
  #shown(every = false) {
    return Array.from({ length: LAYERS }, (_, l) => l).filter((l) => every || !this.hidden.has(l));
  }

  /** Is there anything on these layers? */
  #anyOn(layers) {
    return this.strokes.some((s) => layers.includes(layerOf(s)));
  }

  /** The page on white paper at its own pixel size, layer by layer; `frame` −1 = straight lines. */
  #flatten(layers, frame = -1) {
    const out = readable(PIX_W, PIX_H);
    const ctx = out.getContext('2d');
    ctx.fillStyle = PAPER;
    ctx.fillRect(0, 0, PIX_W, PIX_H);
    const sheet = readable(PIX_W, PIX_H);
    const sctx = sheet.getContext('2d');
    for (const l of layers) {
      sctx.clearRect(0, 0, PIX_W, PIX_H);
      for (const s of this.strokes) if (layerOf(s) === l) this.#drawStroke(sctx, s, PIX_W / BOARD_W, frame);
      crisp(sctx);
      ctx.drawImage(sheet, 0, 0);
    }
    return out;
  }

  #png(layers) {
    return new Promise((resolve) => this.#flatten(layers).toBlob(resolve, 'image/png'));
  }

  /** Every wiggle frame, looping, as an animated GIF (one frame when Wiggle is off). */
  #gif(layers) {
    const count = this.wiggle ? WIGGLE_FRAMES : 1;
    const frames = Array.from({ length: count }, (_, f) => this.#flatten(layers, this.wiggle ? f : -1).getContext('2d').getImageData(0, 0, PIX_W, PIX_H).data);
    return new Blob([encodeGif(frames, PIX_W, PIX_H, WIGGLE_MS)], { type: 'image/gif' });
  }

  // ── Tools ────────────────────────────────────────────────────────────────

  #bindTools() {
    this.root.addEventListener('click', async (e) => {
      if (!e.target.closest('.draw-tools, .draw-palette, .draw-actions')) return;
      const color = e.target.closest('[data-color]')?.dataset.color;
      const size = e.target.closest('[data-size]')?.dataset.size;
      const tool = e.target.closest('[data-tool]')?.dataset.tool;
      const shape = e.target.closest('[data-shape]')?.dataset.shape;
      const layer = e.target.closest('[data-layer]')?.dataset.layer;
      const eye = e.target.closest('[data-eye]')?.dataset.eye;
      if (shape) this.#setTool(shape);
      if (layer) this.#setLayer(Number(layer));
      if (eye) this.#toggleHidden(Number(eye));
      if (color) {
        this.color = color;
        this.#setEraser(false);
        this.#closePalette(); // (phones) picked: back to drawing
      }
      if (tool === 'palette') this.#togglePalette(e.target.closest('[data-tool]'));
      if (size) this.size = Number(size);
      if (tool === 'eraser') this.#setEraser(!this.eraser);
      if (tool === 'wiggle') {
        this.wiggle = !this.wiggle;
        try { localStorage.setItem('yap.wiggle', this.wiggle ? '1' : '0'); } catch {}
        this.#redrawCommitted();
      }
      if (tool === 'undo') this.undo();
      const left = this.hidden.size ? ' Hidden layers were left out.' : '';
      if (tool === 'save') {
        await this.#once(async () => {
          if (await this.save()) this.hooks.toast(`Saved — downloaded and added to the gallery.${left}`);
        });
      }
      if (tool === 'gif') {
        if (!this.#anyOn(this.#shown())) return this.hooks.toast('Draw something first.');
        await this.#once(async () => {
          if (await this.save({ gif: true })) this.hooks.toast(`Saved a GIF — downloaded and added to the gallery.${left}`);
        });
      }
      if (tool === 'share') {
        if (!this.#anyOn(this.#shown())) return this.hooks.toast('Draw something first.');
        // With Wiggle on, the chat gets the wiggling GIF.
        await this.#once(async () => this.hooks.share(this.wiggle ? this.#gif(this.#shown()) : await this.#png(this.#shown())));
      }
      if (tool === 'clear') this.#toggleClear(e.target.closest('[data-tool]'));
      this.#syncTools();
    });
    this.clearPop.addEventListener('click', async (e) => {
      const what = e.target.closest('[data-clear]')?.dataset.clear;
      if (!what) return;
      this.#closeClear();
      if (what === 'cancel') return;
      await this.#once(async () => {
        // Archived first, every layer, wiggling — whichever way it's cleared. No archive, no clear.
        if (this.strokes.length && !(await this.save({ download: false, gif: true, every: true }))) {
          return this.hooks.toast("Couldn't keep this page in the gallery, so nothing was cleared. Try again in a moment.");
        }
        this.hooks.send(what === 'page' ? { t: 'draw.clear' } : { t: 'draw.wipe', l: this.layer });
      });
    });
    this.root.querySelector('input[type=color]').addEventListener('input', (e) => {
      this.color = e.target.value;
      this.#setEraser(false);
      this.#syncTools();
    });
  }

  /** Save, GIF, Share and Clear one at a time: a double click makes one picture, not two. */
  async #once(job) {
    if (this.busy) return;
    this.busy = true;
    try {
      await job();
    } finally {
      this.busy = false;
    }
  }

  #setEraser(on) {
    this.eraser = on;
    if (on) this.tool = 'pen';
    this.#syncTools();
  }

  #setTool(tool) {
    this.tool = tool;
    if (tool !== 'pen') this.eraser = false;
    this.#syncTools();
  }

  /** Draw on this layer from now on (it shows again if you'd hidden it). */
  #setLayer(l) {
    this.layer = l;
    this.hidden.delete(l);
    this.#syncTools();
    this.#paint();
  }

  #toggleHidden(l) {
    if (!this.hidden.delete(l)) this.hidden.add(l);
    this.#syncTools();
    this.#paint();
  }

  #toggleClear(button) {
    if (!this.clearPop.hidden) return this.#closeClear();
    const n = this.layer + 1;
    this.clearPop.innerHTML = `
      <div class="pop-title"><span>Clear for everyone?</span></div>
      <p class="muted draw-clear-note">This page goes to the gallery first.</p>
      <div class="pop-actions">
        <button type="button" class="btn small danger" data-clear="layer">Clear layer ${n}</button>
        <button type="button" class="btn small danger" data-clear="page">Clear the page</button>
        <button type="button" class="btn small" data-clear="cancel">Cancel</button>
      </div>`;
    this.clearPop.hidden = false;
    const r = button.getBoundingClientRect();
    placeNear(this.clearPop, button, { align: r.left + r.width / 2 > innerWidth / 2 ? 'right' : 'left' });
    // (Phones: Clear is in the bar under the page; open upwards when there's no room below.)
    if (r.bottom + 6 + this.clearPop.offsetHeight > innerHeight - 8) this.clearPop.style.top = `${Math.max(8, r.top - this.clearPop.offsetHeight - 6)}px`;
    this.clearPop.querySelector('[data-clear=cancel]').focus();
  }

  #closeClear() {
    this.clearPop.hidden = true;
  }

  /** Phones: colors, sizes and layers live in a palette under the brush button. */
  #togglePalette(button) {
    if (!this.palette.hidden) return this.#closePalette();
    this.#closeClear();
    this.palette.hidden = false;
    button.setAttribute('aria-expanded', 'true');
    placeNear(this.palette, button);
  }

  #closePalette() {
    this.palette.hidden = true;
    this.root.querySelector('[data-tool=palette]').setAttribute('aria-expanded', 'false');
  }

  /**
   * Desktop: everything in one toolbar (colors, sizes, tools, layers, then the
   * actions on the right). Phones: one row of tools; colors, sizes and layers
   * move into the palette, and the actions sit in a bar under the page.
   */
  #layout() {
    const q = (sel) => this.root.querySelector(sel);
    const [tools, colors, sizes, shapes, wiggle, layers, actions, stage] = ['.draw-tools', '.draw-colors', '.draw-sizes', '.draw-shapes', '[data-tool=wiggle]', '.draw-layers', '.draw-actions', '.draw-stage'].map(q);
    if (NARROW.matches) {
      const label = q('.draw-palette-label');
      label.before(colors, sizes);
      label.after(layers);
      stage.after(actions);
    } else {
      this.#closePalette();
      tools.insertBefore(colors, shapes);
      tools.insertBefore(sizes, shapes);
      wiggle.after(layers);
      tools.append(actions);
    }
    this.root.classList.toggle('narrow', NARROW.matches);
    this.#resize();
  }

  #syncTools() {
    for (const b of this.root.querySelectorAll('[data-color]')) b.setAttribute('aria-pressed', String(!this.eraser && b.dataset.color === this.color));
    for (const b of this.root.querySelectorAll('[data-size]')) b.setAttribute('aria-pressed', String(Number(b.dataset.size) === this.size));
    this.root.querySelector('[data-tool=eraser]').setAttribute('aria-pressed', String(this.eraser));
    this.root.querySelector('[data-tool=wiggle]').setAttribute('aria-pressed', String(this.wiggle));
    for (const b of this.root.querySelectorAll('[data-shape]')) b.setAttribute('aria-pressed', String(b.dataset.shape === this.tool));
    for (const b of this.root.querySelectorAll('[data-layer]')) b.setAttribute('aria-pressed', String(Number(b.dataset.layer) === this.layer));
    for (const b of this.root.querySelectorAll('[data-eye]')) {
      const l = Number(b.dataset.eye);
      const hidden = this.hidden.has(l);
      b.innerHTML = icon(hidden ? 'eye-off' : 'eye');
      b.setAttribute('aria-pressed', String(hidden));
      b.title = hidden ? `Show layer ${l + 1}` : `Hide layer ${l + 1} (just for you)`;
      b.setAttribute('aria-label', b.title);
      b.closest('.layer-chip').classList.toggle('hidden-layer', hidden);
    }
    this.paper.classList.toggle('erasing', this.eraser);
    this.paper.classList.toggle('typing', this.tool === 'text');
    const brush = this.root.querySelector('[data-tool=palette]');
    brush.style.setProperty('--c', this.color);
    brush.querySelector('.draw-brush-layer').textContent = String(this.layer + 1);
    brush.title = `Color, size and layer (drawing on layer ${this.layer + 1})`;
  }

  // ── Input ────────────────────────────────────────────────────────────────

  #toBoard(e) {
    const r = this.paper.getBoundingClientRect();
    return [Math.round(((e.clientX - r.left) / r.width) * BOARD_W), Math.round(((e.clientY - r.top) / r.height) * BOARD_H)];
  }

  #bindPointer() {
    const canvas = this.paper;
    canvas.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 || this.tool === 'text' || e.target.tagName !== 'CANVAS') return;
      // Another finger while one draws: ignored. A primary pointer means the last stroke's
      // end never came (its finger is long gone), so that one is kept first.
      if (this.drawing) {
        if (!e.isPrimary) return;
        finish();
      }
      canvas.setPointerCapture(e.pointerId);
      // Drawing on a layer you'd hidden shows it again.
      if (this.hidden.delete(this.layer)) this.#syncTools();
      if (this.tool !== 'pen') {
        const [x, y] = this.#toBoard(e);
        this.drawing = { pointer: e.pointerId, sk: id(), c: this.color, w: this.size, e: false, l: this.layer, shape: this.tool, x0: x, y0: y, ...shapeStroke(this.tool, x, y, x, y) };
        this.live.set('me', this.drawing);
        return this.#scheduleLive();
      }
      this.drawing = { pointer: e.pointerId, sk: id(), c: this.color, w: this.size, e: this.eraser, l: this.layer, p: [...this.#toBoard(e)], sent: 0, lastSend: 0 };
      this.live.set('me', this.drawing);
      this.#pump(true);
      this.#scheduleLive();
    });
    canvas.addEventListener('pointermove', (e) => {
      if (this.drawing?.pointer !== e.pointerId) return;
      if (this.drawing.shape) {
        const [x, y] = this.#toBoard(e);
        Object.assign(this.drawing, { k: undefined }, shapeStroke(this.drawing.shape, this.drawing.x0, this.drawing.y0, x, y));
        return this.#scheduleLive();
      }
      for (const ev of e.getCoalescedEvents?.() ?? [e]) this.drawing.p.push(...this.#toBoard(ev));
      this.#pump(false);
      this.#scheduleLive();
    });
    const finish = () => {
      const d = this.drawing;
      if (!d) return;
      this.drawing = null;
      this.live.delete('me');
      const stroke = { id: d.sk, uid: this.hooks.me(), c: d.c, w: d.w, e: d.e, p: d.p, ...(d.k ? { k: d.k } : {}), ...(d.l ? { l: d.l } : {}) };
      this.#add(stroke);
    };
    const end = (e) => this.drawing?.pointer === e.pointerId && finish();
    canvas.addEventListener('pointerup', end);
    canvas.addEventListener('pointercancel', end);
    // Text opens on click, after the browser has finished moving focus around.
    canvas.addEventListener('click', (e) => {
      if (this.tool === 'text' && e.target.tagName === 'CANVAS') this.#startText(e);
    });
  }

  /** Keep a finished stroke (pen, shape or text) and send it. */
  #add(stroke) {
    this.strokes.push(stroke);
    this.mine.push(stroke.id);
    this.#commit(stroke);
    const { id: sk, uid, ...rest } = stroke;
    this.hooks.send({ t: 'draw.stroke', sk, ...rest });
    this.#scheduleLive();
  }

  /** Text tool: a little box where you clicked; Enter places it, Esc cancels. */
  #startText(e) {
    const input = this.root.querySelector('.draw-text');
    const [x, y] = this.#toBoard(e);
    const r = this.paper.getBoundingClientRect();
    const l = this.layer;
    if (this.hidden.delete(l)) {
      this.#syncTools();
      this.#paint();
    }
    Object.assign(input.style, { left: `${e.clientX - r.left}px`, top: `${e.clientY - r.top}px`, color: this.color, fontSize: `${(textSize(this.size) * r.width) / BOARD_W}px` });
    input.value = '';
    input.hidden = false;
    input.focus();
    const done = (keep) => {
      input.onkeydown = input.onblur = null;
      const text = input.value.trim();
      input.hidden = true;
      if (keep && text) this.#add({ id: id(), uid: this.hooks.me(), c: this.color, w: this.size, e: false, p: [x, y], k: 'text', text, ...(l ? { l } : {}) });
    };
    input.onkeydown = (ev) => {
      if (ev.key === 'Enter') done(true);
      if (ev.key === 'Escape') done(false);
    };
    input.onblur = () => done(true);
  }

  /** Send the new points of the stroke in progress (with one point of overlap so pieces join). */
  #pump(force) {
    const d = this.drawing;
    if (!d || (!force && Date.now() - d.lastSend < SEND_EVERY_MS)) return;
    const from = Math.max(0, d.sent - 2);
    if (d.p.length - from < 2) return;
    this.hooks.send({ t: 'draw.seg', sk: d.sk, c: d.c, w: d.w, e: d.e, p: d.p.slice(from), ...(d.l ? { l: d.l } : {}) });
    d.sent = d.p.length;
    d.lastSend = Date.now();
  }

  // ── Rendering ────────────────────────────────────────────────────────────

  #resize() {
    const stage = this.root.querySelector('.draw-stage');
    if (!stage.clientWidth) return;
    const ratio = BOARD_W / BOARD_H;
    // clientWidth/Height include the stage's padding; the page fits inside it.
    const pad = getComputedStyle(stage);
    const innerW = stage.clientWidth - parseFloat(pad.paddingLeft) - parseFloat(pad.paddingRight);
    let innerH = stage.clientHeight - parseFloat(pad.paddingTop) - parseFloat(pad.paddingBottom);
    if (pad.flexGrow === '0') {
      // Phones: the page sits at the top, as wide as it can be; the stage hugs it,
      // so the room it may take is what the tools, the actions and a row of gallery leave.
      const taken = ['.draw-tools', '.draw-actions'].reduce((h, sel) => h + (this.root.querySelector(sel)?.offsetHeight ?? 0), 0);
      innerH = this.root.clientHeight - taken - 64 - parseFloat(pad.paddingTop) - parseFloat(pad.paddingBottom);
    }
    const w = Math.max(0, Math.min(innerW, innerH * ratio));
    this.paper.style.width = `${w}px`;
    this.paper.style.height = `${w / ratio}px`;
    // Nothing to redraw: the page is always 640 × 400 pixels, and CSS scales it.
  }

  /** Re-render every layer and frame from scratch (undo, clear, wipe, resize, wiggle toggle). */
  #redrawCommitted() {
    const count = this.wiggle ? WIGGLE_FRAMES : 1;
    for (let l = 0; l < LAYERS; l++) {
      for (let f = 0; f < count; f++) {
        const ctx = this.frames[l][f].getContext('2d');
        ctx.clearRect(0, 0, PIX_W, PIX_H);
        for (const s of this.strokes) if (layerOf(s) === l) this.#drawStroke(ctx, s, undefined, this.wiggle ? f : -1);
        crisp(ctx);
      }
    }
    this.#paint();
    this.#thumbs();
  }

  /** Add one finished stroke to every frame of its layer. */
  #commit(stroke) {
    const count = this.wiggle ? WIGGLE_FRAMES : 1;
    for (let f = 0; f < count; f++) {
      const ctx = this.frames[layerOf(stroke)][f].getContext('2d');
      this.#drawStroke(ctx, stroke, undefined, this.wiggle ? f : -1);
      crisp(ctx);
    }
    this.#paint();
    this.#thumbs();
  }

  #scheduleLive() {
    this.raf ||= requestAnimationFrame(() => {
      this.raf = 0;
      this.#paint();
    });
  }

  /**
   * Show the current frame: each layer's sheet = its committed frame plus the
   * strokes being drawn on it (a live eraser really erases, on its own layer
   * only), and name tags at the tip of other people's strokes.
   */
  #paint() {
    const now = Date.now();
    const live = Array.from({ length: LAYERS }, () => []);
    let tags = '';
    for (const [key, s] of this.live) {
      if (key !== 'me' && now - s.at > 8000) {
        this.live.delete(key); // a stroke whose end we never got
        continue;
      }
      live[layerOf(s)]?.push({ ...s, id: s.sk ?? key });
      if (key !== 'me' && s.who && s.p.length >= 2 && !this.hidden.has(layerOf(s))) {
        const x = (s.p[s.p.length - 2] / BOARD_W) * 100;
        const y = (s.p[s.p.length - 1] / BOARD_H) * 100;
        tags += `<span class="draw-tag" style="left:${x}%;top:${y}%;--c:${esc(s.who.color)}">${esc(s.who.name)}</span>`;
      }
    }
    const frame = this.wiggle ? this.frame : 0;
    this.sheets.forEach((sheet, l) => {
      sheet.hidden = this.hidden.has(l);
      if (sheet.hidden) return;
      const ctx = sheet.getContext('2d');
      ctx.clearRect(0, 0, PIX_W, PIX_H);
      ctx.drawImage(this.frames[l][frame], 0, 0);
      if (!live[l].length) return;
      for (const s of live[l]) this.#drawStroke(ctx, s, undefined, this.wiggle ? this.frame : -1);
      crisp(ctx);
    });
    if (tags !== this.tagsHtml) {
      this.tagsHtml = tags;
      this.tags.innerHTML = tags;
    }
  }

  /**
   * The little picture of each layer in the toolbar, after the committed page
   * changes. Drawn from the strokes at thumbnail scale (every line at least a
   * pixel wide), not shrunk from the page, where thin lines would vanish.
   */
  #thumbs() {
    this.thumbRaf ||= requestAnimationFrame(() => {
      this.thumbRaf = 0;
      const sheet = Object.assign(document.createElement('canvas'), { width: THUMB_W, height: THUMB_H });
      const sctx = sheet.getContext('2d');
      this.thumbs.forEach((thumb, l) => {
        sctx.clearRect(0, 0, THUMB_W, THUMB_H);
        for (const s of this.strokes) if (layerOf(s) === l) this.#drawStroke(sctx, s, THUMB_W / BOARD_W);
        const ctx = thumb.getContext('2d');
        ctx.fillStyle = PAPER;
        ctx.fillRect(0, 0, THUMB_W, THUMB_H);
        ctx.drawImage(sheet, 0, 0);
      });
    });
  }

  /** @param frame wiggle frame (−1 = drawn straight) */
  #drawStroke(ctx, s, scale = PIX_W / BOARD_W, frame = -1) {
    if (s.k === 'text') {
      // Text wiggles too: the same position-based nudge, plus a hint of tilt.
      const [x, y] = s.p;
      const dx = frame >= 0 ? field(x, y, frame, 0) : 0;
      const dy = frame >= 0 ? field(x, y, frame, 1) : 0;
      ctx.save();
      ctx.translate((x + dx) * scale, (y + dy) * scale);
      if (frame >= 0) ctx.rotate(field(x + 500, y, frame, 2) / 900);
      ctx.font = `${textSize(s.w) * scale}px ${TEXT_FONT}`;
      ctx.textBaseline = 'top';
      ctx.fillStyle = s.c;
      ctx.fillText(s.text ?? '', 0, 0);
      ctx.restore();
      return;
    }
    const p = frame >= 0 && s.p.length > 2 ? wobble(s.p, s.id ?? '', frame) : s.p;
    if (p.length < 2) return;
    ctx.save();
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    ctx.lineWidth = Math.max(1, s.w * scale);
    if (s.e) ctx.globalCompositeOperation = 'destination-out';
    ctx.strokeStyle = ctx.fillStyle = s.e ? '#000' : s.c;
    ctx.beginPath();
    if (p.length === 2) {
      ctx.arc(p[0] * scale, p[1] * scale, ctx.lineWidth / 2, 0, Math.PI * 2);
      ctx.fill();
    } else if (s.k === 'poly') {
      ctx.moveTo(p[0] * scale, p[1] * scale);
      for (let i = 2; i < p.length; i += 2) ctx.lineTo(p[i] * scale, p[i + 1] * scale);
      ctx.stroke();
    } else {
      ctx.moveTo(p[0] * scale, p[1] * scale);
      for (let i = 2; i < p.length - 2; i += 2) {
        const mx = ((p[i] + p[i + 2]) / 2) * scale;
        const my = ((p[i + 1] + p[i + 3]) / 2) * scale;
        ctx.quadraticCurveTo(p[i] * scale, p[i + 1] * scale, mx, my);
      }
      ctx.lineTo(p[p.length - 2] * scale, p[p.length - 1] * scale);
      ctx.stroke();
    }
    ctx.restore();
  }

  #renderGallery() {
    this.galleryEl.innerHTML = this.gallery.length
      ? `<span class="gallery-label">Gallery</span>${this.gallery
        .map((d) => {
          const who = this.hooks.who(d.uid)?.name ?? 'Someone';
          // The same date as Space's gallery viewer ("Oct 7, 2026, 8:42 PM"), and who drew it for screen readers too.
          const when = new Date(d.ts).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' });
          return `<a class="gallery-item" href="/media/${esc(d.media)}" target="_blank" rel="noopener" title="${esc(`${who} · ${when}`)}"><img src="/media/${esc(d.media)}" alt="${esc(`Drawing by ${who}, ${when}`)}" loading="lazy"></a>`;
        })
        .join('')}`
      : '<span class="gallery-label">Saved drawings show up here.</span>';
  }
}
