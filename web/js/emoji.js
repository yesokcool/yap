/*
 * Emojis: the standard set (unicode, shown in the system emoji font) plus
 * custom ones uploaded by the server's members.
 *
 * Custom emojis are stored in message text as `<:name:id>` tokens (like
 * Discord), so renaming or deleting one never breaks old messages: the id is
 * the image's content hash and /emoji/<id> serves it forever. Standard
 * emojis are just the characters themselves. People type `:name:` either
 * way; we swap in the token or character on send.
 */
import { esc } from './format.js';
import { upload } from './settings.js';
import { confirmSheet } from './sheet.js';

const MAX_EMOJI_BYTES = 512 * 1024;
const SIDE = 128; // static images are scaled down to this
const TOKEN_RE = /<:([A-Za-z0-9_]{2,32}):([a-f0-9]{32})>/g;
const CODE = '[A-Za-z0-9_+-]{2,32}';
const RECENT_KEY = 'yap.recentEmoji';
const GROUP_ICONS = ['😀', '🧑', '🐻', '🍔', '✈️', '⚽', '💡', '🔣', '🏁'];

// ── Standard emoji data (web/emoji-data.json, built by tools/build-emoji-data.mjs) ──

let standard = null; // { groups, list: [{ char, group, codes, words }], byCode }
let standardPending = null;

export function loadStandardEmojis() {
  standardPending ??= fetch('/emoji-data.json')
    .then((res) => res.json())
    .then((data) => {
      const list = data.emojis.map(([char, group, codes, words, skins]) => ({ char, group, codes: codes.split(' '), words, skins: skins ? skins.split(' ') : null }));
      const byCode = new Map();
      for (const e of list) for (const code of e.codes) if (!byCode.has(code)) byCode.set(code, e);
      standard = { groups: data.groups, list, byCode };
      return standard;
    })
    .catch((err) => {
      standardPending = null;
      throw err;
    });
  return standardPending;
}

// Skin tone for standard emojis that have one (0 = default yellow, 1–5 light → dark).
const TONE_KEY = 'yap.emojiTone';
const TONE_HANDS = ['✋', '✋🏻', '✋🏼', '✋🏽', '✋🏾', '✋🏿'];
/** Read once, not per emoji (the picker draws ~1,900 of them; the suggestions look at all of them per key). */
let tone = null;
function emojiTone() {
  if (tone === null) {
    try {
      tone = Math.min(5, Math.max(0, Number(localStorage.getItem(TONE_KEY)) || 0));
    } catch {
      tone = 0;
    }
  }
  return tone;
}
function setEmojiTone(value) {
  tone = value;
  try {
    localStorage.setItem(TONE_KEY, String(value));
  } catch {}
}
addEventListener('storage', (e) => e.key === TONE_KEY && (tone = null)); // (picked in another tab)
/** The character to insert for a standard emoji, in your skin tone. */
const toned = (e) => (e.skins && emojiTone() ? e.skins[emojiTone() - 1] : e.char);

/**
 * `:party:` → `<:party:id>` for custom emojis, `:thumbsup:` → 👍 for standard
 * ones (custom names win), outside code spans/blocks.
 */
export function encodeEmojis(text, byName) {
  const pattern = new RegExp(`(?<!<):(${CODE}):(?![a-f0-9]{32}>)`, 'g');
  return text
    .split(/(```[\s\S]*?```|`[^`\n]+`)/)
    .map((part, i) => (i % 2 ? part : part.replace(pattern, (whole, name) => {
      const custom = byName.get(name.toLowerCase());
      if (custom) return `<:${custom.name}:${custom.id}>`;
      const e = standard?.byCode.get(name.toLowerCase());
      return e ? toned(e) : whole;
    })))
    .join('');
}

/** Tokens back to plain `:name:` (notifications, previews). */
export const plainEmojis = (text) => text.replace(TOKEN_RE, ':$1:');

// ── Recently used (per device) ─────────────────────────────────────────────

function recentKeys() {
  try {
    return JSON.parse(localStorage.getItem(RECENT_KEY) || '[]');
  } catch {
    return [];
  }
}

function rememberEmoji(key) {
  const keys = [key, ...recentKeys().filter((k) => k !== key)].slice(0, 27);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(keys));
  } catch {}
}

// ── Uploading custom emojis ────────────────────────────────────────────────

function suggestName(filename) {
  const base = filename.replace(/\.[^.]+$/, '').toLowerCase().replace(/[^a-z0-9_]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 32);
  return base.length >= 2 ? base : `emoji_${base}`.slice(0, 32);
}

/**
 * Shrink big static images to 128 px. Animated GIF/WebP are kept as-is (a
 * canvas would flatten them), so they must already fit the size limit.
 */
async function prepareEmojiFile(file) {
  const maybeAnimated = file.type === 'image/gif' || file.type === 'image/webp';
  if (maybeAnimated) {
    if (file.size > MAX_EMOJI_BYTES) throw new Error(`${file.name}: animated emojis must be under 512 KB.`);
    return file;
  }
  let bitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    throw new Error(`${file.name}: not an image Yap can read (use PNG, GIF, WebP or JPEG).`);
  }
  if (file.size <= MAX_EMOJI_BYTES && Math.max(bitmap.width, bitmap.height) <= SIDE && file.type === 'image/png') {
    bitmap.close();
    return file;
  }
  const scale = Math.min(1, SIDE / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));
  const ctx = canvas.getContext('2d');
  ctx.imageSmoothingQuality = 'high';
  ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close(); // (decoded, a big photo is tens of MB; a bulk upload decodes one after another)
  const blob = await new Promise((resolve) => canvas.toBlob(resolve, 'image/webp', 0.92));
  const out = blob?.type === 'image/webp' ? blob : await new Promise((resolve) => canvas.toBlob(resolve, 'image/png'));
  if (!out || out.size > MAX_EMOJI_BYTES) throw new Error(`${file.name}: too big even after shrinking.`);
  return out;
}

export async function uploadEmoji(blob, name, sid, ut) {
  return (await upload('/api/emoji', blob, { name, sid, ut })).emoji;
}

// ── Picker ────────────────────────────────────────────────────────────────

const customTile = (x) => `
  <button type="button" class="emoji-tile" data-emoji-id="${esc(x.id)}" title=":${esc(x.name)}:" aria-label=":${esc(x.name)}:">
    <img src="/emoji/${esc(x.id)}" alt=":${esc(x.name)}:" loading="lazy" draggable="false">
    <span class="emoji-x" aria-hidden="true">×</span>
  </button>`;

const standardTile = (e) => {
  const char = toned(e);
  return `<button type="button" class="emoji-tile uni" data-uni="${esc(char)}" title=":${esc(e.codes[0])}:" aria-label=":${esc(e.codes[0])}:">${char}</button>`;
};

export class EmojiPicker {
  #el;
  #sections;
  #search;
  #scroll;
  #preview;
  #editing = false;
  #uploadRow;
  #pendingFile = null;

  /**
   * @param {HTMLElement} el
   * @param {{ emojis: () => object[], onPick: (pick: {char?: string, emoji?: object}, keepOpen: boolean) => void,
   *           toast: (text: string) => void, readonly?: boolean,
   *           onUpload?: (blob, name) => Promise<void>, onRename?: (emoji, name) => void, onDelete?: (emoji) => void }} hooks
   *   readonly: picking only (no Upload or Edit), so the last three aren't needed.
   */
  constructor(el, hooks) {
    this.#el = el;
    this.hooks = hooks;
    el.innerHTML = `
      <div class="gif-head">
        <input class="gif-search emoji-search" type="search" placeholder="Find an emoji" aria-label="Find an emoji">
        <div class="emoji-actions" ${hooks.readonly ? 'hidden' : ''}>
          <button type="button" class="btn small" data-emoji="upload">Upload</button>
          <button type="button" class="btn small" data-emoji="edit" aria-pressed="false" title="Rename (click) or delete (×) custom emojis">Edit</button>
          <input type="file" accept="image/png,image/gif,image/webp,image/jpeg" multiple hidden>
        </div>
        <div class="emoji-tones" role="radiogroup" aria-label="Skin tone">${TONE_HANDS.map((h, i) => `<button type="button" data-tone="${i}" aria-pressed="${i === emojiTone()}" title="${i ? `Skin tone ${i}` : 'Default'}">${h}</button>`).join('')}</div>
      </div>
      <form class="emoji-upload" hidden>
        <img alt="">
        <input name="name" maxlength="32" pattern="[A-Za-z0-9_]{2,32}" required aria-label="Emoji name" placeholder="name">
        <button class="btn small primary" type="submit">Add</button>
        <button class="btn small" type="button" data-emoji="cancel">Cancel</button>
        <p class="form-error" hidden></p>
      </form>
      <nav class="emoji-tabs" aria-label="Emoji categories"></nav>
      <div class="gif-scroll emoji-scroll"><div class="emoji-sections"></div></div>
      <div class="emoji-preview"><span class="pv-glyph"></span><span class="pv-name">Pick an emoji</span></div>`;
    this.#sections = el.querySelector('.emoji-sections');
    this.#search = el.querySelector('.emoji-search');
    this.#scroll = el.querySelector('.emoji-scroll');
    this.#preview = el.querySelector('.emoji-preview');
    this.#uploadRow = el.querySelector('.emoji-upload');
    const fileInput = el.querySelector('input[type=file]');

    this.#search.addEventListener('input', () => this.render());
    el.addEventListener('click', (e) => {
      const action = e.target.closest('[data-emoji]')?.dataset.emoji;
      if (action === 'upload') fileInput.click();
      if (action === 'edit') {
        this.#editing = !this.#editing;
        e.target.setAttribute('aria-pressed', String(this.#editing));
        this.#el.classList.toggle('editing', this.#editing);
      }
      if (action === 'cancel') this.#closeUpload();
      const tone = e.target.closest('[data-tone]');
      if (tone) {
        setEmojiTone(Number(tone.dataset.tone));
        for (const b of this.#el.querySelectorAll('[data-tone]')) b.setAttribute('aria-pressed', String(b === tone));
        return this.render();
      }
      const jump = e.target.closest('[data-jump]');
      if (jump) {
        const section = this.#sections.querySelector(`[data-section="${jump.dataset.jump}"]`);
        if (section) this.#scroll.scrollTop = section.offsetTop - this.#sections.offsetTop;
      }
      const uni = e.target.closest('[data-uni]');
      if (uni) {
        rememberEmoji(`u:${uni.dataset.uni}`);
        this.hooks.onPick({ char: uni.dataset.uni }, e.shiftKey);
        return;
      }
      const tile = e.target.closest('[data-emoji-id]');
      if (!tile) return;
      const emoji = this.hooks.emojis().find((x) => x.id === tile.dataset.emojiId);
      if (!emoji) return;
      if (this.#editing) {
        if (e.target.closest('.emoji-x')) {
          confirmSheet({ title: `Delete :${emoji.name}:?`, text: 'Messages that already use it keep showing it.', ok: 'Delete', danger: true })
            .then((yes) => yes && this.hooks.onDelete?.(emoji));
          return;
        }
        confirmSheet({ title: `Rename :${emoji.name}:`, input: { label: 'New name', value: emoji.name, maxlength: 32 }, ok: 'Rename' }).then((answer) => {
          const name = answer?.trim();
          if (!name || name === emoji.name) return;
          if (!/^[A-Za-z0-9_]{2,32}$/.test(name)) return this.hooks.toast('Emoji names are 2–32 letters, numbers or underscores.');
          this.hooks.onRename?.(emoji, name);
        });
      } else {
        rememberEmoji(`c:${emoji.id}`);
        this.hooks.onPick({ emoji }, e.shiftKey);
      }
    });
    el.addEventListener('mouseover', (e) => {
      const tile = e.target.closest('.emoji-tile');
      if (!tile) return;
      const glyph = this.#preview.querySelector('.pv-glyph');
      glyph.innerHTML = tile.dataset.uni ? esc(tile.dataset.uni) : tile.querySelector('img')?.outerHTML ?? '';
      this.#preview.querySelector('.pv-name').textContent = tile.title;
    });
    fileInput.addEventListener('change', () => {
      const files = [...fileInput.files];
      fileInput.value = '';
      if (files.length === 1) this.#startUpload(files[0]);
      else if (files.length > 1) this.#bulkUpload(files);
    });
    this.#uploadRow.addEventListener('submit', async (e) => {
      e.preventDefault();
      const name = this.#uploadRow.name.value.trim();
      const error = this.#uploadRow.querySelector('.form-error');
      try {
        await this.hooks.onUpload(this.#pendingFile, name);
        this.#closeUpload();
      } catch (err) {
        error.textContent = err.message;
        error.hidden = false;
      }
    });
  }

  get isOpen() {
    return !this.#el.hidden;
  }

  open() {
    this.#el.hidden = false;
    this.render();
    if (!standard) loadStandardEmojis().then(() => this.render(), () => {});
    this.#search.focus();
  }

  close() {
    this.#el.hidden = true;
    this.#closeUpload();
  }

  render() {
    if (this.#el.hidden) return;
    const q = this.#search.value.trim().toLowerCase().replace(/^:|:$/g, '');
    const custom = this.hooks.emojis();
    const sections = [];
    if (q) {
      const customHits = custom.filter((x) => x.name.toLowerCase().includes(q));
      const standardHits = (standard?.list ?? [])
        .filter((e) => e.codes.some((c) => c.includes(q)) || e.words.includes(q))
        .slice(0, 300);
      sections.push(['results', 'Results', customHits.map(customTile).join('') + standardHits.map(standardTile).join('')]);
    } else {
      const byId = new Map(custom.map((x) => [x.id, x]));
      const recent = recentKeys()
        .map((k) => (k.startsWith('c:') ? (byId.has(k.slice(2)) ? customTile(byId.get(k.slice(2))) : '') : standardTile({ char: k.slice(2), codes: [standard?.list.find((e) => e.char === k.slice(2))?.codes[0] ?? 'emoji'] })))
        .join('');
      if (recent) sections.push(['recent', 'Recently used', recent]);
      sections.push(['custom', 'Custom', custom.length ? custom.map(customTile).join('') : `<p class="emoji-hint">No custom emojis yet. Upload as many as you like.</p>`]);
      standard?.groups.forEach((name, i) => sections.push([String(i), name, standard.list.filter((e) => e.group === i).map(standardTile).join('')]));
      if (!standard) sections.push(['loading', '', '<p class="emoji-hint">Loading emojis…</p>']);
    }
    this.#sections.innerHTML = sections
      .map(([id, title, tiles]) => `<section data-section="${id}">${title ? `<h4>${esc(title)}</h4>` : ''}<div class="emoji-grid">${tiles || '<p class="emoji-hint">No match.</p>'}</div></section>`)
      .join('');
    this.#el.querySelector('.emoji-tabs').innerHTML = q ? '' : [
      ['recent', '🕘', 'Recently used'],
      ['custom', '★', 'Custom'],
      ...(standard?.groups ?? []).map((name, i) => [String(i), GROUP_ICONS[i] ?? '•', name]),
    ]
      .filter(([id]) => sections.some(([s]) => s === id))
      .map(([id, glyph, title]) => `<button type="button" data-jump="${id}" title="${esc(title)}" aria-label="${esc(title)}">${glyph}</button>`)
      .join('');
  }

  async #startUpload(file) {
    try {
      this.#pendingFile = await prepareEmojiFile(file);
    } catch (err) {
      return this.hooks.toast(err.message);
    }
    const row = this.#uploadRow;
    row.hidden = false;
    row.querySelector('.form-error').hidden = true;
    URL.revokeObjectURL(row.querySelector('img').src);
    row.querySelector('img').src = URL.createObjectURL(this.#pendingFile);
    row.name.value = suggestName(file.name);
    row.name.focus();
    row.name.select();
  }

  async #bulkUpload(files) {
    const taken = new Set(this.hooks.emojis().map((x) => x.name.toLowerCase()));
    let added = 0;
    const errors = [];
    for (const file of files) {
      try {
        const blob = await prepareEmojiFile(file);
        let name = suggestName(file.name);
        for (let n = 2; taken.has(name.toLowerCase()); n++) name = `${suggestName(file.name).slice(0, 29)}_${n}`;
        await this.hooks.onUpload(blob, name);
        taken.add(name.toLowerCase());
        added++;
      } catch (err) {
        errors.push(err.message);
      }
    }
    this.hooks.toast(`Added ${added} emoji${added === 1 ? '' : 's'}${errors.length ? ` — ${errors.length} skipped: ${errors[0]}` : '.'}`);
  }

  #closeUpload() {
    this.#uploadRow.hidden = true;
    this.#pendingFile = null;
  }
}

// ── `:name` autocomplete in the composer ──────────────────────────────────

export class EmojiSuggest {
  #el;
  #input;
  #items = [];
  #index = 0;
  #start = -1;
  #title; // "People" for the @-list (kept while you arrow through it)

  /** `avatar(user)` draws a person's picture in the @-list (optional). */
  constructor(el, input, emojis, people = () => [], avatar = null) {
    this.#el = el;
    this.#input = input;
    this.emojis = emojis;
    this.people = people;
    this.avatar = avatar;
    el.addEventListener('mousedown', (e) => {
      const row = e.target.closest('[data-i]');
      if (!row) return;
      e.preventDefault(); // keep focus in the textarea
      this.#choose(Number(row.dataset.i));
    });
  }

  get isOpen() {
    return !this.#el.hidden;
  }

  update() {
    const caret = this.#input.selectionStart;
    const before = this.#input.value.slice(0, caret);
    // @name → people; @everyone last, and only once you start typing it
    // (so "@" + Enter can't ping everybody by accident).
    const at = before.match(/(?:^|\s)@([^\s@]{0,32})$/);
    if (at && this.#input.selectionEnd === caret) {
      const q = at[1].toLowerCase();
      const people = this.people().filter((p) => p.name.toLowerCase().startsWith(q) || p.name.toLowerCase().includes(` ${q}`)).slice(0, 8);
      if (q && 'everyone'.startsWith(q)) people.push({ name: 'everyone', everyone: true });
      this.#items = people.map((p) => ({ kind: 'person', person: p, label: p.name }));
      if (!this.#items.length) return this.close();
      this.#start = caret - at[1].length - 1;
      this.#index = Math.min(this.#index, this.#items.length - 1);
      this.#title = 'People';
      this.#render();
      this.#el.hidden = false;
      return;
    }
    const m = before.match(new RegExp(`(?:^|[^A-Za-z0-9_:<]):(${CODE})$`));
    if (!m || this.#input.selectionEnd !== caret) return this.close();
    if (!standard) loadStandardEmojis().then(() => this.isOpen || this.update(), () => {});
    const q = m[1].toLowerCase();
    const rank = (names) => (names.some((n) => n.startsWith(q)) ? 0 : names.some((n) => n.includes(q)) ? 1 : 2);
    const custom = this.emojis()
      .map((x) => ({ kind: 'custom', emoji: x, label: x.name, r: rank([x.name.toLowerCase()]) }))
      .filter((x) => x.r < 2);
    const std = (standard?.list ?? [])
      .map((e) => ({ kind: 'standard', char: toned(e), label: e.codes.find((c) => c.startsWith(q)) ?? e.codes.find((c) => c.includes(q)) ?? e.codes[0], r: rank(e.codes) }))
      .filter((x) => x.r < 2);
    this.#items = [...custom, ...std].sort((a, b) => a.r - b.r).slice(0, 8);
    if (!this.#items.length) return this.close();
    this.#start = caret - m[1].length - 1;
    this.#index = Math.min(this.#index, this.#items.length - 1);
    this.#title = null;
    this.#render();
    this.#el.hidden = false;
  }

  /** Returns true if the key was handled (so the composer shouldn't act on it). */
  handleKey(e) {
    if (!this.isOpen) return false;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      const step = e.key === 'ArrowDown' ? 1 : -1;
      this.#index = (this.#index + step + this.#items.length) % this.#items.length;
      this.#render();
    } else if (e.key === 'Enter' || e.key === 'Tab') {
      this.#choose(this.#index);
    } else if (e.key === 'Escape') {
      this.close();
    } else {
      return false;
    }
    e.preventDefault();
    return true;
  }

  close() {
    this.#el.hidden = true;
    this.#index = 0;
    this.#input.removeAttribute('aria-activedescendant');
  }

  #render(title = this.#title) {
    const query = this.#input.value.slice(this.#start + 1, this.#input.selectionStart);
    // (For screen readers: a list the message box points at, with the picked row.)
    this.#el.setAttribute('role', 'listbox');
    this.#el.setAttribute('aria-label', title ?? 'Emojis');
    this.#input.setAttribute('aria-controls', this.#el.id);
    this.#input.setAttribute('aria-autocomplete', 'list');
    this.#el.innerHTML = `<div class="suggest-title" aria-hidden="true">${esc(title ?? `Emoji matching :${query}`)}</div>${this.#items
      .map((x, i) => `<div class="suggest-row ${i === this.#index ? 'active' : ''}" data-i="${i}" role="option" id="suggest-${i}" aria-selected="${i === this.#index}">${
        x.kind === 'person' ? `${!x.person.everyone && this.avatar ? this.avatar(x.person) : '<span class="suggest-glyph">@</span>'}<span>${esc(x.label)}${x.person.everyone ? ' <small class="muted">— pings everyone</small>' : ''}</span>`
          : `${x.kind === 'custom' ? `<img src="/emoji/${esc(x.emoji.id)}" alt="">` : `<span class="suggest-glyph">${x.char}</span>`}<span>:${esc(x.label)}:</span>`
      }</div>`)
      .join('')}`;
    this.#input.setAttribute('aria-activedescendant', `suggest-${this.#index}`);
  }

  #choose(i) {
    const item = this.#items[i];
    if (!item) return;
    const value = this.#input.value;
    const caret = this.#input.selectionStart;
    const insert = item.kind === 'person' ? `@${item.person.name} ` : item.kind === 'custom' ? `:${item.emoji.name}: ` : `${item.char} `;
    if (item.kind !== 'person') rememberEmoji(item.kind === 'custom' ? `c:${item.emoji.id}` : `u:${item.char}`);
    this.#input.value = value.slice(0, this.#start) + insert + value.slice(caret);
    const pos = this.#start + insert.length;
    this.#input.setSelectionRange(pos, pos);
    this.#input.dispatchEvent(new Event('input', { bubbles: true }));
    this.close();
  }
}
