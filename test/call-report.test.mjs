// The call report (web/js/call-report.js): reading stats, the notes on a call and their text. No browser.
//   node test/call-report.test.mjs
import assert from 'node:assert/strict';
import { CallLog, MAX_TIMELINE, clock, describeBrowser, duration, formatReport, negotiatedVoice, readCallStats } from '../web/js/call-report.js';

const cases = [];
const t = (name, fn) => cases.push([name, fn]);

// Addresses a real stats report carries (documentation ranges, a LAN, link-local IPv6, a TURN URL).
const SECRETS = ['192.168.1.23', '203.0.113.7', '198.51.100.9', '2001:db8::17', 'fe80::1c2b:3aff:fe4d:5e6f', 'relay.example.net', '54321', '61234'];
const IPV4 = /\b\d{1,3}(?:\.\d{1,3}){3}\b/;
const IPV6 = /\b[0-9a-f]{1,4}(?::[0-9a-f]{0,4}){2,}::?[0-9a-f]{0,4}\b/i;

/** A getStats() report the way Chrome shapes it: a map of id → stats, with candidates that carry addresses. */
function chromeReport({ bytesSent = 0, packetsSent = 0, received = 0, lost = 0, samples = 0, concealed = 0, silent = 0, jbDelay = 0, jbEmitted = 0, fec = 0, fecDiscarded = 0, bytesIn = 0, rtt = 0.038, local = 'srflx', remote = 'prflx' } = {}) {
  const list = [
    { id: 'T01', type: 'transport', selectedCandidatePairId: 'CPa', dtlsState: 'connected' },
    { id: 'CPa', type: 'candidate-pair', localCandidateId: 'ILa', remoteCandidateId: 'IRb', state: 'succeeded', nominated: true, currentRoundTripTime: rtt },
    { id: 'CPx', type: 'candidate-pair', localCandidateId: 'ILx', remoteCandidateId: 'IRb', state: 'failed', nominated: false, currentRoundTripTime: 9 },
    { id: 'ILa', type: 'local-candidate', candidateType: local, address: '203.0.113.7', ip: '203.0.113.7', port: 54321, relatedAddress: '192.168.1.23', relatedPort: 61234, protocol: 'udp', url: 'stun:relay.example.net:3478' },
    { id: 'ILx', type: 'local-candidate', candidateType: 'host', address: 'fe80::1c2b:3aff:fe4d:5e6f', port: 61234, protocol: 'udp' },
    { id: 'IRb', type: 'remote-candidate', candidateType: remote, address: '198.51.100.9', ip: '198.51.100.9', port: 61234, protocol: 'udp', relayProtocol: 'udp', url: 'turn:relay.example.net' },
    { id: 'IRc', type: 'remote-candidate', candidateType: 'host', address: '2001:db8::17', port: 54321 },
    { id: 'OT0', type: 'outbound-rtp', kind: 'audio', mid: '0', bytesSent, packetsSent, targetBitrate: 128000, codecId: 'C111' },
    { id: 'OT1', type: 'outbound-rtp', kind: 'audio', mid: '1', bytesSent: 999999, packetsSent: 9999 }, // a screen share's sound
    { id: 'IT0', type: 'inbound-rtp', kind: 'audio', mid: '0', packetsReceived: received, packetsLost: lost, bytesReceived: bytesIn, totalSamplesReceived: samples, concealedSamples: concealed, silentConcealedSamples: silent, jitterBufferDelay: jbDelay, jitterBufferEmittedCount: jbEmitted, fecPacketsReceived: fec, fecPacketsDiscarded: fecDiscarded },
    { id: 'IT1', type: 'inbound-rtp', kind: 'audio', mid: '1', packetsReceived: 5, packetsLost: 500 },
    { id: 'C111', type: 'codec', mimeType: 'audio/opus', payloadType: 111, sdpFmtpLine: 'minptime=10;useinbandfec=1' },
  ];
  return new Map(list.map((s) => [s.id, s]));
}

const SDP = (pts, { red = true } = {}) => [
  'v=0', 'o=- 1 2 IN IP4 127.0.0.1', 's=-', 't=0 0',
  `m=audio 9 UDP/TLS/RTP/SAVPF ${pts}`, 'c=IN IP4 0.0.0.0', 'a=candidate:1 1 udp 2122260223 192.168.1.23 54321 typ host', 'a=mid:0', 'a=sendrecv',
  'a=rtpmap:111 opus/48000/2', 'a=fmtp:111 minptime=10;useinbandfec=1;usedtx=0;stereo=1;maxaveragebitrate=128000',
  ...(red ? ['a=rtpmap:63 red/48000/2', 'a=fmtp:63 111/111'] : []),
  'm=audio 9 UDP/TLS/RTP/SAVPF 111 63', 'a=mid:1', 'a=rtpmap:111 opus/48000/2', 'a=rtpmap:63 red/48000/2', '',
].join('\r\n');

t('readCallStats: the voice stream (not the screen\'s sound), the selected pair\'s round trip, candidate types only', () => {
  const r = readCallStats(chromeReport({ bytesSent: 64000, packetsSent: 100, received: 100, lost: 2, samples: 96000, concealed: 960, silent: 0, fec: 3, fecDiscarded: 1 }), '0');
  assert.equal(Math.round(r.rtt), 38);
  assert.deepEqual(r.path, { local: 'srflx', remote: 'prflx' });
  assert.deepEqual(r.sent, { bytes: 64000, packets: 100, target: 128000 });
  assert.equal(r.received.packets, 100);
  assert.equal(r.received.lost, 2);
  assert.equal(r.received.fecReceived, 3);
  assert.equal(r.audioBytesSent, 64000 + 999999, 'the panel\'s kbps counts every audio stream we send');
  const json = JSON.stringify(r);
  for (const secret of SECRETS) assert.ok(!json.includes(secret), `no ${secret} in the reading`);
  assert.doesNotMatch(json, IPV4);
  assert.doesNotMatch(json, IPV6);
});

t('readCallStats: an unknown candidate type is not passed through; no pair yet = no path', () => {
  const odd = chromeReport();
  odd.get('ILa').candidateType = '203.0.113.7';
  assert.equal(readCallStats(odd, '0').path, null);
  const early = new Map([...chromeReport()].filter(([, s]) => s.type !== 'transport' && s.type !== 'candidate-pair'));
  assert.equal(readCallStats(early, '0').path, null);
  assert.equal(readCallStats(early, '0').rtt, null);
  // Older browsers without fecPacketsReceived: "not reported", not zero.
  const old = chromeReport();
  delete old.get('IT0').fecPacketsReceived;
  assert.equal(readCallStats(old, '0').received.fecReceived, null);
});

t('negotiatedVoice: RED each way follows the first codec of the other side\'s description', () => {
  // Both put RED first (new apps): RED both ways, and the Opus fmtp we sent.
  assert.deepEqual(negotiatedVoice(SDP('63 111'), SDP('63 111')), { send: 'red+opus', receive: 'red+opus', fmtp: 'minptime=10;useinbandfec=1;usedtx=0;stereo=1;maxaveragebitrate=128000' });
  // An older app offers RED after Opus (a browser before this change, libwebrtc's default): it gets RED from us, we get Opus.
  assert.deepEqual(negotiatedVoice(SDP('63 111'), SDP('111 63')), { send: 'opus', receive: 'red+opus', fmtp: 'minptime=10;useinbandfec=1;usedtx=0;stereo=1;maxaveragebitrate=128000' });
  // An app without RED at all: plain Opus both ways, even though we offered it.
  assert.deepEqual(negotiatedVoice(SDP('63 111'), SDP('111', { red: false })), { send: 'opus', receive: 'opus', fmtp: 'minptime=10;useinbandfec=1;usedtx=0;stereo=1;maxaveragebitrate=128000' });
  assert.equal(negotiatedVoice(null, SDP('63 111')), null, 'not negotiated yet');
  assert.doesNotMatch(JSON.stringify(negotiatedVoice(SDP('63 111'), SDP('63 111'))), IPV4, 'nothing from the candidate lines');
});

t('clock and duration: +mm:ss, +h:mm:ss past an hour', () => {
  assert.equal(clock(0), '+00:00');
  assert.equal(clock(65_400), '+01:05');
  assert.equal(clock(3_599_999), '+59:59');
  assert.equal(clock(3_723_000), '+1:02:03');
  assert.equal(duration(187_000), '3:07');
  assert.equal(duration(3_723_000), '1:02:03');
});

t('describeBrowser: browser, system, kind of device (nothing else from the user agent)', () => {
  assert.equal(describeBrowser('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.8010.12 Safari/537.36'), 'Chrome 153 on macOS (computer)');
  assert.equal(describeBrowser('Mozilla/5.0 (iPhone; CPU iPhone OS 26_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.1 Mobile/15E148 Safari/604.1'), 'Safari 26.1 on iOS (phone)');
  assert.equal(describeBrowser('Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:150.0) Gecko/20100101 Firefox/150.0'), 'Firefox 150 on Windows (computer)');
  assert.equal(describeBrowser('Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Mobile Safari/537.36', { platform: 'Android', mobile: true }), 'Chrome 150 on Android (phone)');
});

/** A call with Ben: 2 s readings over ~75 s, a weak spell, an ICE restart; Ben leaves; then you do. */
function sampleCall() {
  const t0 = Date.UTC(2026, 9, 11, 7, 41, 5);
  const log = new CallLog({ channel: 'Lounge', channelId: 'lounge', app: 'Yap web 0.42.0 (build 0123456789ab)', device: 'Chrome 153 on macOS (computer)', startedAt: t0 });
  log.setSettings({ Quality: '128 kbps', 'Input mode': 'always on', 'Echo cancellation': 'on' }, { quiet: true, at: t0 });
  log.setDevices({ Microphone: 'Built-in Microphone', Output: 'system default' }, { quiet: true, at: t0 });
  log.note('You joined Lounge; Ben was there', t0);
  log.joined('ben', 'Ben', null, t0);
  log.connection('ben', 'Ben', 'connecting', t0 + 100);
  log.connection('ben', 'Ben', 'connected', t0 + 400);
  log.connection('ben', 'Ben', 'connected', t0 + 450); // (repeats are not lines)
  const conn = {};
  const codecs = negotiatedVoice(SDP('63 111'), SDP('63 111'));
  let c = { bytesSent: 0, packetsSent: 0, received: 0, lost: 0, samples: 0, concealed: 0, jbDelay: 0, jbEmitted: 0, fec: 0, bytesIn: 0 };
  for (let i = 1; i <= 38; i++) {
    const lossy = i >= 10 && i < 16; // 12 s of a bad network
    c = {
      ...c,
      bytesSent: c.bytesSent + 64_500, // 258 kbps over 2 s
      packetsSent: c.packetsSent + 100,
      received: c.received + (lossy ? 95 : 100),
      lost: c.lost + (lossy ? 5 : 0),
      samples: c.samples + 96_000,
      concealed: c.concealed + (lossy ? 4_800 : 0),
      jbDelay: c.jbDelay + 96_000 * 0.04,
      jbEmitted: c.jbEmitted + 96_000,
      fec: c.fec + (lossy ? 4 : 0),
      bytesIn: c.bytesIn + 64_750,
    };
    const at = t0 + 400 + i * 2000;
    log.sample('ben', 'Ben', conn, readCallStats(chromeReport({ ...c, rtt: lossy ? 0.31 : 0.038 }), '0'), codecs, at);
    if (i === 12) log.weak('ben', 'Ben', { concealed: 0.05, rtt: 310 }, at);
    if (i === 18) log.weak('ben', 'Ben', null, at);
    if (i === 14) {
      log.count('ben', 'Ben', 'restarts');
      log.note('Ben: ICE restart (try 1)', at);
    }
    log.tick(at);
  }
  log.setSettings({ Quality: '128 kbps', 'Input mode': 'always on', 'Echo cancellation': 'off' }, { at: t0 + 70_000 });
  log.left('ben', 'Ben', 'Ben left', t0 + 78_000);
  log.end('you left', t0 + 80_000);
  return log;
}

t('formatReport: header, people, timeline, in that order, with the numbers', () => {
  const text = formatReport(sampleCall());
  const lines = text.split('\n');
  assert.equal(lines[0], 'Yap call report');
  assert.ok(lines.includes('App: Yap web 0.42.0 (build 0123456789ab)'));
  assert.ok(lines.includes('Device: Chrome 153 on macOS (computer)'));
  assert.ok(lines.includes('Channel: Lounge'));
  assert.ok(lines.some((l) => /^Started: 2026-10-1[01] \d\d:41:05 \(local time\)$/.test(l)), 'local time');
  assert.ok(lines.includes('Lasted: 1:20'));
  assert.ok(lines.includes('  Echo cancellation: off'), 'settings as they are now');
  assert.ok(lines.includes('  Microphone: Built-in Microphone'));
  const people = lines.indexOf('People (1)');
  const timeline = lines.indexOf('Timeline');
  assert.ok(lines.indexOf('Settings:') < people && people < timeline, 'header, people, timeline');
  const ben = lines.slice(people + 1, timeline).join('\n');
  assert.match(ben, /^Ben$/m);
  assert.match(ben, /^ {2}Connection: direct \(srflx ↔ prflx\)$/m);
  assert.match(ben, /^ {2}Codec: red\+opus out, red\+opus in$/m);
  assert.match(ben, /^ {2}Opus fmtp sent: minptime=10;useinbandfec=1;usedtx=0;stereo=1;maxaveragebitrate=128000$/m);
  assert.match(ben, /^ {2}Bitrate \(median\): 258 kbps out, 259 kbps in$/m);
  assert.match(ben, /^ {2}Round trip: 38 ms median, 310 ms worst$/m);
  assert.match(ben, /^ {2}Lost packets: 0\.8 % \(30 of 3,800\)$/m);
  assert.match(ben, /^ {2}Audio patched over: 0\.8 %$/m);
  assert.match(ben, /^ {2}Rebuilt from a redundant copy: 24 packets \(as the browser counts them\)$/m);
  assert.match(ben, /^ {2}Jitter buffer \(median\): 40 ms$/m);
  assert.match(ben, /^ {2}ICE restarts: 1, connection rebuilds: 0, reconnects kept: 0$/m);
  assert.match(ben, /^ {2}Weak connection shown: 0:12$/m);
  const tl = lines.slice(timeline + 1);
  assert.equal(tl[0], '+00:00 You joined Lounge; Ben was there');
  assert.ok(tl.includes('+00:00 Ben: connecting') && tl.includes('+00:00 Ben: connected'));
  assert.equal(tl.filter((l) => l.endsWith('Ben: connected')).length, 1);
  assert.ok(tl.includes('+00:02 Ben: direct (srflx ↔ prflx)'));
  assert.ok(tl.includes('+00:02 Ben: red+opus out, red+opus in'));
  // A line per person every 10 s for the first minute, then every minute.
  const per = tl.filter((l) => /^\+\d\d:\d\d Ben: rtt /.test(l));
  assert.deepEqual(per.map((l) => l.slice(0, 6)), ['+00:10', '+00:20', '+00:30', '+00:40', '+00:50', '+01:00']);
  assert.ok(per.includes('+00:10 Ben: rtt 38 ms, lost 0.0 %, patched 0.0 %, 259 kbps in, 258 out'), per.join('\n'));
  assert.match(per[1], /lost [1-9]\.\d %, patched [1-9]\.\d %/, 'the lossy stretch shows');
  assert.ok(tl.includes('+00:24 Ben: weak connection (5 % patched over, rtt 310 ms)'));
  assert.ok(tl.includes('+00:36 Ben: weak connection gone'));
  assert.ok(tl.includes('+01:10 Settings: echo cancellation off'));
  assert.ok(tl.includes('+01:18 Ben left'));
  assert.equal(tl.at(-2), '+01:20 Call ended: you left');
});

t('formatReport: no IP address, port or relay name, whatever the stats carried', () => {
  const text = formatReport(sampleCall());
  for (const secret of SECRETS) assert.ok(!text.includes(secret), `no ${secret}`);
  assert.doesNotMatch(text, IPV4);
  assert.doesNotMatch(text, IPV6);
  assert.doesNotMatch(text, /candidate|typ host|address|turn:|stun:/i);
});

t('a rebuilt connection counts from zero again; a call still going says so', () => {
  const t0 = 1_000_000;
  const log = new CallLog({ channel: 'Lounge', app: 'Yap web', device: 'x', startedAt: t0 });
  const old = {};
  const fresh = {};
  log.sample('ben', 'Ben', old, readCallStats(chromeReport({ received: 1000, lost: 0, bytesIn: 300_000, bytesSent: 300_000, packetsSent: 1000 }), '0'), null, t0 + 2000);
  log.sample('ben', 'Ben', old, readCallStats(chromeReport({ received: 1100, lost: 0, bytesIn: 330_000, bytesSent: 330_000, packetsSent: 1100 }), '0'), null, t0 + 4000);
  log.count('ben', 'Ben', 'rebuilds');
  log.sample('ben', 'Ben', fresh, readCallStats(chromeReport({ received: 50, lost: 50, bytesIn: 15_000, bytesSent: 15_000, packetsSent: 50 }), '0'), null, t0 + 6000);
  const text = formatReport(log, t0 + 6000);
  assert.match(text, /Lost packets: 4\.2 % \(50 of 1,200\)/, '1,100 + 50 received, 50 lost');
  assert.match(text, /connection rebuilds: 1/);
  assert.match(text, /^Lasted: 0:06 so far \(still in the call\)$/m);
  assert.match(text, /Codec: not negotiated/);
  assert.match(text, /Rebuilt from a redundant copy: not counted by this browser/, 'zero from the browser is "not counted", not "none"');
  assert.match(text, /^ {2}Bitrate \(median\): 120 kbps out, 120 kbps in$/m, 'rates only from readings on the same connection');
});

t(`the timeline keeps the last ${MAX_TIMELINE} lines; the summary is always there`, () => {
  const log = new CallLog({ channel: 'Lounge', app: 'Yap web', device: 'x', startedAt: 0 });
  for (let i = 0; i < MAX_TIMELINE + 150; i++) log.note(`line ${i}`, i * 1000);
  const text = formatReport(log, (MAX_TIMELINE + 150) * 1000);
  const lines = text.split('\n');
  assert.equal(lines.filter((l) => / line \d+$/.test(l)).length, MAX_TIMELINE);
  assert.ok(lines.includes('(150 earlier lines left out: the timeline keeps the last 2,000)'));
  assert.ok(lines.at(-2).endsWith(`line ${MAX_TIMELINE + 149}`));
  assert.ok(!lines.some((l) => l.endsWith(' line 149')), 'the oldest went first');
  assert.equal(lines[0], 'Yap call report');
  assert.ok(lines.includes('People (0)') && lines.includes('  Nobody else was in the call.'));
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
