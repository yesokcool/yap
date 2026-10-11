/*
 * OBS voice output: a Browser Source that joins a voice channel listen-only
 * and plays it, with an optional overlay of who's talking. The link comes
 * from Settings › Voice › Advanced › Stream to OBS:
 *   /obs#key=<invite key>&listen=<listen-only token>&channel=<voice channel id>
 *        [&only=<name>]   play just one person (one source per person)
 *        [&overlay=0]     audio only, nothing drawn
 * The token can only listen (the server enforces it); the apps leave it out
 * of the channel list and its chimes (2026-10-11).
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

const state = { sid: null, users: new Map(), sessions: new Map(), inCall: false, speaking: new Set() };
const voice = new Voice({
  signal: (to, data) => conn.send({ t: 'signal', to, data }),
  uidOf: (sid) => state.sessions.get(sid)?.uid,
});
// Like the app: the key in the subprotocol, or in the address when it has characters a subprotocol can't carry.
const conn = new Connection(socketUrl(key), keyProtocols(key));
globalThis.yapObs = { state, voice };

function note(text) {
  $('note').hidden = !text;
  $('note').textContent = text ?? '';
}

/** Mute everyone but `only` (when set). */
function applyOnly() {
  if (!only) return;
  for (const sid of voice.peers.keys()) {
    const name = state.users.get(state.sessions.get(sid)?.uid)?.name?.toLowerCase();
    voice.setProximity(sid, name === only ? 1 : 0);
  }
}

function render() {
  const stage = $('stage');
  stage.classList.toggle('hidden', !overlay);
  if (!overlay) return;
  const people = [...state.sessions.values()].filter((s) => s.voice?.channelId === channelId && s.client !== 'obs');
  const shown = only ? people.filter((s) => state.users.get(s.uid)?.name?.toLowerCase() === only) : people;
  stage.innerHTML = shown.map((s) => {
    const u = state.users.get(s.uid);
    const face = u?.avatar ? `<img src="/media/${esc(u.avatar)}" alt="">` : u?.look ? esc(u.look) : esc(initial(u?.name ?? '?'));
    return `<div class="who ${state.speaking.has(s.sid) ? 'speaking' : 'quiet'}"><div class="face" style="--c:${esc(u?.color ?? '#555')}">${face}</div><div class="name">${esc(u?.name ?? '')}</div></div>`;
  }).join('');
}

async function join() {
  const ice = await fetch('/api/ice', { headers: { 'X-Yap-Key': key } }).then((r) => r.json()).then((j) => j.iceServers).catch(() => undefined);
  await voice.prepare(ice, { listenOnly: true });
  conn.send({ t: 'voice.join', channelId, muted: true, deafened: false });
}

function handle(msg) {
  switch (msg.t) {
    case 'ready':
      state.sid = msg.sid;
      voice.mySid = msg.sid;
      state.users = new Map(msg.users.map((u) => [u.id, u]));
      state.sessions = new Map(msg.sessions.map((s) => [s.sid, s]));
      note(state.sessions.size ? '' : '');
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
          voice.enter([...state.sessions.values()].filter((x) => x.sid !== state.sid && x.voice?.channelId === channelId).map((x) => x.sid));
          note('');
        }
      } else if (prev === channelId && msg.channelId !== channelId) {
        voice.drop(msg.sid);
      }
      return render();
    }
    case 'signal': {
      const from = state.sessions.get(msg.from);
      if (state.inCall && from?.voice?.channelId === channelId) voice.handleSignal(msg.from, msg.data);
      return;
    }
    case 'error':
      if (msg.code === 'bad_token') note('This OBS link was turned off. Make a new one in Yap › Settings › Voice › Advanced › Stream to OBS.');
      else if (msg.code === 'no_channel') note('That voice channel is gone. Make a new OBS link in Yap.');
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
