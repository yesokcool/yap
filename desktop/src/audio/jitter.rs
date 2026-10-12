//! Per-peer receive path: RTP packets in, 48 kHz stereo PCM out.
//!
//! An adaptive jitter buffer keyed by (extended) RTP *timestamp*, so the
//! padding-only packets browsers send as bandwidth probes (empty payload, a
//! sequence number of their own) can neither look like loss nor hide the FEC
//! that's in the next real packet.
//!
//! - **Target** (how much audio to hold): the lateness that all but ~1 % of
//!   the last ~10 s of packets beat, plus 10 ms, clamped to 40–200 ms.
//!   Lateness = how much later than the fastest packet of the last ~10 s a
//!   packet arrived, for its timestamp. Late packets (too late to play) are
//!   measured like any other, so they are what makes the target grow. It goes
//!   up at once and comes down slowly (2 ms per second). While packets are
//!   being lost it holds at least 60 ms, so the next frame (with the lost
//!   one's FEC) is already there when the gap comes up.
//! - **Holding it**: we track the level an on-time packet finds (audio queued
//!   ahead of the playout point). When that strays more than 15 ms from the
//!   target (clock drift, a new target) one frame is dropped or one
//!   concealment frame inserted, during quiet audio only (forced after 2 s
//!   without any, e.g. music).
//! - **Gaps**: FEC from the packet of the following frame, else PLC. A frame
//!   that's missing while the buffer is well under its target is probably just
//!   late: conceal without moving on (the buffer grows, and it may still make it).
//! - **RED** (RFC 2198, PROTOCOL.md › Voice › Redundancy): a packet may also
//!   carry the frame before it. That copy only goes in for a frame whose own
//!   packet hasn't come and that isn't played or patched over yet (its turn
//!   hasn't passed), never twice; it then plays like the real thing.
//! - **Underruns**: conceal up to 100 ms, then go quiet and re-buffer.
//!
//! Time comes in from outside (`arrival_ms` per packet; the playout clock is
//! the audio pulled), so the tests below run it on a simulated network.

use std::collections::{BTreeMap, VecDeque};

pub const RATE: usize = 48_000;
const MS: f64 = RATE as f64 / 1000.0;
const MAX_FRAME: usize = 5760; // 120 ms at 48 kHz
const DEFAULT_FRAME: usize = 960;
pub const MIN_TARGET_MS: f64 = 40.0;
pub const MAX_TARGET_MS: f64 = 200.0;
/// While packets are being lost (3+ gaps in the last 10 s), hold at least this
/// much: the frame after a gap has to be here already for its FEC to repair it.
const LOSSY_MIN_TARGET_MS: f64 = 60.0;
const LOSSY_GAPS: usize = 3;
const LOSSY_WINDOW_MS: f64 = 10_000.0;
/// The target covers this share of recent packets' lateness…
const QUANTILE: f64 = 0.99;
/// …plus this much (the playout point moves in 10 ms steps, packets are 20 ms).
const MARGIN_MS: f64 = 10.0;
/// Lateness history: the last ~10 s of packets.
const WINDOW: usize = 500;
const BIN_MS: f64 = 2.0;
const BINS: usize = 200;
/// The fastest packet is looked for in this many 1 s buckets.
const BASE_SECS: i64 = 10;
/// How fast the target comes down (ms per second of audio played).
const LOWER_PER_SEC: f64 = 2.0;
/// Level further than this above the target → one frame out; further below → one in.
const ADJUST_MS: f64 = 15.0;
const ADJUST_UNDER_MS: f64 = 8.0;
/// …during quiet audio, or after this long without any (ms).
const FORCE_AFTER_MS: f64 = 2000.0;
/// A frame at most this loud (RMS, linear; ≈ −45 dBFS) is "quiet".
const QUIET_RMS: f32 = 0.0056;
/// Way too much queued (a burst after a stall): skip ahead in one go.
const JUMP_OVER_MS: f64 = 300.0;
const MAX_PLC_RUN: u32 = 5;
/// Frames to wait (concealing) for a missing one while the buffer is short.
const MAX_WAIT: u32 = 3;

#[derive(Default, Clone, Copy, Debug)]
pub struct JitterStats {
    pub packets: u64,
    pub bytes: u64,
    /// Arrived after their audio was already played (or concealed).
    pub late: u64,
    /// Frames patched over with PLC (loss, late packets, underruns, waiting).
    pub concealed: u64,
    /// Frames recovered from the next packet's in-band FEC.
    pub fec: u64,
    /// Frames rebuilt, word for word, from the copy a RED packet carried.
    pub red: u64,
    /// Packets that came RED-packed (the codec they used: red+opus).
    pub red_packets: u64,
    /// Packets the sequence numbers say were sent (lost = this − received).
    pub expected: u64,
    pub underruns: u64,
    /// Padding-only packets (bandwidth probes): sequence numbers without audio.
    pub padding: u64,
    /// Frames inserted / dropped in quiet audio to follow the target (clock drift, a new target).
    pub inserted: u64,
    pub dropped: u64,
    /// Every frame played: decoded, recovered, concealed or inserted.
    pub frames: u64,
    pub target_ms: f32,
    /// Audio queued ahead of the playout point now.
    pub delay_ms: f32,
    /// The longest single drop (ms).
    pub max_drop_ms: f32,
    /// Playing (not waiting to fill up).
    pub playing: bool,
}

struct Packet {
    data: Vec<u8>,
    /// Samples per channel in this packet (from its TOC).
    samples: usize,
    /// A RED copy standing in for a packet that hasn't come (yet).
    red: bool,
}

/// Samples per channel (48 kHz) in an Opus packet, from its TOC byte.
pub fn opus_samples(p: &[u8]) -> Option<usize> {
    let toc = *p.first()?;
    let cfg = (toc >> 3) as usize;
    let frame = match cfg {
        0..=11 => [480, 960, 1920, 2880][cfg & 3],
        12..=15 => [480, 960][cfg & 1],
        _ => [120, 240, 480, 960][cfg & 3],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => (*p.get(1)? & 0x3f) as usize,
    };
    (frames > 0).then_some(frame * frames)
}

/// Arrival lateness: transit time (arrival − timestamp) over the fastest
/// packet of the last ~10 s, and a histogram of the last `WINDOW` of them.
struct Lateness {
    /// (second, fastest transit in it).
    buckets: VecDeque<(i64, f64)>,
    hist: [u32; BINS],
    ring: VecDeque<u16>,
}

impl Lateness {
    fn new() -> Self {
        Self { buckets: VecDeque::new(), hist: [0; BINS], ring: VecDeque::with_capacity(WINDOW + 1) }
    }

    fn observe(&mut self, transit: f64, now_ms: f64) -> f64 {
        let sec = (now_ms / 1000.0).floor() as i64;
        match self.buckets.back_mut() {
            Some((s, m)) if *s == sec => *m = m.min(transit),
            _ => self.buckets.push_back((sec, transit)),
        }
        while self.buckets.front().is_some_and(|(s, _)| *s <= sec - BASE_SECS) {
            self.buckets.pop_front();
        }
        let base = self.buckets.iter().map(|b| b.1).fold(f64::INFINITY, f64::min);
        let late = (transit - base).max(0.0);
        let bin = ((late / BIN_MS) as usize).min(BINS - 1);
        self.hist[bin] += 1;
        self.ring.push_back(bin as u16);
        if self.ring.len() > WINDOW
            && let Some(b) = self.ring.pop_front()
        {
            self.hist[b as usize] -= 1;
        }
        late
    }

    fn quantile(&self, q: f64) -> f64 {
        let n = self.ring.len();
        if n == 0 {
            return 0.0;
        }
        let want = ((q * n as f64).ceil() as u32).max(1);
        let mut acc = 0;
        for (i, c) in self.hist.iter().enumerate() {
            acc += c;
            if acc >= want {
                return (i + 1) as f64 * BIN_MS;
            }
        }
        BINS as f64 * BIN_MS
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

pub struct JitterBuffer {
    packets: BTreeMap<u64, Packet>,
    /// Highest (extended) timestamp received.
    highest: Option<u64>,
    /// Timestamp of the next frame to play.
    next: Option<u64>,
    playing: bool,
    plc_run: u32,
    late_run: u32,
    /// Frames concealed while waiting for the one at `next`.
    waited: u32,
    /// Frames already concealed in place (an empty queue, or waiting): if the
    /// ones they stood in for turn out to be lost, they're skipped, not
    /// concealed twice; if they arrive after all, that's latency the
    /// buffer gives back later.
    owed: u32,
    decoder: ropus::Decoder,
    /// Samples per channel of the last decoded frame (PLC/FEC size).
    frame: usize,
    /// Interleaved stereo f32 waiting to be played.
    pcm: VecDeque<f32>,
    scratch: Vec<i16>,
    pub stats: JitterStats,
    lateness: Lateness,
    /// What the latest packets ask for, and the target in force (ms).
    wanted: f64,
    target: f64,
    /// Smoothed level an on-time packet finds (ms), while playing.
    level: Option<f64>,
    /// Audio played so far (ms): the clock for slow changes.
    played_ms: f64,
    /// The last frame decoded was quiet.
    quiet: bool,
    /// Since when the level has been off target (ms of `played_ms`).
    off_since: Option<f64>,
    /// When recent frames went missing (ms of `played_ms`).
    gaps: VecDeque<f64>,
    last_adjust: f64,
    /// The highest sequence number seen (for `expected`).
    last_seq: Option<u16>,
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self {
            packets: BTreeMap::new(),
            highest: None,
            next: None,
            playing: false,
            plc_run: 0,
            late_run: 0,
            waited: 0,
            owed: 0,
            decoder: ropus::Decoder::new(RATE as u32, ropus::Channels::Stereo).expect("opus decoder"),
            frame: DEFAULT_FRAME,
            pcm: VecDeque::with_capacity(MAX_FRAME * 4),
            scratch: vec![0; MAX_FRAME * 2],
            stats: JitterStats { target_ms: MIN_TARGET_MS as f32, ..Default::default() },
            lateness: Lateness::new(),
            wanted: MIN_TARGET_MS,
            target: MIN_TARGET_MS,
            level: None,
            played_ms: 0.0,
            quiet: true,
            off_since: None,
            last_adjust: 0.0,
            gaps: VecDeque::new(),
            last_seq: None,
        }
    }

    /// Count a sequence number towards `expected` (a jump of 1000+ is the sender starting over).
    fn count_seq(&mut self, seq: u16) {
        let step = self.last_seq.map_or(1, |l| seq.wrapping_sub(l));
        if step == 0 || step >= 0x8000 {
            return; // a duplicate, or an older one arriving late (already expected)
        }
        self.stats.expected += if step < 1000 { step as u64 } else { 1 };
        self.last_seq = Some(seq);
    }

    fn extend(&self, ts: u32) -> u64 {
        let Some(h) = self.highest else { return (1u64 << 32) + ts as u64 };
        let base = h & !0xFFFF_FFFF;
        let candidates = [base + ts as u64, base + ts as u64 + (1 << 32), (base + ts as u64).wrapping_sub(1 << 32)];
        *candidates.iter().min_by_key(|c| c.abs_diff(h)).unwrap()
    }

    fn pcm_ms(&self) -> f64 {
        (self.pcm.len() / 2) as f64 / MS
    }

    /// Audio queued ahead of the playout point (ms).
    fn ahead_ms(&self) -> f64 {
        let queued = match (self.next, self.packets.last_key_value()) {
            (Some(n), Some((ts, p))) => (*ts + p.samples as u64).saturating_sub(n) as f64 / MS,
            _ => 0.0,
        };
        self.pcm_ms() + queued
    }

    fn restart(&mut self) {
        self.packets.clear();
        self.next = None;
        self.playing = false;
        self.highest = None;
        self.late_run = 0;
        self.level = None;
        self.lateness.reset();
    }

    /// One RTP packet, arrived at `arrival_ms` (any fixed clock). An empty
    /// `payload` is padding: a sequence number with no audio, ignored.
    pub fn push(&mut self, seq: u16, ts: u32, payload: &[u8], arrival_ms: f64) {
        self.count_seq(seq);
        self.push_audio(ts, payload, arrival_ms);
    }

    /// A RED packet (RFC 2198): its own frame goes in like any packet's; each
    /// earlier frame it carries only takes the place of one whose own packet
    /// never came and whose turn hasn't passed. A malformed one is dropped.
    pub fn push_red(&mut self, seq: u16, ts: u32, payload: &[u8], arrival_ms: f64) {
        self.count_seq(seq);
        let Some((copies, primary)) = crate::voice::red::unpack(payload) else { return };
        self.stats.red_packets += 1;
        self.push_audio(ts, primary.data, arrival_ms);
        self.stats.bytes += (payload.len() - primary.data.len()) as u64;
        for c in copies.iter().filter(|c| c.pt == primary.pt && c.offset > 0) {
            self.push_copy(ts.wrapping_sub(c.offset), c.data);
        }
    }

    /// An earlier frame from a RED packet: queued only if its own packet isn't
    /// here and it isn't played or patched over yet (and never twice).
    fn push_copy(&mut self, ts: u32, payload: &[u8]) {
        let (Some(h), Some(samples)) = (self.highest, opus_samples(payload)) else { return };
        let ext = self.extend(ts);
        if ext.abs_diff(h) > 3 * RATE as u64 || self.next.is_some_and(|n| ext < n) || self.packets.contains_key(&ext) {
            return;
        }
        self.packets.insert(ext, Packet { data: payload.to_vec(), samples, red: true });
    }

    fn push_audio(&mut self, ts: u32, payload: &[u8], arrival_ms: f64) {
        if payload.is_empty() {
            self.stats.padding += 1;
            return;
        }
        let Some(samples) = opus_samples(payload) else { return };
        let mut ext = self.extend(ts);
        // A jump of more than 3 s either way (the sender restarted), packets that
        // keep arriving too late to ever catch up, or a pile-up: start over.
        if self.highest.is_some_and(|h| ext.abs_diff(h) > 3 * RATE as u64) || self.late_run >= 25 || self.packets.len() > 300 {
            self.restart();
            ext = self.extend(ts);
        }
        self.stats.packets += 1;
        self.stats.bytes += payload.len() as u64;
        let late = self.lateness.observe(arrival_ms - ext as f64 / MS, arrival_ms);
        while self.gaps.front().is_some_and(|t| self.played_ms - t > LOSSY_WINDOW_MS) {
            self.gaps.pop_front();
        }
        let floor = if self.gaps.len() >= LOSSY_GAPS { LOSSY_MIN_TARGET_MS } else { MIN_TARGET_MS };
        self.wanted = (self.lateness.quantile(QUANTILE) + MARGIN_MS).clamp(floor, MAX_TARGET_MS);
        if self.wanted > self.target {
            self.target = self.wanted;
        }
        if self.next.is_some_and(|n| ext < n) {
            self.stats.late += 1;
            self.late_run += 1;
            return;
        }
        self.late_run = 0;
        if let Some(p) = self.packets.get_mut(&ext) {
            p.red = false; // its own packet made it after all (a copy was waiting)
            return;
        }
        self.highest = Some(self.highest.map_or(ext, |h| h.max(ext)));
        self.packets.insert(ext, Packet { data: payload.to_vec(), samples, red: false });
        // The level an on-time packet would find: this one's slack plus its lateness.
        if self.playing
            && let Some(n) = self.next
        {
            let found = self.pcm_ms() + (ext + samples as u64).saturating_sub(n) as f64 / MS + late;
            self.level = Some(match self.level {
                Some(l) => l + (found - l) * 0.04,
                None => found,
            });
        }
    }

    /// Fill `out` (interleaved stereo) completely; silence where there's nothing.
    /// Tests: decoded audio (stereo, interleaved) to play next.
    #[cfg(test)]
    pub fn inject_pcm(&mut self, pcm: &[f32]) {
        self.pcm.extend(pcm);
    }

    pub fn pull(&mut self, out: &mut [f32]) {
        while self.pcm.len() < out.len() {
            self.decode_one();
        }
        for s in out.iter_mut() {
            *s = self.pcm.pop_front().unwrap_or(0.0);
        }
        let ms = (out.len() / 2) as f64 / MS;
        self.played_ms += ms;
        // Lower the target slowly towards what the packets ask for.
        if self.target > self.wanted {
            self.target = (self.target - LOWER_PER_SEC * ms / 1000.0).max(self.wanted);
        }
        self.stats.target_ms = self.target as f32;
        self.stats.delay_ms = if self.playing { self.ahead_ms() as f32 } else { 0.0 };
        self.stats.playing = self.playing;
    }

    fn push_silence(&mut self, frames: usize) {
        self.pcm.extend(std::iter::repeat_n(0.0, frames * 2));
    }

    /// Decode into `scratch`; returns samples per channel.
    fn decode(&mut self, packet: &[u8], fec: bool, frames: usize) -> Option<usize> {
        let len = if packet.is_empty() || fec { frames * 2 } else { MAX_FRAME * 2 };
        let mode = if fec { ropus::DecodeMode::Fec } else { ropus::DecodeMode::Normal };
        match self.decoder.decode(packet, &mut self.scratch[..len], mode) {
            Ok(n) if n > 0 => Some(n),
            _ => None,
        }
    }

    /// Queue `n` decoded samples per channel from `scratch` (skipping the first `skip`).
    fn queue(&mut self, n: usize, skip: usize) {
        let mut sq = 0f32;
        for &s in &self.scratch[skip * 2..n * 2] {
            let v = s as f32 / 32768.0;
            sq += v * v;
            self.pcm.push_back(v);
        }
        let len = (n.saturating_sub(skip) * 2).max(1);
        self.quiet = (sq / len as f32).sqrt() <= QUIET_RMS;
        self.stats.frames += 1;
    }

    /// The buffer just grew (+) or shrank (−) by `ms` without a packet saying
    /// so: keep the level estimate in step (it's otherwise only measured as
    /// packets arrive, and smoothed).
    fn shift_level(&mut self, ms: f64) {
        if let Some(l) = self.level.as_mut() {
            *l += ms;
        }
    }

    /// Conceal one frame (PLC) at the current position, without consuming anything.
    fn conceal(&mut self) {
        let frame = self.frame;
        match self.decode(&[], false, frame) {
            Some(n) => self.queue(n, 0),
            None => {
                self.push_silence(frame);
                self.stats.frames += 1;
            }
        }
    }

    fn decode_one(&mut self) {
        if !self.playing {
            let buffered = match (self.packets.first_key_value(), self.packets.last_key_value()) {
                (Some((first, _)), Some((last, p))) => (*last + p.samples as u64 - *first) as f64 / MS,
                _ => 0.0,
            };
            if buffered >= self.target {
                self.playing = true;
                self.next = self.packets.keys().next().copied();
                self.plc_run = 0;
                self.waited = 0;
                self.owed = 0;
                self.level = None;
                self.off_since = None;
            } else {
                self.push_silence(RATE / 100);
                return;
            }
        }
        let Some(n) = self.next else {
            self.playing = false;
            self.push_silence(RATE / 100);
            return;
        };
        // A burst after a stall: skip ahead to the target in one go.
        let ahead = self.ahead_ms();
        if ahead > self.target + JUMP_OVER_MS {
            while self.ahead_ms() > self.target + ADJUST_MS && self.packets.len() > 1 {
                if let Some((ts, p)) = self.packets.pop_first() {
                    self.next = Some(ts + p.samples as u64);
                }
            }
            self.next = self.packets.keys().next().copied().or(self.next);
            self.level = None;
            return;
        }
        if self.adjust(n) {
            return;
        }
        if let Some(packet) = self.packets.remove(&n) {
            self.next = Some(n + packet.samples as u64);
            self.plc_run = 0;
            self.waited = 0;
            self.owed = 0;
            if packet.red {
                // Its own packet was lost: rebuilt from the next one's copy (and still a gap).
                self.stats.red += 1;
                self.gaps.push_back(self.played_ms);
            }
            match self.decode(&packet.data, false, self.frame) {
                Some(got) => {
                    self.frame = got;
                    self.queue(got, 0);
                }
                None => {
                    self.push_silence(packet.samples);
                    self.stats.frames += 1;
                }
            }
            return;
        }
        let frame = self.frame;
        if self.packets.is_empty() {
            // Nothing queued at all: conceal briefly (they may only be late), then go quiet and re-buffer.
            if self.plc_run < MAX_PLC_RUN {
                self.plc_run += 1;
                self.owed += 1;
                self.stats.concealed += 1;
                self.conceal();
                self.shift_level(frame as f64 / MS);
            } else {
                self.playing = false;
                self.next = None;
                self.owed = 0;
                self.stats.underruns += 1;
                self.push_silence(RATE / 100);
            }
            return;
        }
        // A gap with later packets queued, already covered by a concealment: lost, skip it.
        if self.owed > 0 {
            self.gaps.push_back(self.played_ms);
            self.owed -= 1;
            self.next = Some(n + frame as u64);
            self.shift_level(-(frame as f64) / MS);
            return;
        }
        // Short of the target: it's probably late, wait for it.
        if self.waited < MAX_WAIT && self.level.unwrap_or(ahead) < self.target - 2.0 * ADJUST_MS {
            self.waited += 1;
            self.stats.concealed += 1;
            self.conceal();
            self.shift_level(frame as f64 / MS);
            return;
        }
        // Lost (or too late). Waited for already: its slot was concealed, move on.
        self.next = Some(n + frame as u64);
        if std::mem::take(&mut self.waited) > 0 {
            self.shift_level(-(frame as f64) / MS);
            return;
        }
        // Else recover it from the next frame's FEC, or conceal it.
        self.gaps.push_back(self.played_ms);
        let fec = self.packets.get(&(n + frame as u64)).map(|p| p.data.clone());
        let got = match fec {
            Some(p) => {
                self.stats.fec += 1;
                self.decode(&p, true, frame)
            }
            None => {
                self.stats.concealed += 1;
                self.decode(&[], false, frame)
            }
        };
        match got {
            Some(got) => self.queue(got, 0),
            None => {
                self.push_silence(frame);
                self.stats.frames += 1;
            }
        }
    }

    /// Follow the target: drop one frame or insert one, in quiet audio (or
    /// forced, after a while). Returns whether it produced the audio.
    fn adjust(&mut self, n: u64) -> bool {
        let Some(level) = self.level else { return false };
        let err = level - self.target;
        if (-ADJUST_UNDER_MS..=ADJUST_MS).contains(&err) {
            self.off_since = None;
            return false;
        }
        let since = *self.off_since.get_or_insert(self.played_ms);
        let forced = self.played_ms - since > FORCE_AFTER_MS;
        let cooldown = if err.abs() > 4.0 * ADJUST_MS { 40.0 } else { 100.0 };
        if self.played_ms - self.last_adjust < cooldown || !(self.quiet || forced) {
            return false;
        }
        // The estimate is smoothed: never drop from a buffer that's actually short,
        // nor pad one that's actually long.
        let ahead = self.ahead_ms();
        if (err > 0.0 && ahead < self.target - 2.0 * ADJUST_MS) || (err < 0.0 && ahead > self.target + 2.0 * ADJUST_MS) {
            self.level = Some(level - err / 2.0);
            return false;
        }
        if err > 0.0 {
            // Too much queued: play the next frame's quiet part only.
            let Some(packet) = self.packets.remove(&n) else { return false };
            self.next = Some(n + packet.samples as u64);
            self.stats.red += packet.red as u64;
            let Some(got) = self.decode(&packet.data, false, self.frame) else { return false };
            self.frame = got;
            let skip = got.min(DEFAULT_FRAME);
            let mut sq = 0f32;
            for &s in &self.scratch[..skip * 2] {
                let v = s as f32 / 32768.0;
                sq += v * v;
            }
            let loud = (sq / (skip * 2) as f32).sqrt() > QUIET_RMS;
            if loud && !forced {
                // Speech starts here after all: keep it.
                self.queue(got, 0);
                return true;
            }
            self.queue(got, skip);
            self.stats.frames -= 1;
            self.stats.dropped += 1;
            let ms = skip as f64 / MS;
            self.stats.max_drop_ms = self.stats.max_drop_ms.max(ms as f32);
            self.level = Some(level - ms);
        } else {
            // Too little: one concealment frame here (quiet, so it's not heard).
            self.conceal();
            self.stats.inserted += 1;
            self.level = Some(level + self.frame as f64 / MS);
        }
        self.last_adjust = self.played_ms;
        true
    }
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opus_silence() -> Vec<u8> {
        let mut enc = ropus::Encoder::builder(48_000, ropus::Channels::Mono, ropus::Application::Voip).build().unwrap();
        let mut buf = vec![0u8; 400];
        let n = enc.encode(&[0i16; 960], &mut buf).unwrap();
        buf.truncate(n);
        buf
    }

    #[test]
    fn survives_a_backward_sequence_restart() {
        let pkt = opus_silence();
        let mut jb = JitterBuffer::new();
        let mut out = vec![0f32; 960];
        let mut t = 0.0;
        for k in 0..100u32 {
            jb.push((40_000 + k) as u16, 1_000_000 + k * 960, &pkt, t);
            jb.pull(&mut out);
            jb.pull(&mut out);
            t += 20.0;
        }
        let before = jb.stats.late;
        // The sender restarts its timestamps far behind where we are.
        for k in 0..100u32 {
            jb.push(k as u16, 5000 + k * 960, &pkt, t);
            jb.pull(&mut out);
            jb.pull(&mut out);
            t += 20.0;
        }
        assert!(jb.stats.late - before < 30, "late {}", jb.stats.late - before);
        assert!(jb.playing, "plays again after the restart");
    }

    #[test]
    fn toc_sizes() {
        assert_eq!(opus_samples(&[0xfc]), Some(960)); // CELT FB 20 ms
        assert_eq!(opus_samples(&[0x78]), Some(960)); // Hybrid FB 20 ms
        assert_eq!(opus_samples(&[0x1b, 0x02]), Some(5760)); // SILK NB 60 ms × 2 (code 3)
        assert_eq!(opus_samples(&[0x18]), Some(2880)); // SILK NB 60 ms
        assert_eq!(opus_samples(&[0xf9]), Some(1920)); // CELT 20 ms × 2
        assert_eq!(opus_samples(&[]), None);
    }

    // ── The audit's network simulation (audit/voice/rs), as tests ──
    //
    // Packets of synthetic speech (syllables and pauses, mono, 20 ms, Opus
    // with FEC like a lossy call), sent every 20 ms of the sender's clock;
    // each arrives after a base delay + exponential jitter, sometimes a spike,
    // or never (loss). The receiver pulls 10 ms at a time on its own clock
    // (± ppm of drift).

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
        fn exp(&mut self, mean: f64) -> f64 {
            -mean * (1.0 - self.next()).ln()
        }
    }

    /// ~6 s of speech-like audio: voiced syllables of 80–300 ms with pauses.
    fn speech() -> Vec<i16> {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        let mut pcm = vec![0i16; RATE * 6];
        let mut at = 0usize;
        while at < pcm.len() {
            let len = (80.0 + rng.next() * 220.0) as usize * 48;
            let f0 = 110.0 + rng.next() * 120.0;
            let amp = 3000.0 + rng.next() * 6000.0;
            for i in 0..len.min(pcm.len() - at) {
                let t = i as f64 / RATE as f64;
                let env = (i as f64 / 480.0).min(1.0) * ((len - i) as f64 / 960.0).min(1.0);
                let v: f64 = (1..8).map(|h| (2.0 * std::f64::consts::PI * f0 * h as f64 * t).sin() / h as f64).sum::<f64>() + (rng.next() - 0.5) * 0.3;
                pcm[at + i] = (v * amp * env).clamp(-32000.0, 32000.0) as i16;
            }
            at += len + (60.0 + rng.next() * 400.0) as usize * 48;
        }
        pcm
    }

    fn packets() -> Vec<Vec<u8>> {
        let mut enc = ropus::Encoder::builder(48_000, ropus::Channels::Mono, ropus::Application::Voip)
            .bitrate(ropus::Bitrate::try_bits(64_000).unwrap())
            .complexity(5)
            .inband_fec(ropus::InbandFec::Enabled)
            .packet_loss_perc(5)
            .build()
            .unwrap();
        let mut buf = vec![0u8; 1500];
        speech()
            .as_chunks::<960>()
            .0
            .iter()
            .map(|f| {
                let n = enc.encode(f, &mut buf).unwrap();
                buf[..n].to_vec()
            })
            .collect()
    }

    struct Net {
        secs: f64,
        base: f64,
        jitter: f64,
        spike_p: f64,
        spike: (f64, f64),
        loss: f64,
        /// Receiver clock error (ppm; + = it plays faster than the sender sends).
        drift_ppm: f64,
        /// A padding-only packet (its own sequence number, no payload) after every n-th packet.
        padding_every: usize,
        /// Every this many seconds the network stalls for 300 ms (what's sent meanwhile arrives in a burst).
        stall_every: f64,
        /// From this second on, the path is 150 ms slower (a route change).
        step_at: f64,
        /// Every packet is RED: it also carries the frame before it.
        red: bool,
    }

    const CLEAN: Net = Net { secs: 60.0, base: 20.0, jitter: 0.0, spike_p: 0.0, spike: (0.0, 0.0), loss: 0.0, drift_ppm: 0.0, padding_every: 0, stall_every: 0.0, step_at: 0.0, red: false };

    struct Outcome {
        sent: usize,
        lost: usize,
        stats: JitterStats,
        /// Average audio held while playing (ms).
        avg_delay: f64,
        rebuffer_ms: f64,
    }

    impl Outcome {
        fn repaired_pct(&self) -> f64 {
            100.0 * (self.stats.fec + self.stats.concealed) as f64 / self.sent as f64
        }
        /// Patched over (PLC), as the connection hint and the call report count it.
        fn patched_pct(&self) -> f64 {
            100.0 * self.stats.concealed as f64 / self.sent as f64
        }
    }

    fn simulate(net: &Net, seed: u64) -> Outcome {
        let pk = packets();
        let mut rng = Rng(seed);
        let n = (net.secs * 50.0) as usize;
        let mut arrivals: Vec<(f64, u16, u32, usize)> = Vec::new();
        let mut seq = 0u16;
        let mut lost = 0;
        for k in 0..n {
            let send = k as f64 * 20.0;
            let ts = 3_000_000_000u32.wrapping_add((k * 960) as u32);
            if rng.next() < net.loss {
                lost += 1;
            } else {
                let mut d = net.base + if net.jitter > 0.0 { rng.exp(net.jitter) } else { 0.0 };
                if rng.next() < net.spike_p {
                    d += net.spike.0 + rng.next() * (net.spike.1 - net.spike.0);
                }
                if net.step_at > 0.0 && send >= net.step_at * 1000.0 {
                    d += 150.0;
                }
                if net.stall_every > 0.0 {
                    let cycle = net.stall_every * 1000.0;
                    let into = send % cycle;
                    if send > cycle && into < 300.0 {
                        d = d.max(300.0 - into + net.base);
                    }
                }
                arrivals.push((send + d, seq, ts, k));
            }
            seq = seq.wrapping_add(1);
            if net.padding_every > 0 && k % net.padding_every == 0 {
                arrivals.push((send + net.base + 1.0, seq, ts, usize::MAX));
                seq = seq.wrapping_add(1);
            }
        }
        arrivals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let tick = 10.0 / (1.0 + net.drift_ppm * 1e-6);
        let mut jb = JitterBuffer::new();
        let mut out = vec![0f32; 960];
        let mut red = Vec::new();
        let (mut ai, mut t) = (0, 0.0);
        let (mut delay_sum, mut delay_n, mut rebuffer) = (0.0, 0usize, 0usize);
        let end = net.secs * 1000.0 - 300.0;
        while t < end {
            while ai < arrivals.len() && arrivals[ai].0 <= t {
                let (at, s, ts, k) = arrivals[ai];
                if k == usize::MAX {
                    jb.push(s, ts, &[], at);
                } else if net.red {
                    let prev: &[u8] = if k > 0 { &pk[(k - 1) % pk.len()] } else { &[] };
                    crate::voice::red::pack(&mut red, 111, &pk[k % pk.len()], prev, 960);
                    jb.push_red(s, ts, &red, at);
                } else {
                    jb.push(s, ts, &pk[k % pk.len()], at);
                }
                ai += 1;
            }
            jb.pull(&mut out);
            if t > 3000.0 {
                if jb.playing {
                    delay_sum += jb.stats.delay_ms as f64;
                    delay_n += 1;
                } else {
                    rebuffer += 1;
                }
            }
            t += tick;
        }
        Outcome { sent: n, lost, stats: jb.stats, avg_delay: delay_sum / delay_n.max(1) as f64, rebuffer_ms: rebuffer as f64 * 10.0 }
    }

    fn report(name: &str, o: &Outcome) {
        let s = o.stats;
        println!(
            "{name:<10} sent {} lost {} | late {} red {} fec {} plc {} ({:.2}% repaired, {:.2}% patched) under {} rebuffer {} ms | inserted {} dropped {} (max {} ms) | delay avg {:.1} ms, target {:.0} ms",
            o.sent, o.lost, s.late, s.red, s.fec, s.concealed, o.repaired_pct(), o.patched_pct(), s.underruns, o.rebuffer_ms, s.inserted, s.dropped, s.max_drop_ms, o.avg_delay, s.target_ms
        );
    }

    #[test]
    fn sim_clean_lan_holds_the_minimum() {
        let o = simulate(&CLEAN, 1);
        report("clean", &o);
        assert_eq!(o.stats.fec + o.stats.concealed + o.stats.late, 0);
        assert!(o.avg_delay < 65.0, "delay {:.1} ms", o.avg_delay);
        assert_eq!(o.rebuffer_ms, 0.0);
    }

    #[test]
    fn sim_wifi_like_under_2_percent() {
        // Busy Wi-Fi: 15 ms + exp(10 ms), 2 % spikes of 60–200 ms, 1 % loss.
        let net = Net { secs: 180.0, base: 15.0, jitter: 10.0, spike_p: 0.02, spike: (60.0, 200.0), loss: 0.01, ..CLEAN };
        for seed in [2, 3] {
            let o = simulate(&net, seed);
            report("wifi", &o);
            assert!(o.repaired_pct() < 2.0, "repaired {:.2}%", o.repaired_pct());
        }
    }

    #[test]
    fn sim_cell_like_under_4_percent() {
        // Cellular: 40 ms + exp(20 ms), 3 % spikes of 100–300 ms, 2 % loss.
        let net = Net { secs: 180.0, base: 40.0, jitter: 20.0, spike_p: 0.03, spike: (100.0, 300.0), loss: 0.02, ..CLEAN };
        for seed in [4, 5] {
            let o = simulate(&net, seed);
            report("cell", &o);
            assert!(o.repaired_pct() < 4.0, "repaired {:.2}%", o.repaired_pct());
        }
    }

    #[test]
    fn sim_clock_drift_is_followed_in_small_steps() {
        for ppm in [100.0, -100.0] {
            let net = Net { secs: 600.0, jitter: 1.0, drift_ppm: ppm, ..CLEAN };
            let o = simulate(&net, 6);
            report(if ppm > 0.0 { "fast" } else { "slow" }, &o);
            assert!(o.stats.max_drop_ms <= 20.0, "a drop of {} ms", o.stats.max_drop_ms);
            assert!(o.avg_delay < 80.0, "latency crept to {:.1} ms", o.avg_delay);
            assert_eq!(o.stats.late, 0, "packets arrived late");
            assert_eq!(o.rebuffer_ms, 0.0);
            assert!(o.stats.inserted + o.stats.dropped > 0, "drift was corrected");
        }
    }

    #[test]
    fn sim_stalls_and_a_slower_path_dont_run_away() {
        // Wi-Fi-like with a 300 ms stall every 7 s, and the path 150 ms slower from 60 s on:
        // the buffer must never drop frames out of an empty queue (it once did, in a loop).
        let net = Net { secs: 120.0, base: 15.0, jitter: 10.0, spike_p: 0.02, spike: (60.0, 200.0), loss: 0.01, stall_every: 7.0, step_at: 60.0, ..CLEAN };
        let o = simulate(&net, 8);
        report("stalls", &o);
        assert!(o.stats.dropped < 60, "dropped {} frames", o.stats.dropped);
        assert!(o.repaired_pct() < 8.0, "repaired {:.2}%", o.repaired_pct());
    }

    #[test]
    fn sim_fec_survives_padding_at_5_percent_loss() {
        // Chrome-style probes: a padding-only packet after every 3rd audio packet.
        let net = Net { secs: 120.0, loss: 0.05, padding_every: 3, ..CLEAN };
        let o = simulate(&net, 7);
        report("loss5+pad", &o);
        let share = o.stats.fec as f64 / (o.stats.fec + o.stats.concealed).max(1) as f64;
        assert!(share > 0.6, "FEC covered {:.0}% of losses", share * 100.0);
        assert!(o.stats.padding > 0);
    }

    /// RED on the simulated networks: the frame before rides along, so a lost
    /// (or too late) packet is rebuilt for real instead of roughed in by FEC
    /// or patched over; what's left is two (and more) gone in a row. Same
    /// seeds, RED off vs on.
    #[test]
    fn sim_red_rebuilds_what_fec_only_roughs_in() {
        let nets = [
            ("loss5", Net { secs: 120.0, loss: 0.05, ..CLEAN }, 11),
            ("wifi", Net { secs: 180.0, base: 15.0, jitter: 10.0, spike_p: 0.02, spike: (60.0, 200.0), loss: 0.01, ..CLEAN }, 2),
            ("cell", Net { secs: 180.0, base: 40.0, jitter: 20.0, spike_p: 0.03, spike: (100.0, 300.0), loss: 0.02, ..CLEAN }, 4),
        ];
        for (name, net, seed) in nets {
            let off = simulate(&net, seed);
            let on = simulate(&Net { red: true, ..net }, seed);
            report(name, &off);
            report(&format!("{name}+red"), &on);
            assert!(on.stats.red > 0, "{name}: RED rebuilt nothing");
            // Only for frames that were lost or came too late: never one that made it.
            assert!(on.stats.red <= on.lost as u64 + on.stats.late, "{name}: rebuilt {} of {} lost + {} late", on.stats.red, on.lost, on.stats.late);
            assert!(on.patched_pct() <= off.patched_pct(), "{name}: patched {:.2}% with RED vs {:.2}% without", on.patched_pct(), off.patched_pct());
            assert!(on.repaired_pct() < off.repaired_pct() * 0.2, "{name}: FEC + PLC {:.2}% with RED vs {:.2}% without", on.repaired_pct(), off.repaired_pct());
        }
        // A clean network: every packet arrives, so no copy is ever used.
        let clean = simulate(&Net { red: true, ..CLEAN }, 1);
        assert_eq!(clean.stats.red + clean.stats.fec + clean.stats.concealed, 0);
        assert!(clean.stats.red_packets > 2000);
    }

    /// A copy only for a frame whose own packet hasn't come and whose turn
    /// hasn't passed; never twice; its own packet arriving later wins.
    #[test]
    fn red_copies_fill_only_real_gaps() {
        let pkt = opus_silence();
        let mut red = Vec::new();
        let mut out = vec![0f32; 960];
        let mut jb = JitterBuffer::new();
        let mut push = |jb: &mut JitterBuffer, k: u32, with_prev: bool| {
            crate::voice::red::pack(&mut red, 111, &pkt, if with_prev { &pkt } else { &[] }, 960);
            jb.push_red(k as u16, 1000 + k * 960, &red, k as f64 * 20.0);
        };
        let at = |jb: &JitterBuffer, k: u32| jb.extend(1000 + k * 960);
        // 0, 1, 2 arrive; 3 is lost; 4 brings 3's copy.
        for k in 0..3 {
            push(&mut jb, k, k > 0);
        }
        push(&mut jb, 4, true);
        assert!(jb.packets.get(&at(&jb, 3)).is_some_and(|p| p.red), "3 from 4's copy");
        // 3 turns up after all: its own packet replaces the copy.
        push(&mut jb, 3, true);
        assert!(jb.packets.values().all(|p| !p.red));
        // Play 0 and 1.
        for _ in 0..4 {
            jb.pull(&mut out);
        }
        assert_eq!(jb.next, Some(at(&jb, 2)));
        // 2 again (a duplicate) with its copy of 1, already played: neither goes in.
        push(&mut jb, 2, true);
        assert_eq!(jb.packets.len(), 3);
        assert!(!jb.packets.contains_key(&at(&jb, 1)));
        // 5 is lost; 6 brings its copy in time: rebuilt, nothing patched.
        push(&mut jb, 6, true);
        for _ in 0..8 {
            jb.pull(&mut out);
        }
        assert_eq!((jb.stats.red, jb.stats.concealed, jb.stats.fec), (1, 0, 0));
        assert_eq!(jb.stats.red_packets, 7);
        assert_eq!(jb.stats.expected, 7);
        // Malformed RED: dropped (but counted as arrived, not lost), never a panic.
        jb.push_red(7, 1000 + 7 * 960, &[0x80 | 111, 15], 140.0);
        jb.push_red(8, 1000 + 8 * 960, &[], 160.0);
        assert_eq!((jb.stats.red_packets, jb.stats.expected), (7, 9));
    }
}
