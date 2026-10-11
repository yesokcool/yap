// The People sheet, one person's controls, and who is waiting to be let in.

import { settings } from '../settings.js';
import { icon } from '../icons.js';
import { esc } from '../format.js';
import { placeNear } from '../place.js';
import { $, awayOf, isMod, myRole, state } from './state.js';
import { avatar, holdFocus, popoverOpened, roleBadge, untilText } from './ui.js';

// ── Waiting room, roles, gate ─────────────────────────────────────────────

/** Names as a sentence: "Ada", "Ada and Ben", "Ada, Ben and 2 others". */
function nameList(names) {
  if (names.length <= 2) return names.join(' and ');
  return names.length === 3 ? `${names[0]}, ${names[1]} and ${names[2]}` : `${names[0]}, ${names[1]} and ${names.length - 2} others`;
}

/** A personal link that was already used: say so, so whoever lets them in can check first (W13). */
export function inviteWarning(u) {
  const used = u.invite?.usedBy ?? [];
  if (!used.length) return '';
  return `<span class="invite-warn" role="note">${icon('alert')}<span>This link was also used by ${esc(nameList(used))} — check with ${esc(u.invite.by)}.</span></span>`;
}

/** The People sheet: everyone, A–Z. No online/offline on purpose — people should be able to drop in and chill. */
export function renderMembers() {
  if (!$('people-dialog').open) return;
  const byName = (a, b) => a.name.localeCompare(b.name);
  const people = [...state.users.values()].filter((u) => u.role !== 'pending' && u.role !== 'banned').sort(byName);
  const voiceOf = (uid) => [...state.sessions.values()].find((s) => s.uid === uid && s.voice && s.client !== 'obs');
  // Only the owner and mods let people in, so only they see who's waiting.
  const waiting = isMod() ? [...state.waiting.values()] : [];
  const row = (user) => {
    const me = user.id === settings.uid;
    const inVoice = voiceOf(user.id);
    const away = awayOf(user);
    const sub = [inVoice ? `${icon('volume')}${esc(state.channels.get(inVoice.voice.channelId)?.name ?? '')}` : '', away ? `<span class="away-sub" title="Away message">${icon('coffee')}${esc(away.text)}${away.until ? ` <span class="muted">· back ${esc(untilText(away.until))}</span>` : ''}</span>` : ''].filter(Boolean).join(' ');
    const open = state.personOpen === user.id && !me;
    return `
      <li class="member ${open ? 'open' : ''}" ${me ? '' : `data-action="person" data-uid="${esc(user.id)}" role="button" tabindex="0" aria-expanded="${open}"`}>
        <span class="avatar-wrap">${avatar(user)}</span>
        <span class="member-text"><span class="name" style="color:${esc(user.color)}">${esc(user.name)}${roleBadge(user)}${me ? ' <small>(you)</small>' : ''}</span>${sub ? `<span class="sub">${sub}</span>` : ''}</span>
      </li>
      ${open ? `<li class="person-controls">${memberControlsHtml(user)}</li>` : ''}`;
  };
  const refocus = holdFocus($('members'));
  $('members').innerHTML = `<div class="buddy-well">
    ${waiting.length ? `<h3>Waiting to join (${waiting.length})</h3><ul>${waiting.map((u) => `
      <li class="member waiting">
        <span class="avatar-wrap">${avatar(u)}</span>
        <span class="member-text"><span class="name">${esc(u.name)}</span>${u.invitedBy ? `<span class="sub">via ${esc(u.invitedBy)}</span>` : ''}${inviteWarning(u)}
        <span class="sub"><button type="button" class="link-btn" data-action="approve" data-uid="${esc(u.id)}">Let in</button> · <button type="button" class="link-btn" data-action="deny" data-uid="${esc(u.id)}">Turn away</button></span></span>
      </li>`).join('')}</ul>` : ''}
    <h3>Everyone (${people.length})</h3>
    <ul>${people.map(row).join('')}</ul></div>`;
  refocus();
}

export function openPeople() {
  state.personOpen = null;
  $('people-dialog').showModal();
  renderMembers();
}

/** Volume and owner/mod actions for one person (People sheet and voice popover). */
function memberControlsHtml(user) {
  const uid = user.id;
  const volume = Math.round((settings.volumes[uid] ?? 1) * 100);
  const canBan = isMod() && user.role !== 'owner';
  const canPromote = myRole() === 'owner' && (user.role === 'member' || user.role === 'mod');
  return `
    <label class="field"><span>Volume <output>${volume}%</output></span><input type="range" min="0" max="200" step="5" value="${volume}" data-volume-uid="${esc(uid)}" title="Past 100% boosts quiet people"></label>
    ${canPromote || canBan ? `<div class="pop-actions">
      ${canPromote ? `<button type="button" class="btn small" data-action="set-role" data-uid="${esc(uid)}" data-role="${user.role === 'mod' ? 'member' : 'mod'}">${user.role === 'mod' ? 'Remove mod' : 'Make mod'}</button>` : ''}
      ${canBan ? `<button type="button" class="btn small" data-action="reset-pin" data-uid="${esc(uid)}">Reset password…</button>` : ''}
      ${canBan ? `<button type="button" class="btn small danger" data-action="ban" data-uid="${esc(uid)}">Ban</button>` : ''}
    </div>` : ''}`;
}

/** Someone under a voice channel: Watch screen while they share (Stop watching if you are), their volume, owner/mod actions. */
export function openMemberMenu(el) {
  const uid = el.dataset.uid;
  const user = state.users.get(uid);
  if (!user || uid === settings.uid) return;
  const sid = el.dataset.voiceSid;
  const sharing = Boolean(sid && state.sessions.get(sid)?.voice?.screen);
  const pop = $('popover');
  pop.innerHTML = `
    <div class="pop-title">${avatar(user, 'sm')}<span>${esc(user.name)}</span>${roleBadge(user)}</div>
    ${awayOf(user) ? `<p class="away-sub">${icon('coffee')}${esc(awayOf(user).text)}</p>` : ''}
    ${sharing ? `<div class="pop-actions pop-screen">${state.screenSid === sid
      ? `<button type="button" class="btn small" data-action="stop-watching">${icon('monitor')}<span>Stop watching</span></button>`
      : `<button type="button" class="btn small" data-action="watch-screen" data-sid="${esc(sid)}">${icon('monitor')}<span>Watch screen</span></button>`}</div>` : ''}
    ${memberControlsHtml(user)}`;
  pop.hidden = false;
  placeNear(pop, el);
  // Under someone in the sidebar: stay over the sidebar, not the chat next to it.
  const side = el.closest('.sidebar')?.getBoundingClientRect();
  if (side?.width) pop.style.left = `${Math.max(8, Math.min(parseFloat(pop.style.left), side.right - pop.offsetWidth - 4))}px`;
  popoverOpened(el);
}
