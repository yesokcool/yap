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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    repaint: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Watch {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { sid: Mutex::new(None), picture: Mutex::new(None), last_at: Mutex::new(None), frames: AtomicU64::new(0), size: Mutex::new((0, 0)), repaint: Mutex::new(None) })
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
        if !self.is_watching(sid) {
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
                    Ok(Some(p)) => watch.show(&sid, Picture { width: p.width, height: p.height, rgba: p.rgba }),
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
