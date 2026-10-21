/*
 * Live pointers on the embedded board (SELF_HOSTING.md › A board).
 *
 * The board is served same-origin at /board, so Yap can watch the mouse
 * inside the iframe and draw everyone else's pointers on top of it without
 * the board's own code knowing. Positions are anchored to content — a card
 * (li[data-id]) or a column (ul[data-lane]), as a fraction of its box — so a
 * pointer lands on the same card for everyone even though people use
 * different window sizes and board densities.
 */
import { esc } from './format.js';

const SEND_EVERY_MS = 60;
const STALE_MS = 30_000;

export class BoardCursors {
  /**
   * @param {HTMLIFrameElement} frame
   * @param {{ send: (anchor: object|null) => void, who: (sid: string) => {name: string, color: string} | null }} hooks
   */
  constructor(frame, hooks) {
    this.frame = frame;
    this.hooks = hooks;
    this.remote = new Map(); // sid -> { a, at }
    this.last = null; // last local pointer (client coords) for re-anchoring on scroll
    this.lastSent = 0;
    this.pending = null;
    this.doc = null;
    frame.addEventListener('load', () => this.#attach());
    setInterval(() => this.#sweep(), 5000);
  }

  update(sid, anchor) {
    if (anchor) this.remote.set(sid, { a: anchor, at: Date.now() });
    else this.remote.delete(sid);
    this.#render();
  }

  remove(sid) {
    this.remote.delete(sid);
    this.#render();
  }

  clear() {
    this.remote.clear();
    this.#render();
  }

  /** Stop sharing our pointer (e.g. switched back to chat). */
  hide() {
    this.last = null;
    this.hooks.send(null);
  }

  #attach() {
    const doc = this.frame.contentDocument;
    if (!doc) return;
    this.doc = doc;
    const style = doc.createElement('style');
    style.textContent = `
      #yap-cursors { position: fixed; inset: 0; pointer-events: none; z-index: 2147483647; overflow: hidden; }
      .yap-cursor { position: absolute; left: 0; top: 0; transition: transform 90ms linear; will-change: transform; }
      .yap-cursor svg { display: block; filter: drop-shadow(0 1px 1.5px rgb(0 0 0 / 0.35)); }
      .yap-cursor span { position: absolute; left: 14px; top: 16px; padding: 2px 7px; border-radius: 7px; color: #fff;
        font: 600 12px/1.4 ui-sans-serif, system-ui, -apple-system, sans-serif; white-space: nowrap; box-shadow: 0 1px 3px rgb(0 0 0 / 0.3); }
      /* Names are Windows 98 tooltips, like Draw's name tags. */
      #yap-cursors .yap-cursor span { padding: 0 4px; border-radius: 0; background: #ffffe1 !important; color: #000;
        font: 11px/16px var(--yap-font, Tahoma, sans-serif); box-shadow: 0 0 0 1px #000; }
      #yap-cursors .yap-cursor span::before { content: ''; display: inline-block; width: 7px; height: 7px; margin-right: 4px;
        background: var(--c); box-shadow: 0 0 0 1px #000; }`;
    doc.head.append(style);
    const layer = doc.createElement('div');
    layer.id = 'yap-cursors';
    doc.body.append(layer);
    this.layer = layer;

    doc.addEventListener('mousemove', (e) => {
      this.last = { x: e.clientX, y: e.clientY };
      this.#queueSend();
    }, { passive: true });
    // Left the board (for Yap's own page, another window…). A mouseout with nowhere to go says so in
    // every browser; not every browser sends a document a mouseleave.
    doc.addEventListener('mouseout', (e) => {
      if (e.relatedTarget || !this.last) return;
      this.last = null;
      clearTimeout(this.pending);
      this.pending = null;
      this.hooks.send(null);
    });
    // Content moved under a still pointer, and others' anchors moved on screen.
    doc.addEventListener('scroll', () => {
      if (this.last) this.#queueSend();
      this.#render();
    }, { capture: true, passive: true });
    this.frame.contentWindow.addEventListener('resize', () => this.#render());
    // The board re-renders itself (votes, saves): keep pointers on their cards.
    new MutationObserver((changes) => {
      if (changes.some((c) => !layer.contains(c.target))) this.#render();
    }).observe(doc.body, { childList: true, subtree: true });
    this.#render();
  }

  #queueSend() {
    const wait = SEND_EVERY_MS - (Date.now() - this.lastSent);
    if (wait <= 0) return this.#sendNow();
    this.pending ??= setTimeout(() => this.#sendNow(), wait);
  }

  #sendNow() {
    clearTimeout(this.pending);
    this.pending = null;
    this.lastSent = Date.now();
    this.hooks.send(this.last ? this.#anchorAt(this.last.x, this.last.y) : null);
  }

  #anchorAt(x, y) {
    const doc = this.doc;
    const el = doc.elementFromPoint(x, y);
    const card = el?.closest('[data-lane] [data-id]');
    if (card) return this.#relative('card', card.dataset.id, card, x, y);
    const lane = el?.closest('[data-lane]');
    if (lane) return this.#relative('lane', lane.dataset.lane, lane, x, y);
    const root = doc.documentElement;
    return { k: 'page', x: (x + doc.defaultView.scrollX) / Math.max(1, root.clientWidth), y: (y + doc.defaultView.scrollY) / Math.max(1, root.scrollHeight) };
  }

  #relative(k, id, el, x, y) {
    const r = el.getBoundingClientRect();
    return { k, id, x: (x - r.left) / Math.max(1, r.width), y: (y - r.top) / Math.max(1, r.height) };
  }

  /** Anchor → point in the iframe's viewport, or null if that card isn't shown here. */
  #point(a) {
    const doc = this.doc;
    if (a.k === 'page') {
      const root = doc.documentElement;
      return { x: a.x * root.clientWidth - doc.defaultView.scrollX, y: a.y * root.scrollHeight - doc.defaultView.scrollY };
    }
    const sel = a.k === 'card' ? `[data-lane] [data-id="${CSS.escape(a.id)}"]` : `[data-lane="${CSS.escape(a.id)}"]`;
    const el = doc.querySelector(sel);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: r.left + a.x * r.width, y: r.top + a.y * r.height };
  }

  #render() {
    if (!this.layer) return;
    if (!this.layer.style.getPropertyValue('--yap-font')) {
      this.layer.style.setProperty('--yap-font', getComputedStyle(document.documentElement).getPropertyValue('--font').trim() || 'sans-serif');
    }
    const seen = new Set();
    for (const [sid, { a }] of this.remote) {
      const who = this.hooks.who(sid);
      const p = who && this.#point(a);
      let node = this.layer.querySelector(`[data-sid="${CSS.escape(sid)}"]`);
      if (!p) {
        node?.remove();
        continue;
      }
      seen.add(sid);
      if (!node) {
        node = this.doc.createElement('div');
        node.className = 'yap-cursor';
        node.dataset.sid = sid;
        this.layer.append(node);
      }
      const color = esc(who.color);
      const html = `<svg width="18" height="20" viewBox="0 0 18 20"><path d="M1 1l6.5 17 2.6-7.1L17 8.3z" fill="${color}" stroke="#fff" stroke-width="1.5" stroke-linejoin="round"/></svg><span style="--c:${color};background:${color}">${esc(who.name)}</span>`;
      if (node.dataset.html !== html) {
        node.innerHTML = html;
        node.dataset.html = html;
      }
      node.style.transform = `translate(${p.x}px, ${p.y}px)`;
    }
    for (const node of this.layer.querySelectorAll('.yap-cursor')) if (!seen.has(node.dataset.sid)) node.remove();
  }

  #sweep() {
    if (!this.remote.size) return;
    const now = Date.now();
    for (const [sid, { at }] of this.remote) if (now - at > STALE_MS) this.remote.delete(sid);
    this.#render();
  }
}
