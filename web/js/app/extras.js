// Extras (PROTOCOL.md › Extras): doodle reactions, the soundboard, the
// picture viewer and shelf, Catch up, hang reminders, away messages and quiet
// hours.

import { settings, save } from '../settings.js';
import { icon } from '../icons.js';
import { esc, plainText, plainHtml, clip } from '../format.js';
import { DoodlePad, doodleId } from '../doodle.js';
import { Soundboard } from '../soundboard.js';
import { placeNear } from '../place.js';
import { $, conn, findMessage, isMod, noHover, state } from './state.js';
import { openedBy, popoverOpened, toast } from './ui.js';
import { postImage, postSound } from './uploads.js';
import { whenPlanner } from './places.js';
import { renderChannels, renderLinks } from './sidebar.js';
import { isUnread } from './chat.js';
import { reactPicker } from './composer.js';

export let doodlePad = null;
export let soundboard = null;

/** How much is new where, with mentions that came while you were away (PROTOCOL.md › Since you left). */
export function requestUnreadCounts() {
  const unread = [...state.channels.values()].filter((c) => c.kind === 'text' && isUnread(c)).slice(0, 50);
  if (unread.length) conn?.send({ t: 'unread.counts', since: Object.fromEntries(unread.map((c) => [c.id, settings.lastRead[c.id] ?? 0])) });
}

export function onUnreadCounts(counts) {
  // Mentions that came while you were away (older servers don't count them: `mentions` is missing).
  for (const [id, c] of Object.entries(counts)) {
    if (typeof c.mentions !== 'number' || (id === settings.channel && document.visibilityState === 'visible')) continue;
    if (c.mentions > (state.mentions.get(id) ?? 0)) state.mentions.set(id, c.mentions);
  }
  renderChannels();
  if ($('catchup-dialog').open) renderCatchup(counts);
}

let doodleTarget = null;
export function openDoodle(button) {
  doodlePad ??= new DoodlePad($('doodle-pad'), {
    done: async (blob) => {
      try {
        const { id } = await postImage('/api/doodle', blob);
        conn?.send({ t: 'react', id: doodleTarget, emoji: `<d:${id}>`, on: true });
        return true;
      } catch (err) {
        toast(`Couldn't send the doodle: ${err.message}`);
        return false;
      }
    },
  });
  reactPicker?.close();
  doodleTarget = button.dataset.id;
  openedBy.set('doodle-pad', button);
  doodlePad.open(button);
}

/** A doodle reaction, big. Yours can be taken back from here. */
export function viewDoodle(el) {
  const media = doodleId(el.dataset.emoji);
  const m = findMessage(el.dataset.id);
  const r = m?.reactions?.find((x) => x.emoji === el.dataset.emoji);
  if (!media || !r) return;
  const names = r.uids.map((uid) => esc(state.users.get(uid)?.name ?? 'someone')).join(', ');
  const pop = $('popover');
  pop.innerHTML = `
    <div class="doodle-view"><img src="/media/${esc(media)}" alt="Doodle" width="192" height="192"></div>
    <div class="pop-title"><span>Doodle by ${names}</span></div>
    ${r.uids.includes(settings.uid) ? `<div class="pop-actions"><button type="button" class="btn small" data-action="doodle-remove" data-id="${esc(m.id)}" data-emoji="${esc(el.dataset.emoji)}">Take mine back</button></div>` : ''}`;
  pop.hidden = false;
  placeNear(pop, el);
  popoverOpened(el);
}

export function openSoundboard(button) {
  soundboard ??= new Soundboard($('soundboard'), {
    sounds: () => [...state.sounds.values()],
    inVoice: () => Boolean(state.voiceChannel),
    play: (id) => conn?.send({ t: 'sound.play', id }),
    upload: async (blob, name, emoji) => {
      await postSound(blob, { kind: 'board', name, emoji });
      toast(`“${name}” is on the soundboard.`);
    },
    remove: (sound) => conn?.send({ t: 'sound.delete', id: sound.id }),
    canRemove: (sound) => sound.uid === settings.uid || isMod(),
    toast,
  });
  openedBy.set('soundboard', button);
  soundboard.toggle(button);
}

/** A picture, big: who and when, Open full size, and every picture in this channel (the shelf). */
export function openPicture(link) {
  const m = findMessage(link.dataset.id);
  const who = state.users.get(m?.uid)?.name ?? 'Someone';
  const channel = state.channels.get(m?.channelId ?? settings.channel);
  $('picture-title').textContent = `${who}’s picture`;
  $('picture-img').src = link.getAttribute('href');
  $('picture-img').alt = plainText(m?.body ?? '', { line: true }) || 'Picture';
  $('picture-meta').textContent = [m ? new Date(m.ts).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' }) : '', channel?.kind === 'text' ? `in ${channel.name}` : ''].filter(Boolean).join(' · ');
  $('picture-full').href = link.getAttribute('href');
  // Encrypted pictures aren't on the shelf (they're inside sealed notes).
  $('picture-shelf').hidden = !m?.image;
  $('picture-shelf').textContent = channel?.kind === 'personal' ? 'All pictures here' : `All pictures in ${channel?.name ?? 'this channel'}`;
  if (!$('picture-dialog').open) $('picture-dialog').showModal();
}

/** Pictures and GIFs of the current channel, newest first. */
export function openShelf() {
  const channel = state.channels.get(settings.channel);
  if (!channel || channel.kind === 'voice') return;
  state.shelf = { channelId: channel.id, items: [], hasMore: false };
  $('shelf-title').textContent = `Pictures & GIFs in ${channel.kind === 'personal' ? 'your Personal space' : channel.name}`;
  $('shelf-grid').innerHTML = '<p class="muted">Loading…</p>';
  $('shelf-more').hidden = true;
  if (!$('shelf-dialog').open) $('shelf-dialog').showModal();
  conn?.send({ t: 'shelf', channelId: channel.id });
}

export function renderShelf(msg) {
  if (msg.channelId !== state.shelf?.channelId) return;
  state.shelf.items.push(...msg.items);
  state.shelf.hasMore = msg.hasMore;
  renderShelfGrid();
}

export function renderShelfGrid() {
  $('shelf-grid').innerHTML = state.shelf.items.map((m) => {
    const who = state.users.get(m.uid)?.name ?? 'Someone';
    const when = new Date(m.ts).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    const src = m.image ? `/media/${esc(m.image.id)}` : esc(m.gif?.still ?? m.gif?.webp ?? m.gif?.gif ?? '');
    const label = esc(`${m.gif ? 'GIF' : 'Picture'} from ${who} · ${when}${m.body ? ` · ${plainText(m.body, { line: true }).slice(0, 80)}` : ''}`);
    return `<button type="button" class="shelf-item" data-action="open-result" data-channel="${esc(m.channelId)}" data-id="${esc(m.id)}" title="${label}" aria-label="${label}">
        <img src="${src}" alt="" loading="lazy" draggable="false">${m.gif ? '<span class="shelf-badge">GIF</span>' : ''}
      </button>`;
  }).join('') || '<p class="muted">No pictures or GIFs here yet.</p>';
  $('shelf-more').hidden = !state.shelf.hasMore;
}

/** Unread text channels (not the one you're reading). */
export const unreadElsewhere = () => [...state.channels.values()].filter((c) => c.kind === 'text' && c.id !== settings.channel && isUnread(c));

export function openCatchup() {
  const unread = unreadElsewhere();
  $('catchup-list').innerHTML = unread.length ? '<li class="muted">Loading…</li>' : '<li class="muted">You’re all caught up.</li>';
  $('catchup-read-all').hidden = !unread.length;
  $('catchup-dialog').querySelector('.devices-hint').textContent = `What's new since you last read each channel. ${noHover.matches ? 'Tap' : 'Click'} one to start at the first unread message.`;
  if (!$('catchup-dialog').open) $('catchup-dialog').showModal();
  if (unread.length) conn?.send({ t: 'unread.counts', since: Object.fromEntries(unread.map((c) => [c.id, settings.lastRead[c.id] ?? 0])) });
}

function renderCatchup(counts) {
  const forYou = (id, c) => Math.max(c.mentions ?? 0, state.mentions.get(id) ?? 0);
  // Mentions first (most first), then the newest.
  const rows = Object.entries(counts).filter(([, c]) => c.n).sort(([ia, a], [ib, b]) => forYou(ib, b) - forYou(ia, a) || (b.first?.ts ?? 0) - (a.first?.ts ?? 0));
  $('catchup-read-all').hidden = !rows.length;
  $('catchup-list').innerHTML = rows.map(([id, c]) => {
    const channel = state.channels.get(id);
    const mentions = forYou(id, c);
    const who = state.users.get(c.first?.uid);
    const line = c.first?.body ? plainHtml(c.first.body, { cut: (t) => clip(t, 160) }) : '';
    return `
      <li class="result">
        <button type="button" class="result-open" data-action="open-result" data-channel="${esc(id)}" data-id="${esc(c.first?.id ?? '')}">
          <span class="result-meta"><b>${esc(channel?.name ?? id)}</b> · ${c.n >= 999 ? '999+' : c.n} new${mentions ? ` · <span class="mention-badge">${mentions}</span> for you` : ''}</span>
          <span class="result-text"><b style="color:${esc(who?.color ?? 'inherit')}">${esc(who?.name ?? 'Someone')}</b>: ${line || '📷 A picture or GIF'}</span>
        </button>
      </li>`;
  }).join('') || '<li class="muted">You’re all caught up.</li>';
}

/** Catch up › Mark all read: every channel counts as read up to its newest message. */
export function markAllRead() {
  const lastRead = { ...settings.lastRead };
  for (const c of state.channels.values()) if (c.kind !== 'voice' && c.lastMessageAt) lastRead[c.id] = Math.max(lastRead[c.id] ?? 0, c.lastMessageAt);
  save({ lastRead });
  state.mentions.clear();
  $('catchup-dialog').close();
  renderChannels();
}

/** Hangs changed: the sidebar line, When's good, and the "starting now" bar. */
export function renderHangs() {
  renderLinks();
  whenPlanner?.render();
  const bar = $('hang-bar');
  const hang = state.hangNow;
  bar.hidden = !hang;
  if (!hang) return;
  const where = hang.channelId ? state.channels.get(hang.channelId)?.name : null;
  bar.innerHTML = `<span>🗓 <b>${esc(hang.title)}</b> is starting${where ? ` · ${esc(where)}` : ''}</span><span class="spacer"></span>
    ${hang.channelId && state.voiceChannel !== hang.channelId ? `<button class="btn small primary" type="button" data-action="hang-join" data-id="${esc(hang.id)}">Join voice</button>` : ''}
    <button class="icon-btn tiny" type="button" data-action="hang-dismiss" aria-label="Dismiss" title="Dismiss">${icon('x')}</button>`;
}

/** Settings › Profile: set an away message (it ends by itself). */
export function setAway() {
  const form = $('settings-form');
  const text = form.awayText.value.trim();
  if (!text) return toast('Say what you’re up to first.');
  const now = new Date();
  const choice = form.awayUntil.value;
  let until = null;
  if (choice === 'tomorrow') {
    const t = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 9);
    until = t.getTime();
  } else if (choice) until = now.getTime() + Number(choice) * 60_000;
  conn?.send({ t: 'profile', away: { text, until } });
}

/** Settings › Notifications › Quiet hours → the server (it holds back pushes). */
export function sendQuiet(until = state.notify?.quiet?.until ?? null) {
  const form = $('settings-form');
  const minutes = (v) => {
    const [h, m] = String(v || '0:0').split(':').map(Number);
    return (h * 60 + (m || 0)) % 1440;
  };
  const on = form.quietOn.checked;
  const from = on ? minutes(form.quietFrom.value) : 0;
  const to = on ? minutes(form.quietTo.value) : 0;
  const quiet = on || until ? { from, to, tz: Intl.DateTimeFormat().resolvedOptions().timeZone, mentions: form.quietMentions.checked, until } : null;
  conn?.send({ t: 'notify.quiet', quiet });
}
