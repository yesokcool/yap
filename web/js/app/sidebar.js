// The sidebar and the chat's header: channels, Places links, join requests,
// your own card, Minecraft, the new channel and link dialogs, and the version.

import { settings, api } from '../settings.js';
import { icon } from '../icons.js';
import { esc } from '../format.js';
import { $, conn, isMod, state } from './state.js';
import { avatar, hangWhen, holdFocus } from './ui.js';
import { VIEWS, watchParty } from './places.js';
import { voiceMember } from './voice-ui.js';
import { isUnread, updateTitle } from './chat.js';
import { inviteWarning } from './people.js';
import { unreadElsewhere } from './extras.js';

let minecraftTimer = null;

export function renderConn() {
  const el = $('conn');
  el.dataset.status = state.status;
  el.title = { online: 'Connected', connecting: 'Connecting…', offline: 'Reconnecting…', waiting: 'Waiting to be let in' }[state.status];
  const banner = $('banner');
  banner.hidden = state.status !== 'offline';
  banner.textContent = 'Offline — reconnecting. Messages you send now go out when you’re back.';
}

export function renderChannels() {
  const refocus = holdFocus($('sidebar'));
  const channels = [...state.channels.values()].sort((a, b) => a.position - b.position);
  const personal = channels.find((c) => c.kind === 'personal');
  $('personal-group').hidden = !personal;
  $('personal').innerHTML = personal ? `
    <li class="channel-row">
      <button class="channel personal ${personal.id === settings.channel ? 'active' : ''} ${isUnread(personal) ? 'unread' : ''}" data-action="open-channel" data-id="${esc(personal.id)}" title="Only you can see this. For moving things between your devices — not for passwords or private info.">
        ${icon('lock')}<span class="name">Personal space</span>${isUnread(personal) ? '<span class="unread-dot" title="New notes" role="img" aria-label="new notes"></span>' : ''}
      </button>
    </li>` : '';
  updateTitle();
  const behind = unreadElsewhere();
  $('catchup-row').hidden = behind.length < 2;
  $('catchup-row').innerHTML = `${icon('sparkles')}<span>Catch up</span><small class="muted">${behind.length} channels</small>`;
  $('text-channels').innerHTML = channels
    .filter((c) => c.kind === 'text')
    .map((c) => `
      <li class="channel-row">
        <button class="channel ${c.id === settings.channel ? 'active' : ''} ${isUnread(c) ? 'unread' : ''}" data-action="open-channel" data-id="${esc(c.id)}">
          ${icon('chat')}<span class="name">${esc(c.name)}</span>${state.mentions.get(c.id) ? `<span class="mention-badge" title="Mentions" role="img" aria-label="${state.mentions.get(c.id) === 1 ? '1 mention' : `${state.mentions.get(c.id)} mentions`}">${state.mentions.get(c.id)}</span>` : isUnread(c) ? '<span class="unread-dot" title="New messages" role="img" aria-label="new messages"></span>' : ''}
        </button>
        <button class="icon-btn tiny row-action" data-action="archive-channel" data-id="${esc(c.id)}" title="Archive channel" aria-label="Archive ${esc(c.name)}">${icon('archive')}</button>
      </li>`)
    .join('');

  $('voice-channels').innerHTML = channels
    .filter((c) => c.kind === 'voice')
    .map((c) => {
      // An OBS link listens without showing up (the maintainer: OBS can record anything anyway).
      const members = [...state.sessions.values()].filter((s) => s.voice?.channelId === c.id && s.client !== 'obs');
      const joined = state.voiceChannel === c.id;
      return `
        <li class="channel-row">
          <button class="channel voice ${joined ? 'joined' : ''}" data-action="join-voice" data-id="${esc(c.id)}" title="${joined ? 'You are here' : 'Join voice'}">
            ${icon('volume')}<span class="name">${esc(c.name)}</span>
          </button>
          <button class="icon-btn tiny row-action" data-action="archive-channel" data-id="${esc(c.id)}" title="Archive channel" aria-label="Archive ${esc(c.name)}">${icon('archive')}</button>
        </li>
        ${members.length ? `<li><ul class="voice-members">${members.map(voiceMember).join('')}</ul></li>` : ''}`;
    })
    .join('');
  refocus();
}

export function renderLinks() {
  const links = [
    ...(state.links ?? state.config?.links ?? []).filter((l) => l.view || /^https:\/\//.test(l.url)),
    { name: 'Draw', view: 'draw', description: 'A shared drawing board' },
    { name: 'Space', view: 'space', description: 'Walk around together' },
    { name: 'Watch', view: 'watch', description: 'Watch YouTube together (keeps playing while you browse)' },
    { name: "When's good", view: 'when', description: 'Find a time that works for everyone (starts fresh every Monday)' },
  ];
  $('links-group').hidden = !links.length;
  const refocus = holdFocus($('links'));
  $('links').innerHTML = links
    .map((l) => {
      if (!l.view) {
        return `
          <li class="channel-row">
            <a class="channel link" href="${esc(l.url)}" target="_blank" rel="noopener noreferrer" title="${esc(l.description ?? l.url)}">
              ${icon('external')}<span class="name">${esc(l.name)}</span>
            </a>
            ${l.id ? `<button class="icon-btn tiny row-action" data-action="remove-link" data-id="${esc(l.id)}" title="Remove this link" aria-label="Remove ${esc(l.name)}">${icon('archive')}</button>` : ''}
          </li>`;
      }
      const viewers = [...state.sessions.values()].filter((s) => (l.view === 'watch' ? s.watching : s.view === l.view));
      return `
        <li class="channel-row">
          <button class="channel ${state.view === l.view ? 'active' : ''}" data-action="open-view" data-view="${esc(l.view)}" title="${esc(l.description ?? l.name)} — opens here, voice keeps going">
            ${icon(VIEWS[l.view]?.icon ?? 'board')}<span class="name">${esc(l.name)}</span>${l.view === 'watch' ? watchParty.linkExtra() : ''}${l.view === 'when' ? nextHangHtml() : ''}
          </button>
          ${/^https?:\/\//.test(l.url ?? '') ? `<a class="icon-btn tiny row-action" href="${esc(l.url)}" target="_blank" rel="noopener noreferrer" title="Open in a new tab" aria-label="Open ${esc(l.name)} in a new tab">${icon('external')}</a>` : ''}
        </li>
        ${viewers.length ? `<li><ul class="voice-members viewers">${viewers.map((s) => {
          const user = state.users.get(s.uid);
          return `<li class="voice-member">${avatar(user, 'sm')}<span class="name">${esc(user?.name ?? '')}</span></li>`;
        }).join('')}</ul></li>` : ''}`;
    })
    .join('');
  refocus();
}

/** The next planned hang, small, under When's good ("now" for half an hour once it starts). */
function nextHangHtml() {
  const now = Date.now();
  const next = [...state.hangs.values()].filter((h) => h.at > now - 30 * 60_000).sort((a, b) => a.at - b.at)[0];
  if (!next) return '';
  const when = next.at <= now ? 'now' : hangWhen(next.at);
  return `<small class="link-extra" title="${esc(next.title)} · ${esc(when)}">🗓 ${esc(when)} · ${esc(next.title)}</small>`;
}

// ── Minecraft server status ───────────────────────────────────────────────

export function startMinecraft() {
  if (!state.config?.minecraft || minecraftTimer) return;
  $('mc-group').hidden = false;
  renderMinecraft({ address: state.config.minecraft.address, checking: true });
  const poll = async () => {
    if (document.visibilityState !== 'visible') return;
    try {
      const res = await api('/api/minecraft');
      renderMinecraft(await res.json());
    } catch {}
  };
  poll();
  minecraftTimer = setInterval(poll, 60_000);
  document.addEventListener('visibilitychange', poll);
}

function renderMinecraft(s) {
  const status = s.checking ? 'checking' : s.online ? 'online' : 'offline';
  const players = s.players ? `${s.players.online}/${s.players.max} playing` : '';
  const sub = s.checking ? 'Checking…' : s.online ? [players, s.version].filter(Boolean).join(' · ') : 'Offline';
  const names = s.players?.names?.length ? `\nOn now: ${s.players.names.join(', ')}` : '';
  $('mc').innerHTML = `
    <button class="mc-row" data-action="copy-mc" data-address="${esc(s.address)}" title="Click to copy the address${esc(s.motd ? `\n${s.motd}` : '')}${esc(names)}">
      <span class="mc-dot ${status}" aria-label="${status}"></span>
      <span class="mc-text"><span class="mc-addr">${esc(s.address)}</span><span class="mc-sub">${esc(sub)}</span></span>
    </button>`;
}

/** Who's waiting to be let in: only the owner and mods let people in, so only they see it. */
export function renderRequests() {
  const el = $('requests');
  const waiting = isMod() ? [...state.waiting.values()] : [];
  const badge = $('waiting-badge');
  badge.hidden = !waiting.length || !state.sid;
  badge.textContent = String(waiting.length);
  el.hidden = !waiting.length || !state.sid;
  // One or two get their own bar; more collapse into one so the chat stays visible.
  el.innerHTML = waiting.length > 2
    ? `<div class="request"><span><b>${waiting.length} people</b> are waiting to join</span><span class="spacer"></span>
        <button class="btn small primary" data-action="show-members">Review</button></div>`
    : waiting
      .map((u) => `
        <div class="request ${u.invite?.usedBy?.length ? 'warn' : ''}">
          ${avatar(u, 'sm')}<span class="request-text"><span><b>${esc(u.name)}</b> is waiting to join${u.invitedBy ? ` <small class="muted">· invited by ${esc(u.invitedBy)}</small>` : ''}</span>${inviteWarning(u)}</span>
          <span class="spacer"></span>
          <span class="request-actions">
            <button class="btn small primary" data-action="approve" data-uid="${esc(u.id)}">Let in</button>
            <button class="btn small" data-action="deny" data-uid="${esc(u.id)}">Turn away</button>
          </span>
        </div>`)
      .join('');
}

export function renderHeader() {
  const channel = state.channels.get(settings.channel);
  const view = VIEWS[state.view];
  // Phones get the short words (the header's buttons leave room for about ten letters).
  const narrow = matchMedia('(max-width: 600px)').matches;
  $('channel-title').innerHTML = view
    ? `${icon(view.icon)}<span>${view.title}</span>${state.voiceWanted ? '<small class="muted header-note view-note">voice keeps going</small>' : ''}`
    : channel?.kind === 'personal' ? `${icon('lock')}<span>${narrow ? 'Personal' : 'Personal space'}</span><small class="muted header-note">${state.vault.wrap ? (state.vault.key ? 'end-to-end encrypted' : 'locked') : 'only you · not encrypted'}</small>${state.vault.key ? `<button class="link-btn header-lock" data-action="vault-lock" title="Lock it on this device">Lock</button>` : ''}`
      : channel ? `${icon('chat')}<span>${esc(channel.name)}</span>` : '';
  const locked = channel?.kind === 'personal' && state.vault.wrap && !state.vault.key;
  $('input').disabled = Boolean(locked);
  // (A placeholder that wraps shows half a second line.)
  $('input').placeholder = locked ? (narrow ? 'Unlock it to write' : 'Unlock your Personal space to write')
    : channel?.kind === 'personal' ? (narrow ? 'Note to self' : 'Note to self: a link, a picture, anything')
      : channel ? `Message ${channel.name}` : 'Message';
}

export function renderUserPanel() {
  const muted = settings.muted || settings.deafened;
  const me = state.users.get(settings.uid) ?? { name: settings.name || '…', color: settings.color };
  const refocus = holdFocus($('user-panel')); // (Mute from the keyboard keeps the keyboard on Mute)
  // The whole card opens Settings (on Profile); mute and deafen get big buttons.
  $('user-panel').innerHTML = `
    <button class="me" data-action="open-settings" data-tab="Profile" title="Your profile and settings" ${state.sid ? `data-voice-sid="${esc(state.sid)}"` : ''}>
      ${avatar(me)}
      <span class="me-text"><span class="name">${esc(me.name)}</span><span class="sub">Profile &amp; settings</span></span>
      ${icon('settings')}
    </button>
    <div class="me-actions">
      <button class="btn big-toggle ${muted ? 'on' : ''}" data-action="toggle-mute" title="${muted ? 'Unmute' : 'Mute'} your mic" aria-pressed="${muted}">${icon(muted ? 'mic-off' : 'mic')}<span>${muted ? 'Muted' : 'Mute'}</span></button>
      <button class="btn big-toggle ${settings.deafened ? 'on' : ''}" data-action="toggle-deafen" title="${settings.deafened ? 'Undeafen' : 'Deafen'}: hear nobody, and mute" aria-pressed="${settings.deafened}">${icon(settings.deafened ? 'headphones-off' : 'headphones')}<span>${settings.deafened ? 'Deafened' : 'Deafen'}</span></button>
    </div>`;
  if (state.sid) $('user-panel').querySelector('.me').classList.toggle('speaking', state.speaking.has(state.sid));
  refocus();
}

export function openChannelDialog(kind) {
  const form = $('channel-form');
  form.reset();
  form.kind.value = kind;
  form.name.placeholder = kind === 'voice' ? 'Hangout' : 'new-channel';
  $('channel-dialog').showModal();
}

export function openLinkDialog() {
  $('link-form').reset();
  $('link-dialog').showModal();
}

export function bindLinkDialog() {
  const dialog = $('link-dialog');
  const form = $('link-form');
  dialog.addEventListener('close', () => {
    if (dialog.returnValue !== 'add') return;
    const name = form.name.value.trim();
    const url = form.url.value.trim();
    if (name && url) conn?.send({ t: 'link.add', name, url });
  });
}

export function bindChannelDialog() {
  const dialog = $('channel-dialog');
  const form = $('channel-form');
  form.addEventListener('change', () => {
    form.name.placeholder = form.kind.value === 'voice' ? 'Hangout' : 'new-channel';
  });
  dialog.addEventListener('close', () => {
    if (dialog.returnValue !== 'create') return;
    const name = form.name.value.trim();
    if (!name) return;
    state.awaitingChannel = { kind: form.kind.value, name };
    conn?.send({ t: 'channel.create', name, kind: form.kind.value });
  });
}

export async function showVersion() {
  try {
    const log = await (await fetch('/changelog.json')).json();
    state.appVersion = log.current; // (the call report says which version)
    $('version-link').textContent = `v${log.current} · what's new`;
  } catch {}
}
