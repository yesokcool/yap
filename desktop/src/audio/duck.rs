//! Lower shared media while friends talk (PROTOCOL.md › Shared media, for
//! yourself; Settings › Voice, per device, off by default): while anyone else
//! in your call is speaking, a screen share's sound eases down to 50 % of your
//! volume over ~150 ms, and back over ~600 ms once nobody has spoken for 1 s.
//! A factor on top of the volume you saved, which it never changes. The
//! web's `web/js/duck.js` (`Ducker`), the same numbers; the mixer steps it
//! every 10 ms block.

/// Where it dips to, how fast, and how long it waits after the last word.
pub const LEVEL: f32 = 0.5;
pub const DOWN_MS: f64 = 150.0;
pub const UP_MS: f64 = 600.0;
pub const QUIET_MS: f64 = 1000.0;

#[derive(Debug)]
pub struct Ducker {
    /// × your volume right now (1 = as you set it).
    pub factor: f32,
    last_talk: f64,
    at: Option<f64>,
}

impl Default for Ducker {
    fn default() -> Self {
        Self { factor: 1.0, last_talk: f64::NEG_INFINITY, at: None }
    }
}

impl Ducker {
    /// Move on to `now` (ms): `talking` = someone else in the call is
    /// speaking (and you'd hear them); `on` = the setting.
    pub fn step(&mut self, now: f64, talking: bool, on: bool) -> f32 {
        let dt = self.at.map_or(0.0, |at| (now - at).max(0.0));
        self.at = Some(now);
        if on && talking {
            self.last_talk = now;
        }
        let target = if on && now - self.last_talk < QUIET_MS { LEVEL } else { 1.0 };
        let span = 1.0 - LEVEL as f64;
        if self.factor > target {
            self.factor = target.max(self.factor - (span * dt / DOWN_MS) as f32);
        } else if self.factor < target {
            self.factor = target.min(self.factor + (span * dt / UP_MS) as f32);
        }
        self.factor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The web's cases (test/unit.test.mjs): down within ~150 ms, held 1 s
    /// after the last word, back within ~600 ms, off = untouched.
    #[test]
    fn dips_while_friends_talk_and_comes_back() {
        let mut d = Ducker::default();
        let mut run = |from: u32, to: u32, talking: bool, on: bool| {
            let mut f = 1.0;
            for t in (from..=to).step_by(10) {
                f = d.step(t as f64, talking, on);
            }
            f
        };
        assert_eq!(run(0, 1000, false, true), 1.0, "nobody talking: your volume");
        let half_way = run(1010, 1080, true, true);
        assert!(half_way > LEVEL && half_way < 1.0, "on its way down ({half_way})");
        assert_eq!(run(1090, 1160, true, true), LEVEL, "at 50 % within ~150 ms");
        assert_eq!(run(1170, 3300, true, true), LEVEL, "held while they talk");
        assert_eq!(run(3310, 4290, false, true), LEVEL, "and for a second after the last word");
        let rising = run(4300, 4600, false, true);
        assert!(rising > LEVEL && rising < 1.0, "coming back ({rising})");
        assert_eq!(run(4610, 4910, false, true), 1.0, "back to your volume within ~600 ms");
        assert_eq!(run(4920, 6000, true, false), 1.0, "switched off: friends talking changes nothing");
    }
}
