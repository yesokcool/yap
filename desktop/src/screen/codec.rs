//! H.264 both ways (OpenH264, built from its bundled source): captured BGRA
//! frames → scaled → I420 → constrained-baseline H.264 (Annex B), and H.264
//! access units → RGBA for the viewer.

use super::capture::Frame;
use openh264::OpenH264API;
use openh264::decoder::{Decoder, DecoderConfig, Flush};
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile, RateControlMode, SpsPpsStrategy, UsageType};
use openh264::formats::YUVSource;

/// The biggest picture that fits `max` with the same shape, never bigger than
/// the source, both sides even (H.264 works in 2 × 2 chroma blocks).
pub fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (2, 2);
    }
    let s = (max_w as f64 / w as f64).min(max_h as f64 / h as f64).min(1.0);
    let even = |v: f64| ((v.round() as u32).max(2)) & !1;
    (even(w as f64 * s), even(h as f64 * s))
}

/// An I420 picture (the encoder's input).
pub struct I420 {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl I420 {
    fn new(width: usize, height: usize) -> Self {
        Self { width, height, y: vec![0; width * height], u: vec![128; width * height / 4], v: vec![128; width * height / 4] }
    }
}

impl YUVSource for I420 {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    fn strides(&self) -> (usize, usize, usize) {
        (self.width, self.width / 2, self.width / 2)
    }
    fn y(&self) -> &[u8] {
        &self.y
    }
    fn u(&self) -> &[u8] {
        &self.u
    }
    fn v(&self) -> &[u8] {
        &self.v
    }
}

/// BGRA (any stride) → I420 at `out`'s size (BT.601, limited range: what a
/// decoder assumes when the stream doesn't say). Downscaling averages a 2 × 2
/// sample of each source box; same size copies straight through.
pub fn convert(src: &Frame, out: &mut I420) {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let (dw, dh) = (out.width, out.height);
    // Source columns/rows for each output column/row: the box's first and last pixel.
    let xs: Vec<(usize, usize)> = (0..dw).map(|x| (x * sw / dw, (((x + 1) * sw / dw).max(x * sw / dw + 1) - 1).min(sw - 1))).collect();
    let ys: Vec<(usize, usize)> = (0..dh).map(|y| (y * sh / dh, (((y + 1) * sh / dh).max(y * sh / dh + 1) - 1).min(sh - 1))).collect();
    let px = |x: usize, y: usize| -> (i32, i32, i32) {
        let i = y * src.stride + x * 4;
        (src.data[i + 2] as i32, src.data[i + 1] as i32, src.data[i] as i32)
    };
    let sample = |x: usize, y: usize| -> (i32, i32, i32) {
        let (x0, x1) = xs[x];
        let (y0, y1) = ys[y];
        if x0 == x1 && y0 == y1 {
            return px(x0, y0);
        }
        let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        ((a.0 + b.0 + c.0 + d.0 + 2) >> 2, (a.1 + b.1 + c.1 + d.1 + 2) >> 2, (a.2 + b.2 + c.2 + d.2 + 2) >> 2)
    };
    let luma = |(r, g, b): (i32, i32, i32)| (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
    let cw = dw / 2;
    for cy in 0..dh / 2 {
        for cx in 0..cw {
            let (x, y) = (cx * 2, cy * 2);
            let p = [sample(x, y), sample(x + 1, y), sample(x, y + 1), sample(x + 1, y + 1)];
            out.y[y * dw + x] = luma(p[0]);
            out.y[y * dw + x + 1] = luma(p[1]);
            out.y[(y + 1) * dw + x] = luma(p[2]);
            out.y[(y + 1) * dw + x + 1] = luma(p[3]);
            let (r, g, b) = ((p[0].0 + p[1].0 + p[2].0 + p[3].0) >> 2, (p[0].1 + p[1].1 + p[2].1 + p[3].1) >> 2, (p[0].2 + p[1].2 + p[2].2 + p[3].2) >> 2);
            out.u[cy * cw + cx] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            out.v[cy * cw + cx] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
        }
    }
}

/// One encoded picture (Annex B NAL units).
pub struct Encoded {
    pub data: Vec<u8>,
    pub keyframe: bool,
    pub width: u32,
    pub height: u32,
}

/// H.264 for screen sharing: constrained baseline (what every WebRTC peer
/// takes as `profile-level-id=42e01f`), tuned for screen content, a fixed
/// target bitrate, no periodic keyframes (they come when someone asks).
pub struct VideoEncoder {
    enc: Option<Encoder>,
    bitrate: u32,
    fps: f32,
    pic: Option<I420>,
    /// The bitrate/fps the encoder was made with (remade when they change).
    made: (u32, u32),
}

impl VideoEncoder {
    pub fn new(bitrate: u32, fps: u32) -> Self {
        Self { enc: None, bitrate, fps: fps as f32, pic: None, made: (0, 0) }
    }

    pub fn set_rate(&mut self, bitrate: u32, fps: u32) {
        self.bitrate = bitrate;
        self.fps = fps as f32;
    }

    fn make(&self) -> Result<Encoder, String> {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(self.bitrate))
            .max_frame_rate(FrameRate::from_hz(self.fps))
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .profile(Profile::Baseline)
            .sps_pps_strategy(SpsPpsStrategy::ConstantId)
            .intra_frame_period(IntraFramePeriod::from_num_frames(0))
            .skip_frames(false)
            // A slice per ~1100 bytes: each fits one RTP packet (no FU-A), and a lost packet costs one slice.
            .max_slice_len(1100);
        Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|e| format!("H.264 encoder: {e}"))
    }

    /// Change the target bitrate on the running encoder (no new keyframe):
    /// how a share backs off when a viewer's connection loses packets.
    pub fn set_bitrate_live(&mut self, bps: u32) {
        self.bitrate = bps;
        let Some(enc) = self.enc.as_mut() else { return };
        for option in [openh264_sys2::ENCODER_OPTION_MAX_BITRATE, openh264_sys2::ENCODER_OPTION_BITRATE] {
            let mut info = openh264_sys2::SBitrateInfo { iLayer: openh264_sys2::SPATIAL_LAYER_ALL, iBitrate: bps.min(i32::MAX as u32) as i32 };
            // SAFETY: the encoder is initialised (it has encoded), and both options take an SBitrateInfo.
            unsafe {
                enc.raw_api().set_option(option, (&raw mut info).cast());
            }
        }
        self.made.0 = bps;
    }

    /// Scale `frame` to fit `max`, encode it (a keyframe if `keyframe`).
    pub fn encode(&mut self, frame: &Frame, max: (u32, u32), keyframe: bool) -> Result<Encoded, String> {
        let (w, h) = fit(frame.width, frame.height, max.0, max.1);
        let rate = (self.bitrate, self.fps as u32);
        let mut fresh = false;
        if self.enc.is_none() || self.made != rate {
            self.enc = Some(self.make()?);
            self.made = rate;
            fresh = true;
        }
        if self.pic.as_ref().is_none_or(|p| (p.width, p.height) != (w as usize, h as usize)) {
            self.pic = Some(I420::new(w as usize, h as usize));
        }
        let pic = self.pic.as_mut().expect("picture");
        convert(frame, pic);
        let enc = self.enc.as_mut().expect("encoder");
        if keyframe && !fresh {
            enc.force_intra_frame();
        }
        let bits = enc.encode(&*pic).map_err(|e| format!("H.264 encode: {e}"))?;
        let keyframe = matches!(bits.frame_type(), openh264::encoder::FrameType::IDR);
        Ok(Encoded { data: bits.to_vec(), keyframe, width: w, height: h })
    }
}

/// A decoded picture, RGBA.
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct VideoDecoder {
    dec: Decoder,
}

impl VideoDecoder {
    pub fn new() -> Result<Self, String> {
        // Flush after each access unit: a picture comes out as soon as it's complete.
        let config = DecoderConfig::new().flush_after_decode(Flush::Flush);
        Decoder::with_api_config(OpenH264API::from_source(), config).map(|dec| Self { dec }).map_err(|e| format!("H.264 decoder: {e}"))
    }

    /// One access unit (Annex B) → the picture, if one came out.
    pub fn decode(&mut self, au: &[u8]) -> Result<Option<Decoded>, String> {
        let Some(yuv) = self.dec.decode(au).map_err(|e| e.to_string())? else { return Ok(None) };
        let (w, h) = yuv.dimensions();
        if w == 0 || h == 0 {
            return Ok(None);
        }
        let mut rgba = vec![0u8; w * h * 4];
        yuv.write_rgba8(&mut rgba);
        Ok(Some(Decoded { width: w as u32, height: h as u32, rgba }))
    }
}

/// Does this access unit (Annex B) hold an IDR slice (a picture a decoder can start at)?
pub fn has_idr(au: &[u8]) -> bool {
    openh264::nal_units(au).any(|nal| nal_type(nal) == Some(5))
}

/// A NAL unit's type (`nal_units()` hands each one back with its start code).
fn nal_type(nal: &[u8]) -> Option<u8> {
    let i = nal.iter().position(|&b| b == 1)? + 1;
    nal.get(i).map(|b| b & 0x1f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::capture::pattern;

    #[test]
    fn fitting() {
        assert_eq!(fit(3840, 2160, 1920, 1080), (1920, 1080));
        assert_eq!(fit(1280, 720, 1920, 1080), (1280, 720), "never upscaled");
        assert_eq!(fit(2560, 1600, 1920, 1080), (1728, 1080));
        assert_eq!(fit(1001, 777, 4000, 4000), (1000, 776), "even sides");
    }

    #[test]
    fn converts_colours() {
        // Pure red, green, blue, white → BT.601 limited-range values.
        for (bgra, y) in [([0u8, 0, 255, 255], 82u8), ([0, 255, 0, 255], 145), ([255, 0, 0, 255], 41), ([255, 255, 255, 255], 235)] {
            let frame = Frame { width: 4, height: 4, stride: 16, data: bgra.repeat(16) };
            let mut out = I420::new(4, 4);
            convert(&frame, &mut out);
            assert!(out.y.iter().all(|&v| (v as i32 - y as i32).abs() <= 1), "{bgra:?} → Y {:?}", out.y[0]);
        }
    }

    /// The pattern survives a round trip through our encoder and decoder at
    /// the right size, the first picture a keyframe.
    #[test]
    fn round_trip() {
        let mut enc = VideoEncoder::new(2_000_000, 30);
        let mut dec = VideoDecoder::new().unwrap();
        let mut got = None;
        for n in 0..5 {
            let f = pattern::draw(640, 360, n);
            let e = enc.encode(&f, (1920, 1080), n == 0).unwrap();
            if n == 0 {
                assert!(e.keyframe && has_idr(&e.data), "starts with a keyframe");
            }
            assert_eq!((e.width, e.height), (640, 360));
            if let Some(d) = dec.decode(&e.data).unwrap() {
                got = Some(d);
            }
        }
        let d = got.expect("decoded a picture");
        assert_eq!((d.width, d.height), (640, 360));
        // The first colour bar is white(ish).
        assert!(d.rgba[0] > 200 && d.rgba[1] > 200 && d.rgba[2] > 200, "{:?}", &d.rgba[0..4]);
    }

    #[test]
    fn downscales_while_encoding() {
        let mut enc = VideoEncoder::new(4_000_000, 30);
        let e = enc.encode(&pattern::draw(1280, 720, 0), (640, 640), true).unwrap();
        assert_eq!((e.width, e.height), (640, 360));
    }
}
