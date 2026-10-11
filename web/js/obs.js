/*
 * OBS voice output: a Browser Source that joins a voice channel listen-only
 * and plays it, with an optional overlay of who's talking. The link comes
 * from Settings › Voice › Advanced › Stream to OBS:
 *   /obs#key=<invite key>&listen=<listen-only token>&channel=<voice channel id>
 *        [&only=<name>]   play just one person (one source per person)
 *        [&overlay=0]     audio only, nothing drawn
 * The token can only listen (the server enforces it); the apps leave it out
 * of the channel list and its chimes (2026-10-11). It listens only while its
 * account is in the channel, or for 5 minutes after it left (the server
 * enforces that too: PROTOCOL.md › Voice › OBS links in a call): otherwise
 * it says "Paused: waiting for <name> to join <channel>." and joins again by
 * itself as soon as its account is back.
 *
 * Straight through, with as little delay as it can (2026-10-11): it still
 * connects to each person (voice is peer to peer), but that's all it takes
 * from a call. No mic, no gate, no audio graph, no per-person volume or
 * limiter: each person's sound plays from an <audio> element of its own, at
 * full volume (OBS sets the volume). Every audio receiver asks for the shortest
 * jitter buffer, and its descriptions ask for 10 ms packets (voice.js ›
 * listenOnly). RED stays as negotiated: it measured no added delay. Who's
 * speaking comes from the packets' own audio level (getSynchronizationSources),
 * not from listening to the sound. Measured (headless Chromium, localhost):
 * a click plays ~38 ms after it leaves its sender, from ~48 ms; a talker's
 * first sound comes out ~18 ms sooner (92 ms from 110, always-on). While it
 * listens it also keeps itself audible (keepAudible), so OBS never stops
 * capturing it in a pause.
 */
import { Connection } from './conn.js';
import { Voice } from './voice.js';
import { esc, initial } from './format.js';
import { keyProtocols, socketUrl } from './settings.js';

const params = new URLSearchParams(location.hash.slice(1));
const key = params.get('key') ?? '';
const token = params.get('listen') ?? '';
const channelId = params.get('channel') ?? '';
const only = (params.get('only') ?? '').toLowerCase();
const overlay = params.get('overlay') !== '0';
const $ = (id) => document.getElementById(id);

const state = { sid: null, uid: null, users: new Map(), sessions: new Map(), channels: new Map(), inCall: false, joining: false, paused: false, speaking: new Set() };
const voice = new Voice({
  signal: (to, data) => conn.send({ t: 'signal', to, data }),
  uidOf: (sid) => state.sessions.get(sid)?.uid,
  listenOnly: true,
});
// Like the app: the key in the subprotocol, or in the address when it has characters a subprotocol can't carry.
const conn = new Connection(socketUrl(key), keyProtocols(key));
globalThis.yapObs = { state, voice };

function note(text) {
  $('note').hidden = !text;
  $('note').textContent = text ?? '';
}

/**
 * Keeps the page audible while it listens. Chromium calls a page silent once
 * what it plays stays under -72.25 dBFS (its kSilenceThresholdDBFS) for 2 s,
 * and OBS's browser (CEF) stops capturing the page's sound 2 s after that,
 * then starts again at the next sound: in OBS only, the first moment of
 * everyone's first words after a pause came out cut and glitchy (2026-10-11).
 * Pauses in a call are that quiet (encoded silence from a closed gate, or
 * noise suppression: measured -84 to -94 dBFS). So the page also plays a
 * 20 Hz hum at -60 dBFS, far below what anyone can hear, from an <audio>
 * element of its own (no audio graph), looping a 1 s WAV that ends where it
 * starts. Only while it listens: Paused lets CEF stop.
 */
function keepAudible() {
  const rate = 48000;
  const amp = 32767 * 10 ** (-60 / 20) * Math.SQRT2; // -60 dBFS RMS
  const wav = new DataView(new ArrayBuffer(44 + rate * 2));
  const ascii = (at, s) => [...s].forEach((c, i) => wav.setUint8(at + i, c.charCodeAt(0)));
  ascii(0, 'RIFF');
  wav.setUint32(4, 36 + rate * 2, true);
  ascii(8, 'WAVEfmt ');
  wav.setUint32(16, 16, true);
  wav.setUint16(20, 1, true); // PCM
  wav.setUint16(22, 1, true); // mono
  wav.setUint32(24, rate, true);
  wav.setUint32(28, rate * 2, true);
  wav.setUint16(32, 2, true);
  wav.setUint16(34, 16, true);
  ascii(36, 'data');
  wav.setUint32(40, rate * 2, true);
  for (let i = 0; i < rate; i++) wav.setInt16(44 + i * 2, Math.round(amp * Math.sin((2 * Math.PI * 20 * i) / rate)), true);
  const hum = new Audio(URL.createObjectURL(new Blob([wav], { type: 'audio/wav' })));
  hum.id = 'keep-audible';
  hum.loop = true;
  document.body.append(hum);
  return {
    element: hum,
    on: () => hum.paused && hum.play().catch(() => {}),
    off: () => hum.pause(),
  };
}
const hum = keepAudible();
globalThis.yapObs.hum = hum.element;

/** Mute everyone but `only` (when set): their <audio> element, not a gain. */
function applyOnly() {
  if (!only) return;
  for (const sid of voice.peers.keys()) {
    const name = state.users.get(state.sessions.get(sid)?.uid)?.name?.toLowerCase();
    voice.setProximity(sid, name === only ? 1 : 0);
  }
}

/** Not listening: its account isn't in the channel (and wasn't for the last 5 minutes). */
function pause() {
  state.paused = true;
  hum.off();
  const name = state.users.get(state.uid)?.name ?? 'your account';
  note(`Paused: waiting for ${name} to join ${state.channels.get(channelId)?.name ?? 'the voice channel'}.`);
}

/** Someone with a voice to play: not another OBS link (one per person on a stream: they'd only connect to each other for silence). */
const talks = (session) => session.client !== 'obs';

/** Its account is in the channel (not just another OBS link of theirs). */
const accountIn = () => [...state.sessions.values()].some((s) => s.uid === state.uid && s.client !== 'obs' && s.voice?.channelId === channelId);

function render() {
  const stage = $('stage');
  stage.classList.toggle('hidden', !overlay);
  if (!overlay) return;
  if (!state.inCall) {
    stage.innerHTML = ''; // paused: nobody is being heard
    return;
  }
  const people = [...state.sessions.values()].filter((s) => s.voice?.channelId === channelId && s.client !== 'obs');
  const shown = only ? people.filter((s) => state.users.get(s.uid)?.name?.toLowerCase() === only) : people;
  stage.innerHTML = shown.map((s) => {
    const u = state.users.get(s.uid);
    const face = u?.avatar ? `<img src="/media/${esc(u.avatar)}" alt="">` : u?.look ? esc(u.look) : esc(initial(u?.name ?? '?'));
    return `<div class="who ${state.speaking.has(s.sid) ? 'speaking' : 'quiet'}"><div class="face" style="--c:${esc(u?.color ?? '#555')}">${face}</div><div class="name">${esc(u?.name ?? '')}</div></div>`;
  }).join('');
}

/** STUN only, for when /api/ice says no: an OBS link made before the owner reset the main link still has the old key (worker/ice.ts has the same two). */
const FALLBACK_ICE = [{ urls: ['stun:stun.cloudflare.com:3478', 'stun:stun.l.google.com:19302'] }];

async function join() {
  if (state.joining || state.inCall) return;
  state.joining = true;
  try {
    const ice = await fetch('/api/ice', { headers: { 'X-Yap-Key': key } })
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((j) => j.iceServers, () => FALLBACK_ICE);
    await voice.prepare(ice);
    conn.send({ t: 'voice.join', channelId, muted: true, deafened: false });
  } finally {
    state.joining = false;
  }
}

function handle(msg) {
  switch (msg.t) {
    case 'ready':
      state.sid = msg.sid;
      state.uid = msg.uid;
      voice.mySid = msg.sid;
      state.users = new Map(msg.users.map((u) => [u.id, u]));
      state.sessions = new Map(msg.sessions.map((s) => [s.sid, s]));
      state.channels = new Map(msg.channels.map((c) => [c.id, c]));
      note('');
      // (Even when its account isn't there: it may have left a moment ago, and the server says.)
      join().catch((err) => note(`Couldn't join voice: ${err.message}`));
      return render();
    case 'user':
      state.users.set(msg.user.id, msg.user);
      return render();
    case 'session.join':
      state.sessions.set(msg.session.sid, msg.session);
      return render();
    case 'session.leave':
      state.sessions.delete(msg.sid);
      voice.drop(msg.sid);
      return render();
    case 'voice': {
      const s = state.sessions.get(msg.sid);
      const prev = s?.voice?.channelId ?? null;
      if (s) s.voice = msg.channelId ? { channelId: msg.channelId, muted: msg.muted, deafened: msg.deafened } : null;
      if (msg.sid === state.sid) {
        if (msg.channelId === channelId && !state.inCall) {
          state.inCall = true;
          state.paused = false;
          hum.on(); // (before anyone's voice arrives, so OBS is already capturing)
          voice.enter([...state.sessions.values()].filter((x) => x.sid !== state.sid && x.voice?.channelId === channelId && talks(x)).map((x) => x.sid));
          note('');
        } else if (!msg.channelId && state.inCall) {
          // Its account didn't come back in time: the server took it out.
          state.inCall = false;
          voice.hangupAll();
          pause();
        }
      } else if (prev === channelId && msg.channelId !== channelId) {
        voice.drop(msg.sid);
      }
      // Its account is back: listen again, at once.
      if (!state.inCall && state.paused && msg.channelId === channelId && accountIn()) join().catch((err) => note(`Couldn't join voice: ${err.message}`));
      return render();
    }
    case 'signal': {
      const from = state.sessions.get(msg.from);
      if (state.inCall && from?.voice?.channelId === channelId && talks(from)) voice.handleSignal(msg.from, msg.data);
      return;
    }
    case 'error':
      if (msg.code === 'bad_token') note('This OBS link was turned off. Make a new one in Yap › Settings › Voice › Advanced › Stream to OBS.');
      else if (msg.code === 'no_channel') note('That voice channel is gone. Make a new OBS link in Yap.');
      else if (msg.code === 'not_in_voice') pause(); // (the server sends this before its account's join, which rejoins: below)
      return;
  }
}

voice.addEventListener('speaking', ({ detail: { sid, speaking } }) => {
  if (speaking) state.speaking.add(sid);
  else state.speaking.delete(sid);
  render();
});
voice.addEventListener('peer', () => applyOnly());
setInterval(applyOnly, 1000);

conn.addEventListener('open', () => {
  state.inCall = false;
  state.paused = false;
  voice.hangupAll();
  conn.send({ t: 'hello', v: 1, client: 'obs', token });
});
conn.addEventListener('message', (e) => handle(e.detail));
conn.addEventListener('close', () => {
  state.inCall = false;
  voice.hangupAll();
});

if (!key || !token || !channelId) {
  note('This link is missing its key, token or channel. Copy it again from Yap › Settings › Voice › Advanced › Stream to OBS.');
} else if (globalThis.obsstudio) {
  conn.connect(); // OBS plays audio without a click
} else {
  // A normal browser won't play sound until you click once.
  $('start').hidden = false;
  $('start').onclick = () => {
    $('start').hidden = true;
    conn.connect();
  };
}
