// Reading: a channel's history, new and deleted messages, unread counts and
// mentions, notifications, and jumping to a message. Sending: every message
// on its way to the server (the outbox), and what happens when it's turned away.

import { settings, save } from '../settings.js';
import { sounds } from '../sounds.js';
import { icon } from '../icons.js';
import { plainText } from '../format.js';
import { findMentions } from '../mentions.js';
import { renderAll } from '../app.js';
import { $, MAX_FRAME, coarsePointer, conn, firstTextChannel, state } from './state.js';
import { autosize, hideToast, toast } from './ui.js';
import { setView } from './places.js';
import { renderChannels } from './sidebar.js';
import { nearBottom, nodeCache, renderMessages, renderNewPill, renderTyping } from './messages.js';
import { cancelEdit, cancelReply, putBack } from './composer.js';
import { renderResultList } from './search.js';
import { renderShelfGrid } from './extras.js';

/** Just let in from the waiting room: a chime, a word in the tab title if it's in the background, and "say hi". */
export function welcomeIn() {
  sounds.mention();
  const first = firstTextChannel();
  if (first) addLocalNote(first.id, 'welcome', `${icon('sparkles')}<span><b>You're in.</b> Say hi 👋</span>`);
  if (document.visibilityState !== 'visible') {
    state.titleNote = "You're in!";
    updateTitle();
  }
}

/** A quiet line at the end of a channel, for this session only. */
export function addLocalNote(channelId, key, html) {
  if (!channelId) return;
  state.localNotes = [...state.localNotes.filter((n) => n.key !== key), { key, channelId, ts: Date.now(), html }].slice(-20);
  if (channelId === settings.channel) renderMessages();
}

export function onHistory(msg) {
  const h = state.history.get(msg.channelId);
  if (!h) return;
  if (h.mode === 'older') {
    h.messages = [...msg.messages, ...h.messages];
  } else {
    // Keep anything that arrived live while the page was loading.
    const ids = new Set(msg.messages.map((m) => m.id));
    const newest = msg.messages.at(-1)?.ts ?? 0;
    h.messages = [...msg.messages, ...h.messages.filter((m) => !ids.has(m.id) && m.ts > newest)];
  }
  h.hasMore = msg.hasMore;
  h.loading = false;
  for (const resolve of h.waiters?.splice(0) ?? []) resolve();
  if (msg.channelId === settings.channel) {
    renderMessages({ keep: h.mode === 'older' ? 'anchor' : 'bottom' });
    markRead(msg.channelId);
    // Short first page on a tall screen: keep filling until it scrolls.
    const scroller = $('messages');
    if (h.hasMore && scroller.scrollHeight <= scroller.clientHeight) loadOlder(msg.channelId);
  }
}

export function onMessage({ message: m, nonce }) {
  if (nonce) {
    state.pending.delete(nonce);
    sent(nonce);
  }
  const channel = state.channels.get(m.channelId);
  if (channel) channel.lastMessageAt = Math.max(channel.lastMessageAt ?? 0, m.ts);
  const h = state.history.get(m.channelId);
  if (h && !h.messages.some((x) => x.id === m.id)) h.messages.push(m);
  state.typing.get(m.channelId)?.delete(m.uid);
  // Your own messages count as read — except ones you sent to your Personal
  // space from another device: that's the whole point of it.
  if (m.uid === settings.uid && !(channel?.kind === 'personal' && !nonce)) markRead(m.channelId, m.ts);
  if (m.channelId === settings.channel) {
    renderMessages({ keep: nonce ? 'bottom' : 'auto' });
    // Scrolled up reading older ones: say there's something new below.
    if (m.uid !== settings.uid && state.view === 'chat' && !nearBottom()) {
      state.newBelow = true;
      renderNewPill();
    }
    renderTyping();
    if (document.visibilityState === 'visible') markRead(m.channelId);
  }
  renderChannels();
  updateTitle();
  maybeNotify(m);
}

/** A message was erased: gone from the chat, and from Pins, search results and the shelf too. */
export function onDeleted({ id, channelId }) {
  const h = state.history.get(channelId);
  if (h) h.messages = h.messages.filter((m) => m.id !== id);
  // Replies to it say so straight away (the server says the same on the next load).
  for (const list of state.history.values()) {
    for (const m of list.messages) if (m.reply?.id === id && !m.reply.deleted) m.reply = { id, uid: m.reply.uid, channelId: m.reply.channelId, body: '', deleted: true };
  }
  if (state.editing?.id === id) cancelEdit();
  if (state.replying?.id === id) cancelReply();
  state.pinnedLoose.delete(id);
  state.expanded.delete(id);
  if (state.search.results.some((m) => m.id === id)) {
    state.search.results = state.search.results.filter((m) => m.id !== id);
    if ($('search-dialog').open && !state.search.pending) renderResultList($('search-results'), state.search.results, { q: state.search.q });
  }
  if ($('pins-dialog').open && state.pinsChannel === channelId) {
    $('pins-list').querySelector(`[data-action="open-result"][data-id="${CSS.escape(id)}"]`)?.closest('li')?.remove();
    if (!$('pins-list').querySelector('.result')) renderResultList($('pins-list'), [], { pins: true });
  }
  if (state.shelf?.items.some((m) => m.id === id)) {
    state.shelf.items = state.shelf.items.filter((m) => m.id !== id);
    if ($('shelf-dialog').open) renderShelfGrid();
  }
  renderMessages();
}

// ── Sending ───────────────────────────────────────────────────────────────
// Every `send` goes through the outbox. The server answers sends in order
// (each gets its echo with our nonce, or an error with re: 'send'), so the
// oldest send still out on the wire is the one an error is about.
// "Slow down a little." (more than 10 at once, then one a second): it stays
// pending and goes again, same nonce, 1.1 s later (up to 5 times); until the
// outbox is empty the rest follow one at a time, 1.1 s apart, so they keep
// the order they were written in. Anything else turned away comes back out:
// the dimmed copy goes, the error shows as a toast, and the text goes back
// in the message box if that's empty. No wire change: only the order is kept.

const RESEND_MS = 1100;
const SLOW_DOWN_RESENDS = 5;
let resendTimer = null;
/** After a "slow down": one send at a time, RESEND_MS apart, until the outbox is empty. */
let pacing = false;
/** Numbers sends in the order they went on the wire (`out` in the outbox). */
let wireCount = 0;

/**
 * Send a message to the server. `frame` is a `send` (a nonce is added if it
 * has none); `copy` = the dimmed copy shown until the server has it; `draft` =
 * the text to put back in the message box if the server turns it away.
 */
export function queueSend(frame, { copy, draft } = {}) {
  frame.nonce ??= crypto.randomUUID();
  // Too big for one frame: the server would refuse it without saying it was a send.
  if (JSON.stringify(frame).length > MAX_FRAME) {
    toast("That's too long to send in one go. Try splitting it in two.");
    return putBack({ channelId: frame.channelId, text: draft, replyTo: frame.replyTo });
  }
  if (copy) state.pending.set(frame.nonce, copy);
  state.outbox.push({ nonce: frame.nonce, frame, draft, tries: 0, out: 0 });
  sendOutbox();
}

/** Put on the wire what can go now: all that's waiting, or while pacing, the next one once nothing is out. */
export function sendOutbox() {
  if (state.status !== 'online' || resendTimer) return;
  for (const entry of state.outbox) {
    if (entry.out) {
      if (pacing) return;
      continue;
    }
    if (!conn?.send(entry.frame)) return;
    entry.out = ++wireCount;
    if (pacing) return;
  }
}

function sendLater() {
  resendTimer ??= setTimeout(() => {
    resendTimer = null;
    sendOutbox();
  }, RESEND_MS);
}

/** A send was answered (stored, or turned away for good): the next one can go. */
function settle(entry) {
  state.outbox.splice(state.outbox.indexOf(entry), 1);
  if (!state.outbox.length) pacing = false;
  if (pacing) sendLater();
  else sendOutbox();
}

/** Our echo came back: the server has it. */
function sent(nonce) {
  const entry = state.outbox.find((e) => e.nonce === nonce);
  if (entry) settle(entry);
}

/** The server turned a send away (an `error` with re: 'send'). */
export function onSendError(msg) {
  const entry = state.outbox.reduce((oldest, e) => (e.out && (!oldest || e.out < oldest.out) ? e : oldest), null);
  if (!entry) return toast(msg.message);
  if (msg.code === 'slow_down' && entry.tries < SLOW_DOWN_RESENDS) {
    entry.tries++;
    entry.out = 0; // still pending: it goes again, same nonce
    pacing = true;
    return sendLater();
  }
  toast(msg.message);
  if (state.pending.delete(entry.nonce)) renderMessages();
  putBack({ channelId: entry.frame.channelId, text: entry.draft, replyTo: entry.frame.replyTo });
  settle(entry);
}

/**
 * The socket dropped: nothing is out any more, and `ready` sends the outbox
 * again. A send that was out without a copy to show (a picture, a poll) may
 * have arrived, so it isn't sent twice; one with a copy goes again (as before).
 */
export function onSendsLost() {
  clearTimeout(resendTimer);
  resendTimer = null;
  pacing = false;
  state.outbox = state.outbox.filter((e) => !e.out || state.pending.has(e.nonce));
  for (const e of state.outbox) Object.assign(e, { out: 0, tries: 0 });
}

// ── Text actions ──────────────────────────────────────────────────────────

/** Before marking a channel read: remember how far you'd got, for the "New since…" divider. */
export function rememberSince(id) {
  const channel = state.channels.get(id);
  const read = settings.lastRead[id] ?? 0;
  if (channel?.kind === 'text' && read && (channel.lastMessageAt ?? 0) > read) state.since.set(id, read);
  else if (id !== settings.channel) state.since.delete(id);
}

export function openChannel(id) {
  if (!id) return;
  if (state.editing && state.editing.channelId !== id) cancelEdit();
  if (state.replying && state.replying.channelId !== id) cancelReply();
  setView('chat');
  if (id !== settings.channel) {
    state.since.delete(settings.channel);
    rememberSince(id);
    swapDraft(settings.channel, id);
    state.newBelow = false;
  }
  save({ channel: id });
  if (!state.history.has(id)) loadLatest(id);
  $('app').classList.remove('sidebar-open');
  renderAll();
  renderMessages({ keep: 'bottom' });
  markRead(id);
  if (!coarsePointer) $('input').focus();
}

/**
 * Each channel keeps what you'd typed in it (in memory, until you send it):
 * switching puts this one's away and brings back the other's, so Enter never
 * sends it to the wrong channel.
 */
function swapDraft(from, to) {
  const input = $('input');
  const text = input.value;
  const theirs = state.drafts.get(to) ?? '';
  state.drafts.delete(to);
  if (state.channels.has(from)) {
    if (text.trim()) state.drafts.set(from, text);
    else state.drafts.delete(from);
    input.value = theirs;
  } else {
    // (The channel you were typing in is gone: what you typed comes along rather than vanish.)
    state.drafts.delete(from);
    input.value = theirs && text.trim() ? `${theirs}\n${text}` : theirs || text;
  }
  autosize();
}

export function loadLatest(channelId) {
  if (!conn?.open) return;
  state.history.set(channelId, { messages: [], hasMore: true, loading: true, mode: 'latest' });
  conn.send({ t: 'history', channelId, limit: 50 });
}

/** Ask for the page before the oldest message shown. False if there's nothing to ask (or no connection). */
export function loadOlder(channelId) {
  const h = state.history.get(channelId);
  if (!h || h.loading || !h.hasMore || !h.messages.length || !conn?.open) return false;
  h.loading = true;
  h.mode = 'older';
  const oldest = h.messages[0];
  conn.send({ t: 'history', channelId, limit: 50, before: oldest.ts, beforeId: oldest.id });
  return true;
}

export function markRead(channelId, ts) {
  if (state.mentions.delete(channelId)) renderChannels();
  const channel = state.channels.get(channelId);
  const latest = ts ?? channel?.lastMessageAt;
  if (!latest || (settings.lastRead[channelId] ?? 0) >= latest) return;
  save({ lastRead: { ...settings.lastRead, [channelId]: latest } });
  renderChannels();
  updateTitle();
}

export function isUnread(channel) {
  if (channel.kind === 'voice' || !channel.lastMessageAt) return false;
  if (channel.id === settings.channel && document.visibilityState === 'visible') return false;
  return channel.lastMessageAt > (settings.lastRead[channel.id] ?? 0);
}

export function updateTitle() {
  const unread = [...state.channels.values()].filter(isUnread);
  if (state.titleNote && document.visibilityState === 'visible') state.titleNote = null;
  document.title = state.titleNote ? `${state.titleNote} · Yap` : unread.length ? `(${unread.length}) Yap` : 'Yap';
  // Phones: the sidebar is hidden, so the menu button says "something new elsewhere".
  const elsewhere = unread.some((c) => c.id !== settings.channel);
  const menu = document.querySelector('[data-action="toggle-sidebar"]');
  menu?.classList.toggle('has-unread', elsewhere);
  menu?.classList.toggle('has-mention', [...state.mentions.entries()].some(([id, n]) => n && id !== settings.channel));
  updateAppBadge();
}

let badgeShown = -1; // (-1: whatever the icon was left showing gets set right on the first look)
/**
 * The installed app's icon (Home Screen, Dock, taskbar) shows your unread
 * @mentions and replies: what the sidebar's mention badges add up to.
 * Browsers without app badges (or a page that isn't installed) do nothing.
 */
function updateAppBadge() {
  if (typeof navigator.setAppBadge !== 'function') return;
  let n = 0;
  for (const c of state.channels.values()) if (c.kind === 'text') n += state.mentions.get(c.id) ?? 0;
  if (n === badgeShown) return;
  badgeShown = n;
  try {
    (n ? navigator.setAppBadge(n) : navigator.clearAppBadge()).catch(() => {});
  } catch {}
}

/** Does this message @-mention me (or @everyone / @here)? */
/** The server's rule exactly (web/js/mentions.js), so the highlight and the push always agree. */
export function mentionsMe(m) {
  if (m.uid === settings.uid) return false;
  if (m.reply?.uid === settings.uid) return true; // a reply to you pings you
  if (!m.body) return false;
  const { uids, everyone } = findMentions(m.body, [...state.users.values()]);
  return everyone || uids.includes(settings.uid);
}

/**
 * Quiet hours (PROTOCOL.md › Extras): the same rule as the server, for this
 * page's own sounds and notifications. `mention`: a ping, @mention or reply.
 */
export function isQuietNow(q, mention = false) {
  // Mentions get through the daily window and a snooze alike, when allowed.
  if (!q || (mention && q.mentions)) return false;
  if (q.until && Date.now() < q.until) return true;
  if (q.from === q.to) return false;
  let minute;
  try {
    const parts = new Intl.DateTimeFormat('en-GB', { timeZone: q.tz, hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).formatToParts(Date.now());
    minute = Number(parts.find((p) => p.type === 'hour')?.value) * 60 + Number(parts.find((p) => p.type === 'minute')?.value);
  } catch {
    return false;
  }
  return q.from < q.to ? minute >= q.from && minute < q.to : minute >= q.from || minute < q.to;
}

export function onPing({ from, text }) {
  const who = state.users.get(from)?.name ?? 'Someone';
  const quiet = isQuietNow(state.notify?.quiet, true);
  if (!quiet) sounds.ping();
  toast(`📣 ${who} pinged everyone${text ? `: ${text}` : '!'}`);
  if (!quiet && from !== settings.uid && !state.browserPush && settings.desktopNotify !== 'off' && typeof Notification !== 'undefined' && Notification.permission === 'granted' && document.visibilityState !== 'visible') {
    new Notification(`📣 ${who} pinged everyone`, { body: text || 'Hop on Yap!', tag: 'yap-ping', icon: '/icon.svg' });
  }
}

function maybeNotify(m) {
  if (mentionsMe(m)) {
    if (m.channelId !== settings.channel || document.visibilityState !== 'visible') {
      state.mentions.set(m.channelId, (state.mentions.get(m.channelId) ?? 0) + 1);
      renderChannels();
    }
    if (isQuietNow(state.notify?.quiet, true)) return;
    if (document.visibilityState !== 'visible' || m.channelId !== settings.channel) sounds.mention();
    if (!state.browserPush && settings.desktopNotify !== 'off' && document.visibilityState !== 'visible' && typeof Notification !== 'undefined' && Notification.permission === 'granted') {
      const n = new Notification(`${state.users.get(m.uid)?.name ?? 'Someone'} mentioned you`, { body: plainText(m.body, { line: true }).slice(0, 200), tag: m.channelId, icon: '/icon.svg' });
      // Clicked: straight to the message that mentions you (like a message notification opens its channel).
      n.onclick = () => {
        focus();
        openAndJump(m.channelId, m.id);
        n.close();
      };
    }
    return;
  }
  if (settings.desktopNotify !== 'all' || m.uid === settings.uid || isQuietNow(state.notify?.quiet)) return;
  if (document.visibilityState === 'visible' || typeof Notification === 'undefined' || Notification.permission !== 'granted') return;
  const user = state.users.get(m.uid);
  const channel = state.channels.get(m.channelId);
  const n = new Notification(`${user?.name ?? 'Someone'} in ${channel?.name ?? 'Yap'}`, {
    body: m.body ? plainText(m.body, { line: true }).slice(0, 200) : m.image ? '📷 A picture' : `GIF: ${m.gif?.title || 'a GIF'}`,
    tag: m.channelId,
    icon: '/icon.svg',
  });
  n.onclick = () => {
    focus();
    openChannel(m.channelId);
    n.close();
  };
}

/** Load older pages of a channel until a message is in (≤ 20 pages). */
export async function loadUntil(channelId, id) {
  for (let i = 0; i < 20; i++) {
    const h = state.history.get(channelId);
    if (!h) return false;
    if (h.messages.some((m) => m.id === id)) return true;
    if (h.loading) await new Promise((r) => (h.waiters ??= []).push(r));
    else if (!h.hasMore) return false;
    else {
      const done = new Promise((r) => (h.waiters ??= []).push(r));
      if (!loadOlder(channelId)) return false; // (offline: no page is coming)
      await done;
    }
  }
  return state.history.get(channelId)?.messages.some((m) => m.id === id) ?? false;
}

export async function openAndJump(channelId, id) {
  if (settings.channel !== channelId || state.view !== 'chat') openChannel(channelId);
  if (!(await loadUntil(channelId, id))) return toast('Couldn’t find that message (it may be very far back, or deleted).');
  renderMessages();
  requestAnimationFrame(() => jumpTo(id));
}

export async function jumpTo(id) {
  let el = nodeCache.get(`msg:${id}`)?.el;
  if (!el?.isConnected) {
    toast('Finding it…');
    if (await loadUntil(settings.channel, id)) {
      hideToast();
      renderMessages();
      el = nodeCache.get(`msg:${id}`)?.el;
    }
  }
  if (!el?.isConnected) return toast('Couldn’t find that one — it’s very far back.');
  el.scrollIntoView({ block: 'center', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
  el.classList.remove('flash');
  void el.offsetWidth;
  el.classList.add('flash');
}
