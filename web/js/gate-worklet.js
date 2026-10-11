/*
 * The transmit gate, on the audio thread (an AudioWorklet): mic in → what
 * we send out. It runs at full rate in background tabs (timers don't), and
 * deciding here instead of flipping `track.enabled` from the main thread
 * means a busy page can't clip the start of what you say.
 *
 *  - Voice activity: opens when 10 ms of sound is louder than your
 *    sensitivity (or than the room's noise + 12 dB, whichever is higher:
 *    the adaptive noise floor), stays open while it's within 6 dB of that
 *    but still clear of the noise (hysteresis), and for 450 ms after
 *    (hold). The output runs 40 ms behind the mic (pre-roll), so the gate
 *    opens *before* the first syllable instead of after it. Voice activity
 *    only: push-to-talk and always-on have no extra delay.
 *  - Push to talk: open while the key is down, plus a 200 ms tail.
 *  - Opening fades in over 5 ms, closing fades out over 15 ms (no clicks).
 *  - Every 20 ms it reports the mic's level (dBFS RMS, before the gate),
 *    whether the gate is open, the noise floor, and whether the mic has sent
 *    nothing but digital silence for 5 s.
 * Mute and deafen stay on the main thread (`track.enabled`): those must be
 * certain even if this thread stalls.
 *
 * `Gate` is plain JS so the tests can run it on recorded speech
 * (test/voice.test.mjs); the processors below are the browser glue.
 */

export const GATE = {
  windowMs: 10, // level measured over this much audio for each decision
  holdMs: 450,
  hysteresisDb: 6,
  fadeInMs: 5,
  fadeOutMs: 15,
  pttTailMs: 200,
  preRollMs: 40, // voice activity only (TD3: CLAUDE.md › Decision log)
  floorMarginDb: 12, // the adaptive part: open only this far above the room's noise…
  floorCloseMarginDb: 8, // …and close once it's back within this much of it…
  floorCapDb: -30, // …but never ask for more than this (a loud room still gets through)
  floorBucketMs: 500, // noise floor = quietest 10 ms in the last 8 × 500 ms
  floorBuckets: 8,
  reportMs: 20,
  silenceMs: 5000,
  silencePeak: 1 / 2 ** 23, // below one 24-bit step: digital silence, not a quiet room
};

const toDb = (meanSquare) => (meanSquare > 1e-12 ? 10 * Math.log10(meanSquare) : -120);

export class Gate {
  /** @param {number} sampleRate */
  constructor(sampleRate, { mode = 'vad', threshold = -50, ptt = false, test = false } = {}) {
    this.sr = sampleRate;
    const ms = (v) => Math.round((v / 1000) * sampleRate);
    this.window = ms(GATE.windowMs);
    this.hold = ms(GATE.holdMs);
    this.tail = ms(GATE.pttTailMs);
    this.preRoll = ms(GATE.preRollMs);
    this.windowsPerBucket = Math.round(GATE.floorBucketMs / GATE.windowMs);
    this.bucketWindows = 0;
    this.reportLen = ms(GATE.reportMs);
    this.silenceLen = ms(GATE.silenceMs);
    this.stepIn = 1 / ms(GATE.fadeInMs);
    this.stepOut = 1 / ms(GATE.fadeOutMs);
    this.mode = mode;
    this.threshold = threshold;
    this.ptt = ptt;
    this.test = test; // a mic test: push-to-talk acts like always-on
    this.t = 0; // samples processed
    this.open = mode === 'open';
    this.holdUntil = -1;
    this.gain = this.open ? 1 : 0;
    // Delay line for the pre-roll (per channel, allocated on first use).
    this.ring = [];
    this.ringSize = ms(100);
    this.w = 0;
    this.delay = this.#delayFor(mode);
    // Level for decisions (10 ms) and for the meter (20 ms).
    this.winSum = 0;
    this.winN = 0;
    this.repSum = 0;
    this.repN = 0;
    this.repPeak = 0;
    this.silentFor = 0;
    // Adaptive noise floor: the quietest window in each bucket.
    this.buckets = [];
    this.bucketMin = 0;
    this.bucketN = 0;
    this.floor = -120;
    this.level = -120;
  }

  #delayFor(mode) {
    return mode === 'vad' ? this.preRoll : 0;
  }

  /** Change mode / sensitivity / push-to-talk key state (any subset). */
  set({ mode, threshold, ptt, test } = {}) {
    if (threshold !== undefined) this.threshold = threshold;
    if (ptt !== undefined) {
      this.ptt = ptt;
      if (ptt) this.holdUntil = Number.POSITIVE_INFINITY;
      else if (this.holdUntil === Number.POSITIVE_INFINITY) this.holdUntil = this.t + this.tail;
    }
    if (test !== undefined) this.test = test;
    if (mode !== undefined && mode !== this.mode) {
      this.mode = mode;
      this.holdUntil = -1;
      if (mode === 'ptt' && this.ptt) this.holdUntil = Number.POSITIVE_INFINITY;
      const delay = this.#delayFor(mode);
      if (delay !== this.delay) {
        // A different delay: start the line over (a brief hiccup when switching modes).
        this.delay = delay;
        for (const r of this.ring) r.fill(0);
      }
    }
    this.#decide();
  }

  /** What the gate opens at right now (dBFS): your sensitivity, or higher in a noisy room. */
  get openAt() {
    return Math.max(this.threshold, Math.min(this.floor + GATE.floorMarginDb, GATE.floorCapDb));
  }

  /** What keeps it open (hysteresis): 6 dB under that, but never down into the room's noise. */
  get closeAt() {
    return Math.max(this.threshold - GATE.hysteresisDb, Math.min(this.floor + GATE.floorCloseMarginDb, GATE.floorCapDb - GATE.hysteresisDb));
  }

  #decide() {
    const mode = this.mode === 'ptt' && this.test ? 'open' : this.mode;
    if (mode === 'open') this.open = true;
    else if (mode === 'ptt') this.open = this.ptt || this.t < this.holdUntil;
    else this.open = this.t < this.holdUntil;
  }

  #window(meanSquare) {
    const db = toDb(meanSquare);
    this.level = db;
    // Noise floor: quietest window per bucket, quietest bucket of the last few seconds.
    // Digital silence (a muted device) says nothing about the room, so it's skipped.
    if (db > -110) {
      this.bucketMin = this.bucketN ? Math.min(this.bucketMin, db) : db;
      this.bucketN++;
    }
    if (++this.bucketWindows === this.windowsPerBucket) {
      this.bucketWindows = 0;
      if (this.bucketN) this.buckets.push(this.bucketMin);
      if (this.buckets.length > GATE.floorBuckets) this.buckets.shift();
      this.bucketN = 0;
      this.floor = this.buckets.length ? Math.min(...this.buckets) : -120;
    }
    if (this.mode === 'vad' && db > (this.open ? this.closeAt : this.openAt)) this.holdUntil = this.t + this.hold;
    this.#decide();
  }

  /**
   * One block of audio. `input`/`output`: arrays of Float32Array channels
   * (output already allocated; input may have fewer channels, or none).
   * Returns a report every ~20 ms, else null.
   */
  process(input, output) {
    const n = output[0]?.length ?? input[0]?.length ?? 0;
    const chans = output.length;
    while (this.ring.length < chans) this.ring.push(new Float32Array(this.ringSize));
    const size = this.ringSize;
    let report = null;
    for (let i = 0; i < n; i++) {
      const x0 = input[0]?.[i] ?? 0;
      const sq = x0 * x0;
      this.winSum += sq;
      this.repSum += sq;
      const a = x0 < 0 ? -x0 : x0;
      if (a > this.repPeak) this.repPeak = a;
      const r = (this.w - this.delay + size) % size;
      for (let c = 0; c < chans; c++) {
        const x = (input[c] ?? input[0])?.[i] ?? 0;
        const ring = this.ring[c];
        ring[this.w] = x;
        output[c][i] = ring[r] * this.gain;
      }
      this.w = (this.w + 1) % size;
      this.t++;
      if (this.open) this.gain = Math.min(1, this.gain + this.stepIn);
      else this.gain = Math.max(0, this.gain - this.stepOut);
      if (++this.winN === this.window) {
        this.#window(this.winSum / this.window);
        this.winSum = 0;
        this.winN = 0;
      } else if (this.mode !== 'vad' && this.winN % 128 === 0) {
        this.#decide(); // push-to-talk tail ends on time, not on a window edge
      }
      if (++this.repN === this.reportLen) {
        this.silentFor = this.repPeak < GATE.silencePeak ? this.silentFor + this.repN : 0;
        report = { db: toDb(this.repSum / this.repN), open: this.open, floor: this.floor, at: this.openAt, silent: this.silentFor >= this.silenceLen };
        this.repSum = 0;
        this.repN = 0;
        this.repPeak = 0;
      }
    }
    return report;
  }
}

// ── Browser glue ─────────────────────────────────────────────────────────
const Processor = globalThis.AudioWorkletProcessor ?? class {};

class GateProcessor extends Processor {
  constructor(options) {
    super();
    this.gate = new Gate(sampleRate, options?.processorOptions ?? {});
    this.port.onmessage = (e) => this.gate.set(e.data);
  }

  process(inputs, outputs) {
    const report = this.gate.process(inputs[0] ?? [], outputs[0] ?? []);
    if (report) this.port.postMessage(report);
    return true;
  }
}

/** Records what it hears (channel 0) for "Hear yourself", in ~100 ms chunks. */
class RecordProcessor extends Processor {
  constructor() {
    super();
    this.size = Math.round(sampleRate / 10);
    this.buf = new Float32Array(this.size);
    this.n = 0;
  }

  process(inputs) {
    const x = inputs[0]?.[0];
    const len = x?.length ?? 128;
    for (let i = 0; i < len; i++) {
      this.buf[this.n++] = x ? x[i] : 0;
      if (this.n === this.size) {
        this.port.postMessage(this.buf, [this.buf.buffer]); // (transferred: its length reads 0 after this)
        this.buf = new Float32Array(this.size);
        this.n = 0;
      }
    }
    return true;
  }
}

if (typeof registerProcessor === 'function') {
  registerProcessor('yap-gate', GateProcessor);
  registerProcessor('yap-record', RecordProcessor);
}
