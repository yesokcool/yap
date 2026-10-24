//! Watching someone's screen: their video track's RTP → whole pictures
//! (reordered; a gap waits for the retransmission a moment, then asks for a
//! keyframe) → H.264 decoded on a thread of its own → the latest picture for
//! the UI. Nothing is decoded unless we're watching that person.

use super::codec::{self, VideoDecoder};
use bytes::Bytes;
use parking_lot::Mutex;
use rtc::media::io::sample_builder::SampleBuilder;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtp::codec::h264::H264Packet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};

/// The newest decoded picture (RGBA).
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Watch {
    /// Whose screen we watch (their session id).
    sid: Mutex<Option<String>>,
    picture: Mutex<Option<Picture>>,
    /// When the last picture arrived (None: none yet since we started watching).
    last_at: Mutex<Option<Instant>>,
    /// Pictures decoded since we started watching, and their size.
    frames: AtomicU64,
    size: Mutex<(u32, u32)>,
    /// The biggest picture the UI can show (its GPU's biggest texture side):
    /// a bigger screen (H.264 goes up to 8688 px, some GPUs to 8192) is
    /// shrunk here, on the decoding thread.
    max_side: AtomicU32,
    repaint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Watch {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { sid: Mutex::new(None), picture: Mutex::new(None), last_at: Mutex::new(None), frames: AtomicU64::new(0), size: Mutex::new((0, 0)), max_side: AtomicU32::new(2048), repaint: Mutex::new(None) })
    }

    /// The UI's biggest texture side (egui's `max_texture_side`).
    pub fn set_max_side(&self, px: usize) {
        self.max_side.store(px.clamp(1, u32::MAX as usize) as u32, Ordering::Relaxed);
    }

    pub fn set_repaint(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *self.repaint.lock() = Some(f);
    }

    pub fn is_watching(&self, sid: &str) -> bool {
        self.sid.lock().as_deref() == Some(sid)
    }

    /// Start watching `sid` (None: stop). Forgets the last picture; the
    /// counters start over with the next screen (they stay readable after).
    pub fn set(&self, sid: Option<String>) {
        if sid.is_some() {
            self.frames.store(0, Ordering::Relaxed);
            *self.size.lock() = (0, 0);
        }
        *self.sid.lock() = sid;
        *self.picture.lock() = None;
        *self.last_at.lock() = None;
    }

    /// They resumed their call under a new session id.
    pub fn rename(&self, from: &str, to: &str) {
        let mut s = self.sid.lock();
        if s.as_deref() == Some(from) {
            *s = Some(to.to_string());
        }
    }

    /// The newest picture, if there's one the UI hasn't taken yet.
    pub fn take_picture(&self) -> Option<Picture> {
        self.picture.lock().take()
    }

    pub fn last_at(&self) -> Option<Instant> {
        *self.last_at.lock()
    }

    pub fn stats(&self) -> (u64, (u32, u32)) {
        (self.frames.load(Ordering::Relaxed), *self.size.lock())
    }

    fn show(&self, sid: &str, p: Picture) {
        if !self.is_watching(sid) || p.width == 0 || p.height == 0 {
            return;
        }
        let first = self.frames.fetch_add(1, Ordering::Relaxed) == 0;
        if first {
            log::info!("screen: first picture from {sid}: {} × {}", p.width, p.height);
        }
        *self.size.lock() = (p.width, p.height);
        *self.last_at.lock() = Some(Instant::now());
        *self.picture.lock() = Some(p);
        if let Some(r) = self.repaint.lock().as_ref() {
            r();
        }
    }
}

/// Does this RTP payload hold together? A STAP-A (NAL type 24) is a list of
/// 2-byte sizes, each followed by that many bytes, to the very end; anything
/// else is left to the depacketizer.
fn stap_a_fits(payload: &[u8]) -> bool {
    if payload.first().is_none_or(|b| b & 0x1f != 24) {
        return true;
    }
    let mut at = 1;
    while at < payload.len() {
        let Some(size) = payload.get(at..at + 2).map(|s| u16::from_be_bytes([s[0], s[1]]) as usize) else { return false };
        at += 2 + size;
    }
    at == payload.len()
}

/// `p`, shrunk (same shape, nearest pixel: it's every frame) so neither side
/// is over `max`.
fn fit(p: Picture, max: u32) -> Picture {
    if p.width <= max && p.height <= max {
        return p;
    }
    let Some(img) = image::RgbaImage::from_raw(p.width, p.height, p.rgba) else { return Picture { width: 0, height: 0, rgba: Vec::new() } };
    let scale = max as f64 / p.width.max(p.height) as f64;
    let side = |v: u32| ((v as f64 * scale) as u32).clamp(1, max.max(1));
    let small = image::imageops::resize(&img, side(p.width), side(p.height), image::imageops::FilterType::Nearest);
    Picture { width: small.width(), height: small.height(), rgba: small.into_raw() }
}

/// Feed one remote video track into `watch` while we watch its sender
/// (`sid_of`: who that is now; it changes if they resume under a new session).
pub async fn receive(track: Arc<dyn TrackRemote>, media_ssrc: u32, sid_of: impl Fn() -> String + Send + 'static, watch: Arc<Watch>) {
    let (tx, rx) = std::sync::mpsc::sync_channel::<(String, Bytes)>(8);
    let need_key = Arc::new(AtomicBool::new(true));
    // The decoder lives on its own thread (it's CPU work, and not async).
    {
        let (watch, need_key) = (watch.clone(), need_key.clone());
        let spawned = std::thread::Builder::new().name("yap-screen-decode".into()).spawn(move || {
            let mut dec = match VideoDecoder::new() {
                Ok(d) => d,
                Err(e) => return log::error!("screen: {e}"),
            };
            while let Ok((sid, au)) = rx.recv() {
                match dec.decode(&au) {
                    Ok(Some(p)) => watch.show(&sid, fit(Picture { width: p.width, height: p.height, rgba: p.rgba }, watch.max_side.load(Ordering::Relaxed))),
                    Ok(None) => {}
                    Err(e) => {
                        log::debug!("screen: decode: {e}");
                        need_key.store(true, Ordering::Relaxed);
                    }
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("screen: no decoder thread: {e}");
            return;
        }
    }
    // Packets that arrive out of order (retransmissions) are put back in
    // order; a frame with a hole is given up after 150 ms.
    let mut builder = SampleBuilder::new(512, H264Packet::default(), 90_000).with_max_time_delay(Duration::from_millis(150));
    let mut waiting_key = true;
    let mut last_pli: Option<Instant> = None;
    loop {
        let ev = tokio::time::timeout(Duration::from_millis(500), track.poll()).await;
        let sid = sid_of();
        let watching = watch.is_watching(&sid);
        // Ask for a keyframe when we need one (joined mid-stream, a gap, a decode error), at most twice a second.
        if watching && (waiting_key || need_key.load(Ordering::Relaxed)) && last_pli.is_none_or(|t| t.elapsed() > Duration::from_millis(500)) {
            last_pli = Some(Instant::now());
            need_key.store(false, Ordering::Relaxed);
            waiting_key = true;
            let _ = track.write_rtcp(vec![Box::new(PictureLossIndication { sender_ssrc: 0, media_ssrc })]).await;
        }
        let ev = match ev {
            Ok(Some(ev)) => ev,
            Ok(None) => break,
            Err(_) => continue,
        };
        let packet = match ev {
            TrackRemoteEvent::OnRtpPacket(p) => p,
            TrackRemoteEvent::OnEnded => break,
            _ => continue,
        };
        if !watching {
            waiting_key = true;
            continue;
        }
        // rtc-rtp's depacketizer indexes past the end of a STAP-A whose sizes don't add up (a panic).
        if !stap_a_fits(&packet.payload) {
            continue;
        }
        let now = Instant::now();
        builder.push(now, packet);
        while let Some(sample) = builder.pop(now) {
            if sample.prev_dropped_packets > 0 {
                waiting_key = true;
            }
            if waiting_key {
                if !codec::has_idr(&sample.data) {
                    continue;
                }
                waiting_key = false;
            }
            if tx.try_send((sid.clone(), sample.data)).is_err() {
                // The decoder is behind: drop this picture, and resynchronise on a keyframe.
                waiting_key = true;
            }
        }
    }
    log::info!("screen: video track ended");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A STAP-A whose sizes run past its end (or leave a byte over) never
    /// reaches the depacketizer, which would index past it; the rest pass.
    #[test]
    fn broken_stap_a_is_dropped() {
        assert!(!stap_a_fits(&[0x18, 0x00, 0x00, 0x00]), "one byte left over: the depacketizer read past the end");
        assert!(!stap_a_fits(&[0x18, 0x00, 0x05, 0x65, 0x01]));
        assert!(!stap_a_fits(&[0x18, 0x00]));
        assert!(stap_a_fits(&[0x18, 0x00, 0x02, 0x67, 0x42, 0x00, 0x01, 0x68]));
        assert!(stap_a_fits(&[0x18]));
        assert!(stap_a_fits(&[0x65, 0x88, 0x84]), "a single NAL unit");
        assert!(stap_a_fits(&[0x7c, 0x85, 0x01]), "an FU-A");
        assert!(stap_a_fits(&[]));
    }

    /// A screen bigger than the GPU takes (H.264 goes up to 8688 px) is shrunk
    /// to fit, same shape; one that fits is left alone.
    #[test]
    fn pictures_fit_the_gpu() {
        let p = |w: u32, h: u32| Picture { width: w, height: h, rgba: vec![7; (w * h * 4) as usize] };
        let small = fit(p(8688, 100), 8192);
        assert_eq!((small.width, small.height, small.rgba.len()), (8192, 94, 8192 * 94 * 4));
        let same = fit(p(1920, 1080), 8192);
        assert_eq!((same.width, same.height), (1920, 1080));
        // Bytes that don't add up to the size: nothing to show, nothing shown.
        let broken = fit(Picture { width: 9000, height: 2, rgba: vec![0; 8] }, 8192);
        assert_eq!((broken.width, broken.height), (0, 0));
        let w = Watch::new();
        w.set(Some("s1".into()));
        w.show("s1", broken);
        assert!(w.take_picture().is_none());
    }
}
