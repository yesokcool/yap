// The message list, updated by key (PATTERNS.md › Keyed message list), and
// the HTML for a message and everything in it.

import { settings } from '../settings.js';
import { icon } from '../icons.js';
import { esc, renderBody, isJumbo, formatTime, formatDay, sameDay, plainHtml, youtubeLinkIn } from '../format.js';
import { doodleId } from '../doodle.js';
import { gifHtml, watchGifs, forgetGifs } from '../gifs.js';
import { isEncryptedNote } from '../vault.js';
import { $, GROUP_MS, LONG_CHARS, LONG_LINES, MAX_SIDE_PX, isMod, isPersonalChannel, memberNames, state } from './state.js';
import { avatar, untilText } from './ui.js';
import { mentionsMe } from './chat.js';
import { markEditing } from './composer.js';
import { openNote, personalStartHtml, sealedImageHtml } from './personal.js';

export function renderMessages({ keep = 'auto' } = {}) {
  const list = $('message-list');
  const scroller = $('messages');
  const channelId = settings.channel;
  const channel = state.channels.get(channelId);
  const h = state.history.get(channelId);
  const atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  const prevHeight = scroller.scrollHeight;
  const prevTop = scroller.scrollTop;

  const nodes = [];
  if (!channel || !h || (h.loading && h.mode === 'latest')) {
    nodes.push(nodeFor('empty', `<div class="empty-state">${state.status === 'online' ? 'Loading…' : ''}</div>`));
  } else {
    nodes.push(h.hasMore
      ? nodeFor(`more:${channelId}`, `<div class="load-more">${h.loading ? 'Loading older messages…' : ''}</div>`)
      : nodeFor(`start:${channelId}`, channel.kind === 'personal'
        ? personalStartHtml()
        : `<div class="channel-start"><div class="channel-start-icon">${icon('chat')}</div><h2>Welcome to ${esc(channel.name)}</h2><p>This is the very beginning of the channel.</p></div>`));
    const pending = [...state.pending.values()].filter((p) => p.channelId === channelId);
    const since = state.since.get(channelId);
    const notes = state.localNotes.filter((n) => n.channelId === channelId);
    const noteNode = (n) => nodeFor(`note:${n.key}`, `<div class="local-note">${n.html}</div>`);
    let divided = false;
    let prev = null;
    for (const m of [...h.messages, ...pending]) {
      // This session's own lines ("Cleo joined…") go where they happened.
      let afterNote = false;
      while (notes.length && notes[0].ts < m.ts && !m.pending) {
        nodes.push(noteNode(notes.shift()));
        afterNote = true;
      }
      const newDay = !prev || !sameDay(prev.ts, m.ts);
      if (newDay) nodes.push(nodeFor(`day:${channelId}:${new Date(m.ts).toDateString()}`, `<div class="day-divider"><span>${esc(formatDay(m.ts))}</span></div>`));
      // Since you left: one quiet line above the first message you haven't seen.
      const fresh = since && !divided && m.ts > since && m.uid !== settings.uid && !m.pending;
      if (fresh) {
        divided = true;
        nodes.push(nodeFor(`new:${channelId}:${since}`, `<div class="new-divider" id="new-divider"><span>New since ${esc(formatTime(since))}</span></div>`));
      }
      const grouped = !newDay && !fresh && !afterNote && prev.uid === m.uid && m.ts - prev.ts < GROUP_MS;
      nodes.push(nodeFor(`msg:${m.id}`, messageHtml(m, grouped)));
      for (const note of state.awayNotes.filter((n) => n.after === m.id && n.channelId === channelId)) {
        const who = state.users.get(note.uid);
        nodes.push(nodeFor(`away:${m.id}:${note.uid}`, `<div class="away-note">${icon('coffee')}<span><b style="color:${esc(who?.color ?? 'inherit')}">${esc(who?.name ?? 'They')}</b> is away: ${esc(note.text)}${note.until ? ` <span class="muted">· back ${esc(untilText(note.until))}</span>` : ''}</span></div>`));
      }
      prev = m;
    }
    for (const n of notes) nodes.push(noteNode(n));
  }
  reconcile(list, nodes);
  if (state.actionsFor) nodeCache.get(`msg:${state.actionsFor}`)?.el.classList.add('show-actions'); // (a redrawn row keeps them)
  markEditing();
  renderJumpPill();

  if (keep === 'bottom' || (keep === 'auto' && atBottom)) scroller.scrollTop = scroller.scrollHeight;
  else if (keep === 'anchor') scroller.scrollTop = prevTop + (scroller.scrollHeight - prevHeight);
  renderNewPill();
}

/** Reading the newest messages (within a line or so of the bottom). */
export function nearBottom() {
  const scroller = $('messages');
  return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
}

/** "↓ New messages": someone wrote below while you're scrolled up; gone once you're back at the bottom. */
export function renderNewPill() {
  const pill = $('new-pill');
  if (!pill) return;
  if (state.newBelow && nearBottom()) state.newBelow = false;
  pill.hidden = !(state.newBelow && state.view === 'chat');
}

/** "Jump to first unread" while the "New since…" divider is above the screen. */
export function renderJumpPill() {
  const pill = $('jump-pill');
  const divider = $('new-divider');
  if (!pill) return;
  // (By where they are on screen: the divider's offsetTop counts from the page, not the list.)
  const above = divider && state.view === 'chat' && divider.getBoundingClientRect().top < $('messages').getBoundingClientRect().top;
  pill.hidden = !above;
}

/*
 * The message list is updated by key instead of rebuilt, so existing rows
 * (and any GIFs playing in them) aren't torn down every time a message
 * arrives. A row is re-created only when its HTML actually changes.
 */
export const nodeCache = new Map(); // key -> { html, el }

function nodeFor(key, html) {
  const hit = nodeCache.get(key);
  if (hit && hit.html === html) return hit.el;
  const template = document.createElement('template');
  template.innerHTML = html.trim();
  const el = template.content.firstElementChild;
  if (hit) forgetGifs(hit.el);
  watchGifs(el);
  nodeCache.set(key, { html, el });
  return el;
}

function reconcile(list, nodes) {
  nodes.forEach((node, i) => {
    const current = list.children[i];
    if (current !== node) list.insertBefore(node, current ?? null);
  });
  while (list.children.length > nodes.length) list.lastElementChild.remove();
  const live = new Set(nodes);
  for (const [key, { el }] of nodeCache) {
    if (!live.has(el)) {
      forgetGifs(el);
      nodeCache.delete(key);
    }
  }
}

function emojiDisplay(emoji) {
  const custom = emoji.match(/^<:([A-Za-z0-9_]{2,32}):([a-f0-9]{32})>$/);
  return custom ? `<img src="/emoji/${custom[2]}" alt=":${esc(custom[1])}:">` : `<span class="emoji-char">${esc(emoji)}</span>`;
}

function reactionsHtml(m) {
  if (!m.reactions?.length) return '';
  return `<div class="reactions">${m.reactions
    .filter((r) => r.uids.length)
    .map((r) => {
      const names = r.uids.map((uid) => state.users.get(uid)?.name ?? 'someone').join(', ');
      // Doodles: drawn by one person each, shown big; click to see it large.
      const doodle = doodleId(r.emoji);
      if (doodle) {
        return `<button class="reaction doodle-reaction ${r.uids.includes(settings.uid) ? 'mine' : ''}" data-action="doodle-view" data-id="${esc(m.id)}" data-emoji="${esc(r.emoji)}" title="A doodle by ${esc(names)}"><img src="/media/${esc(doodle)}" alt="Doodle by ${esc(names)}" loading="lazy" draggable="false"></button>`;
      }
      const mine = r.uids.includes(settings.uid);
      const said = r.emoji.match(/^<:([A-Za-z0-9_]{2,32}):/)?.[1];
      return `<button class="reaction ${mine ? 'mine' : ''}" data-action="react-toggle" data-id="${esc(m.id)}" data-emoji="${esc(r.emoji)}" title="${esc(names)}" aria-pressed="${mine}" aria-label="${esc(said ? `:${said}:` : r.emoji)} ${r.uids.length}: ${esc(names)}">${emojiDisplay(r.emoji)}${r.uids.length}</button>`;
    })
    .join('')}</div>`;
}

/** A poll: one row per option, with a bar, a count and who picked it. */
function pollHtml(m) {
  const total = m.poll.votes.reduce((n, v) => n + v.length, 0);
  const mine = m.poll.votes.findIndex((v) => v.includes(settings.uid));
  return `<div class="poll" role="group" aria-label="Poll">${m.poll.options.map((option, i) => {
    const voters = m.poll.votes[i] ?? [];
    const pct = total ? Math.round((voters.length / total) * 100) : 0;
    const names = voters.map((uid) => state.users.get(uid)?.name ?? 'someone').join(', ');
    return `<button type="button" class="poll-option ${mine === i ? 'mine' : ''}" data-action="poll-vote" data-id="${esc(m.id)}" data-option="${i}" aria-pressed="${mine === i}" title="${esc(names || 'No votes yet')}${mine === i ? ' — click again to take your vote back' : ''}" ${m.pending ? 'disabled' : ''}>
        <span class="poll-bar" style="width:${pct}%"></span>
        <span class="poll-text">${mine === i ? '✓ ' : ''}${esc(option)}</span>
        <span class="poll-count">${voters.length}${total ? ` · ${pct}%` : ''}</span>
      </button>`;
  }).join('')}<small class="muted poll-foot">${total === 1 ? '1 vote' : `${total} votes`} · one each, change any time</small></div>`;
}

/** A server-rolled /roll or /flip: big number, the dice underneath. */
function rollHtml(m) {
  const r = m.roll;
  if (r.kind === 'flip') return `<div class="roll" title="Flipped by the server — nobody can fake it"><span class="roll-face">🪙</span><b class="roll-total">${r.rolls[0] === 0 ? 'Heads' : 'Tails'}</b></div>`;
  const dice = /d/.test(r.spec) && (r.rolls.length > 1 || /[+-]/.test(r.spec)) ? `<small class="roll-dice">${esc(r.rolls.join(' + '))}${esc((/[+-]\d+$/.exec(r.spec)?.[0] ?? '').replace('-', ' − ').replace('+', ' + '))}</small>` : '';
  return `<div class="roll" title="Rolled by the server — nobody can fake it"><span class="roll-face">🎲</span><span class="roll-spec">${esc(/^\d+$/.test(r.spec) ? `1–${r.spec}` : r.spec)}</span><b class="roll-total">${esc(String(r.total))}</b>${dice}</div>`;
}

function messageHtml(m, grouped) {
  const user = state.users.get(m.uid);
  const mine = m.uid === settings.uid;
  const edited = m.editedAt ? ` <span class="edited" title="Edited ${esc(new Date(m.editedAt).toLocaleString())}">(edited)</span>` : '';
  // An encrypted Personal-space note shows what it decrypts to (or a lock).
  let text = m.body;
  let gif = m.gif;
  let image = m.image ? imageHtml(m.image, m.id) : '';
  let sealedNote = '';
  if (isEncryptedNote(m.body)) {
    const note = openNote(m.body);
    text = note?.text ?? '';
    gif = note?.gif;
    image = note?.image ? sealedImageHtml(note.image, m.id) : '';
    if (!note) sealedNote = state.vault.key ? '🔒 Decrypting…' : '🔒 Encrypted. Unlock your Personal space to read it.';
    else if (note.error) sealedNote = '🔒 Can’t open this note (it was locked with another passcode).';
  }
  // Long messages fold after ~30 lines until you ask for the rest.
  const long = text && (text.length > LONG_CHARS || text.split('\n').length > LONG_LINES);
  const open = long && state.expanded.has(m.id);
  const more = long ? `<button type="button" class="link-btn show-more" data-action="toggle-long" data-id="${esc(m.id)}">${open ? 'Show less' : `Show more (${text.length.toLocaleString()} characters)`}</button>` : '';
  const video = !sealedNote && youtubeLinkIn(text);
  const body = (sealedNote ? `<div class="body sealed">${sealedNote}</div>` : '')
    + (m.roll ? rollHtml(m) : text ? `<div class="body ${isJumbo(text) ? 'jumbo' : ''} ${long && !open ? 'folded' : ''}">${renderBody(text, memberNames())}${edited}</div>${more}` : '')
    + (m.pending && state.status !== 'online' ? '<small class="pending-note" title="Not sent yet: it goes out by itself when you’re back online">↻ Waiting to send…</small>' : '')
    + (m.poll ? pollHtml(m) : '')
    + (video ? `<button type="button" class="btn small yt-watch" data-action="watch-together" data-video="${esc(video)}" title="Play it for everyone in Watch together">${icon('play')}<span>Watch together</span></button>` : '')
    + (gif ? `<div class="gif-wrap">${gifHtml(gif)}</div>` : '')
    + (image ? `<div class="img-wrap">${image}</div>` : '')
    + reactionsHtml(m);
  const pinMark = m.pinnedAt ? `<span class="pin-mark" title="Pinned ${esc(new Date(m.pinnedAt).toLocaleString())}">${icon('pin')}</span>` : '';
  const personal = isPersonalChannel(m.channelId);
  const actions = m.pending ? '' : `<div class="msg-actions" role="toolbar" aria-label="Message actions">
      <button class="icon-btn tiny ${m.pinnedAt ? 'on' : ''}" data-action="pin-message" data-id="${esc(m.id)}" title="${m.pinnedAt ? 'Unpin' : 'Pin to this channel'}" aria-label="${m.pinnedAt ? 'Unpin' : 'Pin'}">${icon('pin')}</button>
      <button class="icon-btn tiny" data-action="react-open" data-id="${esc(m.id)}" title="Add a reaction" aria-label="Add a reaction">${icon('smile-plus')}</button>
      <button class="icon-btn tiny" data-action="doodle-open" data-id="${esc(m.id)}" title="Doodle a reaction" aria-label="Doodle a reaction">${icon('squiggle')}</button>
      ${personal ? '' : `<button class="icon-btn tiny" data-action="reply-message" data-id="${esc(m.id)}" title="Reply" aria-label="Reply">${icon('reply')}</button>`}
      ${personal ? '' : `<button class="icon-btn tiny" data-action="save-personal" data-id="${esc(m.id)}" title="Save to your Personal space" aria-label="Save to Personal space">${icon('bookmark')}</button>`}
      ${mine && !m.roll ? `<button class="icon-btn tiny" data-action="edit-message" data-id="${esc(m.id)}" title="Edit (or press ↑ in an empty message box)" aria-label="Edit message">${icon('pencil')}</button>` : ''}
      ${mine || isMod() ? `<button class="icon-btn tiny" data-action="delete-message" data-id="${esc(m.id)}" title="Delete (shift-click skips the question)" aria-label="Delete message">${icon('trash')}</button>` : ''}
    </div>`;
  const cls = `msg ${mine ? 'mine' : ''} ${grouped ? 'cont' : ''} ${m.pending ? 'pending' : ''} ${mentionsMe(m) ? 'mentions-me' : ''}`;
  const full = esc(new Date(m.ts).toLocaleString());
  // An AIM chat log: every line says who; the buddy icon starts each run.
  return `
    <article class="${cls}">
      ${grouped ? '<span></span>' : avatar(user, 'sm')}
      <div class="msg-main">${m.reply ? replyQuoteHtml(m.reply, isPersonalChannel(m.channelId)) : ''}<header><span class="author ${mine ? 'mine' : ''}" style="--c:${esc(user?.color ?? '#8d8d8d')}">${esc(user?.name ?? 'Unknown')}</span>${pinMark}</header>${body}</div>
      <time title="${full}">${esc(formatTime(m.ts))}</time>
      ${actions}
    </article>`;
}

export function renderTyping() {
  const now = Date.now();
  const typers = [...(state.typing.get(settings.channel) ?? new Map())]
    .filter(([uid, until]) => until > now && uid !== settings.uid)
    .map(([uid]) => `<b>${esc(state.users.get(uid)?.name ?? 'Someone')}</b>`);
  const el = $('typing');
  el.innerHTML =
    typers.length === 0 ? ''
      : typers.length === 1 ? `<span class="dots"><i></i><i></i><i></i></span>${typers[0]} is typing…`
        : typers.length === 2 ? `<span class="dots"><i></i><i></i><i></i></span>${typers[0]} and ${typers[1]} are typing…`
          : `<span class="dots"><i></i><i></i><i></i></span>Several people are typing…`;
}

/**
 * Touch screens have no hover: a tap on a message shows its actions under it
 * (another tap, or a tap anywhere else, hides them). A tap on a link, a
 * reaction or anything else in it does just that. After one of the actions,
 * the row closes (once the action has placed its picker).
 */
export function tapMessage(e, el) {
  if (e.target.closest('.msg-actions')) return void setTimeout(() => showActionsFor(null));
  const row = e.target.closest('.msg');
  const id = row && !el && !e.target.closest('a, button, input, textarea, video, label')
    ? row.querySelector('.msg-actions [data-id]')?.dataset.id ?? null
    : null;
  showActionsFor(id && id !== state.actionsFor ? id : null);
  if (state.actionsFor) requestAnimationFrame(() => row.scrollIntoView({ block: 'nearest', behavior: 'smooth' }));
}

function showActionsFor(id) {
  state.actionsFor = id;
  for (const row of document.querySelectorAll('.msg.show-actions')) row.classList.remove('show-actions');
  if (id) nodeCache.get(`msg:${id}`)?.el.classList.add('show-actions');
}

function replyQuoteHtml(r, saved = false) {
  const who = state.users.get(r.uid);
  const elsewhere = r.channelId && r.channelId !== settings.channel;
  if (r.deleted) return `<div class="reply-quote deleted">${icon(saved ? 'bookmark' : 'reply')}<span>${saved ? 'The saved message was deleted' : 'Original message was deleted'}</span></div>`;
  const text = isEncryptedNote(r.body) ? '🔒 Encrypted note' : plainHtml(r.body) || (r.image ? '📷 A picture' : r.gif ? 'GIF' : '');
  const where = elsewhere ? ` <small class="muted">in ${esc(state.channels.get(r.channelId)?.name ?? 'a channel')}</small>` : '';
  return `<button type="button" class="reply-quote ${saved ? 'saved' : ''}" data-action="${elsewhere ? 'open-result' : 'jump-to'}" data-channel="${esc(r.channelId ?? '')}" data-id="${esc(r.id)}" title="Show the original">${icon(saved ? 'bookmark' : 'reply')}<b style="color:${esc(who?.color ?? 'inherit')}">${esc(who?.name ?? 'Someone')}</b>${where}<span>${text}</span></button>`;
}

/** A picture in a message: opens big in Yap (Ctrl/⌘-click: a new tab, as before). */
export function imageHtml(img, msgId = '') {
  const scale = Math.min(1, MAX_SIDE_PX / img.width, MAX_SIDE_PX / img.height);
  const w = Math.round(img.width * scale);
  const h = Math.round(img.height * scale);
  return `<a href="/media/${esc(img.id)}" target="_blank" rel="noopener" title="Show it big" data-action="picture" data-id="${esc(msgId)}"><img class="msg-image" src="/media/${esc(img.id)}" width="${w}" height="${h}" style="aspect-ratio:${img.width}/${img.height}" alt="Picture" loading="lazy"></a>`;
}
