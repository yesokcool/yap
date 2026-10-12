//! Global keys by polling key state (~100 Hz): push-to-talk (double-tap to
//! stay live), mute, deafen and push-to-mute (PROTOCOL.md › Voice › Transmit
//! gate). Works while Yap is in the background and sees both key down and key
//! up, without hooks or grabbing the key away from other apps. A key can come
//! with modifiers held ("Ctrl+Shift+KeyM"), so a mute key needn't be a key you
//! type with.
//!
//! * Windows: `GetAsyncKeyState` (keys and mouse buttons).
//! * Linux: evdev `EVIOCGKEY` on /dev/input/event* when readable (X11 *and*
//!   Wayland; needs the user in the `input` group), else X11 `XQueryKeymap`
//!   (libX11 loaded at runtime). Under Wayland without evdev access only the
//!   in-window fallback works.
//! * macOS: `CGEventSourceKeyState` (needs Input Monitoring permission).
//!
//! Key names follow the web client's `KeyboardEvent.code` (`Backquote`,
//! `KeyV`, `ShiftLeft`…) plus `Mouse3`/`Mouse4`/`Mouse5`.

use crate::audio::Gate;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// (name, windows VK, X11 keysym, evdev code, macOS keycode)
#[allow(dead_code)] // each platform reads its own column
pub struct KeyDef {
    pub name: &'static str,
    pub vk: u16,
    pub keysym: u32,
    pub evdev: u16,
    pub mac: u16,
}

const NONE16: u16 = 0xFFFF;
const NONE32: u32 = 0;

macro_rules! k {
    ($n:expr, $vk:expr, $ks:expr, $ev:expr, $mac:expr) => {
        KeyDef { name: $n, vk: $vk, keysym: $ks, evdev: $ev, mac: $mac }
    };
}

pub static KEYS: &[KeyDef] = &[
    k!("Backquote", 0xC0, 0x60, 41, 0x32),
    k!("Space", 0x20, 0x20, 57, 0x31),
    k!("Tab", 0x09, 0xFF09, 15, 0x30),
    k!("CapsLock", 0x14, 0xFFE5, 58, 0x39),
    k!("ShiftLeft", 0xA0, 0xFFE1, 42, 0x38),
    k!("ShiftRight", 0xA1, 0xFFE2, 54, 0x3C),
    k!("ControlLeft", 0xA2, 0xFFE3, 29, 0x3B),
    k!("ControlRight", 0xA3, 0xFFE4, 97, 0x3E),
    k!("AltLeft", 0xA4, 0xFFE9, 56, 0x3A),
    k!("AltRight", 0xA5, 0xFFEA, 100, 0x3D),
    k!("MetaLeft", 0x5B, 0xFFEB, 125, 0x37),
    k!("MetaRight", 0x5C, 0xFFEC, 126, 0x36),
    k!("KeyA", 0x41, 0x61, 30, 0x00),
    k!("KeyB", 0x42, 0x62, 48, 0x0B),
    k!("KeyC", 0x43, 0x63, 46, 0x08),
    k!("KeyD", 0x44, 0x64, 32, 0x02),
    k!("KeyE", 0x45, 0x65, 18, 0x0E),
    k!("KeyF", 0x46, 0x66, 33, 0x03),
    k!("KeyG", 0x47, 0x67, 34, 0x05),
    k!("KeyH", 0x48, 0x68, 35, 0x04),
    k!("KeyI", 0x49, 0x69, 23, 0x22),
    k!("KeyJ", 0x4A, 0x6A, 36, 0x26),
    k!("KeyK", 0x4B, 0x6B, 37, 0x28),
    k!("KeyL", 0x4C, 0x6C, 38, 0x25),
    k!("KeyM", 0x4D, 0x6D, 50, 0x2E),
    k!("KeyN", 0x4E, 0x6E, 49, 0x2D),
    k!("KeyO", 0x4F, 0x6F, 24, 0x1F),
    k!("KeyP", 0x50, 0x70, 25, 0x23),
    k!("KeyQ", 0x51, 0x71, 16, 0x0C),
    k!("KeyR", 0x52, 0x72, 19, 0x0F),
    k!("KeyS", 0x53, 0x73, 31, 0x01),
    k!("KeyT", 0x54, 0x74, 20, 0x11),
    k!("KeyU", 0x55, 0x75, 22, 0x20),
    k!("KeyV", 0x56, 0x76, 47, 0x09),
    k!("KeyW", 0x57, 0x77, 17, 0x0D),
    k!("KeyX", 0x58, 0x78, 45, 0x07),
    k!("KeyY", 0x59, 0x79, 21, 0x10),
    k!("KeyZ", 0x5A, 0x7A, 44, 0x06),
    k!("Digit0", 0x30, 0x30, 11, 0x1D),
    k!("Digit1", 0x31, 0x31, 2, 0x12),
    k!("Digit2", 0x32, 0x32, 3, 0x13),
    k!("Digit3", 0x33, 0x33, 4, 0x14),
    k!("Digit4", 0x34, 0x34, 5, 0x15),
    k!("Digit5", 0x35, 0x35, 6, 0x17),
    k!("Digit6", 0x36, 0x36, 7, 0x16),
    k!("Digit7", 0x37, 0x37, 8, 0x1A),
    k!("Digit8", 0x38, 0x38, 9, 0x1C),
    k!("Digit9", 0x39, 0x39, 10, 0x19),
    k!("Minus", 0xBD, 0x2D, 12, 0x1B),
    k!("Equal", 0xBB, 0x3D, 13, 0x18),
    k!("BracketLeft", 0xDB, 0x5B, 26, 0x21),
    k!("BracketRight", 0xDD, 0x5D, 27, 0x1E),
    k!("Backslash", 0xDC, 0x5C, 43, 0x2A),
    k!("Semicolon", 0xBA, 0x3B, 39, 0x29),
    k!("Quote", 0xDE, 0x27, 40, 0x27),
    k!("Comma", 0xBC, 0x2C, 51, 0x2B),
    k!("Period", 0xBE, 0x2E, 52, 0x2F),
    k!("Slash", 0xBF, 0x2F, 53, 0x2C),
    k!("F1", 0x70, 0xFFBE, 59, 0x7A),
    k!("F2", 0x71, 0xFFBF, 60, 0x78),
    k!("F3", 0x72, 0xFFC0, 61, 0x63),
    k!("F4", 0x73, 0xFFC1, 62, 0x76),
    k!("F5", 0x74, 0xFFC2, 63, 0x60),
    k!("F6", 0x75, 0xFFC3, 64, 0x61),
    k!("F7", 0x76, 0xFFC4, 65, 0x62),
    k!("F8", 0x77, 0xFFC5, 66, 0x64),
    k!("F9", 0x78, 0xFFC6, 67, 0x65),
    k!("F10", 0x79, 0xFFC7, 68, 0x6D),
    k!("F11", 0x7A, 0xFFC8, 87, 0x67),
    k!("F12", 0x7B, 0xFFC9, 88, 0x6F),
    k!("F13", 0x7C, 0xFFCA, 183, 0x69),
    k!("F14", 0x7D, 0xFFCB, 184, 0x6B),
    k!("F15", 0x7E, 0xFFCC, 185, 0x71),
    k!("F16", 0x7F, 0xFFCD, 186, 0x6A),
    k!("F17", 0x80, 0xFFCE, 187, 0x40),
    k!("F18", 0x81, 0xFFCF, 188, 0x4F),
    k!("F19", 0x82, 0xFFD0, 189, 0x50),
    k!("F20", 0x83, 0xFFD1, 190, 0x5A),
    k!("Insert", 0x2D, 0xFF63, 110, 0x72),
    k!("Delete", 0x2E, 0xFFFF, 111, 0x75),
    k!("Home", 0x24, 0xFF50, 102, 0x73),
    k!("End", 0x23, 0xFF57, 107, 0x77),
    k!("PageUp", 0x21, 0xFF55, 104, 0x74),
    k!("PageDown", 0x22, 0xFF56, 109, 0x79),
    k!("ArrowUp", 0x26, 0xFF52, 103, 0x7E),
    k!("ArrowDown", 0x28, 0xFF54, 108, 0x7D),
    k!("ArrowLeft", 0x25, 0xFF51, 105, 0x7B),
    k!("ArrowRight", 0x27, 0xFF53, 106, 0x7C),
    k!("Pause", 0x13, 0xFF13, 119, NONE16),
    k!("ScrollLock", 0x91, 0xFF14, 70, NONE16),
    k!("Numpad0", 0x60, 0xFFB0, 82, 0x52),
    k!("Numpad1", 0x61, 0xFFB1, 79, 0x53),
    k!("Numpad2", 0x62, 0xFFB2, 80, 0x54),
    k!("Numpad3", 0x63, 0xFFB3, 81, 0x55),
    k!("Numpad4", 0x64, 0xFFB4, 75, 0x56),
    k!("Numpad5", 0x65, 0xFFB5, 76, 0x57),
    k!("Numpad6", 0x66, 0xFFB6, 77, 0x58),
    k!("Numpad7", 0x67, 0xFFB7, 71, 0x59),
    k!("Numpad8", 0x68, 0xFFB8, 72, 0x5B),
    k!("Numpad9", 0x69, 0xFFB9, 73, 0x5C),
    // Mouse buttons: X11 keysym slot holds the XQueryPointer mask (middle only).
    k!("Mouse3", 0x04, 1 << 9, 0x112, 2),
    k!("Mouse4", 0x05, NONE32, 0x113, 3),
    k!("Mouse5", 0x06, NONE32, 0x114, 4),
];

pub fn key_def(name: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|k| k.name == name)
}

/// One key's own label ("` (backtick)", "Left Shift", "M").
fn single_label(name: &str) -> String {
    match name {
        "Backquote" => "` (backtick)".into(),
        "CapsLock" => "Caps Lock".into(),
        "ShiftLeft" => "Left Shift".into(),
        "ShiftRight" => "Right Shift".into(),
        "ControlLeft" => "Left Ctrl".into(),
        "ControlRight" => "Right Ctrl".into(),
        "AltLeft" => if cfg!(target_os = "macos") { "Left Option" } else { "Left Alt" }.into(),
        "AltRight" => if cfg!(target_os = "macos") { "Right Option" } else { "Right Alt" }.into(),
        "MetaLeft" => if cfg!(target_os = "macos") { "Left ⌘" } else { "Left Super" }.into(),
        "MetaRight" => if cfg!(target_os = "macos") { "Right ⌘" } else { "Right Super" }.into(),
        "Mouse3" => "Middle mouse".into(),
        "Mouse4" => "Mouse 4 (back)".into(),
        "Mouse5" => "Mouse 5 (forward)".into(),
        n => n.trim_start_matches("Key").trim_start_matches("Digit").to_string(),
    }
}

/// A key's (or a combination's) label for Settings: "` (backtick)", "Ctrl+Shift+M"
/// ("⇧⌘M" on a Mac), "None" when nothing is set.
pub fn key_label(name: &str) -> String {
    if name.is_empty() {
        return "None".into();
    }
    Combo::parse(name).map(|c| c.label()).unwrap_or_else(|| single_label(name))
}

/// Modifiers a combination can ask for (either side counts): bit, name, the two keys.
const MODS: [(u8, &str, [&str; 2]); 4] = [
    (CTRL, "Ctrl", ["ControlLeft", "ControlRight"]),
    (ALT, "Alt", ["AltLeft", "AltRight"]),
    (SHIFT, "Shift", ["ShiftLeft", "ShiftRight"]),
    (META, "Meta", ["MetaLeft", "MetaRight"]),
];
const CTRL: u8 = 1;
const ALT: u8 = 2;
const SHIFT: u8 = 4;
const META: u8 = 8;

/// Which modifier a key is (Left/Right Shift…), if it is one.
fn modifier_bit(name: &str) -> Option<u8> {
    MODS.iter().find(|(_, _, keys)| keys.contains(&name)).map(|(bit, ..)| *bit)
}

/// A key, maybe with modifiers held: "Backquote", "Mouse4", "Ctrl+Shift+KeyM".
/// The last part is a key from `KEYS` (it may be a modifier on its own, like
/// "ShiftLeft" for push-to-talk); the ones before it are "Ctrl", "Alt",
/// "Shift" or "Meta", either side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    mods: u8,
    key: &'static KeyDef,
}

impl PartialEq for KeyDef {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl Eq for KeyDef {}
impl std::fmt::Debug for KeyDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

impl Combo {
    pub fn parse(s: &str) -> Option<Combo> {
        let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let key = key_def(parts.pop()?)?;
        let mut mods = 0;
        for p in parts {
            mods |= MODS.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(p))?.0;
        }
        Some(Combo { mods, key })
    }

    /// The name kept in config.json ("Ctrl+Shift+KeyM").
    pub fn name(&self) -> String {
        let mut out = String::new();
        for (bit, n, _) in MODS {
            if self.mods & bit != 0 {
                out.push_str(n);
                out.push('+');
            }
        }
        out.push_str(self.key.name);
        out
    }

    /// "Ctrl+Shift+M" ("⌘+Shift+M", "Option+…" on a Mac, ⌘ first the way
    /// people say it; in words, since the UI font has no ⌃ or ⇧).
    pub fn label(&self) -> String {
        let key = single_label(self.key.name);
        let order: [(u8, &str); 4] = if cfg!(target_os = "macos") {
            [(META, "⌘"), (CTRL, "Ctrl"), (ALT, "Option"), (SHIFT, "Shift")]
        } else {
            [(CTRL, "Ctrl"), (ALT, "Alt"), (SHIFT, "Shift"), (META, if cfg!(windows) { "Win" } else { "Super" })]
        };
        let mut out: Vec<&str> = Vec::new();
        for (bit, n) in order {
            if self.mods & bit != 0 {
                out.push(n);
            }
        }
        out.push(&key);
        out.join("+")
    }

    /// Held right now, by a key-state reader (extra modifiers don't matter:
    /// push-to-talk still works while you hold Shift to run in a game).
    fn down(&self, is_down: impl Fn(&KeyDef) -> bool) -> bool {
        is_down(self.key)
            && MODS.iter().filter(|(bit, ..)| self.mods & bit != 0).all(|(_, _, keys)| keys.iter().filter_map(|k| key_def(k)).any(&is_down))
    }
}

/// egui key + modifiers -> our combination name (the in-window fallback and binding).
pub fn combo_from_egui(key: eframe::egui::Key, m: eframe::egui::Modifiers) -> Option<String> {
    let mut mods = 0;
    if m.ctrl {
        mods |= CTRL;
    }
    if m.alt {
        mods |= ALT;
    }
    if m.shift {
        mods |= SHIFT;
    }
    if m.mac_cmd {
        mods |= META;
    }
    Some(Combo { mods, key: key_def(from_egui(key)?)? }.name())
}

/// Does an in-window key event (key + modifiers held) press this combination?
fn egui_matches(c: &Combo, name: &str, m: eframe::egui::Modifiers) -> bool {
    let held = (if m.ctrl { CTRL } else { 0 }) | (if m.alt { ALT } else { 0 }) | (if m.shift { SHIFT } else { 0 }) | (if m.mac_cmd { META } else { 0 });
    c.key.name == name && c.mods & held == c.mods
}

/// egui key -> our key name, for the in-window fallback and binding.
pub fn from_egui(key: eframe::egui::Key) -> Option<&'static str> {
    use eframe::egui::Key as K;
    Some(match key {
        K::Backtick => "Backquote",
        K::Space => "Space",
        K::Tab => "Tab",
        K::Minus => "Minus",
        K::Equals => "Equal",
        K::OpenBracket => "BracketLeft",
        K::CloseBracket => "BracketRight",
        K::Backslash => "Backslash",
        K::Semicolon => "Semicolon",
        K::Quote => "Quote",
        K::Comma => "Comma",
        K::Period => "Period",
        K::Slash => "Slash",
        K::Insert => "Insert",
        K::Delete => "Delete",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "PageUp",
        K::PageDown => "PageDown",
        K::ArrowUp => "ArrowUp",
        K::ArrowDown => "ArrowDown",
        K::ArrowLeft => "ArrowLeft",
        K::ArrowRight => "ArrowRight",
        K::A => "KeyA", K::B => "KeyB", K::C => "KeyC", K::D => "KeyD", K::E => "KeyE", K::F => "KeyF",
        K::G => "KeyG", K::H => "KeyH", K::I => "KeyI", K::J => "KeyJ", K::K => "KeyK", K::L => "KeyL",
        K::M => "KeyM", K::N => "KeyN", K::O => "KeyO", K::P => "KeyP", K::Q => "KeyQ", K::R => "KeyR",
        K::S => "KeyS", K::T => "KeyT", K::U => "KeyU", K::V => "KeyV", K::W => "KeyW", K::X => "KeyX",
        K::Y => "KeyY", K::Z => "KeyZ",
        K::Num0 => "Digit0", K::Num1 => "Digit1", K::Num2 => "Digit2", K::Num3 => "Digit3", K::Num4 => "Digit4",
        K::Num5 => "Digit5", K::Num6 => "Digit6", K::Num7 => "Digit7", K::Num8 => "Digit8", K::Num9 => "Digit9",
        K::F1 => "F1", K::F2 => "F2", K::F3 => "F3", K::F4 => "F4", K::F5 => "F5", K::F6 => "F6",
        K::F7 => "F7", K::F8 => "F8", K::F9 => "F9", K::F10 => "F10", K::F11 => "F11", K::F12 => "F12",
        K::F13 => "F13", K::F14 => "F14", K::F15 => "F15", K::F16 => "F16", K::F17 => "F17", K::F18 => "F18",
        K::F19 => "F19", K::F20 => "F20",
        _ => return None,
    })
}

/// What a global key does (Settings › Voice, next to each other).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Push to talk (held; double-tap to stay live).
    Talk,
    /// Toggle mute.
    Mute,
    /// Toggle deafen.
    Deafen,
    /// Push to mute (held: muted, whatever the mode — a "cough button").
    Cough,
}

impl Action {
    pub const ALL: [Action; 4] = [Action::Talk, Action::Mute, Action::Deafen, Action::Cough];

    fn index(self) -> usize {
        self as usize
    }

    /// The row's label in Settings › Voice.
    pub fn label(self) -> &'static str {
        match self {
            Action::Talk => "Push-to-talk key:",
            Action::Mute => "Mute key:",
            Action::Deafen => "Deafen key:",
            Action::Cough => "Push-to-mute key:",
        }
    }

    /// What it's called in "that's already your … key".
    pub fn noun(self) -> &'static str {
        match self {
            Action::Talk => "push-to-talk",
            Action::Mute => "mute",
            Action::Deafen => "deafen",
            Action::Cough => "push-to-mute",
        }
    }
}

/// The config field an action's key lives in.
pub fn config_key(c: &crate::config::Config, a: Action) -> &str {
    match a {
        Action::Talk => &c.ptt_key,
        Action::Mute => &c.mute_key,
        Action::Deafen => &c.deafen_key,
        Action::Cough => &c.push_to_mute_key,
    }
}

/// Bind `combo` to `a` ("" clears it; push-to-talk always has a key). A key
/// that's already another action's is refused, in words for a toast.
pub fn bind(c: &mut crate::config::Config, a: Action, combo: &str) -> Result<(), String> {
    if combo.is_empty() && a == Action::Talk {
        return Err("Push-to-talk needs a key.".into());
    }
    if !combo.is_empty()
        && let Some(other) = Action::ALL.into_iter().find(|o| *o != a && Combo::parse(config_key(c, *o)).is_some_and(|k| Some(k) == Combo::parse(combo)))
    {
        return Err(format!("{} is already your {} key.", key_label(combo), other.noun()));
    }
    let slot = match a {
        Action::Talk => &mut c.ptt_key,
        Action::Mute => &mut c.mute_key,
        Action::Deafen => &mut c.deafen_key,
        Action::Cough => &mut c.push_to_mute_key,
    };
    *slot = combo.to_string();
    Ok(())
}

/// The window's own mute and deafen shortcuts (like the web's): Ctrl+Shift+M
/// and Ctrl+Shift+D, ⌘+Shift+M and ⌘+Shift+D on a Mac.
pub const MUTE_SHORTCUT: &str = if cfg!(target_os = "macos") { "Meta+Shift+KeyM" } else { "Ctrl+Shift+KeyM" };
pub const DEAFEN_SHORTCUT: &str = if cfg!(target_os = "macos") { "Meta+Shift+KeyD" } else { "Ctrl+Shift+KeyD" };

/// Is a window shortcut free, i.e. not also one of the global keys (which
/// would then toggle twice)?
pub fn shortcut_free(c: &crate::config::Config, shortcut: &str) -> bool {
    let s = Combo::parse(shortcut);
    !Action::ALL.into_iter().any(|a| Combo::parse(config_key(c, a)) == s)
}

/// A second mute or deafen press closer than this is the same press seen
/// twice (the poller and the window both see a quick tap), not a second toggle.
const TOGGLE_DEBOUNCE: Duration = Duration::from_millis(250);

/// Two presses of push-to-talk this close together keep it open.
pub const DOUBLE_TAP: Duration = Duration::from_millis(400);

/// Double-tap to stay live (PROTOCOL.md › Voice › Transmit gate): a second
/// press within 400 ms of the first latches push-to-talk open; the next
/// press unlatches it (and works as an ordinary hold until it's let go).
#[derive(Default, Debug)]
pub struct Latch {
    last_press: Option<Instant>,
    pub on: bool,
}

impl Latch {
    /// The key went down.
    pub fn press(&mut self, now: Instant) {
        if self.on {
            // Tap to stop; that press doesn't count as the first of another double-tap.
            self.on = false;
            self.last_press = None;
        } else if self.last_press.is_some_and(|t| now.duration_since(t) <= DOUBLE_TAP) {
            self.on = true;
            self.last_press = None;
        } else {
            self.last_press = Some(now);
        }
    }

    pub fn reset(&mut self) {
        *self = Latch::default();
    }
}

/// Catching the next key pressed (Settings' "Press a key…"), from global key
/// scans: a key with whatever modifiers are held, or a modifier alone once
/// it's let go without anything else (so Shift can be a push-to-talk key, and
/// Ctrl+Shift+M can be a mute key).
#[derive(Default)]
struct Binder {
    prev: Vec<bool>,
    /// The modifier pressed first (None after a second one joined it).
    lone: Option<usize>,
    mixed: bool,
}

impl Binder {
    /// Start over: keys already held when binding starts don't count.
    fn reset(&mut self) {
        self.prev = vec![true; KEYS.len()];
        self.lone = None;
        self.mixed = false;
    }

    fn scan(&mut self, down: &[bool]) -> Option<String> {
        if self.prev.len() != down.len() {
            self.reset();
        }
        let mut found = None;
        for (i, k) in KEYS.iter().enumerate() {
            let (was, is) = (self.prev[i], down[i]);
            if is && !was && found.is_none() {
                if modifier_bit(k.name).is_some() {
                    if self.lone.is_some() {
                        self.mixed = true;
                    } else {
                        self.lone = Some(i);
                    }
                } else {
                    let mods = KEYS.iter().zip(down).filter(|(_, d)| **d).filter_map(|(k, _)| modifier_bit(k.name)).fold(0, |a, b| a | b);
                    found = Some(Combo { mods, key: k }.name());
                }
            } else if !is && was && self.lone == Some(i) && found.is_none() {
                if !self.mixed {
                    found = Some(k.name.to_string());
                }
                self.lone = None;
                self.mixed = false;
            }
        }
        self.prev.copy_from_slice(down);
        found
    }
}

/// Per action: down in the window (fallback), down globally, and what was
/// last acted on.
#[derive(Default)]
struct KeyState {
    focused: [bool; 4],
    global: [bool; 4],
    level: [bool; 4],
    latch: Latch,
    /// When each toggle last fired (mute, deafen).
    toggled: [Option<Instant>; 4],
}

type OnToggle = Arc<dyn Fn(Action) + Send + Sync>;

/// The global keys: push-to-talk, mute, deafen and push-to-mute.
pub struct Ptt {
    gate: Arc<Gate>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    keys: Mutex<[Option<Combo>; 4]>,
    /// Push-to-talk can matter (its mode, in a call or a mic test).
    talk_active: AtomicBool,
    /// In a call: the mute, deafen and push-to-mute keys work.
    keys_active: AtomicBool,
    /// Settings is waiting for a key for this action.
    binding: Mutex<Option<Action>>,
    bound: Mutex<Option<(Action, String)>>,
    state: Mutex<KeyState>,
    started: AtomicBool,
    /// Mute / deafen pressed: the core toggles them.
    on_toggle: Mutex<Option<OnToggle>>,
    /// Human-readable backend status, for the settings screen.
    pub status: Mutex<String>,
}

impl Ptt {
    pub fn new(gate: Arc<Gate>, repaint: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            gate,
            repaint,
            keys: Mutex::new([Combo::parse("Backquote"), None, None, None]),
            talk_active: AtomicBool::new(false),
            keys_active: AtomicBool::new(false),
            binding: Mutex::new(None),
            bound: Mutex::new(None),
            state: Mutex::new(KeyState::default()),
            started: AtomicBool::new(false),
            on_toggle: Mutex::new(None),
            status: Mutex::new(String::new()),
        })
    }

    /// What the mute and deafen keys do (the core's toggles).
    pub fn set_on_toggle(&self, f: OnToggle) {
        *self.on_toggle.lock() = Some(f);
    }

    /// The four keys, by name ("" = none).
    pub fn set_keys(&self, talk: &str, mute: &str, deafen: &str, cough: &str) {
        let parse = |s: &str| if s.is_empty() { None } else { Combo::parse(s) };
        let keys = [parse(talk), parse(mute), parse(deafen), parse(cough)];
        let changed = *self.keys.lock() != keys;
        if changed {
            *self.keys.lock() = keys;
            // A key that changed while held mustn't stay held.
            let mut st = self.state.lock();
            st.focused = [false; 4];
            st.global = [false; 4];
            drop(st);
            self.update();
        }
    }

    /// Poll only while the keys can matter: push-to-talk (`talk`: its mode,
    /// in a call or a mic test) and the others (`keys`: in a call, and only
    /// when one is set, so nobody who never set one is asked for Input
    /// Monitoring on a Mac).
    pub fn set_active(self: &Arc<Self>, talk: bool, keys: bool) {
        self.talk_active.store(talk, Ordering::Relaxed);
        self.keys_active.store(keys, Ordering::Relaxed);
        if self.polling() {
            self.ensure_thread();
        }
        if !talk {
            let mut st = self.state.lock();
            st.focused[Action::Talk.index()] = false;
            st.latch.reset();
        }
        if !keys {
            self.state.lock().focused = [false; 4];
        }
        self.update();
    }

    fn polling(&self) -> bool {
        let keys = self.keys.lock();
        self.talk_active.load(Ordering::Relaxed) || (self.keys_active.load(Ordering::Relaxed) && keys[1..].iter().any(Option::is_some)) || self.binding.lock().is_some()
    }

    fn active(&self, a: Action) -> bool {
        match a {
            Action::Talk => self.talk_active.load(Ordering::Relaxed),
            _ => self.keys_active.load(Ordering::Relaxed),
        }
    }

    /// Start capturing the next pressed key (keyboard or mouse 3-5) for `action`.
    pub fn start_binding(self: &Arc<Self>, action: Action) {
        *self.bound.lock() = None;
        *self.binding.lock() = Some(action);
        self.ensure_thread();
    }

    pub fn cancel_binding(&self) {
        *self.binding.lock() = None;
    }

    pub fn binding(&self) -> Option<Action> {
        *self.binding.lock()
    }

    /// The key captured by `start_binding`, once.
    pub fn take_bound(&self) -> Option<(Action, String)> {
        self.bound.lock().take()
    }

    /// Binding fallback from the window's key events (when global polling can't see keys).
    pub fn offer_bound(&self, combo: &str) {
        if let Some(a) = self.binding.lock().take() {
            *self.bound.lock() = Some((a, combo.to_string()));
        }
    }

    /// In-window key events (fallback for Wayland / no permission): `name`
    /// went down (with these modifiers held) or up.
    pub fn window_key(&self, name: &str, pressed: bool, mods: eframe::egui::Modifiers) {
        let keys = *self.keys.lock();
        let mut changed = false;
        {
            let mut st = self.state.lock();
            for a in Action::ALL {
                let Some(c) = keys[a.index()] else { continue };
                if c.key.name != name {
                    continue;
                }
                let down = pressed && egui_matches(&c, name, mods);
                changed |= st.focused[a.index()] != down;
                st.focused[a.index()] = down;
            }
        }
        if changed {
            self.update();
        }
    }

    /// The window lost the focus: nothing is held in it any more.
    pub fn window_blurred(&self) {
        let any = std::mem::take(&mut self.state.lock().focused).iter().any(|d| *d);
        if any {
            self.update();
        }
    }

    /// What global keys can do here (the poller's own finding once it ran).
    pub fn describe(&self) -> String {
        let found = self.status.lock().clone();
        if !found.is_empty() {
            return found;
        }
        if cfg!(windows) {
            "Works everywhere, even when Yap isn't focused.".into()
        } else if cfg!(target_os = "macos") {
            "Outside the window this needs Input Monitoring permission (System Settings › Privacy & Security).".into()
        } else if std::fs::read_dir("/dev/input").map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("event") && std::fs::File::open(e.path()).is_ok())).unwrap_or(false) {
            "Works everywhere (reading /dev/input).".into()
        } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            "On Wayland, keys outside the Yap window need read access to /dev/input: run `sudo usermod -aG input $USER`, then log out and back in. (Or use `yap --remote`.)".into()
        } else {
            "Works everywhere (X11).".into()
        }
    }

    /// Work out what's held now and act on it: push-to-talk (with the latch)
    /// and push-to-mute go straight to the gate; mute and deafen toggle on
    /// the way down.
    fn update(&self) {
        let now = Instant::now();
        let keys = *self.keys.lock();
        let mut toggles = Vec::new();
        let (talk, cough, latched) = {
            let mut st = self.state.lock();
            for a in Action::ALL {
                let i = a.index();
                let level = keys[i].is_some() && self.active(a) && (st.focused[i] || st.global[i]);
                if level && !st.level[i] {
                    match a {
                        Action::Talk => st.latch.press(now),
                        Action::Mute | Action::Deafen if st.toggled[i].is_none_or(|t| now.duration_since(t) >= TOGGLE_DEBOUNCE) => {
                            st.toggled[i] = Some(now);
                            toggles.push(a);
                        }
                        _ => {}
                    }
                }
                st.level[i] = level;
            }
            if !self.active(Action::Talk) {
                st.latch.reset();
            }
            (st.level[0] || st.latch.on, st.level[3], st.latch.on)
        };
        let mut repaint = self.gate.ptt_down.swap(talk, Ordering::Relaxed) != talk;
        repaint |= self.gate.cough.swap(cough, Ordering::Relaxed) != cough;
        repaint |= self.gate.ptt_latched.swap(latched, Ordering::Relaxed) != latched;
        if !toggles.is_empty() {
            let f = self.on_toggle.lock().clone();
            if let Some(f) = f {
                for a in toggles {
                    f(a);
                }
            }
        }
        if repaint {
            (self.repaint)();
        }
    }

    fn ensure_thread(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::Relaxed) {
            return;
        }
        let me = self.clone();
        let _ = std::thread::Builder::new().name("yap-ptt".into()).spawn(move || me.poll_loop());
    }

    fn poll_loop(self: Arc<Self>) {
        let mut backend = backend::Backend::open();
        *self.status.lock() = backend.describe();
        let mut binder = Binder::default();
        binder.reset();
        let mut scan = vec![false; KEYS.len()];
        loop {
            let binding = self.binding.lock().is_some();
            if !self.polling() {
                let had = std::mem::take(&mut self.state.lock().global).iter().any(|d| *d);
                if had {
                    self.update();
                }
                std::thread::sleep(Duration::from_millis(150));
                binder.reset();
                continue;
            }
            backend.refresh();
            if binding {
                for (d, k) in scan.iter_mut().zip(KEYS) {
                    *d = backend.is_down(k);
                }
                if let Some(combo) = binder.scan(&scan) {
                    let mut b = self.binding.lock();
                    if let Some(a) = b.take() {
                        *self.bound.lock() = Some((a, combo));
                        (self.repaint)();
                    }
                }
            } else {
                binder.reset();
            }
            let keys = *self.keys.lock();
            let global: [bool; 4] = std::array::from_fn(|i| self.active(Action::ALL[i]) && keys[i].is_some_and(|c| c.down(|k| backend.is_down(k))));
            let changed = {
                let mut st = self.state.lock();
                let changed = st.global != global;
                st.global = global;
                changed
            };
            if changed {
                self.update();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}


#[cfg(target_os = "windows")]
mod backend {
    use super::KeyDef;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    pub struct Backend;
    impl Backend {
        pub fn open() -> Self {
            Backend
        }
        pub fn describe(&self) -> String {
            "Works everywhere, even when Yap isn't focused.".into()
        }
        pub fn refresh(&mut self) {}
        pub fn is_down(&self, k: &KeyDef) -> bool {
            // SAFETY: plain Win32 call with a virtual-key code.
            (unsafe { GetAsyncKeyState(k.vk as i32) } as u16 & 0x8000) != 0
        }
    }
}

#[cfg(target_os = "macos")]
mod backend {
    use super::KeyDef;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
        fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }
    const HID_SYSTEM_STATE: i32 = 1;

    pub struct Backend {
        allowed: bool,
    }
    impl Backend {
        pub fn open() -> Self {
            // SAFETY: CoreGraphics permission queries take no arguments.
            let allowed = unsafe { CGPreflightListenEventAccess() } || unsafe { CGRequestListenEventAccess() };
            Backend { allowed }
        }
        pub fn describe(&self) -> String {
            if self.allowed {
                "Works everywhere, even when Yap isn't focused.".into()
            } else {
                "Allow Yap under System Settings › Privacy & Security › Input Monitoring for push-to-talk outside the window.".into()
            }
        }
        pub fn refresh(&mut self) {}
        pub fn is_down(&self, k: &KeyDef) -> bool {
            if k.name.starts_with("Mouse") {
                // SAFETY: plain CoreGraphics state query.
                return unsafe { CGEventSourceButtonState(HID_SYSTEM_STATE, k.mac as u32) };
            }
            if k.mac == super::NONE16 {
                return false;
            }
            // SAFETY: plain CoreGraphics state query.
            unsafe { CGEventSourceKeyState(HID_SYSTEM_STATE, k.mac) }
        }
    }
}

#[cfg(target_os = "linux")]
mod backend {
    use super::KeyDef;
    use std::os::unix::io::AsRawFd;
    use std::time::Instant;

    const KEY_BYTES: usize = 96; // KEY_MAX 0x2ff -> 768 bits

    struct X11 {
        xlib: x11_dl::xlib::Xlib,
        display: *mut x11_dl::xlib::Display,
        keymap: [i8; 32],
        pointer_mask: u32,
    }

    pub struct Backend {
        evdev: Vec<std::fs::File>,
        evdev_bits: [u8; KEY_BYTES],
        last_scan: Instant,
        x11: Option<X11>,
        wayland: bool,
    }

    // SAFETY: the Display is only ever used from the PTT thread that opened it.
    unsafe impl Send for Backend {}

    fn eviocgkey(len: usize) -> libc::c_ulong {
        ((2u64 << 30) | ((len as u64) << 16) | (0x45u64 << 8) | 0x18) as libc::c_ulong
    }

    fn open_evdev() -> Vec<std::fs::File> {
        let mut out = Vec::new();
        if let Ok(dir) = std::fs::read_dir("/dev/input") {
            for e in dir.flatten() {
                let name = e.file_name();
                if name.to_string_lossy().starts_with("event")
                    && let Ok(f) = std::fs::File::open(e.path())
                {
                    out.push(f);
                }
            }
        }
        out
    }

    impl Backend {
        pub fn open() -> Self {
            let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
            let evdev = open_evdev();
            let x11 = if std::env::var_os("DISPLAY").is_some() {
                x11_dl::xlib::Xlib::open().ok().and_then(|xlib| {
                    // SAFETY: XOpenDisplay with NULL opens $DISPLAY; checked for null below.
                    let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
                    (!display.is_null()).then_some(X11 { xlib, display, keymap: [0; 32], pointer_mask: 0 })
                })
            } else {
                None
            };
            Self { evdev, evdev_bits: [0; KEY_BYTES], last_scan: Instant::now(), x11, wayland }
        }

        pub fn describe(&self) -> String {
            if !self.evdev.is_empty() {
                "Works everywhere (reading /dev/input).".into()
            } else if self.x11.is_some() && !self.wayland {
                "Works everywhere (X11).".into()
            } else if self.wayland {
                "On Wayland, push-to-talk outside the Yap window needs read access to /dev/input: run `sudo usermod -aG input $USER`, then log out and back in.".into()
            } else {
                "Only works while Yap is focused (no X11 display or /dev/input access).".into()
            }
        }

        pub fn refresh(&mut self) {
            if !self.evdev.is_empty() {
                // Pick up keyboards plugged in later.
                if self.last_scan.elapsed().as_secs() >= 5 {
                    self.evdev = open_evdev();
                    self.last_scan = Instant::now();
                }
                self.evdev_bits = [0; KEY_BYTES];
                let mut buf = [0u8; KEY_BYTES];
                for f in &self.evdev {
                    // SAFETY: EVIOCGKEY writes at most KEY_BYTES into buf.
                    let r = unsafe { libc::ioctl(f.as_raw_fd(), eviocgkey(KEY_BYTES) as _, buf.as_mut_ptr()) };
                    if r >= 0 {
                        for (a, b) in self.evdev_bits.iter_mut().zip(buf.iter()) {
                            *a |= *b;
                        }
                    }
                }
            } else if let Some(x) = self.x11.as_mut() {
                // SAFETY: display is valid for the life of the backend; keymap is 32 bytes.
                unsafe {
                    (x.xlib.XQueryKeymap)(x.display, x.keymap.as_mut_ptr());
                    let root = (x.xlib.XDefaultRootWindow)(x.display);
                    let (mut r, mut c) = (0, 0);
                    let (mut rx, mut ry, mut wx, mut wy) = (0, 0, 0, 0);
                    let mut mask = 0u32;
                    (x.xlib.XQueryPointer)(x.display, root, &mut r, &mut c, &mut rx, &mut ry, &mut wx, &mut wy, &mut mask);
                    x.pointer_mask = mask;
                }
            }
        }

        pub fn is_down(&self, k: &KeyDef) -> bool {
            if !self.evdev.is_empty() {
                let c = k.evdev as usize;
                return c / 8 < KEY_BYTES && self.evdev_bits[c / 8] & (1 << (c % 8)) != 0;
            }
            if let Some(x) = &self.x11 {
                if k.name.starts_with("Mouse") {
                    return k.keysym != 0 && x.pointer_mask & k.keysym != 0;
                }
                // SAFETY: display is valid.
                let kc = unsafe { (x.xlib.XKeysymToKeycode)(x.display, k.keysym as u64) } as usize;
                return kc != 0 && (x.keymap[kc / 8] as u8) & (1 << (kc % 8)) != 0;
            }
            false
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod backend {
    use super::KeyDef;
    pub struct Backend;
    impl Backend {
        pub fn open() -> Self {
            Backend
        }
        pub fn describe(&self) -> String {
            "Only works while Yap is focused.".into()
        }
        pub fn refresh(&mut self) {}
        pub fn is_down(&self, _k: &KeyDef) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_parse_and_print() {
        // Old configs: one key.
        assert_eq!(Combo::parse("Backquote").map(|c| c.name()).as_deref(), Some("Backquote"));
        assert_eq!(Combo::parse("ShiftLeft").map(|c| c.name()).as_deref(), Some("ShiftLeft"));
        // Modifiers in a fixed order, whatever order they came in.
        assert_eq!(Combo::parse("Shift+Ctrl+KeyM").map(|c| c.name()).as_deref(), Some("Ctrl+Shift+KeyM"));
        assert_eq!(Combo::parse("Meta+Alt+F13").map(|c| c.name()).as_deref(), Some("Alt+Meta+F13"));
        assert!(Combo::parse("").is_none());
        assert!(Combo::parse("Hyper+KeyM").is_none());
        assert!(Combo::parse("Ctrl+NotAKey").is_none());
        assert_eq!(key_label(""), "None");
        assert_eq!(key_label("Backquote"), "` (backtick)");
        assert_eq!(key_label("Ctrl+Shift+KeyM"), "Ctrl+Shift+M");
        if cfg!(target_os = "macos") {
            assert_eq!(key_label("Meta+Shift+KeyD"), "⌘+Shift+D");
        }
    }

    #[test]
    fn a_combo_is_held_with_either_side_and_extra_modifiers() {
        let c = Combo::parse("Ctrl+Shift+KeyM").unwrap();
        let held = |names: &'static [&'static str]| move |k: &KeyDef| names.contains(&k.name);
        assert!(c.down(held(&["ControlLeft", "ShiftRight", "KeyM"])));
        assert!(c.down(held(&["ControlRight", "ShiftLeft", "AltLeft", "KeyM"])), "an extra modifier doesn't stop it");
        assert!(!c.down(held(&["ControlLeft", "KeyM"])), "Shift is missing");
        assert!(!c.down(held(&["ControlLeft", "ShiftLeft"])), "M isn't down");
        let ptt = Combo::parse("Backquote").unwrap();
        assert!(ptt.down(held(&["ShiftLeft", "Backquote"])), "push-to-talk while running (Shift held)");
    }

    #[test]
    fn in_window_events_match_combos() {
        use eframe::egui::Modifiers;
        let c = Combo::parse("Ctrl+Shift+KeyM").unwrap();
        assert!(egui_matches(&c, "KeyM", Modifiers { ctrl: true, shift: true, ..Default::default() }));
        assert!(!egui_matches(&c, "KeyM", Modifiers { ctrl: true, ..Default::default() }));
        assert!(!egui_matches(&c, "KeyN", Modifiers { ctrl: true, shift: true, ..Default::default() }));
        assert_eq!(combo_from_egui(eframe::egui::Key::D, Modifiers { ctrl: true, shift: true, ..Default::default() }).as_deref(), Some("Ctrl+Shift+KeyD"));
        assert_eq!(combo_from_egui(eframe::egui::Key::F13, Modifiers::default()).as_deref(), Some("F13"));
    }

    fn scan_of(names: &[&str]) -> Vec<bool> {
        KEYS.iter().map(|k| names.contains(&k.name)).collect()
    }

    #[test]
    fn binding_takes_a_key_with_its_modifiers_or_a_modifier_alone() {
        let mut b = Binder::default();
        b.reset();
        // Something held when binding starts doesn't count.
        assert_eq!(b.scan(&scan_of(&["KeyA"])), None);
        assert_eq!(b.scan(&scan_of(&[])), None);
        // Ctrl, then Shift, then M: Ctrl+Shift+M at M's press.
        assert_eq!(b.scan(&scan_of(&["ControlLeft"])), None);
        assert_eq!(b.scan(&scan_of(&["ControlLeft", "ShiftLeft"])), None);
        assert_eq!(b.scan(&scan_of(&["ControlLeft", "ShiftLeft", "KeyM"])).as_deref(), Some("Ctrl+Shift+KeyM"));
        // A modifier on its own: bound when it's let go.
        let mut b = Binder::default();
        b.reset();
        b.scan(&scan_of(&[]));
        assert_eq!(b.scan(&scan_of(&["ShiftLeft"])), None);
        assert_eq!(b.scan(&scan_of(&[])).as_deref(), Some("ShiftLeft"));
        // Two modifiers let go without a key: nothing (ambiguous).
        let mut b = Binder::default();
        b.reset();
        b.scan(&scan_of(&[]));
        b.scan(&scan_of(&["ControlLeft"]));
        b.scan(&scan_of(&["ControlLeft", "AltLeft"]));
        assert_eq!(b.scan(&scan_of(&["ControlLeft"])), None);
        assert_eq!(b.scan(&scan_of(&[])), None);
        // A mouse button, plainly.
        assert_eq!(b.scan(&scan_of(&["Mouse4"])).as_deref(), Some("Mouse4"));
    }

    #[test]
    fn binding_refuses_a_key_thats_taken() {
        let mut c = crate::config::Config::default();
        assert!(bind(&mut c, Action::Mute, "Ctrl+Shift+KeyM").is_ok());
        assert_eq!(c.mute_key, "Ctrl+Shift+KeyM");
        let err = bind(&mut c, Action::Deafen, "Shift+Ctrl+KeyM").unwrap_err();
        assert!(err.ends_with("is already your mute key."), "{err}");
        assert!(bind(&mut c, Action::Cough, "Backquote").unwrap_err().ends_with("is already your push-to-talk key."));
        assert!(bind(&mut c, Action::Talk, "").is_err(), "push-to-talk keeps a key");
        assert!(bind(&mut c, Action::Mute, "").is_ok());
        assert_eq!(c.mute_key, "");
        assert!(bind(&mut c, Action::Deafen, "Ctrl+Shift+KeyM").is_ok(), "free again");
    }

    #[test]
    fn window_shortcuts_step_aside_for_a_global_key() {
        let mut c = crate::config::Config::default();
        assert!(shortcut_free(&c, MUTE_SHORTCUT) && shortcut_free(&c, DEAFEN_SHORTCUT));
        c.deafen_key = MUTE_SHORTCUT.into();
        assert!(!shortcut_free(&c, MUTE_SHORTCUT), "a global key on the same keys: the window's shortcut stands aside");
        assert!(shortcut_free(&c, DEAFEN_SHORTCUT));
    }

    #[test]
    fn double_tap_latches_and_the_next_press_lets_go() {
        let t0 = Instant::now();
        let ms = |v: u64| t0 + Duration::from_millis(v);
        let mut l = Latch::default();
        l.press(ms(0));
        assert!(!l.on, "one press is an ordinary push-to-talk");
        l.press(ms(300));
        assert!(l.on, "a second within 400 ms latches it");
        l.press(ms(2000));
        assert!(!l.on, "the next press unlatches");
        l.press(ms(2200));
        assert!(!l.on, "…and that press isn't the first of another double-tap");
        let mut slow = Latch::default();
        slow.press(ms(0));
        slow.press(ms(500));
        assert!(!slow.on, "500 ms apart: two ordinary presses");
        slow.press(ms(800));
        assert!(slow.on, "but the 2nd and 3rd were close enough");
    }

    /// The keys, end to end through the in-window path: push-to-talk holds,
    /// double-taps and lets go; mute and deafen toggle once per press, and
    /// only in a call; push-to-mute holds the gate's cough switch.
    #[test]
    fn keys_drive_the_gate_and_the_toggles() {
        use eframe::egui::Modifiers;
        let gate = Arc::new(crate::audio::Gate::new());
        let toggles = Arc::new(Mutex::new(Vec::new()));
        let p = Ptt::new(gate.clone(), Arc::new(|| {}));
        let t = toggles.clone();
        p.set_on_toggle(Arc::new(move |a| t.lock().push(a)));
        p.set_keys("Backquote", "Ctrl+Shift+KeyM", "F13", "KeyC");
        let none = Modifiers::default();
        let cs = Modifiers { ctrl: true, shift: true, ..Default::default() };
        // Not in a call: nothing happens.
        p.window_key("KeyM", true, cs);
        p.window_key("KeyM", false, cs);
        assert!(toggles.lock().is_empty());
        // In a call, push-to-talk mode.
        p.talk_active.store(true, Ordering::Relaxed);
        p.keys_active.store(true, Ordering::Relaxed);
        p.window_key("Backquote", true, none);
        assert!(gate.ptt_down.load(Ordering::Relaxed));
        p.window_key("Backquote", false, none);
        assert!(!gate.ptt_down.load(Ordering::Relaxed));
        p.window_key("Backquote", true, none);
        p.window_key("Backquote", false, none);
        assert!(gate.ptt_down.load(Ordering::Relaxed) && gate.ptt_latched.load(Ordering::Relaxed), "double-tapped: live");
        p.window_key("Backquote", true, none);
        p.window_key("Backquote", false, none);
        assert!(!gate.ptt_down.load(Ordering::Relaxed) && !gate.ptt_latched.load(Ordering::Relaxed), "tapped again: off");
        // Mute: Ctrl+Shift+M toggles once per press; M alone doesn't.
        p.window_key("KeyM", true, none);
        p.window_key("KeyM", false, none);
        p.window_key("KeyM", true, cs);
        p.window_key("KeyM", true, cs); // (a repeat)
        p.window_key("KeyM", false, cs);
        p.window_key("F13", true, none);
        p.window_key("F13", false, none);
        assert_eq!(*toggles.lock(), vec![Action::Mute, Action::Deafen]);
        // The same quick tap seen twice (the poller, then the window): one toggle.
        p.window_key("F13", true, none);
        p.window_key("F13", false, none);
        assert_eq!(*toggles.lock(), vec![Action::Mute, Action::Deafen], "within 250 ms: the same press");
        std::thread::sleep(Duration::from_millis(260));
        p.window_key("F13", true, none);
        p.window_key("F13", false, none);
        assert_eq!(*toggles.lock(), vec![Action::Mute, Action::Deafen, Action::Deafen], "a real second press");
        // Push-to-mute: held.
        p.window_key("KeyC", true, none);
        assert!(gate.cough.load(Ordering::Relaxed));
        p.window_key("KeyC", false, none);
        assert!(!gate.cough.load(Ordering::Relaxed));
        // Out of the call, a held key lets go.
        p.window_key("KeyC", true, none);
        p.talk_active.store(false, Ordering::Relaxed);
        p.keys_active.store(false, Ordering::Relaxed);
        p.update();
        assert!(!gate.cough.load(Ordering::Relaxed));
    }
}
