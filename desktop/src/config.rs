//! Per-device settings, persisted as JSON in the OS config dir
//! (`~/.config/yap/config.json`, `%APPDATA%\Yap\config.json`,
//! `~/Library/Application Support/Yap/config.json`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const COLORS: [&str; 10] = [
    "#5b6cff", "#e5484d", "#f76b15", "#f5a524", "#30a46c", "#12a594", "#0090ff", "#8e4ec6", "#d6409f",
    "#8d8d8d",
];

pub const BITRATES: [u32; 5] = [64_000, 96_000, 128_000, 256_000, 510_000];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum InputMode {
    /// Voice activity: transmit while the mic is above the threshold.
    #[default]
    Vad,
    /// Push to talk.
    Ptt,
    /// Always on.
    Open,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    System,
    Light,
    /// "Charcoal", the default (like the web client).
    #[default]
    Dark,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Server origin, e.g. `https://yap.example.workers.dev` (no trailing slash).
    pub server: String,
    /// Invite key (may be empty for an open dev server).
    pub key: String,
    /// Random id generated once; it is who you are.
    pub uid: String,
    pub name: String,
    pub color: String,

    /// Audio device names ("" = system default).
    pub input_device: String,
    pub output_device: String,
    pub mode: InputMode,
    /// Voice-activity threshold in dBFS.
    pub vad_threshold: f32,
    /// Push-to-talk key, as a KeyboardEvent.code-style name ("Backquote", "KeyV", "Mouse4"…),
    /// maybe with modifiers in front ("Ctrl+Shift+KeyM"; `ptt::Combo`).
    pub ptt_key: String,
    /// Settings › Voice: global keys next to push-to-talk, the same kind of
    /// name ("" = none, the default): toggle mute, toggle deafen, and
    /// push-to-mute (held: muted, whatever the mode). They work in a call.
    pub mute_key: String,
    pub deafen_key: String,
    pub push_to_mute_key: String,
    /// Opus bitrate in bits/s.
    pub bitrate: u32,
    /// Settings › Voice › Noise suppression: WebRTC's own, what browsers use (fans and hum).
    pub noise_suppression: bool,
    /// Settings › Voice › Advanced › Stronger noise removal: RNNoise instead (typing and clicks too).
    pub stronger_noise: bool,
    /// Settings › Voice › Automatic gain: speech towards −23 dBFS, at most +20 dB.
    pub auto_gain: bool,
    /// Settings › Voice › Mic volume: how loud your mic goes out (1.0 = 100 %, 0–2),
    /// after the automatic gain and before the gate.
    pub mic_volume: f32,
    /// Settings › Voice › Echo cancellation: what the speakers play is taken
    /// back out of the mic (AEC3), so friends don't hear themselves.
    pub echo_cancellation: bool,
    pub sounds: bool,
    /// Settings › Voice › "Remind me when I talk while muted": "You're muted"
    /// by the Mute button, at most every 5 s, and a soft blip the first time since muting.
    pub remind_muted: bool,
    /// That blip was explained once on this device.
    pub muted_explained: bool,
    /// Settings › Voice › "Push-to-talk click": a quiet click, for your ears
    /// only, when push-to-talk opens and when it closes.
    pub ptt_click: bool,
    /// Desktop notifications for @mentions, pings and join requests.
    pub notifications: bool,
    /// …and for every message (only when `notifications` is on).
    pub notify_all: bool,
    pub muted: bool,
    pub deafened: bool,
    /// uid -> volume (1.0 = 100%, up to 2.0).
    pub volumes: HashMap<String, f32>,
    /// uid -> how loud their screen share's sound plays (0–1), apart from their voice.
    pub screen_volumes: HashMap<String, f32>,
    /// Settings › Voice › "Lower shared media while friends talk" (off): a
    /// screen share's sound dips to half while someone in the call speaks.
    pub lower_media: bool,
    /// The last screen-share choices (the share dialog starts from them).
    pub share_quality: crate::screen::Quality,
    pub share_sound: bool,

    /// Last open text channel.
    pub channel: String,
    /// channelId -> newest ts seen.
    pub last_read: HashMap<String, i64>,
    pub theme: ThemePref,
    /// 0.6.0's "Text size" (1.0 = 100%); moved into `size` on load, kept at
    /// 1.0 so an older version still reads a sane value.
    pub zoom: f32,
    /// Settings › Look › Size: None = Auto (bigger on a big, low-scale
    /// monitor), else 0.9–1.75 on top of the OS scale.
    pub size: Option<f32>,
    /// Settings › Voice › Soundboard: how loud clips and chimes play (0–1,
    /// default 0.5; never above a clip's measured gain) / no soundboard at all.
    pub sound_volume: f32,
    pub sound_mute: bool,
    /// Saving to an encrypted Personal space was explained once.
    pub saved_note_seen: bool,
    /// Settings › Voice › "Hide my IP address in calls": voice only through
    /// the server's TURN relay (when it has one), like the web's `relayOnly`.
    pub relay_only: bool,
    /// "Choose a password" prompts Later put off on this device, for good, by
    /// `ready.weakReason` ("pin", "temporary", "guessable"), like the web's
    /// `passwordLater`: putting off one doesn't put off the others.
    pub password_later: Vec<String>,
    /// The call you were in, kept up to date while you're in it (and cleared
    /// when you leave it yourself): the next launch, within 10 minutes, offers
    /// "You were in Lounge. [Rejoin]" (after an update, a crash or a quit).
    pub last_call: Option<LastCall>,
}

/// A call we were in (`Config::last_call`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LastCall {
    pub server: String,
    pub channel: String,
    /// Unix ms: when we were last in it.
    pub at: i64,
}

/// How long after a call "You were in …" is offered.
pub const REJOIN_MS: i64 = 10 * 60_000;

impl LastCall {
    /// Is this a call to offer again on `server`, `now` (Unix ms)? (A clock that
    /// went backwards doesn't count as recent.)
    pub fn offer(&self, server: &str, now: i64) -> bool {
        self.server == server && !self.channel.is_empty() && (0..=REJOIN_MS).contains(&(now - self.at))
    }
}

/// Settings › Look › Size limits.
pub const SIZE_MIN: f32 = 0.9;
pub const SIZE_MAX: f32 = 1.75;

impl Default for Config {
    fn default() -> Self {
        Self {
            server: String::new(),
            key: String::new(),
            uid: String::new(),
            name: String::new(),
            color: String::new(),
            input_device: String::new(),
            output_device: String::new(),
            mode: InputMode::Vad,
            vad_threshold: -50.0,
            ptt_key: "Backquote".into(),
            mute_key: String::new(),
            deafen_key: String::new(),
            push_to_mute_key: String::new(),
            bitrate: 128_000,
            noise_suppression: true,
            stronger_noise: false,
            auto_gain: true,
            mic_volume: 1.0,
            echo_cancellation: true,
            sounds: true,
            remind_muted: true,
            muted_explained: false,
            ptt_click: true,
            notifications: true,
            notify_all: false,
            muted: false,
            deafened: false,
            volumes: HashMap::new(),
            screen_volumes: HashMap::new(),
            lower_media: false,
            share_quality: crate::screen::Quality::Smooth,
            share_sound: true,
            channel: "general".into(),
            last_read: HashMap::new(),
            theme: ThemePref::Dark,
            zoom: 1.0,
            size: None,
            sound_volume: 0.5,
            sound_mute: false,
            saved_note_seen: false,
            relay_only: false,
            password_later: Vec::new(),
            last_call: None,
        }
    }
}

impl Config {
    pub fn default_path() -> PathBuf {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        // Linux convention is lowercase; Windows/macOS show the app name.
        if cfg!(target_os = "linux") { base.join("yap") } else { base.join("Yap") }.join("config.json")
    }

    /// Load (or create) the config, filling in a uid and color on first run.
    pub fn load(path: &Path) -> Self {
        let mut cfg: Config = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        cfg.fill_defaults();
        cfg
    }

    pub fn fill_defaults(&mut self) {
        if self.uid.is_empty() {
            self.uid = random_hex(16);
        }
        if self.color.is_empty() {
            let i = (random_u32() as usize) % COLORS.len();
            self.color = COLORS[i].to_string();
        }
        if self.ptt_key.is_empty() {
            self.ptt_key = "Backquote".into();
        }
        if !BITRATES.contains(&self.bitrate) {
            self.bitrate = 128_000;
        }
        if (0.5..=2.0).contains(&self.zoom) && (self.zoom - 1.0).abs() > 0.001 && self.size.is_none() {
            self.size = Some(self.zoom.clamp(SIZE_MIN, SIZE_MAX));
        }
        self.zoom = 1.0;
        if !self.sound_volume.is_finite() {
            self.sound_volume = 0.5;
        }
        self.sound_volume = self.sound_volume.clamp(0.0, 1.0);
        self.mic_volume = if self.mic_volume.is_finite() { self.mic_volume.clamp(0.0, crate::audio::dsp::MIC_VOLUME_MAX) } else { 1.0 };
        if let Some(s) = self.size {
            self.size = s.is_finite().then(|| s.clamp(SIZE_MIN, SIZE_MAX));
        }
        self.server = normalize_server(&self.server);
    }

    /// Write atomically (temp file + rename) so a crash never leaves half a file.
    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(json) = serde_json::to_string_pretty(self) else { return };
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    pub fn volume_for(&self, uid: &str) -> f32 {
        self.volumes.get(uid).copied().unwrap_or(1.0)
    }
}

/// `https://host/path/` -> `https://host/path`; adds `https://` when missing.
pub fn normalize_server(s: &str) -> String {
    let s = s.trim().trim_end_matches('/');
    if s.is_empty() {
        return String::new();
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        s.to_string()
    } else if s.starts_with("localhost") || s.starts_with("127.") {
        format!("http://{s}")
    } else {
        format!("https://{s}")
    }
}

/// Is `host` (a URL's host, `[…]` around IPv6 or not) this computer:
/// `localhost`, 127.0.0.0/8 or ::1?
pub fn is_loopback(host: &str) -> bool {
    let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(host);
    host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// The host in `scheme://[user@]host[:port][/…]`.
fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    }
}

/// What Yap says when a server's address is plain `http://`.
pub const INSECURE_SERVER: &str = "That link starts with http://, which would send your invite key and password unencrypted. Use the server's https:// link (plain http:// only works for a server on this computer).";

/// Why Yap won't talk to `url`, if it won't: plain `http://` (or `ws://`)
/// would carry the invite key, the device token and your password in the
/// clear, and let anyone on the way change what comes back (an update's
/// manifest too), so it's only for a server on this computer. Every address
/// set by `--server`, a pasted link or the saved settings goes through this,
/// and so does every request (`http::secure_only`, `net::ws_request`).
pub fn insecure(url: &str) -> Option<&'static str> {
    let url = url.trim();
    let scheme = url.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    let plain = matches!(scheme.as_deref(), Some("http" | "ws"));
    (plain && !is_loopback(url_host(url))).then_some(INSECURE_SERVER)
}

pub fn invite_link(cfg: &Config) -> String {
    if cfg.key.is_empty() { cfg.server.clone() } else { format!("{}/#key={}", cfg.server, percent_encode(&cfg.key)) }
}

/// Parse an invite link `https://host/#key=K` (or `?key=K`), or a bare host.
/// Returns (server, key).
pub fn parse_invite(input: &str) -> (String, Option<String>) {
    let input = input.trim();
    let (before_hash, fragment) = match input.split_once('#') {
        Some((a, b)) => (a, Some(b)),
        None => (input, None),
    };
    let (base, query) = match before_hash.split_once('?') {
        Some((a, b)) => (a, Some(b)),
        None => (before_hash, None),
    };
    let find_key = |params: &str| {
        params.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == "key").then(|| percent_decode(v))
        })
    };
    let key = fragment.and_then(find_key).or_else(|| query.and_then(find_key));
    (normalize_server(base), key)
}

/// Device tokens, per server, in their own file (`tokens.json`, owner-only
/// on Unix) so the settings file never carries account secrets.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Tokens {
    #[serde(default)]
    pub servers: HashMap<String, String>,
}

impl Tokens {
    pub fn path(config_path: &Path) -> PathBuf {
        config_path.with_file_name("tokens.json")
    }

    pub fn load(config_path: &Path) -> Self {
        std::fs::read_to_string(Self::path(config_path)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, config_path: &Path) {
        let path = Self::path(config_path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(json) = serde_json::to_string_pretty(self) else { return };
        let tmp = path.with_extension("json.tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        if let Ok(mut f) = opts.open(&tmp) {
            use std::io::Write;
            if f.write_all(json.as_bytes()).is_ok() {
                drop(f);
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| (b as char).to_digit(16);
    while i < bytes.len() {
        // (By bytes: "%a" then a letter like é would cut that letter in half, which panicked.)
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    if getrandom::fill(&mut buf).is_err() {
        // Extremely unlikely; fall back to time-based noise.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (t >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(97);
        }
    }
    buf
}

pub fn random_u32() -> u32 {
    u32::from_le_bytes(random_bytes::<4>())
}

pub fn random_hex(bytes: usize) -> String {
    let mut out = String::with_capacity(bytes * 2);
    for chunk in 0..bytes.div_ceil(16) {
        let b = random_bytes::<16>();
        for x in b.iter().take((bytes - chunk * 16).min(16)) {
            out.push_str(&format!("{x:02x}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_setting() {
        // 0.6.0's Text size carries over; 100% (never touched) becomes Auto.
        let mut c: Config = serde_json::from_str(r#"{"zoom": 1.2}"#).unwrap();
        c.fill_defaults();
        assert_eq!((c.size, c.zoom), (Some(1.2), 1.0));
        let mut c: Config = serde_json::from_str(r#"{"zoom": 1.0}"#).unwrap();
        c.fill_defaults();
        assert_eq!(c.size, None);
        let mut c: Config = serde_json::from_str(r#"{"size": 3.0}"#).unwrap();
        c.fill_defaults();
        assert_eq!(c.size, Some(SIZE_MAX));
        let mut c: Config = serde_json::from_str(r#"{"size": null, "zoom": 0.8}"#).unwrap();
        c.fill_defaults();
        assert_eq!(c.size, Some(SIZE_MIN));
    }

    #[test]
    fn invite_links() {
        assert_eq!(
            parse_invite("https://yap.example.dev/#key=abc%20d"),
            ("https://yap.example.dev".into(), Some("abc d".into()))
        );
        assert_eq!(parse_invite("http://localhost:8788"), ("http://localhost:8788".into(), None));
        assert_eq!(parse_invite("localhost:8788/"), ("http://localhost:8788".into(), None));
        assert_eq!(parse_invite("yap.example.dev?key=k1"), ("https://yap.example.dev".into(), Some("k1".into())));
        // A pasted link with a stray "%" before a non-ASCII letter keeps it as typed (it panicked).
        assert_eq!(parse_invite("https://yap.example.dev/#key=%aé"), ("https://yap.example.dev".into(), Some("%aé".into())));
        assert_eq!(percent_decode("%C3%A9%x1%-1"), "é%x1%-1", "only two hex digits make a byte");
    }

    /// Plain http:// (or ws://) only for this computer; https:// anywhere.
    #[test]
    fn plain_http_only_for_this_computer() {
        for ok in ["https://yap.example.dev", "wss://yap.example.dev/ws", "http://localhost:8788", "http://LOCALHOST", "http://127.0.0.1:8787/x", "http://127.9.9.9", "http://[::1]:8787", "ws://localhost:8788/ws", "https://192.168.1.4", ""] {
            assert_eq!(insecure(ok), None, "{ok}");
        }
        for bad in ["http://yap.example.dev", "HTTP://yap.example.dev", "ws://yap.example.dev/ws", "http://192.168.1.4:8787", "http://10.0.0.1", "http://localhost.evil.example", "http://127.0.0.1.evil.example", "http://localhost@evil.example/", "http://evil.example\\@localhost", "http://[::2]", "http://0.0.0.0:8787"] {
            assert_eq!(insecure(bad), Some(INSECURE_SERVER), "{bad}");
        }
        // What --server, a pasted link and the saved settings come out as.
        assert_eq!(insecure(&parse_invite("localhost:8788").0), None);
        assert_eq!(insecure(&parse_invite("127.0.0.1:8788/#key=k").0), None);
        assert_eq!(insecure(&parse_invite("yap.example.dev/#key=k").0), None, "a bare host gets https://");
        assert!(insecure(&parse_invite("http://yap.example.dev/#key=k").0).is_some());
        let mut c: Config = serde_json::from_str(r#"{"server": "http://chat.example.com/"}"#).unwrap();
        c.fill_defaults();
        assert!(insecure(&c.server).is_some(), "a saved plain-http server is kept, and refused when connecting");
    }

    /// The prompts Later put off are a new field: a config from an older
    /// version (no such key, or the unreleased `password_prompt_done`) still
    /// loads, everything kept, with nothing put off; what Later saves stays.
    #[test]
    fn password_prompts_put_off() {
        let dir = std::env::temp_dir().join(format!("yap-prompt-test-{}", random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"server":"https://chat.example.com","name":"Ada","uid":"u1","bitrate":96000,"relay_only":true,"password_prompt_done":true}"#).unwrap();
        let mut c = Config::load(&path);
        assert_eq!((c.name.as_str(), c.uid.as_str(), c.bitrate, c.relay_only), ("Ada", "u1", 96_000, true), "an old config loads whole");
        assert!(c.password_later.is_empty(), "nothing put off on this device yet");
        c.password_later.push("pin".into());
        c.save(&path);
        let again = Config::load(&path);
        assert_eq!(again.password_later, ["pin"], "Later is remembered, for that reason only");
        assert_eq!(again.name, "Ada");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Round 10's settings are new fields: a config from before them loads
    /// whole, with their defaults (no extra keys, reminders and the
    /// push-to-talk click on, shared media left alone), and what's saved stays.
    #[test]
    fn round10_settings_default_and_old_configs_load() {
        let dir = std::env::temp_dir().join(format!("yap-r10-test-{}", random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"server":"https://chat.example.com","name":"Ada","uid":"u1","mode":"ptt","ptt_key":"KeyV","muted":true,"volumes":{"u2":0.6}}"#).unwrap();
        let mut c = Config::load(&path);
        assert_eq!((c.name.as_str(), c.ptt_key.as_str(), c.mode, c.muted, c.volume_for("u2")), ("Ada", "KeyV", InputMode::Ptt, true, 0.6));
        assert_eq!((c.mute_key.as_str(), c.deafen_key.as_str(), c.push_to_mute_key.as_str()), ("", "", ""));
        assert!(c.remind_muted && !c.muted_explained, "the reminder is on, not explained yet");
        assert!(c.ptt_click, "the push-to-talk click is on");
        assert!(!c.lower_media, "shared media is left alone unless you ask");
        c.mute_key = "Ctrl+Shift+KeyM".into();
        c.push_to_mute_key = "Mouse4".into();
        c.save(&path);
        let again = Config::load(&path);
        assert_eq!((again.mute_key.as_str(), again.push_to_mute_key.as_str(), again.deafen_key.as_str()), ("Ctrl+Shift+KeyM", "Mouse4", ""));
        // An empty push-to-talk key (hand-edited) falls back to the default.
        std::fs::write(&path, r#"{"ptt_key":""}"#).unwrap();
        assert_eq!(Config::load(&path).ptt_key, "Backquote");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mic_volume_and_last_call() {
        // Older configs: Mic volume 100 %, no call to offer. Broken values come back sane.
        let mut c: Config = serde_json::from_str("{}").unwrap();
        c.fill_defaults();
        assert_eq!((c.mic_volume, c.last_call.clone()), (1.0, None));
        let mut c: Config = serde_json::from_str(r#"{"mic_volume": 7.5}"#).unwrap();
        c.fill_defaults();
        assert_eq!(c.mic_volume, 2.0);
        let mut c: Config = serde_json::from_str(r#"{"mic_volume": -1}"#).unwrap();
        c.fill_defaults();
        assert_eq!(c.mic_volume, 0.0);
        // A call is offered again on the same server within 10 minutes, not after,
        // not on another server, and not from a clock that went backwards.
        let call = LastCall { server: "https://a".into(), channel: "lounge".into(), at: 1_000_000 };
        assert!(call.offer("https://a", 1_000_000));
        assert!(call.offer("https://a", 1_000_000 + REJOIN_MS));
        assert!(!call.offer("https://a", 1_000_000 + REJOIN_MS + 1));
        assert!(!call.offer("https://b", 1_000_000));
        assert!(!call.offer("https://a", 999_000));
        // It survives a save and a load.
        let mut c = Config { last_call: Some(call.clone()), ..Config::default() };
        c = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c.last_call, Some(call));
    }

    #[test]
    fn hex() {
        assert_eq!(random_hex(16).len(), 32);
        assert_eq!(random_hex(20).len(), 40);
    }
}

#[cfg(test)]
mod skin_tests {
    use super::*;

    /// There's one look now (Windows 98). A config from an older version that
    /// names a removed skin ("apple", "arch") still loads, as Windows 98,
    /// keeping everything else, light or dark included; saving drops the key.
    #[test]
    fn removed_skins_load_as_win98() {
        let dir = std::env::temp_dir().join(format!("yap-skin-test-{}", random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        for (skin, theme, want) in [("apple", "light", ThemePref::Light), ("arch", "dark", ThemePref::Dark), ("apple", "system", ThemePref::System), ("win98", "light", ThemePref::Light)] {
            let path = dir.join(format!("{skin}-{theme}.json"));
            let json = format!(r#"{{"server":"https://chat.example.com","name":"Ada","uid":"u1","skin":"{skin}","theme":"{theme}","bitrate":256000}}"#);
            std::fs::write(&path, json).unwrap();
            let c = Config::load(&path);
            assert_eq!((c.server.as_str(), c.name.as_str(), c.uid.as_str(), c.bitrate), ("https://chat.example.com", "Ada", "u1", 256_000), "{skin}: nothing else lost");
            assert_eq!(c.theme, want, "{skin}: light/dark kept");
            c.save(&path);
            let saved = std::fs::read_to_string(&path).unwrap();
            assert!(!saved.contains("skin"), "{skin}: the old key is gone once saved");
            assert_eq!(Config::load(&path).theme, want);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
