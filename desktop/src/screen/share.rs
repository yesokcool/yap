//! Our own screen share. Nothing is captured or encoded until someone
//! watches: the first viewer starts a worker thread (capture → scale → H.264,
//! and the computer's sound → stereo Opus), the last one leaving stops it.
//! One encoder serves every viewer; each peer connection just forwards the
//! same pictures (and sound) to its own track.

use super::Quality;
use super::capture::{self, Frame, Options, Source};
use super::codec::{Encoded, VideoEncoder};
use bytes::Bytes;
use parking_lot::{Condvar, Mutex};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};

/// A static screen still sends a picture this often, so viewers never take
/// it for a paused share (they say "paused" after 3 s without one).
const REPEAT_EVERY: Duration = Duration::from_millis(500);
/// Share sound is music: stereo, and at least this rich (PROTOCOL.md › Screen sharing).
pub const SOUND_BITRATE: u32 = 160_000;

/// One encoded picture, for every viewer's track.
pub struct EncodedVideo {
    pub data: Bytes,
    pub keyframe: bool,
    pub at: Instant,
}

/// One 20 ms Opus packet of the share's sound.
pub struct EncodedAudio {
    pub data: Bytes,
}

/// What the core hears about our share.
#[derive(Debug, Clone)]
pub enum ShareEvent {
    /// Capturing stopped by itself (the window closed, permission revoked…), in plain words.
    Ended(String),
}

#[derive(Clone)]
struct Wanted {
    source: Option<Source>,
    quality: Quality,
    sound: bool,
    /// Bumped on every change (the worker notices and follows).
    generation: u64,
}

struct Worker {
    stop: Arc<AtomicBool>,
    wake: Arc<(Mutex<Option<Frame>>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

pub struct Share {
    wanted: Arc<Mutex<Wanted>>,
    /// Peer connections sending our share right now.
    viewers: Mutex<HashSet<u64>>,
    /// How much of our video each viewer loses (0–1), from their reports.
    loss: Arc<Mutex<HashMap<u64, f64>>>,
    pub video: broadcast::Sender<Arc<EncodedVideo>>,
    pub audio: broadcast::Sender<Arc<EncodedAudio>>,
    keyframe: Arc<AtomicBool>,
    worker: Mutex<Option<Worker>>,
    events: Mutex<Option<mpsc::UnboundedSender<ShareEvent>>>,
    /// The share's stream id (`a=msid:<this> <track>`): the web takes a
    /// track that arrives with a stream for a screen share.
    pub stream_id: String,
    /// Pictures encoded so far (headless stats), and the last one's size.
    encoded: Arc<AtomicU64>,
    size: Arc<Mutex<(u32, u32)>>,
}

impl Share {
    pub fn new() -> Arc<Self> {
        let (video, _) = broadcast::channel(8);
        let (audio, _) = broadcast::channel(64);
        Arc::new(Self {
            wanted: Arc::new(Mutex::new(Wanted { source: None, quality: Quality::Smooth, sound: false, generation: 0 })),
            viewers: Mutex::new(HashSet::new()),
            loss: Arc::new(Mutex::new(HashMap::new())),
            video,
            audio,
            keyframe: Arc::new(AtomicBool::new(false)),
            worker: Mutex::new(None),
            events: Mutex::new(None),
            stream_id: format!("yap-screen-{}", crate::config::random_hex(6)),
            encoded: Arc::new(AtomicU64::new(0)),
            size: Arc::new(Mutex::new((0, 0))),
        })
    }

    pub fn set_events(&self, tx: mpsc::UnboundedSender<ShareEvent>) {
        *self.events.lock() = Some(tx);
    }

    pub fn sharing(&self) -> bool {
        self.wanted.lock().source.is_some()
    }

    /// Sending the computer's sound (asked for, and possible here).
    pub fn sound(&self) -> bool {
        let w = self.wanted.lock();
        w.source.is_some() && w.sound
    }

    fn change(&self, f: impl FnOnce(&mut Wanted)) {
        {
            let mut w = self.wanted.lock();
            f(&mut w);
            w.generation += 1;
        }
        self.sync_worker();
        if let Some(w) = self.worker.lock().as_ref() {
            w.wake.1.notify_all();
        }
    }

    /// Start sharing `source` (the worker waits for a viewer).
    pub fn start(&self, source: Source, quality: Quality, sound: bool) {
        let sound = sound && capture::sound_supported();
        let previous = self.wanted.lock().source.clone();
        // Held first (Apple's picker on a Mac), so the worker finds it as soon as it looks.
        self.hold(&source, quality, sound);
        self.change(|w| {
            w.source = Some(source.clone());
            w.quality = quality;
            w.sound = sound;
        });
        if let Some(previous) = previous.filter(|p| p.id != source.id) {
            capture::release(&previous);
        }
    }

    pub fn stop(&self) {
        let previous = self.wanted.lock().source.clone();
        self.change(|w| w.source = None);
        if let Some(previous) = previous {
            capture::release(&previous);
        }
    }

    /// Smooth / Sharp / Max while sharing: no restart.
    pub fn set_quality(&self, quality: Quality) {
        self.change(|w| w.quality = quality);
        self.rehold();
    }

    /// Something else to share (another window or screen): same tracks, a new picture.
    pub fn set_source(&self, source: Source) {
        let (previous, quality, sound) = {
            let w = self.wanted.lock();
            (w.source.clone(), w.quality, w.sound)
        };
        self.hold(&source, quality, sound);
        self.change(|w| w.source = Some(source.clone()));
        self.request_keyframe();
        if let Some(previous) = previous.filter(|p| p.id != source.id) {
            capture::release(&previous);
        }
    }

    pub fn set_sound(&self, on: bool) {
        self.change(|w| w.sound = on && capture::sound_supported());
        self.rehold();
    }

    /// A source from the system picker (macOS 14+) keeps one stream for the whole share:
    /// start it (or tell it the quality and sound). If it ends by itself (stopped from the
    /// menu bar, the window closed), the share ends. Other sources: nothing to do here.
    fn hold(&self, source: &Source, quality: Quality, sound: bool) {
        let events = self.events.lock().clone();
        capture::hold(
            source,
            options(quality, sound),
            Box::new(move |why: String| {
                log::info!("screen: the shared screen or window ended: {why}");
                if let Some(tx) = events {
                    let _ = tx.send(ShareEvent::Ended(why));
                }
            }),
        );
    }

    fn rehold(&self) {
        let w = self.wanted.lock().clone();
        if let Some(source) = &w.source {
            self.hold(source, w.quality, w.sound);
        }
    }

    /// A peer connection started (or stopped) sending our share.
    pub fn add_viewer(&self, peer: u64) {
        self.viewers.lock().insert(peer);
        self.request_keyframe();
        self.sync_worker();
    }

    pub fn remove_viewer(&self, peer: u64) {
        self.viewers.lock().remove(&peer);
        self.loss.lock().remove(&peer);
        self.sync_worker();
    }

    pub fn viewers(&self) -> usize {
        self.viewers.lock().len()
    }

    /// A viewer can't decode (it joined, or lost packets): the next picture is a keyframe.
    pub fn request_keyframe(&self) {
        self.keyframe.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.lock().as_ref() {
            w.wake.1.notify_all();
        }
    }

    /// A viewer's latest report about our video (fraction lost, 0–1).
    pub fn report_loss(&self, peer: u64, fraction: f64) {
        self.loss.lock().insert(peer, fraction);
    }

    /// Run the worker exactly while we share and someone watches.
    fn sync_worker(&self) {
        let want = self.sharing() && !self.viewers.lock().is_empty();
        let mut worker = self.worker.lock();
        match (want, worker.is_some()) {
            (true, false) => *worker = Some(self.spawn_worker()),
            (false, true) => {
                if let Some(mut w) = worker.take() {
                    w.stop.store(true, Ordering::Relaxed);
                    w.wake.1.notify_all();
                    if let Some(t) = w.thread.take() {
                        let _ = t.join();
                    }
                }
            }
            _ => {}
        }
    }

    fn spawn_worker(&self) -> Worker {
        let stop = Arc::new(AtomicBool::new(false));
        let wake = Arc::new((Mutex::new(None), Condvar::new()));
        let ctx = WorkerCtx {
            wanted: self.wanted.clone(),
            loss: self.loss.clone(),
            video: self.video.clone(),
            audio: self.audio.clone(),
            keyframe: self.keyframe.clone(),
            events: self.events.lock().clone(),
            stop: stop.clone(),
            wake: wake.clone(),
        };
        let (encoded, size) = (self.encoded.clone(), self.size.clone());
        let thread = std::thread::Builder::new()
            .name("yap-screen".into())
            .spawn(move || run_worker(ctx, encoded, size))
            .map_err(|e| log::error!("screen: no worker thread: {e}"))
            .ok();
        log::info!("screen: someone's watching: capturing and encoding");
        Worker { stop, wake, thread }
    }

    /// Pictures encoded so far, and the last one's size.
    pub fn stats(&self) -> (u64, (u32, u32)) {
        (self.encoded.load(Ordering::Relaxed), *self.size.lock())
    }
}

struct WorkerCtx {
    wanted: Arc<Mutex<Wanted>>,
    loss: Arc<Mutex<HashMap<u64, f64>>>,
    video: broadcast::Sender<Arc<EncodedVideo>>,
    audio: broadcast::Sender<Arc<EncodedAudio>>,
    keyframe: Arc<AtomicBool>,
    events: Option<mpsc::UnboundedSender<ShareEvent>>,
    stop: Arc<AtomicBool>,
    wake: Arc<(Mutex<Option<Frame>>, Condvar)>,
}

fn options(q: Quality, sound: bool) -> Options {
    let (max_width, max_height) = q.max_size();
    Options { max_width, max_height, fps: q.fps(), sound }
}

/// Opus for the share's sound: stereo music, 20 ms packets.
fn sound_encoder() -> Option<ropus::Encoder> {
    ropus::Encoder::builder(crate::audio::RATE, ropus::Channels::Stereo, ropus::Application::Audio)
        .bitrate(ropus::Bitrate::try_bits(SOUND_BITRATE).unwrap_or(ropus::Bitrate::Max))
        .complexity(9)
        .vbr(true)
        .dtx(false)
        .inband_fec(ropus::InbandFec::Enabled)
        .build()
        .map_err(|e| log::error!("share sound: opus encoder: {e:?}"))
        .ok()
}

/// The H.264 encoder: the Mac's hardware one (VideoToolbox) where there is
/// one, else OpenH264 on the CPU. `YAP_SCREEN_ENCODER=openh264` always takes
/// OpenH264; a hardware encoder that keeps failing gives way to it too.
enum Encoder {
    Soft(Box<VideoEncoder>),
    #[cfg(target_os = "macos")]
    Mac { enc: super::vt::HwEncoder, failures: u32 },
}

impl Encoder {
    fn new(bitrate: u32, fps: u32) -> Self {
        #[cfg(target_os = "macos")]
        if std::env::var("YAP_SCREEN_ENCODER").as_deref() != Ok("openh264")
            && let Some(enc) = super::vt::HwEncoder::new(bitrate, fps).filter(|e| e.hardware())
        {
            return Self::Mac { enc, failures: 0 };
        }
        log::info!("H.264: OpenH264");
        Self::Soft(Box::new(VideoEncoder::new(bitrate, fps)))
    }

    fn set_rate(&mut self, bitrate: u32, fps: u32) {
        match self {
            Self::Soft(e) => e.set_rate(bitrate, fps),
            #[cfg(target_os = "macos")]
            Self::Mac { enc, .. } => enc.set_rate(bitrate, fps),
        }
    }

    fn set_bitrate_live(&mut self, bps: u32) {
        match self {
            Self::Soft(e) => e.set_bitrate_live(bps),
            #[cfg(target_os = "macos")]
            Self::Mac { enc, .. } => enc.set_bitrate_live(bps),
        }
    }

    /// `rate`: the bitrate and fps, for OpenH264 if the hardware gives up.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    fn encode(&mut self, frame: &Frame, max: (u32, u32), keyframe: bool, rate: (u32, u32)) -> Result<Encoded, String> {
        match self {
            Self::Soft(e) => e.encode(frame, max, keyframe),
            #[cfg(target_os = "macos")]
            Self::Mac { enc, failures } => {
                let out = enc.encode(frame, max, keyframe);
                *failures = if out.is_ok() { 0 } else { *failures + 1 };
                if *failures >= 3 {
                    log::warn!("H.264: the hardware encoder keeps failing; OpenH264 from now on");
                    *self = Self::Soft(Box::new(VideoEncoder::new(rate.0, rate.1)));
                }
                out
            }
        }
    }
}

/// Start capturing what's wanted: pictures land in `wake`'s slot, sound is
/// encoded on the capture's own thread and goes straight out.
fn start_capture(ctx: &WorkerCtx, source: &Source, q: Quality, sound: bool) -> Result<Box<dyn capture::Capture>, String> {
    let wake = ctx.wake.clone();
    let on_frame: capture::OnFrame = Box::new(move |f: Frame| {
        *wake.0.lock() = Some(f);
        wake.1.notify_all();
    });
    let on_audio: Option<capture::OnAudio> = sound.then(|| {
        let tx = ctx.audio.clone();
        let mut enc = sound_encoder();
        let mut pending: Vec<f32> = Vec::with_capacity(4096);
        let mut pcm = vec![0i16; 1920];
        let mut out = vec![0u8; 4000];
        Box::new(move |samples: &[f32]| {
            pending.extend_from_slice(samples);
            while pending.len() >= 1920 {
                for (o, s) in pcm.iter_mut().zip(pending.drain(..1920)) {
                    *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                }
                if let Some(e) = enc.as_mut()
                    && tx.receiver_count() > 0
                    && let Ok(n) = e.encode(&pcm, &mut out)
                {
                    let _ = tx.send(Arc::new(EncodedAudio { data: Bytes::copy_from_slice(&out[..n]) }));
                }
            }
        }) as capture::OnAudio
    });
    let (events, stop) = (ctx.events.clone(), ctx.stop.clone());
    let wake = ctx.wake.clone();
    let on_ended: capture::OnEnded = Box::new(move |why: String| {
        log::info!("screen: capture ended: {why}");
        if !stop.load(Ordering::Relaxed)
            && let Some(tx) = &events
        {
            let _ = tx.send(ShareEvent::Ended(why));
        }
        wake.1.notify_all();
    });
    capture::start(source, options(q, sound), on_frame, on_audio, on_ended)
}

fn run_worker(ctx: WorkerCtx, encoded: Arc<AtomicU64>, size: Arc<Mutex<(u32, u32)>>) {
    let mut seen = u64::MAX;
    let mut cap: Option<Box<dyn capture::Capture>> = None;
    let mut current: Option<(String, bool)> = None;
    let mut q = Quality::Smooth;
    let mut enc = Encoder::new(q.bitrate(), q.fps());
    let mut last: Option<Frame> = None;
    let mut last_sent = Instant::now() - REPEAT_EVERY;
    // Loss-based backing off (one encoder for everyone, so the worst viewer counts).
    let mut factor = 1.0f64;
    let mut last_adapt = Instant::now();
    while !ctx.stop.load(Ordering::Relaxed) {
        // Follow what's wanted.
        let w = ctx.wanted.lock().clone();
        if w.generation != seen {
            seen = w.generation;
            let Some(source) = w.source.clone() else { break };
            let want = (source.id.clone(), w.sound);
            if current.as_ref() != Some(&want) {
                if let Some(mut c) = cap.take() {
                    c.stop();
                }
                last = None;
                match start_capture(&ctx, &source, w.quality, w.sound) {
                    Ok(c) => {
                        log::info!("screen: capturing {:?} ({}, sound {})", source.name, w.quality.label(), if w.sound { "on" } else { "off" });
                        cap = Some(c);
                        current = Some(want);
                        ctx.keyframe.store(true, Ordering::Relaxed);
                    }
                    Err(e) => {
                        log::warn!("screen: couldn't capture {:?}: {e}", source.name);
                        if let Some(tx) = &ctx.events {
                            let _ = tx.send(ShareEvent::Ended(e));
                        }
                        current = None;
                        // Wait for something else to be wanted (or to be stopped).
                        let mut slot = ctx.wake.0.lock();
                        ctx.wake.1.wait_for(&mut slot, Duration::from_millis(500));
                        continue;
                    }
                }
            }
            if w.quality != q {
                q = w.quality;
                if let Some(c) = cap.as_mut() {
                    c.set_options(options(q, w.sound));
                }
                enc.set_rate((q.bitrate() as f64 * factor) as u32, q.fps());
                ctx.keyframe.store(true, Ordering::Relaxed);
            }
        }
        // Wait for a new picture (or the time to repeat the last one).
        let interval = Duration::from_secs_f64(1.0 / q.fps() as f64);
        let frame = {
            let mut slot = ctx.wake.0.lock();
            if slot.is_none() && !ctx.keyframe.load(Ordering::Relaxed) {
                let wait = REPEAT_EVERY.saturating_sub(last_sent.elapsed()).max(Duration::from_millis(1));
                ctx.wake.1.wait_for(&mut slot, wait);
            }
            slot.take()
        };
        if ctx.stop.load(Ordering::Relaxed) {
            break;
        }
        let fresh = frame.is_some();
        if let Some(f) = frame {
            last = Some(f);
        }
        let keyframe = ctx.keyframe.load(Ordering::Relaxed);
        if !fresh && !keyframe && last_sent.elapsed() < REPEAT_EVERY {
            continue;
        }
        let Some(f) = last.as_ref() else { continue };
        // Back off when viewers lose packets; creep back up when they don't.
        if last_adapt.elapsed() >= Duration::from_secs(1) {
            last_adapt = Instant::now();
            let worst = ctx.loss.lock().values().copied().fold(0.0, f64::max);
            let before = factor;
            if worst > 0.10 {
                factor = (factor * 0.8).max(0.15);
            } else if worst > 0.03 {
                factor = (factor * 0.95).max(0.15);
            } else if worst < 0.01 {
                factor = (factor * 1.08).min(1.0);
            }
            if (factor - before).abs() > 0.001 {
                enc.set_bitrate_live((q.bitrate() as f64 * factor) as u32);
                log::info!("screen: viewers lose {:.1}% → {:.0}% of {} kbps", worst * 100.0, factor * 100.0, q.bitrate() / 1000);
            }
        }
        if ctx.video.receiver_count() == 0 {
            // Nobody's track is ready yet: keep the keyframe for when one is.
            std::thread::sleep(interval.min(Duration::from_millis(20)));
            continue;
        }
        let want_key = ctx.keyframe.swap(false, Ordering::Relaxed);
        let started = Instant::now();
        match enc.encode(f, q.max_size(), want_key, ((q.bitrate() as f64 * factor) as u32, q.fps())) {
            Ok(e) if !e.data.is_empty() => {
                *size.lock() = (e.width, e.height);
                encoded.fetch_add(1, Ordering::Relaxed);
                let _ = ctx.video.send(Arc::new(EncodedVideo { data: Bytes::from(e.data), keyframe: e.keyframe, at: Instant::now() }));
                last_sent = Instant::now();
            }
            Ok(_) => {
                if want_key {
                    ctx.keyframe.store(true, Ordering::Relaxed);
                }
            }
            Err(e) => {
                log::warn!("screen: {e}");
                ctx.keyframe.store(true, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        // Never faster than the quality's frame rate.
        let spent = started.elapsed();
        if spent < interval && !ctx.keyframe.load(Ordering::Relaxed) {
            std::thread::sleep(interval - spent);
        }
    }
    if let Some(mut c) = cap.take() {
        c.stop();
    }
    log::info!("screen: nobody's watching (or we stopped): capture and encoder off");
}
