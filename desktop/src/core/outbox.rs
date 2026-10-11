//! Our messages on their way to the server, until it takes or refuses each.
//!
//! The server answers sends in order: each `send` gets its `message` echo
//! (with our nonce) or an `error` with `re: "send"`, so an error is about the
//! oldest send it hasn't answered yet (nothing on the wire says which).
//! - `slow_down` (more than 10 at once): the same frame, same nonce, goes
//!   again 1.1 s later, up to 5 more times. Until nothing is waiting,
//!   everything goes one at a time, 1.1 s apart, in the order it was written
//!   (so a later message never overtakes one that was turned away). No toast
//!   meanwhile; only giving up shows the error.
//! - Any other refusal (too long, no channel, the replied-to message is
//!   gone…): the grey copy goes and its text goes back in the message box if
//!   that's empty. The server's words are the toast.
//!
//! Pure bookkeeping (the core does the sending), so it's tested on its own.

use serde_json::Value;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A send turned away with `slow_down` goes again this long after (the
/// server lets one more through a second)…
pub const RETRY_AFTER: Duration = Duration::from_millis(1100);
/// …up to this many times (6 tries in all); then it fails like any refusal.
pub const MAX_RETRIES: u32 = 5;

/// Where a refused message's text goes back: the message box of `channel`,
/// if it's empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Restore {
    pub channel: String,
    pub text: String,
}

#[derive(Debug)]
struct Item {
    nonce: String,
    frame: Value,
    retries: u32,
    /// Not sent (or turned away): goes when it's its turn.
    waiting: bool,
    restore: Option<Restore>,
    /// It has a grey copy on screen: after a reconnect it goes again.
    again_on_reconnect: bool,
}

/// What became of the send an `error {re: "send"}` was about.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// `slow_down`: it goes again in a moment, nothing to show.
    Retrying { nonce: String, retries: u32 },
    /// Given up: drop its grey copy (`nonce`), put its text back, show the error.
    Failed { nonce: String, restore: Option<Restore> },
}

#[derive(Default)]
pub struct Outbox {
    /// Not taken yet, in the order written.
    items: Vec<Item>,
    /// Sent and not answered yet, in the order sent (the order of the answers).
    in_flight: VecDeque<String>,
    /// Since a `slow_down`: one at a time until nothing waits or is unanswered,
    /// the next not before this.
    slow: Option<Instant>,
}

impl Outbox {
    /// A new send. True: send it now. False: it waits its turn (we're going one
    /// at a time after a `slow_down`).
    pub fn add(&mut self, nonce: String, frame: Value, restore: Option<Restore>, again_on_reconnect: bool) -> bool {
        let now = self.slow.is_none();
        if now {
            self.in_flight.push_back(nonce.clone());
        }
        self.items.push(Item { nonce, frame, retries: 0, waiting: !now, restore, again_on_reconnect });
        now
    }

    /// The socket couldn't take it (offline): it waits (for a reconnect, or its turn).
    pub fn unsent(&mut self, nonce: &str) {
        if let Some(i) = self.in_flight.iter().rposition(|n| n == nonce) {
            self.in_flight.remove(i);
        }
        if let Some(item) = self.items.iter_mut().find(|i| i.nonce == nonce) {
            item.waiting = true;
        }
    }

    /// Our echo arrived: it went through. Sends before it were answered too
    /// (by something that didn't say `re: "send"`): no longer awaited.
    pub fn answered(&mut self, nonce: &str) {
        if let Some(i) = self.in_flight.iter().position(|n| n == nonce) {
            self.in_flight.drain(..=i);
        }
        self.items.retain(|i| i.nonce != nonce);
        self.settle();
    }

    /// An `error` with `re: "send"` (`code`): about the oldest unanswered send.
    /// None if we weren't waiting for any.
    pub fn refused(&mut self, code: &str, now: Instant) -> Option<Refused> {
        let nonce = self.in_flight.pop_front()?;
        let index = self.items.iter().position(|i| i.nonce == nonce)?;
        let item = &mut self.items[index];
        if code == "slow_down" && item.retries < MAX_RETRIES {
            item.retries += 1;
            item.waiting = true;
            let at = now + RETRY_AFTER;
            self.slow = Some(self.slow.map_or(at, |t| t.max(at)));
            return Some(Refused::Retrying { nonce, retries: item.retries });
        }
        let item = self.items.remove(index);
        self.settle();
        Some(Refused::Failed { nonce, restore: item.restore })
    }

    /// While going one at a time: the next waiting send, if it's time (now in
    /// flight; the caller sends it).
    pub fn due(&mut self, now: Instant) -> Option<(String, Value)> {
        if self.slow.is_none_or(|t| now < t) {
            return None;
        }
        let item = self.items.iter_mut().find(|i| i.waiting)?;
        item.waiting = false;
        self.in_flight.push_back(item.nonce.clone());
        self.slow = Some(now + RETRY_AFTER);
        Some((item.nonce.clone(), item.frame.clone()))
    }

    /// After a reconnect: everything with a grey copy goes again, in the order
    /// written (all in flight now). Sends without one are forgotten.
    pub fn reconnect(&mut self) -> Vec<(String, Value)> {
        self.in_flight.clear();
        self.slow = None;
        self.items.retain(|i| i.again_on_reconnect);
        for item in &mut self.items {
            item.waiting = false;
            item.retries = 0;
            self.in_flight.push_back(item.nonce.clone());
        }
        self.items.iter().map(|i| (i.nonce.clone(), i.frame.clone())).collect()
    }

    /// Signed out, or another server: nothing's on its way anymore.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Back to sending right away once nothing waits or is unanswered.
    fn settle(&mut self) {
        if self.slow.is_some() && self.in_flight.is_empty() && !self.items.iter().any(|i| i.waiting) {
            self.slow = None;
        }
    }

    #[cfg(test)]
    fn waiting(&self) -> Vec<&str> {
        self.items.iter().filter(|i| i.waiting).map(|i| i.nonce.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn add(o: &mut Outbox, n: &str) -> bool {
        o.add(n.into(), json!({ "t": "send", "nonce": n }), Some(Restore { channel: "c".into(), text: format!("text {n}") }), true)
    }

    #[test]
    fn a_burst_turned_away_goes_out_one_at_a_time_in_order() {
        let t0 = Instant::now();
        let mut o = Outbox::default();
        // Twelve at once: the server takes ten and turns two away.
        for i in 1..=12 {
            assert!(add(&mut o, &format!("m{i}")));
        }
        for i in 1..=10 {
            o.answered(&format!("m{i}"));
        }
        assert_eq!(o.refused("slow_down", t0), Some(Refused::Retrying { nonce: "m11".into(), retries: 1 }));
        assert_eq!(o.refused("slow_down", t0), Some(Refused::Retrying { nonce: "m12".into(), retries: 1 }));
        // Something written meanwhile waits behind them.
        assert!(!add(&mut o, "m13"));
        assert_eq!(o.waiting(), ["m11", "m12", "m13"]);
        // Not before 1.1 s, then one at a time, 1.1 s apart, in the order written.
        assert_eq!(o.due(t0 + Duration::from_millis(1000)), None);
        let t1 = t0 + RETRY_AFTER;
        assert_eq!(o.due(t1).map(|x| x.0), Some("m11".into()));
        assert_eq!(o.due(t1), None);
        o.answered("m11");
        let t2 = t1 + RETRY_AFTER;
        assert_eq!(o.due(t2).map(|x| x.0), Some("m12".into()));
        // Turned away again: it goes before m13 (written earlier).
        assert_eq!(o.refused("slow_down", t2), Some(Refused::Retrying { nonce: "m12".into(), retries: 2 }));
        let t3 = t2 + RETRY_AFTER;
        let (nonce, frame) = o.due(t3).unwrap();
        assert_eq!((nonce.as_str(), frame["nonce"].as_str()), ("m12", Some("m12")), "same nonce, same frame");
        o.answered("m12");
        assert_eq!(o.due(t3 + RETRY_AFTER).map(|x| x.0), Some("m13".into()));
        o.answered("m13");
        // All through: back to sending right away.
        assert!(add(&mut o, "m14"));
        assert!(o.waiting().is_empty());
    }

    #[test]
    fn slow_down_gives_up_after_five_more_tries() {
        let mut t = Instant::now();
        let mut o = Outbox::default();
        add(&mut o, "a");
        for tries in 1..=MAX_RETRIES {
            assert_eq!(o.refused("slow_down", t), Some(Refused::Retrying { nonce: "a".into(), retries: tries }));
            t += RETRY_AFTER;
            assert_eq!(o.due(t).map(|x| x.0), Some("a".into()));
        }
        assert_eq!(o.refused("slow_down", t), Some(Refused::Failed { nonce: "a".into(), restore: Some(Restore { channel: "c".into(), text: "text a".into() }) }));
        assert!(add(&mut o, "b"), "nothing waits anymore");
    }

    #[test]
    fn other_refusals_fail_the_oldest_unanswered_send() {
        let t = Instant::now();
        let mut o = Outbox::default();
        add(&mut o, "a");
        o.add("save".into(), json!({}), None, false); // Save to Personal: no grey copy, no text
        add(&mut o, "b");
        assert!(matches!(o.refused("no_message", t), Some(Refused::Failed { nonce, restore: Some(_) }) if nonce == "a"));
        assert_eq!(o.refused("too_long", t), Some(Refused::Failed { nonce: "save".into(), restore: None }));
        o.answered("b");
        assert_eq!(o.refused("no_channel", t), None, "nothing unanswered");
    }

    #[test]
    fn an_echo_answers_everything_sent_before_it() {
        let t = Instant::now();
        let mut o = Outbox::default();
        add(&mut o, "a");
        add(&mut o, "b");
        // "a" got an answer we couldn't attribute (no `re`); "b"'s echo arrives.
        o.answered("b");
        assert_eq!(o.refused("slow_down", t), None);
    }

    #[test]
    fn offline_sends_and_reconnects() {
        let t = Instant::now();
        let mut o = Outbox::default();
        add(&mut o, "a");
        o.unsent("a");
        o.add("save".into(), json!({}), None, false);
        add(&mut o, "b");
        assert_eq!(o.refused("slow_down", t), Some(Refused::Retrying { nonce: "save".into(), retries: 1 }));
        // Back: everything with a grey copy goes again, in the order written; not going slowly.
        let again: Vec<String> = o.reconnect().into_iter().map(|x| x.0).collect();
        assert_eq!(again, ["a", "b"]);
        assert!(add(&mut o, "c"));
        o.answered("a");
        o.answered("b");
        o.answered("c");
        o.clear();
        assert_eq!(o.refused("slow_down", t), None);
    }
}
