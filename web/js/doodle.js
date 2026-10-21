/*
 * Doodle reactions (PROTOCOL.md › Extras › Doodle reactions): a tiny pixel
 * pad that floats next to a message. 48 × 48 pixels, drawn big, saved as a
 * 96 × 96 GIF — three wobbly frames when Wiggle is on, like Draw.
 */
import { encodeGif } from './gif.js';
import { placeNear } from './place.js';

const SIZE = 48;
const SCALE = 2; // the saved GIF is 96 × 96: crisp at reaction size
const PENS = ['#111111', '#e5484d', '#f76b15', '#f5c400', '#30a46c', '#0090ff', '#8e4ec6', '#d6409f'];
const PAPER = '#ffffff';
const FRAMES = 3;

/** -1, 0 or 1, the same every time for the same inputs (so frames are stable). */
const jitter = (a, b, c) => {
  const x = Math.sin(a * 12.9898 + b * 78.233 + c * 37.719) * 43758.5453;
  return Math.floor((x - Math.floor(x)) * 3) - 1;
};

export class DoodlePad {
  /**
   * @param {HTMLElement} el the floating container
   * @param {{ done: (blob: Blob) => Promise<void> }} hooks
   */
  constructor(el, hooks) {
    this.el = el;
    this.hooks = hooks;
    this.strokes = [];
    this.pen = PENS[0];
    this.erasing = false;
    this.wiggle = true;
    this.frame = 0;
    this.render();
    this.canvas = el.querySelector('canvas');
    this.ctx = this.canvas.getContext('2d');
    this.bind();
  }

  get isOpen() {
    return !this.el.hidden;
  }

  open(near) {
    this.strokes = [];
    this.el.hidden = false;
    this.paint();
    placeNear(this.el, near, { align: 'right' });
    clearInterval(this.timer);
    this.timer = setInterval(() => {
      if (!this.wiggle || this.el.hidden) return;
      this.frame = (this.frame + 1) % FRAMES;
      this.paint();
    }, 140);
  }

  close() {
    this.el.hidden = true;
    clearInterval(this.timer);
  }

  render() {
    this.el.innerHTML = `
      <div class="doodle-head"><b>Doodle a reaction</b><span class="spacer"></span><button type="button" class="icon-btn tiny" data-doodle="close" aria-label="Close" title="Close">×</button></div>
      <canvas width="${SIZE * 4}" height="${SIZE * 4}" aria-label="Drawing pad"></canvas>
      <div class="doodle-pens" role="group" aria-label="Colors">
        ${PENS.map((c) => `<button type="button" class="swatch" style="--c:${c}" data-doodle="pen" data-color="${c}" aria-label="Pen ${c}" aria-pressed="${c === this.pen}"></button>`).join('')}
      </div>
      <div class="doodle-tools" role="group" aria-label="Tools">
        <button type="button" class="btn small" data-doodle="draw" aria-pressed="true">Pen</button>
        <button type="button" class="btn small" data-doodle="erase" aria-pressed="false">Eraser</button>
        <button type="button" class="btn small" data-doodle="wiggle" aria-pressed="true" title="Wobbly lines, like WigglyPaint">Wiggle</button>
      </div>
      <div class="doodle-actions">
        <button type="button" class="btn small" data-doodle="undo">Undo</button>
        <button type="button" class="btn small" data-doodle="clear">Clear</button>
        <span class="spacer"></span>
        <button type="button" class="btn small primary" data-doodle="send">React</button>
      </div>`;
  }

  bind() {
    const at = (e) => {
      const r = this.canvas.getBoundingClientRect();
      return [Math.floor(((e.clientX - r.left) / r.width) * SIZE), Math.floor(((e.clientY - r.top) / r.height) * SIZE)];
    };
    this.canvas.addEventListener('pointerdown', (e) => {
      e.preventDefault();
      if (this.drawing && !e.isPrimary) return; // a second finger while one draws (Draw does the same)
      this.canvas.setPointerCapture(e.pointerId);
      this.drawing = { pointer: e.pointerId, color: this.erasing ? PAPER : this.pen, size: this.erasing ? 3 : 1, points: [at(e)] };
      this.strokes.push(this.drawing);
      this.paint();
    });
    this.canvas.addEventListener('pointermove', (e) => {
      if (this.drawing?.pointer !== e.pointerId) return;
      const p = at(e);
      const last = this.drawing.points.at(-1);
      if (p[0] === last[0] && p[1] === last[1]) return;
      this.drawing.points.push(p);
      this.paint();
    });
    const end = (e) => {
      if (this.drawing?.pointer === e.pointerId) this.drawing = null;
    };
    this.canvas.addEventListener('pointerup', end);
    this.canvas.addEventListener('pointercancel', end);
    this.el.addEventListener('click', async (e) => {
      const b = e.target.closest('[data-doodle]');
      if (!b) return;
      const what = b.dataset.doodle;
      if (what === 'close') return this.close();
      if (what === 'pen') {
        this.pen = b.dataset.color;
        this.erasing = false;
      }
      if (what === 'draw') this.erasing = false;
      if (what === 'erase') this.erasing = !this.erasing;
      if (what === 'wiggle') this.wiggle = !this.wiggle;
      if (what === 'undo') this.strokes.pop();
      if (what === 'clear') this.strokes = [];
      if (what === 'send') {
        if (!this.strokes.some((s) => s.color !== PAPER)) return;
        b.disabled = true;
        try {
          if (await this.hooks.done(this.gif())) this.close();
        } finally {
          b.disabled = false;
        }
        return;
      }
      for (const p of this.el.querySelectorAll('[data-doodle=pen]')) p.setAttribute('aria-pressed', String(!this.erasing && p.dataset.color === this.pen));
      this.el.querySelector('[data-doodle=draw]').setAttribute('aria-pressed', String(!this.erasing));
      this.el.querySelector('[data-doodle=erase]').setAttribute('aria-pressed', String(this.erasing));
      this.el.querySelector('[data-doodle=wiggle]').setAttribute('aria-pressed', String(this.wiggle));
      this.paint();
    });
  }

  /** The 48 × 48 picture for one frame (-1 = steady, no wiggle). */
  pixels(frame) {
    const c = new OffscreenCanvas(SIZE, SIZE);
    const ctx = c.getContext('2d');
    ctx.fillStyle = PAPER;
    ctx.fillRect(0, 0, SIZE, SIZE);
    this.strokes.forEach((s, si) => {
      ctx.fillStyle = s.color;
      const pts = s.points.map(([x, y], pi) => (frame > 0 ? [x + jitter(si, pi, frame), y + jitter(pi, si, frame + 7)] : [x, y]));
      const dot = (x, y) => ctx.fillRect(x - Math.floor(s.size / 2), y - Math.floor(s.size / 2), s.size, s.size);
      pts.forEach(([x, y], i) => {
        if (!i) return dot(x, y);
        // Bresenham between points: hard pixel lines.
        let [x0, y0] = pts[i - 1];
        const dx = Math.abs(x - x0);
        const dy = -Math.abs(y - y0);
        const sx = x0 < x ? 1 : -1;
        const sy = y0 < y ? 1 : -1;
        let err = dx + dy;
        for (;;) {
          dot(x0, y0);
          if (x0 === x && y0 === y) break;
          const e2 = 2 * err;
          if (e2 >= dy) {
            err += dy;
            x0 += sx;
          }
          if (e2 <= dx) {
            err += dx;
            y0 += sy;
          }
        }
      });
    });
    return c;
  }

  paint() {
    this.ctx.imageSmoothingEnabled = false;
    this.ctx.drawImage(this.pixels(this.wiggle ? this.frame : -1), 0, 0, this.canvas.width, this.canvas.height);
  }

  gif() {
    const out = new OffscreenCanvas(SIZE * SCALE, SIZE * SCALE);
    const ctx = out.getContext('2d');
    ctx.imageSmoothingEnabled = false;
    const frames = Array.from({ length: this.wiggle ? FRAMES : 1 }, (_, f) => {
      ctx.drawImage(this.pixels(this.wiggle ? f : -1), 0, 0, out.width, out.height);
      return ctx.getImageData(0, 0, out.width, out.height).data;
    });
    return new Blob([encodeGif(frames, out.width, out.height, 140)], { type: 'image/gif' });
  }
}

/** A doodle reaction token's media id, or null. */
export const doodleId = (emoji) => /^<d:([a-f0-9]{32})>$/.exec(emoji ?? '')?.[1] ?? null;
