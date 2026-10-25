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
 *  - Push to talk: open while the key is down, plus a 200 ms tail. (Double-tap
 *    to stay live is decided on the main thread, by PttLatch below: the gate
 *    only hears "held" or not.) Its report says when push-to-talk is open
 *    (`pttOpen`), so the page can click when it closes after the tail.
 *  - Opening fades in over 5 ms, closing fades out over 15 ms (no clicks).
 *  - Every 20 ms it reports the mic's level (dBFS RMS, before the gate),
 *    whether the gate is open, the noise floor, and whether the mic has sent
 *    nothing but digital silence for 5 s.
 *  - …and the loudest sample in those 20 ms (`peak`, dBFS), for the meter's
 *    "Too loud (clipping)" (ClipWatch below).
 *  - Mic volume (`volume`, 0–2, Settings › Voice) is applied first, so the
 *    level, voice activity and what's sent all hear it; above 1 a soft
 *    limiter keeps it from going past 0 dBFS. At 1 the samples are left exactly as
 *    they are. `peak` stays the mic's own (before the volume: clipping
 *    there is the system's input level); `boost` is the loudest sample after
 *    the volume, before the limiter (it's working hard: turn it down).
 *  - It also says whether voice activity *would* open it (`voice`), in every
 *    mode: you're talking. Mute is applied after this (below), so that still
 *    works while you're muted: the "You're muted" reminder.
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

/** Mic volume's limiter: straight through up to −1 dBFS, then a soft knee that levels off at 0 dBFS (never past it). */
const KNEE = 10 ** (-1 / 20);
export function limit(y) {
  const a = y < 0 ? -y : y;
  if (a <= KNEE) return y;
  const out = KNEE + (1 - KNEE) * Math.tanh((a - KNEE) / (1 - KNEE));
  return y < 0 ? -out : out;
}

export class Gate {
  /** @param {number} sampleRate */
  constructor(sampleRate, { mode = 'vad', threshold = -50, ptt = false, test = false, volume = 1 } = {}) {
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
    this.volume = volume; // Mic volume (1 = as the mic gives it)
    this.t = 0; // samples processed
    this.open = mode === 'open';
    this.holdUntil = -1;
    this.voiceUntil = -1; // voice activity in any mode (talking while muted: would the gate open?)
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
    this.repBoost = 0;
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

  /** Change mode / sensitivity / push-to-talk key state / mic volume (any subset). */
  set({ mode, threshold, ptt, test, volume } = {}) {
    if (threshold !== undefined) this.threshold = threshold;
    if (volume !== undefined) this.volume = volume;
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
    // The same decision whatever the mode: are you talking? (Push-to-talk and always-on don't use it to send.)
    if (db > (this.t < this.voiceUntil ? this.closeAt : this.openAt)) this.voiceUntil = this.t + this.hold;
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
    const v = this.volume;
    let report = null;
    for (let i = 0; i < n; i++) {
      const raw = input[0]?.[i] ?? 0;
      const a = raw < 0 ? -raw : raw;
      if (a > this.repPeak) this.repPeak = a;
      let x0 = raw;
      if (v !== 1) {
        const y = raw * v;
        const b = y < 0 ? -y : y;
        if (b > this.repBoost) this.repBoost = b;
        x0 = limit(y);
      }
      const sq = x0 * x0;
      this.winSum += sq;
      this.repSum += sq;
      const r = (this.w - this.delay + size) % size;
      for (let c = 0; c < chans; c++) {
        let x = c === 0 ? x0 : (input[c] ?? input[0])?.[i] ?? 0;
        if (c !== 0 && v !== 1) x = limit(x * v);
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
        report = {
          db: toDb(this.repSum / this.repN), peak: toDb(this.repPeak * this.repPeak), boost: toDb(this.repBoost * this.repBoost), open: this.open, voice: this.t < this.voiceUntil,
          pttOpen: this.mode === 'ptt' && !this.test && this.open,
          floor: this.floor, at: this.openAt, silent: this.silentFor >= this.silenceLen,
        };
        this.repSum = 0;
        this.repN = 0;
        this.repPeak = 0;
        this.repBoost = 0;
      }
    }
    return report;
  }
}

/**
 * Talking while muted (PROTOCOL.md › Transmit gate): when to remind you. At
 * most once every 5 s, and not in the first second after you muted (you may
 * be finishing a word). The page shows "You're muted" each time; only the
 * first reminder since you muted (or joined a call muted) blips, so staying
 * muted while you talk to someone in the room doesn't beep every 5 s.
 */
export const MUTED_REMINDER = { everyMs: 5000, graceMs: 1000 };

export class MutedReminder {
  #mutedAt = -Infinity;
  #remindedAt = -Infinity;
  #blipped = false;

  /** You just muted, or joined a call muted (ms, any clock). */
  muted(now) {
    this.#mutedAt = now;
    this.#blipped = false;
  }

  /** You're talking (or pressed push-to-talk) while muted: remind you now? false, 'blip' (the first since you muted) or 'quiet'. */
  due(now) {
    if (now - this.#mutedAt < MUTED_REMINDER.graceMs || now - this.#remindedAt < MUTED_REMINDER.everyMs) return false;
    this.#remindedAt = now;
    if (this.#blipped) return 'quiet';
    this.#blipped = true;
    return 'blip';
  }
}

/**
 * Push-to-talk's double-tap (PROTOCOL.md › Transmit gate): pressed twice
 * within 400 ms (key down to key down), it stays open after you let go
 * ("Live — tap to stop"); the next press ends that, and is an ordinary
 * press (open while held, then the tail), so it never cuts you off mid-word.
 */
export const PTT_LATCH_MS = 400;

export class PttLatch {
  held = false;
  latched = false;
  #lastDown = -Infinity;

  /** The key went down (ms, any clock). */
  down(now) {
    if (this.latched) {
      this.latched = false;
      this.#lastDown = -Infinity; // (the press that ends it doesn't start a new double-tap)
    } else if (now - this.#lastDown <= PTT_LATCH_MS) {
      this.latched = true;
      this.#lastDown = -Infinity;
    } else {
      this.#lastDown = now;
    }
    this.held = true;
  }

  up() {
    this.held = false;
  }

  /** Off for good (the input mode changed). */
  reset() {
    this.held = false;
    this.latched = false;
    this.#lastDown = -Infinity;
  }

  /** Push-to-talk is on: held, or latched. */
  get open() {
    return this.held || this.latched;
  }
}

/**
 * Stronger noise removal lets loud, lasting sounds through (PROTOCOL.md ›
 * Voice › Transmit gate › Noise suppression). RNNoise is trained on speech, so a
 * hard, high-pitched laugh, a shout or singing can come out of it 20–60 dB
 * down. Given the mic both after RNNoise (`wet`) and as RNNoise got it
 * (`dry`), this crossfades to the plain mic while the mic is loud (−40 dBFS
 * and 15 dB over the room's noise, per 10 ms), that lasts (3 of the last 8
 * blocks: a key click is one) and RNNoise takes more than 6 dB off it, for
 * 250 ms at a time (renewed while it stays loud and lasting). Each 10 ms
 * block decides the next one. The dry side waits `delay` samples, so it
 * lines up with what RNNoise gives back.
 */
export const LOUD_PASS = { windowMs: 10, minDb: -40, overFloorDb: 15, lasting: 3, of: 8, cutDb: -6, holdMs: 250, fadeMs: 10 };
/** The vendored RNNoise worklet's delay at 48 kHz: its 512-sample ring plus RNNoise's own 480 (test/voice.test.mjs measures it). */
export const RNNOISE_DELAY = 992;

export class LoudPass {
  constructor(sampleRate, delay = RNNOISE_DELAY) {
    const ms = (v) => Math.round((v / 1000) * sampleRate);
    this.window = ms(LOUD_PASS.windowMs);
    this.holdWindows = Math.round(LOUD_PASS.holdMs / LOUD_PASS.windowMs);
    this.step = 1 / ms(LOUD_PASS.fadeMs);
    this.windowsPerBucket = Math.round(GATE.floorBucketMs / LOUD_PASS.windowMs);
    this.delay = delay;
    this.ring = new Float32Array(Math.max(1, delay));
    this.w = 0;
    this.out = new Float32Array(128);
    this.drySum = 0;
    this.wetSum = 0;
    this.n = 0;
    this.recent = 0; // a bit per 10 ms block, newest lowest: was it loud?
    this.held = 0; // blocks left of passing
    this.want = 0;
    this.mix = 0; // 0 = RNNoise's output, 1 = the plain mic
    // The room's noise, measured like the gate's (on the mic before RNNoise).
    this.buckets = [];
    this.bucketMin = 0;
    this.bucketN = 0;
    this.bucketWindows = 0;
    this.floor = -120;
  }

  /** Passing the plain mic through right now (tests). */
  get passing() {
    return this.want === 1;
  }

  /** One block, mono: RNNoise's output and the mic it was given. Returns the mix (a buffer reused next time). */
  process(wet, dry) {
    const n = wet.length;
    if (this.out.length < n) this.out = new Float32Array(n);
    const out = this.out;
    for (let i = 0; i < n; i++) {
      let d = dry[i] ?? 0;
      if (this.delay > 0) {
        const late = this.ring[this.w];
        this.ring[this.w] = d;
        this.w = (this.w + 1) % this.delay;
        d = late;
      }
      const x = wet[i];
      this.drySum += d * d;
      this.wetSum += x * x;
      const step = this.want - this.mix;
      this.mix += step > this.step ? this.step : step < -this.step ? -this.step : step;
      out[i] = x + (d - x) * this.mix;
      if (++this.n === this.window) this.#decide();
    }
    return out.subarray(0, n);
  }

  #decide() {
    const db = toDb(this.drySum / this.window);
    const cut = toDb(this.wetSum / this.window) - db;
    this.drySum = 0;
    this.wetSum = 0;
    this.n = 0;
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
    const loud = db >= LOUD_PASS.minDb && db >= this.floor + LOUD_PASS.overFloorDb;
    this.recent = ((this.recent << 1) | (loud ? 1 : 0)) & ((1 << LOUD_PASS.of) - 1);
    let count = 0;
    for (let b = this.recent; b; b &= b - 1) count++;
    const lasting = count >= LOUD_PASS.lasting;
    if (loud && lasting && (cut < LOUD_PASS.cutDb || this.held > 0)) this.held = this.holdWindows;
    else if (this.held > 0) this.held--;
    this.want = this.held > 0 ? 1 : 0;
  }
}

/**
 * "Too loud (clipping)" on the mic meter: the loudest sample of a 20 ms
 * report at −1 dBFS or more, three times within 2 s, shows it for 3 s.
 * (Once is a pop; again and again is a mic turned up too far.)
 */
export const CLIP = { db: -1, hits: 3, windowMs: 2000, holdMs: 3000 };

export class ClipWatch {
  #hits = [];
  #until = -Infinity;

  /** One report's peak (dBFS) at `now` (ms): is it clipping? */
  push(peakDb, now) {
    if (peakDb >= CLIP.db) this.#hits.push(now);
    while (this.#hits.length && this.#hits[0] <= now - CLIP.windowMs) this.#hits.shift();
    if (this.#hits.length >= CLIP.hits) this.#until = now + CLIP.holdMs;
    return now < this.#until;
  }
}

// ── Browser glue ─────────────────────────────────────────────────────────
const Processor = globalThis.AudioWorkletProcessor ?? class {};

/** Input 0: the mic (after RNNoise when it runs). Input 1, with RNNoise: the mic as RNNoise got it (LoudPass). */
class GateProcessor extends Processor {
  constructor(options) {
    super();
    this.gate = new Gate(sampleRate, options?.processorOptions ?? {});
    this.pass = new LoudPass(sampleRate);
    this.port.onmessage = (e) => this.gate.set(e.data);
  }

  process(inputs, outputs) {
    let input = inputs[0] ?? [];
    const dry = inputs[1]?.[0];
    if (dry && input[0]) input = [this.pass.process(input[0], dry)]; // (mono: RNNoise never runs in music mode)
    const report = this.gate.process(input, outputs[0] ?? []);
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
