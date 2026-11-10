/*
 * The shared drawing board. Everyone on the "draw" view sees strokes as
 * they're drawn (draw.seg pieces), finished strokes are stored on the server
 * (draw.stroke).
 *
 * One drawing is on the easel at a time, the same for everyone. Next to it is
 * a bulletin board of the drawings people saved (PROTOCOL.md › Draw drawings
 * and the board): "Save to board" pins a picture of the one on the easel,
 * clicking a pinned one puts it back on the easel for everyone, and "New
 * drawing" starts a blank one. Whatever leaves the easel is pinned first, so
 * nothing drawn is lost. History shows earlier versions of the drawing on the
 * easel: look at one (just you), then go back to it for everyone.
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
/**
 * What the server keeps of a stroke (worker/validate.ts › cleanStroke): at most
 * this many numbers (x, y pairs), each no further than this past the page. A
 * stroke outside either is refused whole, so the pen stays inside both.
 */
const STROKE_NUMBERS = 8000;
const OFF_PAGE = 500;
const clampTo = (n, max) => Math.min(max + OFF_PAGE, Math.max(-OFF_PAGE, n));
const LAYERS = 3;
const LAYER_NAMES = ['bottom', 'middle', 'top'];
const THUMB_W = 40;
const THUMB_H = 25;
/** Every layer (a picture for the board keeps them all, whatever you've hidden). */
const ALL_LAYERS = Array.from({ length: LAYERS }, (_, l) => l);
/** The easel's card on the board: the drawing as it is right now. */
const CARD_W = 160;
const CARD_H = 100;
const PIN_COLORS = ['#e5484d', '#3e63dd', '#30a46c', '#f5d90a', '#8e4ec6', '#f76b15'];
/** A pinned card's slight tilt, in degrees: the same for everyone. */
const tilt = (id) => ((((id * 7919) % 9) + 9) % 9 - 4) * 0.55;
/** "3:12 PM" today, "Nov 8, 3:12 PM" before. */
function when(ts) {
  const d = new Date(ts);
  const time = d.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  return d.toDateString() === new Date().toDateString() ? time : `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })}, ${time}`;
}
/** Save a file to this device: the share sheet inside the iPhone/Mac app (its web view has no downloads), else a download. */
async function saveFile(blob, name) {
  const file = new File([blob], name, { type: blob.type });
  if (document.documentElement.dataset.embed && navigator.canShare?.({ files: [file] })) {
    try {
      await navigator.share({ files: [file] });
    } catch {} // (closed without picking anything)
    return;
  }
  const a = Object.assign(document.createElement('a'), { href: URL.createObjectURL(blob), download: name });
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 5000);
}
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
   * @param {{ send: (msg: object) => void, upload: (blob: Blob, params: object) => Promise<object | null>, share: (blob: Blob) => Promise<void>,
   *           who: (uid: string) => {name: string, color: string} | null, whoSid: (sid: string) => {name: string, color: string} | null,
   *           me: () => string, toast: (t: string) => void }} hooks
   */
  constructor(root, hooks) {
    this.root = root;
    this.hooks = hooks;
    this.strokes = []; // committed, in order (the drawing on the easel)
    this.live = new Map(); // key (sid:sk or 'me') -> { c, w, e, p, at, who }
    this.mine = []; // my stroke ids, for undo
    this.pageId = null; // the drawing on the easel
    this.board = []; // the drawings on the board, taken down, and the easel's (DrawPage)
    this.versions = null; // the easel drawing's history, once asked for
    this.preview = null; // an earlier version you're looking at: { id, ts, why, uid, n, strokes }
    this.wanted = null; // the version asked for, until it comes
    this.popKind = null; // what the popover shows
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
    this.easelThumb = Object.assign(document.createElement('canvas'), { width: CARD_W, height: CARD_H, className: 'easel-thumb' });

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
          <button class="btn small danger" data-tool="clear" aria-haspopup="dialog" title="Clear a layer or the whole drawing for everyone (History can bring it back)">Clear</button>
        </div>
      </div>
      <div class="popover draw-palette" role="dialog" aria-label="Color, size and layer" hidden>
        <div class="pop-title"><span>Color, size and layer</span></div>
        <p class="draw-palette-label muted">Layer <small>— the eye hides one just for you</small></p>
      </div>
      <div class="popover draw-pop" role="dialog" hidden></div>
      <div class="draw-main">
        <div class="draw-easel">
          <div class="easel-bar">
            <div class="easel-now">
              <button class="btn small easel-new" data-tool="new" title="Start a blank drawing for everyone. This one stays on the board.">${icon('plus')}<span>New drawing</span></button>
              <span class="easel-name"><b class="easel-title"></b> <span class="easel-status"></span></span>
              <span class="easel-buttons">
                <button class="btn small" data-tool="save" title="Pin this drawing to the board, or update its picture there">${icon('pin')}<span>Save to board</span></button>
                <button class="btn small" data-tool="download" aria-haspopup="dialog" title="Save a picture of it to this device">Download</button>
                <button class="btn small" data-tool="share" title="Post this drawing in the chat">Share</button>
                <button class="btn small" data-tool="history" aria-haspopup="dialog" title="Earlier versions of this drawing, to go back to">History</button>
              </span>
            </div>
            <div class="easel-then" role="status" hidden></div>
          </div>
          <div class="draw-stage">
            <div class="draw-paper">
              ${Array.from({ length: LAYERS }, (_, l) => `<canvas class="draw-layer" data-layer-canvas="${l}"></canvas>`).join('')}
              <div class="draw-tags"></div>
              <input class="draw-text" maxlength="200" placeholder="Type, then Enter" hidden>
            </div>
          </div>
        </div>
        <section class="draw-board" aria-label="Bulletin board">
          <div class="cork-head">${icon('pin')} Bulletin board</div>
          <div class="cork"></div>
        </section>
      </div>`;
    this.paper = root.querySelector('.draw-paper');
    this.sheets = [...root.querySelectorAll('.draw-layer')];
    for (const c of this.sheets) readable(PIX_W, PIX_H, c);
    this.thumbs = [...root.querySelectorAll('.layer-thumb')];
    this.pop = root.querySelector('.draw-pop');
    this.palette = root.querySelector('.draw-palette');
    this.tags = root.querySelector('.draw-tags');
    this.cork = root.querySelector('.cork');
    this.#bindTools();
    this.#bindBoard();
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
        if (!this.pop.hidden || !this.palette.hidden) {
          this.#closePop();
          this.#closePalette();
        } else if (this.preview) this.#endPreview();
      }
    });
    addEventListener('pointerdown', (e) => {
      if (!this.pop.hidden && !e.target.closest('.draw-pop') && !this.popFor?.contains(e.target)) this.#closePop();
      if (!this.palette.hidden && !e.target.closest('.draw-palette, [data-tool=palette]')) this.#closePalette();
    });
    this.#syncTools();
    this.#renderBoard();
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

  /** The drawing on the easel and the board: when you open Draw, and for everyone when the easel changes. */
  onState({ canvas, strokes, board, by, why, at }) {
    const moved = this.pageId !== null && canvas !== this.pageId;
    this.pageId = canvas;
    this.strokes = strokes;
    this.mine = strokes.filter((s) => s.uid === this.hooks.me()).map((s) => s.id);
    this.#dropLive();
    this.board = Array.isArray(board) ? board : [];
    this.versions = null;
    this.preview = null;
    this.wanted = null;
    if (moved || why === 'restore') this.#closePop();
    this.#renderThen();
    this.#renderBoard();
    this.#redrawCommitted();
    this.#scheduleLive();
    // The easel moved: show its card on the board (it may be far down, or off to the side).
    if (moved) this.#showEaselCard();
    if (!by || !why) return;
    const mine = by === this.hooks.me();
    const name = this.hooks.who(by)?.name ?? 'Someone';
    if (why === 'restore') this.hooks.toast(mine ? `Went back to ${when(at)} for everyone. History has what was there before.` : `${name} took this drawing back to ${when(at)}. History has what was there before.`);
    else if (mine) return;
    else if (why === 'new') this.hooks.toast(`${name} started a new drawing. The one before is on the board.`);
    else if (why === 'open') this.hooks.toast(`${name} put Drawing ${canvas} on the easel.`);
  }

  /** The board changed: a save, a drawing taken down or put back. */
  onBoard({ canvas, board }) {
    if (canvas !== this.pageId || !Array.isArray(board)) return;
    this.board = board;
    this.#renderBoard();
  }

  onHistory({ page, versions }) {
    if (page !== this.pageId) return;
    this.versions = versions;
    if (this.popKind === 'history') this.#showHistory();
  }

  /** An earlier version you asked to see: shown just for you until you go back to it or close it. */
  onVersion({ id, page, ts, strokes }) {
    if (id !== this.wanted || page !== this.pageId) return;
    this.wanted = null;
    const v = this.versions?.find((x) => x.id === id) ?? { id, ts, why: 'auto', n: strokes.length };
    this.preview = { ...v, ts, strokes };
    this.#closePop();
    this.#renderThen();
    this.#redrawCommitted();
  }

  onSeg({ sid, sk, c, w, e, p, l, pg }) {
    if (pg != null && pg !== this.pageId) return;
    const key = `${sid}:${sk}`;
    const entry = this.live.get(key) ?? { sk, c, w, e, l: l ?? 0, p: [], who: this.hooks.whoSid(sid) };
    entry.p.push(...p);
    entry.at = Date.now();
    this.live.set(key, entry);
    this.#scheduleLive();
  }

  onStroke({ stroke, pg }) {
    for (const [key] of this.live) if (key.endsWith(`:${stroke.id}`)) this.live.delete(key);
    if ((pg != null && pg !== this.pageId) || this.strokes.some((s) => s.id === stroke.id)) return this.#scheduleLive();
    this.strokes.push(stroke);
    this.#commit(stroke);
    this.#changed();
    this.#scheduleLive();
  }

  onUndo({ id: strokeId }) {
    const before = this.strokes.length;
    this.strokes = this.strokes.filter((s) => s.id !== strokeId);
    this.mine = this.mine.filter((x) => x !== strokeId);
    if (this.strokes.length === before) return;
    this.#changed();
    this.#redrawCommitted();
  }

  /** One layer emptied for everyone (`all`: each of them in turn, the whole drawing cleared). */
  onWipe({ l, by, pg, all }) {
    if (pg != null && pg !== this.pageId) return;
    this.strokes = this.strokes.filter((s) => layerOf(s) !== l);
    const left = new Set(this.strokes.map((s) => s.id));
    this.mine = this.mine.filter((x) => left.has(x));
    this.#changed();
    this.#redrawCommitted();
    const name = this.hooks.who(by)?.name ?? 'Someone';
    if (by === this.hooks.me() || (all && l !== 0)) return;
    this.hooks.toast(all ? `${name} cleared the whole drawing. History can bring it back.` : `${name} cleared layer ${l + 1}. History can bring it back.`);
  }

  /** Forget other people's strokes in progress (a fresh page or state); the one you're drawing stays. */
  #dropLive() {
    for (const key of this.live.keys()) if (key !== 'me') this.live.delete(key);
  }

  /** The drawing on the easel, as the board knows it. */
  #easel() {
    return this.board.find((p) => p.id === this.pageId) ?? null;
  }

  /** Something on the easel changed: its picture on the board is out of date. */
  #changed() {
    const page = this.#easel();
    if (page && !page.dirty) {
      page.dirty = true;
      this.#renderStatus();
    }
  }

  // ── Actions ──────────────────────────────────────────────────────────────

  undo() {
    if (this.preview) return;
    const last = this.mine.pop();
    if (last) this.hooks.send({ t: 'draw.undo', id: last });
  }

  /** Download what you see (hidden layers left out), as a PNG or the wiggling GIF. */
  async #download(gif) {
    const layers = this.#shown();
    if (!this.#anyOn(layers)) return this.hooks.toast('Draw something first.');
    const blob = gif ? this.#gif(layers) : await this.#png(layers);
    await saveFile(blob, `yap-drawing-${this.pageId}-${new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-')}.${gif ? 'gif' : 'png'}`);
  }

  /** A drawing's picture from the board, saved to this device. */
  async #downloadPinned(page) {
    try {
      const res = await fetch(`/media/${encodeURIComponent(page.media)}`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const blob = await res.blob();
      await saveFile(blob, `yap-drawing-${page.id}.${blob.type === 'image/gif' ? 'gif' : 'png'}`);
    } catch (err) {
      this.hooks.toast(`Couldn't download it: ${err.message}`);
    }
  }

  /** A picture of the whole drawing on the easel (every layer) becomes its picture on the board; `pin` puts it (back) up. */
  async #pinPicture(pin) {
    return this.hooks.upload(await this.#png(ALL_LAYERS), { page: String(this.pageId), pin: pin ? '1' : '0' });
  }

  async #saveToBoard() {
    if (!this.strokes.length) return this.hooks.toast('Draw something first.');
    const page = this.#easel();
    const up = page?.pinnedAt && !page.downAt;
    if (up && !page.dirty && page.media) return this.hooks.toast('The board already has this drawing as it is.');
    await this.#once(async () => {
      if (await this.#pinPicture(true)) this.hooks.toast(up ? 'Updated its picture on the board.' : 'Pinned to the board.');
    });
  }

  /**
   * Before the easel changes: the drawing on it goes on the board as it is now,
   * unless the board has that already (or it's blank). False when that failed.
   */
  async #keepEasel() {
    const page = this.#easel();
    if (!this.strokes.length || (page && !page.dirty && page.media)) return true;
    return Boolean(await this.#pinPicture(!page?.downAt));
  }

  async #newDrawing() {
    if (this.preview) this.#endPreview();
    if (!this.strokes.length) return this.hooks.toast("This one's blank already: draw away. The others are on the board.");
    await this.#once(async () => {
      if (!(await this.#keepEasel())) return this.hooks.toast("Couldn't pin this drawing to the board, so nothing changed. Try again in a moment.");
      this.hooks.send({ t: 'draw.new', from: this.pageId });
    });
  }

  /** Put a drawing from the board on the easel, for everyone. */
  async #openPage(id) {
    if (id === this.pageId) return;
    await this.#once(async () => {
      if (!(await this.#keepEasel())) return this.hooks.toast("Couldn't pin the drawing on the easel to the board, so it stayed. Try again in a moment.");
      this.hooks.send({ t: 'draw.open', page: id, from: this.pageId });
    });
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
      if (!e.target.closest('.draw-tools, .draw-palette, .draw-actions, .easel-bar')) return;
      const color = e.target.closest('[data-color]')?.dataset.color;
      const size = e.target.closest('[data-size]')?.dataset.size;
      const button = e.target.closest('[data-tool]');
      const tool = button?.dataset.tool;
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
      if (tool === 'palette') this.#togglePalette(button);
      if (size) this.size = Number(size);
      if (tool === 'eraser') this.#setEraser(!this.eraser);
      if (tool === 'wiggle') {
        this.wiggle = !this.wiggle;
        try { localStorage.setItem('yap.wiggle', this.wiggle ? '1' : '0'); } catch {}
        this.#redrawCommitted();
      }
      if (tool === 'undo') this.undo();
      if (tool === 'new') await this.#newDrawing();
      if (tool === 'save') await this.#saveToBoard();
      if (tool === 'download') this.#togglePop('download', button);
      if (tool === 'history') this.#togglePop('history', button);
      if (tool === 'clear') this.#togglePop('clear', button);
      if (tool === 'share') {
        if (!this.#anyOn(this.#shown())) return this.hooks.toast('Draw something first.');
        // With Wiggle on, the chat gets the wiggling GIF.
        await this.#once(async () => this.hooks.share(this.wiggle ? this.#gif(this.#shown()) : await this.#png(this.#shown())));
      }
      if (tool === 'restore' && this.preview) this.hooks.send({ t: 'draw.restore', id: this.preview.id });
      if (tool === 'now') this.#endPreview();
      this.#syncTools();
    });
    this.pop.addEventListener('click', async (e) => {
      const button = e.target.closest('[data-pop]');
      if (!button) return;
      const what = button.dataset.pop;
      const page = this.board.find((p) => p.id === Number(button.dataset.page));
      if (what === 'version') {
        this.wanted = button.dataset.version;
        this.hooks.send({ t: 'draw.version', id: this.wanted });
        return;
      }
      this.#closePop();
      if (what === 'png' || what === 'gif') await this.#download(what === 'gif');
      if (what === 'clear-layer' || what === 'clear-all') {
        const all = what === 'clear-all';
        if (!this.strokes.some((s) => all || layerOf(s) === this.layer)) return this.hooks.toast('Nothing to clear.');
        this.hooks.send({ t: 'draw.wipe', l: all ? 0 : this.layer, pg: this.pageId, ...(all ? { all } : {}) });
      }
      if (!page) return;
      if (what === 'open') await this.#openPage(page.id);
      if (what === 'get') page.id === this.pageId ? await this.#download(false) : await this.#downloadPinned(page);
      if (what === 'down' || what === 'up') this.hooks.send({ t: 'draw.pin', page: page.id, on: what === 'up' });
    });
    this.root.querySelector('input[type=color]').addEventListener('input', (e) => {
      this.color = e.target.value;
      this.#setEraser(false);
      this.#syncTools();
    });
  }

  /** Save, Share, New and the easel's moves one at a time: a double click makes one picture, not two. */
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

  // ── Popovers: Clear, Download, History, a card on the board, what was taken down ──

  #togglePop(kind, button, page) {
    if (!this.pop.hidden && this.popKind === kind && this.popFor === button) return this.#closePop();
    this.#closePalette();
    this.popKind = kind;
    this.popFor = button;
    this.popPage = page;
    if (kind === 'history') {
      this.versions = null;
      this.hooks.send({ t: 'draw.history' });
    }
    this.#fillPop();
    this.pop.hidden = false;
    this.#placePop();
    this.pop.querySelector('button')?.focus();
  }

  #placePop() {
    const r = this.popFor.getBoundingClientRect();
    placeNear(this.pop, this.popFor, { align: r.left + r.width / 2 > innerWidth / 2 ? 'right' : 'left' });
  }

  #closePop() {
    this.pop.hidden = true;
    this.popKind = null;
    this.popFor = null;
  }

  #showHistory() {
    this.#fillPop();
    this.#placePop();
  }

  #fillPop() {
    const kind = this.popKind;
    this.pop.className = `popover draw-pop draw-pop-${kind}`;
    const title = (t) => `<div class="pop-title"><span>${esc(t)}</span></div>`;
    const note = (t) => `<p class="muted draw-pop-note">${t}</p>`;
    if (kind === 'clear') {
      this.pop.innerHTML = `${title('Clear for everyone?')}${note('History can bring it back.')}
        <div class="pop-actions">
          <button type="button" class="btn small danger" data-pop="clear-layer">Clear layer ${this.layer + 1}</button>
          <button type="button" class="btn small danger" data-pop="clear-all">Clear the whole drawing</button>
          <button type="button" class="btn small" data-pop="cancel">Cancel</button>
        </div>`;
    } else if (kind === 'download') {
      this.pop.innerHTML = `${title('Download')}
        <div class="pop-actions">
          <button type="button" class="btn small" data-pop="png">Picture (PNG)</button>
          <button type="button" class="btn small" data-pop="gif">Wiggling GIF</button>
        </div>${this.hidden.size ? note('What you see: hidden layers are left out.') : ''}`;
    } else if (kind === 'history') {
      const list = this.versions;
      const rows = !list
        ? note('Loading…')
        : !list.length
          ? note("Nothing kept yet. Versions are kept every few minutes while people draw, when it's saved to the board, and before a layer is cleared.")
          : `<div class="draw-versions">${list.map((v) => `
              <button type="button" class="draw-version" data-pop="version" data-version="${esc(v.id)}"${v.now ? ' disabled' : ''}>
                <span class="draw-version-pic">${v.media ? `<img src="/media/${esc(v.media)}" alt="" loading="lazy">` : ''}</span>
                <span class="draw-version-text"><b>${esc(when(v.ts))}</b>${v.now ? ' <span class="draw-version-now">Now</span>' : ''}<br>
                <span class="muted">${esc(this.#versionLabel(v))} · ${v.n} ${v.n === 1 ? 'stroke' : 'strokes'}</span></span>
              </button>`).join('')}</div>`;
      this.pop.innerHTML = `${title(`History of Drawing ${this.pageId}`)}${list?.length ? note('Pick one to look at it (only you see it), then go back to it for everyone if you want.') : ''}${rows}`;
    } else if (kind === 'card') {
      const p = this.popPage;
      const easel = p.id === this.pageId;
      const by = this.#names(easel ? this.#artistsNow() : p.artists ?? []);
      const saved = p.savedAt ? `saved ${when(p.savedAt)}` : 'not saved yet';
      const actions = p.downAt
        ? `<button type="button" class="btn small" data-pop="up" data-page="${p.id}">Put back on the board</button>`
        : `${easel ? '' : `<button type="button" class="btn small" data-pop="open" data-page="${p.id}">Put on the easel</button>`}
           ${easel || p.media ? `<button type="button" class="btn small" data-pop="get" data-page="${p.id}">Download</button>` : ''}
           ${p.pinnedAt ? `<button type="button" class="btn small" data-pop="down" data-page="${p.id}">Take down</button>` : ''}`;
      const hint = easel
        ? "It's on the easel now."
        : p.downAt
          ? `Taken down by ${esc(this.hooks.who(p.downBy)?.name ?? 'someone')}, ${esc(when(p.downAt))}.`
          : `Everyone on Draw switches to it. Drawing ${this.pageId}, on the easel now, stays on the board.`;
      this.pop.innerHTML = `${title(`Drawing ${p.id}`)}${note(`${by ? `By ${esc(by)} · ` : ''}${esc(saved)}`)}<div class="pop-actions">${actions}</div>${note(hint)}`;
    } else if (kind === 'down') {
      const down = this.board.filter((p) => p.downAt);
      this.pop.innerHTML = `${title('Taken down')}${note('Drawings taken off the board. Put one back to pin it where it was.')}
        <div class="draw-down-list">${down.map((p) => `
          <div class="draw-down-row">
            <span class="draw-version-pic">${p.media ? `<img src="/media/${esc(p.media)}" alt="" loading="lazy">` : ''}</span>
            <span class="draw-version-text"><b>Drawing ${p.id}</b><br><span class="muted">by ${esc(this.hooks.who(p.downBy)?.name ?? 'someone')}, ${esc(when(p.downAt))}</span></span>
            <button type="button" class="btn small" data-pop="up" data-page="${p.id}">Put back</button>
          </div>`).join('')}</div>`;
    }
  }

  #versionLabel(v) {
    const name = this.hooks.who(v.uid)?.name ?? 'someone';
    if (v.why === 'save') return `Saved to the board by ${name}`;
    if (v.why === 'wipe') return `Before ${name} cleared a layer`;
    if (v.why === 'restore') return `Before ${name} went back`;
    if (v.why === 'leave') return `When ${name} switched drawings`;
    return 'Kept while drawing';
  }

  /** "Ada", "Ada & Ben", "Ada, Ben +2". */
  #names(uids) {
    const names = uids.map((u) => this.hooks.who(u)?.name).filter(Boolean);
    if (names.length <= 2) return names.join(' & ');
    return `${names.slice(0, 2).join(', ')} +${names.length - 2}`;
  }

  /** Who drew what's on the easel now. */
  #artistsNow() {
    return [...new Set(this.strokes.map((s) => s.uid))];
  }

  /** Phones: colors, sizes and layers live in a palette under the brush button. */
  #togglePalette(button) {
    if (!this.palette.hidden) return this.#closePalette();
    this.#closePop();
    this.palette.hidden = false;
    button.setAttribute('aria-expanded', 'true');
    placeNear(this.palette, button);
  }

  #closePalette() {
    this.palette.hidden = true;
    this.root.querySelector('[data-tool=palette]').setAttribute('aria-expanded', 'false');
  }

  // ── The bulletin board ───────────────────────────────────────────────────

  #bindBoard() {
    this.cork.addEventListener('click', (e) => {
      const card = e.target.closest('[data-card]');
      const page = card && this.board.find((p) => p.id === Number(card.dataset.card));
      if (page) return this.#togglePop('card', card, page);
      const down = e.target.closest('[data-down-list]');
      if (down) this.#togglePop('down', down);
    });
  }

  /**
   * The drawings pinned to the board, newest first. The one on the easel shows
   * as it is right now (first, when it isn't pinned yet), and what was taken
   * down is behind a note at the end.
   */
  #renderBoard() {
    const easel = this.#easel() ?? (this.pageId === null ? null : { id: this.pageId });
    const pinned = this.board.filter((p) => p.pinnedAt && !p.downAt);
    const down = this.board.filter((p) => p.downAt && p.id !== this.pageId);
    const cards = [];
    if (easel && !pinned.some((p) => p.id === easel.id)) cards.push(this.#card(easel));
    for (const p of pinned) cards.push(this.#card(p));
    if (!pinned.length) cards.push('<p class="cork-empty">Save to board pins the drawing on the easel here. Click a pinned one to put it back on the easel.</p>');
    if (down.length) cards.push(`<button type="button" class="cork-down" data-down-list>Taken down (${down.length})</button>`);
    this.cork.innerHTML = cards.join('');
    this.cork.querySelector('.pin-card.on-easel .pin-photo')?.append(this.easelThumb);
    if (this.popKind === 'card' || this.popKind === 'down') this.#closePop(); // (its buttons may be for a card that moved)
    this.#renderStatus();
  }

  /** Scroll the board (only the board, never the page around it) so the easel's card is in view. */
  #showEaselCard() {
    const card = this.cork.querySelector('.pin-card.on-easel');
    if (!card) return;
    const c = this.cork.getBoundingClientRect();
    const r = card.getBoundingClientRect();
    if (r.top < c.top || r.bottom > c.bottom) this.cork.scrollTop += r.top - c.top - 16;
    if (r.left < c.left || r.right > c.right) this.cork.scrollLeft += r.left - c.left - 16;
  }

  #card(p) {
    const easel = p.id === this.pageId;
    const ghost = easel && (!p.pinnedAt || p.downAt);
    const by = this.#names(easel ? this.#artistsNow() : p.artists ?? []);
    const day = new Date(p.savedAt ?? p.pinnedAt ?? p.createdAt ?? Date.now()).toLocaleDateString([], { month: 'short', day: 'numeric' });
    const photo = easel ? '' : p.media ? `<img src="/media/${esc(p.media)}" alt="" loading="lazy" draggable="false">` : '<span class="pin-blank">No picture yet</span>';
    const label = `Drawing ${p.id}${by ? ` by ${by}` : ''}${easel ? ', on the easel' : `, ${day}`}`;
    return `<button type="button" class="pin-card${easel ? ' on-easel' : ''}${ghost ? ' ghost' : ''}" data-card="${p.id}" style="--tilt:${tilt(p.id)}deg;--pin:${PIN_COLORS[p.id % PIN_COLORS.length]}" aria-label="${esc(label)}" title="${esc(label)}">
      <i class="pushpin"></i>
      <span class="pin-photo">${photo}</span>
      <span class="pin-caption"><b>Drawing ${p.id}</b>${by ? ` · ${esc(by)}` : ''}</span>
      <span class="pin-date">${easel ? '● On the easel' : esc(day)}</span>
    </button>`;
  }

  /** The easel bar: which drawing is on the easel, and whether the board has it as it is. */
  #renderStatus() {
    const page = this.#easel();
    this.root.querySelector('.easel-title').textContent = this.pageId === null ? 'Draw' : `Drawing ${this.pageId}`;
    const status = !page
      ? ''
      : !page.pinnedAt
        ? this.strokes.length ? 'not on the board yet' : 'new'
        : page.downAt
          ? 'taken down from the board'
          : page.dirty ? 'on the board · changed since it was saved' : 'on the board';
    this.root.querySelector('.easel-status').textContent = status ? `— ${status}` : '';
  }

  /** Looking at an earlier version: the bar over the page says which, and offers to go back to it. */
  #renderThen() {
    const bar = this.root.querySelector('.easel-then');
    const now = this.root.querySelector('.easel-now');
    const v = this.preview;
    bar.hidden = !v;
    now.hidden = Boolean(v);
    this.paper.classList.toggle('then', Boolean(v));
    if (!v) return;
    bar.innerHTML = `<span class="easel-then-text"><b>${esc(when(v.ts))}</b> — ${esc(this.#versionLabel(v))}. Only you see this.</span>
      <span class="easel-buttons">
        <button class="btn small" data-tool="restore" title="Put the drawing back like this for everyone. What's there now is kept in History.">Go back to this</button>
        <button class="btn small" data-tool="now">Back to now</button>
      </span>`;
  }

  #endPreview() {
    this.preview = null;
    this.wanted = null;
    this.#renderThen();
    this.#redrawCommitted();
    this.#scheduleLive();
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
    // (A captured pointer keeps drawing past the page's edge, over the tools: the line stops just outside it.)
    return [clampTo(Math.round(((e.clientX - r.left) / r.width) * BOARD_W), BOARD_W), clampTo(Math.round(((e.clientY - r.top) / r.height) * BOARD_H), BOARD_H)];
  }

  /** Looking at an earlier version: no drawing on it (it isn't the real page). */
  #lookingBack() {
    if (!this.preview) return false;
    this.hooks.toast("You're looking at an earlier version. Back to now to draw.");
    return true;
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
      if (this.#lookingBack() || this.pageId === null) return;
      canvas.setPointerCapture(e.pointerId);
      // Drawing on a layer you'd hidden shows it again.
      if (this.hidden.delete(this.layer)) this.#syncTools();
      if (this.tool !== 'pen') {
        const [x, y] = this.#toBoard(e);
        this.drawing = { pointer: e.pointerId, sk: id(), pg: this.pageId, c: this.color, w: this.size, e: false, l: this.layer, shape: this.tool, x0: x, y0: y, ...shapeStroke(this.tool, x, y, x, y) };
        this.live.set('me', this.drawing);
        return this.#scheduleLive();
      }
      this.drawing = { pointer: e.pointerId, sk: id(), pg: this.pageId, c: this.color, w: this.size, e: this.eraser, l: this.layer, p: [...this.#toBoard(e)], sent: 0, lastSend: 0 };
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
      for (const ev of e.getCoalescedEvents?.() ?? [e]) {
        const d = this.drawing;
        const [x, y] = this.#toBoard(ev);
        if (x === d.p.at(-2) && y === d.p.at(-1)) continue; // (a fast mouse reports the same spot many times)
        d.p.push(x, y);
        if (d.p.length < STROKE_NUMBERS) continue;
        // As long as a stroke may be: it's kept, and the line goes on as a new one from here.
        finish();
        this.drawing = { pointer: d.pointer, sk: id(), pg: d.pg, c: d.c, w: d.w, e: d.e, l: d.l, p: [x, y], sent: 0, lastSend: 0 };
        this.live.set('me', this.drawing);
      }
      this.#pump(false);
      this.#scheduleLive();
    });
    const finish = () => {
      const d = this.drawing;
      if (!d) return;
      this.drawing = null;
      this.live.delete('me');
      const stroke = { id: d.sk, uid: this.hooks.me(), c: d.c, w: d.w, e: d.e, p: d.p, ...(d.k ? { k: d.k } : {}), ...(d.l ? { l: d.l } : {}) };
      this.#add(stroke, d.pg);
    };
    const end = (e) => this.drawing?.pointer === e.pointerId && finish();
    canvas.addEventListener('pointerup', end);
    canvas.addEventListener('pointercancel', end);
    // Text opens on click, after the browser has finished moving focus around.
    canvas.addEventListener('click', (e) => {
      if (this.tool === 'text' && e.target.tagName === 'CANVAS' && !this.#lookingBack()) this.#startText(e);
    });
  }

  /**
   * Keep a finished stroke (pen, shape or text) and send it, for the drawing it
   * was started on: when the easel changed meanwhile, it stays on that one.
   */
  #add(stroke, pg = this.pageId) {
    const { id: sk, uid, ...rest } = stroke;
    this.hooks.send({ t: 'draw.stroke', sk, ...rest, pg });
    if (pg !== this.pageId) return this.#scheduleLive();
    this.strokes.push(stroke);
    this.mine.push(stroke.id);
    this.#commit(stroke);
    this.#changed();
    this.#scheduleLive();
  }

  /** Text tool: a little box where you clicked; Enter places it, Esc cancels. */
  #startText(e) {
    const input = this.root.querySelector('.draw-text');
    const [x, y] = this.#toBoard(e);
    const r = this.paper.getBoundingClientRect();
    const l = this.layer;
    const pg = this.pageId;
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
      if (keep && text) this.#add({ id: id(), uid: this.hooks.me(), c: this.color, w: this.size, e: false, p: [x, y], k: 'text', text, ...(l ? { l } : {}) }, pg);
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
    this.hooks.send({ t: 'draw.seg', sk: d.sk, c: d.c, w: d.w, e: d.e, p: d.p.slice(from), ...(d.l ? { l: d.l } : {}), pg: d.pg });
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
      // so the room it may take is what the tools, the easel bar, the actions and a row of the board leave.
      const taken = ['.draw-tools', '.easel-bar', '.draw-actions', '.cork-head'].reduce((h, sel) => h + (this.root.querySelector(sel)?.offsetHeight ?? 0), 0);
      innerH = this.root.clientHeight - taken - 132 - parseFloat(pad.paddingTop) - parseFloat(pad.paddingBottom);
    }
    const w = Math.max(0, Math.min(innerW, innerH * ratio));
    this.paper.style.width = `${w}px`;
    this.paper.style.height = `${w / ratio}px`;
    // Nothing to redraw: the page is always 640 × 400 pixels, and CSS scales it.
  }

  /** What the page shows: the drawing on the easel, or the earlier version you're looking at. */
  #pageStrokes() {
    return this.preview?.strokes ?? this.strokes;
  }

  /** Re-render every layer and frame from scratch (undo, clear, wipe, resize, wiggle toggle). */
  #redrawCommitted() {
    const count = this.wiggle ? WIGGLE_FRAMES : 1;
    const strokes = this.#pageStrokes();
    for (let l = 0; l < LAYERS; l++) {
      for (let f = 0; f < count; f++) {
        const ctx = this.frames[l][f].getContext('2d');
        ctx.clearRect(0, 0, PIX_W, PIX_H);
        for (const s of strokes) if (layerOf(s) === l) this.#drawStroke(ctx, s, undefined, this.wiggle ? f : -1);
        crisp(ctx);
      }
    }
    this.#paint();
    this.#thumbs();
  }

  /** Add one finished stroke to every frame of its layer. */
  #commit(stroke) {
    if (!this.preview) {
      const count = this.wiggle ? WIGGLE_FRAMES : 1;
      for (let f = 0; f < count; f++) {
        const ctx = this.frames[layerOf(stroke)][f].getContext('2d');
        this.#drawStroke(ctx, stroke, undefined, this.wiggle ? f : -1);
        crisp(ctx);
      }
      this.#paint();
    }
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
   * only), and name tags at the tip of other people's strokes. An earlier
   * version you're looking at shows without them.
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
      if (this.preview || (s.pg != null && s.pg !== this.pageId)) continue;
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
   * The little pictures after the committed page changes: each layer in the
   * toolbar, and the easel's card on the board (every layer, as it is now).
   * Drawn from the strokes at their own scale (every line at least a pixel
   * wide), not shrunk from the page, where thin lines would vanish.
   */
  #thumbs() {
    this.thumbRaf ||= requestAnimationFrame(() => {
      this.thumbRaf = 0;
      const sheet = Object.assign(document.createElement('canvas'), { width: THUMB_W, height: THUMB_H });
      const sctx = sheet.getContext('2d');
      const strokes = this.#pageStrokes();
      this.thumbs.forEach((thumb, l) => {
        sctx.clearRect(0, 0, THUMB_W, THUMB_H);
        for (const s of strokes) if (layerOf(s) === l) this.#drawStroke(sctx, s, THUMB_W / BOARD_W);
        const ctx = thumb.getContext('2d');
        ctx.fillStyle = PAPER;
        ctx.fillRect(0, 0, THUMB_W, THUMB_H);
        ctx.drawImage(sheet, 0, 0);
      });
      const card = this.easelThumb.getContext('2d');
      card.fillStyle = PAPER;
      card.fillRect(0, 0, CARD_W, CARD_H);
      const layer = Object.assign(document.createElement('canvas'), { width: CARD_W, height: CARD_H });
      const lctx = layer.getContext('2d');
      for (let l = 0; l < LAYERS; l++) {
        lctx.clearRect(0, 0, CARD_W, CARD_H);
        for (const s of this.strokes) if (layerOf(s) === l) this.#drawStroke(lctx, s, CARD_W / BOARD_W);
        card.drawImage(layer, 0, 0);
      }
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
}
