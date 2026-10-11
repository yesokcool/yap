//! Echo cancellation: WebRTC's AEC3, the canceller browsers use, through
//! `sonora` (a pure-Rust port of WebRTC's audio processing, checked against
//! WebRTC's own C++ tests).
//!
//! The far end is exactly what the app plays (every voice, a screen share's
//! sound, the soundboard, chimes; after the mixer's soft clip), tapped where
//! it leaves for the output device (`EchoRef::push`). The near end is the mic
//! after the 80 Hz high-pass; noise removal and automatic gain come after it,
//! which is libwebrtc's own order (high-pass → AEC3 → noise suppression →
//! gain). It runs at 32 kHz inside, libwebrtc's default and so the browser's:
//! the mic keeps everything up to 16 kHz while it's on.
//!
//! The speakers and the mic run on their own threads and clocks, in bursts of
//! whatever size the OS likes, while AEC3 wants one 10 ms far-end frame per
//! 10 ms mic frame, the far end a little ahead. So `Canceller` takes the far
//! end in lockstep with the mic from a small backlog it keeps between `LOW`
//! and `HIGH`: it starts over when the output stream changes, takes silence
//! for anything not played yet (and skips it when it turns up, so the two
//! stay lined up), and brings the backlog back when the two clocks drift
//! apart. AEC3 estimates the rest of the delay (the device buffers and the
//! room) by itself. Measured offline in `echo_sim.rs`.

use parking_lot::Mutex;
use std::collections::VecDeque;
use std::sync::Arc;

use super::{BLOCK, RATE};

/// More than this waiting means nobody's taking it (no mic open): keep the newest.
const RING_MAX: usize = RATE as usize;
/// The backlog (played, not yet handed to AEC3) is kept between these, judged
/// by its smallest value over a `WINDOW`…
const LOW: i64 = RATE as i64 / 1000; // 1 ms
const HIGH: i64 = RATE as i64 / 50; // 20 ms
/// …and brought back to this when it leaves them.
const TARGET: i64 = RATE as i64 * 8 / 1000; // 8 ms
/// 1 s of 10 ms blocks; the first window after starting over is shorter, to
/// line up quickly.
const WINDOW: u32 = 100;
const FIRST_WINDOW: u32 = 50;
/// Corrections up to this go one sample per block; bigger ones at once
/// (AEC3 finds the new delay within a few hundred ms).
const JUMP: i64 = RATE as i64 / 500; // 2 ms
/// Waiting this long for audio that should have been played means the output
/// stalled: line up again when it's back.
const STALL: usize = RATE as usize / 5; // 200 ms

/// What the app plays, on its way to the echo canceller. The output side
/// pushes; the capture thread's `Canceller` takes.
#[derive(Default)]
pub struct EchoRef {
    inner: Mutex<Ring>,
}

#[derive(Default)]
struct Ring {
    /// Mono, 48 kHz, oldest first.
    samples: VecDeque<f32>,
    /// Bumped whenever the stream feeding us starts or stops: the canceller lines up again.
    epoch: u32,
    /// An output stream is feeding us.
    live: bool,
}

impl EchoRef {
    /// An output stream started (or replaced the last one).
    pub fn start(&self) {
        let mut r = self.inner.lock();
        r.samples.clear();
        r.epoch = r.epoch.wrapping_add(1);
        r.live = true;
    }

    /// The output stream stopped: nothing's playing.
    pub fn stop(&self) {
        let mut r = self.inner.lock();
        r.samples.clear();
        r.epoch = r.epoch.wrapping_add(1);
        r.live = false;
    }

    /// Interleaved stereo at 48 kHz, exactly as it goes to the device.
    pub fn push(&self, stereo: &[f32]) {
        let mut r = self.inner.lock();
        if !r.live {
            return;
        }
        r.samples.extend(stereo.as_chunks::<2>().0.iter().map(|[l, r]| (l + r) * 0.5));
        let over = r.samples.len().saturating_sub(RING_MAX);
        if over > 0 {
            r.samples.drain(..over);
        }
    }
}

/// What the canceller is doing, for the logs and the call report.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EchoStats {
    /// AEC3 is running on the mic.
    pub on: bool,
    /// AEC3's estimate of the delay from playing to hearing, ms (the
    /// speakers' buffer, the room, the mic's buffer), once it has one.
    pub delay_ms: Option<i32>,
    /// How much echo it takes out (echo return loss enhancement), dB.
    pub erle_db: Option<f64>,
    /// Far-end samples taken as silence because they hadn't been played yet.
    pub underruns: u64,
    /// Times it lined up again (a new output stream, a stall, a big correction).
    pub realigned: u64,
}

/// The echo canceller, on the capture thread.
pub struct Canceller {
    apm: Option<Box<sonora::AudioProcessing>>,
    reference: Arc<EchoRef>,
    epoch: Option<u32>,
    /// Waiting for the output to come back after a stall.
    stalled: bool,
    /// The echo path or its delay changed (new speakers, a stall, a big
    /// correction): AEC3 starts fresh.
    fresh: bool,
    /// Far-end samples taken as silence (not played yet); skipped when they arrive.
    owed: usize,
    /// Silence to feed before taking more (raises the backlog at once).
    silence: usize,
    /// Samples to drop from the far end (+) or to repeat (−), one per block.
    adjust: i64,
    /// The smallest backlog this window, how many blocks into it we are, and its length.
    low: i64,
    blocks: u32,
    window: u32,
    far: [f32; BLOCK],
    scratch: [f32; BLOCK],
    near: [f32; BLOCK],
    last: f32,
    stats: EchoStats,
}

impl Canceller {
    pub fn new(reference: Arc<EchoRef>) -> Self {
        Self {
            apm: None,
            reference,
            epoch: None,
            stalled: false,
            fresh: false,
            owed: 0,
            silence: 0,
            adjust: 0,
            low: i64::MAX,
            blocks: 0,
            window: FIRST_WINDOW,
            far: [0.0; BLOCK],
            scratch: [0.0; BLOCK],
            near: [0.0; BLOCK],
            last: 0.0,
            stats: EchoStats::default(),
        }
    }

    pub fn stats(&self) -> EchoStats {
        self.stats
    }

    /// One 10 ms block of the mic (48 kHz mono), its echo taken out in place
    /// when `on`. Call it for every block, on or off, so the far end stays
    /// lined up.
    pub fn process(&mut self, on: bool, mic: &mut [f32]) {
        debug_assert_eq!(mic.len(), BLOCK);
        self.take_far();
        if std::mem::take(&mut self.fresh) {
            // A fresh AEC3 settles faster than one hunting for a delay that jumped.
            self.apm = None;
        }
        if !on {
            // Off: let AEC3 go (back on, it starts fresh and settles within a second or two).
            if self.apm.take().is_some() {
                self.stats.on = false;
                self.stats.delay_ms = None;
                self.stats.erle_db = None;
            }
            return;
        }
        let apm = self.apm.get_or_insert_with(|| Box::new(new_apm()));
        let _ = apm.process_render_f32(&[&self.far], &mut [&mut self.scratch]);
        self.near.copy_from_slice(mic);
        let _ = apm.process_capture_f32(&[&self.near], &mut [mic]);
        let s = apm.statistics();
        self.stats.on = true;
        self.stats.delay_ms = s.delay_ms;
        self.stats.erle_db = s.echo_return_loss_enhancement;
    }

    /// The next 10 ms of what was played, in lockstep with the mic.
    fn take_far(&mut self) {
        let reference = self.reference.clone();
        let mut r = reference.inner.lock();
        if self.epoch != Some(r.epoch) {
            // A new output stream (or none): line up from scratch.
            self.epoch = Some(r.epoch);
            self.stalled = false;
            self.start_over(&mut r);
        }
        if !r.live || (self.stalled && r.samples.is_empty()) {
            // Nothing's playing: silence is exactly what the far end is.
            self.far.fill(0.0);
            self.last = 0.0;
            return;
        }
        if self.stalled {
            self.stalled = false;
            self.start_over(&mut r);
        }
        // Skip what was taken as silence before it was played.
        let skip = self.owed.min(r.samples.len());
        r.samples.drain(..skip);
        self.owed -= skip;
        let mut filled = 0;
        if self.silence > 0 {
            filled = self.silence.min(BLOCK);
            self.far[..filled].fill(0.0);
            self.silence -= filled;
        }
        // A small correction: one sample dropped or repeated per block.
        if filled < BLOCK && self.adjust != 0 {
            if self.adjust > 0 && r.samples.len() > BLOCK {
                r.samples.pop_front();
                self.adjust -= 1;
            } else if self.adjust < 0 {
                self.far[filled] = self.last;
                filled += 1;
                self.adjust += 1;
            }
            if self.adjust == 0 {
                self.new_window(WINDOW);
            }
        }
        let want = BLOCK - filled;
        let have = want.min(r.samples.len());
        for (d, s) in self.far[filled..filled + have].iter_mut().zip(r.samples.drain(..have)) {
            *d = s;
        }
        if have < want {
            // Not played yet (the speakers' callback runs late, or in big bursts).
            self.far[filled + have..].fill(0.0);
            self.owed += want - have;
            self.stats.underruns += (want - have) as u64;
            if self.owed > STALL {
                // The output stalled; line up again when it's back.
                self.stalled = true;
                self.owed = 0;
                return;
            }
        }
        self.last = self.far[BLOCK - 1];
        let backlog = r.samples.len() as i64 - self.owed as i64;
        self.low = self.low.min(backlog);
        self.blocks += 1;
        if self.blocks >= self.window {
            let low = self.low;
            self.new_window(WINDOW);
            if self.adjust == 0 && self.silence == 0 && !(LOW..=HIGH).contains(&low) {
                self.correct(&mut r, low - TARGET);
            }
        }
    }

    /// Bring the backlog down (`by` > 0) or up (`by` < 0).
    fn correct(&mut self, r: &mut Ring, by: i64) {
        if by.abs() <= JUMP {
            self.adjust = by;
            return;
        }
        if by > 0 {
            let n = (by as usize).min(r.samples.len());
            r.samples.drain(..n);
        } else {
            self.silence = (-by) as usize;
        }
        // AEC3 would hunt for the new delay; a fresh one settles faster (measured in echo_sim).
        self.fresh = true;
        self.stats.realigned += 1;
    }

    fn new_window(&mut self, len: u32) {
        self.low = i64::MAX;
        self.blocks = 0;
        self.window = len;
    }

    fn start_over(&mut self, r: &mut Ring) {
        self.fresh = true;
        // Keep at most the newest `HIGH`, and start with at least `TARGET` left
        // after the next take (silence makes up the rest); the first window
        // then checks it.
        let n = r.samples.len().saturating_sub(HIGH as usize);
        r.samples.drain(..n);
        self.owed = 0;
        self.silence = (TARGET as usize + BLOCK).saturating_sub(r.samples.len());
        self.adjust = 0;
        self.last = 0.0;
        self.new_window(FIRST_WINDOW);
        self.stats.realigned += 1;
    }
}

fn new_apm() -> sonora::AudioProcessing {
    use sonora::config::{EchoCanceller, Pipeline};
    let config = sonora::Config {
        // Mono in, mono out; processing at 32 kHz inside (the default).
        pipeline: Pipeline { multi_channel_render: false, multi_channel_capture: false, ..Default::default() },
        // Our own 80 Hz high-pass runs just before.
        echo_canceller: Some(EchoCanceller { enforce_high_pass_filtering: false, ..Default::default() }),
        ..Default::default()
    };
    let stream = sonora::StreamConfig::new(RATE, 1);
    sonora::AudioProcessing::builder().config(config).capture_config(stream).render_config(stream).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(v: f32) -> Vec<f32> {
        vec![v; BLOCK * 2]
    }

    #[test]
    fn far_end_is_taken_in_lockstep_and_lined_up_after_a_late_burst() {
        let r = Arc::new(EchoRef::default());
        r.start();
        let mut c = Canceller::new(r.clone());
        // Two blocks played, then the mic asks for three: the third is owed as silence…
        r.push(&block(0.1));
        r.push(&block(0.2));
        c.take_far();
        assert_eq!(c.far[0], 0.1);
        c.take_far();
        assert_eq!(c.far[0], 0.2);
        c.take_far();
        assert_eq!(c.far[0], 0.0, "not played yet: silence");
        assert_eq!(c.owed, BLOCK);
        // …and skipped when it turns up late, so the next block lines up with the mic.
        r.push(&block(0.3));
        r.push(&block(0.4));
        c.take_far();
        assert_eq!(c.far[0], 0.4);
        assert_eq!(c.owed, 0);
        assert_eq!(c.stats.underruns, BLOCK as u64);
    }

    #[test]
    fn nothing_playing_is_silence_and_a_new_stream_starts_over() {
        let r = Arc::new(EchoRef::default());
        let mut c = Canceller::new(r.clone());
        r.push(&block(0.5)); // no stream yet: ignored
        c.take_far();
        assert!(c.far.iter().all(|&x| x == 0.0));
        // A stream with a long backlog (the mic opened late): only the newest `HIGH` is kept.
        r.start();
        for _ in 0..20 {
            r.push(&block(0.5));
        }
        c.take_far();
        assert_eq!(r.inner.lock().samples.len(), HIGH as usize - BLOCK);
        // The output stalls: after `STALL` of nothing, silence without owing anything.
        for _ in 0..30 {
            c.take_far();
        }
        assert!(c.stalled);
        assert_eq!(c.owed, 0);
        r.stop();
        c.take_far();
        assert!(!c.stalled && c.far.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn off_leaves_the_mic_alone() {
        let r = Arc::new(EchoRef::default());
        r.start();
        let mut c = Canceller::new(r.clone());
        let mut mic: Vec<f32> = (0..BLOCK).map(|i| (i as f32 * 0.01).sin() * 0.3).collect();
        let before = mic.clone();
        r.push(&block(0.2));
        c.process(false, &mut mic);
        assert_eq!(mic, before);
        assert!(!c.stats().on);
        c.process(true, &mut mic);
        assert!(c.stats().on);
    }
}
