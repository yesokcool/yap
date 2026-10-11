// The web half of the native voice interop test (apple/README.md › Voice interop).
//
// A headless Chromium runs the real web client with a synthetic mic (a
// music-like chord with noise, made in WebAudio: getUserMedia is replaced,
// so no device is opened and the OS never asks for the microphone) and plays
// these scenarios against whatever native client shows up:
//   1. web-first: the web user sits in Lounge; the native client joins and
//      OFFERS to it (native = newcomer).
//   2. native-first: the web user waits until a session whose client label is
//      "yap-voice-test" is in Lounge, then joins and offers (native ANSWERS).
//      Mid-call the native test drops its socket; once the call has moved to
//      the native's new session, the web drops its own too. Both must keep
//      the same connection (PROTOCOL.md › Keeping calls across reconnects):
//      the web reports every gap in the audio it receives, the connections
//      it saw and the chimes it played.
//   3. web-sharing: the web user sits in Lounge sharing a synthetic screen
//      (a moving canvas + a 440 Hz tone as its sound; getDisplayMedia is
//      replaced, nothing real is captured) through the web's own
//      `voice.startScreen`; the native client joins, watches (`{screen:
//      "want"}`), checks it decodes H.264, then stops watching. The web
//      reports who it's sending video to.
//   4. native-sharing: the native client shares a synthetic screen; the web
//      clicks its LIVE badge (the real UI), counts the frames it decodes
//      (codec, size), then clicks Stop watching.
// Every 250 ms it writes what the web side sees (connection state, inbound
// Opus bytes/packets/level, fmtp, and the counters RED shows up in) to a
// JSON status file the native test reads.
//
// YAP_IMPAIR=wifi|cell|loss5|loss2 puts the calls behind an impairment proxy
// (RTP only, both ways; the voice audit's profiles, the same as
// desktop/tests/interop.mjs): the web routes the native client's candidates
// through a local UDP proxy and never tells it its own, so everything goes
// through it. YAP_PEER_SCENARIOS=voice plays only the two voice scenarios.
//
//   YAP_URL=http://127.0.0.1:8789 YAP_KEY=testkey YAP_ADMIN_KEY=testadmin \
//     node apple/Tools/web-voice-peer.mjs <status.json>
// Needs the repo's node_modules (playwright + its Chromium).
import dgram from 'node:dgram';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { chromium } from 'playwright';

const BASE = process.env.YAP_URL ?? 'http://127.0.0.1:8789';
const KEY = process.env.YAP_KEY ?? '';
const ADMIN = process.env.YAP_ADMIN_KEY ?? '';
const STATUS = process.argv[2] ?? path.join(os.tmpdir(), 'yap-web-peer.json');
const NATIVE_CLIENT = process.env.YAP_NATIVE_CLIENT ?? 'yap-voice-test';
const TIMEOUT_MS = Number(process.env.YAP_PEER_TIMEOUT_MS ?? 240_000);
const RUN = Math.random().toString(36).slice(2, 6);
const NAME = `Webby ${RUN}`;
const PASSWORD = 'correct-horse-582039';
const VOICE_ONLY = process.env.YAP_PEER_SCENARIOS === 'voice';

const IMPAIR = process.env.YAP_IMPAIR || null;
const PROFILES = {
  loss2: { base: 0, jit: 0, spikeP: 0, spike: [0, 0], loss: 0.02 },
  loss5: { base: 0, jit: 0, spikeP: 0, spike: [0, 0], loss: 0.05 },
  wifi: { base: 15, jit: 10, spikeP: 0.02, spike: [60, 200], loss: 0.01 },
  cell: { base: 40, jit: 20, spikeP: 0.03, spike: [100, 300], loss: 0.02 },
};
if (IMPAIR && !PROFILES[IMPAIR]) throw new Error(`YAP_IMPAIR: one of ${Object.keys(PROFILES).join(', ')}`);
const impairStats = { rtp: 0, dropped: 0 };
const proxies = [];
/** A UDP proxy in front of one of the native client's candidates; RTP through it (both ways) gets the profile. */
function mkProxy(host, port) {
  const prof = PROFILES[IMPAIR];
  const sock = dgram.createSocket('udp4');
  proxies.push(sock);
  let client = null;
  sock.on('message', (msg, r) => {
    const fromTarget = r.address === host && r.port === port;
    if (!fromTarget) client = { address: r.address, port: r.port };
    const to = fromTarget ? client : { address: host, port };
    if (!to) return;
    // RTP: version 2, and not RTCP (RFC 5761: 64–95 in the payload-type bits). Not "96–127":
    // RED's payload type is usually 63, and it would slip through unimpaired.
    const pt = msg[1] & 0x7f;
    const isRtp = msg.length > 12 && (msg[0] & 0xc0) === 0x80 && !(pt >= 64 && pt <= 95);
    if (!isRtp) return sock.send(msg, to.port, to.address);
    impairStats.rtp++;
    if (Math.random() < prof.loss) return impairStats.dropped++;
    let d = prof.base + (prof.jit ? -prof.jit * Math.log(1 - Math.random()) : 0);
    if (Math.random() < prof.spikeP) d += prof.spike[0] + Math.random() * (prof.spike[1] - prof.spike[0]);
    if (d < 1) return sock.send(msg, to.port, to.address);
    setTimeout(() => sock.send(msg, to.port, to.address), d);
  });
  return new Promise((res) => sock.bind(0, '0.0.0.0', () => res(sock.address().port)));
}

const browser = await chromium.launch({
  args: [
    '--use-fake-device-for-media-stream',
    '--use-fake-ui-for-media-stream',
    '--autoplay-policy=no-user-gesture-required',
  ],
});

const status = { name: NAME, phase: 'starting', sid: null, uid: null, scenarios: {}, error: null, at: Date.now() };
const write = () => {
  status.at = Date.now();
  if (status.impair) Object.assign(status.impair, impairStats);
  fs.writeFileSync(`${STATUS}.tmp`, JSON.stringify(status, null, 2));
  fs.renameSync(`${STATUS}.tmp`, STATUS);
};

try {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 }, permissions: ['microphone'] });
  if (IMPAIR) {
    status.impair = { profile: IMPAIR, ...impairStats };
    await context.exposeFunction('__mkProxy', (host, port) => mkProxy(host, port));
    await context.addInitScript(() => {
      const stripCandidates = (sdp) => sdp.split('\r\n').filter((l) => !l.startsWith('a=candidate:') && l !== 'a=end-of-candidates').join('\r\n');
      const add = RTCPeerConnection.prototype.addIceCandidate;
      RTCPeerConnection.prototype.addIceCandidate = async function (c, ...rest) {
        const line = c?.candidate ?? '';
        const p = line.split(' ');
        if (!line) return add.call(this, c, ...rest);
        if (p[2]?.toLowerCase() !== 'udp' || p[4].includes(':') || p[4].endsWith('.local')) return; // TCP, IPv6, mDNS: not through the proxy
        p[5] = String(await globalThis.__mkProxy(p[4], Number(p[5])));
        return add.call(this, { ...c, candidate: p.join(' ') }, ...rest);
      };
      // Candidates inside a description would skip the proxy: none either way.
      const setRemote = RTCPeerConnection.prototype.setRemoteDescription;
      RTCPeerConnection.prototype.setRemoteDescription = function (d, ...rest) {
        return setRemote.call(this, d?.sdp ? { type: d.type, sdp: stripCandidates(d.sdp) } : d, ...rest);
      };
      const local = Object.getOwnPropertyDescriptor(RTCPeerConnection.prototype, 'localDescription');
      Object.defineProperty(RTCPeerConnection.prototype, 'localDescription', {
        get() {
          const d = local.get.call(this);
          if (!d) return d;
          const out = { type: d.type, sdp: stripCandidates(d.sdp) };
          return { ...out, toJSON: () => out };
        },
      });
      // Never tell the native client our own candidates: it must find us through the proxy (peer-reflexive).
      const desc = Object.getOwnPropertyDescriptor(RTCPeerConnection.prototype, 'onicecandidate');
      Object.defineProperty(RTCPeerConnection.prototype, 'onicecandidate', {
        set(fn) { desc.set.call(this, fn && ((e) => { if (e.candidate) return; fn(e); })); },
        get() { return desc.get.call(this); },
      });
    });
  }
  await context.addInitScript(() => {
    // The mic: a 12 s chord with noise, looped (Opus spends its bits on it).
    const realGetUserMedia = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getUserMedia = async (constraints) => {
      if (!constraints?.audio || constraints.video) return realGetUserMedia(constraints);
      const sr = 48000;
      const ac = new AudioContext({ sampleRate: sr });
      const buffer = ac.createBuffer(1, sr * 12, sr);
      const data = buffer.getChannelData(0);
      let seed = 1;
      const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) * 2 - 1;
      for (let i = 0; i < data.length; i++) {
        const t = i / sr;
        const chord = Math.floor(t) % 2 ? [196, 246.94, 293.66] : [220, 277.18, 329.63];
        let v = chord.reduce((sum, f) => sum + Math.sin(2 * Math.PI * f * t) * 0.18, 0) + rnd() * 0.12;
        v *= 0.6 + 0.4 * Math.sin(2 * Math.PI * 0.5 * t);
        data[i] = Math.max(-1, Math.min(1, v)) * 0.92;
      }
      const source = new AudioBufferSourceNode(ac, { buffer, loop: true });
      const out = ac.createMediaStreamDestination();
      source.connect(out);
      source.start();
      return out.stream;
    };
    // Screen sharing without a screen: a moving test card and a tone, like the native's synthetic feeder.
    navigator.mediaDevices.getDisplayMedia = async () => {
      const canvas = document.createElement('canvas');
      canvas.width = 640;
      canvas.height = 360;
      const g = canvas.getContext('2d');
      let n = 0;
      setInterval(() => {
        n++;
        g.fillStyle = `hsl(${(n * 3) % 360} 55% 35%)`;
        g.fillRect(0, 0, 640, 360);
        g.fillStyle = '#fff';
        g.fillRect((n * 8) % 640, 0, 64, 360);
      }, 33);
      const stream = canvas.captureStream(30);
      const ac = new AudioContext();
      const tone = new OscillatorNode(ac, { frequency: 440 });
      const out = ac.createMediaStreamDestination();
      tone.connect(new GainNode(ac, { gain: 0.2 })).connect(out);
      tone.start();
      stream.addTrack(out.stream.getAudioTracks()[0]);
      return stream;
    };
    if (!localStorage.getItem('yap.settings')) {
      localStorage.setItem('yap.settings', JSON.stringify({ color: '#12a594', bitrate: 128000, mode: 'open' }));
    }
    // Chimes and blips are oscillators: count every start.
    globalThis.__chimes = 0;
    const start = OscillatorNode.prototype.start;
    OscillatorNode.prototype.start = function (...args) {
      globalThis.__chimes++;
      return start.apply(this, args);
    };
  });
  const page = await context.newPage();
  page.on('pageerror', (err) => console.log('[web] page error:', err.message));
  await page.goto(`${BASE}/#key=${encodeURIComponent(KEY)}`);

  // Get online like a person would: "I'm new" with a password (one Password
  // field, no "again" since round 6) → waiting room → the admin lets us in.
  // The web never opens its password dialog by itself any more (only from
  // Settings or the "Choose a password" bar), so there's nothing else to fill.
  for (let i = 0; i < 8; i++) {
    const s = await page.waitForFunction(() => {
      if (document.querySelector('#welcome-dialog')?.open) return 'welcome';
      const st = globalThis.yap?.state.status;
      return st === 'online' || st === 'waiting' ? st : false;
    }, null, { timeout: 20000 }).then((h) => h.jsonValue());
    if (s === 'welcome') {
      await page.click('#welcome-form label:has(input[value=signup])');
      await page.fill('#welcome-form input[name=name]', NAME);
      await page.fill('#welcome-form input[name=pin]', PASSWORD);
      await page.click('#welcome-form button[type=submit]');
      await page.waitForFunction(() => !document.querySelector('#welcome-dialog')?.open || document.querySelector('#welcome-error:not([hidden])'), null, { timeout: 10000 });
      // Refused (e.g. the server's password rules): say so instead of timing out later.
      const refused = await page.evaluate(() => (document.querySelector('#welcome-dialog')?.open && document.querySelector('#welcome-error:not([hidden])')?.textContent) || '');
      if (refused) throw new Error(`sign-up refused: ${refused}`);
    } else if (s === 'waiting') {
      const uid = await page.evaluate(() => yap.settings.uid);
      await fetch(`${BASE}/api/admin/role?uid=${uid}&role=member`, { method: 'POST', headers: { Authorization: `Bearer ${ADMIN}` } });
      await page.waitForFunction(() => yap.state.status === 'online', null, { timeout: 10000 });
    } else {
      await page.waitForTimeout(700);
      if (await page.evaluate(() => !document.querySelector('dialog[open]'))) break;
    }
  }
  Object.assign(status, await page.evaluate(() => ({ sid: yap.state.sid, uid: yap.settings.uid })));

  // Every 20 ms: did packets arrive? Gaps over 100 ms are kept (epoch ms, length).
  await page.evaluate(() => {
    globalThis.__gaps = [];
    let last = null;
    let lastAt = null;
    let busy = false;
    let id = 0;
    const ids = new WeakMap();
    globalThis.__pcIds = new Set();
    setInterval(async () => {
      if (busy) return;
      busy = true;
      try {
        let total = 0;
        for (const peer of yap.voice.peers.values()) {
          if (!ids.has(peer.pc)) ids.set(peer.pc, ++id);
          globalThis.__pcIds.add(ids.get(peer.pc));
          (await peer.pc.getStats()).forEach((s) => { if (s.type === 'inbound-rtp' && s.kind === 'audio') total += s.packetsReceived; });
        }
        const now = Date.now();
        if (last !== null && total !== last && total > 0) {
          if (lastAt !== null && now - lastAt > 100) globalThis.__gaps.push([lastAt, now - lastAt]);
          lastAt = now;
        } else if (last === null && total > 0) lastAt = now;
        last = total;
      } finally {
        busy = false;
      }
    }, 20);
  });

  const snapshot = () => page.evaluate(async () => {
    const peers = [];
    for (const [sid, peer] of yap.voice.peers) {
      const report = await peer.pc.getStats();
      let inbound, outbound, codec, outCodec;
      report.forEach((s) => {
        if (s.type === 'inbound-rtp' && s.kind === 'audio') inbound = s;
        if (s.type === 'outbound-rtp' && s.kind === 'audio') outbound = s;
      });
      if (inbound) codec = report.get(inbound.codecId);
      if (outbound) outCodec = report.get(outbound.codecId);
      const session = yap.state.sessions.get(sid);
      // The voice section's codecs, first one first (libwebrtc sends the first of the remote description's).
      const firstCodec = (sdp) => {
        const lines = (sdp ?? '').split('\r\n');
        const m = lines.find((l) => l.startsWith('m=audio'));
        const pt = m?.split(' ')[3];
        return lines.find((l) => l.startsWith(`a=rtpmap:${pt} `))?.split(' ')[1]?.split('/')[0] ?? null;
      };
      peers.push({
        sid,
        uid: peer.uid ?? session?.uid ?? null,
        client: session?.client ?? null,
        polite: peer.polite,
        state: peer.pc.connectionState,
        bytesReceived: inbound?.bytesReceived ?? 0,
        packetsReceived: inbound?.packetsReceived ?? 0,
        packetsLost: inbound?.packetsLost ?? 0,
        level: inbound?.audioLevel ?? 0,
        mime: codec?.mimeType ?? null,
        fmtp: codec?.sdpFmtpLine ?? null,
        bytesSent: outbound?.bytesSent ?? 0,
        packetsSent: outbound?.packetsSent ?? 0,
        outMime: outCodec?.mimeType ?? null,
        outFmtp: outCodec?.sdpFmtpLine ?? null,
        targetBitrate: outbound?.targetBitrate ?? null,
        packetsDiscarded: inbound?.packetsDiscarded ?? 0,
        concealedSamples: inbound?.concealedSamples ?? 0,
        silentConcealedSamples: inbound?.silentConcealedSamples ?? 0,
        totalSamplesReceived: inbound?.totalSamplesReceived ?? 0,
        fecPacketsReceived: inbound?.fecPacketsReceived ?? 0,
        fecPacketsDiscarded: inbound?.fecPacketsDiscarded ?? 0,
        jitterBufferDelay: inbound?.jitterBufferDelay ?? 0,
        jitterBufferEmittedCount: inbound?.jitterBufferEmittedCount ?? 0,
        localFirstCodec: firstCodec(peer.pc.localDescription?.sdp),
        remoteFirstCodec: firstCodec(peer.pc.remoteDescription?.sdp),
        remoteDescriptionHasTunedOpus: /maxaveragebitrate=\d+/.test(peer.pc.remoteDescription?.sdp ?? ''),
        speaking: document.querySelector(`[data-voice-sid="${CSS.escape(sid)}"]`)?.classList.contains('speaking') ?? false,
        mutedFlag: !!session?.voice?.muted,
      });
    }
    return { voiceChannel: yap.state.voiceChannel, sid: yap.state.sid, peers, gaps: globalThis.__gaps, connections: globalThis.__pcIds.size, chimes: globalThis.__chimes };
  });

  /** The video the web receives / sends, per peer (screen sharing). */
  const videoStats = () => page.evaluate(async () => {
    const out = {};
    for (const [sid, peer] of yap.voice.peers) {
      const report = await peer.pc.getStats();
      const v = { client: yap.state.sessions.get(sid)?.client ?? null, framesDecoded: 0, frameWidth: 0, frameHeight: 0, mime: null, fmtp: null, framesEncoded: 0, sentMime: null, sending: Boolean(peer.screenSenders), wants: Boolean(peer.wantsScreen) };
      report.forEach((s) => {
        if (s.type === 'inbound-rtp' && s.kind === 'video') {
          v.framesDecoded += s.framesDecoded ?? 0;
          v.frameWidth = s.frameWidth ?? v.frameWidth;
          v.frameHeight = s.frameHeight ?? v.frameHeight;
          const codec = report.get(s.codecId);
          v.mime = codec?.mimeType ?? v.mime;
          v.fmtp = codec?.sdpFmtpLine ?? v.fmtp;
        }
        if (s.type === 'outbound-rtp' && s.kind === 'video') {
          v.framesEncoded += s.framesEncoded ?? 0;
          v.sentMime = report.get(s.codecId)?.mimeType ?? v.sentMime;
        }
      });
      out[sid] = v;
    }
    return out;
  });

  /** Scenario 3: share a synthetic screen; the native asks for it, watches, stops. */
  async function webShares() {
    const record = { sharing: false, watchedBy: null, sentFrames: 0, sentMime: null, stoppedWatching: false };
    status.scenarios.webShares = record;
    await page.click('button.channel.voice[data-id="lounge"]');
    await page.waitForFunction(() => yap.state.voiceChannel === 'lounge', null, { timeout: 10000 });
    await page.evaluate(() => yap.voice.startScreen({ quality: 'smooth' }));
    record.sharing = true;
    status.phase = 'web-sharing';
    write();
    console.log('[web] sharing a synthetic screen in Lounge; waiting for the native client to watch');
    const started = Date.now();
    while (Date.now() - started < TIMEOUT_MS) {
      const stats = await videoStats();
      const native = Object.entries(stats).find(([, v]) => v.client === NATIVE_CLIENT);
      if (native) {
        const [sid, v] = native;
        if (v.sending) record.watchedBy = sid;
        record.sentFrames = Math.max(record.sentFrames, v.framesEncoded);
        record.sentMime = v.sentMime ?? record.sentMime;
        record.sending = v.sending;
        if (record.watchedBy && !v.sending && !v.wants) record.stoppedWatching = true;
      }
      write();
      if (record.stoppedWatching) break;
      await page.waitForTimeout(250);
    }
    await page.evaluate(() => yap.voice.stopScreen());
    record.sharing = false;
    write();
    // The native leaves voice next; then it's its turn to share.
    await page.waitForFunction((label) => ![...yap.state.sessions.values()].some((s) => s.client === label && s.voice?.channelId === 'lounge'), NATIVE_CLIENT, { timeout: TIMEOUT_MS, polling: 200 });
  }

  /** Scenario 4: the native shares; click its LIVE badge, count decoded frames, Stop watching. */
  async function nativeShares() {
    const record = { watching: false, framesDecoded: 0, frameWidth: 0, frameHeight: 0, mime: null, fmtp: null, stopped: false, live: false };
    status.scenarios.nativeShares = record;
    status.phase = 'waiting-for-native-share';
    write();
    console.log('[web] waiting for the native client to share its screen in Lounge');
    const sharer = await page.waitForFunction((label) => [...yap.state.sessions.values()].find((s) => s.client === label && s.voice?.channelId === 'lounge' && s.voice?.screen)?.sid,
      NATIVE_CLIENT, { timeout: TIMEOUT_MS, polling: 200 }).then((h) => h.jsonValue());
    record.live = true;
    // The real UI: the LIVE badge next to them under Lounge.
    await page.click(`button.live-badge[data-sid="${sharer}"]`);
    record.watching = true;
    status.phase = 'native-sharing';
    write();
    console.log(`[web] watching ${sharer}'s screen`);
    const started = Date.now();
    while (Date.now() - started < 60_000) {
      const v = (await videoStats())[sharer];
      if (v) {
        record.framesDecoded = v.framesDecoded;
        record.frameWidth = v.frameWidth;
        record.frameHeight = v.frameHeight;
        record.mime = v.mime ?? record.mime;
        record.fmtp = v.fmtp ?? record.fmtp;
      }
      write();
      if (record.framesDecoded >= 60) break;
      await page.waitForTimeout(250);
    }
    await page.waitForTimeout(1000);
    await page.click('[data-screen="stop"]');
    record.stopped = true;
    status.phase = 'native-shared';
    write();
    console.log(`[web] decoded ${record.framesDecoded} frames ${record.frameWidth}x${record.frameHeight} ${record.mime} [${record.fmtp}]; stopped watching`);
    await page.waitForFunction((label) => ![...yap.state.sessions.values()].some((s) => s.client === label && s.voice?.channelId === 'lounge'), NATIVE_CLIENT, { timeout: TIMEOUT_MS, polling: 200 });
    await page.click('[data-action="leave-voice"]').catch(() => {});
  }

  /** Run one scenario: record what the web side sees until the native peer leaves. */
  async function scenario(key) {
    const record = { connected: false, maxBytesReceived: 0, maxPacketsReceived: 0, maxLevel: 0, samples: [], mime: null, fmtp: null, nativeClient: null };
    status.scenarios[key] = record;
    const started = Date.now();
    const chimes0 = await page.evaluate(() => {
      globalThis.__pcIds = new Set();
      return globalThis.__chimes;
    });
    let sawPeer = false;
    let nativeUid = null;
    let firstNativeSid = null;
    while (Date.now() - started < TIMEOUT_MS) {
      const snap = await snapshot();
      status.voiceChannel = snap.voiceChannel;
      status.sid = snap.sid ?? status.sid;
      record.gaps = snap.gaps;
      record.connections = snap.connections;
      record.chimes = snap.chimes - chimes0;
      const native = snap.peers.find((p) => p.client === NATIVE_CLIENT || (nativeUid && p.uid === nativeUid)) ?? snap.peers[0];
      if (key === 'nativeFirst' && native?.state === 'connected') {
        // The native test drops its socket first; once the call moved to its
        // new session (same connection, new sid), it's our turn.
        nativeUid ??= native.uid;
        firstNativeSid ??= native.sid;
        if (native.sid !== firstNativeSid && !record.webDropAt) {
          record.nativeResumedAt ??= Date.now();
          if (Date.now() - record.nativeResumedAt > 3000) {
            record.webDropAt = Date.now();
            await page.evaluate(() => yap.ui.dropSocket());
            console.log('[web] dropped our socket mid-call');
          }
        }
      }
      if (native) {
        sawPeer = true;
        record.nativeClient = native.client;
        record.polite = native.polite;
        if (native.state === 'connected') record.connected = true;
        record.state = native.state;
        record.maxBytesReceived = Math.max(record.maxBytesReceived, native.bytesReceived);
        record.maxPacketsReceived = Math.max(record.maxPacketsReceived, native.packetsReceived);
        record.maxLevel = Math.max(record.maxLevel, native.level);
        record.mime = native.mime ?? record.mime;
        record.fmtp = native.fmtp ?? record.fmtp;
        record.outFmtp = native.outFmtp ?? record.outFmtp;
        record.remoteDescriptionHasTunedOpus = native.remoteDescriptionHasTunedOpus;
        record.bytesSent = native.bytesSent;
        record.latest = native;
        record.samples.push({ t: Date.now() - started, bytes: native.bytesReceived, level: native.level, state: native.state, muted: native.mutedFlag, speaking: native.speaking });
        if (record.samples.length > 400) record.samples.shift();
      } else if (sawPeer) {
        record.left = true;
        write();
        return record;
      }
      write();
      await page.waitForTimeout(250);
    }
    record.timedOut = true;
    return record;
  }

  // Scenario 1: web first.
  await page.click('button.channel.voice[data-id="lounge"]');
  await page.waitForFunction(() => yap.state.voiceChannel === 'lounge', null, { timeout: 10000 });
  status.phase = 'web-first';
  write();
  console.log(`[web] ${NAME} (${status.sid}) is in Lounge; waiting for ${NATIVE_CLIENT} to join and offer`);
  await scenario('webFirst');
  await page.click('[data-action="leave-voice"]');
  await page.waitForFunction(() => !yap.state.voiceChannel, null, { timeout: 5000 });

  // Scenario 2: native first.
  status.phase = 'waiting-for-native';
  write();
  console.log(`[web] waiting for ${NATIVE_CLIENT} to sit in Lounge first`);
  await page.waitForFunction((label) => [...yap.state.sessions.values()].some((s) => s.client === label && s.voice?.channelId === 'lounge'), NATIVE_CLIENT, { timeout: TIMEOUT_MS, polling: 200 });
  await page.click('button.channel.voice[data-id="lounge"]');
  status.phase = 'native-first';
  write();
  console.log('[web] joined after the native client; offering to it');
  await scenario('nativeFirst');
  await page.click('[data-action="leave-voice"]').catch(() => {});
  await page.waitForFunction(() => !yap.state.voiceChannel, null, { timeout: 5000 }).catch(() => {});

  if (!VOICE_ONLY) {
    // Scenario 3: the web shares, the native watches.
    await webShares();
    // Scenario 4: the native shares, the web watches (its LIVE badge).
    await nativeShares();
  }
  status.phase = 'done';
} catch (err) {
  status.error = String(err?.stack ?? err);
  status.phase = 'error';
  console.log('[web] error:', err);
} finally {
  write();
  await browser.close();
  for (const sock of proxies) sock.close();
  if (IMPAIR) console.log(`[web] impairment ${IMPAIR}: ${impairStats.rtp} RTP packets through the proxy, ${impairStats.dropped} dropped`);
  for (const [key, r] of Object.entries(status.scenarios)) {
    if (r.maxLevel === undefined) console.log(`[web] ${key}: ${JSON.stringify(r)}`);
    else console.log(`[web] ${key}: connected=${r.connected} received ${r.maxBytesReceived} B / ${r.maxPacketsReceived} packets from native, max level ${r.maxLevel.toFixed(3)}, ${r.mime} fmtp: ${r.fmtp}`);
  }
  process.exit(status.error ? 1 : 0);
}
