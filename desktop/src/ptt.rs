//! Global push-to-talk by polling key state (~100 Hz) — works while Yap is in
//! the background and sees both key down and key up, without hooks or
//! grabbing the key away from other apps.
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
use std::time::Duration;

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

pub fn key_label(name: &str) -> String {
    match name {
        "Backquote" => "` (backtick)".into(),
        "CapsLock" => "Caps Lock".into(),
        "ShiftLeft" => "Left Shift".into(),
        "ShiftRight" => "Right Shift".into(),
        "ControlLeft" => "Left Ctrl".into(),
        "ControlRight" => "Right Ctrl".into(),
        "AltLeft" => "Left Alt".into(),
        "AltRight" => "Right Alt".into(),
        "MetaLeft" => if cfg!(target_os = "macos") { "Left ⌘" } else { "Left Super" }.into(),
        "MetaRight" => if cfg!(target_os = "macos") { "Right ⌘" } else { "Right Super" }.into(),
        "Mouse3" => "Middle mouse".into(),
        "Mouse4" => "Mouse 4 (back)".into(),
        "Mouse5" => "Mouse 5 (forward)".into(),
        n => n.trim_start_matches("Key").trim_start_matches("Digit").to_string(),
    }
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

pub struct Ptt {
    gate: Arc<Gate>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    key: Mutex<String>,
    active: AtomicBool,
    binding: AtomicBool,
    bound: Mutex<Option<String>>,
    /// Pressed according to the window's own key events (fallback).
    focused_down: AtomicBool,
    global_down: AtomicBool,
    started: AtomicBool,
    /// Human-readable backend status, for the settings screen.
    pub status: Mutex<String>,
}

impl Ptt {
    pub fn new(gate: Arc<Gate>, repaint: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            gate,
            repaint,
            key: Mutex::new("Backquote".into()),
            active: AtomicBool::new(false),
            binding: AtomicBool::new(false),
            bound: Mutex::new(None),
            focused_down: AtomicBool::new(false),
            global_down: AtomicBool::new(false),
            started: AtomicBool::new(false),
            status: Mutex::new(String::new()),
        })
    }

    pub fn key(&self) -> String {
        self.key.lock().clone()
    }

    pub fn set_key(self: &Arc<Self>, name: &str) {
        *self.key.lock() = name.to_string();
    }

    /// Poll only while push-to-talk can matter (PTT mode and in voice).
    pub fn set_active(self: &Arc<Self>, on: bool) {
        self.active.store(on, Ordering::Relaxed);
        if on {
            self.ensure_thread();
        } else {
            self.focused_down.store(false, Ordering::Relaxed);
            self.update();
        }
    }

    /// Start capturing the next pressed key (keyboard or mouse 3-5).
    pub fn start_binding(self: &Arc<Self>) {
        *self.bound.lock() = None;
        self.binding.store(true, Ordering::Relaxed);
        self.ensure_thread();
    }

    pub fn cancel_binding(&self) {
        self.binding.store(false, Ordering::Relaxed);
    }

    pub fn is_binding(&self) -> bool {
        self.binding.load(Ordering::Relaxed)
    }

    /// The key captured by `start_binding`, once.
    pub fn take_bound(&self) -> Option<String> {
        self.bound.lock().take()
    }

    /// Binding fallback from the window's key events (when global polling can't see keys).
    pub fn offer_bound(&self, name: &str) {
        if self.binding.swap(false, Ordering::Relaxed) {
            *self.bound.lock() = Some(name.to_string());
        }
    }

    /// In-window key events (fallback for Wayland / no permission).
    pub fn set_focused_down(&self, down: bool) {
        if self.focused_down.swap(down, Ordering::Relaxed) != down {
            self.update();
        }
    }

    /// What global push-to-talk can do here (the poller's own finding once it ran).
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
            "On Wayland, push-to-talk outside the Yap window needs read access to /dev/input: run `sudo usermod -aG input $USER`, then log out and back in.".into()
        } else {
            "Works everywhere (X11).".into()
        }
    }

    fn update(&self) {
        let down = self.active.load(Ordering::Relaxed)
            && (self.focused_down.load(Ordering::Relaxed) || self.global_down.load(Ordering::Relaxed));
        if self.gate.ptt_down.swap(down, Ordering::Relaxed) != down {
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
        let mut prev_scan: Vec<bool> = vec![true; KEYS.len()]; // treat everything as held at start
        loop {
            let active = self.active.load(Ordering::Relaxed);
            let binding = self.binding.load(Ordering::Relaxed);
            if !active && !binding {
                self.global_down.store(false, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(150));
                prev_scan.iter_mut().for_each(|x| *x = true);
                continue;
            }
            backend.refresh();
            if binding {
                for (i, k) in KEYS.iter().enumerate() {
                    let down = backend.is_down(k);
                    if down && !prev_scan[i] && self.binding.swap(false, Ordering::Relaxed) {
                        *self.bound.lock() = Some(k.name.to_string());
                        (self.repaint)();
                    }
                    prev_scan[i] = down;
                }
            } else {
                prev_scan.iter_mut().for_each(|x| *x = true);
            }
            if active {
                let key = self.key.lock().clone();
                let down = key_def(&key).is_some_and(|k| backend.is_down(k));
                self.global_down.store(down, Ordering::Relaxed);
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
