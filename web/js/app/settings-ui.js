// The Settings dialog: its tabs, voice and devices, notifications, invites,
// the activity log, storage and Stream to OBS.

import { settings, save } from '../settings.js';
import { CHIMES, playChime } from '../sounds.js';
import { icon } from '../icons.js';
import { esc } from '../format.js';
import { $, awayOf, conn, isMod, myRole, state } from './state.js';
import { ago, avatar, copyByHand, nameIn, nameOf, swatches, toast, untilText } from './ui.js';
import { applyTheme } from './look.js';
import { postImage } from './uploads.js';
import { currentPushSubscription, disableBrowserPush, enableBrowserPush } from './push.js';
import { fetchIce, hasRelay, renderVoicePanel, screenView, voice } from './voice-ui.js';
import { renderUserPanel } from './sidebar.js';
import { canAddToHome } from './account.js';
import { sendQuiet } from './extras.js';

// ── Invites, activity log, storage (Settings) ───────────────────────────────

/** Settings lists show this many rows until "Show all". */
const SHORT_LIST = 5;
/** Which of them are showing everything (until Settings is opened again). */
export const showingAll = new Set();

/** The first few rows of a list, and its "Show all" button. */
function shortList(name, rows) {
  const button = document.querySelector(`[data-action="show-all"][data-list="${name}"]`);
  const all = showingAll.has(name) || rows.length <= SHORT_LIST + 1;
  if (button) {
    button.hidden = all;
    button.textContent = `Show all ${rows.length}`;
  }
  return all ? rows : rows.slice(0, SHORT_LIST);
}

export function renderInvites() {
  const list = shortList('invites', state.invites ?? []);
  $('invite-list').innerHTML = list.length ? list.map((i) => `
    <li class="${i.revoked ? 'revoked' : ''}">
      <span class="invite-label"><b>${esc(i.label)}</b>${i.by !== settings.uid ? ` <small class="muted">by ${esc(nameIn(i.by))}</small>` : ''}</span>
      <small class="muted">${i.uses === 1 ? '1 person joined' : `${i.uses} people joined`} · made ${esc(ago(i.createdAt))}${i.revoked ? ' · turned off' : ''}</small>
      ${i.revoked ? '' : `<button class="btn small" type="button" data-action="revoke-invite" data-id="${esc(i.id)}">Turn off</button>`}
    </li>`).join('') : '<li class="muted">No invite links yet.</li>';
}

/** Settings › Owner & mods › Activity, in plain words: "<actor> let Ada in". Every action the server logs (hub.ts › log). */
const ROLE_WORDS = { mod: 'a mod', member: 'a member', owner: 'the owner' };
const LOG_WORDS = {
  'member.approve': (e) => `let ${nameIn(e.target)} in`,
  'member.deny': (e) => `turned ${nameIn(e.target)} away`,
  'member.ban': (e) => `banned ${nameIn(e.target)}`,
  'member.role': (e) => {
    const who = nameIn(e.target);
    if (e.detail === 'banned') return `banned ${who}`;
    if (e.detail === 'pending') return `sent ${who} back to the waiting room`;
    // From the app only the owner sets roles, and "member" means taking a mod's role away.
    if (e.detail === 'member' && e.actor !== 'admin') return `made ${who} a member again (no longer a mod)`;
    return `made ${who} ${ROLE_WORDS[e.detail] ?? e.detail ?? 'a member'}`;
  },
  'member.resetPin': (e) => `reset ${nameIn(e.target)}'s PIN`,
  'member.merge': (e) => `merged a duplicate account into ${nameIn(e.target)}'s`,
  'member.unmerge': () => 'split a merged account back out',
  'owner.claim': () => 'became the owner',
  'message.delete': (e) => `deleted a message by ${nameIn(e.target)}`,
  'channel.create': (e) => `made a ${e.detail ?? 'channel'}`,
  'channel.archive': (e) => `archived the channel ${e.detail ?? ''}`.trim(),
  'channel.unarchive': (e) => `brought back the ${e.detail ?? 'channel'}`,
  'channel.bitrate': (e) => {
    const [, channel, to] = /^(".*") → (.+)$/.exec(e.detail ?? '') ?? [];
    if (!channel) return 'changed a voice channel\'s quality';
    return to === 'personal settings' ? `let everyone in ${channel} pick their own voice quality again` : `set the voice quality in ${channel} to ${to} for everyone`;
  },
  'emoji.delete': (e) => `deleted the emoji ${e.detail ?? ''}`.trim(),
  'invite.create': (e) => `made an invite link${e.detail ? ` (${e.detail})` : ''}`,
  'invite.revoke': (e) => `turned off an invite link${e.detail ? ` (${e.detail})` : ''}`,
  'link.add': (e) => {
    const [, name, host] = /^(.*) → (.+)$/.exec(e.detail ?? '') ?? [];
    return name ? `added ${name} (${host}) to Places` : `added ${e.detail ?? 'a link'} to Places`;
  },
  'link.remove': (e) => `removed ${e.detail ?? 'a link'} from Places`,
  'obs.create': () => 'made an OBS link',
  'obs.revoke': (e) => (Number(e.detail) > 1 ? `turned off their ${e.detail} OBS links` : 'turned off their OBS link'),
  'sound.delete': (e) => `deleted a soundboard clip by ${nameIn(e.target)}`,
};
/** An action this page doesn't know yet (a newer server): its name in words. */
const logFallback = (e) => `did “${String(e.action).replace(/[._]/g, ' ')}”${e.detail ? ` (${e.detail})` : ''}`;

export function renderModLog() {
  const entries = shortList('modlog', state.modlog ?? []);
  $('modlog').innerHTML = entries.length ? entries.map((e) => `
    <li><b>${esc(nameOf(e.actor))}</b> ${esc((LOG_WORDS[e.action] ?? logFallback)(e))} <small class="muted">${esc(ago(e.ts))}</small></li>`).join('')
    : '<li class="muted">Nothing yet.</li>';
}

/** Settings › Voice › Advanced › Stream to OBS: the link for the channel, person and overlay picked. */
export function renderObsLink() {
  $('obs-made').hidden = !state.obsToken;
  if (!state.obsToken) return;
  const params = new URLSearchParams({ key: settings.key, listen: state.obsToken, channel: $('obs-channel').value });
  if ($('obs-only').value) params.set('only', $('obs-only').value);
  if ($('obs-sound-only').checked) params.set('overlay', '0');
  $('obs-link').value = `${location.origin}/obs#${params}`;
}

export function renderStorage({ chatBytes, mediaBytes, limitBytes }) {
  const row = (label, bytes) => {
    const pct = Math.min(100, (bytes / limitBytes) * 100);
    const size = bytes > 1e9 ? `${(bytes / 1e9).toFixed(2)} GB` : bytes > 1e6 ? `${(bytes / 1e6).toFixed(1)} MB` : `${Math.round(bytes / 1e3)} KB`;
    return `<div class="storage-row ${pct > 80 ? 'hot' : ''}"><span>${label}</span><div class="meter"><div class="meter-fill" style="width:${pct.toFixed(1)}%"></div></div><small class="muted">${size} of ${Math.round(limitBytes / 1e9)} GB</small></div>`;
  };
  $('storage').innerHTML = row('Messages', chatBytes) + row('Pictures & emojis', mediaBytes);
}

// ── Settings ──────────────────────────────────────────────────────────────

export function openSettings(tab) {
  const form = $('settings-form');
  renderAvatarSettings();
  renderNotifySettings();
  $('owner-status').textContent = myRole() === 'owner' ? "You're the owner." : myRole() === 'mod' ? "You're a mod." : '';
  $('owner-claim').hidden = myRole() === 'owner';
  form.name.value = settings.name;
  swatches($('settings-colors'), settings.color, (color) => {
    save({ color });
    conn?.send({ t: 'profile', color });
    renderUserPanel();
  });
  form.theme.value = settings.theme;
  form.mode.value = settings.mode;
  form.vadThreshold.value = settings.vadThreshold;
  form.bitrate.value = String(settings.bitrate);
  for (const k of ['music', 'noiseSuppression', 'rnnoise', 'echoCancellation', 'autoGain', 'sounds', 'relayOnly']) form[k].checked = settings[k];
  // Relay-only needs a TURN relay on the server; say so instead of pretending.
  fetchIce().then((ice) => {
    const ok = hasRelay(ice);
    form.relayOnly.disabled = !ok;
    $('relay-note').textContent = ok
      ? "— sends your voice through the server's voice relay instead of straight to friends (a little more delay)"
      : "— needs the voice relay, which isn't switched on for this server yet";
  });
  form.desktopNotify.value = settings.desktopNotify;
  form.soundboardVolume.value = Math.round(settings.soundboardVolume * 100);
  form.soundboardMuted.checked = settings.soundboardMuted;
  renderAwaySettings();
  renderChimeSettings();
  $('ptt-key').textContent = keyLabel(settings.pttKey);
  syncSettingsVisibility();
  renderMeter(voice.rawStream ? voice.localDb : -100);
  fillDevices();
  // Invites, activity and (owner/mods) storage load fresh each time.
  $('invite-made').hidden = true;
  $('home-row').hidden = !canAddToHome();
  $('vault-setup-btn').hidden = Boolean(state.vault.wrap);
  $('vault-change-btn').hidden = !state.vault.wrap;
  $('vault-lock-btn').hidden = !state.vault.key;
  state.obsToken = null; // a link is shown once, like an invite
  $('obs-channel').innerHTML = [...state.channels.values()].filter((c) => c.kind === 'voice').sort((a, b) => a.position - b.position)
    .map((c) => `<option value="${esc(c.id)}" ${c.id === state.voiceWanted ? 'selected' : ''}>${esc(c.name)}</option>`).join('');
  $('obs-only').innerHTML = `<option value="">Everyone in the call</option>${[...state.users.values()].sort((a, b) => a.name.localeCompare(b.name))
    .map((u) => `<option value="${esc(u.name)}">Only ${esc(u.name)}</option>`).join('')}`;
  $('obs-sound-only').checked = false;
  renderObsLink();
  $('voice-advanced').open = false;
  showingAll.clear();
  renderInvites(); // short again right away (fresh lists follow from the server)
  renderModLog();
  $('main-invite').hidden = !isMod();
  $('storage-field').hidden = !isMod();
  currentPushSubscription().then((sub) => ($('browser-push').checked = Boolean(sub)), () => {});
  conn?.send({ t: 'invite.list' });
  conn?.send({ t: 'modlog' });
  if (isMod()) conn?.send({ t: 'stats' });
  settingsTabs(tab);
  $('settings-dialog').showModal();
  // Phones: the open tab in view, if the tab strip is wider than the screen.
  $('settings-tabs').querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}

/**
 * Settings is a property sheet: one tab per section, named by its heading.
 * It opens on the tab you last had open on this device, unless asked for
 * one (the user card asks for Profile).
 */
const TAB_KEY = 'yap.settingsTab';
let settingsTab = (() => {
  try {
    return localStorage.getItem(TAB_KEY) ?? 0;
  } catch {
    return 0;
  }
})();
function settingsTabs(name) {
  const sections = [...document.querySelectorAll('#settings-form .tab-panel > section')];
  const tabName = (s) => s.querySelector('h3').textContent;
  // Remembered by name, so it stays right if tabs are ever added.
  const find = (n) => sections.findIndex((s) => tabName(s) === n);
  let index = name ? find(name) : -1;
  if (index < 0) index = find(settingsTab);
  if (index < 0) index = 0;
  const strip = $('settings-tabs');
  const panel = document.querySelector('#settings-form .tab-panel');
  const show = (i) => {
    settingsTab = tabName(sections[i]);
    try {
      localStorage.setItem(TAB_KEY, settingsTab);
    } catch {}
    sections.forEach((s, j) => (s.hidden = j !== i));
    strip.querySelectorAll('[role=tab]').forEach((b, j) => {
      b.setAttribute('aria-selected', String(j === i));
      b.tabIndex = j === i ? 0 : -1; // one Tab stop for the strip; arrows move along it
    });
    panel.setAttribute('aria-labelledby', `settings-tab-${i}`);
    panel.scrollTop = 0;
    // A tab strip that scrolls sideways (phones) keeps the open tab in view.
    strip.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  };
  if (!strip.children.length) {
    strip.innerHTML = sections.map((s, i) => `<button type="button" role="tab" id="settings-tab-${i}" data-tab="${i}">${esc(s.querySelector('h3').textContent)}</button>`).join('');
    strip.addEventListener('click', (e) => {
      const tab = e.target.closest('[data-tab]');
      if (tab) show(Number(tab.dataset.tab));
    });
    // ← → (and Home/End) move between tabs, like any tab strip.
    strip.addEventListener('keydown', (e) => {
      const tabs = [...strip.querySelectorAll('[role=tab]')];
      const at = tabs.indexOf(document.activeElement);
      const to = { ArrowRight: at + 1, ArrowLeft: at - 1, Home: 0, End: tabs.length - 1 }[e.key];
      if (at < 0 || to === undefined) return;
      e.preventDefault();
      const i = (to + tabs.length) % tabs.length;
      show(i);
      tabs[i].focus();
    });
  }
  show(index);
}

async function fillDevices() {
  const form = $('settings-form');
  let devices = [];
  try {
    devices = await navigator.mediaDevices.enumerateDevices();
  } catch {}
  const options = (kind, selected) => {
    const list = devices.filter((d) => d.kind === kind && d.deviceId && d.deviceId !== 'default');
    return [`<option value="">System default</option>`, ...list.map((d, i) =>
      `<option value="${esc(d.deviceId)}" ${d.deviceId === selected ? 'selected' : ''}>${esc(d.label || `${kind === 'audioinput' ? 'Microphone' : 'Speaker'} ${i + 1}`)}</option>`)].join('');
  };
  form.inputId.innerHTML = options('audioinput', settings.inputId);
  form.outputId.innerHTML = options('audiooutput', settings.outputId);
  // Safari/Firefox can't pick an output device for <audio>.
  form.outputId.closest('.field').hidden = !('setSinkId' in HTMLMediaElement.prototype);
}

function syncSettingsVisibility() {
  const form = $('settings-form');
  for (const el of form.querySelectorAll('[data-show-mode]')) el.hidden = el.dataset.showMode !== settings.mode;
  for (const el of form.querySelectorAll('[data-hide-music]')) el.hidden = settings.music;
  for (const el of form.querySelectorAll('[data-show-music]')) el.hidden = !settings.music;
  $('music-vad-note').hidden = !(settings.music && settings.mode === 'vad');
}

export function renderMeter(db) {
  const pct = (v) => `${Math.max(0, Math.min(100, ((v + 80) / 80) * 100))}%`;
  const fill = $('meter-fill');
  fill.style.width = pct(db);
  // "Hot" = loud enough to open the gate (in a noisy room that's above your line: the gate adapts).
  fill.classList.toggle('hot', db > Math.max(settings.vadThreshold, voice.openAt));
  $('meter-mark').style.left = pct(settings.vadThreshold);
}

export async function startMicTest() {
  if (voice.rawStream) return;
  state.micTest = true;
  try {
    await voice.prepare();
    if (!voice.micOk) toast('Could not open the microphone.');
    fillDevices(); // labels appear once permission is granted
  } catch (err) {
    toast('Could not open the microphone.');
  }
}

/** Settings › Voice › Hear yourself: 4 s through everything a call does, then played back. */
export async function hearYourself() {
  const button = document.querySelector('[data-action="hear-yourself"]');
  const status = $('hear-status');
  if (button.disabled) return;
  button.disabled = true;
  try {
    if (!voice.rawStream) {
      state.micTest = true; // closing Settings releases the mic again (if not in a call)
      await voice.prepare();
      fillDevices();
    }
    if (!voice.micOk) throw new Error('no mic');
    const holdKey = settings.mode === 'ptt' && voice.peers.size > 0;
    await voice.hearYourself({
      seconds: 4,
      phase: (p, info) => {
        status.textContent = p === 'recording'
          ? `Recording 4 seconds${holdKey ? ` — hold ${keyLabel(settings.pttKey)} and` : ' —'} say something…${info?.codec ? '' : ' (without the codec)'}`
          : p === 'playing' ? 'Playing it back…' : 'That’s how you sound to others.';
      },
    });
  } catch (err) {
    console.warn('hear yourself failed', err);
    status.textContent = 'Couldn’t open the microphone.';
  } finally {
    button.disabled = false;
  }
}

export let bindingPtt = false;
export function bindPttKey() {
  const button = $('ptt-key');
  bindingPtt = true;
  button.textContent = 'Press a key…';
  const onKey = (e) => {
    e.preventDefault();
    e.stopPropagation();
    removeEventListener('keydown', onKey, true);
    bindingPtt = false;
    if (e.key !== 'Escape') save({ pttKey: e.code });
    button.textContent = keyLabel(settings.pttKey);
  };
  addEventListener('keydown', onKey, true);
}

function keyLabel(code) {
  const named = { Backquote: '`', Space: 'Space', CapsLock: 'Caps Lock', ShiftLeft: 'Left Shift', ShiftRight: 'Right Shift', ControlLeft: 'Left Ctrl', ControlRight: 'Right Ctrl', AltLeft: 'Left Alt', AltRight: 'Right Alt', MetaLeft: 'Left ⌘', MetaRight: 'Right ⌘' };
  return named[code] ?? code.replace(/^Key|^Digit/, '');
}

export function bindSettings() {
  const dialog = $('settings-dialog');
  const form = $('settings-form');
  $('avatar-file').addEventListener('change', async (e) => {
    const file = e.target.files[0];
    e.target.value = '';
    if (!file) return;
    try {
      await postImage('/api/avatar', await prepareAvatar(file));
      toast('Picture updated.');
    } catch (err) {
      toast(`Couldn't set that picture: ${err.message}`);
    }
  });

  $('browser-push').addEventListener('change', async (e) => {
    try {
      if (e.target.checked) {
        await enableBrowserPush();
        toast('Browser pushes are on. Pings and @mentions will pop up even with Yap closed.');
      } else {
        await disableBrowserPush();
      }
    } catch (err) {
      e.target.checked = false;
      toast(err.message);
    }
  });

  form.addEventListener('change', async (e) => {
    if (e.target.closest('.obs-field')) return renderObsLink();
    const field = e.target.name;
    switch (field) {
      case 'name': {
        const name = form.name.value.trim();
        if (!name) return;
        save({ name });
        conn?.send({ t: 'profile', name });
        return renderUserPanel();
      }
      case 'inputId':
        save({ inputId: form.inputId.value });
        return voice.restartMic();
      case 'outputId':
        save({ outputId: form.outputId.value });
        screenView?.applyAudio(); // a screen you're watching plays its sound there too
        return voice.applySettings();
      case 'mode':
        save({ mode: form.mode.value });
        voice.setPtt(false);
        return syncSettingsVisibility();
      case 'vadThreshold':
        return save({ vadThreshold: Number(form.vadThreshold.value) });
      case 'bitrate':
        save({ bitrate: Number(form.bitrate.value) });
        voice.applySettings({ renegotiate: true });
        return renderVoicePanel();
      case 'music':
      case 'noiseSuppression':
      case 'rnnoise':
      case 'echoCancellation':
      case 'autoGain':
        save({ [field]: form[field].checked });
        syncSettingsVisibility();
        await voice.restartMic();
        voice.applySettings({ renegotiate: field === 'music' });
        return renderVoicePanel();
      case 'sounds':
        return save({ sounds: form.sounds.checked });
      case 'chime': {
        const chime = form.chime.value;
        conn?.send({ t: 'profile', chime: chime === 'default' ? null : chime });
        return playChime(chime, chime.startsWith('s:') ? state.sounds.get(chime.slice(2)) : null, { preview: true });
      }
      case 'soundboardVolume':
        return save({ soundboardVolume: Number(form.soundboardVolume.value) / 100 });
      case 'soundboardMuted':
        return save({ soundboardMuted: form.soundboardMuted.checked });
      case 'quietOn':
        $('quiet-times').hidden = !form.quietOn.checked;
        return sendQuiet();
      case 'quietFrom':
      case 'quietTo':
      case 'quietMentions':
        return sendQuiet();
      case 'relayOnly':
        save({ relayOnly: form.relayOnly.checked });
        return state.voiceChannel && toast('Takes effect the next time you join a voice channel.');
      case 'theme':
        save({ theme: form.theme.value });
        return applyTheme({ changed: true });
      case 'notifyMode':
        return conn?.send({ t: 'notify.settings', mode: form.notifyMode.value });
      case 'desktopNotify': {
        const choice = form.desktopNotify.value;
        if (choice !== 'off' && typeof Notification !== 'undefined' && Notification.permission !== 'granted') {
          const result = await Notification.requestPermission();
          if (result !== 'granted') toast('Notifications are blocked for this site in your browser settings.');
        }
        return save({ desktopNotify: choice });
      }
    }
  });
  form.vadThreshold.addEventListener('input', () => {
    save({ vadThreshold: Number(form.vadThreshold.value) });
    renderMeter(voice.localDb);
  });

  dialog.addEventListener('close', () => {
    if (state.micTest && !state.voiceWanted) voice.stop();
    state.micTest = false;
  });
}

/** Square-crop to 256 px (animated GIF/WebP kept as-is if small enough). */
async function prepareAvatar(file) {
  if ((file.type === 'image/gif' || file.type === 'image/webp') && file.size <= 512 * 1024) return file;
  const bmp = await createImageBitmap(file);
  const side = Math.min(bmp.width, bmp.height);
  const canvas = Object.assign(document.createElement('canvas'), { width: 256, height: 256 });
  const ctx = canvas.getContext('2d');
  ctx.imageSmoothingQuality = 'high';
  ctx.drawImage(bmp, (bmp.width - side) / 2, (bmp.height - side) / 2, side, side, 0, 0, 256, 256);
  const webp = await new Promise((r) => canvas.toBlob(r, 'image/webp', 0.9));
  return webp?.type === 'image/webp' ? webp : new Promise((r) => canvas.toBlob(r, 'image/png'));
}

export function renderAwaySettings() {
  const away = awayOf(state.users.get(settings.uid));
  $('away-now').innerHTML = away
    ? `${icon('coffee')}<span>Away: <b>${esc(away.text)}</b>${away.until ? ` · back ${esc(untilText(away.until))}` : ' · until your next message'}</span> <button class="link-btn" type="button" data-action="away-clear">I'm back</button>`
    : '<span class="muted">Not set. People see it when they mention you, or in People.</span>';
}

export function renderChimeSettings() {
  const form = $('settings-form');
  const mine = state.users.get(settings.uid)?.chime ?? 'default';
  const own = [...state.sounds.values()].filter((s) => s.kind === 'chime' && s.uid === settings.uid);
  form.chime.innerHTML = [
    ...Object.entries(CHIMES).map(([id, c]) => `<option value="${id}">${esc(c.label)}</option>`),
    ...own.map((s) => `<option value="s:${esc(s.id)}">Your clip (${(s.ms / 1000).toFixed(1)} s)</option>`),
  ].join('');
  form.chime.value = mine;
}

export function renderNotifySettings() {
  const form = $('settings-form');
  if (!form) return;
  const n = state.notify ?? { mode: 'off', topic: null };
  form.notifyMode.value = n.mode;
  // Quiet hours.
  const q = n.quiet;
  const hhmm = (m) => `${String(Math.floor(m / 60)).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`;
  form.quietOn.checked = Boolean(q && q.from !== q.to);
  if (q && q.from !== q.to) {
    form.quietFrom.value = hhmm(q.from);
    form.quietTo.value = hhmm(q.to);
  }
  form.quietMentions.checked = Boolean(q?.mentions);
  $('quiet-times').hidden = !form.quietOn.checked;
  const snoozed = q?.until && q.until > Date.now();
  $('quiet-snooze').innerHTML = snoozed
    ? `😴 Pushes snoozed until ${esc(untilText(q.until).replace(/^at /, ''))}. <button class="link-btn" type="button" data-action="quiet-unsnooze">Turn back on</button>`
    : '<button class="btn small" type="button" data-action="quiet-snooze">😴 Snooze pushes until tomorrow morning</button>';
  $('ntfy-help').hidden = n.mode === 'off' || !n.topic;
  // This server's ntfy (NTFY_SERVER), else ntfy.sh: subscribing anywhere else would never get a push.
  let server = new URL('https://ntfy.sh');
  try {
    const url = new URL(state.config?.ntfy?.server ?? '');
    if (/^https?:$/.test(url.protocol)) server = url;
  } catch {}
  $('ntfy-host').textContent = server.host;
  if (n.topic) {
    $('ntfy-topic').textContent = n.topic;
    $('ntfy-open').href = `${server.origin}${server.pathname.replace(/\/+$/, '')}/${encodeURIComponent(n.topic)}`;
  }
}

export function renderAvatarSettings() {
  const me = state.users.get(settings.uid);
  $('settings-avatar').innerHTML = avatar(me ?? { name: settings.name, color: settings.color }, 'lg');
  document.querySelector('[data-action="remove-avatar"]').hidden = !me?.avatar;
}

export function copyInvite() {
  const link = settings.key ? `${location.origin}/#key=${encodeURIComponent(settings.key)}` : location.origin;
  navigator.clipboard.writeText(link).then(
    () => toast('Invite link copied.'),
    () => copyByHand('Invite link', link),
  );
}
