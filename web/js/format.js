const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };

/** HTML-escape anything user-supplied before it touches innerHTML. */
export const esc = (value) => String(value ?? '').replace(/[&<>"']/g, (c) => ESC[c]);

/**
 * Tiny, safe message formatter: escape first, then a handful of Markdown
 * bits — ```blocks```, `code`, **bold**, *italic*, _italic_, ~~strike~~,
 * ||spoilers||, "> quotes", and bare links. Code is set aside first so
 * nothing inside it gets formatted.
 */
let mentionKey = null;
let mentionCache = null;
/** One compiled @mention regex per member list (names longest-first, so "@Sam Lee" beats "@Sam"). */
function mentionRegex(names) {
  const key = names.join('\u0000');
  if (key !== mentionKey) {
    const who = ['everyone', 'here', ...names].map((n) => esc(n).replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).sort((a, b) => b.length - a.length);
    // (Set-aside code, links and custom emojis, \ue000N\ue001, count as words on both sides, like mentions.js.)
    mentionCache = new RegExp(`(^|[^\\w@\ue001])@(${who.join('|')})(?![\\w\ue000])`, 'gi');
    mentionKey = key;
  }
  mentionCache.lastIndex = 0;
  return mentionCache;
}

export function renderBody(text, names = []) {
  const slots = [];
  const keep = (html) => `${slots.push(html) - 1}`;
  let s = esc(text);
  s = s.replace(/```(?:[\w+-]*\n)?([\s\S]*?)```/g, (_, code) => keep(`<pre><code>${code.replace(/^\n+|\n+$/g, '')}</code></pre>`));
  s = s.replace(/`([^`\n]+)`/g, (_, code) => keep(`<code>${code}</code>`));
  // Custom emoji tokens <:name:id> (escaped by now). Images come only from our own /emoji/.
  s = s.replace(/&lt;:([A-Za-z0-9_]{2,32}):([a-f0-9]{32})&gt;/g, (_, name, id) =>
    keep(`<img class="emoji" src="/emoji/${id}" alt=":${name}:" title=":${name}:" draggable="false" loading="lazy">`));
  s = s.replace(/\bhttps?:\/\/[^\s<]+/g, (url) => {
    const trail = url.match(/(?:[).,!?;:]|&#39;|&quot;)+$/)?.[0] ?? '';
    const href = trail ? url.slice(0, -trail.length) : url;
    return keep(`<a href="${href}" target="_blank" rel="noopener noreferrer">${href}</a>`) + trail;
  });
  // @mentions (after links, so "@name" inside a URL stays part of the link)
  const mentionRe = mentionRegex(names);
  s = s.replace(mentionRe, (_, pre, name) => `${pre}${keep(`<span class="mention">@${name}</span>`)}`);
  s = s.replace(/\*\*(?=\S)([\s\S]*?\S)\*\*/g, '<strong>$1</strong>');
  s = s.replace(/(^|[^*\w])\*(?=\S)([^*\n]*?\S)\*(?![*\w])/g, '$1<em>$2</em>');
  s = s.replace(/(^|[^_\w])_(?=\S)([^_\n]*?\S)_(?![_\w])/g, '$1<em>$2</em>');
  s = s.replace(/~~(?=\S)([\s\S]*?\S)~~/g, '<s>$1</s>');
  // Spoilers: hidden until clicked (per message; nothing remembered).
  s = s.replace(SPOILER, '<span class="spoiler" data-action="reveal-spoiler" role="button" tabindex="0" title="Spoiler — click to show">$1</span>');
  s = s.replace(/^&gt; ?(.*)$/gm, '<span class="quote">$1</span>');
  s = s.replace(/\n/g, '<br>');
  return s.replace(/(\d+)/g, (_, i) => slots[i] ?? '');
}

const SPOILER = /\|\|((?:(?!\|\|)[^\n])+)\|\|/g;
/** Plain-text places (quotes, search, notifications) show spoilers as blocks. */
export const hideSpoilers = (text) => String(text ?? '').replace(SPOILER, '▒▒▒');

const CUSTOM_EMOJI = /<:([A-Za-z0-9_]{2,32}):([a-f0-9]{32})>/g;

/**
 * A message as plain text, for every place that shows a snippet of one:
 * reply quotes, the reply bar, pins, search results, catch-up and
 * notifications (and phone pushes). The formatting marks go — **bold**,
 * __bold__, *italic*, _italic_, ~~strike~~, `code`, ```blocks```, "> " —
 * spoilers become ▒▒▒ and custom emojis their :name:. Code and links are
 * kept as written (an underscore in a link isn't italics). `line: true`
 * folds it onto one line.
 */
export function plainText(text, { line = false } = {}) {
  const slots = [];
  const keep = (s) => `${slots.push(s) - 1}`;
  let s = String(text ?? '');
  s = s.replace(/```(?:[\w+-]*\n)?([\s\S]*?)```/g, (_, code) => keep(code.replace(/^\n+|\n+$/g, '')));
  s = s.replace(/`([^`\n]+)`/g, (_, code) => keep(code));
  s = s.replace(CUSTOM_EMOJI, (_, name) => keep(`:${name}:`));
  s = s.replace(/\bhttps?:\/\/[^\s<]+/g, (url) => keep(url));
  s = s.replace(SPOILER, '▒▒▒');
  s = s.replace(/\*\*(?=\S)([\s\S]*?\S)\*\*/g, '$1');
  s = s.replace(/__(?=\S)([\s\S]*?\S)__/g, '$1');
  s = s.replace(/(^|[^*\w])\*(?=\S)([^*\n]*?\S)\*(?![*\w])/g, '$1$2');
  s = s.replace(/(^|[^_\w])_(?=\S)([^_\n]*?\S)_(?![_\w])/g, '$1$2');
  s = s.replace(/~~(?=\S)([\s\S]*?\S)~~/g, '$1');
  s = s.replace(/^> ?/gm, '');
  if (line) s = s.replace(/\s*\n\s*/g, ' ').trim();
  return s.replace(/(\d+)/g, (_, i) => slots[i] ?? '');
}

/** At most `n` characters, cut at a word where there's one nearby, with "…" (never "end.…"). */
export function clip(text, n) {
  if (text.length <= n) return text;
  const cut = text.slice(0, n);
  const space = /\s/.test(text[n]) ? n : cut.lastIndexOf(' '); // (a cut that ends a word keeps it)
  return `${(space > n * 0.6 ? cut.slice(0, space) : cut).replace(/[\s.,;:!?…—-]+$/, '')}…`;
}

const EMOJI_MARK = 0xe100; // custom emojis in a snippet: one private-use character each
const EMOJI_MARKS = /[-]/g;

/**
 * plainText as HTML, with custom emojis drawn as small pictures. `cut(text)`
 * may shorten the plain text first (each emoji counts as one character, so a
 * cut never splits one) and `mark(html)` may highlight the escaped text.
 */
export function plainHtml(text, { line = true, cut = (t) => t, mark = (h) => h } = {}) {
  const emojis = [];
  const swapped = String(text ?? '').replace(CUSTOM_EMOJI, (token, name, id) =>
    emojis.length < 0xf8ff - EMOJI_MARK ? String.fromCharCode(EMOJI_MARK + emojis.push({ name, id }) - 1) : token);
  const html = mark(esc(cut(plainText(swapped, { line }))));
  return html.replace(EMOJI_MARKS, (c) => {
    const e = emojis[c.charCodeAt(0) - EMOJI_MARK];
    return e ? `<img class="emoji" src="/emoji/${e.id}" alt=":${e.name}:" title=":${e.name}:" draggable="false" loading="lazy">` : '';
  });
}

const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;

/**
 * The 11-character video id in a YouTube link (watch, youtu.be, shorts,
 * music.youtube, embed, live), or a bare id. null if it isn't one.
 */
export function youtubeId(input) {
  const text = String(input ?? '').trim();
  if (VIDEO_ID.test(text)) return text;
  let url;
  try {
    url = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : `https://${text}`);
  } catch {
    return null;
  }
  if (!/^https?:$/.test(url.protocol)) return null;
  const host = url.hostname.toLowerCase().replace(/^(?:www|m|music)\./, '');
  let id = null;
  if (host === 'youtu.be') id = url.pathname.split('/')[1];
  else if (host === 'youtube.com' || host === 'youtube-nocookie.com') {
    id = url.pathname === '/watch' ? url.searchParams.get('v') : url.pathname.match(/^\/(?:shorts|embed|live|v|e)\/([^/]+)/)?.[1];
  }
  return id && VIDEO_ID.test(id) ? id : null;
}

/** The first YouTube video a message links to (its "Watch together" button), or null. */
export function youtubeLinkIn(text) {
  for (const [url] of String(text ?? '').matchAll(/\bhttps?:\/\/[^\s<]+/g)) {
    const id = youtubeId(url.replace(/[).,!?;:'"]+$/, ''));
    if (id) return id;
  }
  return null;
}

const EMOJI_ONLY = /^(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|\p{Emoji_Modifier}|‍|️|\s)*$/u;
const CUSTOM_TOKEN = /<:[A-Za-z0-9_]{2,32}:[a-f0-9]{32}>/g;
/** A short all-emoji message (unicode and/or custom) renders big, like everywhere else. */
export function isJumbo(text) {
  const custom = text.match(CUSTOM_TOKEN)?.length ?? 0;
  const rest = text.replace(CUSTOM_TOKEN, '');
  if (!EMOJI_ONLY.test(rest)) return false;
  const count = custom + [...rest.matchAll(/\p{Extended_Pictographic}/gu)].length;
  return count > 0 && count <= 8;
}

const timeFmt = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' });
const dayFmt = new Intl.DateTimeFormat(undefined, { weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' });

export const sameDay = (a, b) => new Date(a).toDateString() === new Date(b).toDateString();
export const formatTime = (ts) => timeFmt.format(ts);
export const formatDay = (ts) => dayFmt.format(ts);

export function initial(name) {
  return [...(name || '?').trim()][0]?.toUpperCase() ?? '?';
}
