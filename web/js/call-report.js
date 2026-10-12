/*
 * The call report (PROTOCOL.md › Voice › Call report): what Yap noticed about
 * one call — how each person was connected, lost and patched-over audio,
 * devices and settings, and a timeline — as plain text to copy when a call
 * sounded off. Each app keeps its own; nothing goes on the wire.
 *
 * It never holds an IP address, message text or a key: a candidate is kept by
 * its *type* only (host, srflx, prflx, relay), and stats are copied field by
 * field from a fixed list of numbers, never whole.
 *
 * No DOM in here (test/unit.test.mjs runs it in Node); js/app/call-report-ui.js
 * feeds it from the page.
 */

/** At most this many timeline lines (the oldest go first; the summary never does). */
export const MAX_TIMELINE = 2000;
const CANDIDATE_TYPES = new Set(['host', 'srflx', 'prflx', 'relay']);
const NO_INBOUND = { bytes: 0, packets: 0, lost: 0, samples: 0, concealed: 0, silentConcealed: 0, jitterBufferDelay: 0, jitterBufferEmitted: 0, fecReceived: 0, fecDiscarded: 0 };

/**
 * One getStats() report of a voice connection → the numbers the report (and
 * the connection hints) use. Counters stay cumulative, as the browser gives
 * them. `voiceMid`: the voice transceiver's mid (a screen share's sound is
 * another audio stream on the same connection).
 */
export function readCallStats(report, voiceMid) {
  const all = [...report.values()];
  const byId = (id) => (id ? report.get(id) : undefined);
  const transport = all.find((s) => s.type === 'transport' && s.selectedCandidatePairId);
  const pair = byId(transport?.selectedCandidatePairId) ?? all.find((s) => s.type === 'candidate-pair' && s.nominated && s.state === 'succeeded');
  const type = (candidate) => (CANDIDATE_TYPES.has(candidate?.candidateType) ? candidate.candidateType : null);
  const local = type(byId(pair?.localCandidateId));
  const remote = type(byId(pair?.remoteCandidateId));
  const pick = (kind) => {
    const list = all.filter((s) => s.type === kind && s.kind === 'audio');
    return list.find((s) => voiceMid != null && s.mid === voiceMid) ?? list[0] ?? null;
  };
  const out = pick('outbound-rtp');
  const inbound = pick('inbound-rtp');
  // What they report receiving from us (their RTCP receiver reports on our voice).
  const back = out ? all.find((s) => s.type === 'remote-inbound-rtp' && s.kind === 'audio' && (s.localId === out.id || (s.ssrc != null && s.ssrc === out.ssrc))) : null;
  const n = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : 0);
  return {
    rtt: typeof pair?.currentRoundTripTime === 'number' ? pair.currentRoundTripTime * 1000 : null,
    path: local && remote ? { local, remote } : null,
    /** Every audio stream we send on this connection (the voice and a share's sound), for the voice panel's kbps. */
    audioBytesSent: all.filter((s) => s.type === 'outbound-rtp' && s.kind === 'audio').reduce((sum, s) => sum + n(s.bytesSent), 0),
    sent: out ? { bytes: n(out.bytesSent), packets: n(out.packetsSent), target: typeof out.targetBitrate === 'number' ? out.targetBitrate : null } : null,
    received: inbound ? {
      bytes: n(inbound.bytesReceived),
      packets: n(inbound.packetsReceived),
      lost: n(inbound.packetsLost),
      samples: n(inbound.totalSamplesReceived),
      concealed: n(inbound.concealedSamples),
      silentConcealed: n(inbound.silentConcealedSamples),
      jitterBufferDelay: n(inbound.jitterBufferDelay),
      jitterBufferEmitted: n(inbound.jitterBufferEmittedCount),
      // Packets rebuilt from a redundant copy, as far as the browser counts them (Chrome 153: Opus FEC, not RED).
      fecReceived: typeof inbound.fecPacketsReceived === 'number' ? inbound.fecPacketsReceived : null,
      fecDiscarded: n(inbound.fecPacketsDiscarded),
    } : null,
    /** Their side of our voice: packets of ours they lost (so far), the share lost in their last report, the round trip. */
    theirs: back ? {
      lost: n(back.packetsLost),
      fraction: typeof back.fractionLost === 'number' && Number.isFinite(back.fractionLost) ? back.fractionLost : null,
      rtt: typeof back.roundTripTime === 'number' ? back.roundTripTime * 1000 : null,
    } : null,
  };
}

/** The voice (first audio) section of an SDP: its payload types in order, which ones are RED and Opus, and Opus's fmtp. */
function voiceSection(sdp) {
  const section = sdp?.split(/\r?\n(?=m=)/).find((s) => s.startsWith('m=audio'));
  if (!section) return null;
  const first = section.split(/\r?\n/)[0].split(' ')[3] ?? null;
  const pt = (name) => section.match(new RegExp(`^a=rtpmap:(\\d+) ${name}/48000`, 'im'))?.[1] ?? null;
  const opus = pt('opus');
  const red = pt('red');
  const fmtp = opus ? (section.match(new RegExp(`^a=fmtp:${opus} ([^\\r\\n]*)`, 'm'))?.[1] ?? '').trim() : '';
  return { first, opus, red, fmtp };
}

/**
 * What the voice goes as each way, from the descriptions: browsers send the
 * first codec of the *other* side's description, so we send RED when their
 * description lists it first, and receive it when ours does (and both have
 * it). `fmtp` is the Opus line we sent. (Stats can't tell: Chrome reports
 * the Opus inside RED as the codec both ways.)
 */
export function negotiatedVoice(localSdp, remoteSdp) {
  const local = voiceSection(localSdp);
  const remote = voiceSection(remoteSdp);
  if (!local || !remote) return null;
  const codec = (lead, other) => (lead.red && lead.first === lead.red && other.red ? 'red+opus' : 'opus');
  return { send: codec(remote, local), receive: codec(local, remote), fmtp: local.fmtp };
}

/** "Chrome 153 on macOS (computer)", from the user agent (and its client hints, when there are any). */
export function describeBrowser(ua = '', hints = null) {
  const browsers = [
    [/Edg(?:e|A|iOS)?\/(\d+)/, 'Edge'], [/OPR\/(\d+)/, 'Opera'], [/FxiOS\/(\d+)/, 'Firefox'], [/Firefox\/(\d+)/, 'Firefox'],
    [/CriOS\/(\d+)/, 'Chrome'], [/HeadlessChrome\/(\d+)/, 'Headless Chrome'], [/Chrome\/(\d+)/, 'Chrome'],
    [/Version\/(\d+(?:\.\d+)?).*Safari/, 'Safari'],
  ];
  let browser = 'A browser';
  for (const [re, name] of browsers) {
    const m = ua.match(re);
    if (m) {
      browser = `${name} ${m[1]}`;
      break;
    }
  }
  const os = /iPhone/.test(ua) ? 'iOS' : /iPad/.test(ua) ? 'iPadOS' : /Android/.test(ua) ? 'Android' : /Windows/.test(ua) ? 'Windows'
    : /CrOS/.test(ua) ? 'ChromeOS' : /Mac OS X|Macintosh/.test(ua) ? 'macOS' : /Linux/.test(ua) ? 'Linux' : hints?.platform || 'an unknown system';
  const kind = hints?.mobile || /iPhone|Mobile/.test(ua) ? 'phone' : /iPad|Tablet|Android/.test(ua) ? 'tablet' : 'computer';
  return `${browser} on ${hints?.platform || os} (${kind})`;
}

// ── Formatting ────────────────────────────────────────────────────────────

const two = (n) => String(n).padStart(2, '0');
/** "+mm:ss" since you joined ("+h:mm:ss" past an hour). */
export function clock(ms) {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  return h ? `+${h}:${two(Math.floor(s / 60) % 60)}:${two(s % 60)}` : `+${two(Math.floor(s / 60))}:${two(s % 60)}`;
}
/** "3:07", "1:02:03". */
export function duration(ms) {
  const s = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(s / 3600);
  return h ? `${h}:${two(Math.floor(s / 60) % 60)}:${two(s % 60)}` : `${Math.floor(s / 60)}:${two(s % 60)}`;
}
const localTime = (ms) => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
};
const pct = (part, whole) => `${(whole > 0 ? (Math.max(0, part) / whole) * 100 : 0).toFixed(1)} %`;
const thousands = (n) => String(Math.round(n)).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
const median = (list) => {
  if (!list.length) return null;
  const s = [...list].sort((a, b) => a - b);
  const mid = s.length >> 1;
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
};
const ms = (v) => (v == null ? 'n/a' : `${Math.round(v)} ms`);
const kbps = (v) => (v == null ? 'n/a' : `${Math.round(v)} kbps`);
const pathText = ({ local, remote }) => `${local === 'relay' || remote === 'relay' ? 'relay' : 'direct'} (${local} ↔ ${remote})`;

// ── The notes on one call ─────────────────────────────────────────────────

const emptySpan = () => ({ readings: 0, ms: 0, rtts: [], packets: 0, lost: 0, samples: 0, concealed: 0, bytesIn: 0, bytesOut: 0 });

/**
 * Everything noticed about one call, from joining it until leaving it. People
 * are kept by a key that outlives a session (their account), so a call kept
 * across a reconnect or a rebuilt connection stays one person.
 */
export class CallLog {
  /** @param {{ channel: string, channelId?: string, app: string, device: string, startedAt?: number }} info */
  constructor({ channel, channelId = null, app, device, startedAt = Date.now() }) {
    this.channel = channel;
    this.channelId = channelId;
    this.app = app;
    this.device = device;
    this.startedAt = startedAt;
    this.endedAt = null;
    this.endReason = null;
    /** Label → value ("Echo cancellation" → "on"), as they are now. */
    this.settings = {};
    this.devices = {};
    /** key → what we know about that person */
    this.people = new Map();
    /** { at: ms since joining, text } */
    this.lines = [];
    this.droppedLines = 0;
    this.nextTick = 10_000;
  }

  /** A timeline line. */
  note(text, at = Date.now()) {
    this.lines.push({ at: Math.max(0, at - this.startedAt), text });
    const extra = this.lines.length - MAX_TIMELINE;
    if (extra > 0) {
      this.lines.splice(0, extra);
      this.droppedLines += extra;
    }
  }

  /** Settings or devices as they are now; what changed goes on the timeline (unless `quiet`). */
  setSettings(next, { quiet = false, at = Date.now() } = {}) {
    this.#update('settings', next, quiet, at, (k, v) => `Settings: ${k.toLowerCase()} ${v}`);
  }

  setDevices(next, { quiet = false, at = Date.now() } = {}) {
    this.#update('devices', next, quiet, at, (k, v) => `${k}: ${v}`);
  }

  #update(field, next, quiet, at, line) {
    const before = this[field];
    for (const [k, v] of Object.entries(next)) {
      if (!quiet && k in before && before[k] !== v) this.note(line(k, v), at);
    }
    this[field] = { ...next };
  }

  /** That person's notes (made on first mention). */
  person(key, name) {
    let p = this.people.get(key);
    if (!p) {
      p = {
        name: name ?? 'Someone', present: true, state: null, paths: [], codecs: null, codecText: null,
        out: [], in: [], rtt: [], jitter: [], packets: 0, lost: 0, samples: 0, concealed: 0, rebuilt: 0, rebuiltKnown: false,
        restarts: 0, rebuilds: 0, kept: 0, weakMs: 0, weakSince: null, last: null, span: emptySpan(),
        yours: { sent: 0, lost: 0, rtt: [] }, // what they report about our voice (remote-inbound-rtp)
      };
      this.people.set(key, p);
    }
    if (name) p.name = name;
    return p;
  }

  /** Count an ICE restart ('restarts'), a rebuilt connection ('rebuilds') or a reconnect the call survived ('kept'). */
  count(key, name, what) {
    this.person(key, name)[what]++;
  }

  /** Their connection's state changed (closing is said by whoever leaves, not here). */
  connection(key, name, state, at = Date.now()) {
    if (state === 'closed') return;
    const p = this.person(key, name);
    if (state === p.state) return;
    p.state = state;
    this.note(`${p.name}: ${state}`, at);
  }

  /** *Weak connection* went on (`info` = { concealed: 0..1, rtt }) or off (null). */
  weak(key, name, info, at = Date.now()) {
    const p = this.person(key, name);
    if (info && p.weakSince == null) {
      p.weakSince = at;
      const why = [`${Math.round((info.concealed ?? 0) * 100)} % patched over`, info.rtt != null ? `rtt ${Math.round(info.rtt)} ms` : ''].filter(Boolean).join(', ');
      this.note(`${p.name}: weak connection (${why})`, at);
    } else if (!info && p.weakSince != null) {
      p.weakMs += at - p.weakSince;
      p.weakSince = null;
      this.note(`${p.name}: weak connection gone`, at);
    }
  }

  /** "Your connection is dropping some of your audio" went on (`info` = { lost: 0..1 }) or off (null). */
  ownWeak(info, at = Date.now()) {
    this.note(info ? `Your connection is dropping some of your audio (they lose about ${Math.round((info.lost ?? 0) * 100)} % of yours)` : 'Your connection is fine again', at);
  }

  /** They're out of the call (`text` says how); their numbers stay. */
  left(key, name, text, at = Date.now()) {
    const p = this.person(key, name);
    this.#closeWeak(p, at);
    p.present = false;
    p.state = null;
    p.last = null;
    if (text) this.note(text, at);
  }

  /** They're in the call (again). */
  joined(key, name, text, at = Date.now()) {
    const p = this.person(key, name);
    p.present = true;
    if (text) this.note(text, at);
  }

  #closeWeak(p, at) {
    if (p.weakSince == null) return;
    p.weakMs += at - p.weakSince;
    p.weakSince = null;
  }

  /**
   * One 2 s reading of someone's connection (readCallStats), with what the
   * voice goes as (negotiatedVoice). `conn` tells connections apart: counters
   * start again from zero on a new one.
   */
  sample(key, name, conn, reading, codecs, at = Date.now()) {
    if (!reading || this.endedAt) return;
    const p = this.person(key, name);
    const prev = p.last?.conn === conn ? p.last : null;
    const r = reading.received;
    const s = reading.sent;
    const inBefore = prev?.reading.received && r && r.packets >= prev.reading.received.packets ? prev.reading.received : null;
    const outBefore = prev?.reading.sent && s && s.packets >= prev.reading.sent.packets ? prev.reading.sent : null;
    const elapsed = prev ? at - prev.at : 0;
    const span = p.span;
    span.readings++;
    if (r) {
      const b = inBefore ?? NO_INBOUND;
      const packets = r.packets - b.packets;
      const lost = r.lost - b.lost;
      const samples = r.samples - b.samples;
      const concealed = Math.max(0, (r.concealed - r.silentConcealed) - (b.concealed - b.silentConcealed));
      p.packets += packets;
      p.lost += lost;
      p.samples += samples;
      p.concealed += concealed;
      Object.assign(span, { packets: span.packets + packets, lost: span.lost + lost, samples: span.samples + samples, concealed: span.concealed + concealed });
      if (r.fecReceived != null) {
        p.rebuiltKnown = true;
        p.rebuilt += Math.max(0, (r.fecReceived - r.fecDiscarded) - ((b.fecReceived ?? 0) - b.fecDiscarded));
      }
      const emitted = r.jitterBufferEmitted - b.jitterBufferEmitted;
      if (emitted > 0) p.jitter.push(((r.jitterBufferDelay - b.jitterBufferDelay) / emitted) * 1000);
      if (inBefore && elapsed > 0) {
        p.in.push(((r.bytes - b.bytes) * 8) / elapsed);
        span.bytesIn += r.bytes - b.bytes;
      }
    }
    if (s && outBefore && elapsed > 0) {
      p.out.push(((s.bytes - outBefore.bytes) * 8) / elapsed);
      span.bytesOut += s.bytes - outBefore.bytes;
    }
    // Ours, as they got it: their lost count against what we sent, from their first report on.
    const theirs = reading.theirs;
    if (theirs && s) {
      const b = prev?.reading.theirs && outBefore ? { lost: prev.reading.theirs.lost, sent: outBefore.packets } : { lost: 0, sent: 0 };
      p.yours.sent += Math.max(0, s.packets - b.sent);
      p.yours.lost += Math.max(0, theirs.lost - b.lost);
      if (theirs.rtt != null) p.yours.rtt.push(theirs.rtt);
    }
    if (prev && elapsed > 0) span.ms += elapsed;
    if (reading.rtt != null) {
      p.rtt.push(reading.rtt);
      span.rtts.push(reading.rtt);
    }
    if (reading.path) {
      const text = pathText(reading.path);
      if (p.paths.at(-1) !== text) {
        p.paths.push(text);
        this.note(`${p.name}: ${text}`, at);
      }
    }
    if (codecs) {
      const text = `${codecs.send} out, ${codecs.receive} in`;
      if (text !== p.codecText) {
        p.codecText = text;
        this.note(`${p.name}: ${text}`, at);
      }
      p.codecs = codecs;
    }
    p.last = { conn, reading, at };
  }

  /**
   * After each round of readings: a line per person every 10 s for the first
   * minute, then every minute. True when it wrote them (a good moment to save).
   */
  tick(at = Date.now()) {
    const since = at - this.startedAt;
    if (this.endedAt || since < this.nextTick) return false;
    while (this.nextTick <= since) this.nextTick += this.nextTick < 60_000 ? 10_000 : 60_000;
    for (const p of this.people.values()) {
      const span = p.span;
      p.span = emptySpan();
      if (!p.present || !span.readings) continue;
      const rate = (bytes) => (span.ms > 0 ? Math.round((bytes * 8) / span.ms) : 'n/a');
      this.note(`${p.name}: rtt ${ms(median(span.rtts))}, lost ${pct(span.lost, span.lost + span.packets)}, patched ${pct(span.concealed, span.samples)}, ${rate(span.bytesIn)} kbps in, ${rate(span.bytesOut)} out`, at);
    }
    return true;
  }

  /** The call is over; `reason` says why ("You left"). */
  end(reason, at = Date.now()) {
    if (this.endedAt) return;
    for (const p of this.people.values()) this.#closeWeak(p, at);
    this.endedAt = at;
    this.endReason = reason;
    this.note(`Call ended: ${reason}`, at);
  }
}

/** The report as plain text (see PROTOCOL.md › Voice › Call report for what's in it, in this order). */
export function formatReport(log, now = Date.now()) {
  const end = log.endedAt ?? now;
  const lines = [
    'Yap call report',
    `App: ${log.app}`,
    `Device: ${log.device}`,
    `Channel: ${log.channel}`,
    `Started: ${localTime(log.startedAt)} (local time)`,
    `Lasted: ${duration(end - log.startedAt)}${log.endedAt ? '' : ' so far (still in the call)'}`,
    'Settings:',
    ...Object.entries(log.settings).map(([k, v]) => `  ${k}: ${v}`),
    'Audio devices:',
    ...Object.entries(log.devices).map(([k, v]) => `  ${k}: ${v}`),
    '',
    `People (${log.people.size})`,
  ];
  if (!log.people.size) lines.push('  Nobody else was in the call.');
  for (const p of log.people.values()) {
    const weakMs = p.weakMs + (p.weakSince != null ? end - p.weakSince : 0);
    const rttMedian = median(p.rtt);
    lines.push(
      p.name,
      `  Connection: ${p.paths.length ? p.paths.join(', then ') : 'never connected'}`,
      `  Codec: ${p.codecText ?? 'not negotiated'}`,
      `  Opus fmtp sent: ${p.codecs?.fmtp || 'n/a'}`,
      `  Bitrate (median): ${kbps(median(p.out))} out, ${kbps(median(p.in))} in`,
      `  Round trip: ${ms(rttMedian)} median, ${ms(p.rtt.length ? Math.max(...p.rtt) : null)} worst`,
      `  Lost packets: ${pct(p.lost, p.lost + p.packets)} (${thousands(Math.max(0, p.lost))} of ${thousands(Math.max(0, p.lost) + p.packets)})`,
      `  Audio patched over: ${pct(p.concealed, p.samples)}`,
      // (Chrome counts Opus FEC here but not RED's copies: then lost-but-not-patched-over is what RED rebuilt.)
      `  Rebuilt from a redundant copy: ${p.rebuiltKnown && p.rebuilt > 0 ? `${thousands(p.rebuilt)} packets (as the browser counts them)` : 'not counted by this browser (lost but not patched over = rebuilt)'}`,
      `  Jitter buffer (median): ${ms(median(p.jitter))}`,
      `  ICE restarts: ${p.restarts}, connection rebuilds: ${p.rebuilds}, reconnects kept: ${p.kept}`,
      `  Weak connection shown: ${weakMs > 0 ? duration(weakMs) : 'never'}`,
      `  From you: ${p.yours.sent > 0 ? `they lost ${pct(p.yours.lost, p.yours.sent)} of yours (their round trip ${ms(median(p.yours.rtt))} median)` : 'no reports from them'}`,
    );
  }
  lines.push('', 'Timeline');
  if (log.droppedLines) lines.push(`(${thousands(log.droppedLines)} earlier lines left out: the timeline keeps the last ${thousands(MAX_TIMELINE)})`);
  for (const l of log.lines) lines.push(`${clock(l.at)} ${l.text}`);
  return `${lines.join('\n')}\n`;
}
