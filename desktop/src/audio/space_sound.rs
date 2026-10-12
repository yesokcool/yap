//! Space's sound: its instruments (web/js/space-sound.js), synthesized once
//! into mono 48 kHz clips — the floor piano (C4–G5, black keys too) and four
//! drum pads, played through the clip bus at the web's output gain (0.22) —
//! and where voices come from: left or right by where people stand
//! (PROTOCOL.md › Space; the mixer pans each voice with `pan_gains`).

use std::collections::HashMap;
use std::f32::consts::TAU;
use std::sync::{Arc, LazyLock};

const RATE: f32 = super::RATE as f32;
/// The web's master gain for the instruments.
pub const GAIN: f32 = 0.22;

/// `exponentialRampToValueAtTime` from `a` at `t0` to `b` at `t1`.
fn exp_ramp(a: f32, b: f32, t0: f32, t1: f32, t: f32) -> f32 {
    a * (b / a).powf(((t - t0) / (t1 - t0)).clamp(0.0, 1.0))
}

fn triangle(phase: f32) -> f32 {
    4.0 * (phase - (phase + 0.5).floor()).abs() - 1.0
}

/// A soft, piano-ish note: a triangle plus a quieter octave, quick attack, long fade.
fn note(midi: u8) -> Vec<f32> {
    let f = 440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0);
    let n = (1.45 * RATE) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE;
            let env = if t < 0.008 {
                0.9 * t / 0.008
            } else if t < 0.25 {
                exp_ramp(0.9, 0.25, 0.008, 0.25, t)
            } else {
                exp_ramp(0.25, 0.0001, 0.25, 1.4, t)
            };
            env * (triangle(f * t) + 0.3 * (TAU * 2.0 * f * t).sin())
        })
        .collect()
}

/// Deterministic white noise.
fn noise(n: usize) -> Vec<f32> {
    let mut seed = 0x2545_f491u32;
    (0..n)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        })
        .collect()
}

/// RBJ high-pass biquad (Web Audio's default Q of 1 dB ≈ 1.12).
fn highpass(x: &mut [f32], cutoff: f32) {
    let w = TAU * cutoff / RATE;
    let q = 10f32.powf(1.0 / 20.0);
    let alpha = w.sin() / (2.0 * q);
    let cw = w.cos();
    let (b0, b1, b2) = ((1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0);
    let (a0, a1, a2) = (1.0 + alpha, -2.0 * cw, 1.0 - alpha);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    for s in x.iter_mut() {
        let x0 = *s;
        let y0 = (b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2) / a0;
        (x2, x1, y2, y1) = (x1, x0, y1, y0);
        *s = y0;
    }
}

/// kick, snare, tom or hat.
fn drum(kind: &str) -> Vec<f32> {
    match kind {
        "kick" | "tom" => {
            let (from, to, len) = if kind == "kick" { (150.0f32, 42.0f32, 0.38f32) } else { (230.0, 110.0, 0.4) };
            let n = ((len + 0.02) * RATE) as usize;
            let mut phase = 0.0f32;
            (0..n)
                .map(|i| {
                    let t = i as f32 / RATE;
                    let f = if t < len * 0.7 { exp_ramp(from, to, 0.0, len * 0.7, t) } else { to };
                    phase += f / RATE;
                    exp_ramp(1.2, 0.0001, 0.0, len, t) * (TAU * phase).sin()
                })
                .collect()
        }
        _ => {
            let hat = kind == "hat";
            let len = if hat { 0.07 } else { 0.2 };
            let total = if hat { len + 0.02 } else { 0.22f32.max(len + 0.02) };
            let n = (total * RATE) as usize;
            let mut buf = noise(n);
            highpass(&mut buf, if hat { 7000.0 } else { 1400.0 });
            let start = if hat { 0.5 } else { 0.8 };
            for (i, s) in buf.iter_mut().enumerate() {
                let t = i as f32 / RATE;
                *s *= if t <= len { exp_ramp(start, 0.0001, 0.0, len, t) } else { 0.0 };
                if !hat && t < 0.14 {
                    // The drum's body under the rattle.
                    *s += exp_ramp(0.5, 0.0001, 0.0, 0.12, t) * triangle(190.0 * t);
                }
            }
            buf
        }
    }
}

static NOTES: LazyLock<HashMap<u8, Arc<Vec<f32>>>> = LazyLock::new(|| (60..=79).map(|m| (m, Arc::new(note(m)))).collect());
static DRUMS: LazyLock<HashMap<&'static str, Arc<Vec<f32>>>> = LazyLock::new(|| ["kick", "snare", "tom", "hat"].into_iter().map(|k| (k, Arc::new(drum(k)))).collect());

/// A piano key (MIDI 60–79).
pub fn piano(midi: u8) -> Option<Arc<Vec<f32>>> {
    NOTES.get(&midi).cloned()
}

/// A drum pad: kick, snare, tom or hat.
pub fn pad(kind: &str) -> Option<Arc<Vec<f32>>> {
    DRUMS.get(kind).cloned()
}

/// Space voice, left/right (PROTOCOL.md › Space): someone to your right on
/// screen comes from the right. Pan = dx / 400, from −1 (left) to 1 (right);
/// anyone within 60 units is in the middle (both ears full). The web's
/// `voicePan`.
pub const PAN_WIDTH: f32 = 400.0;
pub const PAN_MIDDLE: f32 = 60.0;

pub fn voice_pan(dx: f32, dy: f32) -> f32 {
    if dx.hypot(dy) < PAN_MIDDLE {
        return 0.0;
    }
    (dx / PAN_WIDTH).clamp(-1.0, 1.0)
}

/// A pan as a gain per ear, (left, right): the near ear stays full, the far
/// one goes down to 1/4 of it at the edge (±1); the middle is full in both.
/// The web's `panGains`.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = if pan.is_finite() { pan.clamp(-1.0, 1.0) } else { 0.0 };
    let far = 1.0 - 0.75 * p.abs();
    if p >= 0.0 { (far, 1.0) } else { (1.0, far) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruments_are_there_and_sane() {
        for m in 60..=79 {
            let n = piano(m).unwrap();
            assert!((n.len() as f32 / RATE - 1.45).abs() < 0.01);
            let peak = n.iter().fold(0f32, |a, x| a.max(x.abs()));
            assert!(peak > 0.5 && peak < 1.4, "note {m} peak {peak}");
            assert!(n.last().unwrap().abs() < 0.01, "fades out");
        }
        assert!(piano(59).is_none() && piano(80).is_none());
        for k in ["kick", "snare", "tom", "hat"] {
            let d = pad(k).unwrap();
            let peak = d.iter().fold(0f32, |a, x| a.max(x.abs()));
            assert!(peak > 0.05 && peak < 1.5, "{k} peak {peak}");
        }
    }

    #[test]
    fn voices_come_from_where_people_stand() {
        assert_eq!(voice_pan(0.0, 0.0), 0.0);
        assert_eq!(voice_pan(50.0, 20.0), 0.0, "within 60 units: the middle");
        assert_eq!(voice_pan(200.0, 0.0), 0.5, "to the right");
        assert_eq!(voice_pan(-200.0, 300.0), -0.5, "to the left (only across counts)");
        assert_eq!(voice_pan(900.0, 0.0), 1.0, "far right is all the way");
        assert_eq!(voice_pan(-2000.0, 0.0), -1.0);
        assert_eq!(pan_gains(0.0), (1.0, 1.0), "the middle: both ears full");
        assert_eq!(pan_gains(1.0), (0.25, 1.0), "right: the left ear at a quarter, never less");
        assert_eq!(pan_gains(-1.0), (1.0, 0.25));
        assert_eq!(pan_gains(0.5), (0.625, 1.0));
        assert_eq!(pan_gains(5.0), (0.25, 1.0));
        assert_eq!(pan_gains(f32::NAN), (1.0, 1.0));
    }
}
