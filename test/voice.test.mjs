// Voice logic that needs no browser: SDP tuning, codec order (RED) and the transmit gate.
//   node test/voice.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { orderAudioCodecs, orderCodecs, OwnConnection, panGains, SCREEN_QUALITY, tuneOpus } from '../web/js/voice.js';
import { PAN, voicePan } from '../web/js/space-map.js';
import { settings } from '../web/js/settings.js';
import { CLIP, ClipWatch, Gate, GATE, limit, LoudPass, LOUD_PASS, MutedReminder, MUTED_REMINDER, PttLatch, PTT_LATCH_MS, RNNOISE_DELAY } from '../web/js/gate-worklet.js';

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

t('nobody asks for a packet size: the browser\'s 20 ms, OBS links included (10 ms went with v0.45.0)', () => {
  const red = OFFER.replace('m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126', 'm=audio 9 UDP/TLS/RTP/SAVPF 63 111 9 0 8 13 110 126');
  assert.doesNotMatch(tuneOpus(red), /a=ptime/);
  assert.equal(tuneOpus.length, 1, 'one argument: the description');
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

t('gate: says when you talk in every mode (the "You\'re muted" reminder), not only in voice activity', () => {
  const x = noise(SR * 3, -75);
  syllable(x, ms(1200), 300, -30);
  for (const mode of ['vad', 'open', 'ptt']) {
    const { reports } = run(x, { mode, threshold: -50 });
    const talking = reports.filter((r) => r.voice).map((r) => r.t);
    assert.ok(talking.length, `${mode}: talking is reported`);
    assert.ok(talking[0] >= ms(1150) && talking[0] <= ms(1260), `${mode}: from the syllable on (${talking[0] / 48} ms)`);
    assert.ok(talking.at(-1) <= ms(1500 + GATE.holdMs + 40), `${mode}: and its hold, then quiet again (${talking.at(-1) / 48} ms)`);
    assert.ok(!reports.some((r) => r.voice && r.t < ms(1100)), `${mode}: never in the quiet room before`);
  }
  // Always-on sends everything, but only actual talking counts as talking.
  const quiet = run(noise(SR * 2, -75), { mode: 'open' });
  assert.ok(quiet.reports.every((r) => r.open && !r.voice), 'always-on: open, but nobody talking');
});

t('muted reminder: at most once every 5 s, never in the first second after muting, the blip only the first time', () => {
  const r = new MutedReminder();
  r.muted(0);
  assert.equal(r.due(500), false, 'still finishing a word as you mute');
  assert.equal(r.due(MUTED_REMINDER.graceMs), 'blip', 'then the first reminder, with the blip');
  assert.equal(r.due(MUTED_REMINDER.graceMs + 20), false);
  assert.equal(r.due(MUTED_REMINDER.graceMs + 4999), false, 'not again within 5 s');
  assert.equal(r.due(MUTED_REMINDER.graceMs + 5000), 'quiet', 'again after 5 s, just the words');
  assert.equal(r.due(MUTED_REMINDER.graceMs + 10_000), 'quiet');
  r.muted(20_000);
  assert.equal(r.due(20_400), false, 'muting again starts the grace again');
  assert.equal(r.due(21_000), 'blip', 'and the blip');
  assert.equal(MUTED_REMINDER.everyMs, 5000, 'the spec says 5 s');
});

t('gate: reports push-to-talk open until the tail ends (the closing click), never in a mic test', () => {
  const x = new Float32Array(SR * 2).fill(0.25);
  const press = ms(500);
  const release = ms(800);
  const { reports } = run(x, { mode: 'ptt' }, [{ at: press, set: { ptt: true } }, { at: release, set: { ptt: false } }]);
  const open = reports.filter((r) => r.pttOpen).map((r) => r.t);
  assert.ok(open[0] >= press - 128 && open[0] <= press + ms(25), `opens with the key (${open[0] / 48} ms)`);
  const closed = open.at(-1);
  assert.ok(closed >= release + ms(GATE.pttTailMs) - ms(25) && closed <= release + ms(GATE.pttTailMs) + ms(25), `closes after the 200 ms tail (${(closed - release) / 48} ms after release)`);
  assert.ok(run(x, { mode: 'ptt', test: true }, [{ at: press, set: { ptt: true } }]).reports.every((r) => !r.pttOpen), 'a mic test: never');
  assert.ok(run(x, { mode: 'open' }).reports.every((r) => !r.pttOpen), 'always-on: never');
});

t('push-to-talk double-tap: two presses within 400 ms stay live; the next press ends it (and is an ordinary press)', () => {
  const p = new PttLatch();
  p.down(0);
  p.up();
  assert.equal(p.open, false, 'one tap: closed after it');
  p.down(PTT_LATCH_MS);
  assert.equal(p.latched, true, 'second press 400 ms after the first: latched');
  p.up();
  assert.equal(p.open, true, 'stays open with the key up');
  p.down(5000);
  assert.equal(p.latched, false, 'the next press ends the latch…');
  assert.equal(p.open, true, '…but is held, so nothing is cut off');
  p.up();
  assert.equal(p.open, false);
  p.down(5200);
  p.up();
  assert.equal(p.latched, false, 'the press that ended it never counts as the first of a double-tap');
  const slow = new PttLatch();
  slow.down(0);
  slow.up();
  slow.down(PTT_LATCH_MS + 1);
  assert.equal(slow.latched, false, '401 ms apart: no');
  const long = new PttLatch();
  long.down(0);
  long.up();
  long.down(2000);
  assert.equal(long.latched, false, 'a long hold then a press: no');
  long.reset();
  assert.deepEqual([long.open, long.latched], [false, false]);
});

t('your own connection: shown when most people lose > 3 % of yours three rounds in a row; gone after three good ones', () => {
  const own = new OwnConnection();
  const bad = { fraction: 0.08, rtt: 40 };
  const good = { fraction: 0.01, rtt: 40 };
  assert.equal(own.round([bad]), false);
  assert.equal(own.round([bad]), false);
  assert.equal(own.round([bad]), true, 'the only one, three rounds: shown');
  assert.deepEqual(own.weak, { lost: 0.08, rtt: 40 });
  assert.equal(own.round([good]), false);
  assert.equal(own.round([good]), false, 'still shown after two good rounds');
  assert.equal(own.round([good]), true, 'gone after three');
  assert.equal(own.weak, null);
  // Two of three people losing it: most. One of two: not most (it's probably that person's network).
  for (let i = 0; i < 3; i++) own.round([bad, bad, good]);
  assert.ok(own.weak, '2 of 3');
  const half = new OwnConnection();
  for (let i = 0; i < 5; i++) half.round([bad, good]);
  assert.equal(half.weak, null, '1 of 2 is not most');
  // Exactly 3 % isn't "more than 3 %"; no reports at all (alone, or none yet) clears it.
  const edge = new OwnConnection();
  for (let i = 0; i < 4; i++) edge.round([{ fraction: 0.03, rtt: null }]);
  assert.equal(edge.weak, null);
  for (let i = 0; i < 3; i++) edge.round([bad]);
  assert.ok(edge.weak);
  assert.equal(edge.round([]), true, 'alone again: gone');
  assert.equal(edge.weak, null);
  assert.equal(edge.round([{ fraction: null }]), false, 'a report without a fraction is not judged');
});

t('Space left/right: pan = dx / 400 (clamped), the middle within 60; the far ear never under 1/4, both full in the middle', () => {
  assert.equal(voicePan(0, 0), 0);
  assert.equal(voicePan(59, 0), 0, 'within 60: the middle');
  assert.equal(voicePan(40, 40), 0, '(by distance, not just across)');
  assert.equal(voicePan(200, 0), 0.5);
  assert.equal(voicePan(-400, 300), -1);
  assert.equal(voicePan(2000, 0), 1, 'clamped');
  assert.equal(voicePan(100, 900), 0.25, 'far up or down the screen: only across counts');
  assert.deepEqual([PAN.width, PAN.middle], [400, 60], 'the spec\'s numbers');
  assert.deepEqual(panGains(0), [1, 1], 'the middle: both ears full');
  assert.deepEqual(panGains(1), [0.25, 1], 'all the way right: the left ear at 1/4');
  assert.deepEqual(panGains(-1), [1, 0.25]);
  assert.deepEqual(panGains(0.5), [0.625, 1]);
  assert.deepEqual(panGains(-7), [1, 0.25], 'clamped');
  for (let p = -1; p <= 1; p += 0.05) {
    const [l, r] = panGains(p);
    assert.ok(Math.min(l, r) / Math.max(l, r) >= 0.25 - 1e-9 && Math.max(l, r) === 1, `pan ${p.toFixed(2)}: near ear full, far ≥ 1/4`);
  }
});

t('mic meter: each report carries its loudest sample (dBFS); "Too loud (clipping)" after three at −1 dBFS or more within 2 s, for 3 s', () => {
  const x = noise(SR, -40);
  x[ms(500)] = 0.95; // −0.4 dBFS, once
  const { reports } = run(x, { mode: 'open' });
  const loud = reports.filter((r) => r.peak >= -1);
  assert.equal(loud.length, 1, 'one report with the pop in it');
  assert.ok(Math.abs(loud[0].peak - 20 * Math.log10(0.95)) < 0.01, `its peak ${loud[0].peak.toFixed(2)} dBFS`);
  assert.ok(reports.filter((r) => r.peak < -1).every((r) => r.peak < -25), 'the rest: the quiet room');
  const w = new ClipWatch();
  assert.equal(w.push(-0.5, 0), false, 'one pop: no');
  assert.equal(w.push(-0.2, 500), false);
  assert.equal(w.push(-6, 900), false, 'a loud word that stays under −1 dBFS: no');
  assert.equal(w.push(0, 1500), true, 'the third within 2 s: clipping');
  assert.equal(w.push(-20, 4400), true, 'shown for 3 s after…');
  assert.equal(w.push(-20, 4600), false, '…then gone');
  const spread = new ClipWatch();
  for (const at of [0, 1500, 3000, 4500]) assert.equal(spread.push(-0.5, at), false, 'one every 1.5 s is never three in 2 s');
  assert.equal(CLIP.db, -1);
});

t('mic volume: 100 % leaves every sample exactly as it was; 50 % is 6 dB down in the level, voice activity and what goes out', () => {
  const x = noise(SR * 2, -70);
  syllable(x, ms(500), 600, -30);
  const plain = run(x, { mode: 'open' });
  const at100 = run(x, { mode: 'open', volume: 1 });
  assert.ok(plain.out.every((v, i) => Object.is(v, at100.out[i])), 'bit-identical at 100 %');
  const half = run(x, { mode: 'open', volume: 0.5 });
  const rms = (a, from, to) => Math.sqrt(a.subarray(from, to).reduce((s, v) => s + v * v, 0) / (to - from));
  const drop = 20 * Math.log10(rms(half.out, ms(600), ms(1000)) / rms(plain.out, ms(600), ms(1000)));
  assert.ok(Math.abs(drop + 6.02) < 0.05, `sent ${drop.toFixed(2)} dB`);
  const loud = (r) => Math.max(...r.reports.filter((p) => p.t > ms(600) && p.t < ms(1000)).map((p) => p.db));
  assert.ok(Math.abs(loud(half) - loud(plain) + 6.02) < 0.1, 'the meter hears it too');
  // A syllable just over the sensitivity opens voice activity at 100 %, not at 50 %.
  const y = noise(SR * 2, -80);
  syllable(y, ms(500), 300, -46);
  assert.ok(run(y, { mode: 'vad', threshold: -50 }).gainOf.some((g) => g > 0.99), 'opens at 100 %');
  assert.equal(run(y, { mode: 'vad', threshold: -50, volume: 0.5 }).gainOf.reduce((m, g) => Math.max(m, g), 0), 0, 'not at 50 %');
  // Live, without a new gate.
  const live = run(x, { mode: 'open' }, [{ at: ms(800), set: { volume: 0.5 } }]);
  assert.ok(rms(live.out, ms(900), ms(1000)) < rms(live.out, ms(600), ms(700)) * 0.6, 'turned down mid-stream');
});

t('mic volume past 100 %: a soft limiter, never past 0 dBFS; "peak" is still the mic\'s own, "boost" what the volume made of it', () => {
  assert.equal(limit(0.5), 0.5, 'straight through below −1 dBFS');
  assert.equal(limit(-0.8), -0.8);
  for (const v of [0.95, 1.2, 1.5]) assert.ok(limit(v) < 1 && limit(v) > limit(v * 0.99) && limit(-v) === -limit(v), `soft, below 1, still rising: ${v}`);
  for (const v of [3, 10, 1e6]) assert.ok(limit(v) <= 1 && limit(-v) === -limit(v), `never past full scale: ${v}`);
  const x = noise(SR, -70);
  syllable(x, ms(200), 600, -6); // peaks near −3 dBFS
  const r = run(x, { mode: 'open', volume: 2 });
  assert.ok(r.out.every((v) => Math.abs(v) <= 1), 'never past full scale');
  const loudest = r.reports.filter((p) => p.t > ms(300) && p.t < ms(700));
  assert.ok(loudest.every((p) => p.peak < -2), `the mic's own peaks (${loudest[0].peak.toFixed(1)} dBFS): not clipping`);
  assert.ok(loudest.every((p) => p.boost > -1), `doubled, they reach the limiter (${loudest[0].boost.toFixed(1)} dBFS)`);
  assert.ok(run(x, { mode: 'open' }).reports.every((p) => p.boost === -120), 'at 100 % there is no boost to report');
});

// ── Noise suppression: loud, lasting sounds get past RNNoise (LoudPass) ──

/** A seeded generator, so every run hears the same noise. */
function rng(seed) {
  let a = seed >>> 0;
  const u = () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  return () => Math.sqrt(-2 * Math.log(u() || 1e-12)) * Math.cos(2 * Math.PI * u());
}
const dbOf = (x, from = 0, to = x.length) => {
  let s = 0;
  for (let i = from; i < to; i++) s += x[i] * x[i];
  return 10 * Math.log10(s / Math.max(1, to - from) + 1e-12);
};
/** Pink noise (a fan, a room) at `db` dBFS RMS. */
function pink(n, db, seed = 1) {
  const g = rng(seed);
  const x = new Float32Array(n);
  let b0 = 0, b1 = 0, b2 = 0, b3 = 0, b4 = 0, b5 = 0, b6 = 0;
  for (let i = 0; i < n; i++) {
    const w = g();
    b0 = 0.99886 * b0 + w * 0.0555179; b1 = 0.99332 * b1 + w * 0.0750759; b2 = 0.969 * b2 + w * 0.153852;
    b3 = 0.8665 * b3 + w * 0.3104856; b4 = 0.55 * b4 + w * 0.5329522; b5 = -0.7616 * b5 - w * 0.016898;
    x[i] = b0 + b1 + b2 + b3 + b4 + b5 + b6 + w * 0.5362;
    b6 = w * 0.115926;
  }
  const k = 10 ** (db / 20) / 10 ** (dbOf(x) / 20);
  return x.map((v) => v * k);
}
/** Bursts of a voiced sound: pitch gliding f0 → f1, `harmonics` of them, a little breath. */
function voiced({ f0, f1, burst, gap, bursts, amp, harmonics, breath, seed = 3 }) {
  const g = rng(seed);
  const out = [...new Float32Array(SR / 10)];
  let ph = 0;
  for (let b = 0; b < bursts; b++) {
    const L = Math.round(burst * SR);
    for (let i = 0; i < L; i++) {
      const t = i / L;
      ph += (2 * Math.PI * (f0 + (f1 - f0) * t + 30 * Math.sin((2 * Math.PI * 7 * i) / SR))) / SR;
      let v = 0;
      for (let k = 1; k <= harmonics; k++) v += Math.sin(k * ph) / k ** 1.2;
      out.push(amp * Math.sqrt(Math.sin(Math.PI * t)) * (v * 0.5 + breath * g()));
    }
    for (let i = 0; i < Math.round(gap * SR); i++) out.push(amp * 0.02 * g());
  }
  return Float32Array.from(out);
}
/** A hard, high-pitched laugh (a squeal, 1–1.2 kHz): what RNNoise guts. */
const squeal = () => voiced({ f0: 1200, f1: 1000, burst: 0.2, gap: 0.04, bursts: 16, amp: 0.3, harmonics: 6, breath: 0.05 });
/** "Ha ha ha" low down (260 → 220 Hz): what RNNoise keeps. */
const ha = () => voiced({ f0: 260, f1: 220, burst: 0.12, gap: 0.08, bursts: 20, amp: 0.35, harmonics: 10, breath: 0.15 });
/** Typing: 6 ms clicks about every 120 ms over a quiet room. */
function typing(n) {
  const x = pink(n, -60, 7);
  const g = rng(9);
  for (let at = 0.3 * SR; at < n - SR / 10; at += 0.12 * SR + (g() * 0.015 * SR) | 0) {
    for (let j = 0; j < 0.006 * SR; j++) x[at + j] += g() * Math.exp(-j / (0.0012 * SR)) * 0.25;
  }
  return x;
}

/** Run a LoudPass over `dry` and `wet`, 128 samples at a time (the worklet's blocks); also how often it passed. */
function loudPass(dry, wet, delay = RNNOISE_DELAY) {
  const p = new LoudPass(SR, delay);
  const y = new Float32Array(dry.length);
  let passed = 0;
  for (let i = 0; i + 128 <= dry.length; i += 128) {
    y.set(p.process(wet.subarray(i, i + 128), dry.subarray(i, i + 128)), i);
    if (p.passing) passed++;
  }
  return { y, passed };
}
const late = (x, d) => { const y = new Float32Array(x.length); y.set(x.subarray(0, x.length - d), d); return y; };

t('loud pass: a loud, lasting sound noise removal guts comes through as the plain mic; a click, a hum or what it keeps never trips it', () => {
  const n = SR * 3, d = RNNOISE_DELAY;
  // A quiet room, then a loud tone 1.0–2.0 s that a stand-in "RNNoise" takes 60 dB off (and passes the rest).
  const dry = pink(n, -60);
  for (let i = SR; i < 2 * SR; i++) dry[i] += 0.1 * Math.sin((2 * Math.PI * 1100 * i) / SR);
  const wet = late(dry, d);
  for (let i = SR + d; i < 2 * SR + d; i++) wet[i] *= 0.001;
  const { y } = loudPass(dry, wet);
  // From 50 ms in (3 loud blocks, a decision, a 10 ms fade) it's the mic, lined up with RNNoise's delay.
  const from = SR + d + Math.round(0.06 * SR), to = 2 * SR + d;
  assert.ok(Math.abs(dbOf(y, from, to) - dbOf(dry, from - d, to - d)) < 0.1, `passed: ${dbOf(y, from, to).toFixed(1)} dBFS`);
  for (let i = from; i < to; i += 997) assert.ok(Math.abs(y[i] - dry[i - d]) < 1e-6, 'sample for sample, the plain mic');
  // 250 ms after it went quiet, back to the noise removal's output.
  const after = 2 * SR + d + Math.round(0.3 * SR);
  assert.ok(Math.abs(y[after] - wet[after]) < 1e-9 && Math.abs(y[n - 200] - wet[n - 200]) < 1e-9, 'then RNNoise again');

  // A click (one 10 ms block at −15 dBFS) that's taken out: never passed.
  const click = pink(n, -60);
  for (let i = SR; i < SR + 480; i++) click[i] += 0.25 * Math.sin(i);
  const clickWet = late(click, d).map((v, i) => (i >= SR + d && i < SR + d + 480 ? v * 0.001 : v));
  assert.equal(loudPass(click, clickWet).passed, 0, 'a click');
  // Steady hum the noise removal takes out entirely, 15 dB over nothing: never loud.
  const hum = pink(n, -45);
  assert.equal(loudPass(hum, late(hum, d).map((v) => v * 0.01)).passed, 0, 'a fan');
  // Loud, lasting, and the noise removal keeps it (speech): nothing to do.
  const kept = late(dry, d);
  assert.equal(loudPass(dry, kept).passed, 0, 'speech it keeps');
  assert.deepEqual(LOUD_PASS, { windowMs: 10, minDb: -40, overFloorDb: 15, lasting: 3, of: 8, cutDb: -6, holdMs: 250, fadeMs: 10 }, 'the spec');
});

/** The web's RNNoise worklet (vendored, unchanged) run in Node, 128 samples at a time like the browser: a fresh one per call. */
let RnnoiseProcessor = null;
async function rnnoise(x) {
  if (!RnnoiseProcessor) {
    const before = { AudioWorkletProcessor: globalThis.AudioWorkletProcessor, registerProcessor: globalThis.registerProcessor, debug: console.debug };
    globalThis.AudioWorkletProcessor = class { port = { addEventListener() {}, postMessage() {} }; };
    globalThis.registerProcessor = (_name, c) => (RnnoiseProcessor = c);
    globalThis.__dirname ??= '/'; // (emscripten looks for it under Node)
    console.debug = () => {};
    await import('../web/vendor/noise/rnnoise-worklet.js');
    Object.assign(globalThis, { AudioWorkletProcessor: before.AudioWorkletProcessor, registerProcessor: before.registerProcessor });
    console.debug = before.debug;
  }
  const p = new RnnoiseProcessor({ processorOptions: { wasmBinary: fs.readFileSync(new URL('../web/vendor/noise/rnnoise_simd.wasm', import.meta.url)), maxChannels: 1 } });
  while (!p.processor) await new Promise((r) => setTimeout(r, 5));
  const y = new Float32Array(x.length);
  for (let i = 0; i + 128 <= x.length; i += 128) {
    const out = new Float32Array(128);
    p.process([[x.subarray(i, i + 128)]], [[out]]);
    y.set(out, i);
  }
  p.destroy();
  return y;
}

t("loud pass, with the web's real RNNoise: its delay is RNNOISE_DELAY; a squealing laugh it guts comes through; a fan and typing stay out", async () => {
  // The delay: where RNNoise's output lines up with a voice it keeps.
  const voice = ha();
  const kept = await rnnoise(voice);
  let best = 0, bestAt = 0;
  for (let lag = 900; lag <= 1100; lag++) {
    let c = 0;
    for (let i = SR / 5; i < voice.length - 1200; i += 2) c += voice[i] * kept[i + lag];
    if (c > best) [best, bestAt] = [c, lag];
  }
  assert.equal(bestAt, RNNOISE_DELAY, 'the vendored worklet changed: measure its delay again');
  assert.ok(dbOf(kept) - dbOf(voice) > -1, 'a low "ha ha ha" RNNoise keeps');

  const laugh = squeal();
  const gutted = await rnnoise(laugh);
  const passed = loudPass(laugh, gutted).y;
  const lost = dbOf(gutted) - dbOf(laugh), now = dbOf(passed) - dbOf(laugh);
  assert.ok(lost < -6, `RNNoise alone takes ${lost.toFixed(1)} dB off the squeal`);
  assert.ok(now > -1.5, `with the loud pass: ${now.toFixed(1)} dB`);

  const fan = pink(SR * 4, -50);
  const fanOut = loudPass(fan, await rnnoise(fan));
  assert.equal(fanOut.passed, 0);
  assert.ok(dbOf(fanOut.y, SR) - dbOf(fan, SR) < -15, `the fan stays out: ${(dbOf(fanOut.y, SR) - dbOf(fan, SR)).toFixed(1)} dB`);

  const keys = typing(SR * 4);
  const keysWet = await rnnoise(keys);
  const keysOut = loudPass(keys, keysWet);
  assert.equal(keysOut.passed, 0, 'typing never trips it');
  assert.ok(Math.abs(dbOf(keysOut.y) - dbOf(keysWet)) < 0.01);
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
