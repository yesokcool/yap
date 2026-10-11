/*
 * Quick switcher (Ctrl/⌘+K; PROTOCOL.md › Extras): type a few letters of a
 * channel, place, person or action and press Enter. The last row always
 * searches messages for what you typed.
 */
import { esc } from './format.js';
import { icon } from './icons.js';

/** Fuzzy score: every letter of `q` in order; earlier, word-start and consecutive hits win. 0 = no match. */
export function score(q, text) {
  if (!q) return 1;
  const t = text.toLowerCase();
  if (t.startsWith(q)) return 1000 - t.length;
  const word = t.indexOf(` ${q}`);
  if (word >= 0) return 800 - word;
  const at = t.indexOf(q);
  if (at >= 0) return 600 - at;
  let i = 0;
  let run = 0;
  let total = 0;
  for (let j = 0; j < t.length && i < q.length; j++) {
    if (t[j] === q[i]) {
      i++;
      run++;
      total += run * 2 + (j === 0 || t[j - 1] === ' ' || t[j - 1] === '-' ? 5 : 0);
    } else run = 0;
  }
  return i === q.length ? total : 0;
}

export class QuickSwitcher {
  /**
   * @param {HTMLDialogElement} dialog
   * @param {{ items: () => {label: string, sub?: string, icon: string, kind: string, run: () => void}[], search: (q: string) => void }} hooks
   */
  constructor(dialog, hooks) {
    this.dialog = dialog;
    this.hooks = hooks;
    this.active = 0;
    dialog.innerHTML = `
      <form method="dialog" class="switcher" autocomplete="off">
        <input class="switcher-input" type="text" placeholder="Jump to a channel, place or person…" aria-label="Jump to" spellcheck="false"
          role="combobox" aria-expanded="true" aria-autocomplete="list" aria-controls="switcher-list">
        <ul class="switcher-list" id="switcher-list" role="listbox" aria-label="Matches"></ul>
        <p class="switcher-hint muted"><kbd>↑</kbd><kbd>↓</kbd> to move · <kbd>Enter</kbd> to open · <kbd>Esc</kbd> to close</p>
      </form>`;
    this.input = dialog.querySelector('input');
    this.list = dialog.querySelector('ul');
    this.input.addEventListener('input', () => {
      this.active = 0;
      this.render();
    });
    this.input.addEventListener('keydown', (e) => {
      const n = this.results.length;
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        this.active = (this.active + (e.key === 'ArrowDown' ? 1 : -1) + n) % n;
        this.render();
      } else if (e.key === 'Enter') {
        e.preventDefault();
        this.pick(this.active);
      }
    });
    this.list.addEventListener('click', (e) => {
      const row = e.target.closest('[data-i]');
      if (row) this.pick(Number(row.dataset.i));
    });
    this.list.addEventListener('pointermove', (e) => {
      const row = e.target.closest('[data-i]');
      if (row && Number(row.dataset.i) !== this.active) {
        this.active = Number(row.dataset.i);
        this.render();
      }
    });
  }

  open() {
    this.input.value = '';
    this.active = 0;
    this.render();
    if (!this.dialog.open) this.dialog.showModal();
    this.input.focus();
  }

  pick(i) {
    const hit = this.results[i];
    if (!hit) return;
    this.dialog.close();
    hit.run();
  }

  render() {
    const q = this.input.value.trim().toLowerCase();
    const ranked = this.hooks.items()
      .map((item) => ({ item, s: Math.max(score(q, item.label), item.sub ? score(q, item.sub) * 0.5 : 0) }))
      .filter((r) => r.s > 0)
      .sort((a, b) => b.s - a.s)
      .slice(0, 8)
      .map((r) => r.item);
    const text = this.input.value.trim();
    this.results = [...ranked, { label: text ? `Search messages for “${text}”` : 'Search messages…', icon: 'search', kind: 'search', run: () => this.hooks.search(text) }];
    this.active = Math.min(this.active, this.results.length - 1);
    this.list.innerHTML = this.results.map((r, i) => `
      <li role="option" id="switcher-opt-${i}" class="switcher-row ${i === this.active ? 'active' : ''}" data-i="${i}" aria-selected="${i === this.active}">
        ${icon(r.icon)}<span class="switcher-label">${esc(r.label)}</span>${r.sub ? `<small class="muted">${esc(r.sub)}</small>` : ''}<span class="spacer"></span><small class="switcher-kind">${esc(r.kind)}</small>
      </li>`).join('');
    this.list.querySelector('.active')?.scrollIntoView({ block: 'nearest' });
    this.input.setAttribute('aria-activedescendant', `switcher-opt-${this.active}`); // (screen readers read the picked row)
  }
}
