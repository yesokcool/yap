//! Decoding whatever sound file someone picks for the soundboard (WAV, MP3,
//! M4A/AAC, OGG Vorbis, FLAC, AIFF, CAF/ALAC…) into mono f32 samples, with
//! Symphonia (pure Rust). Our own WAV reader is the fallback for odd WAVs.

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

/// The longest file we bother decoding (we keep at most 5 s of it anyway).
const MAX_SECONDS: u32 = 10 * 60;

/// Mono samples (-1..1) at their own rate.
#[derive(Clone, Debug)]
pub struct Decoded {
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl Decoded {
    pub fn seconds(&self) -> f32 {
        self.samples.len() as f32 / self.rate.max(1) as f32
    }
}

/// Decode `bytes` (`name` hints at the format by its extension).
pub fn decode(bytes: &[u8], name: &str) -> Result<Decoded, String> {
    match symphonia_decode(bytes, name) {
        Ok(d) if !d.samples.is_empty() => Ok(d),
        first => match super::wav::decode_mono(bytes) {
            Ok(w) if !w.samples.is_empty() => Ok(Decoded { rate: w.rate, samples: w.samples }),
            _ => Err(match first {
                Err(e) => e,
                Ok(_) => "there's no sound in it".into(),
            }),
        },
    }
}

fn symphonia_decode(bytes: &[u8], name: &str) -> Result<Decoded, String> {
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(name).extension().and_then(|e| e.to_str()) {
        hint.with_extension(&ext.to_ascii_lowercase());
    }
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes.to_vec())), MediaSourceStreamOptions::default());
    let mut format = symphonia::default::get_probe().probe(&hint, mss, FormatOptions::default(), MetadataOptions::default()).map_err(|e| {
        log::debug!("probe: {e}");
        "not a sound file Yap knows".to_string()
    })?;
    let track = format.default_track(TrackType::Audio).ok_or("no audio in it")?.clone();
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("no audio in it")?.clone();
    if params.codec == symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS {
        return opus(format, track.id, params.extra_data.as_deref());
    }
    let mut decoder = symphonia::default::get_codecs().make_audio_decoder(&params, &AudioDecoderOptions::default()).map_err(|e| {
        log::debug!("codec: {e}");
        "Yap can't decode that kind of audio".to_string()
    })?;
    let mut rate = params.sample_rate.unwrap_or(0);
    let mut out: Vec<f32> = Vec::new();
    let mut buf: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return if out.is_empty() { Err(e.to_string()) } else { Ok(Decoded { rate: rate.max(1), samples: out }) },
        };
        if packet.track_id != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A damaged packet: skip it, like players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return if out.is_empty() { Err(e.to_string()) } else { Ok(Decoded { rate: rate.max(1), samples: out }) },
        };
        let spec = decoded.spec();
        rate = spec.rate();
        let channels = spec.channels().count().max(1);
        buf.clear();
        decoded.copy_to_vec_interleaved::<f32>(&mut buf);
        out.extend(buf.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32));
        if rate > 0 && out.len() as u64 > rate as u64 * MAX_SECONDS as u64 {
            break;
        }
    }
    if rate == 0 {
        return Err("couldn't tell its sample rate".into());
    }
    Ok(Decoded { rate, samples: out })
}

/// Opus (voice memos, .opus/.ogg): Symphonia demuxes, our pure-Rust Opus
/// decoder (the one calls use) decodes at 48 kHz.
fn opus(mut format: Box<dyn symphonia::core::formats::FormatReader + '_>, track: u32, head: Option<&[u8]>) -> Result<Decoded, String> {
    // OpusHead: channel count at byte 9, pre-skip (48 kHz samples) at 10–11.
    let channels = head.and_then(|h| h.get(9)).copied().unwrap_or(2);
    let skip = head.filter(|h| h.len() >= 12).map(|h| u16::from_le_bytes([h[10], h[11]]) as usize).unwrap_or(0);
    let layout = if channels == 1 { ropus::Channels::Mono } else { ropus::Channels::Stereo };
    let n = if channels == 1 { 1 } else { 2 };
    let mut dec = ropus::Decoder::new(48_000, layout).map_err(|_| "can't decode that kind of audio".to_string())?;
    let mut buf = vec![0f32; 5760 * 2];
    let mut out: Vec<f32> = Vec::new();
    while let Ok(Some(packet)) = format.next_packet() {
        if packet.track_id != track {
            continue;
        }
        if let Ok(frames) = dec.decode_float(&packet.data, &mut buf[..5760 * n], ropus::DecodeMode::Normal) {
            out.extend(buf[..frames * n].chunks(n).map(|f| f.iter().sum::<f32>() / n as f32));
        }
        if out.len() > 48_000 * MAX_SECONDS as usize {
            break;
        }
    }
    out.drain(..skip.min(out.len()));
    Ok(Decoded { rate: 48_000, samples: out })
}

/// Where the sound starts: the first sample louder than about −40 dBFS (a
/// little before it, so the attack isn't clipped).
pub fn sound_start(d: &Decoded) -> usize {
    let threshold = 0.01;
    let first = d.samples.iter().position(|s| s.abs() > threshold).unwrap_or(0);
    first.saturating_sub((d.rate / 100) as usize)
}

/// The clip to upload: `start`..`start + len` seconds of `d`, mono, at
/// 24 kHz, with 10 ms fades, as a 16-bit WAV.
pub fn clip_wav(d: &Decoded, start: f32, len: f32) -> Vec<u8> {
    let rate = d.rate.max(1) as f32;
    let a = ((start.max(0.0) * rate) as usize).min(d.samples.len());
    let b = (((start.max(0.0) + len.max(0.0)) * rate) as usize).min(d.samples.len()).max(a);
    let mut s = super::wav::resample_mono(&d.samples[a..b], d.rate, 24_000);
    let fade = 240.min(s.len() / 2);
    let n = s.len();
    for i in 0..fade {
        let g = i as f32 / fade as f32;
        s[i] *= g;
        s[n - 1 - i] *= g;
    }
    super::wav::encode_mono16(&s, 24_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stereo 16-bit WAV: a 440 Hz tone after `lead` s of silence.
    fn stereo_wav(rate: u32, secs: f32, lead: f32) -> Vec<u8> {
        let n = (rate as f32 * secs) as usize;
        let mut out = Vec::new();
        let data = (n * 4) as u32;
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let v = if t < lead { 0.0 } else { (t * 440.0 * std::f32::consts::TAU).sin() * 0.5 };
            let s = (v * 32767.0) as i16;
            out.extend_from_slice(&s.to_le_bytes());
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    #[test]
    fn decodes_a_wav_to_mono() {
        let d = decode(&stereo_wav(44_100, 1.5, 0.0), "tone.wav").unwrap();
        assert_eq!(d.rate, 44_100);
        assert!((d.seconds() - 1.5).abs() < 0.01, "{}", d.seconds());
        let peak = d.samples.iter().fold(0f32, |a, x| a.max(x.abs()));
        assert!((peak - 0.5).abs() < 0.01, "mono mix keeps the level ({peak})");
        // Without a helpful name too.
        assert!(decode(&stereo_wav(22_050, 0.2, 0.0), "clip").is_ok());
    }

    #[test]
    fn junk_is_refused_calmly() {
        assert!(decode(b"definitely not audio, just some words", "x.mp3").is_err());
    }

    #[test]
    fn clips_are_24k_mono_16_bit_and_trimmed() {
        let d = decode(&stereo_wav(48_000, 8.0, 1.0), "long.wav").unwrap();
        let start = sound_start(&d) as f32 / d.rate as f32;
        assert!((start - 0.99).abs() < 0.02, "skips the leading silence ({start})");
        let wav = clip_wav(&d, start, 5.0);
        let back = crate::audio::wav::decode_mono(&wav).unwrap();
        assert_eq!(back.rate, 24_000);
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1, "mono");
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16, "16-bit");
        assert!((back.samples.len() as f32 / 24_000.0 - 5.0).abs() < 0.01);
        assert!(wav.len() <= 480 * 1024);
        // Faded in and out.
        assert!(back.samples[0].abs() < 0.01 && back.samples.last().unwrap().abs() < 0.01);
    }
}

#[cfg(test)]
mod files {
    /// `YAP_DECODE_FILES="a.mp3 b.m4a" cargo test decode_files -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn decode_files() {
        for f in std::env::var("YAP_DECODE_FILES").unwrap_or_default().split_whitespace() {
            let bytes = std::fs::read(f).unwrap();
            match super::decode(&bytes, f) {
                Ok(d) => println!("{f}: {} Hz, {:.2} s, peak {:.2}", d.rate, d.seconds(), d.samples.iter().fold(0f32, |a, x| a.max(x.abs()))),
                Err(e) => println!("{f}: ERROR {e}"),
            }
        }
    }
}
