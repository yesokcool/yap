//! Minimal WAV reading (PCM 8/16/24/32-bit, float32) and 16-bit writing, for
//! the headless test mode.

use std::io::Write;

pub struct Wav {
    pub rate: u32,
    /// Mono, -1..1.
    pub samples: Vec<f32>,
}

pub fn read_mono(path: &str) -> Result<Wav, String> {
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    decode_mono(&data).map_err(|e| format!("{path}: {e}"))
}

/// Any common PCM WAV (8/16/24/32-bit, float32), mixed down to mono.
pub fn decode_mono(data: &[u8]) -> Result<Wav, String> {
    let path = "WAV";
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(format!("{path}: not a WAV file"));
    }
    let (mut fmt, mut channels, mut rate, mut bits) = (0u16, 0u16, 0u32, 0u16);
    let mut pos = 12;
    let mut body: Option<&[u8]> = None;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let start = pos + 8;
        let end = (start + size).min(data.len());
        if id == b"fmt " && end - start >= 16 {
            let c = &data[start..end];
            fmt = u16::from_le_bytes([c[0], c[1]]);
            channels = u16::from_le_bytes([c[2], c[3]]);
            rate = u32::from_le_bytes([c[4], c[5], c[6], c[7]]);
            bits = u16::from_le_bytes([c[14], c[15]]);
            if fmt == 0xFFFE && c.len() >= 26 {
                fmt = u16::from_le_bytes([c[24], c[25]]); // WAVE_FORMAT_EXTENSIBLE subformat
            }
        } else if id == b"data" {
            body = Some(&data[start..end]);
        }
        pos = start + size + (size & 1);
    }
    let body = body.ok_or_else(|| format!("{path}: no data chunk"))?;
    let _ = path;
    if channels == 0 || rate == 0 {
        return Err(format!("{path}: bad fmt chunk"));
    }
    let bytes_per = (bits / 8) as usize;
    if bytes_per == 0 {
        return Err(format!("{path}: unsupported WAV format {fmt}/{bits}-bit"));
    }
    let frame = bytes_per * channels as usize;
    let mut samples = Vec::with_capacity(body.len() / frame.max(1));
    for f in body.chunks_exact(frame) {
        let mut sum = 0f32;
        for c in 0..channels as usize {
            let s = &f[c * bytes_per..(c + 1) * bytes_per];
            sum += match (fmt, bits) {
                (1, 8) => (s[0] as f32 - 128.0) / 128.0,
                (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
                (1, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
                (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
                (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                _ => return Err(format!("{path}: unsupported WAV format {fmt}/{bits}-bit")),
            };
        }
        samples.push(sum / channels as f32);
    }
    Ok(Wav { rate, samples })
}

/// Streams 16-bit PCM to disk; the header is patched on `finish`.
pub struct WavWriter {
    file: std::io::BufWriter<std::fs::File>,
    channels: u16,
    rate: u32,
    bytes: u32,
}

impl WavWriter {
    pub fn create(path: &str, rate: u32, channels: u16) -> Result<Self, String> {
        let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
        let mut w = Self { file: std::io::BufWriter::new(file), channels, rate, bytes: 0 };
        w.header().map_err(|e| e.to_string())?;
        Ok(w)
    }

    fn header(&mut self) -> std::io::Result<()> {
        let block = self.channels * 2;
        let f = &mut self.file;
        f.write_all(b"RIFF")?;
        f.write_all(&(36 + self.bytes).to_le_bytes())?;
        f.write_all(b"WAVEfmt ")?;
        f.write_all(&16u32.to_le_bytes())?;
        f.write_all(&1u16.to_le_bytes())?;
        f.write_all(&self.channels.to_le_bytes())?;
        f.write_all(&self.rate.to_le_bytes())?;
        f.write_all(&(self.rate * block as u32).to_le_bytes())?;
        f.write_all(&block.to_le_bytes())?;
        f.write_all(&16u16.to_le_bytes())?;
        f.write_all(b"data")?;
        f.write_all(&self.bytes.to_le_bytes())
    }

    pub fn write(&mut self, interleaved: &[f32]) {
        for &s in interleaved {
            let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            let _ = self.file.write_all(&v.to_le_bytes());
        }
        self.bytes += interleaved.len() as u32 * 2;
    }

    pub fn finish(mut self) {
        use std::io::Seek;
        let _ = self.file.flush();
        if self.file.get_mut().seek(std::io::SeekFrom::Start(0)).is_ok() {
            let _ = self.header();
            let _ = self.file.flush();
        }
    }
}

/// Mono samples as a 16-bit PCM WAV file (what `/api/sound` takes).
pub fn encode_mono16(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    out
}

/// Resample mono samples to `to` Hz (windowed sinc).
pub fn resample_mono(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let mut r = super::resample::Resampler::new(from, to, 1);
    let mut out = Vec::with_capacity((samples.len() as u64 * to as u64 / from as u64) as usize + 64);
    r.process(samples, &mut out);
    // Flush the filter's tail with a little silence.
    r.process(&vec![0.0; (from / 50) as usize], &mut out);
    out.truncate((samples.len() as u64 * to as u64 / from as u64) as usize);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn wav_round_trip() {
        let s: Vec<f32> = (0..2400).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let bytes = super::encode_mono16(&s, 24_000);
        assert_eq!(&bytes[..4], b"RIFF");
        let w = super::decode_mono(&bytes).unwrap();
        assert_eq!((w.rate, w.samples.len()), (24_000, 2400));
        assert!((w.samples[100] - s[100]).abs() < 0.001);
        let up = super::resample_mono(&w.samples, 24_000, 48_000);
        assert_eq!(up.len(), 4800);
    }
}
