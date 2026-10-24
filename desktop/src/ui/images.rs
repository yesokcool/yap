//! Remote images (custom emojis, GIF messages): fetched on a few worker
//! threads, decoded with `image`, uploaded as textures.
//!
//! Static first frame by default; animation only on request (hover), so an
//! animated emoji on screen doesn't keep the whole UI repainting. Emoji images
//! are content-addressed and immutable, so they're also cached on disk. KLIPY
//! media is never written to disk (their terms). One HTTP agent for all of
//! them, so a screenful of emojis shares a few connections instead of a TLS
//! handshake each. A picture that couldn't be fetched (offline, the server
//! busy) is tried again a little later; one that's gone or isn't a picture
//! isn't.
//!
//! Whatever the bytes claim, decoding stays bounded: `image::Limits` on every
//! decoder, animations over 2048 px a side show still, each frame is shrunk as
//! soon as it's decoded (all of them together keep at most 192 MB), and no
//! texture is ever bigger than the GPU takes (`fit_texture`).

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_SIDE: u32 = 512;
/// The picture viewer: big, but still a texture every GPU takes.
const MAX_FULL: u32 = 2560;
/// Pixel art (saved drawings) keeps its own size, up to this.
const MAX_PIXEL: u32 = 2048;
const MAX_FRAMES: usize = 150;
/// An animation bigger than this on a side shows still (its first frame).
const MAX_ANIM_SIDE: u32 = 2048;
/// Decoding work for one animation (canvas pixels × frames): the frames past it are left out.
const MAX_ANIM_PIXELS: u64 = 200_000_000;
/// What one animation's frames keep, all together, once shrunk: past it every frame is halved again.
const MAX_ANIM_BYTES: usize = 192 << 20;
/// What any one decode may allocate, and the biggest picture it takes.
const MAX_ALLOC: u64 = 320 << 20;
const MAX_DIMENSION: u32 = 16_384;
const EVICT_AFTER: Duration = Duration::from_secs(120);
/// A fetch that failed for a passing reason goes again after this (while it's still on screen).
const RETRY_AFTER: Duration = Duration::from_secs(15);

enum Entry {
    Loading,
    /// `retry`: when to fetch it again (None: for good — gone, or not a picture).
    Failed { retry: Option<Instant> },
    Static { tex: TextureHandle, size: egui::Vec2 },
    Animated { frames: Vec<(TextureHandle, Duration)>, total: Duration, size: egui::Vec2 },
}

struct Slot {
    entry: Entry,
    used: Instant,
}

impl Slot {
    /// A fetch that failed for a passing reason, due to go again.
    fn retry_due(&self) -> bool {
        matches!(self.entry, Entry::Failed { retry: Some(at) } if Instant::now() >= at)
    }
}

enum Decoded {
    Static(ColorImage),
    Animated(Vec<(ColorImage, Duration)>),
    Failed,
    /// Couldn't be fetched just now (no network, a timeout, the server's 5xx).
    Unreachable,
}

/// (cache key, URL, animate, decrypt with: a Personal space key for sealed pictures).
type Job = (String, String, bool, Option<crate::vault::Key>);

pub struct Images {
    ctx: egui::Context,
    slots: Arc<Mutex<HashMap<String, Slot>>>,
    queue: mpsc::Sender<Job>,
    done: Arc<Mutex<Vec<(String, Decoded)>>>,
    last_evict: Instant,
    /// The GPU's biggest texture side, for the decoding threads (egui's
    /// default until the first frame says).
    max_texture: Arc<AtomicUsize>,
}

fn cache_path(url: &str) -> Option<std::path::PathBuf> {
    let id = url.rsplit_once("/emoji/").or_else(|| url.rsplit_once("/media/"))?.1;
    (id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())).then(|| dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("yap").join("emoji").join(id))
}

/// Why a fetch failed, and whether trying again later could help.
struct FetchError {
    why: String,
    passing: bool,
}

impl FetchError {
    fn new(e: ureq::Error) -> Self {
        // 4xx: gone or refused, for good. Too big: it'll stay too big. The rest (I/O, timeouts, 5xx) passes.
        let passing = match &e {
            ureq::Error::StatusCode(code) => *code >= 500,
            ureq::Error::BodyExceedsLimit(_) | ureq::Error::BadUri(_) | ureq::Error::RequireHttpsOnly(_) => false,
            _ => true,
        };
        Self { why: e.to_string(), passing }
    }

    fn lasting(why: String) -> Self {
        Self { why, passing: false }
    }
}

fn fetch(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, FetchError> {
    if let Some(path) = cache_path(url)
        && let Ok(bytes) = std::fs::read(&path)
    {
        return Ok(bytes);
    }
    let mut res = agent.get(url).call().map_err(FetchError::new)?;
    let bytes = res.body_mut().with_config().limit(8 * 1024 * 1024).read_to_vec().map_err(FetchError::new)?;
    if let Some(path) = cache_path(url) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, &bytes);
    }
    Ok(bytes)
}

/// The limits every decoder here gets: a picture claiming to be huge is refused
/// before anything is allocated for it.
fn limits() -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(MAX_DIMENSION);
    l.max_image_height = Some(MAX_DIMENSION);
    l.max_alloc = Some(MAX_ALLOC);
    l
}

/// Decode a still picture (the first frame of an animation), within `limits()`.
fn load(bytes: &[u8]) -> Option<image::DynamicImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits());
    reader.decode().ok()
}

/// `img` shrunk (same shape) so neither side is over `max`.
fn shrink(img: image::RgbaImage, max: u32) -> image::RgbaImage {
    if img.width() <= max && img.height() <= max {
        return img;
    }
    let scale = max as f64 / img.width().max(img.height()) as f64;
    let side = |v: u32| ((v as f64 * scale) as u32).clamp(1, max.max(1));
    image::imageops::resize(&img, side(img.width()), side(img.height()), image::imageops::FilterType::Triangle)
}

/// `fit`: the longest side it may have (None = as it is: pixel art).
fn to_color_image(img: image::RgbaImage, fit: Option<u32>) -> ColorImage {
    let img = match fit {
        Some(max) => shrink(img, max),
        None => img,
    };
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

/// `img`, shrunk to `max` px a side if it's bigger: the biggest texture this
/// GPU takes (egui's `max_texture_side`; a bigger one aborts the app). Every
/// picture from elsewhere goes through this on its way to a texture.
pub fn fit_texture(img: ColorImage, max: usize) -> ColorImage {
    let [w, h] = img.size;
    if (w <= max && h <= max) || max == 0 {
        return img;
    }
    let Some(rgba) = image::RgbaImage::from_raw(w as u32, h as u32, img.as_raw().to_vec()) else { return ColorImage::filled([1, 1], egui::Color32::TRANSPARENT) };
    let small = shrink(rgba, max as u32);
    // (ColorImage is premultiplied already, and stays so.)
    ColorImage::from_rgba_premultiplied([small.width() as usize, small.height() as usize], small.as_raw())
}

/// An animation's frames (GIF or WebP) and its canvas, if it is one, within
/// `limits()` and no bigger than `MAX_ANIM_SIDE`.
fn frames(bytes: &[u8], fmt: Option<image::ImageFormat>) -> Option<(image::Frames<'_>, (u32, u32))> {
    use image::{AnimationDecoder, ImageDecoder};
    let cursor = std::io::Cursor::new(bytes);
    // (Neither allocates anything until its first frame is asked for.)
    let (frames, size) = match fmt? {
        image::ImageFormat::Gif => {
            let mut d = image::codecs::gif::GifDecoder::new(cursor).ok()?;
            d.set_limits(limits()).ok()?;
            let size = d.dimensions();
            (d.into_frames(), size)
        }
        image::ImageFormat::WebP => {
            let mut d = image::codecs::webp::WebPDecoder::new(cursor).ok().filter(|d| d.has_animation())?;
            d.set_limits(limits()).ok()?;
            let size = d.dimensions();
            (d.into_frames(), size)
        }
        _ => return None,
    };
    (size.0 <= MAX_ANIM_SIDE && size.1 <= MAX_ANIM_SIDE).then_some((frames, size))
}

/// Up to `MAX_FRAMES` frames (fewer for a big canvas: `MAX_ANIM_PIXELS`),
/// each shrunk to `fit` as soon as it's decoded; while all of them together
/// would keep more than `MAX_ANIM_BYTES`, every frame is halved again.
fn collect_frames(frames: image::Frames<'_>, canvas: (u32, u32), fit: u32) -> Vec<(image::RgbaImage, Duration)> {
    let px = (canvas.0 as u64 * canvas.1 as u64).max(1);
    let most = MAX_FRAMES.min((MAX_ANIM_PIXELS / px).max(1) as usize);
    let (mut side, mut kept, mut out) = (fit.min(canvas.0.max(canvas.1).max(1)), 0usize, Vec::new());
    for frame in frames.take(most) {
        let Ok(f) = frame else { continue };
        let (n, d) = f.delay().numer_denom_ms();
        let ms = n.checked_div(d).map_or(100, |ms| ms.max(20));
        let img = shrink(f.into_buffer(), side);
        kept += img.as_raw().len();
        out.push((img, Duration::from_millis(ms as u64)));
        while kept > MAX_ANIM_BYTES && side > 16 {
            side /= 2;
            for (img, _) in out.iter_mut() {
                *img = shrink(std::mem::take(img), side);
            }
            kept = out.iter().map(|(img, _)| img.as_raw().len()).sum();
        }
    }
    out
}

fn options(key: &str) -> TextureOptions {
    if key.contains("#pixel") { TextureOptions::NEAREST } else { TextureOptions::LINEAR }
}

/// Buddy icons: a center-cropped 128 px square.
fn decode_square(bytes: &[u8]) -> Decoded {
    let Some(img) = load(bytes) else { return Decoded::Failed };
    let side = img.width().min(img.height());
    let sq = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side);
    let px = sq.resize_exact(128, 128, image::imageops::FilterType::Triangle).to_rgba8();
    Decoded::Static(ColorImage::from_rgba_unmultiplied([128, 128], px.as_raw()))
}

/// `fit`: scale down to this (None: pixel art keeps its size, up to `MAX_PIXEL`).
/// `max_texture`: the GPU's biggest texture side; nothing comes out bigger.
fn decode(bytes: &[u8], animate: bool, fit: Option<u32>, max_texture: u32) -> Decoded {
    let fmt = image::guess_format(bytes).ok();
    let pixel_max = MAX_PIXEL.min(max_texture);
    if animate && let Some((frames, canvas)) = frames(bytes, fmt) {
        // Pixel art that wouldn't fit shows still (and then fails, below), like a still one.
        if fit.is_some() || (canvas.0 <= pixel_max && canvas.1 <= pixel_max) {
            let frames = collect_frames(frames, canvas, fit.unwrap_or(pixel_max).min(max_texture));
            if frames.len() > 1 {
                return Decoded::Animated(frames.into_iter().map(|(img, d)| (to_color_image(img, None), d)).collect());
            }
        }
    }
    match load(bytes) {
        Some(img) if fit.is_none() && (img.width() > pixel_max || img.height() > pixel_max) => Decoded::Failed,
        Some(img) => Decoded::Static(to_color_image(img.to_rgba8(), fit.map(|f| f.min(max_texture)))),
        None => Decoded::Failed,
    }
}

impl Images {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let done: Arc<Mutex<Vec<(String, Decoded)>>> = Arc::new(Mutex::new(Vec::new()));
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).middleware(crate::http::secure_only).build().into();
        let max_texture = Arc::new(AtomicUsize::new(ctx.input(|i| i.max_texture_side)));
        for n in 0..4 {
            let rx = rx.clone();
            let done = done.clone();
            let ctx = ctx.clone();
            let agent = agent.clone();
            let max_texture = max_texture.clone();
            let _ = std::thread::Builder::new().name(format!("yap-img-{n}")).spawn(move || {
                loop {
                    let job = rx.lock().recv();
                    let Ok((key, url, animate, seal)) = job else { return };
                    // A sealed picture: opaque bytes until this device's key opens them.
                    let fetched = fetch(&agent, &url).and_then(|b| match &seal {
                        Some(k) => crate::vault::decrypt_blob(k, &b).map_err(FetchError::lasting),
                        None => Ok(b),
                    });
                    let decoded = match fetched {
                        Ok(bytes) if key.ends_with("#square") => decode_square(&bytes),
                        Ok(bytes) => {
                            let fit = if key.contains("#pixel") {
                                None
                            } else if key.contains("#full") {
                                Some(MAX_FULL)
                            } else {
                                Some(MAX_SIDE)
                            };
                            decode(&bytes, animate, fit, max_texture.load(Ordering::Relaxed).min(u32::MAX as usize) as u32)
                        }
                        Err(e) => {
                            log::debug!("image {url}: {}{}", e.why, if e.passing { " (trying again later)" } else { "" });
                            if e.passing { Decoded::Unreachable } else { Decoded::Failed }
                        }
                    };
                    done.lock().push((key, decoded));
                    ctx.request_repaint();
                }
            });
        }
        Self { ctx: ctx.clone(), slots: Arc::new(Mutex::new(HashMap::new())), queue: tx, done, last_evict: Instant::now(), max_texture }
    }

    /// Upload decoded images; drop ones unused for a while. Call once per frame.
    pub fn pump(&mut self) {
        let max = self.ctx.input(|i| i.max_texture_side);
        self.max_texture.store(max, Ordering::Relaxed);
        let done: Vec<(String, Decoded)> = std::mem::take(&mut *self.done.lock());
        let mut slots = self.slots.lock();
        for (key, decoded) in done {
            let entry = match decoded {
                Decoded::Static(img) => {
                    let img = fit_texture(img, max);
                    let size = egui::vec2(img.size[0] as f32, img.size[1] as f32);
                    Entry::Static { tex: self.ctx.load_texture(&key, img, options(&key)), size }
                }
                Decoded::Animated(frames) if !frames.is_empty() => {
                    let frames: Vec<(ColorImage, Duration)> = frames.into_iter().map(|(img, d)| (fit_texture(img, max), d)).collect();
                    let size = egui::vec2(frames[0].0.size[0] as f32, frames[0].0.size[1] as f32);
                    let total = frames.iter().map(|f| f.1).sum();
                    let frames = frames
                        .into_iter()
                        .enumerate()
                        .map(|(i, (img, d))| (self.ctx.load_texture(format!("{key}#{i}"), img, options(&key)), d))
                        .collect();
                    Entry::Animated { frames, total, size }
                }
                Decoded::Animated(_) => Entry::Failed { retry: None },
                Decoded::Failed => Entry::Failed { retry: None },
                Decoded::Unreachable => Entry::Failed { retry: Some(Instant::now() + RETRY_AFTER) },
            };
            slots.insert(key, Slot { entry, used: Instant::now() });
        }
        if self.last_evict.elapsed() > Duration::from_secs(10) {
            self.last_evict = Instant::now();
            slots.retain(|_, s| matches!(s.entry, Entry::Loading) || s.used.elapsed() < EVICT_AFTER);
        }
    }

    /// The texture to draw now (and its pixel size), starting a load if needed.
    /// `animate`: play animated GIF/WebP (requests repaints while drawn).
    /// A square, 128 px version of an image (buddy icons).
    pub fn get_square(&self, url: &str) -> Option<egui::TextureId> {
        let key = format!("{url}#square");
        let mut slots = self.slots.lock();
        match slots.get_mut(&key).filter(|s| !s.retry_due()) {
            None => {
                slots.insert(key.clone(), Slot { entry: Entry::Loading, used: Instant::now() });
                let _ = self.queue.send((key, url.to_string(), false, None));
                None
            }
            Some(slot) => {
                slot.used = Instant::now();
                match &slot.entry {
                    Entry::Static { tex, .. } => Some(tex.id()),
                    _ => None,
                }
            }
        }
    }

    pub fn get(&self, url: &str, animate: bool) -> Option<(egui::TextureId, egui::Vec2)> {
        self.get_with(url, animate, None)
    }

    /// Pixel art (saved drawings): full size, drawn nearest-neighbour.
    pub fn get_pixel(&self, url: &str, animate: bool) -> Option<(egui::TextureId, egui::Vec2)> {
        self.get_with(&format!("{url}#pixel"), animate, None)
    }

    /// A sealed (encrypted Personal space) picture, opened with `key`.
    pub fn get_sealed(&self, url: &str, key: crate::vault::Key) -> Option<(egui::TextureId, egui::Vec2)> {
        self.get_with(url, false, Some(key))
    }

    /// The picture viewer: up to 2560 px (GIFs play), sealed ones opened with `key`.
    pub fn get_full(&self, url: &str, key: Option<crate::vault::Key>) -> Option<(egui::TextureId, egui::Vec2)> {
        self.get_with(&format!("{url}#full"), key.is_none(), key)
    }

    /// Whether the viewer's copy failed to load / open.
    pub fn failed_full(&self, url: &str, sealed: bool) -> bool {
        let key = if sealed { format!("{url}#full#sealed") } else { format!("{url}#full#anim") };
        self.slots.lock().get(&key).is_some_and(|s| matches!(s.entry, Entry::Failed { .. }))
    }

    /// Whether a sealed picture failed to open (not just still loading).
    pub fn failed(&self, url: &str, sealed: bool) -> bool {
        let key = if sealed { format!("{url}#sealed") } else { url.to_string() };
        self.slots.lock().get(&key).is_some_and(|s| matches!(s.entry, Entry::Failed { .. }))
    }

    fn get_with(&self, url: &str, animate: bool, seal: Option<crate::vault::Key>) -> Option<(egui::TextureId, egui::Vec2)> {
        let key = if seal.is_some() {
            format!("{url}#sealed")
        } else if animate {
            format!("{url}#anim")
        } else {
            url.to_string()
        };
        let mut slots = self.slots.lock();
        match slots.get_mut(&key).filter(|s| !s.retry_due()) {
            None => {
                slots.insert(key.clone(), Slot { entry: Entry::Loading, used: Instant::now() });
                let _ = self.queue.send((key, url.trim_end_matches("#pixel").trim_end_matches("#full").to_string(), animate, seal));
                None
            }
            Some(slot) => {
                slot.used = Instant::now();
                match &slot.entry {
                    Entry::Loading | Entry::Failed { .. } => None,
                    Entry::Static { tex, size } => Some((tex.id(), *size)),
                    Entry::Animated { frames, total, size } => {
                        let t = self.ctx.input(|i| i.time);
                        let total_ms = total.as_millis().max(1) as u64;
                        let mut at = ((t * 1000.0) as u64) % total_ms;
                        for (tex, d) in frames {
                            let d_ms = d.as_millis() as u64;
                            if at < d_ms {
                                self.ctx.request_repaint_after(Duration::from_millis(d_ms - at));
                                return Some((tex.id(), *size));
                            }
                            at -= d_ms;
                        }
                        Some((frames[0].0.id(), *size))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GIF with a `w` × `h` canvas and `n` one-pixel frames (a few bytes each).
    fn gif(w: u16, h: u16, n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut e = gif::Encoder::new(&mut out, w, h, &[0, 0, 0, 255, 255, 255]).unwrap();
            for i in 0..n {
                let frame = gif::Frame { width: 1, height: 1, left: (i % w as usize) as u16, delay: 2, buffer: std::borrow::Cow::Owned(vec![1]), ..Default::default() };
                e.write_frame(&frame).unwrap();
            }
        }
        out
    }

    fn sides(d: &Decoded) -> Vec<[usize; 2]> {
        match d {
            Decoded::Static(img) => vec![img.size],
            Decoded::Animated(frames) => frames.iter().map(|(img, _)| img.size).collect(),
            _ => vec![],
        }
    }

    /// A normal animated GIF decodes exactly as before: every frame, its own size.
    #[test]
    fn normal_animations_are_untouched() {
        let d = decode(&gif(64, 48, 3), true, Some(MAX_SIDE), 8192);
        assert!(matches!(d, Decoded::Animated(_)));
        assert_eq!(sides(&d), vec![[64, 48]; 3]);
        assert_eq!(sides(&decode(&gif(64, 48, 3), false, Some(MAX_SIDE), 8192)), vec![[64, 48]]);
    }

    /// A few KB claiming a 4096 × 4096 canvas and 150 frames (10 GB of frames
    /// once): too big to animate, so it shows still, shrunk.
    #[test]
    fn huge_animation_canvases_show_still() {
        let bytes = gif(4096, 4096, 150);
        assert!(bytes.len() < 4096, "{} bytes", bytes.len());
        let d = decode(&bytes, true, Some(MAX_SIDE), 8192);
        assert!(matches!(d, Decoded::Static(_)));
        assert_eq!(sides(&d), vec![[512, 512]]);
    }

    /// Frames are shrunk as they're decoded, and all of them together stay within the budget.
    #[test]
    fn animation_frames_stay_within_budget() {
        let bytes = gif(1024, 1024, 150);
        let (frames, canvas) = frames_of(&bytes);
        let out = collect_frames(frames, canvas, MAX_FULL);
        assert_eq!(out.len(), 150);
        assert!(out.iter().map(|(img, _)| img.as_raw().len()).sum::<usize>() <= MAX_ANIM_BYTES);
        assert!(out.iter().all(|(img, _)| img.dimensions() == out[0].0.dimensions() && img.width() < 1024));
        // A big canvas gets fewer frames: no more decoding work than MAX_ANIM_PIXELS.
        let bytes = gif(2048, 2048, 150);
        let (frames, canvas) = frames_of(&bytes);
        assert_eq!(collect_frames(frames, canvas, MAX_FULL).len(), (MAX_ANIM_PIXELS / (2048 * 2048)) as usize);
    }

    fn frames_of(bytes: &[u8]) -> (image::Frames<'_>, (u32, u32)) {
        frames(bytes, Some(image::ImageFormat::Gif)).unwrap()
    }

    /// Pixel art keeps its size, but not past 2048 px or what the GPU takes:
    /// a 66-byte 20000 × 16 GIF once made a texture egui aborts on.
    #[test]
    fn pixel_art_is_never_bigger_than_a_texture() {
        let wide = gif(20_000, 16, 2);
        assert!(matches!(decode(&wide, true, None, 8192), Decoded::Failed));
        assert!(matches!(decode(&wide, false, None, 8192), Decoded::Failed));
        assert!(matches!(decode(&gif(1500, 16, 2), true, None, 1024), Decoded::Failed));
        assert_eq!(sides(&decode(&gif(320, 200, 2), true, None, 8192)), vec![[320, 200]; 2]);
        // Shrunk to fit the GPU when it's smaller than our own limits.
        assert_eq!(sides(&decode(&gif(4000, 1000, 1), false, Some(MAX_FULL), 1024)), vec![[1024, 256]]);
    }

    /// Whatever reaches a texture fits the GPU's biggest side.
    #[test]
    fn textures_fit_the_gpu() {
        let img = ColorImage::filled([5000, 10], egui::Color32::RED);
        assert_eq!(fit_texture(img, 2048).size, [2048, 4]);
        let img = ColorImage::filled([300, 200], egui::Color32::RED);
        assert_eq!(fit_texture(img, 2048).size, [300, 200]);
        let img = ColorImage::filled([1, 9000], egui::Color32::RED);
        assert_eq!(fit_texture(img, 8192).size, [1, 8192]);
    }

    /// A picture that claims to be bigger than any decoder here allocates is refused up front.
    #[test]
    fn decoders_have_limits() {
        assert!(load(&gif(4096, 4096, 1)).is_some());
        // Wider than any picture we take, then too much memory (1 GB) for one decode.
        for huge in [gif(20_000, 20_000, 1), gif(16_000, 16_000, 1)] {
            assert!(load(&huge).is_none());
            assert!(matches!(decode_square(&huge), Decoded::Failed));
            assert!(matches!(decode(&huge, true, Some(MAX_FULL), 8192), Decoded::Failed));
        }
    }

    /// What's worth trying again: no network, a timeout, the server's 5xx; not a 4xx or a file over the limit.
    #[test]
    fn passing_failures_go_again() {
        let io = || ureq::Error::Io(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"));
        assert!(FetchError::new(io()).passing);
        assert!(FetchError::new(ureq::Error::StatusCode(503)).passing);
        assert!(!FetchError::new(ureq::Error::StatusCode(404)).passing);
        assert!(!FetchError::new(ureq::Error::BodyExceedsLimit(8 << 20)).passing);
        assert!(!FetchError::new(ureq::Error::RequireHttpsOnly("http://x".into())).passing);
        assert!(!FetchError::lasting("can't open it".into()).passing);
        // A real refused connection: tried again later, then due once its time has come.
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(2))).build().into();
        let e = fetch(&agent, "http://127.0.0.1:9/x.png").expect_err("nothing listens on port 9");
        assert!(e.passing, "{}", e.why);
        let slot = |at| Slot { entry: Entry::Failed { retry: Some(at) }, used: Instant::now() };
        assert!(!slot(Instant::now() + RETRY_AFTER).retry_due());
        assert!(slot(Instant::now() - Duration::from_millis(1)).retry_due());
        assert!(!Slot { entry: Entry::Failed { retry: None }, used: Instant::now() }.retry_due());
    }
}
