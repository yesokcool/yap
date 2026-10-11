//! The transmit gate (PROTOCOL.md › Voice › Transmit gate), a port of the
//! web's `web/js/gate-worklet.js` with the same numbers and the same tests.
//! It runs on the capture-processing thread, on the processed mic (after
//! noise removal and gain), one 10 ms block at a time:
//!
//!  - Voice activity: opens when 10 ms of sound is louder than your
//!    sensitivity (or than the room's noise + 12 dB, whichever is higher: the
//!    adaptive noise floor), stays open while it's within 6 dB of that but
//!    still clear of the noise (hysteresis), and for 450 ms after (hold). The
//!    output runs 40 ms behind the mic (pre-roll), so the gate opens *before*
//!    the first syllable. Voice activity only: push-to-talk and always-on
//!    have no extra delay.
//!  - Push to talk: open while the key is down, plus a 200 ms tail.
//!  - Opening fades in over 5 ms, closing fades out over 15 ms (no clicks).
//!  - Every 20 ms it reports the level (dBFS RMS, before the gate), whether
//!    it's open, the noise floor, and whether its input has been nothing but
//!    digital silence for 5 s.
//!
//! Mute and deafen are not its business: the caller zeroes the output
//! outside the gate, so they hold whatever the gate thinks.

use std::collections::VecDeque;

pub const WINDOW_MS: f64 = 10.0;
pub const HOLD_MS: f64 = 450.0;
pub const HYSTERESIS_DB: f32 = 6.0;
pub const FADE_IN_MS: f64 = 5.0;
pub const FADE_OUT_MS: f64 = 15.0;
pub const PTT_TAIL_MS: f64 = 200.0;
/// Voice activity only (TD3: CLAUDE.md › Decision log).
pub const PRE_ROLL_MS: f64 = 40.0;
/// The adaptive part: open only this far above the room's noise…
const FLOOR_MARGIN_DB: f32 = 12.0;
/// …and close once it's back within this much of it…
const FLOOR_CLOSE_MARGIN_DB: f32 = 8.0;
/// …but never ask for more than this (a loud room still gets through).
const FLOOR_CAP_DB: f32 = -30.0;
/// Noise floor = the quietest 10 ms in each 500 ms, the quietest of the last 8.
const FLOOR_BUCKET_MS: f64 = 500.0;
const FLOOR_BUCKETS: usize = 8;
const REPORT_MS: f64 = 20.0;
const SILENCE_MS: f64 = 5000.0;
/// Below one 24-bit step: digital silence, not a quiet room.
const SILENCE_PEAK: f32 = 1.0 / (1u32 << 23) as f32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Vad,
    Ptt,
    Open,
}

/// What the gate says every 20 ms (the web's report; the app reads the
/// level and where it opens, the tests read it all).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct Report {
    /// Level of the last 20 ms (dBFS RMS, before the gate).
    pub db: f32,
    pub open: bool,
    /// The room's noise floor (dBFS).
    pub floor: f32,
    /// Where voice activity opens right now.
    pub at: f32,
    /// Nothing but digital silence for 5 s.
    pub silent: bool,
}

const NEVER: i64 = -1;
const HELD: i64 = i64::MAX;

fn to_db(mean_square: f64) -> f32 {
    if mean_square > 1e-12 { (10.0 * mean_square.log10()) as f32 } else { -120.0 }
}

pub struct Gate {
    window: usize,
    hold: i64,
    tail: i64,
    pre_roll: usize,
    windows_per_bucket: usize,
    bucket_windows: usize,
    report_len: usize,
    silence_len: usize,
    step_in: f32,
    step_out: f32,
    mode: Mode,
    threshold: f32,
    ptt: bool,
    /// A mic test: push-to-talk acts like always-on.
    test: bool,
    /// Samples processed.
    t: i64,
    open: bool,
    hold_until: i64,
    gain: f32,
    ring: Vec<f32>,
    w: usize,
    delay: usize,
    win_sum: f64,
    win_n: usize,
    rep_sum: f64,
    rep_n: usize,
    rep_peak: f32,
    silent_for: usize,
    buckets: VecDeque<f32>,
    bucket_min: f32,
    bucket_n: usize,
    floor: f32,
    /// Level of the last 10 ms window (dBFS).
    pub level: f32,
}

impl Gate {
    pub fn new(rate: u32, mode: Mode, threshold: f32) -> Self {
        let ms = |v: f64| (v / 1000.0 * rate as f64).round() as usize;
        let open = mode == Mode::Open;
        let mut g = Self {
            window: ms(WINDOW_MS),
            hold: ms(HOLD_MS) as i64,
            tail: ms(PTT_TAIL_MS) as i64,
            pre_roll: ms(PRE_ROLL_MS),
            windows_per_bucket: (FLOOR_BUCKET_MS / WINDOW_MS).round() as usize,
            bucket_windows: 0,
            report_len: ms(REPORT_MS),
            silence_len: ms(SILENCE_MS),
            step_in: 1.0 / ms(FADE_IN_MS) as f32,
            step_out: 1.0 / ms(FADE_OUT_MS) as f32,
            mode,
            threshold,
            ptt: false,
            test: false,
            t: 0,
            open,
            hold_until: NEVER,
            gain: if open { 1.0 } else { 0.0 },
            ring: vec![0.0; ms(100.0)],
            w: 0,
            delay: 0,
            win_sum: 0.0,
            win_n: 0,
            rep_sum: 0.0,
            rep_n: 0,
            rep_peak: 0.0,
            silent_for: 0,
            buckets: VecDeque::new(),
            bucket_min: 0.0,
            bucket_n: 0,
            floor: -120.0,
            level: -120.0,
        };
        g.delay = g.delay_for(mode);
        g
    }

    fn delay_for(&self, mode: Mode) -> usize {
        if mode == Mode::Vad { self.pre_roll } else { 0 }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = threshold;
    }

    pub fn set_ptt(&mut self, down: bool) {
        if down == self.ptt {
            return;
        }
        self.ptt = down;
        if down {
            self.hold_until = HELD;
        } else if self.hold_until == HELD {
            self.hold_until = self.t + self.tail;
        }
        self.decide();
    }

    pub fn set_test(&mut self, test: bool) {
        self.test = test;
        self.decide();
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.hold_until = if mode == Mode::Ptt && self.ptt { HELD } else { NEVER };
        let delay = self.delay_for(mode);
        if delay != self.delay {
            // A different delay: start the line over (a brief hiccup when switching modes).
            self.delay = delay;
            self.ring.fill(0.0);
        }
        self.decide();
    }

    /// What voice activity opens at right now (dBFS): your sensitivity, or higher in a noisy room.
    pub fn open_at(&self) -> f32 {
        self.threshold.max((self.floor + FLOOR_MARGIN_DB).min(FLOOR_CAP_DB))
    }

    /// What keeps it open (hysteresis): 6 dB under that, but never down into the room's noise.
    pub fn close_at(&self) -> f32 {
        (self.threshold - HYSTERESIS_DB).max((self.floor + FLOOR_CLOSE_MARGIN_DB).min(FLOOR_CAP_DB - HYSTERESIS_DB))
    }

    fn decide(&mut self) {
        let mode = if self.mode == Mode::Ptt && self.test { Mode::Open } else { self.mode };
        self.open = match mode {
            Mode::Open => true,
            Mode::Ptt => self.ptt || self.t < self.hold_until,
            Mode::Vad => self.t < self.hold_until,
        };
    }

    fn on_window(&mut self, mean_square: f64) {
        let db = to_db(mean_square);
        self.level = db;
        // Noise floor: quietest window per bucket, quietest bucket of the last few seconds.
        // Digital silence (a muted device) says nothing about the room, so it's skipped.
        if db > -110.0 {
            self.bucket_min = if self.bucket_n > 0 { self.bucket_min.min(db) } else { db };
            self.bucket_n += 1;
        }
        self.bucket_windows += 1;
        if self.bucket_windows == self.windows_per_bucket {
            self.bucket_windows = 0;
            if self.bucket_n > 0 {
                self.buckets.push_back(self.bucket_min);
            }
            if self.buckets.len() > FLOOR_BUCKETS {
                self.buckets.pop_front();
            }
            self.bucket_n = 0;
            self.floor = self.buckets.iter().copied().fold(f32::INFINITY, f32::min);
            if !self.floor.is_finite() {
                self.floor = -120.0;
            }
        }
        if self.mode == Mode::Vad && db > if self.open { self.close_at() } else { self.open_at() } {
            self.hold_until = self.t + self.hold;
        }
        self.decide();
    }

    /// One block of mono audio: `output` gets the gated (and, in voice
    /// activity mode, 40 ms delayed) input. Returns a report every ~20 ms.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Option<Report> {
        let size = self.ring.len();
        let mut report = None;
        for (i, out) in output.iter_mut().enumerate() {
            let x = input.get(i).copied().unwrap_or(0.0);
            let sq = (x as f64) * (x as f64);
            self.win_sum += sq;
            self.rep_sum += sq;
            self.rep_peak = self.rep_peak.max(x.abs());
            let r = (self.w + size - self.delay) % size;
            self.ring[self.w] = x;
            *out = self.ring[r] * self.gain;
            self.w = (self.w + 1) % size;
            self.t += 1;
            self.gain = if self.open { (self.gain + self.step_in).min(1.0) } else { (self.gain - self.step_out).max(0.0) };
            self.win_n += 1;
            if self.win_n == self.window {
                let ms = self.win_sum / self.window as f64;
                self.on_window(ms);
                self.win_sum = 0.0;
                self.win_n = 0;
            } else if self.mode != Mode::Vad && self.win_n.is_multiple_of(128) {
                self.decide(); // push-to-talk's tail ends on time, not on a window edge
            }
            self.rep_n += 1;
            if self.rep_n == self.report_len {
                self.silent_for = if self.rep_peak < SILENCE_PEAK { self.silent_for + self.rep_n } else { 0 };
                report = Some(Report {
                    db: to_db(self.rep_sum / self.rep_n as f64),
                    open: self.open,
                    floor: self.floor,
                    at: self.open_at(),
                    silent: self.silent_for >= self.silence_len,
                });
                self.rep_sum = 0.0;
                self.rep_n = 0;
                self.rep_peak = 0.0;
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    //! The web's gate tests (test/voice.test.mjs), on the same synthetic audio.
    use super::*;

    const SR: u32 = 48_000;
    fn ms(v: f64) -> usize {
        (v / 1000.0 * SR as f64).round() as usize
    }

    struct Rnd(u64);
    impl Rnd {
        fn next(&mut self) -> f32 {
            self.0 = (self.0 * 16807) % 2147483647;
            (self.0 as f64 / 2147483647.0 * 2.0 - 1.0) as f32
        }
    }

    fn db_to_amp(db: f32) -> f32 {
        10f32.powf(db / 20.0)
    }

    /// White noise at an RMS level.
    fn noise(rnd: &mut Rnd, n: usize, db: f32) -> Vec<f32> {
        (0..n).map(|_| rnd.next() * db_to_amp(db) * 3f32.sqrt()).collect()
    }

    /// A "syllable": a 220 Hz buzz at `db` RMS with a `rise` ms attack, starting at sample `at`.
    fn syllable(x: &mut [f32], at: usize, len_ms: f64, db: f32, rise: f64) {
        let len = ms(len_ms);
        for i in 0..len {
            if at + i >= x.len() {
                break;
            }
            let env = (i as f32 / ms(rise) as f32).min(1.0) * ((len - i) as f32 / ms(10.0) as f32).min(1.0);
            x[at + i] += (2.0 * std::f32::consts::PI * 220.0 * i as f32 / SR as f32).sin() * db_to_amp(db) * std::f32::consts::SQRT_2 * env;
        }
    }

    enum Ev {
        Ptt(bool),
    }

    struct Run {
        out: Vec<f32>,
        /// The gain each *input* sample went out at (the pre-roll delay accounted for).
        gain_of: Vec<f32>,
        /// The gain at each output sample.
        gains: Vec<f32>,
        reports: Vec<(usize, Report)>,
    }

    /// Run audio through a Gate. Events (the push-to-talk key) land at the
    /// start of their 10 ms block, like the capture thread's; samples go
    /// through one at a time, which is the same as a block (the gate is
    /// sequential), so the gain can be read at every sample.
    fn run(x: &[f32], mode: Mode, threshold: f32, ptt: bool, test: bool, events: &[(usize, Ev)]) -> Run {
        let mut g = Gate::new(SR, mode, threshold);
        g.set_ptt(ptt);
        g.set_test(test);
        let delay = if mode == Mode::Vad { ms(PRE_ROLL_MS) } else { 0 };
        let n = x.len() + delay;
        let (mut out, mut gains, mut reports) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::new());
        let mut o = [0.0f32];
        for i in 0..n {
            if i % 480 == 0 {
                for (at, e) in events {
                    if (i..i + 480).contains(at) {
                        match e {
                            Ev::Ptt(d) => g.set_ptt(*d),
                        }
                    }
                }
            }
            gains.push(g.gain);
            if let Some(r) = g.process(&[x.get(i).copied().unwrap_or(0.0)], &mut o) {
                reports.push((i, r));
            }
            out.push(o[0]);
        }
        let gain_of = (0..x.len()).map(|k| gains[k + delay]).collect();
        out.truncate(x.len());
        Run { out, gain_of, gains, reports }
    }

    #[test]
    fn opens_before_the_first_syllable_and_holds() {
        let mut rnd = Rnd(3);
        let mut x = noise(&mut rnd, SR as usize * 3, -75.0);
        let at = ms(1200.0);
        syllable(&mut x, at, 300.0, -30.0, 20.0);
        let r = run(&x, Mode::Vad, -50.0, false, false, &[]);
        assert!(r.gain_of[at] >= 0.99, "the syllable's very first sample goes out at full volume (gain {})", r.gain_of[at]);
        assert_eq!(r.gain_of[at - ms(100.0)], 0.0, "closed well before it");
        let end = at + ms(300.0);
        assert!(r.gain_of[end + ms(400.0)] > 0.99, "still open 400 ms after (hold)");
        assert_eq!(r.gain_of[end + ms(HOLD_MS + FADE_OUT_MS + 30.0)], 0.0, "closed after the hold and fade");
    }

    #[test]
    fn soft_attacks_are_not_clipped() {
        let mut rnd = Rnd(3);
        let mut x = noise(&mut rnd, SR as usize * 12, -70.0);
        let starts: Vec<usize> = (0..20).map(|i| ms(400.0 + i as f64 * 560.0)).collect();
        // Quiet-ish speech (-34…-42 dBFS) with slow attacks (10–40 ms), just over the -50 sensitivity.
        for (i, &at) in starts.iter().enumerate() {
            syllable(&mut x, at, 120.0, -34.0 - (i % 3) as f32 * 4.0, 10.0 + (i % 4) as f64 * 10.0);
        }
        let r = run(&x, Mode::Vad, -50.0, false, false, &[]);
        // Ground truth onset: where the syllable passes -60 dBFS (what a listener would notice).
        let clipped: Vec<f64> = starts
            .iter()
            .map(|&at| {
                let mut k = at;
                while x[k].abs() < db_to_amp(-60.0) && k < at + ms(80.0) {
                    k += 1;
                }
                let mut first = k;
                while r.gain_of[first] < 0.5 && first < at + ms(200.0) {
                    first += 1;
                }
                (first - k) as f64 / 48.0
            })
            .collect();
        assert!(clipped.iter().all(|&c| c == 0.0), "clipped ms per syllable: {clipped:?}");
    }

    #[test]
    fn hysteresis_keeps_it_open_but_never_opens_it() {
        let mut rnd = Rnd(3);
        let mut x = noise(&mut rnd, SR as usize * 4, -75.0);
        syllable(&mut x, ms(500.0), 1500.0, -53.0, 1.0); // between -56 (close) and -50 (open)
        let r = run(&x, Mode::Vad, -50.0, false, false, &[]);
        assert_eq!(r.gain_of.iter().copied().fold(0.0, f32::max), 0.0, "never louder than the sensitivity: doesn't open");
        let mut y = noise(&mut rnd, SR as usize * 4, -75.0);
        syllable(&mut y, ms(500.0), 100.0, -40.0, 1.0); // opens it…
        syllable(&mut y, ms(600.0), 1500.0, -53.0, 1.0); // …then sits under the open level but over the close level
        let r = run(&y, Mode::Vad, -50.0, false, false, &[]);
        assert!(r.gain_of[ms(1900.0)] > 0.99, "held open by hysteresis well past the 450 ms hold");
    }

    #[test]
    fn fades_in_and_out() {
        let x = vec![0.25f32; SR as usize * 2];
        let r = run(&x, Mode::Ptt, -50.0, false, false, &[(ms(500.0), Ev::Ptt(true)), (ms(1000.0), Ev::Ptt(false))]);
        let max_step = r.gains.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(max_step <= 1.0 / ms(FADE_IN_MS) as f32 + 1e-6, "largest gain step {max_step}");
        let x2: Vec<f32> = r.out.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
        assert!(x2.iter().copied().fold(0.0, f32::max) < 0.25 / ms(FADE_IN_MS) as f32 + 1e-4, "no click in the output");
    }

    #[test]
    fn push_to_talk_opens_at_once_and_keeps_a_tail() {
        let x = vec![0.25f32; SR as usize * 2];
        let (press, release) = (ms(500.0), ms(1000.0));
        let r = run(&x, Mode::Ptt, -50.0, false, false, &[(press, Ev::Ptt(true)), (release, Ev::Ptt(false))]);
        assert_eq!(r.out[press - 200], 0.0, "silent before the key");
        assert!(r.out[press + ms(FADE_IN_MS) + 480] > 0.24, "full volume within a block + the 5 ms fade");
        assert!(r.out[release + ms(150.0)] > 0.24, "still sending 150 ms after release");
        assert_eq!(r.out[release + ms(PTT_TAIL_MS + FADE_OUT_MS) + 960], 0.0, "closed after the tail");
    }

    #[test]
    fn mic_test_makes_ptt_always_on_and_open_has_no_delay() {
        let x = vec![0.25f32; SR as usize];
        let r = run(&x, Mode::Ptt, -50.0, false, true, &[]);
        assert!(r.out[ms(10.0)] > 0.24);
        let r = run(&x, Mode::Open, -50.0, false, false, &[]);
        assert!(r.out[1] > 0.24, "always-on starts open and undelayed");
    }

    #[test]
    fn adaptive_noise_floor() {
        // A fan at -52 dBFS stays out; a soft voice over it gets in.
        let mut rnd = Rnd(3);
        let x = noise(&mut rnd, SR as usize * 20, -52.0);
        let r = run(&x, Mode::Vad, -50.0, false, false, &[]);
        let open_pct = 100.0 * r.gain_of.iter().filter(|&&g| g > 0.5).count() as f64 / r.gain_of.len() as f64;
        assert!(open_pct < 10.0, "open {open_pct:.1}% of the time on fan noise alone (a fixed -50 gate: ~100%)");
        let mut y = noise(&mut rnd, SR as usize * 6, -52.0);
        syllable(&mut y, ms(4500.0), 400.0, -34.0, 20.0);
        let r = run(&y, Mode::Vad, -50.0, false, false, &[]);
        assert!(r.gain_of[ms(4500.0) + ms(50.0)] > 0.99, "speech 18 dB over the noise opens it");
        let floor = r.reports.last().unwrap().1.floor;
        assert!(floor > -60.0 && floor < -50.0, "floor estimate {floor}");
    }

    #[test]
    fn reports_every_20_ms_and_digital_silence_after_5_s() {
        let zeros = vec![0.0f32; SR as usize * 6];
        let r = run(&zeros, Mode::Open, -50.0, false, false, &[]);
        assert!((r.reports.len() as i64 - 300).abs() <= 2, "{} reports in 6 s", r.reports.len());
        assert!(!r.reports.iter().find(|(t, _)| *t < ms(4900.0)).unwrap().1.silent);
        assert!(r.reports.last().unwrap().1.silent);
        let mut rnd = Rnd(3);
        let room = run(&noise(&mut rnd, SR as usize * 6, -90.0), Mode::Open, -50.0, false, false, &[]);
        let last = room.reports.last().unwrap().1;
        assert!(!last.silent, "a very quiet but real mic is not \"silence\"");
        assert!((last.db + 90.0).abs() < 1.5, "level {} dBFS", last.db);
    }
}
