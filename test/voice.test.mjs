// Voice logic that needs no browser: SDP tuning, codec order (RED) and the transmit gate.
//   node test/voice.test.mjs
import assert from 'node:assert/strict';
import { orderAudioCodecs, orderCodecs, SCREEN_QUALITY, tuneOpus } from '../web/js/voice.js';
import { settings } from '../web/js/settings.js';
import { Gate, GATE } from '../web/js/gate-worklet.js';

const cases = [];
const t = (name, fn) => cases.push([name, fn]);

// What Chrome offers for a voice call that later gained a screen share (voice, tab audio, video).
const CRLF = '\r\n';
const OFFER = [
  'v=0', 'o=- 1 2 IN IP4 127.0.0.1', 's=-', 't=0 0', 'a=group:BUNDLE 0 1 2', 'a=msid-semantic: WMS',
  'm=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126',
  'c=IN IP4 0.0.0.0', 'a=mid:0', 'a=sendrecv',
  'a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level',
  'a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01',
  'a=rtpmap:111 opus/48000/2', 'a=rtcp-fb:111 transport-cc', 'a=fmtp:111 minptime=10;useinbandfec=1',
  'a=rtpmap:63 red/48000/2', 'a=fmtp:63 111/111',
  'm=audio 9 UDP/TLS/RTP/SAVPF 111 63',
  'c=IN IP4 0.0.0.0', 'a=mid:1', 'a=sendrecv',
  'a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level',
  'a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01',
  'a=rtpmap:111 opus/48000/2', 'a=rtcp-fb:111 transport-cc', 'a=fmtp:111 minptime=10;useinbandfec=1',
  'm=video 9 UDP/TLS/RTP/SAVPF 96',
  'c=IN IP4 0.0.0.0', 'a=mid:2', 'a=sendrecv',
  'a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01',
  'a=rtpmap:96 VP8/90000', 'a=rtcp-fb:96 transport-cc', 'a=rtcp-fb:96 nack',
  '',
].join(CRLF);

const sections = (sdp) => sdp.split(`${CRLF}m=`).slice(1);
const fmtp = (section) => Object.fromEntries(section.match(/^a=fmtp:111 (.*)$/m)[1].split(';').map((kv) => kv.split('=')));

t('every audio section gets the Opus tuning (screen audio too: B4)', () => {
  const [voice, screen] = sections(tuneOpus(OFFER));
  for (const s of [voice, screen]) {
    const p = fmtp(s);
    assert.equal(p.stereo, '1');
    assert.equal(p.useinbandfec, '1');
    assert.equal(p.usedtx, '0');
    assert.equal(p.maxplaybackrate, '48000');
  }
  assert.equal(fmtp(voice).maxaveragebitrate, String(settings.bitrate));
  assert.ok(Number(fmtp(screen).maxaveragebitrate) >= 160000, 'screen audio is at least 160 kbps');
});

t('sprop-stereo: the voice follows music mode; screen audio is stereo where we send it', () => {
  settings.music = false;
  let [voice, screen] = sections(tuneOpus(OFFER));
  assert.equal(fmtp(voice)['sprop-stereo'], '0');
  assert.equal(fmtp(screen)['sprop-stereo'], '1', 'sendrecv screen audio = ours');
  [voice, screen] = sections(tuneOpus(OFFER.replace('a=mid:1\r\na=sendrecv', 'a=mid:1\r\na=recvonly')));
  assert.equal(fmtp(screen)['sprop-stereo'], '0', "a screen we only watch isn't ours to describe");
  settings.music = true;
  [voice] = sections(tuneOpus(OFFER));
  assert.equal(fmtp(voice)['sprop-stereo'], '1');
  settings.music = false;
});

t('transport-cc is stripped from audio only (B5); video keeps it', () => {
  const [voice, screen, video] = sections(tuneOpus(OFFER));
  for (const s of [voice, screen]) {
    assert.doesNotMatch(s, /transport-cc/);
    assert.doesNotMatch(s, /transport-wide-cc/);
    assert.match(s, /ssrc-audio-level/, 'speaking indicators keep their header extension');
  }
  assert.match(video, /a=rtcp-fb:96 transport-cc/);
  assert.match(video, /transport-wide-cc/);
});

t('tuning is idempotent, keeps other lines and line endings', () => {
  const once = tuneOpus(OFFER);
  assert.equal(tuneOpus(once), once);
  assert.ok(once.endsWith(CRLF));
  assert.match(once, /a=fmtp:63 111\/111/);
  assert.match(once, /^a=group:BUNDLE 0 1 2$/m);
  const lf = tuneOpus(OFFER.replaceAll(CRLF, '\n'));
  assert.ok(!lf.includes('\r'), 'LF-only SDP stays LF-only');
});

// ── RED on the voice (PROTOCOL.md › Voice › Redundancy) ───────────────────
// What Chrome lists in RTCRtpReceiver.getCapabilities('audio') (153), in its order.
const AUDIO_CAPS = [
  { mimeType: 'audio/opus', clockRate: 48000, channels: 2, sdpFmtpLine: 'minptime=10;useinbandfec=1' },
  { mimeType: 'audio/red', clockRate: 48000, channels: 2 },
  { mimeType: 'audio/G722', clockRate: 8000 }, { mimeType: 'audio/PCMU', clockRate: 8000 }, { mimeType: 'audio/PCMA', clockRate: 8000 },
  { mimeType: 'audio/CN', clockRate: 8000 }, { mimeType: 'audio/telephone-event', clockRate: 48000 }, { mimeType: 'audio/telephone-event', clockRate: 8000 },
];

t('RED: the voice lists RED first, then Opus, then everything else in the browser\'s order', () => {
  const order = orderAudioCodecs(AUDIO_CAPS, { red: true });
  assert.deepEqual(order.map((c) => c.mimeType), ['audio/red', 'audio/opus', 'audio/G722', 'audio/PCMU', 'audio/PCMA', 'audio/CN', 'audio/telephone-event', 'audio/telephone-event']);
  assert.equal(order.length, AUDIO_CAPS.length, 'nothing dropped');
  assert.equal(AUDIO_CAPS[0].mimeType, 'audio/opus', 'the capabilities list itself is left alone');
});

t('RED: a screen share\'s sound (and the voice with ?red=0) is Opus first, without RED', () => {
  const order = orderAudioCodecs(AUDIO_CAPS, { red: false });
  assert.equal(order[0].mimeType, 'audio/opus');
  assert.ok(!order.some((c) => c.mimeType === 'audio/red'));
  assert.equal(order.length, AUDIO_CAPS.length - 1, 'only RED goes');
  // A browser without RED: Opus first either way.
  assert.equal(orderAudioCodecs(AUDIO_CAPS.filter((c) => c.mimeType !== 'audio/red'), { red: true })[0].mimeType, 'audio/opus');
});

t('RED: tuneOpus keeps RED first and its fmtp, tunes the Opus line inside it, strips transport-cc', () => {
  const red = OFFER.replace('m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126', 'm=audio 9 UDP/TLS/RTP/SAVPF 63 111 9 0 8 13 110 126');
  const [voice, screen] = sections(tuneOpus(red));
  assert.match(voice, /^audio 9 UDP\/TLS\/RTP\/SAVPF 63 111 9 0 8 13 110 126\r\n/, 'the m= line order is the browser\'s (setCodecPreferences)');
  assert.match(voice, /a=rtpmap:63 red\/48000\/2\r\na=fmtp:63 111\/111/, 'one earlier frame (distance 1)');
  const p = fmtp(voice);
  assert.equal(p.maxaveragebitrate, String(settings.bitrate), 'Opus keeps the quality setting');
  assert.equal(p.useinbandfec, '1');
  assert.equal(p.usedtx, '0');
  assert.doesNotMatch(voice, /transport-cc|transport-wide-cc/);
  assert.match(screen, /^audio 9 UDP\/TLS\/RTP\/SAVPF 111 63\r\n/, 'the screen\'s sound is untouched');
  assert.equal(tuneOpus(tuneOpus(red)), tuneOpus(red), 'idempotent');
});

t('a listen-only page (OBS) asks for 10 ms packets on the voice only; nobody else asks for any', () => {
  const red = OFFER.replace('m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126', 'm=audio 9 UDP/TLS/RTP/SAVPF 63 111 9 0 8 13 110 126');
  const tuned = tuneOpus(red, { ptime: 10 });
  const [voice, screen, video] = sections(tuned);
  assert.match(voice, /a=fmtp:111 [^\r]*\r\na=ptime:10\r\n/, 'right after the Opus parameters');
  assert.equal(voice.match(/^a=ptime:/gm).length, 1);
  assert.doesNotMatch(screen, /a=ptime/);
  assert.doesNotMatch(video, /a=ptime/);
  assert.equal(fmtp(voice).maxaveragebitrate, String(settings.bitrate), 'the rest of the tuning as always');
  assert.equal(tuneOpus(tuned, { ptime: 10 }), tuned, 'idempotent');
  assert.equal(sections(tuneOpus(red.replace('a=fmtp:111 minptime=10;useinbandfec=1\r\na=rtpmap:63', 'a=ptime:20\r\na=fmtp:111 minptime=10;useinbandfec=1\r\na=rtpmap:63'), { ptime: 10 }))[0].match(/^a=ptime:\d+/gm).join(), 'a=ptime:10', 'one ptime, ours');
  assert.doesNotMatch(tuneOpus(red), /a=ptime/, 'a call page leaves the packet size to the browser (20 ms)');
});

t('an Opus line without fmtp gets one right after its rtpmap', () => {
  const bare = OFFER.replace('a=fmtp:111 minptime=10;useinbandfec=1\r\na=rtpmap:63', 'a=rtpmap:63');
  const [voice] = sections(tuneOpus(bare));
  assert.match(voice, /a=rtpmap:111 opus\/48000\/2\r\na=fmtp:111 minptime=10;useinbandfec=1;usedtx=0;stereo=1/);
});

// ── Screen sharing presets (PROTOCOL.md › Screen sharing) ─────────────────
// What Chrome lists in RTCRtpReceiver.getCapabilities('video'), in its order.
const H264 = (pm, id) => ({ mimeType: 'video/H264', sdpFmtpLine: `level-asymmetry-allowed=1;packetization-mode=${pm};profile-level-id=${id}` });
const CAPS = [
  { mimeType: 'video/VP8' }, { mimeType: 'video/rtx' }, { mimeType: 'video/VP9', sdpFmtpLine: 'profile-id=0' },
  H264(1, '42001f'), H264(0, '42001f'), H264(1, '42e01f'), H264(0, '42e01f'), H264(1, '4d001f'),
  { mimeType: 'video/AV1' }, { mimeType: 'video/red' }, { mimeType: 'video/ulpfec' },
];

t('screen presets: the same numbers the native apps use (Smooth 1080p60 8 Mbps, Sharp 1440p30 6 Mbps, Max 4K60 25 Mbps)', () => {
  const pick = ({ label, w, h, fps, bitrate }) => ({ label, w, h, fps, bitrate });
  assert.deepEqual(Object.values(SCREEN_QUALITY).map(pick), [
    { label: 'Smooth', w: 1920, h: 1080, fps: 60, bitrate: 8_000_000 },
    { label: 'Sharp', w: 2560, h: 1440, fps: 30, bitrate: 6_000_000 },
    { label: 'Max', w: 3840, h: 2160, fps: 60, bitrate: 25_000_000 },
  ]);
});

t('screen presets: their codec comes first, nothing is dropped, so H.264 constrained baseline (packetization-mode=1) is always offered', () => {
  for (const [name, q] of Object.entries(SCREEN_QUALITY)) {
    const order = orderCodecs(CAPS, q.codecs);
    assert.equal(order.length, CAPS.length, `${name}: every codec stays`);
    assert.equal(order[0].mimeType, `video/${q.codecs[0]}`, `${name}: ${q.codecs[0]} first`);
    assert.ok(order.some((c) => c.mimeType === 'video/H264' && /profile-level-id=42e01f/.test(c.sdpFmtpLine) && /packetization-mode=1/.test(c.sdpFmtpLine)), `${name}: H.264 42e01f pm=1`);
    // Codecs the preset doesn't name keep the browser's order (rtx, red, FEC, AV1…).
    const rest = order.filter((c) => !q.codecs.some((n) => c.mimeType === `video/${n}`));
    assert.deepEqual(rest, CAPS.filter((c) => rest.includes(c)));
  }
  assert.deepEqual(orderCodecs(CAPS, ['H264']).slice(0, 5).map((c) => c.sdpFmtpLine), CAPS.filter((c) => c.mimeType === 'video/H264').map((c) => c.sdpFmtpLine), 'H.264 profiles keep their own order');
});

// ── The transmit gate (web/js/gate-worklet.js), run on synthetic audio ──
const SR = 48000;
const ms = (v) => Math.round((v / 1000) * SR);
let seed = 3;
const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) * 2 - 1;
const dbToAmp = (db) => 10 ** (db / 20);
/** White noise at an RMS level. */
const noise = (n, db) => Float32Array.from({ length: n }, () => rnd() * dbToAmp(db) * Math.sqrt(3));
/** A "syllable": a 220 Hz buzz at `db` RMS with a `rise` ms attack, starting at sample `at`. */
function syllable(x, at, lenMs, db, rise = 20) {
  const len = ms(lenMs);
  for (let i = 0; i < len && at + i < x.length; i++) {
    const env = Math.min(1, i / ms(rise)) * Math.min(1, (len - i) / ms(10));
    x[at + i] += Math.sin((2 * Math.PI * 220 * i) / SR) * dbToAmp(db) * Math.SQRT2 * env;
  }
}
/** Run audio through a Gate in 128-sample blocks: the output, the gain each input sample went out at, and the reports. */
function run(x, opts = {}, events = []) {
  const g = new Gate(SR, opts);
  const out = new Float32Array(x.length);
  const gainOf = new Float32Array(x.length); // gain applied to *input* sample k (delay accounted)
  const ones = new Float32Array(128).fill(1);
  const reports = [];
  const delay = opts.mode === 'vad' || !opts.mode ? ms(GATE.preRollMs) : 0;
  const gains = new Float32Array(x.length + delay + 128);
  for (let i = 0; i < x.length + delay; i += 128) {
    for (const e of events) if (e.at >= i && e.at < i + 128) g.set(e.set);
    const block = new Float32Array(128);
    block.set(x.subarray(Math.min(i, x.length), Math.min(i + 128, x.length)));
    const o = [new Float32Array(128), new Float32Array(128)];
    const r = g.process([block, ones], o);
    if (r) reports.push({ ...r, t: i });
    if (i < x.length) out.set(o[0].subarray(0, Math.min(128, x.length - i)), i);
    gains.set(o[1], i);
  }
  for (let k = 0; k < x.length; k++) gainOf[k] = gains[k + delay];
  return { out, gainOf, gains, reports, gate: g };
}

t('gate: voice activity opens before the first syllable (pre-roll) and holds ~450 ms', () => {
  const x = noise(SR * 3, -75);
  const at = ms(1200);
  syllable(x, at, 300, -30);
  const { gainOf } = run(x, { mode: 'vad', threshold: -50 });
  assert.ok(gainOf[at] >= 0.99, `the syllable's very first sample goes out at full volume (gain ${gainOf[at]})`);
  assert.equal(gainOf[at - ms(100)], 0, 'closed well before it');
  const end = at + ms(300);
  assert.ok(gainOf[end + ms(400)] > 0.99, 'still open 400 ms after (hold)');
  assert.equal(gainOf[end + ms(GATE.holdMs + GATE.fadeOutMs + 30)], 0, 'closed after the hold and fade');
});

t('gate: soft attacks are not clipped (onset clipping 0 ms on 20 syllables)', () => {
  const x = noise(SR * 12, -70);
  const starts = Array.from({ length: 20 }, (_, i) => ms(400 + i * 560));
  // Quiet-ish speech (-34…-42 dBFS) with slow attacks (10–40 ms), just over the -50 sensitivity.
  for (const [i, at] of starts.entries()) syllable(x, at, 120, -34 - (i % 3) * 4, 10 + (i % 4) * 10);
  const { gainOf } = run(x, { mode: 'vad', threshold: -50 });
  // Ground truth onset: where the syllable passes -60 dBFS (what a listener would notice).
  const clipped = starts.map((at) => {
    let k = at;
    while (Math.abs(x[k]) < dbToAmp(-60) && k < at + ms(80)) k++;
    let first = k;
    while (gainOf[first] < 0.5 && first < at + ms(200)) first++;
    return (first - k) / 48;
  });
  assert.ok(Math.max(...clipped) === 0, `clipped ms per syllable: ${clipped.join(', ')}`);
});

t('gate: hysteresis — a level between close and open keeps it open but never opens it', () => {
  const x = noise(SR * 4, -75);
  syllable(x, ms(500), 1500, -53, 1); // between -56 (close) and -50 (open)
  let r = run(x, { mode: 'vad', threshold: -50 });
  assert.equal(r.gainOf.reduce((m, g) => Math.max(m, g), 0), 0, "never louder than the sensitivity: doesn't open");
  const y = noise(SR * 4, -75);
  syllable(y, ms(500), 100, -40, 1); // opens it…
  syllable(y, ms(600), 1500, -53, 1); // …then sits under the open level but over the close level
  r = run(y, { mode: 'vad', threshold: -50 });
  assert.ok(r.gainOf[ms(1900)] > 0.99, 'held open by hysteresis well past the 450 ms hold');
});

t('gate: fades in and out (no clicks)', () => {
  const x = new Float32Array(SR * 2).fill(0.25);
  const { gains } = run(x, { mode: 'ptt', ptt: false }, [{ at: ms(500), set: { ptt: true } }, { at: ms(1000), set: { ptt: false } }]);
  let maxStep = 0;
  for (let i = 1; i < gains.length; i++) maxStep = Math.max(maxStep, Math.abs(gains[i] - gains[i - 1]));
  assert.ok(maxStep <= 1 / ms(GATE.fadeInMs) + 1e-6, `largest gain step ${maxStep}`);
});

t('gate: push-to-talk opens at once (no pre-roll delay) and keeps a ~200 ms tail', () => {
  const x = new Float32Array(SR * 2).fill(0.25);
  const press = ms(500);
  const release = ms(1000);
  const { out } = run(x, { mode: 'ptt' }, [{ at: press, set: { ptt: true } }, { at: release, set: { ptt: false } }]);
  assert.equal(out[press - 200], 0, 'silent before the key');
  assert.ok(out[press + ms(GATE.fadeInMs) + 128] > 0.24, 'full volume within a block + the 5 ms fade');
  assert.ok(out[release + ms(150)] > 0.24, 'still sending 150 ms after release');
  assert.equal(out[release + ms(GATE.pttTailMs + GATE.fadeOutMs) + 256], 0, 'closed after the tail');
});

t('gate: a mic test makes push-to-talk act like always-on; always-on has no delay', () => {
  const x = new Float32Array(SR).fill(0.25);
  let { out } = run(x, { mode: 'ptt', test: true });
  assert.ok(out[ms(10)] > 0.24);
  ({ out } = run(x, { mode: 'open' }));
  assert.ok(out[1] > 0.24, 'always-on starts open and undelayed');
});

t('gate: adaptive noise floor — a fan at -52 dBFS stays out, a soft voice over it gets in', () => {
  const x = noise(SR * 20, -52);
  const quiet = run(x, { mode: 'vad', threshold: -50 }).gainOf;
  const openPct = (100 * quiet.reduce((s, g) => s + (g > 0.5 ? 1 : 0), 0)) / quiet.length;
  assert.ok(openPct < 10, `open ${openPct.toFixed(1)}% of the time on fan noise alone (a fixed -50 gate: ~100%)`);
  const y = noise(SR * 6, -52);
  syllable(y, ms(4500), 400, -34, 20);
  const r = run(y, { mode: 'vad', threshold: -50 });
  assert.ok(r.gainOf[ms(4500) + ms(50)] > 0.99, 'speech 18 dB over the noise opens it');
  assert.ok(r.reports.at(-1).floor > -60 && r.reports.at(-1).floor < -50, `floor estimate ${r.reports.at(-1).floor}`);
});

t('gate: reports level every 20 ms, and digital silence after 5 s (not a quiet room)', () => {
  const zeros = new Float32Array(SR * 6);
  const r = run(zeros, { mode: 'open' });
  assert.ok(Math.abs(r.reports.length - 300) <= 2, `${r.reports.length} reports in 6 s`);
  assert.equal(r.reports.find((x) => x.t < ms(4900))?.silent, false);
  assert.equal(r.reports.at(-1).silent, true);
  const room = run(noise(SR * 6, -90), { mode: 'open' });
  assert.equal(room.reports.at(-1).silent, false, 'a very quiet but real mic is not "silence"');
  assert.ok(Math.abs(room.reports.at(-1).db + 90) < 1.5, `level ${room.reports.at(-1).db} dBFS`);
});

t('gate: stereo (music mode) gates both channels alike', () => {
  const g = new Gate(SR, { mode: 'open' });
  const l = new Float32Array(128).fill(0.5);
  const r = new Float32Array(128).fill(-0.25);
  const out = [new Float32Array(128), new Float32Array(128)];
  g.process([l, r], out);
  assert.equal(out[0][100], 0.5);
  assert.equal(out[1][100], -0.25);
});

let failed = 0;
for (const [name, fn] of cases) {
  try {
    await fn();
    console.log(`✓ ${name}`);
  } catch (err) {
    failed++;
    console.log(`✗ ${name}\n    ${err.stack}`);
  }
}
console.log(`\n${cases.length - failed}/${cases.length} passed`);
process.exit(failed ? 1 : 0);
