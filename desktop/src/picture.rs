//! Pictures for messages, like the web's `preparePicture()` (web/js/app.js):
//! compressed by default — at most 1600 px on the long side, re-encoded
//! (JPEG here: there's no pure-Rust lossy WebP encoder; pictures with
//! transparency try PNG first), a phone photo ends up ~150–300 KB. "Full res"
//! sends the original bytes when they fit the 1.9 MB cap (JPEG / PNG / WebP),
//! else the sharpest version that fits. GIFs that fit always go as they are
//! (they stay animated).

use image::{GenericImageView, ImageDecoder, ImageFormat};

/// The server's cap for one picture.
pub const MAX_BYTES: usize = 1_900_000;
/// (longest side, JPEG quality): the web's `COMPRESSED_STEPS` / `FULL_RES_STEPS`.
const COMPRESSED_STEPS: [(u32, u8); 3] = [(1600, 80), (1280, 74), (1024, 68)];
const FULL_RES_STEPS: [(u32, u8); 4] = [(u32::MAX, 92), (3072, 90), (2048, 88), (1600, 82)];

/// What happened to it (the tray says so).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// The bytes you picked, untouched.
    Original,
    /// Made smaller (the default).
    Compressed,
    /// Full res asked, but the original is over the cap: the sharpest that fits.
    Shrunk,
}

#[derive(Clone)]
pub struct Picture {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    pub kind: Kind,
}

impl Picture {
    /// "47 KB, compressed" for the tray.
    pub fn describe(&self) -> String {
        let n = self.bytes.len();
        let size = if n >= 1_000_000 { format!("{:.1} MB", n as f64 / 1e6) } else { format!("{} KB", (n as f64 / 1000.0).round().max(1.0)) };
        match self.kind {
            Kind::Original => format!("{size}, original"),
            Kind::Compressed => format!("{size}, compressed"),
            Kind::Shrunk => format!("{size}, the sharpest that fits"),
        }
    }
}

fn mime_of(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Png => Some("image/png"),
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::Gif => Some("image/gif"),
        ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

/// Decode, turned the right way up (phone photos keep that in EXIF, which
/// re-encoding drops).
fn load(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let bad = || "that isn't a picture Yap can read".to_string();
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|_| bad())?.into_decoder().map_err(|_| bad())?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|_| bad())?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// Make a picture ready to send: compressed (default) or full res.
pub fn prepare(bytes: &[u8], full: bool) -> Result<Picture, String> {
    let format = image::guess_format(bytes).map_err(|_| "use a PNG, GIF, WebP or JPEG".to_string())?;
    let mime = mime_of(format).ok_or_else(|| "use a PNG, GIF, WebP or JPEG".to_string())?;
    let img = load(bytes)?;
    let (w, h) = img.dimensions();
    let original = || Picture { bytes: bytes.to_vec(), mime, width: w, height: h, kind: Kind::Original };
    let fits = bytes.len() <= MAX_BYTES;
    // Small GIFs stay animated; full res = the original when it fits.
    if fits && (format == ImageFormat::Gif || full) {
        return Ok(original());
    }
    let alpha = img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p.0[3] < 255);
    for (side, quality) in if full { &FULL_RES_STEPS[..] } else { &COMPRESSED_STEPS[..] } {
        let scale = (*side as f32 / w.max(h) as f32).min(1.0);
        let (nw, nh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
        let resized = if scale < 1.0 { img.resize_exact(nw, nh, image::imageops::FilterType::CatmullRom) } else { img.clone() };
        let mut out = Vec::new();
        let mut out_mime = "image/jpeg";
        if alpha {
            // Keep the transparency if a PNG of it fits.
            resized.write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png).map_err(|e| e.to_string())?;
            out_mime = "image/png";
        }
        if out.is_empty() || out.len() > MAX_BYTES {
            out.clear();
            out_mime = "image/jpeg";
            // JPEG has no alpha: flatten onto white.
            let rgba = resized.to_rgba8();
            let rgb = image::RgbImage::from_fn(nw, nh, |x, y| {
                let p = rgba.get_pixel(x, y).0;
                let a = p[3] as u32;
                let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
                image::Rgb([mix(p[0]), mix(p[1]), mix(p[2])])
            });
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, *quality).encode_image(&rgb).map_err(|e| e.to_string())?;
        }
        if out.len() <= MAX_BYTES {
            // Already small and no bigger than our copy: the original is the better deal.
            if fits && scale >= 1.0 && format != ImageFormat::Gif && bytes.len() <= out.len() {
                return Ok(original());
            }
            let kind = if full { Kind::Shrunk } else { Kind::Compressed };
            return Ok(Picture { bytes: out, mime: out_mime, width: nw, height: nh, kind });
        }
    }
    Err("that picture is too big".into())
}

/// A picture being prepared in the background: None while it works.
pub type PrepSlot = std::sync::Arc<parking_lot::Mutex<Option<Result<Picture, String>>>>;

/// Shrink/re-encode on a worker thread (so it's ready by the time you hit
/// Send); `thumb` (if given) gets a small preview first.
pub fn prepare_in_background(bytes: std::sync::Arc<Vec<u8>>, full: bool, thumb: Option<std::sync::Arc<parking_lot::Mutex<Option<image::RgbaImage>>>>, done: impl Fn() + Send + 'static) -> PrepSlot {
    let slot: PrepSlot = std::sync::Arc::new(parking_lot::Mutex::new(None));
    let out = slot.clone();
    let _ = std::thread::Builder::new().name("yap-picture".into()).spawn(move || {
        if let Some(thumb) = thumb
            && let Ok(img) = load(&bytes)
        {
            *thumb.lock() = Some(img.thumbnail(160, 160).to_rgba8());
            done();
        }
        *out.lock() = Some(prepare(&bytes, full));
        done();
    });
    slot
}

/// Clipboard pixels (RGBA) as a PNG file.
pub fn png_from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Vec<u8>, String> {
    let img = image::RgbaImage::from_raw(width, height, rgba).ok_or("bad clipboard image")?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut img = image::RgbaImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            // Noise-ish so it doesn't compress to nothing.
            let v = ((x * 7919 + y * 104729) % 251) as u8;
            *p = image::Rgba([v, v.wrapping_mul(3), v.wrapping_mul(7), 255]);
        }
        png_from_rgba(w, h, img.into_raw()).unwrap()
    }

    /// A smooth photo-like gradient (compresses like a real picture).
    fn gradient_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, 128, 255]));
        png_from_rgba(w, h, img.into_raw()).unwrap()
    }

    #[test]
    fn compressed_by_default_to_1600() {
        let p = prepare(&gradient_png(3000, 1500), false).unwrap();
        assert_eq!((p.mime, p.kind), ("image/jpeg", Kind::Compressed));
        assert_eq!((p.width, p.height), (1600, 800));
        assert!(p.describe().ends_with(", compressed"), "{}", p.describe());
        // A noisy one still lands at ≤ 1600 and under the cap, same shape.
        let p = prepare(&png(3000, 2000), false).unwrap();
        assert!(p.bytes.len() <= MAX_BYTES && p.width <= 1600 && (p.width as f32 / p.height as f32 - 1.5).abs() < 0.01, "{}x{}", p.width, p.height);
    }

    #[test]
    fn full_res_sends_the_original_when_it_fits() {
        let bytes = gradient_png(3000, 1500);
        assert!(bytes.len() <= MAX_BYTES);
        let p = prepare(&bytes, true).unwrap();
        assert_eq!((p.kind, p.mime, p.width), (Kind::Original, "image/png", 3000));
        assert_eq!(p.bytes, bytes);
        // Too big as it is: the sharpest version that fits.
        let big = png(3000, 2000);
        assert!(big.len() > MAX_BYTES);
        let p = prepare(&big, true).unwrap();
        assert_eq!(p.kind, Kind::Shrunk);
        assert!(p.bytes.len() <= MAX_BYTES && p.width >= 1600, "{}x{}", p.width, p.height);
    }

    #[test]
    fn small_pictures_stay_when_our_copy_is_no_smaller() {
        let bytes = png(64, 48);
        let p = prepare(&bytes, false).unwrap();
        assert_eq!(p.width, 64);
        assert!(p.bytes.len() <= bytes.len());
    }

    #[test]
    fn small_gifs_go_as_they_are() {
        let mut out = Vec::new();
        {
            let mut enc = gif::Encoder::new(&mut out, 4, 4, &[0, 0, 0, 255, 255, 255]).unwrap();
            for i in 0..2u8 {
                enc.write_frame(&gif::Frame { width: 4, height: 4, buffer: std::borrow::Cow::Owned(vec![i; 16]), delay: 10, ..Default::default() }).unwrap();
            }
        }
        let p = prepare(&out, false).unwrap();
        assert_eq!((p.mime, p.kind), ("image/gif", Kind::Original));
        assert_eq!(p.bytes, out);
    }

    #[test]
    fn transparency_survives() {
        let img = image::RgbaImage::from_fn(2000, 1000, |x, _| image::Rgba([200, 30, 30, if x < 1000 { 0 } else { 255 }]));
        let bytes = png_from_rgba(2000, 1000, img.into_raw()).unwrap();
        let p = prepare(&bytes, false).unwrap();
        assert_eq!((p.mime, p.width), ("image/png", 1600));
    }

    #[test]
    fn rejects_non_images() {
        assert!(prepare(b"hello", false).is_err());
    }
}
