//! Streaming windowed-sinc resampler (interleaved, any channel count).
//! Only used when a device doesn't run at 48 kHz (e.g. a 44.1 kHz headset on
//! Windows); 32 taps with a Blackman window is clean for voice and cheap.

const HALF_TAPS: usize = 16;
const TAPS: usize = HALF_TAPS * 2;
const PHASES: usize = 256;

pub struct Resampler {
    channels: usize,
    /// Input samples advanced per output sample.
    step: f64,
    table: Vec<f32>,
    /// Interleaved input history.
    buf: Vec<f32>,
    /// Position (in input frames, relative to `buf`) of the next output frame.
    pos: f64,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32, channels: usize) -> Self {
        let step = in_rate as f64 / out_rate as f64;
        // Low-pass at the lower Nyquist (with a little guard band).
        let cutoff = (out_rate as f64 / in_rate as f64).min(1.0) * 0.92;
        let mut table = vec![0f32; (PHASES + 1) * TAPS];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let mut sum = 0.0;
            let row = &mut table[p * TAPS..(p + 1) * TAPS];
            for (k, tap) in row.iter_mut().enumerate() {
                let t = k as f64 - (HALF_TAPS as f64 - 1.0) - frac;
                let x = cutoff * t;
                let sinc = if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) };
                // Blackman window over [-HALF_TAPS, HALF_TAPS].
                let w = (t + HALF_TAPS as f64) / TAPS as f64;
                let win = if (0.0..=1.0).contains(&w) {
                    0.42 - 0.5 * (2.0 * std::f64::consts::PI * w).cos() + 0.08 * (4.0 * std::f64::consts::PI * w).cos()
                } else {
                    0.0
                };
                let v = cutoff * sinc * win;
                *tap = v as f32;
                sum += v;
            }
            // Unity gain at DC for every phase.
            for tap in row.iter_mut() {
                *tap = (*tap as f64 / sum) as f32;
            }
        }
        Self { channels, step, table, buf: vec![0.0; HALF_TAPS * channels], pos: (HALF_TAPS - 1) as f64 }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let ch = self.channels;
        self.buf.extend_from_slice(input);
        let frames = self.buf.len() / ch;
        loop {
            let i = self.pos.floor() as usize;
            if i + HALF_TAPS + 1 > frames {
                break;
            }
            let frac = self.pos - i as f64;
            let fp = frac * PHASES as f64;
            let p0 = fp.floor() as usize;
            let mix = (fp - p0 as f64) as f32;
            let h0 = &self.table[p0 * TAPS..(p0 + 1) * TAPS];
            let h1 = &self.table[(p0 + 1).min(PHASES) * TAPS..((p0 + 1).min(PHASES) + 1) * TAPS];
            let start = i + 1 - HALF_TAPS;
            for c in 0..ch {
                let mut acc = 0f32;
                for k in 0..TAPS {
                    let h = h0[k] + (h1[k] - h0[k]) * mix;
                    acc += self.buf[(start + k) * ch + c] * h;
                }
                out.push(acc);
            }
            self.pos += self.step;
        }
        // Drop consumed history, keeping what the next window needs.
        let keep_from = (self.pos.floor() as usize).saturating_sub(HALF_TAPS - 1);
        if keep_from > 0 {
            self.buf.drain(..keep_from * ch);
            self.pos -= keep_from as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_and_gain() {
        let mut r = Resampler::new(44_100, 48_000, 1);
        let input: Vec<f32> = (0..44_100).map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44_100.0).sin() * 0.5).collect();
        let mut out = Vec::new();
        for chunk in input.chunks(441) {
            r.process(chunk, &mut out);
        }
        assert!((out.len() as i64 - 48_000).abs() < 40, "len {}", out.len());
        let rms = (out[2000..40_000].iter().map(|x| x * x).sum::<f32>() / 38_000.0).sqrt();
        assert!((rms - 0.5 / 2f32.sqrt()).abs() < 0.01, "rms {rms}");
    }

    #[test]
    fn stereo_down() {
        let mut r = Resampler::new(48_000, 44_100, 2);
        let input = vec![0.25f32; 48_000 * 2];
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert_eq!(out.len() % 2, 0);
        assert!((out[out.len() - 100] - 0.25).abs() < 1e-3);
    }
}
