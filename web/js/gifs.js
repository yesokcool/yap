/*
 * GIF picker backed by KLIPY (the free successor to Tenor's API).
 *
 * KLIPY's integration rules shape this file:
 *  - API calls and media loads go straight from the browser to KLIPY (no
 *    proxying through our Worker), using the URLs exactly as returned.
 *  - Trending/search results are shown in the order given, in their own grid.
 *  - KLIPY branding is shown in the picker.
 * A testing-mode key allows 100 requests/hour for the whole server, so the
 * picker caches trending and debounces search.
 */
import { settings } from './settings.js';
import { esc } from './format.js';
import { loadConfig as loadServerConfig } from './config.js';

const API = 'https://api.klipy.com/api/v1';
const PER_PAGE = 24;
const DEBOUNCE_MS = 400;
const TRENDING_TTL_MS = 10 * 60_000;

let config = null; // { provider, appKey } | null, once loaded

/** One-tap searches above Trending. */
const MOODS = ['lol', 'yes', 'no', 'wow', 'gg', 'hype', 'love', 'sad', 'facepalm', 'dance', 'thank you', 'good night'];

// ── Favorites: starred GIFs, kept on this device ──────────────────────────
const FAV_KEY = 'yap.gifFavorites';
const MAX_FAVORITES = 120;
function loadFavorites() {
  try {
    return JSON.parse(localStorage.getItem(FAV_KEY) || '[]');
  } catch {
    return [];
  }
}
function saveFavorites(list) {
  try {
    localStorage.setItem(FAV_KEY, JSON.stringify(list.slice(0, MAX_FAVORITES)));
  } catch {}
}
const favKey = (gif) => gif.slug || gif.mp4 || gif.webp || gif.gif;
const isFavorite = (gif) => loadFavorites().some((g) => favKey(g) === favKey(gif));
let customerId = null;
const trendingCache = { at: 0, pages: [] };

async function loadConfig() {
  if (config !== null) return config;
  config = (await loadServerConfig()).gifs ?? false;
  return config;
}

/** A stable, anonymous id for KLIPY's per-user features (a hash of our uid). */
async function klipyCustomerId() {
  if (customerId) return customerId;
  const bytes = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(`yap:${settings.uid}`)));
  customerId = [...bytes.slice(0, 16)].map((b) => b.toString(16).padStart(2, '0')).join('');
  return customerId;
}

class KlipyError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

async function klipy(path, params = {}) {
  const cfg = await loadConfig();
  if (!cfg) throw new KlipyError(0, 'not-configured');
  const url = new URL(`${API}/${encodeURIComponent(cfg.appKey)}/gifs/${path}`);
  for (const [k, v] of Object.entries(params)) if (v !== undefined && v !== '') url.searchParams.set(k, v);
  const res = await fetch(url);
  const body = await res.json().catch(() => ({}));
  if (!res.ok || body.result === false) throw new KlipyError(res.status, body.errors?.message?.[0] ?? `HTTP ${res.status}`);
  return body.data;
}

async function fetchPage(mode, query, page) {
  const customer_id = await klipyCustomerId();
  const locale = (navigator.language.split('-')[1] ?? '').toLowerCase();
  if (mode === 'recent') return klipy(`recent/${customer_id}`, { page, per_page: PER_PAGE });
  // content_filter: KLIPY's own safety filter (we may not filter results ourselves).
  if (mode === 'search') return klipy('search', { q: query, page, per_page: PER_PAGE, customer_id, locale, content_filter: 'medium' });
  // Trending is the same for everyone and changes slowly: cache it.
  if (Date.now() - trendingCache.at > TRENDING_TTL_MS) trendingCache.pages = [];
  if (!trendingCache.pages[page]) {
    trendingCache.pages[page] = await klipy('trending', { page, per_page: PER_PAGE, customer_id, locale, content_filter: 'medium' });
    trendingCache.at = Date.now();
  }
  return trendingCache.pages[page];
}

/** Tell KLIPY a GIF was shared (feeds "Recent" and their ranking). */
export async function reportShare(slug, query) {
  try {
    const cfg = await loadConfig();
    if (!cfg) return;
    await fetch(`${API}/${encodeURIComponent(cfg.appKey)}/gifs/share/${encodeURIComponent(slug)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ customer_id: await klipyCustomerId(), ...(query ? { q: query } : {}) }),
    });
  } catch {}
}

/** KLIPY item → the Gif we put in a message (medium size, URLs untouched). */
function toGif(item) {
  const md = item.file?.md ?? item.file?.hd;
  const size = md?.mp4 ?? md?.webp ?? md?.gif;
  if (!md || !size) return null;
  const gif = { provider: 'klipy', slug: item.slug, title: item.title ?? '', width: size.width, height: size.height };
  if (md.mp4) gif.mp4 = md.mp4.url;
  if (md.webm) gif.webm = md.webm.url;
  if (md.webp) gif.webp = md.webp.url;
  if (md.gif) gif.gif = md.gif.url;
  if (md.jpg) gif.still = md.jpg.url;
  return gif;
}

// ── Picker UI ─────────────────────────────────────────────────────────────

export class GifPicker {
  #el;
  #grid;
  #columns = [];
  #status;
  #input;
  #mode = 'trending';
  #query = '';
  #page = 1;
  #hasNext = false;
  #loading = false;
  #token = 0;
  #debounce = null;
  #observer;
  #onPick;

  constructor(el, onPick) {
    this.#el = el;
    this.#onPick = onPick;
    el.innerHTML = `
      <div class="gif-head">
        <input class="gif-search" type="search" placeholder="Search KLIPY" aria-label="Search GIFs" enterkeyhint="search">
        <div class="gif-tabs" role="tablist">
          <button type="button" role="tab" data-mode="trending" aria-selected="true">Trending</button>
          <button type="button" role="tab" data-mode="recent" aria-selected="false">Recent</button>
          <button type="button" role="tab" data-mode="favorites" aria-selected="false">★ Favorites</button>
        </div>
        <div class="gif-moods">${MOODS.map((m) => `<button type="button" data-q="${esc(m)}">${esc(m)}</button>`).join('')}</div>
      </div>
      <div class="gif-scroll"><div class="gif-grid"></div><div class="gif-status"></div><div class="gif-sentinel"></div></div>
      <a class="gif-credit" href="https://klipy.com" target="_blank" rel="noopener noreferrer">Powered by <b>KLIPY</b></a>`;
    this.#grid = el.querySelector('.gif-grid');
    this.#status = el.querySelector('.gif-status');
    this.#input = el.querySelector('.gif-search');

    this.#input.addEventListener('input', () => {
      clearTimeout(this.#debounce);
      this.#debounce = setTimeout(() => {
        const q = this.#input.value.trim();
        this.#load(q ? 'search' : 'trending', q);
      }, DEBOUNCE_MS);
    });
    el.querySelector('.gif-tabs').addEventListener('click', (e) => {
      const tab = e.target.closest('[data-mode]');
      if (!tab) return;
      this.#input.value = '';
      this.#load(tab.dataset.mode, '');
    });
    el.querySelector('.gif-moods').addEventListener('click', (e) => {
      const mood = e.target.closest('[data-q]');
      if (!mood) return;
      this.#input.value = mood.dataset.q;
      this.#load('search', mood.dataset.q);
    });
    this.#grid.addEventListener('click', (e) => {
      const item = e.target.closest('[data-index]');
      if (!item) return;
      const gif = this.items[Number(item.dataset.index)];
      if (e.target.closest('[data-fav]')) return this.#toggleFavorite(gif, e.target.closest('[data-fav]'));
      this.#onPick(gif, this.#mode === 'search' ? this.#query : '');
    });
    this.#observer = new IntersectionObserver(
      (entries) => entries.some((e) => e.isIntersecting) && this.#more(),
      { root: el.querySelector('.gif-scroll'), rootMargin: '300px' },
    );
    this.#observer.observe(el.querySelector('.gif-sentinel'));
  }

  items = [];

  open() {
    this.#el.hidden = false;
    this.#input.focus();
    if (!this.items.length || this.#mode === 'recent') this.#load(this.#mode, this.#query);
  }

  close() {
    this.#el.hidden = true;
  }

  get isOpen() {
    return !this.#el.hidden;
  }

  async #load(mode, query) {
    this.#mode = mode;
    this.#query = query;
    this.#page = 1;
    this.items = [];
    for (const tab of this.#el.querySelectorAll('[data-mode]')) {
      tab.setAttribute('aria-selected', String(tab.dataset.mode === (mode === 'search' ? '' : mode)));
    }
    this.#el.querySelector('.gif-moods').hidden = mode === 'favorites' || mode === 'recent';
    this.#layout();
    if (mode === 'favorites') {
      ++this.#token;
      this.items = loadFavorites();
      this.#hasNext = false;
      this.#append(0);
      return this.#setStatus(this.items.length ? '' : 'Tap the ★ on any GIF to keep it here.');
    }
    await this.#fetch();
  }

  #toggleFavorite(gif, star) {
    const list = loadFavorites();
    const key = favKey(gif);
    const on = !list.some((g) => favKey(g) === key);
    saveFavorites(on ? [gif, ...list] : list.filter((g) => favKey(g) !== key));
    star.classList.toggle('on', on);
    star.title = on ? 'Remove from favorites' : 'Add to favorites';
    if (!on && this.#mode === 'favorites') this.#load('favorites', '');
  }

  async #more() {
    if (this.#hasNext && !this.#loading && this.items.length) {
      this.#page++;
      await this.#fetch();
    }
  }

  async #fetch() {
    const token = ++this.#token;
    this.#loading = true;
    this.#setStatus('Loading…');
    try {
      const data = await fetchPage(this.#mode, this.#query, this.#page);
      if (token !== this.#token) return; // a newer search replaced this one
      const fresh = (data?.data ?? []).map(toGif).filter(Boolean);
      const start = this.items.length;
      this.items.push(...fresh);
      this.#hasNext = Boolean(data?.has_next);
      this.#append(start);
      this.#setStatus(this.items.length ? '' : this.#mode === 'recent' ? 'GIFs you send show up here.' : 'No GIFs found.');
    } catch (err) {
      if (token !== this.#token) return;
      this.#hasNext = false;
      this.#setStatus(
        err.message === 'not-configured' ? 'GIFs aren’t set up on this server yet (needs a KLIPY key).'
          : err.status === 429 ? 'GIF search is resting — the free KLIPY key allows 100 searches an hour. Try again soon.'
            : 'Couldn’t reach KLIPY. Try again in a moment.',
      );
    } finally {
      if (token === this.#token) this.#loading = false;
    }
  }

  #setStatus(text) {
    this.#status.textContent = text;
    this.#status.hidden = !text;
  }

  /** Masonry: each tile goes into the shortest column, so order reads left→right, top→bottom. */
  #layout() {
    const count = this.#el.clientWidth > 420 ? 3 : 2;
    this.#grid.innerHTML = '';
    this.#columns = Array.from({ length: count }, () => {
      const col = document.createElement('div');
      col.className = 'gif-col';
      col.dataset.height = '0';
      this.#grid.append(col);
      return col;
    });
  }

  #append(start) {
    for (let i = start; i < this.items.length; i++) {
      const gif = this.items[i];
      const col = this.#columns.reduce((a, b) => (Number(a.dataset.height) <= Number(b.dataset.height) ? a : b));
      col.dataset.height = String(Number(col.dataset.height) + gif.height / gif.width);
      const tile = document.createElement('button');
      tile.type = 'button';
      tile.className = 'gif-tile';
      tile.dataset.index = String(i);
      tile.title = gif.title;
      tile.style.aspectRatio = `${gif.width} / ${gif.height}`;
      const fav = isFavorite(gif);
      tile.innerHTML = `<img src="${esc(gif.webp ?? gif.gif ?? gif.still)}" alt="${esc(gif.title)}" loading="lazy" decoding="async"><span class="gif-fav ${fav ? 'on' : ''}" data-fav title="${fav ? 'Remove from favorites' : 'Add to favorites'}">★</span>`;
      col.append(tile);
    }
  }
}

// ── Rendering GIF messages ────────────────────────────────────────────────

const MAX_SIDE = 300;
const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)');

export function gifHtml(gif) {
  const scale = Math.min(1, MAX_SIDE / gif.width, MAX_SIDE / gif.height);
  const w = Math.round(gif.width * scale);
  const h = Math.round(gif.height * scale);
  const size = `width="${w}" height="${h}" style="aspect-ratio:${gif.width}/${gif.height}"`;
  const label = esc(gif.title || 'GIF');
  if (gif.mp4 || gif.webm) {
    const sources = [gif.mp4 && `<source src="${esc(gif.mp4)}" type="video/mp4">`, gif.webm && `<source src="${esc(gif.webm)}" type="video/webm">`].filter(Boolean).join('');
    const poster = gif.still ? `poster="${esc(gif.still)}"` : '';
    return `<video class="gif-embed" ${size} ${poster} muted loop playsinline preload="metadata" aria-label="${label}" title="${label}">${sources}</video>`;
  }
  return `<img class="gif-embed" ${size} src="${esc(gif.webp ?? gif.gif)}" alt="${label}" title="${label}" loading="lazy">`;
}

/** Play GIF videos only while they're on screen (and not for reduced motion). */
const playObserver = new IntersectionObserver((entries) => {
  for (const { target, isIntersecting } of entries) {
    if (isIntersecting && !reduceMotion.matches) target.play().catch(() => {});
    else target.pause();
  }
}, { threshold: 0.25 });

export function watchGifs(root) {
  for (const video of root.querySelectorAll('video.gif-embed')) {
    playObserver.observe(video);
    // Reduced motion: still frame until hovered or tapped.
    video.onmouseenter = () => reduceMotion.matches && video.play().catch(() => {});
    video.onmouseleave = () => reduceMotion.matches && video.pause();
    video.onclick = () => (video.paused ? video.play() : video.pause());
  }
}

export function forgetGifs(root) {
  for (const video of root.querySelectorAll('video.gif-embed')) playObserver.unobserve(video);
}
