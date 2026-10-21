//! Small signal-processing pieces for the voice path: an 80 Hz high-pass and
//! a slow automatic gain on the mic (V12), Settings › Voice › Mic volume, and
//! the limiter on people you've turned up past 100 % (V13).

/// A 2nd-order Butterworth high-pass (RBJ cookbook biquad): takes rumble,
/// desk thumps and mains hum out of the mic before anything else hears it.
pub struct HighPass {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl HighPass {
    pub fn new(rate: u32, cutoff: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * cutoff / rate as f32;
        let (sin, cos) = w0.sin_cos();
        // Butterworth: Q = 1/√2.
        let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 + cos) / 2.0 / a0,
            b1: -(1.0 + cos) / a0,
            b2: (1.0 + cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let x = *s;
            let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
            self.x2 = self.x1;
            self.x1 = x;
            self.y2 = self.y1;
            // Flush denormals (long silences).
            self.y1 = if y.abs() < 1e-20 { 0.0 } else { y };
            *s = y;
        }
    }
}

/// Speech is brought to about this level (RMS, dBFS) while you talk…
pub const AGC_TARGET_DB: f32 = -23.0;
/// …with at most this much boost, or this much cut.
pub const AGC_MAX_GAIN_DB: f32 = 20.0;
pub const AGC_MIN_GAIN_DB: f32 = -12.0;
/// Coming down is quick (per 10 ms block), going up slow (≈ 3 dB/s, while speaking).
const AGC_ATTACK_DB: f32 = 1.0;
const AGC_RELEASE_DB: f32 = 0.03;

/// A slow automatic gain for speech, like a browser's: it only learns while
/// you speak (so pauses and room noise are never pumped up), cuts quickly
/// when you get loud, recovers slowly, and never clips.
pub struct Agc {
    gain_db: f32,
    /// Smoothed level of recent speech (dBFS).
    speech_db: Option<f32>,
    /// The room's noise (dBFS), for telling speech apart without RNNoise.
    noise_db: f32,
}

impl Default for Agc {
    fn default() -> Self {
        Self { gain_db: 0.0, speech_db: None, noise_db: -60.0 }
    }
}

fn rms_db(buf: &[f32]) -> f32 {
    let ms = buf.iter().map(|x| x * x).sum::<f32>() / buf.len().max(1) as f32;
    if ms > 1e-12 { 10.0 * ms.log10() } else { -120.0 }
}

impl Agc {
    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }

    /// One 10 ms block, in place. `voice`: RNNoise's speech probability for
    /// this block, when it's running (else the level decides).
    pub fn process(&mut self, block: &mut [f32], voice: Option<f32>) {
        let db = rms_db(block);
        // The room: follows quiet blocks down quickly, creeps up slowly.
        if db > -110.0 {
            self.noise_db = if db < self.noise_db + 3.0 { self.noise_db + (db - self.noise_db) * 0.2 } else { self.noise_db + 0.02 };
        }
        let speech = match voice {
            Some(p) => p > 0.6 && db > -70.0,
            None => db > (self.noise_db + 10.0).max(-60.0),
        };
        let before = self.gain_db;
        if speech {
            let level = match self.speech_db {
                Some(s) => s + (db - s) * 0.05,
                None => db,
            };
            self.speech_db = Some(level);
            let want = (AGC_TARGET_DB - level).clamp(AGC_MIN_GAIN_DB, AGC_MAX_GAIN_DB);
            self.gain_db = if want < self.gain_db { (self.gain_db - AGC_ATTACK_DB).max(want) } else { (self.gain_db + AGC_RELEASE_DB).min(want) };
        }
        // Never clip: if this block would go over, come down for it at once.
        let peak = block.iter().fold(0f32, |m, x| m.max(x.abs()));
        let limit_db = if peak > 0.0 { 20.0 * (0.89 / peak).log10() } else { f32::INFINITY };
        if self.gain_db > limit_db {
            self.gain_db = limit_db.max(AGC_MIN_GAIN_DB - 20.0);
        }
        // Ramp across the block (no zipper noise); a cut for this block's peak starts at once.
        let (g0, g1) = (10f32.powf(before.min(limit_db) / 20.0), 10f32.powf(self.gain_db / 20.0));
        let n = block.len().max(1) as f32;
        for (i, s) in block.iter_mut().enumerate() {
            let g = g0 + (g1 - g0) * (i as f32 + 1.0) / n;
            *s = (*s * g).clamp(-1.0, 1.0);
        }
    }
}

/// Settings › Voice › Mic volume (0–200 %), on the capture thread just
/// before the gate, so voice activity and the meter hear what friends will.
/// A change ramps across one 10 ms block (no zipper noise). Above 100 % a
/// peak limiter keeps it under −1 dBFS (instant attack, 150 ms release, like
/// the boost on people), so a loud moment is turned down, never squared off.
/// At exactly 100 % it does nothing at all: the samples are today's, bit for bit.
pub struct MicGain {
    gain: f32,
    reduction: f32,
    release: f32,
}

/// Where Mic volume's limiter holds peaks (−1 dBFS, the automatic gain's ceiling).
pub const MIC_LIMIT: f32 = 0.89;
/// The slider's range.
pub const MIC_VOLUME_MAX: f32 = 2.0;

impl MicGain {
    pub fn new(rate: u32) -> Self {
        Self { gain: 1.0, reduction: 1.0, release: 1.0 - (-1.0 / (0.150 * rate as f32)).exp() }
    }

    /// One block, in place, at `target` (1 = 100 %).
    pub fn process(&mut self, block: &mut [f32], target: f32) {
        let target = if target.is_finite() { target.clamp(0.0, MIC_VOLUME_MAX) } else { 1.0 };
        let (g0, g1) = (self.gain, target);
        self.gain = target;
        if g0 == 1.0 && g1 == 1.0 && self.reduction >= 1.0 {
            return;
        }
        let n = block.len().max(1) as f32;
        for (i, s) in block.iter_mut().enumerate() {
            let g = if g0 == g1 { g1 } else { g0 + (g1 - g0) * (i as f32 + 1.0) / n };
            let x = *s * g;
            let want = if x.abs() > MIC_LIMIT { MIC_LIMIT / x.abs() } else { 1.0 };
            self.reduction = if want < self.reduction { want } else { (self.reduction + (want - self.reduction) * self.release).min(1.0) };
            // (Settled back to none: exactly 1, so 100 % is untouched again.)
            if self.reduction > 0.9999 {
                self.reduction = 1.0;
            }
            *s = (x * self.reduction).clamp(-1.0, 1.0);
        }
    }
}

/// A peak limiter for someone turned up past 100 %: instant attack, 150 ms
/// release, just under full scale (−3 dBFS), like the web's
/// DynamicsCompressor after its gain. The boost is for quiet talkers; a loud
/// one can't clip or blast your ears.
pub struct Limiter {
    /// Gain reduction now (1 = none).
    reduction: f32,
    release: f32,
}

pub const LIMIT: f32 = 0.708; // −3 dBFS

impl Limiter {
    pub fn new(rate: u32) -> Self {
        Self { reduction: 1.0, release: 1.0 - (-1.0 / (0.150 * rate as f32)).exp() }
    }

    /// Interleaved stereo, in place: multiply by `gain`, then limit.
    pub fn process(&mut self, buf: &mut [f32], gain: f32) {
        for f in buf.chunks_mut(2) {
            let peak = f.iter().fold(0f32, |m, x| m.max((x * gain).abs()));
            let want = if peak > LIMIT { LIMIT / peak } else { 1.0 };
            self.reduction = if want < self.reduction { want } else { self.reduction + (want - self.reduction) * self.release };
            for x in f.iter_mut() {
                *x *= gain * self.reduction;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn tone(freq: f32, db: f32, secs: f32) -> Vec<f32> {
        let amp = 10f32.powf(db / 20.0) * std::f32::consts::SQRT_2;
        (0..(SR as f32 * secs) as usize).map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / SR as f32).sin() * amp).collect()
    }

    #[test]
    fn high_pass_cuts_rumble_keeps_voice() {
        for (f, lo, hi) in [(30.0, -100.0, -15.0), (80.0, -4.0, -2.0), (300.0, -0.5, 0.3), (2000.0, -0.2, 0.2)] {
            let mut x = tone(f, -20.0, 1.0);
            HighPass::new(SR, 80.0).process(&mut x);
            let db = rms_db(&x[SR as usize / 2..]) + 20.0;
            assert!(db > lo && db < hi, "{f} Hz: {db:.1} dB");
        }
    }

    /// Speech-like: 300 ms syllables with 200 ms pauses over a quiet room.
    fn talk(db: f32, secs: f32) -> Vec<f32> {
        let mut x = tone(180.0, db, secs);
        for (i, s) in x.iter_mut().enumerate() {
            let t = i as f32 / SR as f32;
            if t % 0.5 > 0.3 {
                *s = 0.0;
            }
            *s += ((i * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.0004;
        }
        x
    }

    fn run_agc(x: &mut [f32]) -> Agc {
        let mut agc = Agc::default();
        for b in x.chunks_mut(480) {
            agc.process(b, None);
        }
        agc
    }

    fn speech_db(x: &[f32]) -> f32 {
        // RMS over the syllables of the last 2 s.
        let tail = &x[x.len() - 2 * SR as usize..];
        let on: Vec<f32> = tail.chunks(480).filter(|b| rms_db(b) > -60.0).flatten().copied().collect();
        rms_db(&on)
    }

    #[test]
    fn agc_brings_quiet_speech_up_slowly_and_at_most_20_db() {
        let mut x = talk(-38.0, 12.0);
        let agc = run_agc(&mut x);
        assert!((speech_db(&x) - AGC_TARGET_DB).abs() < 2.0, "quiet talker ends at {:.1} dBFS (gain {:.1})", speech_db(&x), agc.gain_db());
        let mut y = talk(-60.0, 20.0);
        let agc = run_agc(&mut y);
        assert!(agc.gain_db() <= AGC_MAX_GAIN_DB + 0.01, "gain {:.1}", agc.gain_db());
        // Slow: after 1 s of speech it's still far from +15 dB.
        let mut z = talk(-38.0, 1.0);
        let agc = run_agc(&mut z);
        assert!(agc.gain_db() < 5.0, "gain after 1 s {:.1}", agc.gain_db());
    }

    #[test]
    fn agc_cuts_loud_speech_quickly_and_never_clips() {
        let mut x = talk(-6.0, 4.0);
        let agc = run_agc(&mut x);
        assert!(agc.gain_db() < -10.0, "gain {:.1}", agc.gain_db());
        assert!(x.iter().all(|s| s.abs() <= 1.0));
        let peak = x[SR as usize..].iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak < 0.95, "peak {peak}");
    }

    #[test]
    fn agc_leaves_pauses_and_noise_alone() {
        // Room noise only: nothing to learn from, so no gain at all.
        let mut x: Vec<f32> = (0..SR as usize * 10).map(|i| ((i * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.002).collect();
        let agc = run_agc(&mut x);
        assert_eq!(agc.gain_db(), 0.0);
    }

    #[test]
    fn limiter_holds_boosted_speech_under_full_scale() {
        let mut x: Vec<f32> = tone(200.0, -6.0, 1.0).iter().flat_map(|s| [*s, *s]).collect();
        let mut lim = Limiter::new(SR);
        lim.process(&mut x, 2.0);
        let peak = x.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak <= LIMIT + 1e-4, "peak {peak}");
        // A quiet talker at 200 % comes through doubled, untouched.
        let mut q: Vec<f32> = tone(200.0, -30.0, 0.5).iter().flat_map(|s| [*s, *s]).collect();
        let orig = q.clone();
        Limiter::new(SR).process(&mut q, 2.0);
        assert!(q.iter().zip(&orig).all(|(a, b)| (a - 2.0 * b).abs() < 1e-6));
    }
    #[test]
    fn mic_gain_at_100_is_untouched_and_at_50_is_minus_6_db() {
        let orig = tone(300.0, -20.0, 0.5);
        let mut x = orig.clone();
        let mut g = MicGain::new(SR);
        for b in x.chunks_mut(480) {
            g.process(b, 1.0);
        }
        assert!(x.iter().zip(&orig).all(|(a, b)| a.to_bits() == b.to_bits()), "bit-exact at 100 %");
        let mut half = orig.clone();
        let mut g = MicGain::new(SR);
        for b in half.chunks_mut(480) {
            g.process(b, 0.5);
        }
        // (After the first block's ramp.)
        let db = rms_db(&half[480..]) - rms_db(&orig[480..]);
        assert!((db + 6.02).abs() < 0.05, "{db:.2} dB");
        // Back to 100 %: one block's ramp, then the samples are the input again.
        let mut back = orig.clone();
        for b in back.chunks_mut(480) {
            g.process(b, 1.0);
        }
        assert!(back[480..].iter().zip(&orig[480..]).all(|(a, b)| a.to_bits() == b.to_bits()));
    }

    #[test]
    fn mic_gain_past_100_never_clips() {
        // A loud mic at 200 %: held under −1 dBFS, never squared off at the top.
        let mut x = tone(200.0, -3.0, 1.0);
        let mut g = MicGain::new(SR);
        for b in x.chunks_mut(480) {
            g.process(b, 2.0);
        }
        let peak = x.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak <= MIC_LIMIT + 1e-4, "peak {peak}");
        // A quiet one comes through doubled (+6 dB), the limiter never touching it.
        let orig = tone(200.0, -30.0, 0.5);
        let mut q = orig.clone();
        let mut g = MicGain::new(SR);
        for b in q.chunks_mut(480) {
            g.process(b, 2.0);
        }
        assert!(q[480..].iter().zip(&orig[480..]).all(|(a, b)| (a - 2.0 * b).abs() < 1e-6));
        // Out-of-range or broken values: 0–200 %, or 100 %.
        let mut z = orig.clone();
        let mut g = MicGain::new(SR);
        for b in z.chunks_mut(480) {
            g.process(b, f32::NAN);
        }
        assert!(z.iter().zip(&orig).all(|(a, b)| a.to_bits() == b.to_bits()));
        let mut m = orig.clone();
        let mut g = MicGain::new(SR);
        for b in m.chunks_mut(480) {
            g.process(b, -1.0);
        }
        assert!(m[480..].iter().all(|s| *s == 0.0), "0 % is silence");
    }
}
