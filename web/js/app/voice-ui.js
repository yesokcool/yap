// Voice on the page: joining and leaving, mute and deafen, the voice panel
// and who's in a call, calls kept across reconnects, and screen sharing. The
// WebRTC side is ../voice.js.

import { SCREEN_QUALITY, Voice } from '../voice.js';
import { settings, save, api } from '../settings.js';
import { sounds, playChime, playClip, stopClips } from '../sounds.js';
import { icon } from '../icons.js';
import { esc } from '../format.js';
import { placeNear } from '../place.js';
import { ScreenView } from '../screen.js';
import { endResume, renderAll } from '../app.js';
import { $, conn, state } from './state.js';
import { avatar, holdFocus, popoverOpened, toast } from './ui.js';
import { maybeUpdate } from './updates.js';
import { setView } from './places.js';
import { renderChannels, renderHeader, renderUserPanel } from './sidebar.js';
import { renderMembers } from './people.js';
import { soundboard } from './extras.js';
import { renderMeter } from './settings-ui.js';
import { callEnded, callJoined, callMuteChanged, callPerson, callStats } from './call-report-ui.js';

let statsTimer = null;

export const voice = new Voice({
  signal: (to, data) => conn?.send({ t: 'signal', to, data }),
  uidOf: (sid) => state.sessions.get(sid)?.uid,
  resumable: (sid) => Boolean(state.sessions.get(sid)?.voice?.resume),
  fetchIce: () => fetchIce(),
});

export async function fetchIce() {
  try {
    const res = await api('/api/ice');
    return (await res.json()).iceServers;
  } catch {
    return [{ urls: 'stun:stun.cloudflare.com:3478' }];
  }
}

/** Does this server hand out a TURN relay (needed for relay-only voice)? */
export const hasRelay = (servers) => servers.some((s) => [s.urls].flat().some((u) => /^turns?:/.test(u)));

const GHOST_MS = 30_000;

export function onVoice(msg) {
  if (msg.sid === state.staleSid) return; // our own old session going (see onReady): no chime, no "reconnecting…"
  const session = state.sessions.get(msg.sid);
  const prev = session?.voice?.channelId ?? null;
  const wasResumable = Boolean(session?.voice?.resume);
  const wasSharing = Boolean(session?.voice?.screen);
  if (session) {
    session.voice = msg.channelId
      ? { channelId: msg.channelId, muted: msg.muted, deafened: msg.deafened, ...(msg.screen ? { screen: true } : {}), ...(msg.resume ? { resume: true } : {}), ...(msg.prev ? { prev: msg.prev } : {}) }
      : null;
  }
  // Someone continues an earlier session's call (maybe one we're keeping): move it over.
  const resumed = msg.channelId && msg.sid !== state.sid ? adoptResumed({ sid: msg.sid, uid: session?.uid, voice: session?.voice }) : false;
  // (A sharer whose socket dropped may be back in a moment, screen and all: keep watching.)
  if (!msg.screen && state.screenSid === msg.sid && !(msg.lost && wasResumable)) sharerStopped();

  if (msg.sid === state.sid) {
    if (msg.channelId && msg.channelId !== state.voiceChannel) {
      state.voiceChannel = msg.channelId;
      state.voiceWanted = msg.channelId;
      const others = [...state.sessions.values()]
        .filter((s) => s.sid !== state.sid && s.sid !== state.staleSid && s.sid !== state.resume?.sid && s.voice?.channelId === msg.channelId && !state.ghosts.has(s.sid))
        .map((s) => s.sid);
      const handover = Boolean(state.resume && msg.prev === state.resume.sid);
      if (handover) {
        voice.handover(others); // the same call, under our new session
      } else {
        // A fresh join (or the server didn't take our resume): start over.
        for (const sid of [...state.ghosts.keys()]) dropGhost(sid);
        voice.hangupAll();
        voice.enter(others);
      }
      if (!state.rejoining) sounds.join();
      state.rejoining = false;
      endResume();
      callJoined(msg.channelId, others, { handover });
      startStats();
    } else if (!msg.channelId && state.voiceChannel) {
      voice.hangupAll();
      state.voiceChannel = null;
      // Not a switch we asked for (e.g. the channel was archived): we're out.
      if (state.voiceWanted === prev) {
        state.voiceWanted = null;
        callEnded('Yap took you out of the channel (it may have been archived)');
        voice.stop();
        sounds.leave();
        stopStats();
      }
    }
  } else if (state.voiceChannel) {
    if (msg.channelId === state.voiceChannel && prev !== state.voiceChannel) {
      const uid = session?.uid;
      // The same call back under a new session: no chimes (V7: nor when anyone's straight back).
      let quiet = resumed;
      if (!resumed) {
        // A plain join by someone whose old connection we were keeping: they
        // couldn't resume it (or their app can't), so it's dead weight now.
        for (const sid of [...state.ghosts.keys()]) {
          if (state.sessions.get(sid)?.uid !== uid) continue;
          dropGhost(sid, { quiet: true });
          quiet = true;
        }
      }
      if (state.leaveChimes.has(uid)) {
        // Back within 5 s of losing their connection: neither chime.
        clearTimeout(state.leaveChimes.get(uid));
        state.leaveChimes.delete(uid);
        quiet = true;
      }
      callPerson(msg.sid, resumed ? 'back' : 'joined');
      if (!quiet) {
        // Their own join chime (they offer to us once their join lands).
        const chime = state.users.get(uid)?.chime;
        playChime(chime, chime?.startsWith('s:') ? state.sounds.get(chime.slice(2)) : null);
      }
    } else if (prev === state.voiceChannel && msg.channelId !== state.voiceChannel) {
      if (msg.lost && wasResumable && voice.peers.has(msg.sid)) {
        // Their socket dropped but the call may well continue: keep it, quietly.
        if (session) session.voice = { channelId: prev, muted: msg.muted, deafened: msg.deafened, resume: true, ...(wasSharing ? { screen: true } : {}) };
        holdGhost(msg.sid);
        callPerson(msg.sid, 'away');
      } else {
        callPerson(msg.sid, 'left', msg.lost ? 'lost their connection' : '');
        voice.drop(msg.sid);
        if (msg.lost) deferLeaveChime(session?.uid);
        else sounds.peerLeave();
      }
    }
  }
  renderChannels();
  renderVoicePanel();
  renderMembers();
}

/**
 * A session that continues an earlier one's call (`voice.prev`, checked by
 * the server): move our connection — and everything we keep by sid — over to
 * it. Returns whether there was a connection (or a ghost) to move.
 */
export function adoptResumed(session) {
  const prev = session.voice?.prev;
  if (!prev || prev === session.sid || prev === state.sid) return false;
  const old = state.sessions.get(prev);
  const peer = voice.peers.get(prev);
  // The server checked it's the same account; check again, cheaply.
  if ((old && old.uid !== session.uid) || (peer?.uid && peer.uid !== session.uid)) return false;
  const had = state.ghosts.has(prev) || Boolean(peer);
  clearGhost(prev);
  state.sessions.delete(prev);
  voice.rekey(prev, session.sid);
  if (state.screenSid === prev) {
    state.screenSid = session.sid;
    screenView?.moveTo(session.sid);
  }
  state.speaking.delete(prev);
  return had;
}

/**
 * Keep someone's connection while their socket is gone (they may resume):
 * up to GHOST_MS, or until the connection itself fails / has been
 * disconnected for 5 s. They stay listed, marked "reconnecting…".
 */
export function holdGhost(sid) {
  clearGhost(sid);
  const session = state.sessions.get(sid);
  if (session) {
    session.view = null;
    session.watching = false;
  }
  state.ghosts.set(sid, { timer: setTimeout(() => dropGhost(sid), GHOST_MS), lost: null });
  ghostHealth(sid, voice.peers.get(sid)?.pc.connectionState);
}

function ghostHealth(sid, peerState) {
  const ghost = state.ghosts.get(sid);
  if (!ghost) return;
  if (peerState === 'failed' || peerState === 'closed' || !peerState) return dropGhost(sid);
  if (peerState === 'connected') {
    clearTimeout(ghost.lost);
    ghost.lost = null;
  } else if (!ghost.lost) {
    ghost.lost = setTimeout(() => dropGhost(sid), 5000);
  }
}

function clearGhost(sid) {
  const ghost = state.ghosts.get(sid);
  if (!ghost) return;
  clearTimeout(ghost.timer);
  clearTimeout(ghost.lost);
  state.ghosts.delete(sid);
}

/** They didn't come back: hang up on the kept connection (and chime, unless `quiet`). */
export function dropGhost(sid, { quiet = false } = {}) {
  if (!state.ghosts.has(sid)) return;
  const uid = state.sessions.get(sid)?.uid;
  callPerson(sid, 'left', quiet ? 'their old connection, closed' : "they didn't come back");
  if (state.screenSid === sid) sharerStopped(); // they never came back
  clearGhost(sid);
  state.sessions.delete(sid);
  voice.drop(sid);
  if (!quiet && state.voiceChannel && ![...state.sessions.values()].some((s) => s.uid === uid && s.voice?.channelId === state.voiceChannel)) sounds.peerLeave();
  renderChannels();
  renderVoicePanel();
  renderMembers();
}

/** A connection lost without a goodbye: chime in 5 s, unless they're back by then (V7). */
export function deferLeaveChime(uid) {
  if (!uid) return sounds.peerLeave();
  clearTimeout(state.leaveChimes.get(uid));
  state.leaveChimes.set(uid, setTimeout(() => {
    state.leaveChimes.delete(uid);
    if (state.voiceChannel) sounds.peerLeave();
  }, 5000));
}

// ── Voice actions ─────────────────────────────────────────────────────────

voice.addEventListener('speaking', ({ detail: { sid, speaking } }) => {
  if (speaking) state.speaking.add(sid);
  else state.speaking.delete(sid);
  for (const el of document.querySelectorAll(`[data-voice-sid="${CSS.escape(sid)}"]`)) {
    el.classList.toggle('speaking', speaking);
  }
});
voice.addEventListener('screen-local', () => {
  sendVoiceState();
  renderVoicePanel();
  renderSharePanel();
});
voice.addEventListener('viewers', () => renderSharePanel());
voice.addEventListener('screen', ({ detail: { sid, stream } }) => screenView?.setStream(sid, stream));
voice.addEventListener('peer', ({ detail: { sid, state: s } }) => {
  if (s === 'closed') state.peerStates.delete(sid);
  else state.peerStates.set(sid, s);
  ghostHealth(sid, s === 'closed' ? null : s);
  renderChannels();
  renderVoicePanel();
});
voice.addEventListener('mic', ({ detail }) => {
  if (!detail.ok && state.voiceWanted) toast(micErrorText(detail.error));
  sendVoiceState();
  renderUserPanel();
});
voice.addEventListener('health', () => renderVoicePanel());
voice.addEventListener('quality', () => renderChannels());
voice.addEventListener('level', ({ detail: db }) => {
  if ($('settings-dialog').open) renderMeter(db);
});

export async function joinVoice(channelId) {
  if (!channelId || state.voiceWanted === channelId) return;
  if (state.status !== 'online') return toast('Not connected yet.');
  state.voiceWanted = channelId;
  voice.setChannelBitrate(state.channels.get(channelId)?.bitrate ?? null);
  renderVoicePanel();
  renderHeader(); // a Place's "voice keeps going"
  try {
    const ice = await fetchIce();
    voice.relayOnly = settings.relayOnly && hasRelay(ice);
    await voice.prepare(ice);
  } catch (err) {
    console.warn('voice setup failed', err);
  }
  if (state.voiceWanted !== channelId) return; // changed their mind meanwhile
  voice.setMuted(settings.muted);
  voice.setDeafened(settings.deafened);
  // `resume`: we keep calls across reconnects. `prev`/`proof`: this join continues our call from before the reconnect.
  const resume = state.resume && state.resume.channelId === channelId ? state.resume : null;
  conn.send({
    t: 'voice.join', channelId, muted: settings.muted || !voice.micOk, deafened: settings.deafened, resume: true,
    ...(resume ? { prev: resume.sid, proof: resume.key, ...(voice.screenStream ? { screen: true } : {}) } : {}),
  });
}

export function leaveVoice() {
  if (!state.voiceWanted) return;
  state.voiceWanted = null;
  state.voiceChannel = null;
  state.rejoining = false;
  endResume();
  for (const sid of [...state.ghosts.keys()]) dropGhost(sid, { quiet: true });
  conn?.send({ t: 'voice.leave' });
  callEnded('you left');
  stopClips();
  soundboard?.render();
  voice.stopScreen();
  stopWatchingScreen();
  voice.stop();
  state.speaking.clear();
  state.peerStates.clear();
  sounds.leave();
  stopStats();
  renderAll();
  maybeUpdate();
}

export function toggleMute() {
  if (settings.deafened) save({ deafened: false, muted: false });
  else save({ muted: !settings.muted });
  applyVoiceToggles();
  (settings.muted ? sounds.mute : sounds.unmute)();
}

export function toggleDeafen() {
  save({ deafened: !settings.deafened });
  if (settings.deafened) stopClips();
  applyVoiceToggles();
  (settings.deafened ? sounds.mute : sounds.unmute)();
}

function applyVoiceToggles() {
  callMuteChanged();
  voice.setMuted(settings.muted);
  voice.setDeafened(settings.deafened);
  screenView?.applyAudio(); // deafened: the screen's sound goes quiet too
  sendVoiceState();
  renderUserPanel();
}

function sendVoiceState() {
  if (state.voiceChannel) {
    conn?.send({ t: 'voice.state', muted: settings.muted || !voice.micOk, deafened: settings.deafened, screen: Boolean(voice.screenStream) });
  }
}

function startStats() {
  stopStats();
  statsTimer = setInterval(async () => {
    state.voiceStats = await voice.stats();
    callStats(state.voiceStats);
    renderVoicePanel();
  }, 2000);
}

function stopStats() {
  clearInterval(statsTimer);
  statsTimer = null;
  state.voiceStats = null;
}

/** A soundboard clip someone in our voice channel played (not if we're deafened, or they're muted for us). */
export function onSound({ id, uid }) {
  const sound = state.sounds.get(id);
  if (!sound || !state.voiceChannel || settings.deafened) return;
  if ((settings.volumes[uid] ?? 1) === 0) return;
  state.soundsHeard = (state.soundsHeard ?? 0) + 1; // (tests read this; there's no hearing it headless)
  playClip(sound);
}

function micErrorText(error) {
  if (error?.name === 'NotAllowedError') return 'Microphone blocked — you can listen but not talk. Allow it in the browser and rejoin.';
  if (error?.name === 'NotFoundError') return 'No microphone found — you can listen but not talk.';
  return 'Microphone unavailable — you can listen but not talk.';
}

export function voiceMember(session) {
  const user = state.users.get(session.uid);
  if (session.client === 'obs') {
    return `<li class="voice-member obs" data-voice-sid="${esc(session.sid)}" title="An OBS link of ${esc(user?.name ?? 'someone')}'s is listening to this call">
      <span class="obs-badge">📺</span><span class="name">${esc(user?.name ?? 'Someone')}'s OBS</span></li>`;
  }
  const me = session.sid === state.sid;
  const inMyChannel = state.voiceChannel && session.voice?.channelId === state.voiceChannel;
  const ghost = state.ghosts.has(session.sid);
  const connecting = inMyChannel && !me && !ghost && state.peerStates.get(session.sid) !== 'connected';
  const flag = session.voice.deafened ? icon('headphones-off') : session.voice.muted ? icon('mic-off') : '';
  const note = inMyChannel && !me ? voiceNote(session, user, ghost) : null;
  return `
    <li class="voice-member ${state.speaking.has(session.sid) ? 'speaking' : ''} ${connecting ? 'connecting' : ''} ${ghost ? 'reconnecting' : ''}"
        data-voice-sid="${esc(session.sid)}" ${me ? '' : `data-action="member-menu" data-uid="${esc(session.uid)}" role="button" tabindex="0" aria-haspopup="dialog"`}
        title="${connecting ? 'Connecting…' : esc(user?.name ?? '')}">
      ${avatar(user, 'sm')}<span class="name">${esc(user?.name ?? 'Unknown')}</span>
      ${note ? `<span class="vq ${note.kind}" title="${esc(note.title)}">${esc(note.text)}</span>` : ''}
      ${session.voice.screen ? `<button class="live-badge" data-action="${me ? 'share-screen' : 'watch-screen'}" data-sid="${esc(session.sid)}" title="${me ? 'You’re sharing your screen (click to stop)' : `Watch ${esc(user?.name ?? 'their')}'s screen`}">LIVE</button>` : ''}
      <span class="flags" ${flag ? `role="img" aria-label="${session.voice.deafened ? 'deafened' : 'muted'}"` : ''}>${flag}</span>
    </li>`;
}

/** A quiet word next to someone in your call, only when something's off (else nothing). */
function voiceNote(session, user, ghost) {
  const name = user?.name ?? 'They';
  if (ghost) return { kind: 'away', text: 'reconnecting…', title: `${name} lost their connection to Yap. Their voice is still coming through; it usually sorts itself out in a few seconds.` };
  const peer = voice.peers.get(session.sid);
  if (!peer) return null;
  if (peer.failed && peer.pc.connectionState !== 'connected') {
    return hasRelay(voice.iceServers)
      ? { kind: 'blocked', text: "can't connect", title: `Your network and ${name}'s can't reach each other right now. Yap keeps trying.` }
      : { kind: 'blocked', text: 'needs the voice relay', title: `Your network and ${name}'s can't reach each other directly (strict firewalls or mobile networks do this). The server's owner can switch on the voice relay, then it just works — see SELF_HOSTING.md.` };
  }
  if (peer.weak) {
    const pct = Math.round(peer.weak.concealed * 100);
    const why = [pct >= 3 ? `about ${pct}% of their audio is getting lost or arriving late` : '', (peer.weak.rtt ?? 0) > 300 ? `the delay is ${Math.round(peer.weak.rtt)} ms` : ''].filter(Boolean).join(', and ');
    return { kind: 'weak', text: 'weak connection', title: `${name}'s voice may break up: ${why}. It usually passes; if not, one of you is on a busy network.` };
  }
  return null;
}

export function renderVoicePanel() {
  const panel = $('voice-panel');
  if (!state.voiceWanted) {
    panel.hidden = true;
    return;
  }
  const channel = state.channels.get(state.voiceWanted);
  const peers = [...state.peerStates.values()];
  const connected = state.voiceChannel === state.voiceWanted && peers.every((s) => s === 'connected');
  const stats = state.voiceStats;
  const resuming = !connected && Boolean(state.resume);
  const details = [
    esc(channel?.name ?? ''),
    // (On the second line, like the desktop app: on one it ran under the buttons.)
    resuming ? 'voice is still on' : stats?.rtt != null ? `${stats.rtt}\u00a0ms` : '',
  ].filter(Boolean).join(' · ');
  // The channel can set one quality for everyone in it: said here only when it has (the picker is under ⋯).
  const shared = channel?.bitrate ?? null;
  panel.hidden = false;
  const refocus = holdFocus(panel); // (it's redrawn every 2 s with the call's stats)
  panel.innerHTML = `
    <div class="vp-text">
      <div class="vp-status ${connected ? 'ok' : 'pending'}">${icon('signal')}<span>${connected ? 'Voice connected' : resuming ? 'Reconnecting…' : 'Connecting…'}</span></div>
      <div class="vp-sub"><span>${details}${settings.music ? ' · stereo' : ''}</span><button type="button" class="icon-btn tiny vp-more" data-action="voice-more" title="Call settings: sound quality for everyone here, and the call report" aria-label="Call settings">${icon('more')}</button></div>
      ${shared ? `<div class="vp-shared" title="Set for everyone in ${esc(channel?.name ?? 'this channel')} (⋯ changes it)">Sound: ${shared / 1000}k for everyone</div>` : ''}
      ${voice.micOk || state.voiceChannel !== state.voiceWanted ? '' : '<div class="vp-warn">Listening only — no mic</div>'}
      ${voice.micOk && voice.silent && !settings.muted && !settings.deafened ? '<div class="vp-warn" title="Nothing at all is coming from your microphone. Check its mute switch, or pick another one in Settings › Voice.">Your mic is sending silence</div>' : ''}
      ${voice.micOk && voice.stalled ? '<div class="vp-warn" title="Your browser paused Yap\'s audio processing. Others still hear you, just without noise removal or voice activity. Click anywhere to bring it back.">Sending your plain mic for now</div>' : ''}
    </div>
    <div class="vp-buttons">
      ${canShareScreen && !voice.screenStream ? `<button class="btn small" data-action="share-screen" title="Share your screen with this channel">${icon('monitor')}<span>Share screen</span></button>` : ''}
      <button class="btn small" data-action="soundboard" title="Soundboard: short clips for everyone in this channel">${icon('music')}<span>Sounds</span></button>
      <button class="btn small" data-action="leave-voice" title="Disconnect from voice">${icon('phone-off')}<span>Leave</span></button>
    </div>`;
  refocus();
}

// (Not on phones, and not where the page's Permissions-Policy would refuse it.)
const canShareScreen = Boolean(navigator.mediaDevices?.getDisplayMedia) && !matchMedia('(pointer: coarse)').matches
  && document.featurePolicy?.allowsFeature?.('display-capture') !== false;
export let screenView = null;

/** Share screen: ask what to share, how sharp, and with its sound or not. (While sharing, the LIVE badge stops it.) */
export function toggleScreenShare() {
  if (voice.screenStream) return voice.stopScreen();
  const form = $('share-form');
  form.surface.value = settings.shareSurface;
  form.quality.value = settings.shareQuality;
  form.sound.checked = settings.shareSound;
  $('share-dialog').showModal();
}

/** What a refused or failed screen picker means, in plain words (cancelling it says nothing). */
function shareFailed(err) {
  const refused = err?.name === 'NotAllowedError';
  if (refused && /by system/i.test(err.message)) toast('Your computer didn’t let the browser share the screen. On a Mac, allow it in System Settings › Privacy & Security › Screen Recording, then try again.');
  else if (refused && /policy/i.test(err.message)) toast('This page isn’t allowed to share your screen. Try another browser, or tell whoever runs this Yap.');
  else if (!refused) toast(`Couldn't share your screen: ${err.message}`);
}

export function bindShareDialog() {
  $('share-dialog').addEventListener('close', async () => {
    if ($('share-dialog').returnValue !== 'share') return;
    const form = $('share-form');
    save({ shareSurface: form.surface.value, shareQuality: form.quality.value, shareSound: form.sound.checked });
    try {
      // Still inside the click that closed the dialog, so the browser allows the picker.
      await voice.startScreen({ surface: settings.shareSurface, quality: settings.shareQuality, sound: settings.shareSound });
    } catch (err) {
      shareFailed(err);
    }
  });
  // While sharing (the panel under the voice panel): quality and sound, live.
  $('share-panel').addEventListener('change', (e) => {
    if (e.target.name === 'share-quality') {
      save({ shareQuality: e.target.value });
      voice.setScreenQuality(e.target.value);
    } else if (e.target.name === 'share-sound') {
      save({ shareSound: e.target.checked });
      voice.setScreenSound(e.target.checked);
    }
  });
}

/** Share something else: another window, screen or tab, without stopping (viewers keep watching). */
export async function changeScreenShare() {
  try {
    await voice.changeScreen({ surface: settings.shareSurface });
  } catch (err) {
    shareFailed(err);
  }
}

const QUALITY_HINTS = {
  smooth: '1080p, 60 fps. Games and video.',
  sharp: '1440p, 30 fps. Text, code, slides.',
  max: 'Up to 4K, 60 fps. Fast upload, one or two people watching.',
};

/**
 * While you share: who's watching (from their want/stop signals; someone
 * whose connection goes is dropped), the quality and Share sound (both change
 * live), Share something else, Stop sharing. Redrawn only when one of those
 * changes, not with the call's stats.
 */
export function renderSharePanel() {
  const panel = $('share-panel');
  if (!voice.screenStream || !state.voiceWanted) {
    panel.hidden = true;
    panel.replaceChildren();
    return;
  }
  const names = [...new Set(voice.viewers.map((sid) => state.users.get(state.sessions.get(sid)?.uid)?.name ?? 'Someone'))];
  const quality = Object.keys(SCREEN_QUALITY).find((k) => SCREEN_QUALITY[k] === voice.screenQuality) ?? 'smooth';
  const sound = voice.screenHasSound;
  const refocus = holdFocus(panel);
  panel.hidden = false;
  panel.innerHTML = `
    <div class="sp-head">${icon('monitor')}<b>Sharing your screen</b></div>
    <div class="sp-viewers">${names.length ? `Watching: ${esc(names.join(', '))}` : "No one's watching yet"}</div>
    <div class="segmented sp-quality" role="radiogroup" aria-label="Quality">
      ${Object.entries(SCREEN_QUALITY).map(([k, q]) => `<label title="${esc(QUALITY_HINTS[k])}"><input type="radio" name="share-quality" value="${k}" ${k === quality ? 'checked' : ''}><span>${esc(q.label)}</span></label>`).join('\n      ')}
    </div>
    <label class="toggle sp-sound" title="${sound ? 'The sound of what you share goes to whoever watches.' : 'This share has no sound: browsers share a tab’s sound, or the whole screen’s on Windows.'}">
      <input type="checkbox" name="share-sound" ${sound && voice.screenSound ? 'checked' : ''} ${sound ? '' : 'disabled'}><span>Share sound</span>
    </label>
    <div class="sp-buttons">
      <button type="button" class="btn small" data-action="share-change" title="Another window, screen or tab, without stopping">Share something else</button>
      <button type="button" class="btn small" data-action="share-stop">Stop sharing</button>
    </div>`;
  refocus();
}

/** Open someone's screen share (joining their voice channel first if needed). */
export function watchScreen(sid) {
  const session = state.sessions.get(sid);
  if (!session?.voice?.screen) return;
  if (state.voiceChannel !== session.voice.channelId) joinVoice(session.voice.channelId);
  if (state.screenSid && state.screenSid !== sid) voice.watchScreen(state.screenSid, false);
  clearTimeout(closeTimer);
  state.screenSid = sid;
  const uidOf = (s) => state.sessions.get(s)?.uid;
  screenView ??= new ScreenView($('screen-view'), {
    who: (s) => state.users.get(uidOf(s)) ?? null,
    // The share's sound has its own volume per person (0–100 %), apart from their voice's.
    volume: (s) => settings.screenVolumes[uidOf(s)] ?? 1,
    setVolume: (s, v) => uidOf(s) && save({ screenVolumes: { ...settings.screenVolumes, [uidOf(s)]: v } }),
    muted: () => settings.deafened,
    sinkId: () => settings.outputId,
    stop: () => {
      stopWatchingScreen();
      setView('chat');
    },
    // Closed the small window while elsewhere in Yap: done watching (the sharer sees it).
    leftPip: () => state.view !== 'screen' && stopWatchingScreen(),
  });
  screenView.open(sid);
  voice.watchScreen(sid, true);
  const already = voice.screens.get(sid);
  if (already) screenView.setStream(sid, already);
  setView('screen');
}

let closeTimer = null;

/** The one we watch stopped sharing (or left): say so, then close the viewer after 2 s. */
function sharerStopped() {
  const sid = state.screenSid;
  if (!sid || !screenView) return stopWatchingScreen();
  screenView.ended();
  // Popped out while you're elsewhere in Yap: the view isn't on screen, so say it here too.
  if (state.view !== 'screen') toast(`${screenView.name} stopped sharing.`);
  clearTimeout(closeTimer);
  closeTimer = setTimeout(() => state.screenSid === sid && stopWatchingScreen(), 2000);
}

export function stopWatchingScreen() {
  clearTimeout(closeTimer);
  if (!state.screenSid) return;
  voice.watchScreen(state.screenSid, false);
  screenView?.close();
  state.screenSid = null;
  if (state.view === 'screen') setView('chat');
}

/** Voice panel › ⋯: the sound quality for everyone in this channel (or each person's own), and the call report. */
export function openVoiceMore(el) {
  const channel = state.channels.get(state.voiceWanted);
  if (!channel) return;
  const shared = channel.bitrate ?? null;
  const pop = $('popover');
  pop.innerHTML = `
    <div class="pop-title">${icon('volume')}<span>${esc(channel.name)}</span></div>
    <label class="field"><span>Sound quality for everyone here</span>
      <span class="combo"><select class="vp-quality" data-channel="${esc(channel.id)}">
        <option value="" ${shared === null ? 'selected' : ''}>Each person's own (yours: ${Math.round(settings.bitrate / 1000)}k)</option>
        ${[64000, 96000, 128000, 256000, 510000].map((b) => `<option value="${b}" ${shared === b ? 'selected' : ''}>${b / 1000}k for everyone</option>`).join('')}
      </select></span>
    </label>
    <small class="muted">Your own quality is in Settings › Voice › Advanced.</small>
    <div class="pop-actions"><button type="button" class="btn small" data-action="copy-call-report" title="What Yap noticed about this call: connections, lost audio, devices. No messages or IP addresses.">Copy call report</button></div>`;
  pop.hidden = false;
  placeNear(pop, el);
  popoverOpened(el);
}
