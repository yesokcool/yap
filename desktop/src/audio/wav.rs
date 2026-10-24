//! Minimal WAV reading (PCM 8/16/24/32-bit, float32) and 16-bit writing, for
//! the headless test mode, odd local files, and the soundboard's clips.
//!
//! Chunks are read the way the server's `checkWav` (`worker/extras.ts`) reads
//! them: the `fmt ` must come before the data, and nothing after the first
//! `data` chunk counts, so a clip can't tell the server one thing and us another.

use std::io::Write;

pub struct Wav {
    pub rate: u32,
    /// Mono, -1..1.
    pub samples: Vec<f32>,
}

/// A clip from the server may run this much past its kind's limit
/// (`Sound::max_ms`); anything longer is cut there.
pub const CLIP_SLACK_MS: u32 = 500;

pub fn read_mono(path: &str) -> Result<Wav, String> {
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    decode_mono(&data).map_err(|e| format!("{path}: {e}"))
}

/// What the `fmt ` chunk says, and the first `data` chunk's bytes.
struct Chunks<'a> {
    format: u16,
    channels: u16,
    rate: u32,
    bits: u16,
    body: &'a [u8],
}

fn chunks(data: &[u8]) -> Result<Chunks<'_>, String> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
        let start = pos + 8;
        let end = start.saturating_add(size).min(data.len());
        if id == b"fmt " && end - start >= 16 {
            let c = &data[start..end];
            let mut format = u16::from_le_bytes([c[0], c[1]]);
            if format == 0xFFFE && c.len() >= 26 {
                format = u16::from_le_bytes([c[24], c[25]]); // WAVE_FORMAT_EXTENSIBLE subformat
            }
            fmt = Some((format, u16::from_le_bytes([c[2], c[3]]), u32::from_le_bytes([c[4], c[5], c[6], c[7]]), u16::from_le_bytes([c[14], c[15]])));
        } else if id == b"data" {
            // The first data chunk is the sound; what follows it is never read.
            let (format, channels, rate, bits) = fmt.ok_or("its format comes after its sound")?;
            return Ok(Chunks { format, channels, rate, bits, body: &data[start..end] });
        }
        pos = start.saturating_add(size).saturating_add(size & 1);
    }
    Err(if fmt.is_some() { "no data chunk" } else { "no fmt chunk" }.into())
}

/// Any common PCM WAV (8/16/24/32-bit, float32), mixed down to mono: a file
/// on this computer (the headless tests' mic, a soundboard pick Symphonia
/// didn't take). Clips from the server go through `decode_clip`.
pub fn decode_mono(data: &[u8]) -> Result<Wav, String> {
    let path = "WAV";
    let Chunks { format: fmt, channels, rate, bits, body } = chunks(data).map_err(|e| format!("{path}: {e}"))?;
    // (Below 1 kHz isn't sound anyone records; it would only make resampling huge.)
    if channels == 0 || !(1_000..=768_000).contains(&rate) {
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

/// A soundboard clip or join chime from the server: only what the server
/// takes (16-bit mono PCM, 8–48 kHz), and at most `max_ms` (+ a little
/// slack) of it, so whatever a server sends, playing it costs at most a few
/// seconds of samples.
pub fn decode_clip(data: &[u8], max_ms: u32) -> Result<Wav, String> {
    let c = chunks(data)?;
    if c.format != 1 || c.channels != 1 || c.bits != 16 || !(8_000..=48_000).contains(&c.rate) {
        return Err(format!("not 16-bit mono PCM at 8–48 kHz ({}/{} ch/{}-bit/{} Hz)", c.format, c.channels, c.bits, c.rate));
    }
    let most = (c.rate as u64 * (max_ms as u64 + CLIP_SLACK_MS as u64) / 1000) as usize;
    let samples = c.body.as_chunks::<2>().0.iter().take(most).map(|s| i16::from_le_bytes(*s) as f32 / 32768.0).collect();
    Ok(Wav { rate: c.rate, samples })
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
    if from == 0 || to == 0 {
        return Vec::new();
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

    /// A RIFF/WAVE file made of these chunks, as they are.
    fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = b"WAVE".to_vec();
        for (id, data) in chunks {
            body.extend_from_slice(*id);
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(data);
            if data.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }

    fn fmt(format: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
        let block = channels * bits / 8;
        let mut c = Vec::new();
        for v in [format, channels] {
            c.extend_from_slice(&v.to_le_bytes());
        }
        c.extend_from_slice(&rate.to_le_bytes());
        c.extend_from_slice(&(rate * block as u32).to_le_bytes());
        c.extend_from_slice(&block.to_le_bytes());
        c.extend_from_slice(&bits.to_le_bytes());
        c
    }

    /// A clip the server takes, followed by a second `fmt ` saying 1 Hz: the
    /// server stops at the first data chunk, and so do we (that 1 Hz once made
    /// playing it reserve gigabytes).
    #[test]
    fn nothing_after_the_first_data_chunk_counts() {
        let pcm = vec![0x10u8; 96_000]; // 1 s at 48 kHz
        let bytes = riff(&[(b"fmt ", fmt(1, 1, 48_000, 16)), (b"data", pcm), (b"fmt ", fmt(1, 1, 1, 16))]);
        let clip = super::decode_clip(&bytes, 2000).unwrap();
        assert_eq!((clip.rate, clip.samples.len()), (48_000, 48_000));
        let any = super::decode_mono(&bytes).unwrap();
        assert_eq!((any.rate, any.samples.len()), (48_000, 48_000));
        // A second data chunk is ignored too.
        let bytes = riff(&[(b"fmt ", fmt(1, 1, 8_000, 16)), (b"data", vec![0; 800]), (b"data", vec![0; 8_000_000])]);
        assert_eq!(super::decode_clip(&bytes, 5000).unwrap().samples.len(), 400);
    }

    #[test]
    fn the_format_comes_first() {
        let bytes = riff(&[(b"data", vec![0; 800]), (b"fmt ", fmt(1, 1, 8_000, 16))]);
        assert!(super::decode_clip(&bytes, 5000).is_err());
        assert!(super::decode_mono(&bytes).is_err());
        assert!(super::decode_clip(&riff(&[(b"data", vec![0; 800])]), 5000).is_err());
        assert!(super::decode_clip(&riff(&[(b"fmt ", fmt(1, 1, 8_000, 16))]), 5000).is_err());
        assert!(super::decode_clip(b"RIFF\0\0\0\0WAVE", 5000).is_err());
        assert!(super::decode_clip(b"not a wav", 5000).is_err());
    }

    /// Only what the server takes: 16-bit mono PCM, 8–48 kHz.
    #[test]
    fn clips_are_what_the_server_takes() {
        let clip = |f: Vec<u8>| super::decode_clip(&riff(&[(b"fmt ", f), (b"data", vec![0; 4_000])]), 5000);
        assert!(clip(fmt(1, 1, 8_000, 16)).is_ok());
        assert!(clip(fmt(1, 1, 48_000, 16)).is_ok());
        for bad in [fmt(1, 1, 7_999, 16), fmt(1, 1, 48_001, 16), fmt(1, 1, 1, 16), fmt(1, 1, 0, 16), fmt(1, 2, 24_000, 16), fmt(1, 1, 24_000, 8), fmt(1, 1, 24_000, 24), fmt(3, 1, 24_000, 32), fmt(0xFFFE, 1, 24_000, 16), fmt(1, 0, 24_000, 16)] {
            assert!(clip(bad.clone()).is_err(), "{bad:?}");
        }
        // A local file may be anything common, but not a rate that would only make resampling huge.
        let any = |f: Vec<u8>| super::decode_mono(&riff(&[(b"fmt ", f), (b"data", vec![0; 4_000])]));
        assert!(any(fmt(1, 2, 44_100, 24)).is_ok());
        assert!(any(fmt(3, 1, 96_000, 32)).is_ok());
        assert!(any(fmt(1, 1, 1, 16)).is_err());
        assert!(any(fmt(1, 1, 0, 16)).is_err());
    }

    /// A clip longer than its kind may be is cut after the slack.
    #[test]
    fn clips_are_cut_at_their_limit() {
        let long = riff(&[(b"fmt ", fmt(1, 1, 8_000, 16)), (b"data", vec![0; 8_000 * 2 * 60])]);
        assert_eq!(super::decode_clip(&long, 2000).unwrap().samples.len(), 8_000 * (2000 + super::CLIP_SLACK_MS as usize) / 1000);
        let pcm = super::decode_clip(&long, 5000).unwrap();
        let out = super::resample_mono(&pcm.samples, pcm.rate, 48_000);
        assert_eq!(out.len(), 48_000 * (5000 + super::CLIP_SLACK_MS as usize) / 1000);
    }

    /// Sizes that overflow or point past the end, and odd rates, fail or stop; never panic.
    #[test]
    fn broken_sizes_are_harmless() {
        let mut huge = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        huge.extend_from_slice(&u32::MAX.to_le_bytes());
        huge.extend_from_slice(&fmt(1, 1, 8_000, 16));
        assert!(super::decode_clip(&huge, 5000).is_err());
        let mut short = riff(&[(b"fmt ", fmt(1, 1, 8_000, 16))]);
        short.extend_from_slice(b"data");
        short.extend_from_slice(&u32::MAX.to_le_bytes());
        short.extend_from_slice(&[1, 2, 3]);
        assert_eq!(super::decode_clip(&short, 5000).unwrap().samples.len(), 1);
        assert!(super::resample_mono(&[0.5; 10], 0, 48_000).is_empty());
        assert!(super::resample_mono(&[0.5; 10], 48_000, 0).is_empty());
    }
}
