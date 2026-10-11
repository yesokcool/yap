//! The echo canceller, measured offline and deterministically: speech-like
//! voices, a small room, two sound devices with their own clocks and callback
//! sizes, and the app's own chain in between (the 80 Hz high-pass, then
//! `EchoRef` + `Canceller`). No real mic or speaker.
//!
//! `cargo test echo_sim` runs the quick checks; the full table (every delay,
//! level, drift and device case, 5 minutes of clock drift, CPU per 10 ms):
//! `cargo test --release echo_report -- --ignored --nocapture`.

use std::sync::Arc;

use super::dsp::HighPass;
use super::echo::{Canceller, EchoRef, EchoStats};
use super::{BLOCK, RATE};

const FS: f64 = RATE as f64;

/// xorshift64*: the same numbers every run.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [0, 1).
    fn f(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
    /// Roughly normal, unit variance (a sum of four uniforms).
    fn gauss(&mut self) -> f64 {
        ((0..4).map(|_| self.f()).sum::<f64>() - 2.0) * 3f64.sqrt()
    }
}

/// Two-pole resonator: one formant.
struct Reson {
    a1: f64,
    a2: f64,
    g: f64,
    y1: f64,
    y2: f64,
}
impl Reson {
    fn new() -> Self {
        Self { a1: 0.0, a2: 0.0, g: 0.0, y1: 0.0, y2: 0.0 }
    }
    fn set(&mut self, f: f64, bw: f64) {
        let r = (-std::f64::consts::PI * bw / FS).exp();
        self.a1 = 2.0 * r * (2.0 * std::f64::consts::PI * f / FS).cos();
        self.a2 = -r * r;
        self.g = 1.0 - r;
    }
    fn run(&mut self, x: f64) -> f64 {
        let y = self.g * x + self.a1 * self.y1 + self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// F1–F3 of a few vowels (Hz).
const VOWELS: [[f64; 3]; 6] = [[730.0, 1090.0, 2440.0], [270.0, 2290.0, 3010.0], [300.0, 870.0, 2240.0], [530.0, 1840.0, 2480.0], [570.0, 840.0, 2410.0], [660.0, 1720.0, 2410.0]];

/// `secs` of speech-like talk at `rms_db` dBFS RMS: syllables with a pitch
/// that moves, vowels gliding into each other (a glottal pulse through three
/// formants), some hissy consonants, gaps between syllables and words.
fn speech(f0: f64, seed: u64, secs: f64, rms_db: f64) -> Vec<f32> {
    let n = (secs * FS) as usize;
    let mut rng = Rng::new(seed);
    let mut out = vec![0f64; n];
    let (mut f1, mut f2, mut f3, mut hiss) = (Reson::new(), Reson::new(), Reson::new(), Reson::new());
    hiss.set(5500.0, 3000.0);
    let mut formants = VOWELS[0];
    let (mut phase, mut prev) = (0.0f64, 0.0f64);
    let mut i = 0usize;
    while i < n {
        let syllable = (rng.range(0.11, 0.30) * FS) as usize;
        let gap = (rng.range(0.02, 0.09) * FS) as usize + if rng.f() < 0.12 { (rng.range(0.15, 0.4) * FS) as usize } else { 0 };
        let voiced = rng.f() < 0.82;
        let target = VOWELS[(rng.next_u64() % VOWELS.len() as u64) as usize];
        let loud = rng.range(0.6, 1.0);
        let pitch = f0 * rng.range(0.85, 1.2);
        let glide = rng.range(-0.15, 0.1);
        for k in 0..(syllable + gap).min(n - i) {
            let t = k as f64 / syllable as f64;
            let env = if k < syllable { (t * 8.0).min(1.0) * (1.0 - t).powf(0.6) * loud } else { 0.0 };
            if k % 48 == 0 {
                for (f, t) in formants.iter_mut().zip(target) {
                    *f += (t - *f) * 0.08;
                }
                f1.set(formants[0], 90.0);
                f2.set(formants[1], 110.0);
                f3.set(formants[2], 160.0);
            }
            phase = (phase + pitch * (1.0 + glide * t) / FS).fract();
            // A Rosenberg glottal pulse, differentiated (the lips radiate its derivative).
            let g = if phase < 0.4 {
                0.5 * (1.0 - (std::f64::consts::PI * phase / 0.4).cos())
            } else if phase < 0.6 {
                (std::f64::consts::PI * (phase - 0.4) / 0.4).cos()
            } else {
                0.0
            };
            let src = (g - prev) * 20.0 + 0.03 * rng.gauss();
            prev = g;
            let s = if voiced { f1.run(src) + f2.run(src) * 0.6 + f3.run(src) * 0.3 } else { hiss.run(rng.gauss()) * 0.5 };
            out[i + k] = s * env;
        }
        i += syllable + gap;
    }
    let rms = (out.iter().map(|x| x * x).sum::<f64>() / n.max(1) as f64).sqrt().max(1e-12);
    let gain = 10f64.powf(rms_db / 20.0) / rms;
    out.into_iter().map(|x| (x * gain) as f32).collect()
}

/// A small room as sparse taps: the direct sound, six early reflections and a
/// tail decaying with `rt60`, scaled so the echo comes back `gain_db` below
/// what was played.
fn room(seed: u64, rt60: f64, gain_db: f64) -> Vec<(usize, f32)> {
    let mut rng = Rng::new(seed);
    let mut taps: Vec<(usize, f64)> = vec![(0, 1.0)];
    for _ in 0..6 {
        let at = rng.range(0.002, 0.015);
        taps.push(((at * FS) as usize, rng.range(0.25, 0.5) * if rng.f() < 0.5 { -1.0 } else { 1.0 }));
    }
    for _ in 0..240 {
        let at = rng.range(0.015, 0.12);
        taps.push(((at * FS) as usize, 0.22 * 10f64.powf(-3.0 * at / rt60) * rng.gauss()));
    }
    let energy: f64 = taps.iter().map(|(_, g)| g * g).sum();
    let scale = 10f64.powf(gain_db / 20.0) / energy.sqrt();
    taps.into_iter().map(|(d, g)| (d, (g * scale) as f32)).collect()
}

/// How the two sound devices behave.
#[derive(Clone, Copy, Debug)]
struct Devices {
    /// Samples per output callback / per mic chunk (48 kHz).
    out_chunk: usize,
    in_chunk: usize,
    /// From being played (handed to the canceller) to its echo reaching the
    /// capture thread, at most, ms.
    delay_ms: f64,
    /// The output's clock against the mic's, parts per million.
    drift_ppm: f64,
    /// The speakers change mid-call: (at s, new delay ms). The old stream
    /// stops (what it hadn't played is lost) and a new one starts 150 ms later.
    switch: Option<(f64, f64)>,
}

const DEVICES: Devices = Devices { out_chunk: 480, in_chunk: 480, delay_ms: 80.0, drift_ppm: 0.0, switch: None };

/// Who talks when (seconds), how loud the echo is, the room.
#[derive(Clone, Debug)]
struct Script {
    secs: f64,
    far: Vec<(f64, f64)>,
    near: Vec<(f64, f64)>,
    /// None: headphones (nothing comes back).
    echo_db: Option<f64>,
    rt60: f64,
    on: bool,
}

impl Script {
    /// 1 s quiet, the far end alone to `dt`, both for 4 s, the near end alone
    /// for 3 s, the far end alone again to the end.
    fn standard(echo_db: Option<f64>, dt: f64, secs: f64) -> Self {
        Self { secs, far: vec![(1.0, dt + 4.0), (dt + 7.0, secs)], near: vec![(dt, dt + 7.0)], echo_db, rt60: 0.25, on: true }
    }
    /// The quick one: far end alone 1–7 s, both 7–9 s, near end alone 9–11 s, far end 11–13 s.
    fn quick(echo_db: Option<f64>) -> Self {
        Self { secs: 13.0, far: vec![(1.0, 9.0), (11.0, 13.0)], near: vec![(7.0, 11.0)], echo_db, rt60: 0.25, on: true }
    }
}

struct Run {
    /// The echo and the near end as they come out of the high-pass (the
    /// canceller's input is their sum plus a little noise), and what came out.
    echo: Vec<f32>,
    near: Vec<f32>,
    out: Vec<f32>,
    /// Time in `Canceller::process`, per 10 ms block (µs).
    us_per_block: f64,
    stats: EchoStats,
}

fn fade(sig: &mut [f32], spans: &[(f64, f64)]) {
    let ramp = 0.02 * FS;
    for (i, s) in sig.iter_mut().enumerate() {
        let t = i as f64;
        let g = spans.iter().map(|&(a, b)| ((t - a * FS) / ramp).clamp(0.0, 1.0).min(((b * FS - t) / ramp).clamp(0.0, 1.0))).fold(0.0, f64::max);
        *s *= g as f32;
    }
}

/// Catmull-Rom interpolation at a fractional index (the far end on the mic's clock).
fn interp(x: &[f32], pos: f64) -> f32 {
    if pos < 1.0 || pos >= (x.len() - 2) as f64 {
        return 0.0;
    }
    let i = pos as usize;
    let t = (pos - i as f64) as f32;
    let (p0, p1, p2, p3) = (x[i - 1], x[i], x[i + 1], x[i + 2]);
    p1 + 0.5 * t * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
}

/// Play `script` through `dev`: the speakers' callbacks push what the mixer
/// renders into `EchoRef` (in 10 ms blocks, like `OutputState::fill`), the
/// mic's chunks go through the high-pass and the `Canceller`, all in the
/// order the two clocks would make them happen.
fn run(script: &Script, dev: Devices, seed: u64) -> Run {
    let total = (script.secs * FS) as usize;
    let mut far = speech(128.0, seed ^ 0xFA4, script.secs + 1.0, -20.0);
    fade(&mut far, &script.far);
    let mut near = speech(205.0, seed ^ 0x4EA, script.secs, -26.0);
    near.truncate(total);
    fade(&mut near, &script.near);

    // One true clock, in seconds. The mic's sample m is captured at m/FS and
    // reaches the capture thread with its chunk, `b_in` after the chunk's end.
    // The mixer's sample k plays at k/fs_out (the speakers started earlier),
    // its callback runs `b_out` before that, and the room takes `acoustic`.
    let fs_out = FS * (1.0 + dev.drift_ppm * 1e-6);
    let acoustic = 0.001;
    let b_in = 0.001_f64.max(dev.in_chunk as f64 / FS * 0.25);
    let b_out_for = |delay_ms: f64| {
        let b = delay_ms / 1000.0 - acoustic - dev.in_chunk as f64 / FS - b_in;
        assert!(b >= dev.out_chunk as f64 / fs_out * 0.99, "the speakers can't play a sample before it's rendered: {dev:?}");
        b
    };
    let b_out = b_out_for(dev.delay_ms);
    const GAP: f64 = 0.15;
    // A switch: when, how far the mixer had rendered by then, the new stream's lead.
    let switch = dev.switch.map(|(at, delay)| {
        let mut rendered = 0;
        let mut c = 0;
        while (c * dev.out_chunk) as f64 / fs_out - b_out < at {
            rendered = ((c + 1) * dev.out_chunk).div_ceil(BLOCK) * BLOCK;
            c += 1;
        }
        (at, rendered, b_out_for(delay))
    });
    // Which mixer sample is at the speaker at time t (None: silence).
    let playing = |t: f64| match switch {
        Some((at, k0, b2)) if t >= at => (t >= at + GAP + b2).then_some(k0 as f64 + (t - at - GAP - b2) * fs_out),
        _ => Some(t * fs_out),
    };

    let mut echo = vec![0f32; total];
    if let Some(db) = script.echo_db {
        let direct: Vec<f32> = (0..total).map(|m| playing(m as f64 / FS - acoustic).map_or(0.0, |k| interp(&far, k))).collect();
        for (d, g) in room(seed ^ 0x700, script.rt60, db) {
            for m in d..total {
                echo[m] += direct[m - d] * g;
            }
        }
    }
    let mut noise = Rng::new(seed ^ 0x401);
    let mic: Vec<f32> = (0..total).map(|m| echo[m] + near[m] + (noise.gauss() * 10f64.powf(-75.0 / 20.0)) as f32).collect();
    // The same high-pass on each part, for measuring.
    let mut hp = HighPass::new(RATE, 80.0);
    hp.process(&mut echo);
    let mut hp = HighPass::new(RATE, 80.0);
    hp.process(&mut near);

    let reference = Arc::new(EchoRef::default());
    reference.start();
    let mut canceller = Canceller::new(reference.clone());
    let mut highpass = HighPass::new(RATE, 80.0);
    let mut out = Vec::with_capacity(total);
    let mut stereo = vec![0f32; BLOCK * 2];
    let mut rendered = 0usize;
    let mut push_until = |need: usize, rendered: &mut usize| {
        while *rendered < need {
            for (j, f) in stereo.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                *f = [far.get(*rendered + j).copied().unwrap_or(0.0); 2];
            }
            reference.push(&stereo);
            *rendered += BLOCK;
        }
    };
    // Output callbacks so far (of the second stream, after a switch).
    let (mut c_out, mut c_in) = (0usize, 0usize);
    let mut switched: Option<bool> = None; // Some(started)
    let mut acc: Vec<f32> = Vec::new();
    let mut busy = std::time::Duration::ZERO;
    let mut blocks = 0usize;
    while (c_in + 1) * dev.in_chunk <= total {
        let t_in = ((c_in + 1) * dev.in_chunk) as f64 / FS + b_in;
        let t_out = match (switch, switched) {
            (Some((at, ..)), Some(_)) => at + GAP + (c_out * dev.out_chunk) as f64 / fs_out,
            _ => (c_out * dev.out_chunk) as f64 / fs_out - b_out,
        };
        if let (Some((at, k0, _)), None) = (switch, switched)
            && t_out >= at
            && t_in >= at
        {
            // The old stream stops; the app's stop_output.
            reference.stop();
            switched = Some(false);
            (c_out, rendered) = (0, k0);
            continue;
        }
        if t_out <= t_in {
            let need = match (switch, switched.as_mut()) {
                (Some((_, k0, _)), Some(started)) => {
                    if !*started {
                        reference.start();
                        *started = true;
                    }
                    k0 + (c_out + 1) * dev.out_chunk
                }
                _ => (c_out + 1) * dev.out_chunk,
            };
            push_until(need, &mut rendered);
            c_out += 1;
        } else {
            acc.extend_from_slice(&mic[c_in * dev.in_chunk..(c_in + 1) * dev.in_chunk]);
            c_in += 1;
            let mut at = 0;
            while acc.len() - at >= BLOCK {
                let mut block = [0f32; BLOCK];
                block.copy_from_slice(&acc[at..at + BLOCK]);
                at += BLOCK;
                highpass.process(&mut block);
                let t0 = std::time::Instant::now();
                canceller.process(script.on, &mut block);
                busy += t0.elapsed();
                blocks += 1;
                out.extend_from_slice(&block);
            }
            acc.drain(..at);
        }
    }
    echo.truncate(out.len());
    near.truncate(out.len());
    Run { echo, near, out, us_per_block: busy.as_secs_f64() * 1e6 / blocks.max(1) as f64, stats: canceller.stats() }
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| v as f64 * v as f64).sum()
}

fn span(r: &Run, a: f64, b: f64) -> (usize, usize) {
    ((a * FS) as usize, ((b * FS) as usize).min(r.out.len()))
}

/// Echo return loss enhancement over [a, b) s: the echo that went in against
/// what came out (only meaningful while the near end is quiet).
fn erle(r: &Run, a: f64, b: f64) -> f64 {
    let (i, j) = span(r, a, b);
    10.0 * (energy(&r.echo[i..j]) / energy(&r.out[i..j]).max(1e-20)).log10()
}

/// Seconds from `from` until ERLE over 250 ms windows reaches `db` and stays
/// within 5 dB of it until `until` (windows with echo under −50 dBFS, i.e.
/// pauses, don't count).
fn convergence(r: &Run, from: f64, until: f64, db: f64) -> Option<f64> {
    let mut first = None;
    let mut t = from;
    while t + 0.25 <= until {
        let (i, j) = span(r, t, t + 0.25);
        if energy(&r.echo[i..j]) / (j - i) as f64 > 1e-5 {
            let v = erle(r, t, t + 0.25);
            match first {
                None if v >= db => first = Some(t + 0.25 - from),
                Some(_) if v < db - 5.0 => first = None,
                _ => {}
            }
        }
        t += 0.25;
    }
    first
}

/// The canceller's own latency (samples): where the output lines up best with the near end.
fn latency(r: &Run, a: f64, b: f64) -> usize {
    let (i, j) = span(r, a, b);
    let j = j.min(r.out.len() - 1200);
    let corr = |l: usize| (i..j).map(|k| r.out[k + l] as f64 * r.near[k] as f64).sum::<f64>();
    (0..1200).max_by(|&p, &q| corr(p).total_cmp(&corr(q))).unwrap_or(0)
}

/// How many dB the near end lost on the way through over [a, b) s: the
/// least-squares gain of the output against the near end, at the
/// canceller's latency.
fn near_loss(r: &Run, a: f64, b: f64, lag: usize) -> f64 {
    let (i, j) = span(r, a, b);
    let j = j.min(r.out.len() - lag);
    let (mut xy, mut xx) = (0.0, 0.0);
    for k in i..j {
        let n = r.near[k] as f64;
        xy += r.out[k + lag] as f64 * n;
        xx += n * n;
    }
    -20.0 * (xy / xx.max(1e-20)).max(1e-9).log10()
}

/// The numbers one run gives.
#[derive(Debug)]
struct Measured {
    converged: Option<f64>,
    /// ERLE once converged (far end alone), and after double talk.
    erle: f64,
    erle_after: f64,
    /// Near-end loss while both talk, and alone.
    double_talk: f64,
    alone: f64,
    delay_ms: Option<i32>,
    us: f64,
}

fn measure(script: &Script, dev: Devices, seed: u64) -> Measured {
    let r = run(script, dev, seed);
    // The far end alone until `dt`, both until `far_stops`, the near end alone until `far_again`.
    let dt = script.near[0].0;
    let far_stops = script.far[0].1;
    let far_again = script.far.get(1).map_or(script.secs, |f| f.0);
    let lag = latency(&r, far_stops + 0.3, far_again - 0.1);
    Measured {
        converged: convergence(&r, 1.0, dt, 20.0),
        erle: erle(&r, 3.0, dt),
        erle_after: erle(&r, far_again + 0.5, script.secs),
        double_talk: near_loss(&r, dt + 0.5, far_stops, lag),
        alone: near_loss(&r, far_stops + 0.3, far_again - 0.1, lag),
        delay_ms: r.stats.delay_ms,
        us: r.us_per_block,
    }
}

#[test]
fn echo_sim_cancels_at_20_80_200_ms() {
    for (delay, out_chunk, in_chunk) in [(20.0, 256, 256), (80.0, 480, 480), (200.0, 1024, 1024)] {
        let dev = Devices { out_chunk, in_chunk, delay_ms: delay, ..DEVICES };
        let m = measure(&Script::quick(Some(-6.0)), dev, 7);
        println!("{delay} ms: {m:?}");
        assert!(m.converged.is_some_and(|t| t <= 2.0), "{delay} ms: 20 dB within 2 s: {m:?}");
        assert!(m.erle >= 30.0 && m.erle_after >= 30.0, "{delay} ms: {m:?}");
        // AEC3 turns the near end down a little while both talk, more early in a call (echo_report has the long numbers).
        assert!(m.double_talk <= 8.0, "{delay} ms: the near end keeps most of its level while both talk: {m:?}");
        assert!(m.alone <= 1.0, "{delay} ms: {m:?}");
        // AEC3 found the delay (it reports what's left after the canceller's own backlog).
        let found = m.delay_ms.expect("a delay estimate") as f64;
        assert!(found > 0.0 && found <= delay, "{delay} ms: {m:?}");
    }
}

#[test]
fn echo_sim_quiet_echo_odd_chunks_and_drift() {
    // −12 dB, a 44.1-kHz-sized output callback against 512-sample mic chunks, clocks 50 ppm apart.
    let dev = Devices { out_chunk: 441, in_chunk: 512, drift_ppm: 50.0, ..DEVICES };
    let m = measure(&Script::quick(Some(-12.0)), dev, 3);
    println!("{m:?}");
    assert!(m.converged.is_some_and(|t| t <= 2.5) && m.erle >= 25.0 && m.erle_after >= 25.0, "{m:?}");
    let dev = Devices { drift_ppm: -50.0, ..dev };
    let m = measure(&Script::quick(Some(-12.0)), dev, 3);
    println!("{m:?}");
    assert!(m.converged.is_some_and(|t| t <= 2.5) && m.erle >= 25.0 && m.erle_after >= 25.0, "{m:?}");
}

#[test]
fn echo_sim_big_bursts_and_fast_drift() {
    // Linux-style 85 ms output periods against 21 ms mic chunks, clocks 1000 ppm apart (far more than real ones).
    let dev = Devices { out_chunk: 4096, in_chunk: 1024, delay_ms: 140.0, drift_ppm: 1000.0, switch: None };
    let m = measure(&Script::quick(Some(-6.0)), dev, 5);
    println!("{m:?}");
    assert!(m.erle >= 20.0 && m.erle_after >= 20.0, "{m:?}");
}

#[test]
fn echo_sim_new_speakers_mid_call() {
    // New speakers 5 s in, with 70 ms more delay: lined up again and cancelling within a second.
    let dev = Devices { switch: Some((5.0, 150.0)), ..DEVICES };
    let script = Script::quick(Some(-6.0));
    let r = run(&script, dev, 9);
    assert!(r.stats.realigned >= 2, "{:?}", r.stats);
    let after = erle(&r, 6.0, 7.0);
    println!("switch: ERLE 6-7 s {after:.1} dB, {:?}", r.stats);
    assert!(after >= 20.0);
}

#[test]
fn echo_sim_headphones_lose_almost_nothing() {
    // Nothing comes back (headphones), but the far end plays: the near end stays, mostly
    // (early in a call AEC3 doesn't know yet that nothing comes back; echo_report has the long numbers).
    let m = measure(&Script::quick(None), DEVICES, 7);
    println!("{m:?}");
    assert!(m.double_talk <= 8.0 && m.alone <= 1.0, "{m:?}");
}

#[test]
fn echo_sim_off_changes_nothing() {
    let script = Script { on: false, ..Script::quick(Some(-6.0)) };
    let r = run(&script, DEVICES, 7);
    assert!(erle(&r, 2.0, 6.0).abs() < 0.01);
    assert!(near_loss(&r, 7.5, 10.5, 0).abs() < 0.1);
    assert!(!r.stats.on);
}

/// The full table for the record (release build for the CPU column).
#[test]
#[ignore]
fn echo_report() {
    let fmt = |v: Option<f64>| v.map_or("never".to_string(), |t| format!("{t:.2} s"));
    println!("| case | 20 dB after | ERLE (s.t.) | ERLE after d.t. | near-end loss, both talking | near end alone | AEC3 delay | µs / 10 ms |");
    println!("|---|---|---|---|---|---|---|---|");
    let mut cases: Vec<(String, Script, Devices)> = Vec::new();
    for db in [-6.0, -12.0] {
        for (delay, out_chunk, in_chunk) in [(20.0, 256, 256), (80.0, 480, 480), (200.0, 1024, 1024)] {
            cases.push((format!("{delay:.0} ms, {db:.0} dB"), Script::standard(Some(db), 9.0, 24.0), Devices { out_chunk, in_chunk, delay_ms: delay, ..DEVICES }));
        }
    }
    for ppm in [50.0, -50.0] {
        cases.push((format!("80 ms, −6 dB, {ppm:+.0} ppm"), Script::standard(Some(-6.0), 9.0, 24.0), Devices { drift_ppm: ppm, ..DEVICES }));
    }
    cases.push(("80 ms, −6 dB, 441/512 chunks".into(), Script::standard(Some(-6.0), 9.0, 24.0), Devices { out_chunk: 441, in_chunk: 512, ..DEVICES }));
    cases.push(("140 ms, 4096/1024 chunks".into(), Script::standard(Some(-6.0), 9.0, 24.0), Devices { out_chunk: 4096, in_chunk: 1024, delay_ms: 140.0, ..DEVICES }));
    cases.push(("headphones (no echo)".into(), Script::standard(None, 9.0, 24.0), DEVICES));
    for (name, script, dev) in cases {
        let m = measure(&script, dev, 7);
        println!(
            "| {name} | {} | {:.1} dB | {:.1} dB | {:.1} dB | {:.1} dB | {} | {:.1} |",
            fmt(m.converged),
            m.erle,
            m.erle_after,
            m.double_talk,
            m.alone,
            m.delay_ms.map_or("–".into(), |d| format!("{d} ms")),
            m.us
        );
    }
    // Long double talk: the far end alone for 20 s, then both for 10 s, three different voices each.
    for (name, db) in [("headphones (no echo)", None), ("speakers, −6 dB", Some(-6.0)), ("speakers, −12 dB", Some(-12.0))] {
        let mut line = String::new();
        for seed in [7u64, 9, 13] {
            let script = Script { secs: 33.0, far: vec![(1.0, 31.0)], near: vec![(21.0, 33.0)], echo_db: db, rt60: 0.25, on: true };
            let r = run(&script, DEVICES, seed);
            let lag = latency(&r, 31.3, 32.9);
            let (mut ducked, mut n) = (0, 0);
            let mut t = 21.5;
            while t < 31.0 {
                let (i, j) = span(&r, t, t + 0.5);
                if energy(&r.near[i..j]) / (j - i) as f64 > 1e-4 {
                    n += 1;
                    ducked += (near_loss(&r, t, t + 0.5, lag) > 6.0) as usize;
                }
                t += 0.5;
            }
            line += &format!("{:.1} dB ({ducked} of {n} half-seconds over 6 dB); ", near_loss(&r, 21.5, 31.0, lag));
        }
        println!("Both talking for 10 s after 20 s of the far end alone, {name}: near end loses {line}");
    }
    // Five minutes of a far end that never stops, the near end joining for 4 s every 30 s, clocks drifting.
    for ppm in [50.0, -50.0] {
        let secs = 300.0;
        let near: Vec<(f64, f64)> = (0..10).map(|i| (i as f64 * 30.0 + 20.0, i as f64 * 30.0 + 24.0)).collect();
        let script = Script { secs, far: vec![(1.0, secs)], near: near.clone(), echo_db: Some(-6.0), rt60: 0.25, on: true };
        let r = run(&script, Devices { in_chunk: 441, drift_ppm: ppm, ..DEVICES }, 11);
        let talking = |t: f64| near.iter().any(|&(a, b)| t + 0.25 > a - 0.3 && t < b + 0.5);
        let (mut loud, mut bad, mut worst, mut echo, mut left) = (0, 0, f64::MAX, 0.0, 0.0);
        let mut t = 3.0;
        while t + 0.25 < secs {
            let (i, j) = span(&r, t, t + 0.25);
            if !talking(t) {
                echo += energy(&r.echo[i..j]);
                left += energy(&r.out[i..j]);
            }
            if !talking(t) && energy(&r.echo[i..j]) / (j - i) as f64 > 1e-4 {
                loud += 1;
                let v = erle(&r, t, t + 0.25);
                bad += (v < 20.0) as usize;
                worst = worst.min(v);
            }
            t += 0.25;
        }
        println!(
            "5 min at {ppm:+.0} ppm: ERLE {:.1} dB while the far end talks alone; {bad} of {loud} loud 250 ms windows under 20 dB (worst {worst:.1} dB); lined up again {} times, {} samples owed; {:.1} µs per 10 ms",
            10.0 * (echo / left).log10(),
            r.stats.realigned,
            r.stats.underruns,
            r.us_per_block
        );
    }
}

