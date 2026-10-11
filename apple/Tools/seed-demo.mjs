// Seeds a LOCAL dev server with a small, friendly conversation for passive
// simulator screenshots (apple/README.md › Screenshots). Never point this at
// the live server.
//
//   YAP_URL=http://127.0.0.1:8789 YAP_KEY=testkey YAP_ADMIN_KEY=testadmin \
//     node apple/Tools/seed-demo.mjs [--stay <seconds>] [--voice [--screen] [--obs]]
//
// Accounts (password correct-horse-582039): Sam, Cleo, Ada, Ben. With --stay, Cleo and Ada
// stay online and Cleo keeps "typing" in general; with --voice too, Cleo,
// Ada and Ben sit in Lounge (no audio: they never connect, for the call
// sheet's screenshots with YAP_SCREEN=voicepanel:lounge); with --screen, Ada
// also says she's sharing her screen (a LIVE badge; no video); with --obs, Ada
// makes an OBS link, it joins Lounge while she's there, and she leaves: her
// OBS link keeps listening (the server keeps it up to 5 minutes), so Lounge
// shows "Ada's OBS is still listening". Also: a picture (the app
// icon, as a "draft"), an edited message, two of Sam's invite links, and
// encrypted notes in Sam's Personal space (passcode "purple tiger river",
// sent from "another device", so unread), and a reply.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createVault, openVault, encryptNote, encryptBlob } from '../../web/js/vault.js';

const BASE = process.env.YAP_URL ?? 'http://127.0.0.1:8789';
if (!/^http:\/\/(127\.0\.0\.1|localhost)/.test(BASE)) throw new Error('seed-demo only talks to a local dev server');
const KEY = process.env.YAP_KEY ?? 'testkey';
const ADMIN = process.env.YAP_ADMIN_KEY ?? 'testadmin';
const PASSWORD = 'correct-horse-582039';
const STAY = Number(process.argv[process.argv.indexOf('--stay') + 1]) || 0;
const VOICE = process.argv.includes('--voice') && STAY > 0;
const SCREEN = VOICE && process.argv.includes('--screen');
const OBS = VOICE && process.argv.includes('--obs');
const here = path.dirname(fileURLToPath(import.meta.url));
const admin = (p, init = {}) => fetch(`${BASE}${p}`, { ...init, headers: { ...init.headers, Authorization: `Bearer ${ADMIN}` } });

class Client {
  constructor(name, color) {
    this.name = name;
    this.color = color;
    this.inbox = [];
    this.waiters = [];
  }
  async open() {
    this.ws = new WebSocket(`${BASE.replace(/^http/, 'ws')}/ws`, ['yap', `key.${KEY}`]);
    this.ws.addEventListener('message', (e) => {
      if (e.data === 'pong') return;
      const msg = JSON.parse(e.data);
      const i = this.waiters.findIndex((w) => w.pred(msg));
      if (i >= 0) this.waiters.splice(i, 1)[0].resolve(msg);
      else this.inbox.push(msg);
    });
    await new Promise((resolve, reject) => {
      this.ws.addEventListener('open', resolve, { once: true });
      this.ws.addEventListener('error', reject, { once: true });
    });
  }
  send(msg) { this.ws.send(JSON.stringify(msg)); }
  next(pred, ms = 5000) {
    const i = this.inbox.findIndex(pred);
    if (i >= 0) return Promise.resolve(this.inbox.splice(i, 1)[0]);
    return new Promise((resolve, reject) => {
      const w = { pred, resolve };
      this.waiters.push(w);
      setTimeout(() => {
        const j = this.waiters.indexOf(w);
        if (j >= 0) { this.waiters.splice(j, 1); reject(new Error(`${this.name}: timeout`)); }
      }, ms);
    });
  }
  /** Sign in, or sign up + get let in by the admin. */
  async enter() {
    await this.open();
    this.send({ t: 'hello', v: 1, client: 'web', login: { name: this.name, pin: PASSWORD } });
    let first = await this.next((m) => m.t === 'ready' || m.t === 'pending' || (m.t === 'error' && m.re === 'hello'));
    if (first.t === 'error') {
      this.send({ t: 'hello', v: 1, client: 'web', signup: { name: this.name, pin: PASSWORD, color: this.color } });
      first = await this.next((m) => m.t === 'ready' || m.t === 'pending' || (m.t === 'error' && m.re === 'hello'));
      if (first.t === 'error') throw new Error(`${this.name}: ${first.message}`);
    }
    if (first.t === 'pending') {
      await admin(`/api/admin/role?uid=${first.uid}&role=member`, { method: 'POST' });
      first = await this.next((m) => m.t === 'ready');
    }
    this.ready = first;
    return this;
  }
  async say(body, extra = {}) {
    const nonce = Math.random().toString(36).slice(2);
    this.send({ t: 'send', channelId: 'general', body, nonce, ...extra });
    const echo = await this.next((m) => m.t === 'message' && m.nonce === nonce);
    await new Promise((r) => setTimeout(r, 250));
    return echo.message;
  }
}

const sam = await new Client('Sam', '#5b6cff').enter();
const cleo = await new Client('Cleo', '#30a46c').enter();
const ada = await new Client('Ada', '#d6409f').enter();
const ben = await new Client('Ben', '#f76b15').enter();

// A custom emoji (the app icon, small) if it isn't there yet.
let yap = sam.ready.emojis.find((e) => e.name === 'yap');
if (!yap) {
  const png = fs.readFileSync(path.join(here, '../Yap/Assets.xcassets/AppIcon.appiconset/mac-32@2x.png'));
  const res = await admin('/api/admin/emoji?name=yap', { method: 'POST', body: png, headers: { 'Content-Type': 'image/png' } });
  yap = (await res.json()).emoji;
}
const token = `<:yap:${yap.id}>`;

const m1 = await ada.say("who's around tonight? thinking **Overcooked** around 9 🍳");
await sam.say('running ten minutes late, save me a kitchen');
const m3 = await ben.say('in! new map pack is out: https://example.com/overcooked/maps');
await cleo.say(`same ${token}`);
const m5 = await ada.say('@Sam you coming? bring the _good_ snacks');
await ben.say('`/join lounge` when you are ready ~~no excuses~~');
await cleo.say('', {
  gif: {
    provider: 'klipy', slug: 'lets-go-demo', title: "Let's go", width: 480, height: 270,
    mp4: 'https://static.klipy.com/demo/lets-go.mp4', still: 'https://static.klipy.com/demo/lets-go.jpg',
  },
});
await ada.say('> bring the good snacks\nI meant it 😌');
// A reply (quotes Ada's question).
await ben.say('count me in, I call the sink station', { replyTo: m1.id });

// A picture: upload with this connection's sid + upload token, then send it.
const iconPng = fs.readFileSync(path.join(here, '../Yap/Assets.xcassets/AppIcon.appiconset/ios-1024.png'));
const upload = await fetch(`${BASE}/api/image?${new URLSearchParams({ sid: cleo.ready.sid, ut: cleo.ready.uploadToken ?? '' })}`, {
  method: 'POST', body: iconPng, headers: { 'Content-Type': 'image/png', 'X-Yap-Key': KEY },
});
const picture = await upload.json();
if (!picture.ok) throw new Error(`picture upload: ${picture.message}`);
await cleo.say('new icon draft 👀', { image: { id: picture.id, width: 1024, height: 1024 } });

// An edited message.
const mine = await sam.say('love it, sihp it');
sam.send({ t: 'edit', id: mine.id, body: 'love it, ship it 🚀' });
await sam.next((m) => m.t === 'edited' && m.id === mine.id);

// Two pinned messages (the plan and the map pack link).
for (const id of [m1.id, m3.id]) {
  sam.send({ t: 'pin', id, on: true });
  await sam.next((m) => m.t === 'pinned' && m.id === id);
}

// Sam's Personal space, encrypted with the passcode "purple tiger river"
// (vault.js, like the web): notes and a sealed picture, from "the web".
const personal = sam.ready.channels.find((c) => c.kind === 'personal');
if (personal) {
  const PASS = 'purple tiger river';
  let key;
  if (sam.ready.vault) {
    key = await openVault(sam.ready.vault, PASS).catch(() => null);
  } else {
    const made = await createVault(PASS, 100_000);
    sam.send({ t: 'vault.set', wrap: made.wrap });
    await sam.next((m) => m.t === 'vault');
    key = made.key;
  }
  if (key) {
    const seal = (payload) => encryptNote(key, payload);
    await sam.say(await seal({ text: 'recipe for saturday https://example.com/recipes/shakshuka' }), { channelId: personal.id });
    await sam.say(await seal({ text: 'spare bike lock combo is in the blue notebook, page 3' }), { channelId: personal.id });
    const sealedPng = await encryptBlob(key, new Uint8Array(iconPng));
    const blobRes = await fetch(`${BASE}/api/blob?${new URLSearchParams({ sid: sam.ready.sid, ut: sam.ready.uploadToken ?? '' })}`, {
      method: 'POST', body: sealedPng, headers: { 'Content-Type': 'application/octet-stream', 'X-Yap-Key': KEY },
    });
    const blob = await blobRes.json();
    if (blob.ok) await sam.say(await seal({ text: 'icon v2 for the app', image: { id: blob.id, width: 1024, height: 1024, mime: 'image/png' } }), { channelId: personal.id });
  }
}

// Invite links (one turned off) for Settings › Invite.
for (const label of ['for Dee', 'for Priya']) {
  sam.send({ t: 'invite.create', label });
  await sam.next((m) => m.t === 'invite.created');
}
sam.send({ t: 'invite.list' });
const { invites } = await sam.next((m) => m.t === 'invites');
const dee = invites.find((i) => i.label === 'for Dee' && !i.revoked);
if (dee) {
  sam.send({ t: 'invite.revoke', id: dee.id });
  await sam.next((m) => m.t === 'invites');
}

for (const [who, msg, emoji] of [[ben, m1, '🎉'], [cleo, m1, '🎉'], [sam, m1, '❤️'], [ada, m3, token], [sam, m5, '👍']]) {
  who.send({ t: 'react', id: msg.id, emoji, on: true });
  await new Promise((r) => setTimeout(r, 150));
}
// The v0.36 extras: a spoiler, a YouTube link, a poll with votes, dice,
// soundboard clips, a chime, an away message, a hang, a doodle reaction.
const pause = (ms) => new Promise((r) => setTimeout(r, ms));
/** A mono 16-bit 24 kHz WAV: notes (Hz) one after another. */
function wav(notes, each = 0.18, amp = 0.6) {
  const rate = 24000;
  const n = Math.round(rate * each * notes.length);
  const buf = Buffer.alloc(44 + n * 2);
  buf.write('RIFF', 0); buf.writeUInt32LE(36 + n * 2, 4); buf.write('WAVEfmt ', 8);
  buf.writeUInt32LE(16, 16); buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22);
  buf.writeUInt32LE(rate, 24); buf.writeUInt32LE(rate * 2, 28); buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34);
  buf.write('data', 36); buf.writeUInt32LE(n * 2, 40);
  for (let i = 0; i < n; i++) {
    const t = i / rate;
    const k = Math.floor(t / each);
    const local = t - k * each;
    const env = Math.min(1, local / 0.01) * Math.exp(-local * 6);
    const v = Math.sign(Math.sin(2 * Math.PI * notes[k] * t)) * 0.5 + Math.sin(2 * Math.PI * notes[k] * t) * 0.5;
    buf.writeInt16LE(Math.round(v * env * amp * 32767), 44 + i * 2);
  }
  return buf;
}
const post = (who, p, body, params = {}, type = 'application/octet-stream') =>
  fetch(`${BASE}${p}?${new URLSearchParams({ sid: who.ready.sid, ut: who.ready.uploadToken ?? '', ...params })}`,
        { method: 'POST', body, headers: { 'Content-Type': type, 'X-Yap-Key': KEY } }).then((r) => r.json());

await ben.say('the ending of that movie: ||the dog lives|| 🐶');
await ada.say('the song from last night https://www.youtube.com/watch?v=dQw4w9WgXcQ');
await pause(800);
const poll = await cleo.say('Snacks for tonight?', { poll: { options: ['chips', 'fruit', 'both obviously'] } });
for (const [who, option] of [[ada, 2], [ben, 2], [cleo, 0]]) {
  who.send({ t: 'poll.vote', id: poll.id, option });
  await pause(150);
}
await pause(1100);
await ben.say('/roll 2d6');
await pause(1100);
await ada.say('/flip');
if (!(sam.ready.sounds ?? []).some((s) => s.name === 'Airhorn')) {
  await post(cleo, '/api/sound', wav([466, 466, 622]), { kind: 'board', name: 'Airhorn', emoji: '📯' }, 'audio/wav');
  await post(ada, '/api/sound', wav([196, 247, 1568], 0.22), { kind: 'board', name: 'Ba dum tss', emoji: '🥁' }, 'audio/wav');
  await post(ben, '/api/sound', wav([523, 659, 784, 1047], 0.12), { kind: 'board', name: 'Victory', emoji: '🏆' }, 'audio/wav');
}
ada.send({ t: 'profile', chime: 'tada' });
cleo.send({ t: 'profile', away: { text: 'at work til 6', until: null } });
await pause(300);
if (!(sam.ready.hangs ?? []).some((h) => h.title === 'Game night')) {
  const at = Math.ceil((Date.now() + 3 * 3_600_000) / 1_800_000) * 1_800_000;
  sam.send({ t: 'hang.set', at, title: 'Game night', channelId: 'lounge' });
  await sam.next((m) => m.t === 'hang' || (m.t === 'error' && m.re === 'hang.set')).catch(() => null);
}
const doodlePng = fs.readFileSync(path.join(here, '../Yap/Assets.xcassets/AppIcon.appiconset/mac-32@2x.png'));
const doodle = await post(ben, '/api/doodle', doodlePng, {}, 'image/png');
if (doodle.ok) {
  ben.send({ t: 'react', id: m1.id, emoji: `<d:${doodle.id}>`, on: true });
  await pause(200);
}
console.log('seeded: Sam, Cleo, Ada, Ben (password correct-horse-582039) + messages in general');

for (const c of VOICE ? [sam] : [sam, ben]) c.ws.close();
if (VOICE) {
  for (const c of [cleo, ada, ben]) c.send({ t: 'voice.join', channelId: 'lounge', muted: c === ben, deafened: false, resume: true });
  console.log('Cleo, Ada and Ben are in Lounge');
  if (SCREEN) {
    await pause(300);
    ada.send({ t: 'voice.state', muted: false, deafened: false, screen: true });
    console.log('Ada is sharing her screen (LIVE)');
  }
}
let obs = null;
if (OBS) {
  // Ada's OBS link joins while she's in Lounge (the server only lets it in
  // then), then Ada leaves and it keeps listening.
  await pause(300);
  ada.send({ t: 'obs.create' });
  const { token: obsToken } = await ada.next((m) => m.t === 'obs.created');
  obs = new Client('Ada (OBS)', '#d6409f');
  await obs.open();
  obs.send({ t: 'hello', v: 1, client: 'obs', token: obsToken });
  obs.ready = await obs.next((m) => m.t === 'ready');
  obs.send({ t: 'voice.join', channelId: 'lounge', muted: true, deafened: false });
  await obs.next((m) => m.t === 'voice' && m.sid === obs.ready.sid && m.channelId === 'lounge');
  ada.send({ t: 'voice.leave' });
  await pause(300);
  console.log("Ada left Lounge; her OBS link is still listening");
}
if (STAY > 0) {
  console.log(`staying online for ${STAY}s (Cleo typing)`);
  const typing = setInterval(() => cleo.send({ t: 'typing', channelId: 'general' }), 3000);
  const ping = setInterval(() => { for (const c of [...(VOICE ? [cleo, ada, ben] : [cleo, ada]), ...(obs ? [obs] : [])]) c.ws.send('ping'); }, 15000);
  cleo.send({ t: 'typing', channelId: 'general' });
  await new Promise((r) => setTimeout(r, STAY * 1000));
  clearInterval(typing);
  clearInterval(ping);
}
cleo.ws.close();
ada.ws.close();
if (VOICE) ben.ws.close();
obs?.ws.close();
setTimeout(() => process.exit(0), 300);
