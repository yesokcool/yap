//! Short clips mixed over everything else: soundboard sounds, join chimes,
//! previews and Space's instruments (PROTOCOL.md › Extras › Soundboard).
//!
//! - At most two soundboard clips at once: a third stops the oldest.
//! - Everything goes through one limiter (threshold −12 dBFS, ratio 20:1,
//!   3 ms attack, 250 ms release — what the web does with a
//!   `DynamicsCompressorNode`), so a loud upload can't blast anyone.
//! - The caller sets each clip's gain (the server's measured `gain` × your
//!   soundboard volume, never above the measured gain); deafened = silence.

use std::sync::Arc;

/// Limiter threshold: −12 dBFS.
pub const THRESHOLD: f32 = 0.251_188_64;
const RATIO: f32 = 20.0;
/// Soundboard clips playing at once.
pub const MAX_BOARD: usize = 2;
/// Anything at all playing at once (piano mashing, chimes…).
const MAX_VOICES: usize = 24;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bus {
    /// Soundboard clips (≤ 2 at once).
    Board,
    /// Chimes, previews, instruments.
    Fx,
}

struct Voice {
    samples: Arc<Vec<f32>>,
    pos: usize,
    gain: f32,
    bus: Bus,
}

/// A little mixer of mono 48 kHz clips, with its limiter.
pub struct Clips {
    voices: Vec<Voice>,
    /// The limiter's level detector.
    level: f32,
    attack: f32,
    release: f32,
}

impl Default for Clips {
    fn default() -> Self {
        Self::new(super::RATE)
    }
}

impl Clips {
    pub fn new(rate: u32) -> Self {
        let coef = |secs: f32| 1.0 - (-1.0 / (secs * rate as f32)).exp();
        Self { voices: Vec::new(), level: 0.0, attack: coef(0.003), release: coef(0.25) }
    }

    /// Queue a clip (mono samples at 48 kHz) at `gain`.
    pub fn push(&mut self, samples: Arc<Vec<f32>>, gain: f32, bus: Bus) {
        if samples.is_empty() || gain <= 0.0 {
            return;
        }
        if bus == Bus::Board {
            while self.voices.iter().filter(|v| v.bus == Bus::Board).count() >= MAX_BOARD {
                let oldest = self.voices.iter().position(|v| v.bus == Bus::Board).unwrap_or(0);
                self.voices.remove(oldest);
            }
        }
        if self.voices.len() >= MAX_VOICES {
            self.voices.remove(0);
        }
        self.voices.push(Voice { samples, pos: 0, gain: gain.min(4.0), bus });
    }

    pub fn is_empty(&self) -> bool {
        self.voices.is_empty()
    }

    /// How many clips on a bus are still playing.
    #[cfg(test)]
    pub fn playing(&self, bus: Bus) -> usize {
        self.voices.iter().filter(|v| v.bus == bus).count()
    }

    pub fn clear(&mut self) {
        self.voices.clear();
    }

    /// Mix into interleaved stereo `out` (48 kHz), on top of what's there.
    pub fn render(&mut self, out: &mut [f32]) {
        if self.voices.is_empty() {
            self.level = 0.0;
            return;
        }
        for frame in out.chunks_mut(2) {
            let mut s = 0.0f32;
            for v in self.voices.iter_mut() {
                if let Some(x) = v.samples.get(v.pos) {
                    s += x * v.gain;
                    v.pos += 1;
                }
            }
            let y = self.limit(s);
            for o in frame.iter_mut() {
                *o += y;
            }
        }
        self.voices.retain(|v| v.pos < v.samples.len());
    }

    /// The limiter: follow the level (fast up, slow down) and squash what's
    /// over the threshold 20:1.
    fn limit(&mut self, s: f32) -> f32 {
        let x = s.abs();
        let coef = if x > self.level { self.attack } else { self.release };
        self.level += (x - self.level) * coef;
        if self.level <= THRESHOLD {
            return s;
        }
        let over = self.level / THRESHOLD;
        let gain = THRESHOLD * over.powf(1.0 / RATIO) / self.level;
        s * gain
    }
}

/// Peak and RMS of a clip (for tests and the upload preview).
pub fn measure(samples: &[f32]) -> (f32, f32) {
    let peak = samples.iter().fold(0f32, |a, x| a.max(x.abs()));
    let rms = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len().max(1) as f32).sqrt();
    (peak, rms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amp: f32, secs: f32) -> Arc<Vec<f32>> {
        Arc::new((0..(48_000.0 * secs) as usize).map(|i| amp * (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin()).collect())
    }

    #[test]
    fn limiter_holds_loud_clips_near_minus_12_db() {
        let mut c = Clips::default();
        c.push(sine(1.0, 0.5), 1.0, Bus::Board);
        let mut out = vec![0f32; 48_000];
        c.render(&mut out);
        // After the attack, a full-scale sine comes out a little over −12 dBFS (20:1), never near 0 dBFS.
        let tail = &out[20_000..];
        let peak = tail.iter().fold(0f32, |a, x| a.max(x.abs()));
        assert!(peak < 0.35, "limited peak {peak}");
        assert!(peak > 0.2, "still audible {peak}");
        // Quiet clips pass untouched.
        let mut c = Clips::default();
        c.push(sine(0.1, 0.2), 1.0, Bus::Fx);
        let mut out = vec![0f32; 4800];
        c.render(&mut out);
        let peak = out.iter().fold(0f32, |a, x| a.max(x.abs()));
        assert!((peak - 0.1).abs() < 0.005, "untouched {peak}");
    }

    #[test]
    fn two_board_clips_at_most() {
        let mut c = Clips::default();
        for _ in 0..3 {
            c.push(sine(0.1, 1.0), 1.0, Bus::Board);
        }
        c.push(sine(0.1, 1.0), 1.0, Bus::Fx);
        assert_eq!(c.playing(Bus::Board), 2);
        assert_eq!(c.playing(Bus::Fx), 1);
        let mut out = vec![0f32; 2 * 48_000 + 200];
        c.render(&mut out);
        assert!(c.is_empty());
    }
}
