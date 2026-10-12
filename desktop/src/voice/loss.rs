//! How much of what we send the far side loses, from its RTCP reports.
//!
//! webrtc-rs fills `remote-inbound-rtp` only from *receiver* reports, but in a
//! call both sides send, so a browser describes our stream in the report
//! blocks of its *sender* reports and the stat never moves. This interceptor
//! reads the blocks from both kinds on the way in (and passes every packet on
//! untouched), for the encoder's packet-loss hint (V8). It also notes, for
//! our screen share's video, what they lose and when they ask for a
//! keyframe (PLI / FIR).

use rtc::interceptor::{Interceptor, Packet, StreamInfo, TaggedPacket};
use rtc::rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtcp::receiver_report::ReceiverReport;
use rtc::rtcp::reception_report::ReceptionReport;
use rtc::rtcp::sender_report::SenderReport;
use rtc::sansio::Protocol;
use rtc::shared::error::Error;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Instant;

/// The latest report about our stream: fraction lost (0–255, of 256), the
/// cumulative count they've lost, and how many reports so far.
#[derive(Default)]
pub struct LossReport {
    fraction: AtomicU32,
    total_lost: AtomicU32,
    reports: AtomicU32,
    /// Our sending SSRC (reports about anything else are ignored); 0 = take any.
    pub ssrc: AtomicU32,
    /// Our screen share's video SSRC (0 = not sending one), what they lose
    /// of it (0–255, of 256) and whether they asked for a keyframe.
    pub video_ssrc: AtomicU32,
    video_fraction: AtomicU32,
    video_reports: AtomicU32,
    pub keyframe: AtomicBool,
}

impl LossReport {
    /// (fraction lost 0–1, reports seen so far).
    pub fn latest(&self) -> (f64, u32) {
        (self.fraction.load(Ordering::Relaxed) as f64 / 256.0, self.reports.load(Ordering::Relaxed))
    }

    /// How many of our packets they've lost in all (RTCP's cumulative count), as of the latest report.
    pub fn total_lost(&self) -> u32 {
        self.total_lost.load(Ordering::Relaxed)
    }

    /// The latest report about our screen video: (fraction lost 0–1, reports so far).
    pub fn video(&self) -> (f64, u32) {
        (self.video_fraction.load(Ordering::Relaxed) as f64 / 256.0, self.video_reports.load(Ordering::Relaxed))
    }

    fn take(&self, blocks: &[ReceptionReport]) {
        let ours = self.ssrc.load(Ordering::Relaxed);
        let video = self.video_ssrc.load(Ordering::Relaxed);
        for b in blocks {
            if video != 0 && b.ssrc == video {
                self.video_fraction.store(b.fraction_lost as u32, Ordering::Relaxed);
                self.video_reports.fetch_add(1, Ordering::Relaxed);
            } else if ours == 0 || b.ssrc == ours {
                self.fraction.store(b.fraction_lost as u32, Ordering::Relaxed);
                // (24 bits, signed: duplicates can make it "negative"; that's none lost.)
                let total = if b.total_lost & 0x80_0000 != 0 { 0 } else { b.total_lost & 0xff_ffff };
                self.total_lost.store(total, Ordering::Relaxed);
                self.reports.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// A keyframe request about our screen video.
    fn wants_keyframe(&self, media_ssrc: u32) {
        let video = self.video_ssrc.load(Ordering::Relaxed);
        if video != 0 && (media_ssrc == video || media_ssrc == 0) {
            self.keyframe.store(true, Ordering::Relaxed);
        }
    }
}

/// A pass-through interceptor that notes report blocks into a [`LossReport`].
pub struct LossWatch {
    report: Arc<LossReport>,
    read: VecDeque<TaggedPacket>,
    write: VecDeque<TaggedPacket>,
}

impl LossWatch {
    pub fn new(report: Arc<LossReport>) -> Self {
        Self { report, read: VecDeque::new(), write: VecDeque::new() }
    }
}

impl Protocol<TaggedPacket, TaggedPacket, ()> for LossWatch {
    type Rout = TaggedPacket;
    type Wout = TaggedPacket;
    type Eout = ();
    type Error = Error;
    type Time = Instant;

    fn handle_read(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        if let Packet::Rtcp(packets) = &msg.message.packet {
            for p in packets {
                if let Some(sr) = p.as_any().downcast_ref::<SenderReport>() {
                    self.report.take(&sr.reports);
                } else if let Some(rr) = p.as_any().downcast_ref::<ReceiverReport>() {
                    self.report.take(&rr.reports);
                } else if let Some(pli) = p.as_any().downcast_ref::<PictureLossIndication>() {
                    self.report.wants_keyframe(pli.media_ssrc);
                } else if let Some(fir) = p.as_any().downcast_ref::<FullIntraRequest>() {
                    for e in &fir.fir {
                        self.report.wants_keyframe(e.ssrc);
                    }
                }
            }
        }
        self.read.push_back(msg);
        Ok(())
    }

    fn poll_read(&mut self) -> Option<Self::Rout> {
        self.read.pop_front()
    }

    fn handle_write(&mut self, msg: TaggedPacket) -> Result<(), Self::Error> {
        self.write.push_back(msg);
        Ok(())
    }

    fn poll_write(&mut self) -> Option<Self::Wout> {
        self.write.pop_front()
    }

    fn handle_timeout(&mut self, _now: Instant) -> Result<(), Self::Error> {
        Ok(())
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        None
    }
}

impl Interceptor for LossWatch {
    fn bind_local_stream(&mut self, _info: &StreamInfo) {}
    fn unbind_local_stream(&mut self, _info: &StreamInfo) {}
    fn bind_remote_stream(&mut self, _info: &StreamInfo) {}
    fn unbind_remote_stream(&mut self, _info: &StreamInfo) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_blocks_from_sender_and_receiver_reports() {
        let r = Arc::new(LossReport::default());
        r.ssrc.store(7, Ordering::Relaxed);
        let block = |ssrc, fraction_lost, total_lost| ReceptionReport { ssrc, fraction_lost, total_lost, ..Default::default() };
        r.take(&SenderReport { reports: vec![block(9, 200, 900), block(7, 13, 40)], ..Default::default() }.reports);
        assert_eq!(r.latest(), (13.0 / 256.0, 1), "only the block about our stream");
        assert_eq!(r.total_lost(), 40);
        r.take(&ReceiverReport { reports: vec![block(7, 0, 41)], ..Default::default() }.reports);
        assert_eq!((r.latest(), r.total_lost()), ((0.0, 2), 41));
        r.take(&ReceiverReport { reports: vec![block(7, 0, 0xff_fffe)], ..Default::default() }.reports);
        assert_eq!(r.total_lost(), 0, "a \"negative\" count (duplicates) is none lost");
    }
}
