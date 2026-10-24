//! The call report (PROTOCOL.md › Voice › Call report): what the app noticed
//! about one call — how each person was connected, lost and patched-over
//! audio, devices and settings, and a timeline — as plain text to copy when a
//! call sounded off. Nothing goes on the wire; the last three are kept next to
//! the settings file. The words and layout are the web's (web/js/call-report.js).
//!
//! It never holds an IP address, message text or a key: a candidate is kept
//! by its *type* only (host, srflx, prflx, relay), the numbers are copied one
//! by one, and the finished text is scrubbed of anything that looks like an
//! address anyway (a device or channel named after one).
//!
//! Here: the notes and their text (no I/O, unit-tested), then the core's half
//! (`impl Core`): what it tells the notes, and when.

use super::*;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::path::Path;

/// At most this many timeline lines (the oldest go first; the summary never does).
pub const MAX_TIMELINE: usize = 2000;
/// Calls kept on disk.
const KEEP: usize = 3;
/// The core reads every connection this often in a call.
const READ_EVERY: Duration = Duration::from_secs(2);

/// A candidate's type, if it's one of the four (anything else is dropped).
pub fn candidate_kind(t: &str) -> Option<&'static str> {
    ["host", "srflx", "prflx", "relay"].into_iter().find(|k| t.eq_ignore_ascii_case(k))
}

/// "direct (host ↔ srflx)", "relay (relay ↔ host)".
pub fn path_text(local: &str, remote: &str) -> String {
    let via = if local == "relay" || remote == "relay" { "relay" } else { "direct" };
    format!("{via} ({local} ↔ {remote})")
}

/// One 2 s reading of someone's connection: the voice engine's counters and
/// their jitter buffer's. Counters are cumulative for `conn` (a rebuilt
/// connection starts again from zero).
#[derive(Clone, Default, Debug)]
pub struct Reading {
    pub conn: u64,
    pub rtt_ms: Option<u32>,
    /// Candidate types of the pair in use (never addresses).
    pub path: Option<(&'static str, &'static str)>,
    pub send_red: bool,
    /// The Opus fmtp we sent (what we asked to receive).
    pub fmtp: String,
    pub bytes_out: u64,
    pub bytes_in: u64,
    /// Packets that arrived (audio and padding), and the ones the sequence numbers say were sent.
    pub packets: u64,
    pub expected: u64,
    /// Frames played, patched over (PLC), rebuilt from a RED copy, roughed in by Opus FEC.
    pub frames: u64,
    pub concealed: u64,
    pub red: u64,
    pub fec: u64,
    /// Packets that came RED-packed.
    pub red_packets: u64,
    /// Audio the jitter buffer held (ms), and what it aimed at.
    pub jitter_ms: Option<f32>,
    pub jitter_target_ms: Option<f32>,
    /// Our voice packets sent to them, and how many of them they report
    /// losing in all (None until their first report; remote-inbound-rtp).
    pub sent: u64,
    pub their_lost: Option<u64>,
}

#[derive(Default)]
struct Span {
    readings: u32,
    ms: u64,
    rtts: Vec<f32>,
    packets: u64,
    lost: u64,
    frames: u64,
    concealed: u64,
    bytes_in: u64,
    bytes_out: u64,
}

struct Person {
    key: String,
    name: String,
    present: bool,
    state: Option<String>,
    paths: Vec<String>,
    codec: Option<String>,
    fmtp: String,
    out: Vec<f32>,
    inn: Vec<f32>,
    rtt: Vec<f32>,
    jitter: Vec<f32>,
    target: Vec<f32>,
    packets: u64,
    lost: u64,
    frames: u64,
    concealed: u64,
    red: u64,
    fec: u64,
    restarts: u32,
    rebuilds: u32,
    kept: u32,
    weak_ms: u64,
    weak_since: Option<u64>,
    /// What they report about our voice: packets sent since their first
    /// report, how many of those they lost, their round trips.
    yours_sent: u64,
    yours_lost: u64,
    yours_rtt: Vec<f32>,
    last: Option<(Reading, u64)>,
    span: Span,
}

/// What `CallLog::count` counts.
#[derive(Clone, Copy)]
pub enum Count {
    Restarts,
    Rebuilds,
    Kept,
}

/// Everything noticed about one call, from joining it until leaving it.
/// People are kept by their account, so a call kept across a reconnect or a
/// rebuilt connection stays one person. Times are ms since joining.
pub struct CallLog {
    pub channel_id: String,
    channel: String,
    app: String,
    device: String,
    started_local: String,
    start: Instant,
    ended: Option<u64>,
    settings: Vec<(String, String)>,
    devices: Vec<(String, String)>,
    people: Vec<Person>,
    lines: VecDeque<(u64, String)>,
    dropped: usize,
    next_tick: u64,
}

fn two(n: u64) -> String {
    format!("{n:02}")
}

/// "+mm:ss" since joining ("+h:mm:ss" past an hour).
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("+{}:{}:{}", s / 3600, two(s / 60 % 60), two(s % 60)) } else { format!("+{}:{}", two(s / 60), two(s % 60)) }
}

/// "3:07", "1:02:03".
pub fn duration(ms: u64) -> String {
    let s = (ms + 500) / 1000;
    if s >= 3600 { format!("{}:{}:{}", s / 3600, two(s / 60 % 60), two(s % 60)) } else { format!("{}:{}", s / 60, two(s % 60)) }
}

fn pct(part: u64, whole: u64) -> String {
    format!("{:.1} %", if whole > 0 { part as f64 * 100.0 / whole as f64 } else { 0.0 })
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn median(list: &[f32]) -> Option<f32> {
    let mut s: Vec<f32> = list.iter().copied().filter(|v| v.is_finite()).collect();
    if s.is_empty() {
        return None;
    }
    s.sort_by(f32::total_cmp);
    let mid = s.len() / 2;
    Some(if s.len() % 2 == 1 { s[mid] } else { (s[mid - 1] + s[mid]) / 2.0 })
}

fn ms_text(v: Option<f32>) -> String {
    v.map_or("n/a".into(), |v| format!("{} ms", v.round()))
}

fn kbps_text(v: Option<f32>) -> String {
    v.map_or("n/a".into(), |v| format!("{} kbps", v.round()))
}

impl CallLog {
    pub fn new(channel_id: &str, channel: &str, app: String, device: String, started_local: String) -> Self {
        Self {
            channel_id: channel_id.into(),
            channel: channel.into(),
            app,
            device,
            started_local,
            start: Instant::now(),
            ended: None,
            settings: Vec::new(),
            devices: Vec::new(),
            people: Vec::new(),
            lines: VecDeque::new(),
            dropped: 0,
            next_tick: 10_000,
        }
    }

    /// Now, in ms since joining.
    pub fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    pub fn ended(&self) -> bool {
        self.ended.is_some()
    }

    /// A timeline line.
    pub fn note(&mut self, at: u64, text: impl Into<String>) {
        self.lines.push_back((at, text.into()));
        while self.lines.len() > MAX_TIMELINE {
            self.lines.pop_front();
            self.dropped += 1;
        }
    }

    /// Settings as they are now; what changed goes on the timeline (unless `quiet`).
    pub fn set_settings(&mut self, at: u64, next: Vec<(String, String)>, quiet: bool) {
        let changed = changes(&self.settings, &next);
        if !quiet {
            for (k, v) in changed {
                self.note(at, format!("Settings: {} {v}", k.to_lowercase()));
            }
        }
        self.settings = next;
    }

    /// Audio devices as they are now; a change goes on the timeline (unless `quiet`).
    pub fn set_devices(&mut self, at: u64, next: Vec<(String, String)>, quiet: bool) {
        let changed = changes(&self.devices, &next);
        if !quiet {
            for (k, v) in changed {
                self.note(at, format!("{k}: {v}"));
            }
        }
        self.devices = next;
    }

    fn person(&mut self, key: &str, name: &str) -> &mut Person {
        let i = match self.people.iter().position(|p| p.key == key) {
            Some(i) => i,
            None => {
                self.people.push(Person {
                    key: key.into(),
                    name: name.into(),
                    present: true,
                    state: None,
                    paths: Vec::new(),
                    codec: None,
                    fmtp: String::new(),
                    out: Vec::new(),
                    inn: Vec::new(),
                    rtt: Vec::new(),
                    jitter: Vec::new(),
                    target: Vec::new(),
                    packets: 0,
                    lost: 0,
                    frames: 0,
                    concealed: 0,
                    red: 0,
                    fec: 0,
                    restarts: 0,
                    rebuilds: 0,
                    kept: 0,
                    weak_ms: 0,
                    weak_since: None,
                    yours_sent: 0,
                    yours_lost: 0,
                    yours_rtt: Vec::new(),
                    last: None,
                    span: Span::default(),
                });
                self.people.len() - 1
            }
        };
        let p = &mut self.people[i];
        if !name.is_empty() {
            p.name = name.into();
        }
        p
    }

    /// Count an ICE restart, a rebuilt connection or a reconnect the call survived.
    pub fn count(&mut self, key: &str, name: &str, what: Count) {
        let p = self.person(key, name);
        match what {
            Count::Restarts => p.restarts += 1,
            Count::Rebuilds => p.rebuilds += 1,
            Count::Kept => p.kept += 1,
        }
    }

    /// Their connection's state changed (closing is said by whoever leaves).
    pub fn connection(&mut self, at: u64, key: &str, name: &str, state: &str) {
        let p = self.person(key, name);
        if state == "closed" || p.state.as_deref() == Some(state) {
            return;
        }
        p.state = Some(state.into());
        let line = format!("{}: {state}", p.name);
        self.note(at, line);
    }

    /// *Weak connection* went on (`Some((patched share 0–1, rtt))`) or off.
    pub fn weak(&mut self, at: u64, key: &str, name: &str, info: Option<(f32, Option<u32>)>) {
        let p = self.person(key, name);
        let line = match info {
            Some((concealed, rtt)) if p.weak_since.is_none() => {
                p.weak_since = Some(at);
                let rtt = rtt.map(|r| format!(", rtt {r} ms")).unwrap_or_default();
                format!("{}: weak connection ({} % patched over{rtt})", p.name, (concealed * 100.0).round())
            }
            None if p.weak_since.is_some() => {
                p.weak_ms += at.saturating_sub(p.weak_since.take().unwrap_or(at));
                format!("{}: weak connection gone", p.name)
            }
            _ => return,
        };
        self.note(at, line);
    }

    /// They're in the call (again); `text` goes on the timeline.
    pub fn joined(&mut self, at: u64, key: &str, name: &str, text: Option<String>) {
        self.person(key, name).present = true;
        if let Some(t) = text {
            self.note(at, t);
        }
    }

    /// They're out of the call; their numbers stay.
    pub fn left(&mut self, at: u64, key: &str, name: &str, text: Option<String>) {
        let p = self.person(key, name);
        if let Some(since) = p.weak_since.take() {
            p.weak_ms += at.saturating_sub(since);
        }
        p.present = false;
        p.state = None;
        p.last = None;
        if let Some(t) = text {
            self.note(at, t);
        }
    }

    /// One 2 s reading of someone's connection.
    pub fn sample(&mut self, at: u64, key: &str, name: &str, r: Reading) {
        if self.ended.is_some() {
            return;
        }
        let mut notes = Vec::new();
        let p = self.person(key, name);
        let prev = p.last.take().filter(|(l, _)| l.conn == r.conn && r.packets >= l.packets && r.frames >= l.frames);
        let span = &mut p.span;
        span.readings += 1;
        if let Some((b, then)) = &prev {
            let elapsed = at.saturating_sub(*then);
            let packets = r.packets - b.packets;
            let lost = (r.expected.saturating_sub(b.expected)).saturating_sub(packets);
            let frames = r.frames - b.frames;
            let concealed = r.concealed.saturating_sub(b.concealed);
            p.packets += packets;
            p.lost += lost;
            p.frames += frames;
            p.concealed += concealed;
            p.red += r.red.saturating_sub(b.red);
            p.fec += r.fec.saturating_sub(b.fec);
            span.packets += packets;
            span.lost += lost;
            span.frames += frames;
            span.concealed += concealed;
            // What they lose of ours, from their first report on.
            if let (Some(before), Some(now)) = (b.their_lost, r.their_lost) {
                p.yours_sent += r.sent.saturating_sub(b.sent);
                p.yours_lost += now.saturating_sub(before);
                p.yours_rtt.extend(r.rtt_ms.map(|v| v as f32));
            }
            if elapsed > 0 {
                let (din, dout) = (r.bytes_in.saturating_sub(b.bytes_in), r.bytes_out.saturating_sub(b.bytes_out));
                p.inn.push(din as f32 * 8.0 / elapsed as f32);
                p.out.push(dout as f32 * 8.0 / elapsed as f32);
                span.bytes_in += din;
                span.bytes_out += dout;
                span.ms += elapsed;
            }
        }
        if let Some(rtt) = r.rtt_ms {
            p.rtt.push(rtt as f32);
            span.rtts.push(rtt as f32);
        }
        if let Some(j) = r.jitter_ms.filter(|_| r.packets > 0) {
            p.jitter.push(j);
            p.target.extend(r.jitter_target_ms);
        }
        if let Some((l, rm)) = r.path {
            let text = path_text(l, rm);
            if p.paths.last() != Some(&text) {
                notes.push(format!("{}: {text}", p.name));
                p.paths.push(text);
            }
        }
        if r.packets > 0 || r.bytes_out > 0 {
            let codec = |red| if red { "red+opus" } else { "opus" };
            let text = format!("{} out, {} in", codec(r.send_red), codec(r.red_packets > 0));
            if p.codec.as_ref() != Some(&text) {
                notes.push(format!("{}: {text}", p.name));
                p.codec = Some(text);
            }
        }
        if !r.fmtp.is_empty() {
            p.fmtp = r.fmtp.clone();
        }
        p.last = Some((r, at));
        for n in notes {
            self.note(at, n);
        }
    }

    /// After each round of readings: a line per person every 10 s for the
    /// first minute, then every minute. True when it wrote them.
    pub fn tick(&mut self, at: u64) -> bool {
        if self.ended.is_some() || at < self.next_tick {
            return false;
        }
        while self.next_tick <= at {
            self.next_tick += if self.next_tick < 60_000 { 10_000 } else { 60_000 };
        }
        let mut notes = Vec::new();
        for p in &mut self.people {
            let s = std::mem::take(&mut p.span);
            if !p.present || s.readings == 0 {
                continue;
            }
            let rate = |bytes: u64| if s.ms > 0 { (bytes as f64 * 8.0 / s.ms as f64).round().to_string() } else { "n/a".into() };
            notes.push(format!(
                "{}: rtt {}, lost {}, patched {}, {} kbps in, {} out",
                p.name,
                ms_text(median(&s.rtts)),
                pct(s.lost, s.lost + s.packets),
                pct(s.concealed, s.frames),
                rate(s.bytes_in),
                rate(s.bytes_out)
            ));
        }
        for n in notes {
            self.note(at, n);
        }
        true
    }

    /// The call is over; `reason` says why ("you left").
    pub fn end(&mut self, at: u64, reason: &str) {
        if self.ended.is_some() {
            return;
        }
        for p in &mut self.people {
            if let Some(since) = p.weak_since.take() {
                p.weak_ms += at.saturating_sub(since);
            }
        }
        self.ended = Some(at);
        self.note(at, format!("Call ended: {reason}"));
    }

    /// The report as plain text (PROTOCOL.md › Voice › Call report, in that order).
    pub fn format(&self, now: u64) -> String {
        let end = self.ended.unwrap_or(now);
        let mut o = String::new();
        let _ = writeln!(o, "Yap call report\nApp: {}\nDevice: {}\nChannel: {}", self.app, self.device, self.channel);
        let _ = writeln!(o, "Started: {} (local time)", self.started_local);
        let _ = writeln!(o, "Lasted: {}{}", duration(end), if self.ended.is_some() { "" } else { " so far (still in the call)" });
        o.push_str("Settings:\n");
        for (k, v) in &self.settings {
            let _ = writeln!(o, "  {k}: {v}");
        }
        o.push_str("Audio devices:\n");
        for (k, v) in &self.devices {
            let _ = writeln!(o, "  {k}: {v}");
        }
        let _ = writeln!(o, "\nPeople ({})", self.people.len());
        if self.people.is_empty() {
            o.push_str("  Nobody else was in the call.\n");
        }
        for p in &self.people {
            let weak = p.weak_ms + p.weak_since.map_or(0, |s| end.saturating_sub(s));
            let worst = p.rtt.iter().copied().reduce(f32::max);
            let _ = writeln!(o, "{}", p.name);
            let _ = writeln!(o, "  Connection: {}", if p.paths.is_empty() { "never connected".into() } else { p.paths.join(", then ") });
            let _ = writeln!(o, "  Codec: {}", p.codec.as_deref().unwrap_or("not negotiated"));
            let _ = writeln!(o, "  Opus fmtp sent: {}", if p.fmtp.is_empty() { "n/a" } else { &p.fmtp });
            let _ = writeln!(o, "  Bitrate (median): {} out, {} in", kbps_text(median(&p.out)), kbps_text(median(&p.inn)));
            let _ = writeln!(o, "  Round trip: {} median, {} worst", ms_text(median(&p.rtt)), ms_text(worst));
            let _ = writeln!(o, "  Lost packets: {} ({} of {})", pct(p.lost, p.lost + p.packets), thousands(p.lost), thousands(p.lost + p.packets));
            let _ = writeln!(o, "  Audio patched over: {}", pct(p.concealed, p.frames));
            let _ = writeln!(o, "  Rebuilt from a redundant copy: {} packets (Opus FEC roughed in {} more)", thousands(p.red), thousands(p.fec));
            let target = median(&p.target).map(|t| format!(" (aiming at {})", ms_text(Some(t)))).unwrap_or_default();
            let _ = writeln!(o, "  Jitter buffer (median): {}{target}", ms_text(median(&p.jitter)));
            let _ = writeln!(o, "  ICE restarts: {}, connection rebuilds: {}, reconnects kept: {}", p.restarts, p.rebuilds, p.kept);
            let _ = writeln!(o, "  Weak connection shown: {}", if weak > 0 { duration(weak) } else { "never".into() });
            let yours = if p.yours_sent > 0 {
                format!("they lost {} of yours (their round trip {} median)", pct(p.yours_lost, p.yours_sent), ms_text(median(&p.yours_rtt)))
            } else {
                "no reports from them".into()
            };
            let _ = writeln!(o, "  From you: {yours}");
        }
        o.push_str("\nTimeline\n");
        if self.dropped > 0 {
            let _ = writeln!(o, "({} earlier lines left out: the timeline keeps the last {})", thousands(self.dropped as u64), thousands(MAX_TIMELINE as u64));
        }
        for (at, text) in &self.lines {
            let _ = writeln!(o, "{} {text}", clock(*at));
        }
        scrub(&o)
    }
}

/// What changed between two lists of (label, value).
fn changes(before: &[(String, String)], next: &[(String, String)]) -> Vec<(String, String)> {
    next.iter().filter(|(k, v)| before.iter().any(|(bk, bv)| bk == k && bv != v)).cloned().collect()
}

/// Anything that looks like an IP address becomes "[address]": IPv4
/// dotted quads (with a port or not), and IPv6 (with "::", or six or more
/// groups of hex; a zone like "%en0" goes too).
pub fn scrub(text: &str) -> String {
    let is_v4 = |t: &str| {
        let parts: Vec<&str> = t.split('.').collect();
        parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok_and(|n| n <= 255))
    };
    let is_v6 = |t: &str| {
        let t = t.split('%').next().unwrap_or(t);
        let groups: Vec<&str> = t.split(':').collect();
        (t.contains("::") || groups.len() >= 6)
            && groups.len() >= 3
            && groups.iter().all(|g| (g.len() <= 4 && g.bytes().all(|b| b.is_ascii_hexdigit())) || is_v4_tail(g))
            && groups.iter().any(|g| !g.is_empty())
    };
    let hit = |t: &str| is_v4(t.trim_end_matches(':')) || is_v6(t) || t.rsplit_once(':').is_some_and(|(a, port)| is_v4(a) && port.bytes().all(|b| b.is_ascii_digit()));
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        let t = token.trim_end_matches('.');
        if !t.is_empty() && hit(t) {
            out.push_str("[address]");
            out.push_str(&token[t.len()..]);
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        let zone = token.contains('%');
        if c.is_ascii_hexdigit() || matches!(c, '.' | ':' | '%') || (zone && c.is_ascii_alphanumeric()) {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// The last group of an IPv4-mapped IPv6 address ("::ffff:192.0.2.1").
fn is_v4_tail(g: &str) -> bool {
    g.split('.').count() == 4 && g.split('.').all(|p| p.parse::<u8>().is_ok())
}

/// The notes on the call we're in (or the last one), and the last three
/// reports, kept next to the settings file.
pub struct Calls {
    pub current: Option<CallLog>,
    /// Finished (or saved-while-running) reports, newest first.
    saved: Vec<String>,
    /// `saved[0]` is the current call's (a newer save replaces it).
    current_saved: bool,
    path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Default)]
struct Stored {
    reports: Vec<String>,
}

/// `calls.json` next to `config.json` (`<name>.calls.json` next to any other settings file).
pub fn reports_path(config: &Path) -> PathBuf {
    let name = match config.file_stem().and_then(|s| s.to_str()) {
        Some("config") | None => "calls.json".to_string(),
        Some(stem) => format!("{stem}.calls.json"),
    };
    config.with_file_name(name)
}

impl Calls {
    /// The reports kept at `path` (None: nothing on disk, e.g. in tests).
    pub fn load(path: Option<PathBuf>) -> Self {
        let saved = path.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<Stored>(&b).ok()).map(|s| s.reports).unwrap_or_default();
        Self { current: None, saved: saved.into_iter().take(KEEP).collect(), current_saved: false, path }
    }

    pub fn in_call(&self) -> bool {
        self.current.as_ref().is_some_and(|c| !c.ended())
    }

    /// A timeline line, if we're in a call.
    pub fn note(&mut self, text: impl Into<String>) {
        if let Some(c) = self.current.as_mut().filter(|c| !c.ended()) {
            let at = c.now();
            c.note(at, text);
        }
    }

    /// A new call starts (the one before, if any, was ended and saved).
    pub fn start(&mut self, log: CallLog) {
        self.current = Some(log);
        self.current_saved = false;
    }

    /// The report to copy: the call we're in or just left, else the last one kept.
    pub fn text(&self) -> Option<String> {
        self.current.as_ref().map(|c| c.format(c.now())).or_else(|| self.saved.first().cloned())
    }

    /// Is there anything to copy?
    pub fn any(&self) -> bool {
        self.current.is_some() || !self.saved.is_empty()
    }

    /// Keep the current call's report (replacing its earlier save) and write the last three.
    pub fn save(&mut self) {
        let Some(text) = self.current.as_ref().map(|c| c.format(c.now())) else { return };
        if self.current_saved && !self.saved.is_empty() {
            self.saved[0] = text;
        } else {
            self.saved.insert(0, text);
            self.saved.truncate(KEEP);
            self.current_saved = true;
        }
        let Some(path) = &self.path else { return };
        let body = serde_json::to_vec(&Stored { reports: self.saved.clone() }).unwrap_or_default();
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    #[cfg(test)]
    fn saved(&self) -> &[String] {
        &self.saved
    }
}

// ── The core's half: what it tells the notes ──────────────────────────────

/// "Ana, Ben and Cy".
fn names(list: &[String]) -> String {
    match list {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn on_off(on: bool) -> String {
    if on { "on" } else { "off" }.into()
}

/// This app and build, and the computer it runs on.
fn app_and_device() -> (String, String) {
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    let app = format!("Yap desktop {} ({build} build, {}-{})", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
    #[cfg(target_os = "macos")]
    let os = format!("macOS {}", objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersionString().to_string().trim_start_matches("Version "));
    #[cfg(target_os = "linux")]
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
        .unwrap_or_else(|| "Linux".into());
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let os = "Windows".to_string();
    (app, format!("{os}, {} computer", std::env::consts::ARCH))
}

/// Echo cancellation as the report says it.
fn echo_cancellation(c: &Config) -> String {
    if c.echo_cancellation { "on (AEC3)".into() } else { "off".into() }
}

impl Core {
    /// Who a session is, for the report: their name, keyed by account (an OBS link apart from its owner).
    fn who(&self, sid: &str) -> (String, String) {
        let m = self.shared.model.lock();
        let s = m.sessions.get(sid);
        let uid = s.map(|s| s.uid.clone()).filter(|u| !u.is_empty()).unwrap_or_else(|| sid.to_string());
        let name = m.users.get(&uid).map(|u| u.name.clone()).unwrap_or_else(|| "Someone".into());
        if s.is_some_and(|s| s.client == "obs") { (format!("{uid}:obs"), format!("{name}'s OBS")) } else { (uid, name) }
    }

    /// Your settings in words, as they are now.
    fn describe_settings(&self) -> Vec<(String, String)> {
        let m = self.shared.model.lock();
        let c = &m.config;
        let kbps = |b: u32| format!("{} kbps", b / 1000);
        let shared = m.voice_wanted.as_deref().and_then(|id| m.channel(id)).and_then(|ch| ch.bitrate);
        let mode = match c.mode {
            InputMode::Vad => format!("voice activity, sensitivity {:.0} dBFS", c.vad_threshold),
            InputMode::Ptt => "push to talk".into(),
            InputMode::Open => "always on".into(),
        };
        let relay = if c.relay_only && m.relay == Some(false) { "on (but this server has no voice relay: no calls)".into() } else { on_off(c.relay_only) };
        [
            ("Quality", format!("{}{}", kbps(c.bitrate), shared.map(|b| format!(" (this channel: {} for everyone)", kbps(b))).unwrap_or_default())),
            ("Input mode", mode),
            ("Noise suppression", if c.noise_suppression { "on (RNNoise)".into() } else { "off".into() }),
            ("Echo cancellation", echo_cancellation(c)),
            ("Automatic gain", on_off(c.auto_gain)),
            ("Music mode", "off (not in this app)".into()),
            ("Hide my IP", relay),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }

    /// Your mic and speakers, as the system names them.
    fn describe_devices(&self) -> Vec<(String, String)> {
        let health = self.shared.audio.health();
        let (picked_in, picked_out, mic_ok) = {
            let m = self.shared.model.lock();
            (m.config.input_device.clone(), m.config.output_device.clone(), m.mic_ok)
        };
        let test = self.shared.audio_override.lock().is_some();
        let name = |opened: Option<String>, picked: &str| match opened {
            _ if test => "none (a test: a file, not a device)".to_string(),
            Some(n) if picked.is_empty() => format!("system default ({n})"),
            Some(n) if n != picked => format!("{n} (the picked one is gone)"),
            Some(n) => n,
            None => "not open".into(),
        };
        vec![
            ("Microphone".into(), if mic_ok { name(health.opened_input, &picked_in) } else { "none (listening only)".into() }),
            ("Output".into(), name(health.opened_output, &picked_out)),
        ]
    }

    /// A timeline line, if we're in a call.
    pub(super) fn call_note(&self, text: impl Into<String>) {
        self.shared.calls.lock().note(text);
    }

    /// Our own join came back: a new call, or the same one going on after our
    /// socket dropped (`handover`: the call was kept, else rejoined as a newcomer).
    pub(super) fn call_joined(&mut self, channel_id: &str, others: &[String], handover: bool) {
        let channel = self.shared.model.lock().channel(channel_id).map(|c| c.name.clone()).unwrap_or_else(|| "a voice channel".into());
        let there: Vec<(String, String)> = others.iter().map(|s| self.who(s)).collect();
        let list: Vec<String> = there.iter().map(|(_, n)| n.clone()).collect();
        let with = if list.is_empty() { "nobody else was there".to_string() } else { format!("{} {} there", names(&list), if list.len() == 1 { "was" } else { "were" }) };
        let kept: Vec<bool> = others.iter().map(|s| self.voice.has_peer(s)).collect();
        let (settings, devices) = (self.describe_settings(), self.describe_devices());
        let relay_only = self.shared.model.lock().config.relay_only;
        let mut calls = self.shared.calls.lock();
        if let Some(log) = calls.current.as_mut().filter(|c| !c.ended() && c.channel_id == channel_id) {
            let at = log.now();
            log.note(at, if handover { "Back on Yap: the call was kept".to_string() } else { format!("Rejoined {channel} as a newcomer; {with}") });
            for ((key, name), kept) in there.iter().zip(kept) {
                if handover && kept {
                    log.count(key, name, Count::Kept);
                }
                log.joined(at, key, name, None);
            }
            return;
        }
        if let Some(log) = calls.current.as_mut().filter(|c| !c.ended()) {
            let at = log.now();
            log.end(at, &format!("moved to {channel}"));
            calls.save();
        }
        let (app, device) = app_and_device();
        let mut log = CallLog::new(channel_id, &channel, app, device, chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
        log.set_settings(0, settings, true);
        log.set_devices(0, devices, true);
        log.note(0, format!("You joined {channel}; {with}"));
        if relay_only && self.voice.has_relay() {
            log.note(0, "Hiding your IP: everything goes through the voice relay");
        } else if relay_only {
            // (A call doesn't start like this any more: Hide my IP never connects directly.)
            log.note(0, "Hide my IP is on, but there's no voice relay: connecting to no one");
        }
        for (key, name) in &there {
            log.joined(0, key, name, None);
        }
        calls.start(log);
        calls.save();
    }

    /// Someone came into our call (`back`: under a new session, the call kept), lost their socket (`away`), or left.
    pub(super) fn call_person(&self, sid: &str, what: &str, how: &str) {
        let (key, name) = self.who(sid);
        let mut calls = self.shared.calls.lock();
        let Some(log) = calls.current.as_mut().filter(|c| !c.ended()) else { return };
        let at = log.now();
        match what {
            "joined" => log.joined(at, &key, &name, Some(format!("{name} joined"))),
            "back" => {
                log.count(&key, &name, Count::Kept);
                log.joined(at, &key, &name, Some(format!("{name} is back: the call was kept")));
            }
            "away" => log.note(at, format!("{name} lost their connection to Yap; keeping the call (reconnecting…)")),
            _ => log.left(at, &key, &name, Some(if how.is_empty() { format!("{name} left") } else { format!("{name} left ({how})") })),
        }
    }

    /// Something about one connection: its state, an ICE restart, a rebuild.
    pub(super) fn call_peer(&self, sid: &str, state: Option<&str>, count: Option<(Count, String)>) {
        let (key, name) = self.who(sid);
        let mut calls = self.shared.calls.lock();
        let Some(log) = calls.current.as_mut().filter(|c| !c.ended()) else { return };
        let at = log.now();
        if let Some(s) = state {
            log.connection(at, &key, &name, s);
        }
        if let Some((what, text)) = count {
            log.count(&key, &name, what);
            log.note(at, format!("{name}: {text}"));
        }
    }

    /// *Weak connection* went on or off for someone.
    pub(super) fn call_weak(&self, sid: &str, info: Option<(f32, Option<u32>)>) {
        let (key, name) = self.who(sid);
        let mut calls = self.shared.calls.lock();
        if let Some(log) = calls.current.as_mut().filter(|c| !c.ended()) {
            let at = log.now();
            log.weak(at, &key, &name, info);
        }
    }

    /// The call is over: why, in a few words ("you left"). The report is kept.
    pub(super) fn call_ended(&self, reason: &str) {
        let mut calls = self.shared.calls.lock();
        let Some(log) = calls.current.as_mut().filter(|c| !c.ended()) else { return };
        let at = log.now();
        log.end(at, reason);
        calls.save();
    }

    /// Every tick: in a call, read every connection every 2 s (and follow settings and devices).
    pub(super) fn tick_report(&mut self) {
        if !self.shared.calls.lock().in_call() || self.last_report.is_some_and(|t| t.elapsed() < READ_EVERY) {
            return;
        }
        self.last_report = Some(Instant::now());
        let (settings, devices) = (self.describe_settings(), self.describe_devices());
        let mut readings = Vec::new();
        for p in self.voice.peers() {
            let sid = p.sid();
            let st = self.shared.audio.stream(&sid).map(|s| s.stats()).unwrap_or_default();
            let (key, name) = self.who(&sid);
            let reading = Reading {
                conn: p.id,
                rtt_ms: Some(p.rtt_ms.load(Ordering::Relaxed)).filter(|r| *r > 0),
                path: *p.path.lock(),
                send_red: p.send_red.load(Ordering::Relaxed),
                fmtp: p.local_fmtp.lock().clone(),
                bytes_out: p.bytes_sent.load(Ordering::Relaxed),
                bytes_in: st.bytes,
                packets: st.packets + st.padding,
                expected: st.expected,
                frames: st.frames,
                concealed: st.concealed,
                red: st.red,
                fec: st.fec,
                red_packets: st.red_packets,
                jitter_ms: st.playing.then_some(st.delay_ms),
                jitter_target_ms: st.playing.then_some(st.target_ms),
                sent: p.packets_sent.load(Ordering::Relaxed),
                their_lost: (p.their_reports.load(Ordering::Relaxed) > 0).then(|| p.their_lost.load(Ordering::Relaxed)),
            };
            readings.push((key, name, reading));
        }
        let mut calls = self.shared.calls.lock();
        let Some(log) = calls.current.as_mut() else { return };
        let at = log.now();
        log.set_settings(at, settings, false);
        log.set_devices(at, devices, false);
        for (key, name, r) in readings {
            log.sample(at, &key, &name, r);
        }
        if log.tick(at) {
            calls.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> CallLog {
        let mut l = CallLog::new("lounge", "Lounge", "Yap desktop 0.0.0 (test)".into(), "a test computer".into(), "2026-10-11 21:00:00".into());
        l.set_settings(0, vec![("Quality".into(), "128 kbps".into()), ("Input mode".into(), "always on".into())], true);
        l.set_devices(0, vec![("Microphone".into(), "Studio Mic".into()), ("Output".into(), "system default (Speakers)".into())], true);
        l
    }

    fn reading(conn: u64, secs: u64, lossy: bool) -> Reading {
        let packets = secs * 50;
        Reading {
            conn,
            rtt_ms: Some(30 + (secs % 3) as u32),
            path: Some(("host", "srflx")),
            send_red: true,
            fmtp: "minptime=10;useinbandfec=1".into(),
            bytes_out: secs * 32_000,
            bytes_in: secs * 33_000,
            packets: if lossy { packets - packets / 50 } else { packets },
            expected: packets,
            frames: packets,
            concealed: if lossy { packets / 200 } else { 0 },
            red: if lossy { packets / 60 } else { 0 },
            fec: 0,
            red_packets: packets,
            jitter_ms: Some(45.0),
            jitter_target_ms: Some(40.0),
            // They report from 4 s on, losing 1 in 25 of ours once it's lossy.
            sent: packets,
            their_lost: (secs >= 4).then_some(if lossy { packets / 25 } else { 0 }),
        }
    }

    #[test]
    fn clock_and_duration() {
        assert_eq!(clock(0), "+00:00");
        assert_eq!(clock(75_400), "+01:15");
        assert_eq!(clock(3_723_000), "+1:02:03");
        assert_eq!(duration(187_000), "3:07");
        assert_eq!(duration(3_723_000), "1:02:03");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(12), "12");
        assert_eq!(names(&["Ana".into(), "Ben".into(), "Cy".into()]), "Ana, Ben and Cy");
    }

    /// A whole call: the header, a person block, the timeline with its 10 s lines.
    #[test]
    fn a_call_reads_like_the_web() {
        let mut l = log();
        l.note(0, "You joined Lounge; Ana was there");
        l.joined(0, "u-ana", "Ana", None);
        l.connection(500, "u-ana", "Ana", "connecting");
        l.connection(900, "u-ana", "Ana", "connected");
        for s in 1..=35u64 {
            let at = s * 2000;
            match at {
                40_000 => l.weak(at, "u-ana", "Ana", Some((0.05, Some(320)))),
                46_000 => l.weak(at, "u-ana", "Ana", None),
                50_000 => l.set_settings(at, vec![("Quality".into(), "256 kbps".into()), ("Input mode".into(), "always on".into())], false),
                _ => {}
            }
            l.sample(at, "u-ana", "Ana", reading(1, s * 2, s > 20));
            l.tick(at);
        }
        l.count("u-ana", "Ana", Count::Restarts);
        l.end(70_000, "you left");
        let text = l.format(80_000);
        println!("{text}");
        for want in [
            "Yap call report\nApp: Yap desktop 0.0.0 (test)\nDevice: a test computer\nChannel: Lounge\n",
            "Started: 2026-10-11 21:00:00 (local time)\nLasted: 1:10\n",
            "Settings:\n  Quality: 256 kbps\n  Input mode: always on\nAudio devices:\n  Microphone: Studio Mic\n",
            "People (1)\nAna\n  Connection: direct (host ↔ srflx)\n  Codec: red+opus out, red+opus in\n  Opus fmtp sent: minptime=10;useinbandfec=1\n",
            "  Bitrate (median): 256 kbps out, 264 kbps in\n",
            "  Round trip: 31 ms median, 32 ms worst\n",
            "  Jitter buffer (median): 45 ms (aiming at 40 ms)\n  ICE restarts: 1, connection rebuilds: 0, reconnects kept: 0\n  Weak connection shown: 0:06\n",
            "Timeline\n+00:00 You joined Lounge; Ana was there\n+00:00 Ana: connecting\n+00:00 Ana: connected\n+00:02 Ana: direct (host ↔ srflx)\n+00:02 Ana: red+opus out, red+opus in\n",
            "+00:10 Ana: rtt 31 ms, lost 0.0 %, patched 0.0 %, 264 kbps in, 256 out\n",
            "+00:40 Ana: weak connection (5 % patched over, rtt 320 ms)\n+00:40 Ana: rtt",
            "+00:46 Ana: weak connection gone\n+00:50 Settings: quality 256 kbps\n",
            "+01:10 Call ended: you left\n",
        ] {
            assert!(text.contains(want), "missing {want:?}");
        }
        // After the first minute, a line a minute.
        let ticks = text.lines().filter(|l| l.contains("Ana: rtt")).count();
        assert_eq!(ticks, 6, "10, 20 … 60 s");
        assert!(text.contains("Lost packets: 2.1 % (70 of 3,400)"), "lossy from 40 s on");
        // What they lost of ours (remote-inbound-rtp): from their first report (4 s) on,
        // 140 of the 3,300 sent since.
        assert!(text.contains("  Weak connection shown: 0:06\n  From you: they lost 4.2 % of yours (their round trip 31 ms median)\n"), "{text}");
        let mut quiet = log();
        quiet.sample(2000, "u-ben", "Ben", Reading { their_lost: None, ..reading(1, 2, false) });
        quiet.sample(4000, "u-ben", "Ben", Reading { their_lost: None, ..reading(1, 4, false) });
        assert!(quiet.format(5000).contains("  From you: no reports from them\n"));
    }

    /// No IP addresses, ever: candidate data with addresses goes in, only types come out;
    /// and an address anywhere else (a device or channel named after one) is scrubbed.
    #[test]
    fn never_an_ip_address() {
        // What webrtc-rs hands over for a candidate: address, port, type (and related address).
        struct Candidate {
            address: &'static str,
            related: &'static str,
            kind: &'static str,
        }
        let local = Candidate { address: "192.168.1.23", related: "", kind: "host" };
        let remote = Candidate { address: "2001:db8:85a3::8a2e:370:7334", related: "203.0.113.7", kind: "srflx" };
        let odd = Candidate { address: "198.51.100.9", related: "", kind: "something-new" };
        let path = (candidate_kind(local.kind).unwrap(), candidate_kind(remote.kind).unwrap());
        assert_eq!(candidate_kind(odd.kind), None, "unknown types are dropped, not passed on");
        let mut l = log();
        l.set_devices(0, vec![("Microphone".into(), "USB mic at 10.0.0.5".into()), ("Output".into(), "fe80::1%en0 speakers".into())], true);
        l.sample(2000, "u-ana", "Ana", Reading { path: Some(path), ..reading(1, 2, false) });
        l.note(3000, format!("Ana: something odd from {}:{} via {}", remote.related, 3478, odd.address));
        l.note(3000, "::ffff:192.0.2.1 and 2001:0db8:0000:0000:0000:ff00:0042:8329 and fd00::");
        let text = l.format(4000);
        for addr in ["192.168.1.23", "2001:db8", "203.0.113.7", "198.51.100.9", "10.0.0.5", "fe80::1", "192.0.2.1", "ff00:0042", "fd00::", local.address, remote.address] {
            assert!(!text.contains(addr), "{addr} leaked:\n{text}");
        }
        assert!(text.contains("Connection: direct (host ↔ srflx)"));
        assert!(text.contains("USB mic at [address]"));
        assert!(text.contains("[address] speakers"));
        // Times, versions and fmtp lines are left alone.
        let fine = "+1:02:03 Yap desktop 0.12.0 minptime=10;useinbandfec=1 111/111 3:07 rtt 38 ms";
        assert_eq!(scrub(fine), fine);
    }

    #[test]
    fn the_timeline_keeps_the_last_2000_lines() {
        let mut l = log();
        for i in 0..2500u64 {
            l.note(i, format!("line {i}"));
        }
        let text = l.format(3000);
        assert!(text.contains("(500 earlier lines left out: the timeline keeps the last 2,000)"));
        assert!(!text.contains("line 499\n"));
        assert!(text.contains("line 500\n") && text.contains("line 2499\n"));
        assert!(text.contains("Lasted: 0:03 so far (still in the call)"));
    }

    /// A rebuilt connection starts its counters again: nothing negative, nothing counted twice.
    #[test]
    fn a_new_connection_starts_counting_again() {
        let mut l = log();
        l.sample(2000, "u-ana", "Ana", reading(1, 10, false));
        l.sample(4000, "u-ana", "Ana", reading(1, 12, false));
        l.sample(6000, "u-ana", "Ana", reading(2, 2, false));
        l.sample(8000, "u-ana", "Ana", reading(2, 4, false));
        let p = &l.people[0];
        assert_eq!(p.packets, 200, "100 from each connection's 2 s");
        assert_eq!(p.lost, 0);
    }

    /// The last three are kept on disk (a temporary folder, never the real settings).
    #[test]
    fn keeps_the_last_three() {
        let dir = std::env::temp_dir().join(format!("yap-report-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = reports_path(&dir.join("config.json"));
        assert_eq!(path.file_name().unwrap(), "calls.json");
        assert_eq!(reports_path(&dir.join("yap-headless-x.json")).file_name().unwrap(), "yap-headless-x.calls.json");
        let mut calls = Calls::load(Some(path.clone()));
        assert!(!calls.any());
        for n in 0..4 {
            let mut l = log();
            l.note(0, format!("call {n}"));
            calls.start(l);
            calls.save(); // while it runs…
            calls.note("still going");
            let c = calls.current.as_mut().unwrap();
            c.end(1000, "you left");
            calls.save(); // …and when it ends: one entry per call
        }
        let again = Calls::load(Some(path.clone()));
        assert_eq!(again.saved().len(), 3);
        assert!(again.saved()[0].contains("call 3") && again.saved()[0].contains("Call ended: you left"));
        assert!(again.saved()[2].contains("call 1"));
        assert!(again.text().unwrap().contains("call 3"), "Copy last call report");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
