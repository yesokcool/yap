// Interop: the web client (headless Chromium, fake mic) and the native client
// (`yap --headless`, WAV mic) in the same server — text, sends the server turns
// away (a burst of 12 slowed down and sent again in order; a refused reply),
// @mentions, reactions,
// pings and a KLIPY GIF both ways, edits both ways, replies both ways, Draw,
// Space (players, emotes, ball, tag, balloons, fire/water, laps, the Gallery), the
// encrypted Personal space (passcode set on the native side, earlier notes
// sealed, notes and a sealed picture read on the other side, both ways; a
// deleted picture's blob erased), a big picture (shrunk and uploaded with the
// per-connection upload token), Places links and personal invites, accounts
// (password + device token; an old PIN account signing in, asked once for a
// password, choosing one), the waiting room (only the owner and mods hear
// about newcomers), then voice in `lounge` with audio flowing in both
// directions, in both join orders, and an OBS link listening to native
// without showing up (no chimes) while its account is in the call, and
// greyed out while it isn't; last, the owner resets the main link (every
// connected app saves the new key; a device with the old key and a device
// token still gets in; a sign-in with the old key is turned away).
//
//   YAP_URL=http://localhost:8788 YAP_KEY=testkey YAP_ADMIN_KEY=testadmin \
//     YAP_BIN=desktop/target/release/yap node desktop/tests/interop.mjs
//
// Server (from the repo root; the test config with its stand-in board):
//   npx wrangler dev -c test/wrangler.test.jsonc -c test/fixtures/board/wrangler.toml --port 8788 \
//     --var YAP_KEY:testkey --var ADMIN_KEY:testadmin --var KLIPY_KEY:test --var PIN_PEPPER:testpepper \
//     --var TEST_HANGS:1 --var TEST_PINS:1 --persist-to <scratch>
// (TEST_PINS lets the admin API give an account an old 6-digit PIN, for the old-PIN check.)
// A local KLIPY mock (tests/klipy-mock.mjs) is started on YAP_KLIPY_PORT (8795).
//
// YAP_IMPAIR=wifi|cell|loss5|loss2 puts the voice calls behind an impairment
// proxy (RTP only; the audit's profiles: wifi = 15 ms + exp(10 ms) jitter, 2 %
// spikes of 60–200 ms, 1 % loss; cell = 40 ms + exp(20 ms), 3 % spikes of
// 100–300 ms, 2 % loss): the web routes the native app's candidates through a
// local UDP proxy and keeps its own to itself, so everything goes through it.
// The evidence then has the native side's jitter-buffer numbers.
//
// Screen sharing both ways, never the real screen: the web's getDisplayMedia
// is replaced by a moving canvas, and the native side shares its built-in
// test pattern (`--share-screen`); each side checks the other's pictures
// arrive (decoded, the right size, H.264).
//
// The web's mic: Chromium's fake device playing the test WAV. Headless
// Chromium on macOS 27 hangs forever opening any audio capture (seen on the
// maintainer's Mac in an agent session): YAP_WEB_MIC=synth gives every page
// the same signal from WebAudio instead. The screen-sharing tests always use
// a page of their own with that WebAudio mic (they're about the video).
//
// Needs a running dev server (see desktop/README.md) and Playwright's Chromium
// (the repo's devDependency). Exits non-zero if any check fails.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import crypto from 'node:crypto';
import zlib from 'node:zlib';
import { chromium } from 'playwright';
import dgram from 'node:dgram';

const here = path.dirname(fileURLToPath(import.meta.url));
const BASE = process.env.YAP_URL ?? 'http://localhost:8788';
const KEY = process.env.YAP_KEY ?? '';
const BITRATE = Number(process.env.YAP_BITRATE ?? 128000);
const BIN = process.env.YAP_BIN ?? [path.join(here, '../target/release/yap'), path.join(here, '../target/debug/yap')].find((p) => fs.existsSync(p));
const OUT = process.env.YAP_OUT ?? fs.mkdtempSync(path.join(os.tmpdir(), 'yap-interop-'));
fs.mkdirSync(OUT, { recursive: true });
const RUN = Date.now().toString(36).slice(-5);

// 12 s of chords + noise (as in test/e2e.test.mjs); two variants so each side is distinguishable.
function writeTestWav(file, variant) {
  const sr = 48000, n = sr * 12, data = Buffer.alloc(n * 2);
  let seed = variant === 'a' ? 1 : 7;
  const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) * 2 - 1;
  for (let i = 0; i < n; i++) {
    const t = i / sr;
    const chord = variant === 'a'
      ? (Math.floor(t) % 2 ? [196, 246.94, 293.66] : [220, 277.18, 329.63])
      : (Math.floor(t) % 2 ? [261.63, 329.63, 392] : [174.61, 220, 261.63]);
    let v = chord.reduce((s, f) => s + Math.sin(2 * Math.PI * f * t) * 0.18, 0) + rnd() * 0.12;
    v *= 0.6 + 0.4 * Math.sin(2 * Math.PI * 0.5 * t);
    data.writeInt16LE(Math.round(Math.max(-1, Math.min(1, v)) * 30000), i * 2);
  }
  const h = Buffer.alloc(44);
  h.write('RIFF', 0); h.writeUInt32LE(36 + data.length, 4); h.write('WAVE', 8);
  h.write('fmt ', 12); h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22);
  h.writeUInt32LE(sr, 24); h.writeUInt32LE(sr * 2, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34);
  h.write('data', 36); h.writeUInt32LE(data.length, 40);
  fs.writeFileSync(file, Buffer.concat([h, data]));
}
const webWav = path.join(OUT, 'web-mic.wav');
const nativeWav = path.join(OUT, 'native-mic.wav');
writeTestWav(webWav, 'a');
writeTestWav(nativeWav, 'b');

const browser = await chromium.launch({
  args: [
    '--use-fake-device-for-media-stream',
    '--use-fake-ui-for-media-stream',
    `--use-file-for-fake-audio-capture=${webWav}`,
    '--autoplay-policy=no-user-gesture-required',
  ],
});

const IMPAIR = process.env.YAP_IMPAIR || null;
const PROFILES = {
  none: { base: 0, jit: 0, spikeP: 0, spike: [0, 0], loss: 0 },
  loss2: { base: 0, jit: 0, spikeP: 0, spike: [0, 0], loss: 0.02 },
  loss5: { base: 0, jit: 0, spikeP: 0, spike: [0, 0], loss: 0.05 },
  wifi: { base: 15, jit: 10, spikeP: 0.02, spike: [60, 200], loss: 0.01 },
  cell: { base: 40, jit: 20, spikeP: 0.03, spike: [100, 300], loss: 0.02 },
};
if (IMPAIR && !PROFILES[IMPAIR]) throw new Error(`YAP_IMPAIR: one of ${Object.keys(PROFILES).join(', ')}`);
// What crossed the proxy, each way: RTP payload types (the header isn't encrypted: RED is 63, Opus 111) and payload bytes (with SRTP's tag).
const impairStats = { rtp: 0, dropped: 0, toWeb: { pt: {}, bytes: 0 }, toNative: { pt: {}, bytes: 0 } };
const resetWire = () => { for (const d of [impairStats.toWeb, impairStats.toNative]) { d.pt = {}; d.bytes = 0; } };
const proxies = [];
// YAP_IMPAIR_SEED=<n>: the same drops and delays every run (so RED on and off meet the same network).
let seed = Number(process.env.YAP_IMPAIR_SEED ?? 0);
const rand = seed > 0 ? () => ((seed = (seed * 16807) % 2147483647) / 2147483647) : Math.random;
/** A UDP proxy in front of one of the native app's candidates; RTP through it gets the profile. */
function mkProxy(host, port) {
  const prof = PROFILES[IMPAIR];
  const sock = dgram.createSocket('udp4');
  proxies.push(sock);
  // Who last sent a STUN check through us, and the socket media (DTLS, RTP, RTCP) last came
  // from: media goes back to the selected pair's socket, so another pair's checks can't steal it.
  // A STUN answer goes back to whoever asked (by transaction id): the web checks one candidate
  // from several sockets at once.
  let client = null;
  let media = null;
  const asked = new Map();
  sock.on('message', (msg, r) => {
    const fromTarget = r.address === host && r.port === port;
    const stun = msg[0] < 4 && msg.length >= 20;
    const txid = stun ? msg.toString('hex', 8, 20) : null;
    if (!fromTarget) {
      client = { address: r.address, port: r.port };
      if (!stun) media = client;
      else if (!(msg.readUInt16BE(0) & 0x0100)) {
        asked.set(txid, client);
        if (asked.size > 500) asked.delete(asked.keys().next().value);
      }
    }
    const answer = stun && msg.readUInt16BE(0) & 0x0100 ? asked.get(txid) : null;
    const to = fromTarget ? (answer ?? (stun ? client : media ?? client)) : { address: host, port };
    if (!to) return;
    // RTP, not RTCP (RFC 5761: RTCP's second byte is 192–223), STUN or DTLS. Any payload type: RED is 63.
    const isRtp = msg.length > 12 && (msg[0] & 0xc0) === 0x80 && (msg[1] < 192 || msg[1] > 223);
    if (!isRtp) return sock.send(msg, to.port, to.address);
    impairStats.rtp++;
    const dir = fromTarget ? impairStats.toWeb : impairStats.toNative;
    const pt = msg[1] & 0x7f;
    if (pt === 63 || pt === 111) {
      dir.pt[pt] = (dir.pt[pt] ?? 0) + 1;
      const csrc = 4 * (msg[0] & 0x0f);
      dir.bytes += msg.length - 12 - csrc - (msg[0] & 0x10 ? 4 + 4 * msg.readUInt16BE(14 + csrc) : 0); // the payload (and SRTP's tag)
    }
    if (rand() < prof.loss) return impairStats.dropped++;
    let d = prof.base + (prof.jit ? -prof.jit * Math.log(1 - rand()) : 0);
    if (rand() < prof.spikeP) d += prof.spike[0] + rand() * (prof.spike[1] - prof.spike[0]);
    if (d < 1) return sock.send(msg, to.port, to.address);
    setTimeout(() => sock.send(msg, to.port, to.address), d);
  });
  return new Promise((res) => sock.bind(0, '0.0.0.0', () => res(sock.address().port)));
}

// The made-up test password every suite uses (PROTOCOL.md › Identity: 8+ characters, not guessable).
const TEST_PASSWORD = 'correct-horse-582039';
const ADMIN = process.env.YAP_ADMIN_KEY ?? 'testadmin';

/**
 * Runs in every test page before the app: a synthetic screen for
 * getDisplayMedia (a 1280 × 720 canvas: colour bars, a moving square, a frame
 * counter; and a stereo test tone as its sound), and a fallback mic: the test
 * WAV's signal (variant a) from WebAudio when the fake device doesn't answer.
 */
function webMedia({ synthMic }) {
  const md = navigator.mediaDevices;
  if (!md) return;
  let ctx = null;
  const audioCtx = () => (ctx ??= new AudioContext({ sampleRate: 48000 }));
  const synthAudio = () => {
    const c = audioCtx();
    const sr = 48000, n = sr * 12, buf = c.createBuffer(1, n, sr), d = buf.getChannelData(0);
    let seed = 1;
    const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) * 2 - 1;
    for (let i = 0; i < n; i++) {
      const t = i / sr;
      const chord = Math.floor(t) % 2 ? [196, 246.94, 293.66] : [220, 277.18, 329.63];
      let v = chord.reduce((s, f) => s + Math.sin(2 * Math.PI * f * t) * 0.18, 0) + rnd() * 0.12;
      v *= 0.6 + 0.4 * Math.sin(2 * Math.PI * 0.5 * t);
      d[i] = Math.max(-1, Math.min(1, v)) * 0.915;
    }
    const src = c.createBufferSource();
    src.buffer = buf;
    src.loop = true;
    const dest = c.createMediaStreamDestination();
    src.connect(dest);
    src.start();
    return dest.stream;
  };
  const real = md.getUserMedia.bind(md);
  md.getUserMedia = async (constraints) => (synthMic && constraints?.audio && !constraints.video ? synthAudio() : real(constraints));
  md.getDisplayMedia = async () => {
    const canvas = document.createElement('canvas');
    canvas.width = 1280;
    canvas.height = 720;
    const g = canvas.getContext('2d');
    let f = 0;
    const draw = () => {
      f++;
      const bars = ['#fff', '#ff0', '#0ff', '#0f0', '#f0f', '#f00', '#00f', '#282828'];
      bars.forEach((b, i) => { g.fillStyle = b; g.fillRect(i * 160, 0, 160, 360); });
      g.fillStyle = '#123';
      g.fillRect(0, 360, 1280, 360);
      g.fillStyle = '#fff';
      g.fillRect((f * 8) % 1160, 405, 120, 120);
      g.font = '64px sans-serif';
      g.fillText(`frame ${f}`, 40, 680);
    };
    draw();
    setInterval(draw, 33);
    const stream = canvas.captureStream(30);
    // The share's sound: a stereo test tone (440 Hz left, 660 Hz right).
    const c = audioCtx();
    const merge = c.createChannelMerger(2);
    for (const [hz, ch] of [[440, 0], [660, 1]]) {
      const o = c.createOscillator();
      o.frequency.value = hz;
      const gain = c.createGain();
      gain.gain.value = 0.2;
      o.connect(gain).connect(merge, 0, ch);
      o.start();
    }
    const dest = c.createMediaStreamDestination();
    merge.connect(dest);
    stream.addTrack(dest.stream.getAudioTracks()[0]);
    return stream;
  };
}

/** Get a web test user online like a person would: sign in or up; admin lets newcomers in. */
async function settle(page, name) {
  let triedLogin = false;
  for (let i = 0; i < 8; i++) {
    const s = await page.waitForFunction(() => {
      if (document.querySelector('#welcome-dialog')?.open) return 'welcome';
      if (document.querySelector('#pin-dialog')?.open) return 'pin';
      const status = globalThis.yap?.state.status;
      return status === 'online' || status === 'waiting' ? status : false;
    }, null, { timeout: 15000 }).then((h) => h.jsonValue());
    if (s === 'welcome') {
      // Sign in first; if there's no such account yet, sign up.
      await page.click(`#welcome-form label:has(input[value=${triedLogin ? 'signup' : 'login'}])`);
      await page.fill('#welcome-form input[name=name]', name);
      await page.fill('#welcome-form input[name=pin]', TEST_PASSWORD);
      await page.click('#welcome-form button[type=submit]');
      triedLogin = true;
      await page.waitForFunction(() => !document.querySelector('#welcome-dialog')?.open || document.querySelector('#welcome-error:not([hidden])'), null, { timeout: 10000 });
    } else if (s === 'pin') {
      await page.fill('#pin-form input[name=pin]', TEST_PASSWORD);
      // (Older pages asked for it twice.)
      if (await page.locator('#pin-form input[name=again]').count()) await page.fill('#pin-form input[name=again]', TEST_PASSWORD);
      await page.click('#pin-form button[type=submit]');
      await page.waitForFunction(() => !document.querySelector('#pin-dialog')?.open);
    } else if (s === 'waiting') {
      const uid = await page.evaluate(() => yap.settings.uid);
      await admit(uid);
      await page.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
    } else if (await page.evaluate(() => !document.querySelector('dialog[open]'))) {
      await page.waitForTimeout(700); // a "choose a password" prompt may still be on its way
      if (await page.evaluate(() => !document.querySelector('dialog[open]'))) return;
    }
  }
  throw new Error(`${name} never got online`);
}

async function admit(uid, role = 'member') {
  await fetch(`${BASE}/api/admin/role?uid=${uid}&role=${role}`, { method: 'POST', headers: { Authorization: `Bearer ${ADMIN}`, 'X-Yap-Key': KEY } });
}

async function webUser(name, uid, { synthMic = process.env.YAP_WEB_MIC === 'synth', inBrowser = browser } = {}) {
  const context = await inBrowser.newContext({ viewport: { width: 1280, height: 800 }, permissions: ['microphone'] });
  if (IMPAIR) {
    await context.exposeFunction('__mkProxy', (host, port) => mkProxy(host, port));
    await context.addInitScript(() => {
      const add = RTCPeerConnection.prototype.addIceCandidate;
      RTCPeerConnection.prototype.addIceCandidate = async function (c, ...rest) {
        const line = c?.candidate ?? '';
        const p = line.split(' ');
        if (!line) return add.call(this, c, ...rest);
        if (p[2]?.toLowerCase() !== 'udp' || p[4].includes(':') || p[4].endsWith('.local')) return; // TCP, IPv6, mDNS: not through the proxy
        p[5] = String(await globalThis.__mkProxy(p[4], Number(p[5])));
        return add.call(this, { ...c, candidate: p.join(' ') }, ...rest);
      };
      // Never tell the native app our own candidates: it must find us through the proxy (peer-reflexive).
      const desc = Object.getOwnPropertyDescriptor(RTCPeerConnection.prototype, 'onicecandidate');
      Object.defineProperty(RTCPeerConnection.prototype, 'onicecandidate', {
        set(fn) { desc.set.call(this, fn && ((e) => { if (e.candidate) return; fn(e); })); },
        get() { return desc.get.call(this); },
      });
    });
  }
  await context.addInitScript(([name, uid, bitrate]) => {
    if (!localStorage.getItem('yap.settings')) {
      localStorage.setItem('yap.settings', JSON.stringify({ name, uid, color: '#5b6cff', bitrate, mode: 'open', sounds: false }));
    }
  }, [name, uid, BITRATE]);
  await context.addInitScript(webMedia, { synthMic });
  const page = await context.newPage();
  page.on('pageerror', (err) => console.log(`[web] page error:`, err.message));
  await page.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);
  await settle(page, name);
  return page;
}

/** Start `yap --headless`; resolves to { done, lines } where done resolves with the SUMMARY. */
/** A real account: sign in with the test password, or sign up (then the doorman lets it in). */
function native(name, _unused, extra, env = {}) {
  const args = ['--headless', '--server', BASE, '--name', name, '--password', TEST_PASSWORD, '--input-wav', nativeWav, '--bitrate', String(BITRATE), ...extra];
  if (KEY) args.push('--key', KEY);
  const child = spawn(BIN, args, { stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, ...env } });
  const lines = [];
  let buf = '';
  const onData = (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      lines.push(buf.slice(0, i));
      buf = buf.slice(i + 1);
    }
  };
  child.stdout.on('data', onData);
  child.stderr.on('data', onData);
  const done = new Promise((resolve) => child.on('exit', () => {
    if (buf) lines.push(buf);
    fs.writeFileSync(path.join(OUT, `native-${name}-${Date.now() % 100000}.log`), lines.join('\n') + '\n');
    const s = lines.find((l) => l.startsWith('SUMMARY '));
    resolve(s ? JSON.parse(s.slice(8)) : null);
  }));
  const waitFor = async (re, ms = 15000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) {
      if (lines.some((l) => re.test(l))) return;
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error(`native never logged ${re}`);
  };
  return { child, lines, done, waitFor };
}

const inbound = (page) => page.evaluate(async () => {
  const peer = [...yap.voice.peers.values()][0];
  if (!peer) return null;
  const report = await peer.pc.getStats();
  let rtp, out, codec, outCodec;
  report.forEach((s) => {
    if (s.type === 'inbound-rtp' && s.kind === 'audio') rtp = s;
    if (s.type === 'outbound-rtp' && s.kind === 'audio') out = s;
  });
  if (rtp) codec = report.get(rtp.codecId);
  if (out) outCodec = report.get(out.codecId);
  const ssrc = peer.pc.getReceivers()[0]?.getSynchronizationSources?.()[0];
  // The voice section's Opus fmtp, as each side described it (with RED the codec stats name red).
  const opusFmtp = (sdp) => {
    const voice = (sdp ?? '').split('\r\nm=')[1] ?? '';
    const pt = voice.match(/a=rtpmap:(\d+) opus\/48000/i)?.[1];
    return pt ? voice.match(new RegExp(`a=fmtp:${pt} ([^\\r\\n]*)`))?.[1] : null;
  };
  const order = (sdp) => {
    const voice = (sdp ?? '').split('\r\nm=')[1] ?? '';
    return voice.split('\r\n')[0].split(' ').slice(3).map((pt) => voice.match(new RegExp(`a=rtpmap:${pt} ([^/]+)/`))?.[1]?.toLowerCase()).filter(Boolean).slice(0, 2).join(',');
  };
  return {
    bytes: rtp?.bytesReceived ?? 0, packets: rtp?.packetsReceived ?? 0, lost: rtp?.packetsLost ?? 0,
    level: rtp?.audioLevel ?? 0, mime: codec?.mimeType, fmtp: codec?.sdpFmtpLine, outMime: outCodec?.mimeType,
    opusFmtp: opusFmtp(peer.pc.remoteDescription?.sdp), localOrder: order(peer.pc.localDescription?.sdp), remoteOrder: order(peer.pc.remoteDescription?.sdp),
    concealed: (rtp?.concealedSamples ?? 0) - (rtp?.silentConcealedSamples ?? 0), samples: rtp?.totalSamplesReceived ?? 0,
    sentBytes: out?.bytesSent ?? 0, sentPackets: out?.packetsSent ?? 0, outFmtp: outCodec?.sdpFmtpLine,
    rtpExtLevel: ssrc?.audioLevel ?? null, at: performance.now(),
  };
});
/** Audio the web patched over between two readings (% of what it received). */
const patchedPct = (a, b) => +((100 * (b.concealed - a.concealed)) / Math.max(1, b.samples - a.samples)).toFixed(2);

const results = [];
const evidence = {};
/** How the native app logs a key it saved (`KEY saved sha256:…`): never the key itself. */
const keyFp = (k) => `sha256:${crypto.createHash('sha256').update(k).digest('hex').slice(0, 8)}`;
async function test(name, fn) {
  // YAP_ONLY=<regex>: run just the matching tests (while iterating).
  if (process.env.YAP_ONLY && !new RegExp(process.env.YAP_ONLY).test(name)) return;
  try {
    await fn();
    results.push(['✓', name]);
  } catch (err) {
    results.push(['✗', name, err]);
  }
}

// The server has a waiting room: newcomers wait until the owner or a mod lets
// them in. A native "doorman" (the owner on a fresh server, else made a mod
// through the admin API) admits everyone for the length of the test.
// (Killed at the end; its --duration only has to outlast the whole suite, ~8 minutes now.)
const doorman = native(process.env.YAP_DOORMAN_NAME ?? 'Doorman', null, ['--approve-all', '--duration', '1200']);
await doorman.waitFor(/ready, sid|PENDING uid/, 15000);
const pendingUid = doorman.lines.map((l) => l.match(/PENDING uid (\S+)/)?.[1]).find(Boolean);
if (pendingUid) await admit(pendingUid, 'mod');
await doorman.waitFor(/ACCOUNT uid=/, 15000);
// A doorman left over as a plain member (an older run): only the owner and mods hear about newcomers.
const doormanAccount = doorman.lines.map((l) => l.match(/ACCOUNT uid=(\S+) .* role=(\S+)/)).find(Boolean);
if (doormanAccount && !['owner', 'mod'].includes(doormanAccount[2])) await admit(doormanAccount[1], 'mod');

// KLIPY mock (KLIPY-style https media URLs, so the server's allowlist accepts the send).
const KLIPY_PORT = Number(process.env.YAP_KLIPY_PORT ?? 8795);
const klipyLog = [];
const klipy = spawn(process.execPath, [path.join(here, 'klipy-mock.mjs'), String(KLIPY_PORT), 'klipy'], { stdio: ['ignore', 'pipe', 'inherit'] });
klipy.stdout.on('data', (d) => klipyLog.push(...String(d).split('\n').filter(Boolean)));
await new Promise((r) => setTimeout(r, 400));

// ── Text ────────────────────────────────────────────────────────────────
const web = await webUser(`Web${RUN}`, `web-${RUN}`);
await test('text: native -> web and web -> native', async () => {
  const n = native(`Native${RUN}`, null, ['--say', `hello from native ${RUN}`, '--expect', `hello from web ${RUN}`, '--duration', '8']);
  await web.waitForSelector(`.msg .body:text("hello from native ${RUN}")`, { timeout: 10000 });
  await web.fill('#input', `hello from web ${RUN} with **bold** and https://example.com/x`);
  await web.press('#input', 'Enter');
  await n.waitFor(new RegExp(`EXPECT_SEEN`), 8000);
  const summary = await n.done;
  assert.ok(summary?.expectSeen, 'native saw the web message');
  evidence.text = n.lines.filter((l) => /message|EXPECT_SEEN|said/.test(l));
});

await test('sends turned away: a burst of 12 from native all reach the web, in order (the server slows the last ones down; native sends them again a moment later, same nonce); a refused reply leaves no grey copy and its text goes back', async () => {
  const notes = Array.from({ length: 12 }, (_, i) => `burst ${RUN} note ${i + 1}.`);
  const refused = `reply to nothing ${RUN}`;
  const n = native(`Native${RUN}`, null, [...notes.flatMap((t) => ['--say', t]), '--reply-to-missing', refused, '--duration', '8']);
  await web.waitForSelector(`.msg .body:text("${notes[11]}")`, { timeout: 15000 });
  const seen = await web.evaluate((prefix) => [...document.querySelectorAll('.msg .body')].map((b) => b.textContent.trim()).filter((t) => t.startsWith(prefix)), `burst ${RUN} note`);
  assert.deepEqual(seen, notes, 'all 12, once each, in the order written');
  const summary = await n.done;
  assert.ok(summary?.sendRetries >= 1, `the server slowed native down (${summary?.sendRetries} retries)`);
  assert.equal(summary.pendingLeft, 0, 'nothing left grey');
  assert.equal(summary.sendsRefused, 1, 'only the reply to nothing was refused');
  assert.deepEqual(summary.restored, [refused], 'its text went back to the message box');
  assert.ok(!n.lines.some((l) => /Slow down a little/.test(l)), 'no slow-down error shown while retrying');
  assert.equal(await web.locator(`.msg .body:text("${refused}")`).count(), 0, 'the refused reply never arrived');
  evidence.sends = n.lines.filter((l) => /SEND|RESTORED|burst/.test(l));
});

// ── Mentions, reactions, pings, GIFs ──────────────────────────────────────
const webName = `Web${RUN}`;
const nativeName = `Native${RUN}`;

await test('@mentions both ways (web highlights, native detects)', async () => {
  const n = native(nativeName, null, ['--say', `@${webName} are you there? ${RUN}`, '--duration', '7']);
  await web.waitForSelector(`.msg.mentions-me .body:has-text("are you there? ${RUN}")`, { timeout: 10000 });
  await web.fill('#input', `@${nativeName} yes, hi ${RUN}`);
  await web.press('#input', 'Enter');
  await n.waitFor(new RegExp(`MENTION from ${webName}: @${nativeName} yes`), 8000);
  await n.done;
  evidence.mentions = { native: n.lines.filter((l) => /MENTION/.test(l)), web: 'message carried .mentions-me' };
});

await test('mentions while native was away: unread.counts says how many mention it (not in code or against an emoji), and they become its badge', async () => {
  // A channel of its own, so it isn't the one native looks at (that one never gets a badge).
  const name = `away-${RUN}`;
  await web.evaluate((n) => yap.ui.send({ t: 'channel.create', name: n, kind: 'text' }), name);
  const ch = await web.waitForFunction((n) => [...yap.state.channels.values()].find((c) => c.name === n)?.id, name, { timeout: 8000 }).then((h) => h.jsonValue());
  const since = Date.now() - 1;
  const post = (body) => web.evaluate(([c, b]) => yap.ui.send({ t: 'send', channelId: c, body: b, nonce: Math.random().toString(36).slice(2) }), [ch, body]);
  await post(`plain while away ${RUN}`);
  await post(`@${nativeName} you there? ${RUN}`);
  await post(`\`@${nativeName}\` in code ${RUN}`);
  await post(`<:wave:${'a'.repeat(32)}>@${nativeName} against an emoji ${RUN}`);
  await new Promise((r) => setTimeout(r, 800)); // (the server handles them in order)
  // Native "read" that channel just before; its first ready asks what came since.
  const n = native(nativeName, null, ['--last-read', `${ch}=${since}`, '--duration', '6']);
  await n.waitFor(new RegExp(`UNREAD .*${ch}:4 \\(1 for you\\)`), 10000);
  await n.waitFor(new RegExp(`MENTIONS ${ch} 1 while away`), 5000);
  await n.done;
  evidence.awayMentions = n.lines.filter((l) => /UNREAD|MENTIONS/.test(l));
});

await test('reactions both ways (native reacts, web shows the pill; web toggles it, native sees)', async () => {
  const n = native(nativeName, null, ['--react-to', `react to me ${RUN}`, '--react', '🎉', '--duration', '10']);
  await n.waitFor(/ready, sid/, 10000);
  await web.fill('#input', `react to me ${RUN}`);
  await web.press('#input', 'Enter');
  const pill = web.locator('.msg', { hasText: `react to me ${RUN}` }).locator('.reaction');
  await pill.first().waitFor({ timeout: 8000 });
  assert.match(await pill.first().innerText(), /🎉\s*1/);
  await pill.first().click(); // the web person adds theirs
  await n.waitFor(new RegExp(`REACTION 🎉 by ${webName} added`), 8000);
  await web.waitForFunction((t) => [...document.querySelectorAll('.msg')].find((m) => m.textContent.includes(t))?.querySelector('.reaction.mine'), `react to me ${RUN}`, { timeout: 5000 });
  await n.done;
  evidence.reactions = n.lines.filter((l) => /REACTION|reacted/.test(l));
});

await test('pings both ways (web toast, native event)', async () => {
  const n = native(nativeName, null, ['--ping', `native ping ${RUN}`, '--duration', '8']);
  await web.waitForSelector(`#toast:has-text("pinged everyone")`, { timeout: 8000 });
  await web.click('[data-action="ping"]');
  // The ping note is asked in Yap's own sheet (not the browser's prompt).
  await web.waitForSelector('#confirm-dialog[open] #confirm-input');
  await web.fill('#confirm-input', `web ping ${RUN}`);
  await web.press('#confirm-input', 'Enter');
  await n.waitFor(new RegExp(`PING from ${webName}: web ping ${RUN}`), 8000);
  await n.done;
  evidence.pings = n.lines.filter((l) => /PING/.test(l));
});

await test('GIF: native searches KLIPY (mock), sends it, reports the share; web renders it', async () => {
  const n = native(nativeName, null, ['--send-gif', `cats${RUN}`, '--duration', '6'], { YAP_KLIPY_API: `http://localhost:${KLIPY_PORT}/api/v1` });
  await n.waitFor(/GIF sent/, 10000);
  await web.waitForSelector(`.msg .gif-embed[title="cats${RUN} 0"], .msg [title="cats${RUN} 0"]`, { timeout: 8000 });
  await n.done;
  const search = klipyLog.find((l) => l.includes(`/gifs/search?q=cats${RUN}`));
  const share = klipyLog.find((l) => l.startsWith(`SHARE cats${RUN}-0`));
  assert.ok(search, 'KLIPY search went straight from the client');
  assert.ok(share && /customer_id/.test(share), 'share reported with customer_id');
  evidence.gif = { search, share };
});

await test('edits both ways (native edits → web shows "(edited)"; web edits with ↑ → native sees it)', async () => {
  const n = native(nativeName, null, ['--say', `edit me ${RUN}`, '--edit-to', `edited by native ${RUN}`, '--duration', '14']);
  await web.waitForSelector(`.msg .body:has-text("edited by native ${RUN}") .edited`, { timeout: 10000 });
  await web.fill('#input', `web original ${RUN}`);
  await web.press('#input', 'Enter');
  await web.waitForSelector(`.msg .body:has-text("web original ${RUN}")`, { timeout: 8000 });
  await web.press('#input', 'ArrowUp');
  await web.fill('#input', `web edited ${RUN}`);
  await web.press('#input', 'Enter');
  await n.waitFor(new RegExp(`EDITED \\S+ web edited ${RUN}`), 8000);
  await n.done;
  evidence.edits = n.lines.filter((l) => /EDITED|editing/.test(l));
});

await test('replies both ways (native replies → web shows the quote and counts it as a mention; web replies → native sees the reply + mention)', async () => {
  const n = native(nativeName, null, ['--reply-to', `reply to me ${RUN}`, '--reply-body', `native reply ${RUN}`, '--duration', '14']);
  await n.waitFor(/ready, sid/, 10000);
  await web.fill('#input', `reply to me ${RUN}`);
  await web.press('#input', 'Enter');
  await n.waitFor(new RegExp(`REPLIED to \\S+: native reply ${RUN}`), 8000);
  const msg = web.locator('.msg', { hasText: `native reply ${RUN}` });
  await msg.locator('.reply-quote').waitFor({ timeout: 8000 });
  assert.match(await msg.locator('.reply-quote').innerText(), new RegExp(`reply to me ${RUN}`));
  assert.ok(await msg.evaluate((el) => el.classList.contains('mentions-me')), 'web: a reply to you counts as a mention');
  await msg.hover();
  await msg.locator('[data-action="reply-message"]').click();
  await web.fill('#input', `web reply ${RUN}`);
  await web.press('#input', 'Enter');
  await n.waitFor(new RegExp(`message #\\S+ <${webName}> web reply ${RUN} \\[reply to \\S+\\]`), 8000);
  await n.waitFor(new RegExp(`MENTION from ${webName}: web reply ${RUN}`), 3000);
  await n.done;
  evidence.replies = n.lines.filter((l) => /REPLIED|MENTION|reply to/.test(l));
});

await test('encrypted Personal space: native sets the passcode (seals an earlier web note), writes a note + picture; web unlocks and reads them; web writes, native reads; deleting the picture erases its blob', () => vaultScenario().finally(() => web.click('#text-channels button.channel >> nth=0')));

async function vaultScenario() {
  const PASS = `purple tiger ${RUN}`;
  await web.click('button.channel.personal');
  await web.fill('#input', `plain before ${RUN}`);
  await web.press('#input', 'Enter');
  await web.waitForSelector(`.msg .body:has-text("plain before ${RUN}")`, { timeout: 8000 });
  // The same account on the native side (another device).
  const photo = path.join(here, '../../web/icon-512.png');
  const n = native(webName, null, ['--channel', 'Personal space', '--vault', PASS, '--say', `sealed by native ${RUN}`, '--send-image', photo, '--delete-after', '16', '--duration', '26']);
  await n.waitFor(/VAULT set/, 15000);
  await n.waitFor(/SEALED 1 earlier notes/, 10000);
  await n.waitFor(/NOTE \S+ text="" \[image /, 15000);
  const blob = n.lines.map((l) => l.match(/NOTE \S+ text="" \[image (\S+) /)?.[1]).find(Boolean);
  // The web hears encryption is on, and stays locked until the passcode.
  await web.waitForFunction(() => yap.state.vault.wrap && !yap.state.vault.key, null, { timeout: 8000 });
  await web.click('[data-action="vault-unlock"]');
  await web.fill('#vault-form input[name=pass]', PASS);
  await web.click('#vault-ok');
  await web.waitForFunction(() => yap.state.vault.key, null, { timeout: 15000 });
  await web.waitForSelector(`.msg .body:has-text("sealed by native ${RUN}")`, { timeout: 8000 });
  await web.waitForSelector(`.msg .body:has-text("plain before ${RUN}")`, { timeout: 8000 });
  const bodies = await web.evaluate(() => yap.state.history.get(`p_${yap.settings.uid}`).messages.map((m) => m.body));
  assert.ok(bodies.length >= 3 && bodies.every((b) => b.startsWith('e1:')), `the server only has ciphertext (${bodies.length} notes)`);
  await web.waitForFunction(() => [...document.querySelectorAll('.msg img.msg-image')].some((i) => i.src.startsWith('blob:') && i.complete && i.naturalWidth > 0), null, { timeout: 10000 });
  assert.equal((await fetch(`${BASE}/media/${blob}`)).status, 200, 'the sealed picture is stored');
  const stored = Buffer.from(await (await fetch(`${BASE}/media/${blob}`)).arrayBuffer());
  assert.ok(!stored.subarray(12, 20).equals(fs.readFileSync(photo).subarray(0, 8)), 'stored picture is not the plain PNG');
  await web.fill('#input', `sealed by web ${RUN}`);
  await web.press('#input', 'Enter');
  await n.waitFor(new RegExp(`NOTE \\S+ text="sealed by web ${RUN}"`), 8000);
  await n.waitFor(/deleting /, 20000);
  let gone = 0;
  for (let i = 0; i < 30 && gone !== 404; i++) {
    await new Promise((r) => setTimeout(r, 200));
    // A fresh URL: the edge cache may keep serving a fetched picture for a day (by design).
    gone = (await fetch(`${BASE}/media/${blob}?after=${Date.now()}`)).status;
  }
  assert.equal(gone, 404, `deleting the note erased its picture (${blob}: ${gone}; native: ${n.lines.filter((l) => /delet|NOTE/.test(l)).join(' | ')})`);
  await n.done;
  evidence.vault = { native: n.lines.filter((l) => /VAULT|SEALED|NOTE|deleting/.test(l)), web: `${bodies.length} notes, all e1:`, blob: { stored: 200, afterDelete: gone } };
}

await test('search + jump: the web says something, 60 messages bury it; native finds it (search everywhere) and jumps to it, loading older pages', async () => {
  await web.fill('#input', `needle in a haystack ${RUN}`);
  await web.press('#input', 'Enter');
  await web.waitForSelector(`.msg .body:has-text("needle in a haystack ${RUN}")`, { timeout: 8000 });
  // Six bots post ten lines each (the rate limit is per connection).
  await Promise.all(Array.from({ length: 6 }, (_, b) => native(`Hay${RUN}${b}`, null, [...Array.from({ length: 10 }, (_, i) => ['--say', `hay ${b}.${i}`]).flat(), '--duration', '4']).done));
  const n = native(nativeName, null, ['--search', `needle haystack ${RUN}`, '--jump-oldest', '--duration', '8']);
  await n.waitFor(/SEARCH 1 results/, 10000);
  await n.waitFor(/JUMPED \S+ after \d+ older pages/, 10000);
  const pages = Number(n.lines.map((l) => l.match(/JUMPED \S+ after (\d+) older pages/)?.[1]).find(Boolean));
  assert.ok(pages >= 1 && pages < 255, `loaded ${pages} older page(s) to reach it`);
  await n.done;
  evidence.search = n.lines.filter((l) => /SEARCH|JUMPED|searching/.test(l));
});

await test('pins both ways: native pins a web message (web shows the pin mark), web pins one (native hears it)', async () => {
  await web.fill('#input', `pin me ${RUN}`);
  await web.press('#input', 'Enter');
  await web.waitForSelector(`.msg .body:has-text("pin me ${RUN}")`, { timeout: 8000 });
  const n = native(nativeName, null, ['--pin-msg', `pin me ${RUN}`, '--duration', '12']);
  await n.waitFor(/PINNED \S+ on/, 10000);
  await web.waitForSelector(`.msg:has(.body:has-text("pin me ${RUN}")) .pin-mark`, { timeout: 8000 });
  await web.fill('#input', `web pins this ${RUN}`);
  await web.press('#input', 'Enter');
  const msg = web.locator('.msg', { hasText: `web pins this ${RUN}` });
  await msg.waitFor({ timeout: 8000 });
  await msg.hover();
  await msg.locator('[data-action="pin-message"]').click();
  const id = await web.evaluate((t) => yap.state.history.get(yap.settings.channel).messages.find((m) => m.body.includes(t))?.id, `web pins this ${RUN}`);
  await n.waitFor(new RegExp(`PINNED ${id} on`), 8000);
  await n.done;
  evidence.pins = n.lines.filter((l) => /PIN/.test(l));
});

await test('Draw: native draws a line (top layer), a box and a word (web sees the strokes, the line with l 2) and saves the GIF (640 × 400, 3 frames, looping; in the web gallery); web draws, native sees it; a wipe of the top layer reaches native', async () => {
  await web.click('[data-action="open-view"][data-view="draw"]');
  await web.waitForFunction(() => yap.ui.draw, null, { timeout: 5000 });
  const before = await web.evaluate(() => yap.ui.draw.strokes.length);
  const n = native(nativeName, null, ['--draw', '--duration', '14']);
  await n.waitFor(/drew a line, a box and a word/, 10000);
  await web.waitForFunction((b) => yap.ui.draw.strokes.length >= b + 3 && yap.ui.draw.strokes.some((s) => s.text === 'hi from native'), before, { timeout: 8000 });
  // Layers (PROTOCOL.md › Draw layers): the line is on the top layer (l 2); the word stays on the bottom one (no l).
  const lineId = n.lines.map((l) => l.match(/the line on the top layer, (\w+)\)/)?.[1]).find(Boolean);
  assert.ok(lineId, 'native says which stroke is the line');
  await web.waitForFunction((id) => yap.ui.draw.strokes.some((s) => s.id === id && s.l === 2), lineId, { timeout: 8000 });
  assert.equal(await web.evaluate(() => 'l' in yap.ui.draw.strokes.find((s) => s.text === 'hi from native')), false, 'the bottom layer is not written');
  const sk = `web${RUN}`;
  await web.evaluate((sk) => yap.ui.send({ t: 'draw.stroke', sk, c: '#ff0000', w: 32, e: false, p: [100, 100, 900, 900, 1500, 400] }), sk);
  await n.waitFor(new RegExp(`DRAW stroke ${sk} by`), 8000);
  // Native's "GIF": 640 × 400, the 3 wiggle frames at 14 cs, looping; the web's gallery gets it.
  await n.waitFor(/DRAW saved (\w+)/, 10000);
  const media = n.lines.map((l) => l.match(/DRAW saved (\w+)/)?.[1]).find(Boolean);
  const gif = Buffer.from(await (await fetch(`${BASE}/media/${media}`)).arrayBuffer());
  assert.equal(gif.subarray(0, 6).toString(), 'GIF89a', 'a GIF');
  assert.deepEqual([gif.readUInt16LE(6), gif.readUInt16LE(8)], [640, 400], '640 × 400');
  const frames = gif.toString('latin1').split('\x21\xf9\x04').slice(1);
  assert.equal(frames.length, 3, 'three wiggle frames');
  assert.ok(frames.every((f) => f.charCodeAt(1) + 256 * f.charCodeAt(2) === 14), '140 ms each');
  assert.ok(gif.includes('NETSCAPE2.0'), 'loops');
  await web.waitForFunction((m) => yap.ui.draw.gallery.some((d) => d.media === m), media, { timeout: 8000 });
  // The web empties the top layer for everyone: native drops the line (and whatever else was up there).
  await web.evaluate(() => yap.ui.send({ t: 'draw.wipe', l: 2 }));
  await n.waitFor(/DRAW wipe 2 by \S+ \([1-9]\d* gone\)/, 8000);
  await n.done;
  await web.click('#text-channels button.channel >> nth=0');
  evidence.draw = n.lines.filter((l) => /DRAW|drew/.test(l));
});

await test('Space: native walks in and waves (web sees the player and the emote); web moves, native sees it; ball, tag, water balloons, fire, laps and the Gallery both ways', async () => {
  await web.click('[data-action="open-view"][data-view="space"]');
  await web.waitForFunction(() => yap.ui.space?.me, null, { timeout: 8000 });
  // Note every space.game the web hears (on top of what it does with it).
  await web.evaluate(() => {
    const sw = yap.ui.space;
    const play = sw.onGame.bind(sw);
    globalThis.spaceGames = [];
    sw.onGame = (m) => { spaceGames.push(m); return play(m); };
  });
  const n = native(nativeName, null, ['--space', '--duration', '12']);
  await n.waitFor(/walked to/, 10000);
  await web.waitForFunction(() => [...yap.ui.space.players.keys()].some((sid) => yap.state.sessions.get(sid)?.client === 'desktop'), null, { timeout: 8000 });
  await web.waitForFunction(() => [...yap.ui.space.emotes.values()].some((e) => e.e === '👋'), null, { timeout: 8000 });
  const sid = await web.evaluate(() => yap.state.sid);
  const nativeSid = await web.evaluate(() => [...yap.ui.space.players.keys()].find((s) => yap.state.sessions.get(s)?.client === 'desktop'));
  await web.evaluate(() => yap.ui.send({ t: 'space.move', x: 700, y: 900, d: 0 }));
  await n.waitFor(new RegExp(`SPACE pos ${sid} 700,900`), 8000);
  // space.game: native's kick reaches the web's ball; the web's games reach native.
  await n.waitFor(/kicked the ball \(k (\d+)\)/, 8000);
  const k = Number(n.lines.map((l) => l.match(/kicked the ball \(k (\d+)\)/)?.[1]).find(Boolean));
  await web.waitForFunction((k) => yap.ui.space.ball?.k === k, k, { timeout: 8000 });
  await web.evaluate(() => yap.ui.send({ t: 'space.game', g: 'tag', d: { it: yap.state.sid.slice(0, 8), k: Math.floor(Date.now() / 1000), left: 90000, t: {} } }));
  await n.waitFor(new RegExp(`SPACE game tag from ${sid}`), 8000);
  // Native throws a balloon, catches fire and runs a lap: the web flies it, draws the flames, posts the lap.
  await n.waitFor(/threw a balloon/, 8000);
  await web.waitForFunction(() => ['throw', 'fx', 'lap'].every((g) => spaceGames.some((m) => m.g === g)), null, { timeout: 8000 });
  const thrown = await web.evaluate(() => spaceGames.find((m) => m.g === 'throw')?.d);
  assert.equal(thrown?.id, 7, 'the balloon reached the web');
  await web.waitForFunction((s) => yap.ui.space.laps.get(s)?.best === 41234, nativeSid, { timeout: 5000 });
  const onFire = await web.evaluate((s) => (yap.ui.space.fx.get(s)?.fire ?? 0) > 0, nativeSid);
  assert.ok(onFire, 'the web sees native on fire');
  // The web throws, gets wet and runs a lap: native hears all three.
  await web.evaluate(() => {
    yap.ui.send({ t: 'space.game', g: 'throw', d: { x: 700, y: 900, a: 0, id: 3 } });
    yap.ui.send({ t: 'space.game', g: 'fx', d: { fire: 0, wet: 4000 } });
    yap.ui.send({ t: 'space.game', g: 'lap', d: { ms: 52000, best: 52000, n: 2 } });
  });
  await n.waitFor(new RegExp(`SPACE game throw from ${sid}`), 8000);
  await n.waitFor(new RegExp(`SPACE game fx from ${sid}`), 8000);
  await n.waitFor(new RegExp(`SPACE game lap from ${sid}`), 8000);
  // The Gallery: native asked for it on arrival and got what the web hangs.
  await n.waitFor(/SPACE gallery \d+/, 8000);
  const nativeGallery = Number(n.lines.map((l) => l.match(/SPACE gallery (\d+)/)?.[1]).find((x) => x !== undefined));
  const webGallery = await web.evaluate(() => yap.ui.space.gallery.length);
  assert.equal(nativeGallery, webGallery, `native's gallery (${nativeGallery}) = the web's (${webGallery})`);
  await n.done;
  await web.click('#text-channels button.channel >> nth=0');
  evidence.space = { native: n.lines.filter((l) => /SPACE|walked|threw/.test(l)).slice(0, 30), webHeard: await web.evaluate(() => spaceGames.map((m) => m.g)), gallery: webGallery };
});

// ── Extras (v0.36) ──────────────────────────────────────────────────────
/** A short mono 16-bit WAV (a two-tone horn). */
function hornWav(file, ms = 800, rate = 24000) {
  const n = Math.round((rate * ms) / 1000);
  const data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++) data.writeInt16LE(Math.round((Math.sin((2 * Math.PI * 440 * i) / rate) * 0.5 + Math.sin((2 * Math.PI * 660 * i) / rate) * 0.3) * (1 - i / n) * 32767), i * 2);
  const h = Buffer.alloc(44);
  h.write('RIFF', 0); h.writeUInt32LE(36 + data.length, 4); h.write('WAVE', 8);
  h.write('fmt ', 12); h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24); h.writeUInt32LE(rate * 2, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34);
  h.write('data', 36); h.writeUInt32LE(data.length, 40);
  fs.writeFileSync(file, Buffer.concat([h, data]));
}
const say = async (text) => {
  await web.fill('#input', text);
  await web.press('#input', 'Enter');
};

await test('extras: polls and dice both ways (native polls, the web votes, native sees the votes; the web polls, native votes; server rolls reach both)', async () => {
  const n = native(nativeName, null, ['--poll', `Native poll ${RUN}|red|blue`, '--say', '/flip', '--vote', '1', '--duration', '16']);
  const card = web.locator('.msg', { hasText: `Native poll ${RUN}` }).locator('.poll');
  await card.waitFor({ timeout: 10000 });
  await card.locator('.poll-option:has-text("blue")').click();
  const pollId = await web.evaluate((t) => yap.state.history.get(yap.settings.channel).messages.find((m) => m.body === t && m.poll)?.id, `Native poll ${RUN}`);
  assert.ok(pollId, 'the web has the poll');
  await n.waitFor(new RegExp(`POLL ${pollId} votes \\[0, 1\\]`), 8000);
  await web.waitForFunction(() => yap.state.history.get(yap.settings.channel).messages.some((m) => m.roll?.kind === 'flip'), null, { timeout: 8000 });
  await say(`/poll Web poll ${RUN} | x | y`);
  await n.waitFor(/voted 1 in/, 8000);
  await web.waitForFunction((t) => yap.state.history.get(yap.settings.channel).messages.find((m) => m.body === t)?.poll?.votes?.[1]?.length === 1, `Web poll ${RUN}`, { timeout: 8000 });
  await say('/roll 2d6');
  await n.waitFor(/ROLL \S+ roll 2d6 \[\d+, \d+\] = \d+/, 8000);
  await n.done;
  evidence.polls = n.lines.filter((l) => /POLL|ROLL|voted|polling/.test(l));
});

await test('extras: away messages (the web is away: native @mentions it and gets the auto-reply; native sets one: the web sees it)', async () => {
  await web.evaluate(() => yap.ui.send({ t: 'profile', away: { text: 'testing the away line', until: null } }));
  await web.waitForFunction(() => yap.state.users.get(yap.settings.uid)?.away?.text === 'testing the away line', null, { timeout: 5000 });
  const n = native(nativeName, null, ['--away', `native away ${RUN}`, '--say', `@${webName} you there? ${RUN}`, '--duration', '8']);
  await n.waitFor(/AWAY \S+: testing the away line/, 8000);
  await web.waitForFunction((t) => [...yap.state.users.values()].some((u) => u.away?.text === t), `native away ${RUN}`, { timeout: 8000 });
  await n.done;
  await web.evaluate(() => yap.ui.send({ t: 'profile', away: null }));
  evidence.away = n.lines.filter((l) => /AWAY|away/.test(l));
});

await test('extras: soundboard (native uploads a WAV and plays it in voice: the web hears it; the web plays it: native hears it)', async () => {
  const wavFile = path.join(OUT, 'horn.wav');
  hornWav(wavFile);
  const name = `Horn${RUN}`;
  await web.click('button.channel.voice:has-text("Lounge")');
  await web.waitForFunction(() => yap.state.voiceChannel === 'lounge', null, { timeout: 10000 });
  const before = await web.evaluate(() => yap.state.soundsHeard ?? 0);
  const n = native(`Sound${RUN}`, null, ['--sound-file', wavFile, '--sound-name', name, '--join-voice', 'lounge', '--play-sound', name, '--duration', '20']);
  await n.waitFor(/SOUND uploaded \S+ board gain/, 12000);
  await n.waitFor(/playing sound/, 12000);
  await web.waitForFunction((b) => (yap.state.soundsHeard ?? 0) > b, before, { timeout: 8000 });
  const id = await web.evaluate((nm) => [...yap.state.sounds.values()].find((s) => s.name === nm)?.id, name);
  assert.ok(id, 'the web knows the clip');
  await new Promise((r) => setTimeout(r, 3200)); // the server allows 1 play per 3 s each
  await web.evaluate((id) => yap.ui.send({ t: 'sound.play', id }), id);
  await n.waitFor(new RegExp(`SOUND ${id} by ${webName} .* played`), 8000);
  await n.done;
  await web.click('[data-action="leave-voice"]');
  evidence.soundboard = n.lines.filter((l) => /SOUND|playing|uploading/.test(l));
});

await test('extras: hangs (native plans one a few seconds out; the web lists it; both hear it start)', async () => {
  const title = `Hang ${RUN}`;
  const n = native(nativeName, null, ['--hang', title, '--hang-in', '4', '--duration', '14']);
  await n.waitFor(/planning|bad_hang/, 10000);
  if (n.lines.some((l) => /bad_hang/.test(l))) {
    console.log('    (start the dev server with --var TEST_HANGS:1 to test hangs end to end)');
    await n.done;
    return;
  }
  await web.waitForFunction((t) => [...yap.state.hangs.values()].some((h) => h.title === t), title, { timeout: 8000 });
  await n.waitFor(new RegExp(`HANG now \\S+ ${title}`), 12000);
  await web.waitForFunction((t) => yap.state.hangNow?.title === t, title, { timeout: 8000 });
  await n.done;
  evidence.hangs = n.lines.filter((l) => /HANG|planning/.test(l));
});

await test('extras: native reacts to a web message with a doodle (the web shows it), saves it to its Personal space (a bookmark quoting it), and asks for the shelf and catch-up counts', async () => {
  const n = native(nativeName, null, ['--doodle-on', `doodle me ${RUN}`, '--save-msg', `doodle me ${RUN}`, '--shelf', '--unread', '--duration', '12']);
  await n.waitFor(/ready, sid/, 10000);
  await new Promise((r) => setTimeout(r, 500));
  await say(`doodle me ${RUN} ||the secret||`);
  const id = (await web.waitForFunction((t) => yap.state.history.get(yap.settings.channel).messages.find((m) => m.body.startsWith(t))?.id, `doodle me ${RUN}`, { timeout: 8000 }).then((h) => h.jsonValue()));
  await n.waitFor(new RegExp(`REACTION <d:[0-9a-f]{32}> by ${nativeName} added on ${id}`), 10000);
  await web.waitForSelector('.doodle-reaction img', { timeout: 8000 });
  await n.waitFor(new RegExp(`message #p_\\S+ <${nativeName}> .*\\[reply to ${id}\\]`), 8000);
  await n.waitFor(/SHELF #\S+ \d+/, 8000);
  await n.done;
  evidence.doodle = n.lines.filter((l) => /REACTION|doodl|sav|SHELF|UNREAD|reply to/.test(l));
});

/** A noisy PNG too big to send as-is (the native client must shrink and re-encode it). */
function bigPng(file, w, h) {
  const raw = crypto.randomBytes((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) raw[y * (w * 3 + 1)] = 0; // filter: none
  const crcTable = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
  const crc = (buf) => { let c = 0xffffffff; for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; };
  const chunk = (type, data) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const td = Buffer.concat([Buffer.from(type), data]);
    const c = Buffer.alloc(4); c.writeUInt32BE(crc(td));
    return Buffer.concat([len, td, c]);
  };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  fs.writeFileSync(file, Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', zlib.deflateSync(raw, { level: 1 })), chunk('IEND', Buffer.alloc(0))]));
}

await test('picture: native compresses a 3000×2000 PNG to 1600 px, uploads it (sid + ut), web renders it; uploads without ut are refused', async () => {
  const file = path.join(OUT, 'big.png');
  bigPng(file, 3000, 2000);
  assert.ok(fs.statSync(file).size > 1_900_000, 'test picture is over the cap');
  const n = native(nativeName, null, ['--send-image', file, '--duration', '12']);
  await n.waitFor(/message #\S+ <\S+> .*\[image\]/, 15000);
  const img = web.locator('.msg img.msg-image').last();
  await img.waitFor({ timeout: 10000 });
  await web.waitForFunction(() => { const i = [...document.querySelectorAll('.msg img.msg-image')].at(-1); return i && i.complete && i.naturalWidth > 0; }, null, { timeout: 10000 });
  const natural = await img.evaluate((i) => [i.naturalWidth, i.naturalHeight]);
  assert.ok(natural[0] === 1600 && Math.abs(natural[0] / natural[1] - 1.5) < 0.01, `compressed to 1600 px, same shape (${natural})`);
  // A session id alone (visible to everyone) is not enough to upload.
  const sid = await web.evaluate(() => yap.state.sid);
  const res = await fetch(`${BASE}/api/image?sid=${sid}`, { method: 'POST', body: fs.readFileSync(path.join(here, '../../web/icon-192.png')), headers: { 'X-Yap-Key': KEY, 'Content-Type': 'image/png' } }).catch((e) => ({ status: String(e) }));
  assert.equal(res.status, 403, 'upload without ut refused');
  await n.done;
  evidence.picture = { natural, noUt: res.status };
});

await test('places + invites: native adds a link (web sidebar shows it); a newcomer joins with a personal invite, the waiting room says who invited them, the inviter (a member, who never saw the request) says "joined — invited by", and they keep the main key', async () => {
  const n = native(nativeName, null, ['--add-link', `Docs${RUN}=https://example.com/${RUN}`, '--make-invite', `for New${RUN}`, '--duration', '25']);
  await n.waitFor(new RegExp(`LINKS .*Docs${RUN}`), 10000);
  await web.waitForSelector(`#links a:has-text("Docs${RUN}")`, { timeout: 8000 });
  await n.waitFor(/INVITE /, 10000);
  const link = n.lines.map((l) => l.match(/INVITE .* (\S+#key=\S+)$/)?.[1]).find(Boolean);
  assert.ok(link && /#key=yi_/.test(link), 'personal invite link');
  const args = ['--headless', '--server', link, '--name', `New${RUN}`, '--password', TEST_PASSWORD, '--signup', '--duration', '6'];
  const out = await new Promise((resolve) => {
    const c = spawn(BIN, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let o = '';
    c.stdout.on('data', (d) => (o += d));
    c.stderr.on('data', (d) => (o += d));
    c.on('exit', () => resolve(o));
  });
  assert.match(out, /ready, sid/, 'newcomer got in (the doorman let them in)');
  assert.match(out, new RegExp(`KEY saved ${keyFp(KEY)}`), 'newcomer adopted the main key');
  await doorman.waitFor(new RegExp(`join request from New${RUN} \\(\\S+\\) invited by ${nativeName} \\(for New${RUN}\\)`), 5000);
  // The inviter is a member: no join request, but join.resolved.user says who joined and whose link.
  await n.waitFor(new RegExp(`JOINED New${RUN} joined — invited by ${nativeName}$`), 5000);
  assert.ok(!n.lines.some((l) => /join request/.test(l)), 'the member heard no join request');
  // The same link again: the join request says who else came in with it (invite.usedBy).
  const again = await new Promise((resolve) => {
    const c = spawn(BIN, ['--headless', '--server', link, '--name', `Neu${RUN}`, '--password', TEST_PASSWORD, '--signup', '--duration', '6'], { stdio: ['ignore', 'pipe', 'pipe'] });
    let o = '';
    c.stdout.on('data', (d) => (o += d));
    c.stderr.on('data', (d) => (o += d));
    c.on('exit', () => resolve(o));
  });
  assert.match(again, /ready, sid/, 'the second newcomer got in too');
  await doorman.waitFor(new RegExp(`join request from Neu${RUN} \\(\\S+\\) invited by ${nativeName} \\(for New${RUN}\\) · link also used by New${RUN}`), 5000);
  await n.done;
  evidence.invites = { link: link.replace(/key=(yi_.{4}).*/, 'key=$1…'), joined: true, usedBy: doorman.lines.filter((l) => l.includes(`Neu${RUN}`) && /also used by/.test(l)), inviterSaw: n.lines.filter((l) => /JOINED/.test(l)).map((l) => l.replace(/^\[\+\s*[\d.]+s\]\s*/, '')) };
});

/** Run `yap --headless …` to the end (with the invite key, or `key`); resolves to everything it printed. */
const runNative = (a, key = KEY) => new Promise((resolve) => {
  const c = spawn(BIN, key ? [...a, '--key', key] : a, { stdio: ['ignore', 'pipe', 'pipe'] });
  let out = '';
  c.stdout.on('data', (d) => (out += d));
  c.stderr.on('data', (d) => (out += d));
  c.on('exit', () => resolve(out));
});

await test('native account: sign in on a "new device" with name + password, then reconnect with the token; a wrong password is turned away', async () => {
  const out1 = await runNative(['--headless', '--server', BASE, '--name', `Native${RUN}`, '--password', TEST_PASSWORD, '--duration', '3']);
  const token = out1.match(/TOKEN (\S+)/)?.[1];
  assert.ok(token, 'login issued a device token');
  assert.match(out1, /ready, sid/);
  assert.match(out1, /ACCOUNT uid=\S+ secret=password weak=false prompt=none role=member/, 'a password account: no "Choose a password" prompt');
  const out2 = await runNative(['--headless', '--server', BASE, '--token', token, '--duration', '2']);
  assert.match(out2, /ready, sid/, 'token reconnect');
  const out3 = await runNative(['--headless', '--server', BASE, '--name', `Native${RUN}`, '--password', 'wrong-horse-000000', '--duration', '2']);
  assert.match(out3, /bad_login/, 'wrong password rejected');
  evidence.accounts = { token: `${token.slice(0, 6)}…`, account: out1.match(/ACCOUNT .*/)?.[0].replace(/uid=\S+ /, ''), wrongPassword: out3.match(/status SignIn (.*)/)?.[1] };
});

await test('an old PIN account (made with the test server switch) signs in with its PIN and is asked once for a password; it chooses one with the PIN as the current, then the password signs in and the PIN no longer does; after a reset the temporary password signs in and asks again', async () => {
  const name = `Old${RUN}`;
  const made = await runNative(['--headless', '--server', BASE, '--name', name, '--password', TEST_PASSWORD, '--signup', '--duration', '3']);
  const uid = made.match(/ACCOUNT uid=(\S+)/)?.[1];
  assert.ok(uid, `the account was made and let in: ${made.match(/status .*/g)?.join(' | ')}`);
  const admin = (route) => fetch(`${BASE}/api/admin/${route}`, { method: 'POST', headers: { Authorization: `Bearer ${ADMIN}`, 'X-Yap-Key': KEY } });
  const set = await admin(`test-pin?uid=${uid}&pin=582039`);
  assert.equal(set.status, 200, 'the server makes it an old PIN account (start it with --var TEST_PINS:1)');
  // Its PIN still signs in; the "Choose a password" prompt would show (an old PIN), and it chooses one.
  const onPin = await runNative(['--headless', '--server', BASE, '--name', name, '--password', '582039', '--set-password', TEST_PASSWORD, '--duration', '3']);
  assert.match(onPin, /ACCOUNT uid=\S+ secret=pin weak=true prompt=Pin /, 'signed in with the PIN, asked for a password');
  assert.match(onPin, /PASSWORD saved/, 'the PIN was taken as the current one');
  const onPassword = await runNative(['--headless', '--server', BASE, '--name', name, '--password', TEST_PASSWORD, '--duration', '2']);
  assert.match(onPassword, /ACCOUNT uid=\S+ secret=password weak=false prompt=none /, 'the new password signs in, nothing asked');
  const oldPin = await runNative(['--headless', '--server', BASE, '--name', name, '--password', '582039', '--duration', '2']);
  assert.match(oldPin, /bad_login/, 'the old PIN no longer works');
  // An owner/mod reset (the admin API's twin): the temporary password signs in and asks again.
  const reset = await (await admin(`reset-pin?uid=${uid}`)).json();
  assert.ok(reset.ok && typeof reset.pin === 'string' && reset.pin.length >= 8, `a temporary password: ${JSON.stringify({ ...reset, pin: reset.pin ? '…' : reset.pin })}`);
  const onTemp = await runNative(['--headless', '--server', BASE, '--name', name, '--password', reset.pin, '--set-password', `${TEST_PASSWORD}-2`, '--duration', '3']);
  assert.match(onTemp, /ACCOUNT uid=\S+ secret=password weak=true prompt=Temporary /, `signed in with the temporary password (${reset.pin.replace(/[a-z]/g, 'x')}), asked to choose their own`);
  assert.match(onTemp, /PASSWORD saved/);
  evidence.oldPin = {
    pin: onPin.match(/ACCOUNT .*/)?.[0].replace(/uid=\S+ /, ''),
    password: onPassword.match(/ACCOUNT .*/)?.[0].replace(/uid=\S+ /, ''),
    pinAfter: oldPin.match(/Error \{ code: "(\w+)"/)?.[1],
    temporary: onTemp.match(/ACCOUNT .*/)?.[0].replace(/uid=\S+ /, ''),
  };
});

await test("waiting room: only the owner and mods hear about a newcomer (a member's app gets no join request, only who joined); the doorman (a mod or the owner) lets them in", async () => {
  const member = native(nativeName, null, ['--duration', '12']);
  await member.waitFor(/ACCOUNT uid=\S+ .* role=member/, 10000);
  const out = await runNative(['--headless', '--server', BASE, '--name', `Wait${RUN}`, '--password', TEST_PASSWORD, '--signup', '--duration', '4']);
  assert.match(out, /PENDING uid/, 'the newcomer waited');
  assert.match(out, /ready, sid/, 'and was let in');
  assert.match(out, /JOINED we're in/, `the newcomer's "You're in"`);
  assert.doesNotMatch(out, new RegExp(`JOINED Wait${RUN} joined`), 'and no "joined" line about itself');
  await doorman.waitFor(new RegExp(`join request from Wait${RUN} `), 5000);
  // The member meets them in the first channel (join.resolved.user): "joined — let in by" the doorman.
  await member.waitFor(new RegExp(`JOINED Wait${RUN} joined — let in by `), 5000);
  member.child.kill();
  await member.done;
  assert.ok(!member.lines.some((l) => /join request/.test(l)), `the member heard nothing: ${member.lines.filter((l) => /join request/.test(l)).join(' | ')}`);
  evidence.waitingRoom = { doorman: doorman.lines.find((l) => l.includes(`join request from Wait${RUN}`))?.replace(/\(\S+\)/, '(…)'), memberJoinRequests: 0, memberSaw: member.lines.find((l) => l.includes(`JOINED Wait${RUN}`))?.replace(/^\[\+\s*[\d.]+s\]\s*/, '') };
});

// ── Voice ───────────────────────────────────────────────────────────────
// RED (PROTOCOL.md › Voice › Redundancy): on unless YAP_RED=0, which the native app also reads (plain Opus both ways).
const RED = process.env.YAP_RED !== '0';
// YAP_VOICE_SECS: how long the native app stays in each call (longer = steadier loss numbers).
const VOICE_SECS = Number(process.env.YAP_VOICE_SECS ?? 24);
async function voiceScenario(label, nativeFirst) {
  const reportFile = path.join(OUT, `call-report-${label}.txt`);
  const n = native(`Voice${RUN}${nativeFirst ? 'N' : 'W'}`, null, ['--join-voice', 'lounge', '--duration', String(VOICE_SECS), '--mute-after', String(VOICE_SECS - 10), '--call-report', reportFile]);
  if (nativeFirst) await n.waitFor(/voice joined lounge/);
  await web.click('button.channel.voice:has-text("Lounge")');
  if (!nativeFirst) {
    await web.waitForSelector('.vp-status.ok', { timeout: 10000 });
  }
  await web.waitForFunction(() => [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 20000 });
  await n.waitFor(/peer \S+ connected/, 20000);
  await web.waitForTimeout(3000); // let the bitrate ramp up
  const s1 = await inbound(web);
  resetWire();
  await web.waitForTimeout(4000);
  const s2 = await inbound(web);
  const kbps = ((s2.bytes - s1.bytes) * 8) / (s2.at - s1.at);
  const sentKbps = ((s2.sentBytes - s1.sentBytes) * 8) / (s2.at - s1.at);
  const nativeRow = () => {
    const s = [...yap.state.sessions.values()].find((x) => x.client === 'desktop' && x.voice);
    return s ? document.querySelector(`.voice-member[data-voice-sid="${CSS.escape(s.sid)}"]`) : null;
  };
  const ring = await web.waitForFunction(`(${nativeRow})()?.classList.contains('speaking')`, null, { timeout: 5000 }).then(() => true, () => false);
  // Native mutes itself 10 s before the end: the web side should see the flag and silence.
  await n.waitFor(/toggling mute/, VOICE_SECS * 1000);
  const s3 = await inbound(web);
  const wire = JSON.parse(JSON.stringify({ toWeb: impairStats.toWeb, toNative: impairStats.toNative }));
  await web.waitForFunction(`!!(${nativeRow})()?.querySelector('.flags svg')`, null, { timeout: 6000 }).catch(() => {});
  await web.waitForTimeout(1500);
  const muted = await inbound(web);
  const flag = await web.evaluate(`!!(${nativeRow})()?.querySelector('.flags svg')`);
  await web.screenshot({ path: path.join(OUT, `web-${nativeFirst ? 'native-first' : 'web-first'}.png`) });
  const summary = await n.done;
  await web.click('[data-action="leave-voice"]');
  await web.waitForFunction(() => yap.voice.peers.size === 0, null, { timeout: 5000 });
  const peer = summary?.peers?.[0] ?? {};
  const pct = (x) => +((100 * (x ?? 0)) / Math.max(1, peer.frames ?? 0)).toFixed(2);
  const secs = (s3.at - s1.at) / 1000;
  evidence[label] = {
    web: { inbound: s2.mime, outbound: s2.outMime, voiceCodecs: { web: s2.localOrder, native: s2.remoteOrder }, opusFmtp: s2.opusFmtp, outFmtp: s2.outFmtp, recvKbps: Math.round(kbps), sentKbps: Math.round(sentKbps),
      wireKbpsFromNative: Math.round(((s3.bytes - s1.bytes) * 8) / 1000 / secs), wireKbpsToNative: Math.round(((s3.sentBytes - s1.sentBytes) * 8) / 1000 / secs),
      packets: s2.packets, lost: s3.lost, patchedPct: patchedPct(s1, s3), audioLevel: s2.level, rtpExtAudioLevel: s2.rtpExtLevel, speakingRing: ring, levelWhileNativeMuted: muted.level, muteFlagShown: flag },
    native: { recvKbps: summary?.recvKbps, maxRecvLevel: summary?.maxRecvLevel, bytesReceived: peer.bytesReceived, packetsReceived: peer.packetsReceived, bytesSent: peer.bytesSent, opusBytesSent: peer.opusBytesSent, sendRed: peer.sendRed, redPacketsReceived: peer.redPackets, sendBitrate: peer.sendBitrate, polite: peer.polite, localFmtp: peer.localFmtp, remoteFmtp: peer.remoteFmtp,
      frames: peer.frames, concealed: peer.concealed, fec: peer.fec, redRebuilt: peer.red, patchedPct: pct(peer.concealed), fecPct: pct(peer.fec), redPct: pct(peer.red), late: peer.late, underruns: peer.underruns, lostPct: +((100 * Math.max(0, (peer.expected ?? 0) - (peer.packetsReceived ?? 0) - (peer.padding ?? 0))) / Math.max(1, peer.expected ?? 0)).toFixed(2), jitterTargetMs: peer.targetMs, lossHint: peer.lossHint },
    ...(IMPAIR ? { impair: { profile: IMPAIR, red: RED, rtpThroughProxy: impairStats.rtp, dropped: impairStats.dropped, nativeRepairedPct: +((100 * ((peer.concealed ?? 0) + (peer.fec ?? 0))) / Math.max(1, peer.frames ?? 0)).toFixed(2),
      wire: Object.fromEntries(Object.entries(wire).map(([k, d]) => [k, { payloadTypes: d.pt, kbps: Math.round((d.bytes * 8) / 1000 / secs) }])) } } : {}),
  };
  // RED both ways when both offer it. (Chrome's stats name the codec that decodes, Opus, even
  // for RED packets: the proof is the packets' payload type on the wire, RED's 63, through the
  // proxy each way; the RED packets native unpacked; the copies doubling the bytes.)
  assert.equal(s2.mime, 'audio/opus');
  assert.equal(s2.remoteOrder, RED ? 'red,opus' : 'opus', "native's description lists RED first");
  assert.ok(RED ? (peer.redPackets ?? 0) > 100 : !peer.redPackets, `native received ${peer.redPackets} RED packets from the web`);
  assert.equal(!!peer.sendRed, RED, 'native sends RED');
  // The copy only fits under 1200 bytes a packet up to ~239 kbps (at 256/510 the primary goes alone).
  const copyFits = 5 + 2 * (BITRATE / 400) <= 1200;
  const fromNative = ((s3.bytes - s1.bytes) * 8) / 1000 / secs;
  assert.ok(RED && copyFits ? fromNative > (BITRATE / 1000) * 1.6 : fromNative < (BITRATE / 1000) * 1.3, `the web received ${fromNative.toFixed(0)} kbps from native (Opus at ${BITRATE / 1000}${RED && copyFits ? ' + its copy' : ''})`);
  if (IMPAIR) {
    for (const [dir, d] of Object.entries(wire)) {
      const red = d.pt[63] ?? 0, opus = d.pt[111] ?? 0;
      assert.ok(RED ? red > 100 && opus === 0 : red === 0 && opus > 100, `${dir}: payload types on the wire ${JSON.stringify(d.pt)}`);
    }
  }
  assert.match(s2.opusFmtp ?? '', /useinbandfec=1/);
  assert.ok(s2.packets > 100, `web received packets: ${s2.packets}`);
  assert.ok(s2.level > 0.001, `web inbound audioLevel: ${s2.level}`);
  assert.ok(kbps > (BITRATE / 1000) * 0.6, `web received ${kbps.toFixed(1)} kbps from native`);
  assert.ok(ring, 'web shows the native speaking ring (RTP audio-level extension)');
  assert.ok(muted.level < 0.001, `web level while native muted: ${muted.level}`);
  assert.ok(flag, 'web shows the native mute flag');
  assert.ok((peer.packetsReceived ?? 0) > 100, `native received packets: ${peer.packetsReceived}`);
  assert.ok((summary?.maxRecvLevel ?? 0) > 0.01, `native decoded level: ${summary?.maxRecvLevel}`);
  assert.ok((summary?.recvKbps ?? 0) > (BITRATE / 1000) * 0.6, `native received ${summary?.recvKbps} kbps from web`);
  assert.match(peer.localFmtp ?? '', new RegExp(`maxaveragebitrate=${BITRATE}`));
  // The call report (PROTOCOL.md › Voice › Call report): the call as native saw it, no IP addresses.
  await new Promise((r) => setTimeout(r, 500));
  const report = fs.existsSync(reportFile) ? fs.readFileSync(reportFile, 'utf8') : '';
  evidence[label].callReport = report.split('\n').filter((l) => /^(App|Channel|Lasted|  (Connection|Codec|Lost|Audio patched|Rebuilt|Jitter)):?/.test(l) || /Call ended/.test(l));
  const webName = `Web${RUN}`;
  for (const want of [/^Yap call report\nApp: Yap desktop \d+\.\d+\.\d+ /, /\nChannel: Lounge\n/, /\nSettings:\n  Quality: \d+ kbps/, /\nAudio devices:\n  Microphone: /,
    /\nPeople \(1\)\n/, new RegExp(`\\n${webName}\\n  Connection: (direct|relay) \\((host|srflx|prflx|relay) ↔ (host|srflx|prflx|relay)\\)`),
    new RegExp(`  Codec: ${RED ? 'red\\+opus out, red\\+opus in' : 'opus out, opus in'}\\n`), /  Opus fmtp sent: .*useinbandfec=1/, /  Lost packets: \d+\.\d % /, /  Audio patched over: \d+\.\d %/,
    /\nTimeline\n\+00:00 You joined Lounge; /, /: connected\n/, /\+00:10 .*: rtt \d+ ms, lost /, /You muted\n/, /Call ended: you quit Yap\n$/]) {
    assert.match(report, want, `call report: ${want}`);
  }
  assert.doesNotMatch(report, /\b\d{1,3}(\.\d{1,3}){3}\b|[0-9a-f]{0,4}::[0-9a-f]|([0-9a-f]{1,4}:){5}/i, 'the call report holds no IP address');
}

await test('voice: native joins first, web offers, native answers; audio both ways', () => voiceScenario('nativeFirst', true));
await test('voice: web joins first, native offers, web answers; audio both ways', () => voiceScenario('webFirst', false));

// An OBS link (the web's /obs page, listen-only) listens without showing up
// while its account is in the call: native still connects to it and sends it
// audio, but no join or leave chime (they're logged as CHIME lines whether or
// not sounds are on); the call report still names it. Its account leaving
// leaves it listening (a newer server: for 5 minutes), and native shows the
// account greyed out (OBSLISTEN lines); back in the call, invisible again.
await test("OBS link: native connects to it and it hears native, with no join or leave chime; the call report names it; while its account is out of the call it shows greyed out, and not once they're back", async () => {
  const reportFile = path.join(OUT, 'call-report-obs.txt');
  const n = native(`Obs${RUN}`, null, ['--join-voice', 'lounge', '--duration', '45', '--call-report', reportFile]);
  await n.waitFor(/voice joined lounge/);
  // The account is in the call first (a newer server only lets its OBS link listen then).
  await web.click('button.channel.voice:has-text("Lounge")');
  await web.waitForFunction(() => [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 20000 });
  await web.evaluate(() => { yap.state.obsToken = null; yap.ui.send({ t: 'obs.create' }); });
  const token = await web.waitForFunction(() => yap.state.obsToken, null, { timeout: 8000 }).then((h) => h.jsonValue());
  const obs = await web.context().newPage();
  const greyed = new RegExp(`OBSLISTEN Lounge: ${webName}$`);
  try {
    await obs.goto(`${BASE}/obs#${new URLSearchParams({ key: KEY, listen: token, channel: 'lounge' })}`);
    await obs.click('#start');
    await obs.waitForFunction(() => yapObs.state.inCall && [...yapObs.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 20000 });
    const obsSid = await obs.evaluate(() => yapObs.state.sid);
    await n.waitFor(new RegExp(`peer ${obsSid} connected`), 10000);
    // Native's voice reaches it. (Polled from here: waitForFunction doesn't await an async predicate.)
    const received = () => obs.evaluate(async () => {
      let total = 0;
      for (const p of yapObs.voice.peers.values()) (await p.pc.getStats()).forEach((r) => { if (r.type === 'inbound-rtp' && r.kind === 'audio') total += r.bytesReceived; });
      return total;
    });
    let bytes = 0;
    for (const until = Date.now() + 10000; (bytes = await received()) < 20000 && Date.now() < until;) await obs.waitForTimeout(250);
    evidence.obs = { bytesFromNative: bytes };
    assert.ok(bytes >= 20000, `the OBS link received ${bytes} bytes of native's voice`);
    await obs.waitForTimeout(3000); // long enough for native's call report to sample it (the codec line)
    assert.ok(!n.lines.some((l) => greyed.test(l)), 'its account is in the call: the OBS link stays invisible');
    // The account leaves: the OBS link listens on, and native shows them greyed out.
    await web.click('[data-action="leave-voice"]');
    await n.waitFor(greyed, 8000);
    const stillListening = await obs.evaluate(() => yapObs.state.inCall);
    // Back in the call: invisible again.
    await web.click('button.channel.voice:has-text("Lounge")');
    await n.waitFor(/OBSLISTEN none$/, 8000);
    evidence.obs = { ...evidence.obs, stillListening, rows: n.lines.filter((l) => /OBSLISTEN/.test(l)).map((l) => l.replace(/^\[\+\s*[\d.]+s\]\s*/, '')) };
  } finally {
    await obs.close();
  }
  // A lost connection's leave chime waits 5 s: give it longer.
  await new Promise((r) => setTimeout(r, 7000));
  const chimes = n.lines.filter((l) => / CHIME (join|leave)/.test(l)).map((l) => l.match(/CHIME (join|leave)/)[1]);
  await web.click('[data-action="leave-voice"]');
  await web.waitForFunction(() => yap.voice.peers.size === 0, null, { timeout: 5000 }).catch(() => {});
  await n.done;
  await new Promise((r) => setTimeout(r, 500));
  const report = fs.existsSync(reportFile) ? fs.readFileSync(reportFile, 'utf8') : '';
  evidence.obs = { ...evidence.obs, chimes, report: report.split('\n').filter((l) => /OBS/.test(l)) };
  assert.deepEqual(chimes, ['join', 'leave', 'join'], 'chimes for the web user joining, leaving and coming back; none for the OBS link');
  assert.match(report, new RegExp(`\\n${webName}'s OBS\\n  Connection: [^\\n]*\\n  Codec: (red\\+)?opus out`), 'the call report names it, with the codec we sent it');
});

await test('screen share + Watch together: native asks what is on at connect; web shares its screen in voice (native sees it on, then off); web joins and leaves the Watch audience (native hears both)', async () => {
  const n = native(nativeName, null, ['--duration', '12']);
  await n.waitFor(/WATCH state /, 10000);
  const sid = await web.evaluate(() => yap.state.sid);
  await web.evaluate(() => yap.ui.send({ t: 'voice.join', channelId: 'lounge' }));
  await web.evaluate(() => yap.ui.send({ t: 'voice.state', muted: true, deafened: false, screen: true }));
  await n.waitFor(new RegExp(`SCREEN ${sid} on`), 8000);
  await web.evaluate(() => yap.ui.send({ t: 'watch.join' }));
  await n.waitFor(new RegExp(`WATCH watching ${sid} on`), 8000);
  await web.evaluate(() => { yap.ui.send({ t: 'watch.leave' }); yap.ui.send({ t: 'voice.leave' }); });
  await n.waitFor(new RegExp(`SCREEN ${sid} off`), 8000);
  await n.waitFor(new RegExp(`WATCH watching ${sid} off`), 8000);
  await n.done;
  evidence.screenWatch = n.lines.filter((l) => /SCREEN|WATCH/.test(l));
});

// ── Screen sharing (never the real screen) ──────────────────────────────
// A web person of their own, in a browser of its own (a fresh audio
// service) with a WebAudio mic, so whatever the voice tests left behind
// doesn't matter here.
await leaveVoice(web).catch(() => {});
const browser2 = await chromium.launch({ args: ['--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream', '--autoplay-policy=no-user-gesture-required'] });
const sweb = await webUser(`Scr${RUN}`, `scr-${RUN}`, { synthMic: true, inBrowser: browser2 });
/** Out of voice (whatever an earlier step left behind). */
async function leaveVoice(page) {
  if (await page.evaluate(() => !yap.state.voiceWanted)) return;
  await page.evaluate(() => yap.voice.stopScreen());
  await page.click('[data-action="leave-voice"]').catch(() => {});
  await page.waitForFunction(() => !yap.state.voiceWanted && yap.voice.peers.size === 0, null, { timeout: 8000 }).catch(() => {});
}
/** The web's stats for the screen it's watching (inbound video, and the share's sound on a later audio transceiver). */
const screenStats = (page) => page.evaluate(async () => {
  const peer = [...yap.voice.peers.values()][0];
  if (!peer) return null;
  const report = await peer.pc.getStats();
  const mic = peer.pc.getTransceivers().find((t) => t.receiver.track.kind === 'audio');
  const micTrack = mic?.receiver.track.id;
  let video = null, codec = null, sound = 0;
  report.forEach((s) => {
    if (s.type === 'inbound-rtp' && s.kind === 'video' && (!video || s.framesDecoded > video.framesDecoded)) video = s;
    if (s.type === 'inbound-rtp' && s.kind === 'audio' && s.trackIdentifier !== micTrack) sound = Math.max(sound, s.bytesReceived ?? 0);
  });
  if (video) codec = report.get(video.codecId);
  const el = document.querySelector('#screen-view video');
  return {
    framesDecoded: video?.framesDecoded ?? 0, width: video?.frameWidth ?? 0, height: video?.frameHeight ?? 0, mime: codec?.mimeType ?? null,
    fmtp: codec?.sdpFmtpLine ?? null, keyFrames: video?.keyFramesDecoded ?? 0, soundBytes: sound,
    element: el ? { w: el.videoWidth, h: el.videoHeight } : null,
  };
});

/** Each audio section's first two codecs in m-line order, as each side described it: [voice, a screen's sound…]. */
const audioOrders = (page) => page.evaluate(() => {
  const peer = [...yap.voice.peers.values()][0];
  const orders = (sdp) => (sdp ?? '').split('\r\nm=').slice(1).filter((s) => s.startsWith('audio')).map((sec) =>
    sec.split('\r\n')[0].split(' ').slice(3).map((pt) => sec.match(new RegExp(`a=rtpmap:${pt} ([^/]+)/`))?.[1]?.toLowerCase()).filter(Boolean).slice(0, 2).join(','));
  return { web: orders(peer?.pc.localDescription?.sdp), native: orders(peer?.pc.remoteDescription?.sdp) };
});
/** RED leads native's voice section only: a screen's sound stays Opus-first (PROTOCOL.md › Voice › Redundancy). */
function assertRedOnVoiceOnly(orders) {
  assert.ok(orders.native.length >= 2, `native described the voice and the screen's sound: ${JSON.stringify(orders)}`);
  assert.equal(orders.native[0], RED ? 'red,opus' : 'opus', "native's voice section");
  for (const o of orders.native.slice(1)) assert.match(o, /^opus\b/, `native's screen sound section: ${JSON.stringify(orders)}`);
}

await test("screen: the desktop watches a browser's share (H.264, the pictures decode at the right size; then the web stops and the desktop hears it)", async () => {
  await leaveVoice(sweb);
  // Native is in the call first; the web joins (and offers), then shares
  // (Sharp: the web keeps the size and drops frames when short of bandwidth,
  // so the size can be checked; its codec list puts VP9 first, but H.264 is
  // all the desktop takes).
  const n = native(`Watch${RUN}`, null, ['--join-voice', 'lounge', '--watch-screen', '--duration', '34']);
  await n.waitFor(/voice joined lounge/, 15000);
  await sweb.click('button.channel.voice:has-text("Lounge")');
  await sweb.waitForFunction(() => [...yap.voice.peers.values()].some((p) => p.pc.connectionState === 'connected'), null, { timeout: 20000 });
  await sweb.evaluate(() => yap.voice.startScreen({ quality: 'sharp' }));
  const webSid = await sweb.evaluate(() => yap.state.sid);
  await n.waitFor(new RegExp(`SCREEN watching ${webSid}`), 15000);
  await n.waitFor(/SCREEN frames [1-9]\d+ 1280x720/, 25000);
  await n.waitFor(/SCREEN frames \d+ 1280x720 sound [1-9]\d{3,}B/, 10000);
  // The web sends H.264 to us (its codec list keeps it; we only take H.264).
  const out = await sweb.evaluate(async () => {
    const peer = [...yap.voice.peers.values()][0];
    const report = await peer.pc.getStats();
    let o = null;
    report.forEach((s) => { if (s.type === 'outbound-rtp' && s.kind === 'video') o = s; });
    return o ? { codec: report.get(o.codecId)?.mimeType, frames: o.framesEncoded, w: o.frameWidth, h: o.frameHeight } : null;
  });
  const orders = await audioOrders(sweb);
  await sweb.waitForTimeout(3000);
  await sweb.evaluate(() => { yap.voice.stopScreen(); });
  await n.waitFor(new RegExp(`SCREEN ${webSid} off`), 8000);
  await n.waitFor(new RegExp(`SCREEN ${webSid} stopped sharing`), 4000);
  await n.waitFor(new RegExp(`SCREEN stopped watching ${webSid}`), 5000);
  const summary = await n.done;
  await leaveVoice(sweb);
  evidence.screenToNative = { webSent: out, audioSections: orders, native: { frames: summary?.screenFrames, size: summary?.screenSize, soundBytes: summary?.screenSoundBytes }, log: n.lines.filter((l) => /SCREEN (watching|stopped|frames 1\d\d )|screen's/.test(l)).slice(0, 8) };
  assert.equal(out?.codec, 'video/H264', `the web sends H.264 (${JSON.stringify(out)})`);
  assertRedOnVoiceOnly(orders);
  assert.ok((summary?.screenFrames ?? 0) > 60, `native decoded ${summary?.screenFrames} pictures`);
  assert.deepEqual(summary?.screenSize, [1280, 720]);
  assert.ok((summary?.screenSoundBytes ?? 0) > 2000, `the share's sound reached native on its own track (${summary?.screenSoundBytes} bytes)`);
});

await test("screen: a browser watches the desktop's share (LIVE badge; H.264 pictures at the pattern's size, the share's sound on its own track; Stop watching stops the encoder)", async () => {
  await leaveVoice(sweb);
  const n = native(`Share${RUN}`, null, ['--join-voice', 'lounge', '--share-screen', 'smooth', '--share-sound', '--duration', '34']);
  await n.waitFor(/SCREEN sharing on/, 15000);
  await n.waitFor(/voice joined lounge/, 15000);
  const nativeSid = await sweb.waitForFunction(() => [...yap.state.sessions.values()].find((s) => s.client === 'desktop' && s.voice?.screen)?.sid, null, { timeout: 10000 }).then((h) => h.jsonValue());
  // The LIVE badge next to them in the voice list opens their screen (and joins the call).
  await sweb.click(`.live-badge[data-sid="${nativeSid}"]`);
  await n.waitFor(/wants our screen/, 30000).catch(async (err) => {
    // What the web saw, for whoever reads the failure.
    await sweb.screenshot({ path: path.join(OUT, 'web-live-click.png') });
    throw err;
  });
  await n.waitFor(/SCREEN encoded [1-9]\d* 1280x720 viewers 1 watching \[Scr/, 20000);
  await sweb.waitForFunction(async () => {
    const peer = [...yap.voice.peers.values()][0];
    if (!peer) return false;
    let ok = false;
    (await peer.pc.getStats()).forEach((s) => { if (s.type === 'inbound-rtp' && s.kind === 'video' && s.framesDecoded > 30) ok = true; });
    return ok;
  }, null, { timeout: 15000, polling: 500 });
  await sweb.waitForTimeout(2000);
  const st = await screenStats(sweb);
  const orders = await audioOrders(sweb);
  await sweb.screenshot({ path: path.join(OUT, 'web-watching-native.png') });
  // Stop watching: the desktop stops sending, and with nobody watching, stops encoding.
  await sweb.click('[data-screen="stop"]');
  await n.waitFor(/is done with our screen/, 8000);
  await n.waitFor(/SCREEN watchers \[\]/, 8000);
  const summary = await n.done;
  await leaveVoice(sweb);
  evidence.screenToWeb = { web: st, audioSections: orders, native: { encoded: summary?.screenEncoded, size: summary?.screenEncodedSize }, log: n.lines.filter((l) => /wants|done with|watchers|encoded [1-9]\d* .* viewers 1|capture and encoder/.test(l)).slice(0, 8) };
  assert.equal(st?.mime, 'video/H264');
  assertRedOnVoiceOnly(orders);
  assert.ok(st.framesDecoded > 30, `web decoded ${st.framesDecoded} pictures`);
  assert.deepEqual([st.width, st.height], [1280, 720]);
  assert.deepEqual(st.element && [st.element.w, st.element.h], [1280, 720], "the web's player shows it at 1280 × 720");
  assert.ok(st.soundBytes > 2000, `the share's sound arrives on its own track (${st.soundBytes} bytes)`);
  assert.ok(n.lines.some((l) => /capture and encoder off/.test(l)), 'nobody watching: no encoding');
});

// ── Resetting the main link ───────────────────────────────────────────────
// Last, because it changes the server's key; the admin API puts YAP_KEY back
// at the end (`DELETE /api/admin/main-key`), so the suite can run again.
// PROTOCOL.md › HTTP › Resetting the main link.
let newKey = null;
await test('main link reset: the owner resets it; connected apps (native and web) save the new key; a device with the old key and a device token still gets in and saves it; a sign-in with the old key is turned away (4003) and one with the new key works; the admin API puts the first key back', async () => {
  const fp = keyFp;
  // A device that signed in before the reset, then was off while it happened.
  const before = await runNative(['--headless', '--server', BASE, '--name', `Stale${RUN}`, '--password', TEST_PASSWORD, '--duration', '3']);
  const staleToken = before.match(/TOKEN (\S+)/)?.[1];
  assert.ok(staleToken && /ready, sid/.test(before), 'a device signed in before the reset');
  // The owner (made so through the admin API), and a member who's connected through it.
  const made = await runNative(['--headless', '--server', BASE, '--name', `Owner${RUN}`, '--password', TEST_PASSWORD, '--duration', '3']);
  const ownerUid = made.match(/ACCOUNT uid=(\S+)/)?.[1];
  assert.ok(ownerUid, 'the owner-to-be signed in');
  await admit(ownerUid, 'owner');
  const member = native(`Member${RUN}`, null, ['--duration', '90']);
  let saved = null;
  try {
    await member.waitFor(/ready, sid/, 15000);
    const owner = native(`Owner${RUN}`, null, ['--reset-main-link', '--duration', '6']);
    await owner.waitFor(/KEY saved sha256:/, 10000);
    saved = owner.lines.find((l) => /KEY saved/.test(l)).match(/sha256:\w+/)[0];
    await owner.done;
    assert.notEqual(saved, fp(KEY), 'a new key');
    await member.waitFor(new RegExp(`KEY saved ${saved}`), 5000);
    newKey = await web.waitForFunction((old) => yap.settings.key !== old && yap.settings.key, KEY, { timeout: 5000 }).then((h) => h.jsonValue(), () => null);
    assert.equal(newKey && fp(newKey), saved, 'the web page saved the same new key');
    // The device that was off: the old key and its token get it in, and it saves the new key.
    const stale = await runNative(['--headless', '--server', BASE, '--token', staleToken, '--duration', '3']);
    assert.match(stale, /ready, sid/, 'the old key plus a device token still connects');
    assert.match(stale, new RegExp(`KEY saved ${saved}`), 'and saves the new key');
    assert.doesNotMatch(stale, /BadKey/);
    // Someone new with the old link: turned away ("The invite key changed…").
    const late = await runNative(['--headless', '--server', BASE, '--name', `Late${RUN}`, '--password', TEST_PASSWORD, '--duration', '4']);
    assert.match(late, /status BadKey/, 'the old link no longer signs anyone in (4003)');
    assert.doesNotMatch(late, /ready, sid|PENDING/);
    // With the new link they get in (the doorman, who saved it too, lets them in).
    const fresh = await runNative(['--headless', '--server', BASE, '--name', `Late${RUN}`, '--password', TEST_PASSWORD, '--duration', '5'], newKey);
    assert.match(fresh, /ready, sid/, 'the new link works');
    evidence.mainLinkReset = { saved, memberSaved: true, webSaved: true, stale: stale.match(/KEY saved \S+/)?.[0], late: late.match(/status BadKey .*/)?.[0], fresh: true };
  } finally {
    // The server's own key again, whatever happened (connected apps get it the same way).
    if (saved) {
      const back = await fetch(`${BASE}/api/admin/main-key`, { method: 'DELETE', headers: { Authorization: `Bearer ${ADMIN}` } });
      await member.waitFor(new RegExp(`KEY saved ${fp(KEY)}`), 5000).catch(() => {});
      evidence.mainLinkReset = { ...evidence.mainLinkReset, back: back.status === 200 && member.lines.some((l) => l.includes(`KEY saved ${fp(KEY)}`)) };
    }
    member.child.kill();
    await member.done;
  }
  assert.ok(evidence.mainLinkReset.back, 'the admin API put YAP_KEY back, and the member saved it');
});

await browser2.close();
await browser.close();
for (const s of proxies) s.close();
doorman.child.kill();
klipy.kill();
for (const [mark, name, err] of results) {
  console.log(`${mark} ${name}`);
  if (err) console.log(`    ${err.message ?? err}`);
}
console.log('\nEVIDENCE ' + JSON.stringify(evidence, null, 2));
console.log(`artifacts: ${OUT}`);
const failed = results.filter((r) => r[0] === '✗').length;
console.log(`${results.length - failed}/${results.length} passed`);
process.exit(failed ? 1 : 0);
