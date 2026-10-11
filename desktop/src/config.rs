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
    /// "Midnight", the default (like the web client).
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
    /// Push-to-talk key, as a KeyboardEvent.code-style name ("Backquote", "KeyV", "Mouse4"…).
    pub ptt_key: String,
    /// Opus bitrate in bits/s.
    pub bitrate: u32,
    pub noise_suppression: bool,
    /// Settings › Voice › Automatic gain: speech towards −23 dBFS, at most +20 dB.
    pub auto_gain: bool,
    pub sounds: bool,
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
            bitrate: 128_000,
            noise_suppression: true,
            auto_gain: true,
            sounds: true,
            notifications: true,
            notify_all: false,
            muted: false,
            deafened: false,
            volumes: HashMap::new(),
            screen_volumes: HashMap::new(),
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
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
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
