//! What can be shared, and capturing it: one backend per OS behind the same
//! few functions (`sources`, `start`, `sound_supported`), plus a synthetic
//! test pattern for tests and screenshots (never the real screen there).
//!
//! - macOS: ScreenCaptureKit (12.3+; sound without Yap's own on 13+), `mac.rs`. On 14+
//!   Apple's own picker chooses what's shared (`system_picker`, `pick`), and the picked
//!   thing's stream is held for the whole share (`hold` / `release`).
//! - Windows: Windows.Graphics.Capture (10 1903+), sound through process
//!   loopback that leaves Yap out (10 2004+), `windows.rs`.
//! - Linux: X11 through `x11-dl` (MIT-SHM or XGetImage); no sound (nothing
//!   can leave Yap's own audio out of a monitor source); Wayland isn't
//!   supported yet, `x11.rs`.

pub mod pattern;

#[cfg(target_os = "macos")]
mod mac;
// The Windows backend's pure helpers also build in tests elsewhere, so they run on a Mac.
// (`self::` because a bare `windows::` would be ambiguous with the `windows` crate.)
#[cfg(any(windows, test))]
#[cfg_attr(not(windows), allow(dead_code))]
mod windows;
#[cfg(any(target_os = "linux", test))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod x11;

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Screen,
    Window,
}

#[derive(Clone, Debug)]
pub struct Source {
    /// Opaque, stable while the thing exists (a display id / HMONITOR / XID, a window id / HWND).
    pub id: String,
    /// What the picker shows: "Screen 1 (2560 × 1440)", or a window's title + app name.
    pub name: String,
    pub kind: SourceKind,
}

/// One captured picture: BGRA8 (B, G, R, A byte order), top-down rows, `stride` bytes per row
/// (stride >= width * 4). Alpha is ignored.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Frames no bigger than this (aspect kept, never upscaled) where the API
    /// scales for free (macOS); otherwise native size and the encoder scales.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub max_width: u32,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub max_height: u32,
    /// At most this many frames per second.
    pub fps: u32,
    /// The computer's sound too (only if `sound_supported()`), without Yap's own audio.
    pub sound: bool,
}

/// Sound: 48 kHz, stereo, interleaved f32 samples (L R L R …), any chunk length.
pub type OnFrame = Box<dyn FnMut(Frame) + Send + 'static>;
pub type OnAudio = Box<dyn FnMut(&[f32]) + Send + 'static>;
/// Called once if the capture ends by itself (window closed, permission revoked, display gone).
pub type OnEnded = Box<dyn FnOnce(String) + Send + 'static>;
/// What the system picker hands back: what was picked, None if it was cancelled, or why it
/// couldn't open. Called once, on any thread.
pub type OnPicked = Box<dyn FnOnce(Result<Option<Source>, String>) + Send + 'static>;

pub trait Capture: Send {
    /// Change size/fps (a quality change while sharing) without stopping.
    fn set_options(&mut self, options: Options);
    /// Stop capturing and release everything (idempotent).
    fn stop(&mut self);
}

/// The test pattern is offered (headless `--share-screen`, or debug builds
/// with `YAP_DEBUG_SCREEN=pattern`).
static PATTERN: AtomicBool = AtomicBool::new(false);

pub fn offer_pattern(on: bool) {
    PATTERN.store(on, Ordering::Relaxed);
}

fn pattern_offered() -> bool {
    PATTERN.load(Ordering::Relaxed) || cfg!(debug_assertions) && std::env::var("YAP_DEBUG_SCREEN").as_deref() == Ok("pattern")
}

/// What can be shared: screens first, then windows. With the test pattern on,
/// it's the only thing offered (tests and screenshots never touch the real screen).
pub fn sources() -> Result<Vec<Source>, String> {
    if pattern_offered() {
        return Ok(vec![pattern::source()]);
    }
    platform::sources()
}

pub fn start(source: &Source, options: Options, on_frame: OnFrame, on_audio: Option<OnAudio>, on_ended: OnEnded) -> Result<Box<dyn Capture>, String> {
    if source.id == pattern::ID {
        return Ok(pattern::start(options, on_frame, on_audio));
    }
    platform::start(source, options, on_frame, on_audio, on_ended)
}

/// Can sharing take the computer's sound along (without Yap's own)?
pub fn sound_supported() -> bool {
    pattern_offered() || platform::sound_supported()
}

/// macOS 14+: what's shared is picked in Apple's own picker (`pick`), not from `sources`
/// (listing them ourselves is what makes macOS 15+ keep asking "allow Yap to bypass…?").
/// Never with the test pattern on.
pub fn system_picker() -> bool {
    !pattern_offered() && platform::system_picker()
}

/// Opens the system picker for a new share; `on_picked` hears what was picked.
pub fn pick(on_picked: OnPicked) {
    platform::pick(on_picked)
}

/// Share something else with the system picker: it changes what the running share shows
/// without stopping. False if there's nothing to change (then `pick` a new source).
pub fn pick_again() -> bool {
    platform::pick_again()
}

/// A share of `source` starts (or its options changed): a picked source's stream runs from
/// now until `release` (idle while nobody watches); `on_ended` hears it end by itself.
/// Nothing to do for other sources: their capture starts and stops with each viewer.
pub fn hold(source: &Source, options: Options, on_ended: OnEnded) {
    platform::hold(source, options, on_ended)
}

/// The share of `source` is over: a picked source's stream stops.
pub fn release(source: &Source) {
    platform::release(source)
}

#[cfg(target_os = "macos")]
mod platform {
    pub use super::mac::{hold, pick, pick_again, release, sound_supported, sources, start, system_picker};
}

#[cfg(windows)]
mod platform {
    pub use super::windows::{sound_supported, sources, start};
    pub use super::no_picker::*;
}

#[cfg(target_os = "linux")]
mod platform {
    pub use super::x11::{sound_supported, sources, start};
    pub use super::no_picker::*;
}

/// Windows and Linux: Yap's own list, and nothing held between viewers.
#[cfg(not(target_os = "macos"))]
mod no_picker {
    use super::*;
    pub fn system_picker() -> bool {
        false
    }
    pub fn pick(on_picked: OnPicked) {
        on_picked(Err("There's no system screen picker here.".into()))
    }
    pub fn pick_again() -> bool {
        false
    }
    pub fn hold(_: &Source, _: Options, _: OnEnded) {}
    pub fn release(_: &Source) {}
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    use super::*;
    pub use super::no_picker::*;
    pub fn sources() -> Result<Vec<Source>, String> {
        Err("Screen sharing isn't available on this system.".into())
    }
    pub fn start(_: &Source, _: Options, _: OnFrame, _: Option<OnAudio>, _: OnEnded) -> Result<Box<dyn Capture>, String> {
        Err("Screen sharing isn't available on this system.".into())
    }
    pub fn sound_supported() -> bool {
        false
    }
}
