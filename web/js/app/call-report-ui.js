// The call report on the page (PROTOCOL.md › Voice › Call report; the notes
// and their text are ../call-report.js). A report starts with a call and is
// told what happens in it: people coming and going, connection states, ICE
// restarts, the socket dropping, settings and devices changing, and the 2 s
// stats readings the connection hints already take. The last one is kept in
// localStorage (only on this device) for Settings › Voice › Copy last call
// report; the call's ⋯ menu copies the one you're in.

import { CallLog, describeBrowser, formatReport } from '../call-report.js';
import { BUILD } from '../build.js';
import { settings } from '../settings.js';
import { $, state } from './state.js';
import { toast } from './ui.js';
import { voice } from './voice-ui.js';

const KEY = 'yap.callReport';
/** The call we're in, or the last one this page was in. */
let log = null;
let netType = null;

const inCall = () => Boolean(log && !log.endedAt);

/**
 * Who a session is, for the report: their name, keyed by account (an OBS link
 * separately from its owner). Null for a session we no longer know (an old
 * one just handed over to a new one): nothing is noted for it.
 */
function who(sid) {
  const session = state.sessions.get(sid);
  const uid = session?.uid ?? voice.peers.get(sid)?.uid;
  if (!uid) return null;
  const name = state.users.get(uid)?.name ?? 'Someone';
  return session?.client === 'obs' ? { key: `${uid}:obs`, name: `${name}'s OBS` } : { key: uid, name };
}

const names = (list) => (list.length < 2 ? list.join('') : `${list.slice(0, -1).join(', ')} and ${list.at(-1)}`);
const onOff = (on) => (on ? 'on' : 'off');

/** Your settings in words, as they are now. */
function describeSettings() {
  const kbps = (b) => `${Math.round(b / 1000)} kbps`;
  const shared = state.channels.get(log?.channelId ?? state.voiceWanted)?.bitrate ?? null;
  const music = settings.music;
  const mode = settings.mode === 'ptt' ? 'push to talk' : settings.mode === 'open' || music ? 'always on' : `voice activity, sensitivity ${settings.vadThreshold} dBFS`;
  return {
    Quality: kbps(settings.bitrate),
    'Channel quality': shared ? `${kbps(shared)} for everyone` : "each person's own",
    'Input mode': mode,
    // (RNNoise once the mic is open; the browser's own where it can't run.)
    'Noise suppression': music ? 'off (music mode)' : !settings.noiseSuppression ? 'off' : voice.rawStream && !voice.rnnoise ? "on (the browser's)" : 'on (RNNoise)',
    'Echo cancellation': music ? 'off (music mode)' : onOff(settings.echoCancellation),
    'Automatic gain': music ? 'off (music mode)' : onOff(settings.autoGain),
    'Music mode': onOff(music),
    'Hide my IP': onOff(settings.relayOnly),
  };
}

/** Your mic and speakers, as the system names them. */
async function describeDevices() {
  let outputs = [];
  try {
    outputs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === 'audiooutput');
  } catch {}
  const picked = settings.outputId ? outputs.find((d) => d.deviceId === settings.outputId) : null;
  const system = outputs.find((d) => d.deviceId === 'default')?.label.replace(/^Default - /, '');
  const mic = voice.rawStream?.getAudioTracks()[0];
  return {
    Microphone: !voice.micOk ? 'none (listening only)' : mic?.label || 'system default',
    Output: picked?.label || (settings.outputId ? 'system default (the picked one is gone)' : system ? `system default (${system})` : 'system default'),
  };
}

function refreshDevices() {
  const at = Date.now();
  describeDevices().then((devices) => inCall() && log.setDevices(devices, { at }));
}

/** Kept on this device: the last call's report, as text. */
function save() {
  if (!log) return;
  try {
    localStorage.setItem(KEY, JSON.stringify({ at: Date.now(), text: formatReport(log) }));
  } catch {}
}

function saved() {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? 'null')?.text ?? null;
  } catch {
    return null;
  }
}

// ── What happens in a call (voice-ui.js and app.js call these) ────────────

/**
 * Our own join came back: a new call — or the same one going on (back after
 * our socket dropped: `handover` = the call was kept, else rejoined).
 */
export function callJoined(channelId, others, { handover = false } = {}) {
  const channel = state.channels.get(channelId)?.name ?? 'a voice channel';
  const there = others.map((sid) => who(sid)?.name ?? 'someone');
  const with_ = there.length ? `${names(there)} ${there.length === 1 ? 'was' : 'were'} there` : 'nobody else was there';
  if (inCall() && log.channelId === channelId) {
    log.note(handover ? 'Back on Yap: the call was kept' : `Rejoined ${channel} as a newcomer; ${with_}`);
    for (const sid of others) {
      const person = who(sid);
      if (handover && person && voice.peers.has(sid)) log.count(person.key, person.name, 'kept');
      if (person) log.joined(person.key, person.name);
    }
    return renderCallReportButton();
  }
  if (inCall()) callEnded(`moved to ${channel}`);
  log = new CallLog({
    channel,
    channelId,
    app: `Yap web ${state.appVersion ?? '(version unknown)'} (build ${BUILD?.app ?? 'unknown'})`,
    device: describeBrowser(navigator.userAgent, navigator.userAgentData ?? null),
  });
  log.setSettings(describeSettings(), { quiet: true });
  log.note(`You joined ${channel}; ${with_}`);
  if (settings.relayOnly && !voice.relayOnly) log.note("Hide my IP is on, but this server has no voice relay: connecting directly");
  else if (voice.relayOnly) log.note('Hiding your IP: everything goes through the voice relay');
  for (const sid of others) {
    const person = who(sid);
    if (person) log.joined(person.key, person.name);
  }
  describeDevices().then((devices) => log?.setDevices(devices, { quiet: true }));
  netType = navigator.connection?.type ?? null;
  save();
  renderCallReportButton();
}

/** A timeline line, if we're in a call. */
export function callNote(text) {
  if (inCall()) log.note(text);
}

/** Someone came into our call (`back`: under a new session, the call kept), or left it (`how`). */
export function callPerson(sid, what, how = '') {
  const person = inCall() ? who(sid) : null;
  if (!person) return;
  const { key, name } = person;
  if (what === 'joined') log.joined(key, name, `${name} joined`);
  else if (what === 'back') {
    log.count(key, name, 'kept');
    log.joined(key, name, `${name} is back: the call was kept`);
  } else if (what === 'away') log.note(`${name} lost their connection to Yap; keeping the call (reconnecting…)`);
  else log.left(key, name, `${name} left${how ? ` (${how})` : ''}`);
}

/** The call is over: why, in a few words ("You left"). The report is kept. */
export function callEnded(reason) {
  if (!inCall()) return;
  log.end(reason);
  save();
  renderCallReportButton();
}

/** A round of the 2 s stats (voice.stats()). */
export function callStats(stats) {
  if (!inCall() || !stats?.peers) return;
  for (const { sid, peer, reading, codecs } of stats.peers) {
    const person = who(sid);
    if (person) log.sample(person.key, person.name, peer, reading, codecs);
  }
  if (log.tick()) save();
}

/** Something in Settings (or the channel's quality) changed: the timeline says what. */
export function callSettingsChanged() {
  if (!inCall()) return;
  log.setSettings(describeSettings());
  refreshDevices();
}

export function callMuteChanged() {
  callNote(settings.deafened ? 'You deafened' : settings.muted ? 'You muted' : 'You unmuted');
}

// ── Copying ──────────────────────────────────────────────────────────────

/** Copy call report (the call's ⋯) / Copy last call report (Settings › Voice). */
export async function copyCallReport() {
  const text = log ? formatReport(log) : saved();
  if (!text) return toast('No call yet.');
  if (inCall()) save();
  try {
    await navigator.clipboard.writeText(text);
    return toast('Call report copied.');
  } catch {}
  // Where the clipboard API says no: the old way, from a hidden text box. Failing that, a file (a
  // one-line box to copy from by hand would lose the line breaks).
  const box = Object.assign(document.createElement('textarea'), { value: text, readOnly: true });
  box.style.cssText = 'position:fixed;top:0;left:0;opacity:0;';
  document.body.append(box);
  box.select();
  let copied = false;
  try {
    copied = document.execCommand('copy');
  } catch {}
  box.remove();
  if (copied) return toast('Call report copied.');
  const link = Object.assign(document.createElement('a'), { href: URL.createObjectURL(new Blob([text], { type: 'text/plain' })), download: 'yap-call-report.txt' });
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 10_000);
  toast("Your browser wouldn't copy it, so the call report was saved as a file.");
}

/** Settings › Voice: "Copy call report" in a call, "Copy last call report" after one, disabled with "No call yet." before any. */
export function renderCallReportButton() {
  const button = $('call-report-copy');
  if (!button) return;
  const have = Boolean(log) || Boolean(saved());
  button.textContent = inCall() ? 'Copy call report' : 'Copy last call report';
  button.disabled = !have;
  $('call-report-none').hidden = have;
}

// ── Listening ────────────────────────────────────────────────────────────

export function bindCallReport() {
  /** Someone in the call we're in (null: not in a call, or not someone we know). */
  const person = (sid) => (inCall() ? who(sid) : null);
  voice.addEventListener('peer', ({ detail: { sid, state: s } }) => {
    const p = s === 'closed' ? null : person(sid); // (closing is said by whoever leaves)
    if (p) log.connection(p.key, p.name, s);
  });
  voice.addEventListener('quality', ({ detail: { sid } }) => {
    const p = person(sid);
    if (p) log.weak(p.key, p.name, voice.peers.get(sid)?.weak ?? null);
  });
  voice.addEventListener('own-quality', ({ detail: { weak } }) => inCall() && log.ownWeak(weak));
  voice.addEventListener('restart', ({ detail: { sid, attempt } }) => {
    const p = person(sid);
    if (!p) return;
    log.count(p.key, p.name, 'restarts');
    log.note(`${p.name}: ICE restart (try ${attempt})`);
  });
  voice.addEventListener('rebuild', ({ detail: { sid, by } }) => {
    const p = person(sid);
    if (!p) return;
    log.count(p.key, p.name, 'rebuilds');
    log.note(`${p.name}: connection rebuilt (${by === 'us' ? 'we started over' : 'they started over'})`);
  });
  let health = { silent: false, stalled: false };
  voice.addEventListener('health', () => {
    const now = { silent: voice.silent, stalled: voice.stalled };
    if (now.silent !== health.silent) callNote(now.silent ? 'Your mic is sending silence (nothing at all comes from it)' : 'Your mic is sending sound again');
    if (now.stalled !== health.stalled) callNote(now.stalled ? "The browser paused Yap's audio processing: sending the plain mic" : 'Audio processing is back');
    health = now;
  });
  voice.addEventListener('mic', ({ detail }) => {
    if (!inCall()) return;
    if (!detail.ok) callNote(`Microphone unavailable (${detail.error?.name ?? 'error'}): listening only`);
    callSettingsChanged(); // RNNoise may have loaded (or not); the mic's name may be another
  });
  let context = null;
  voice.addEventListener('context', ({ detail: { state: s } }) => {
    if (inCall() && context && s !== context && s !== 'closed') {
      callNote(s === 'running' ? 'Audio running again' : s === 'interrupted' ? 'Audio interrupted by the system (a phone call, Siri…)' : 'Audio paused by the browser');
    }
    context = s;
  });
  navigator.mediaDevices?.addEventListener?.('devicechange', () => setTimeout(() => inCall() && refreshDevices(), 1000)); // after voice.js has followed the change
  navigator.connection?.addEventListener?.('change', () => {
    const type = navigator.connection.type ?? null;
    if (type && netType && type !== netType) callNote(`Network: ${netType} → ${type}`);
    netType = type ?? netType;
  });
  addEventListener('offline', () => callNote('This device went offline'));
  addEventListener('online', () => callNote('This device is online again'));
  addEventListener('pagehide', () => callEnded('the page was closed or reloaded'));
  $('settings-form')?.addEventListener('change', () => setTimeout(callSettingsChanged, 0)); // after Settings has saved it
}
