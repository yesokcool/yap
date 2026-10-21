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

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_SIDE: u32 = 512;
/// The picture viewer: big, but still a texture every GPU takes.
const MAX_FULL: u32 = 2560;
const MAX_FRAMES: usize = 150;
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
            ureq::Error::BodyExceedsLimit(_) | ureq::Error::BadUri(_) => false,
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

/// `fit`: the longest side it may have (None = as it is: pixel art).
fn to_color_image(img: image::RgbaImage, fit: Option<u32>) -> ColorImage {
    let img = if let Some(max) = fit.filter(|max| img.width() > *max || img.height() > *max) {
        let scale = max as f32 / img.width().max(img.height()) as f32;
        image::imageops::resize(&img, (img.width() as f32 * scale).max(1.0) as u32, (img.height() as f32 * scale).max(1.0) as u32, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

fn options(key: &str) -> TextureOptions {
    if key.contains("#pixel") { TextureOptions::NEAREST } else { TextureOptions::LINEAR }
}

/// Buddy icons: a center-cropped 128 px square.
fn decode_square(bytes: &[u8]) -> Decoded {
    let Ok(img) = image::load_from_memory(bytes) else { return Decoded::Failed };
    let side = img.width().min(img.height());
    let sq = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side);
    let px = sq.resize_exact(128, 128, image::imageops::FilterType::Triangle).to_rgba8();
    Decoded::Static(ColorImage::from_rgba_unmultiplied([128, 128], px.as_raw()))
}

/// `fit`: scale down to this (None: pixel art keeps its size, up to 2048).
fn decode(bytes: &[u8], animate: bool, fit: Option<u32>) -> Decoded {
    use image::AnimationDecoder;
    let fmt = image::guess_format(bytes).ok();
    if animate {
        let frames = match fmt {
            Some(image::ImageFormat::Gif) => image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))
                .ok()
                .map(|d| d.into_frames().take(MAX_FRAMES).filter_map(Result::ok).collect::<Vec<_>>()),
            Some(image::ImageFormat::WebP) => image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes))
                .ok()
                .filter(|d| d.has_animation())
                .map(|d| d.into_frames().take(MAX_FRAMES).filter_map(Result::ok).collect::<Vec<_>>()),
            _ => None,
        };
        if let Some(frames) = frames.filter(|f| f.len() > 1) {
            return Decoded::Animated(
                frames
                    .into_iter()
                    .map(|f| {
                        let (n, d) = f.delay().numer_denom_ms();
                        let ms = n.checked_div(d).map_or(100, |ms| ms.max(20));
                        (to_color_image(f.into_buffer(), fit), Duration::from_millis(ms as u64))
                    })
                    .collect(),
            );
        }
    }
    match image::load_from_memory(bytes) {
        Ok(img) if fit.is_none() && (img.width() > 2048 || img.height() > 2048) => Decoded::Failed,
        Ok(img) => Decoded::Static(to_color_image(img.to_rgba8(), fit)),
        Err(_) => Decoded::Failed,
    }
}

impl Images {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let done: Arc<Mutex<Vec<(String, Decoded)>>> = Arc::new(Mutex::new(Vec::new()));
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
        for n in 0..4 {
            let rx = rx.clone();
            let done = done.clone();
            let ctx = ctx.clone();
            let agent = agent.clone();
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
                            decode(&bytes, animate, fit)
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
        Self { ctx: ctx.clone(), slots: Arc::new(Mutex::new(HashMap::new())), queue: tx, done, last_evict: Instant::now() }
    }

    /// Upload decoded images; drop ones unused for a while. Call once per frame.
    pub fn pump(&mut self) {
        let done: Vec<(String, Decoded)> = std::mem::take(&mut *self.done.lock());
        let mut slots = self.slots.lock();
        for (key, decoded) in done {
            let entry = match decoded {
                Decoded::Static(img) => {
                    let size = egui::vec2(img.size[0] as f32, img.size[1] as f32);
                    Entry::Static { tex: self.ctx.load_texture(&key, img, options(&key)), size }
                }
                Decoded::Animated(frames) => {
                    let size = egui::vec2(frames[0].0.size[0] as f32, frames[0].0.size[1] as f32);
                    let total = frames.iter().map(|f| f.1).sum();
                    let frames = frames
                        .into_iter()
                        .enumerate()
                        .map(|(i, (img, d))| (self.ctx.load_texture(format!("{key}#{i}"), img, options(&key)), d))
                        .collect();
                    Entry::Animated { frames, total, size }
                }
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

    /// What's worth trying again: no network, a timeout, the server's 5xx; not a 4xx or a file over the limit.
    #[test]
    fn passing_failures_go_again() {
        let io = || ureq::Error::Io(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"));
        assert!(FetchError::new(io()).passing);
        assert!(FetchError::new(ureq::Error::StatusCode(503)).passing);
        assert!(!FetchError::new(ureq::Error::StatusCode(404)).passing);
        assert!(!FetchError::new(ureq::Error::BodyExceedsLimit(8 << 20)).passing);
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
