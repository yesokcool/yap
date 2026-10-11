//! Screen capture on Linux through X11. Xlib and its extensions are loaded at run time
//! (x11-dl), so Yap still starts on machines that don't have them.
//!
//! - Screens are rectangles of the root window, one per RandR monitor.
//! - Windows come from the window manager's client list and are captured directly.
//! - Pixels arrive over MIT-SHM on local displays and through plain XGetImage otherwise
//!   (remote displays such as `ssh -X`).
//! - Wayland isn't supported: XWayland only sees X11 apps and its root window is black.

use super::*;

use std::ffi::CStr;
use std::os::raw::{c_int, c_long, c_uchar, c_uint, c_ulong};
use std::ptr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use x11_dl::xfixes::Xlib as XFixes; // x11-dl names the XFixes function table `Xlib`.
use x11_dl::xlib::{self, Atom, Display, Drawable, Visual, Window, XErrorEvent, XImage, XWindowAttributes, Xlib};
use x11_dl::xrandr::Xrandr;
use x11_dl::xshm::{XShmSegmentInfo, Xext};

const WAYLAND: &str = "Screen sharing on Wayland isn't supported yet.";
const NO_X11: &str = "Screen sharing needs an X11 desktop.";
const NO_CONNECTION: &str = "Yap couldn't connect to the X11 desktop.";
const UNKNOWN_SOURCE: &str = "That screen or window isn't available anymore.";
const NO_THREAD: &str = "Yap couldn't start screen sharing.";
const WINDOW_CLOSED: &str = "The window was closed.";
const SCREEN_GONE: &str = "The screen was disconnected.";
const ODD_COLORS: &str = "This screen uses a color format Yap can't share.";

/// How long start() waits for the first picture before returning anyway.
const START_WAIT: Duration = Duration::from_millis(400);
const ALL_PLANES: c_ulong = !0;
/// Longest property we read, in 32-bit units (titles, the client list).
const MAX_PROPERTY: c_long = 1 << 16;

// ---------------------------------------------------------------------------------------------
// Entry points

pub fn sources() -> Result<Vec<Source>, String> {
    let conn = Conn::open(load_xlib()?)?;
    let mut sources = screen_sources(conn.monitors());
    sources.extend(conn.windows());
    Ok(sources)
}

pub fn start(
    source: &Source,
    options: Options,
    on_frame: OnFrame,
    _on_audio: Option<OnAudio>,
    on_ended: OnEnded,
) -> Result<Box<dyn Capture>, String> {
    let target = parse_target(&source.id).ok_or(UNKNOWN_SOURCE)?;
    let xlib = load_xlib()?;
    let shared = Arc::new(Shared::new(options.fps));
    let setup = Arc::new(Setup::default());
    let thread = std::thread::Builder::new()
        .name("yap-screen-x11".into())
        .spawn({
            let (shared, setup) = (shared.clone(), setup.clone());
            move || run(xlib, target, &shared, &setup, on_frame, on_ended)
        })
        .map_err(|_| NO_THREAD.to_string())?;
    if let Err(message) = setup.wait() {
        let _ = thread.join();
        return Err(message);
    }
    Ok(Box::new(X11Capture { shared, thread: Some(thread) }))
}

/// Always false: PulseAudio/PipeWire can't reliably leave Yap's own audio out of a monitor
/// source, so sharing sound would send the call back into the call.
pub fn sound_supported() -> bool {
    false
}

// ---------------------------------------------------------------------------------------------
// Session checks

fn is_wayland(session_type: Option<&str>, wayland_display: Option<&str>) -> bool {
    session_type.is_some_and(|t| t.trim().eq_ignore_ascii_case("wayland"))
        || wayland_display.is_some_and(|d| !d.is_empty())
}

fn env(name: &str) -> Option<String> {
    std::env::var_os(name).map(|v| v.to_string_lossy().into_owned())
}

/// Loads Xlib once we know this is an X11 session. On Wayland we refuse even when DISPLAY is
/// set: XWayland can't see other apps' windows and its root window is black.
fn load_xlib() -> Result<Xlib, String> {
    if is_wayland(env("XDG_SESSION_TYPE").as_deref(), env("WAYLAND_DISPLAY").as_deref()) {
        return Err(WAYLAND.into());
    }
    if env("DISPLAY").is_none_or(|d| d.is_empty()) {
        return Err(NO_X11.into());
    }
    Xlib::open().map_err(|_| NO_X11.to_string())
}

// ---------------------------------------------------------------------------------------------
// X errors
//
// Xlib's default error handler exits the process, and the handler is process-wide. We install
// ours once and keep it: errors on our own connections are recorded for the caller to check,
// everything else (eframe's connection, push-to-talk's) goes to whatever handler was there
// before, so those keep behaving exactly as they did. Our code also checks every return value,
// so it stays correct if something installs another handler after us.

type ErrorHandler = unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int;

struct Trap {
    id: u64,
    display: usize,
    failed: bool,
}

static TRAPS: Mutex<Vec<Trap>> = Mutex::new(Vec::new());
static NEXT_TRAP: AtomicU64 = AtomicU64::new(1);
static PREVIOUS_HANDLER: OnceLock<Option<ErrorHandler>> = OnceLock::new();

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn install_error_handler(xlib: &Xlib) {
    PREVIOUS_HANDLER.get_or_init(|| {
        // SAFETY: swaps a process-wide function pointer; Xlib returns the old one.
        unsafe { (xlib.XSetErrorHandler)(Some(on_x_error)) }
    });
}

unsafe extern "C" fn on_x_error(display: *mut Display, event: *mut XErrorEvent) -> c_int {
    {
        let mut traps = lock(&TRAPS);
        // Newest first: a closed connection's address can be reused by a new one.
        if let Some(trap) = traps.iter_mut().rev().find(|t| t.display == display as usize) {
            trap.failed = true;
            return 0;
        }
    }
    match PREVIOUS_HANDLER.get() {
        // SAFETY: hands the error to the handler Xlib had before ours, with the same arguments.
        Some(Some(previous)) => unsafe { previous(display, event) },
        _ => 0,
    }
}

// ---------------------------------------------------------------------------------------------
// One connection

/// One X connection. Xlib connections never cross threads: sources() and every capture thread
/// open their own. (eframe's winit already calls XInitThreads at startup, and libX11 1.8+ does
/// it by itself.)
struct Conn {
    xlib: Xlib,
    display: *mut Display,
    trap: u64,
}

impl Conn {
    fn open(xlib: Xlib) -> Result<Conn, String> {
        install_error_handler(&xlib);
        // SAFETY: NULL opens $DISPLAY; a null result is handled below.
        let display = unsafe { (xlib.XOpenDisplay)(ptr::null()) };
        if display.is_null() {
            return Err(NO_CONNECTION.into());
        }
        let trap = NEXT_TRAP.fetch_add(1, Ordering::Relaxed);
        lock(&TRAPS).push(Trap { id: trap, display: display as usize, failed: false });
        Ok(Conn { xlib, display, trap })
    }

    fn take_error(&self) -> bool {
        lock(&TRAPS).iter_mut().find(|t| t.id == self.trap).is_some_and(|t| std::mem::take(&mut t.failed))
    }

    /// Runs one Xlib call that waits for the server's reply and says whether the server
    /// answered it with an error.
    fn trapped<T>(&self, call: impl FnOnce() -> T) -> (T, bool) {
        self.take_error();
        let result = call();
        (result, self.take_error())
    }

    /// Waits until the server has handled everything sent so far.
    fn sync(&self) {
        // SAFETY: valid connection.
        unsafe { (self.xlib.XSync)(self.display, xlib::False) };
    }

    fn root(&self) -> Window {
        // SAFETY: valid connection.
        unsafe { (self.xlib.XDefaultRootWindow)(self.display) }
    }

    fn atom(&self, name: &CStr) -> Atom {
        // SAFETY: valid connection and NUL-terminated name.
        unsafe { (self.xlib.XInternAtom)(self.display, name.as_ptr(), xlib::False) }
    }

    /// None when the window doesn't exist (anymore).
    fn attributes(&self, window: Window) -> Option<XWindowAttributes> {
        // SAFETY: a C struct of numbers and pointers; all zeros is a valid value.
        let mut attributes: XWindowAttributes = unsafe { std::mem::zeroed() };
        // SAFETY: valid connection and out-pointer.
        let (status, failed) =
            self.trapped(|| unsafe { (self.xlib.XGetWindowAttributes)(self.display, window, &mut attributes) });
        (status != 0 && !failed).then_some(attributes)
    }

    /// Where the window's top-left corner is on the root window.
    fn root_position(&self, window: Window) -> Option<(i32, i32)> {
        let (mut x, mut y, mut child) = (0, 0, 0);
        let root = self.root();
        // SAFETY: valid connection and out-pointers.
        let (ok, failed) = self.trapped(|| unsafe {
            (self.xlib.XTranslateCoordinates)(self.display, window, root, 0, 0, &mut x, &mut y, &mut child)
        });
        (ok != 0 && !failed).then_some((x, y))
    }

    fn property(&self, window: Window, name: Atom, kind: Atom) -> Option<Property<'_>> {
        let (mut actual, mut format, mut items, mut after) = (0, 0, 0, 0);
        let mut data: *mut c_uchar = ptr::null_mut();
        // SAFETY: valid connection and out-pointers; Xlib allocates `data` and Property frees it.
        let (status, failed) = self.trapped(|| unsafe {
            (self.xlib.XGetWindowProperty)(
                self.display,
                window,
                name,
                0,
                MAX_PROPERTY,
                xlib::False,
                kind,
                &mut actual,
                &mut format,
                &mut items,
                &mut after,
                &mut data,
            )
        });
        if data.is_null() {
            return None;
        }
        let property = Property { conn: self, kind: actual, format, items: items as usize, data };
        (status == c_int::from(xlib::Success) && !failed).then_some(property)
    }

    fn monitors(&self) -> Vec<Monitor> {
        let monitors = self.randr_monitors();
        if !monitors.is_empty() {
            return monitors;
        }
        // No RandR 1.5: the whole root window is one screen.
        self.attributes(self.root())
            .map(|root| vec![Monitor { rect: Rect::sized(root.width, root.height), primary: true }])
            .unwrap_or_default()
    }

    fn randr_monitors(&self) -> Vec<Monitor> {
        let Ok(randr) = Xrandr::open() else { return Vec::new() };
        let (mut event_base, mut error_base) = (0, 0);
        // SAFETY: valid connection and out-pointers. Asking first avoids Xlib's "missing
        // extension" message on stderr.
        if unsafe { (randr.XRRQueryExtension)(self.display, &mut event_base, &mut error_base) } == 0 {
            return Vec::new();
        }
        let mut count = 0;
        let root = self.root();
        // SAFETY: valid connection and out-pointer; the list is freed below.
        let (list, _) = self.trapped(|| unsafe { (randr.XRRGetMonitors)(self.display, root, xlib::True, &mut count) });
        if list.is_null() {
            return Vec::new();
        }
        // SAFETY: RandR returned `count` monitors at `list`.
        let monitors = unsafe { std::slice::from_raw_parts(list, count.max(0) as usize) }
            .iter()
            .filter(|m| m.width > 0 && m.height > 0)
            .map(|m| Monitor {
                rect: Rect { x: m.x, y: m.y, width: m.width as u32, height: m.height as u32 },
                primary: m.primary != 0,
            })
            .collect();
        // SAFETY: `list` came from XRRGetMonitors and isn't used after this.
        unsafe { (randr.XRRFreeMonitors)(list) };
        monitors
    }

    fn windows(&self) -> Vec<Source> {
        let atoms = Atoms {
            net_wm_name: self.atom(c"_NET_WM_NAME"),
            utf8_string: self.atom(c"UTF8_STRING"),
            pid: self.atom(c"_NET_WM_PID"),
            window_type: self.atom(c"_NET_WM_WINDOW_TYPE"),
            dock: self.atom(c"_NET_WM_WINDOW_TYPE_DOCK"),
            desktop: self.atom(c"_NET_WM_WINDOW_TYPE_DESKTOP"),
        };
        // No EWMH window manager means no list; screens still work.
        let Some(list) = self.property(self.root(), self.atom(c"_NET_CLIENT_LIST"), xlib::XA_WINDOW) else {
            return Vec::new();
        };
        list.longs().iter().filter_map(|&window| self.window_source(window, &atoms)).collect()
    }

    fn window_source(&self, window: Window, atoms: &Atoms) -> Option<Source> {
        // None also when the window went away since the list was read.
        let attributes = self.attributes(window)?;
        // Unmapped = minimised or on another workspace.
        if attributes.map_state != xlib::IsViewable || attributes.width <= 0 || attributes.height <= 0 {
            return None;
        }
        if let Some(pid) = self.property(window, atoms.pid, xlib::XA_CARDINAL)
            && pid.longs().first() == Some(&c_ulong::from(std::process::id()))
        {
            return None;
        }
        if let Some(types) = self.property(window, atoms.window_type, xlib::XA_ATOM)
            && types.longs().iter().any(|&t| t == atoms.dock || t == atoms.desktop)
        {
            return None;
        }
        let title = self.title(window, atoms)?;
        let class = self.property(window, xlib::XA_WM_CLASS, xlib::XA_STRING).and_then(|p| class_name(p.bytes()));
        Some(Source {
            id: format!("w:{window}"),
            name: window_name(&title, class.as_deref()),
            kind: SourceKind::Window,
            width: attributes.width as u32,
            height: attributes.height as u32,
        })
    }

    /// _NET_WM_NAME (UTF-8), else the old WM_NAME. None when both are empty.
    fn title(&self, window: Window, atoms: &Atoms) -> Option<String> {
        let modern = self
            .property(window, atoms.net_wm_name, atoms.utf8_string)
            .map(|p| String::from_utf8_lossy(p.bytes()).into_owned());
        let title = modern.filter(|t| !clean_title(t).is_empty()).or_else(|| {
            let old = self.property(window, xlib::XA_WM_NAME, xlib::AnyPropertyType as Atom)?;
            Some(decode_text(old.kind, old.bytes()))
        })?;
        let title = clean_title(&title);
        (!title.is_empty()).then(|| title.to_string())
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        // SAFETY: the connection is ours and nothing uses it after this.
        unsafe { (self.xlib.XCloseDisplay)(self.display) };
        // Unregister only after closing: closing can still report errors.
        lock(&TRAPS).retain(|t| t.id != self.trap);
    }
}

struct Atoms {
    net_wm_name: Atom,
    utf8_string: Atom,
    pid: Atom,
    window_type: Atom,
    dock: Atom,
    desktop: Atom,
}

/// A window property as Xlib returned it; freed on drop.
struct Property<'a> {
    conn: &'a Conn,
    kind: Atom,
    format: c_int,
    items: usize,
    data: *mut c_uchar,
}

impl Property<'_> {
    fn bytes(&self) -> &[u8] {
        if self.format != 8 {
            return &[];
        }
        // SAFETY: 8-bit data is `items` bytes and lives until drop.
        unsafe { std::slice::from_raw_parts(self.data, self.items) }
    }

    /// 32-bit data, which Xlib hands over as one C long per item.
    fn longs(&self) -> &[c_ulong] {
        if self.format != 32 {
            return &[];
        }
        // SAFETY: 32-bit data is `items` C longs (malloc-aligned) and lives until drop.
        unsafe { std::slice::from_raw_parts(self.data.cast::<c_ulong>(), self.items) }
    }
}

impl Drop for Property<'_> {
    fn drop(&mut self) {
        // SAFETY: Xlib allocated `data` for us and no borrow of it outlives self.
        unsafe { (self.conn.xlib.XFree)(self.data.cast()) };
    }
}

// ---------------------------------------------------------------------------------------------
// Sources

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl Rect {
    fn sized(width: c_int, height: c_int) -> Rect {
        Rect { x: 0, y: 0, width: width.max(0) as u32, height: height.max(0) as u32 }
    }

    fn intersect(self, other: Rect) -> Option<Rect> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (i64::from(self.x) + i64::from(self.width)).min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height)).min(i64::from(other.y) + i64::from(other.height));
        (right > i64::from(left) && bottom > i64::from(top)).then(|| Rect {
            x: left,
            y: top,
            width: (right - i64::from(left)) as u32,
            height: (bottom - i64::from(top)) as u32,
        })
    }

    fn fits_in(self, outer: Rect) -> bool {
        self.intersect(outer) == Some(self)
    }
}

struct Monitor {
    rect: Rect,
    primary: bool,
}

/// Primary first, the rest left to right: "Screen 1 (2560 × 1440)", "Screen 2 (…)".
fn screen_sources(mut monitors: Vec<Monitor>) -> Vec<Source> {
    monitors.sort_by_key(|m| (!m.primary, m.rect.x, m.rect.y));
    monitors
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let Rect { x, y, width, height } = m.rect;
            Source {
                id: format!("m:{x},{y},{width},{height}"),
                name: format!("Screen {} ({width} × {height})", i + 1),
                kind: SourceKind::Screen,
                width,
                height,
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// A rectangle of the root window (a monitor, or the whole screen).
    Area(Rect),
    Window(Window),
}

fn parse_target(id: &str) -> Option<Target> {
    if let Some(rest) = id.strip_prefix("m:") {
        let parts: Vec<&str> = rest.split(',').map(str::trim).collect();
        let [x, y, width, height] = parts[..] else { return None };
        let rect =
            Rect { x: x.parse().ok()?, y: y.parse().ok()?, width: width.parse().ok()?, height: height.parse().ok()? };
        (rect.width > 0 && rect.height > 0).then_some(Target::Area(rect))
    } else {
        let window: Window = id.strip_prefix("w:")?.trim().parse().ok()?;
        (window != 0).then_some(Target::Window(window))
    }
}

/// "Title — Class", unless the title already says which app it is.
fn window_name(title: &str, class: Option<&str>) -> String {
    let title = title.trim();
    match class.map(str::trim).filter(|c| !c.is_empty()) {
        Some(class) if !title.to_lowercase().contains(&class.to_lowercase()) => format!("{title} — {class}"),
        _ => title.to_string(),
    }
}

/// WM_CLASS is "instance\0Class\0"; the class is the nicer name.
fn class_name(bytes: &[u8]) -> Option<String> {
    let mut parts = bytes.split(|&b| b == 0).map(|p| String::from_utf8_lossy(p).trim().to_string());
    let instance = parts.next().unwrap_or_default();
    let class = parts.next().unwrap_or_default();
    [class, instance].into_iter().find(|s| !s.is_empty())
}

/// WM_NAME is usually Latin-1 (STRING); anything else is read as UTF-8 as well as it goes.
fn decode_text(kind: Atom, bytes: &[u8]) -> String {
    if kind == xlib::XA_STRING {
        bytes.iter().map(|&b| char::from(b)).collect()
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn clean_title(title: &str) -> &str {
    title.trim_matches(|c: char| c.is_whitespace() || c == '\0')
}

// ---------------------------------------------------------------------------------------------
// Capture thread

struct X11Capture {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Capture for X11Capture {
    fn set_options(&mut self, options: Options) {
        // No scaling here: frames come at native size and the caller scales.
        self.shared.set_fps(options.fps);
    }

    fn stop(&mut self) {
        self.shared.stop();
        if let Some(thread) = self.thread.take() {
            // Called from inside on_frame/on_ended means we're on the capture thread: it ends
            // by itself right after, and joining itself would hang.
            if thread.thread().id() != std::thread::current().id() {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for X11Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Shared {
    fps: AtomicU32,
    stopped: Mutex<bool>,
    wake: Condvar,
}

impl Shared {
    fn new(fps: u32) -> Shared {
        let shared = Shared { fps: AtomicU32::new(1), stopped: Mutex::new(false), wake: Condvar::new() };
        shared.set_fps(fps);
        shared
    }

    fn set_fps(&self, fps: u32) {
        self.fps.store(fps.clamp(1, 120), Ordering::Relaxed);
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(1) / self.fps.load(Ordering::Relaxed).max(1)
    }

    fn stop(&self) {
        *lock(&self.stopped) = true;
        self.wake.notify_all();
    }

    fn stopped(&self) -> bool {
        *lock(&self.stopped)
    }

    /// Sleeps until `deadline`; true means stop() was called.
    fn sleep_until(&self, deadline: Instant) -> bool {
        let timeout = deadline.saturating_duration_since(Instant::now());
        let stopped = lock(&self.stopped);
        let (stopped, _) = self.wake.wait_timeout_while(stopped, timeout, |s| !*s).unwrap_or_else(|e| e.into_inner());
        *stopped
    }
}

/// Decides who reports a failed setup: start() as an Err while it's still waiting, the thread
/// through on_ended once start() has given up waiting and returned.
#[derive(Default)]
struct Setup {
    state: Mutex<SetupState>,
    changed: Condvar,
}

#[derive(Default)]
enum SetupState {
    #[default]
    Waiting,
    Ready,
    Failed(String),
    Abandoned,
}

impl Setup {
    /// start()'s side: Err only if setup failed in time.
    fn wait(&self) -> Result<(), String> {
        let state = lock(&self.state);
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, START_WAIT, |s| matches!(s, SetupState::Waiting))
            .unwrap_or_else(|e| e.into_inner());
        match std::mem::replace(&mut *state, SetupState::Abandoned) {
            SetupState::Failed(message) => Err(message),
            _ => Ok(()),
        }
    }

    /// The thread's side: returns the message for on_ended if start() already returned.
    fn report(&self, result: Result<(), String>) -> Option<String> {
        let mut state = lock(&self.state);
        if matches!(*state, SetupState::Abandoned) {
            return result.err();
        }
        *state = match result {
            Ok(()) => SetupState::Ready,
            Err(message) => SetupState::Failed(message),
        };
        self.changed.notify_all();
        None
    }
}

fn run(xlib: Xlib, target: Target, shared: &Shared, setup: &Setup, mut on_frame: OnFrame, on_ended: OnEnded) {
    // Connect and take one picture before telling start() it worked, so a closed window or an
    // unsupported screen comes back from start() as an Err.
    let opened = Grabber::new(xlib, target).and_then(|mut grabber| match grabber.grab() {
        Grab::Ended(message) => Err(message),
        first => Ok((grabber, first)),
    });
    let (mut grabber, mut grab) = match opened {
        Ok(opened) => {
            setup.report(Ok(()));
            opened
        }
        Err(message) => {
            if let Some(message) = setup.report(Err(message)) {
                on_ended(message);
            }
            return;
        }
    };
    let mut next = Instant::now();
    loop {
        match grab {
            Grab::Frame(frame) => on_frame(frame),
            // Nothing new this time; the caller repeats the last frame.
            Grab::Skip => {}
            Grab::Ended(message) => {
                if !shared.stopped() {
                    on_ended(message);
                }
                return;
            }
        }
        // A steady deadline, but no burst of frames to catch up after a slow one.
        next = (next + shared.interval()).max(Instant::now());
        if shared.sleep_until(next) {
            return;
        }
        grab = grabber.grab();
    }
    // Dropping the grabber frees the shared memory and closes the connection.
}

// ---------------------------------------------------------------------------------------------
// Grabbing pictures

enum Grab {
    Frame(Frame),
    Skip,
    Ended(String),
}

enum Failure {
    /// The server said no (off-screen, unmapped, gone); usually passes.
    Refused,
    /// A pixel format we can't convert.
    Unsupported,
}

/// A shared-memory XImage that XShmGetImage fills in place.
struct ShmImage {
    image: *mut XImage,
    /// Boxed because the XImage keeps a pointer to it.
    segment: Box<XShmSegmentInfo>,
    attached: bool,
    width: u32,
    height: u32,
    visual: *mut Visual,
    depth: c_int,
}

impl ShmImage {
    fn free(mut self, conn: &Conn, ext: &Xext) {
        if self.attached {
            // SAFETY: the server attached this segment on this connection.
            unsafe { (ext.XShmDetach)(conn.display, &mut *self.segment) };
            conn.sync();
        }
        if !self.segment.shmaddr.is_null() {
            // SAFETY: we mapped this address with shmat and nothing reads it anymore.
            unsafe { libc::shmdt(self.segment.shmaddr.cast()) };
        }
        // SAFETY: clears the pointers XDestroyImage would otherwise free (the mapping and our
        // boxed segment info), then frees the XImage itself.
        unsafe {
            (*self.image).data = ptr::null_mut();
            (*self.image).obdata = ptr::null_mut();
            (conn.xlib.XDestroyImage)(self.image);
        }
    }
}

struct Grabber {
    conn: Conn,
    target: Target,
    root: Window,
    root_visual: *mut Visual,
    root_depth: c_int,
    /// MIT-SHM while it works; None means plain XGetImage (remote displays).
    shm: Option<Xext>,
    image: Option<ShmImage>,
    cursor: Option<XFixes>,
}

impl Grabber {
    fn new(xlib: Xlib, target: Target) -> Result<Grabber, String> {
        let conn = Conn::open(xlib)?;
        let display = conn.display;
        // SAFETY: valid connection.
        let screen = unsafe { (conn.xlib.XDefaultScreen)(display) };
        // SAFETY: valid connection and screen number.
        let (root_visual, root_depth) =
            unsafe { ((conn.xlib.XDefaultVisual)(display, screen), (conn.xlib.XDefaultDepth)(display, screen)) };
        // SAFETY: valid connection; only asks whether the server has MIT-SHM.
        let shm = Xext::open().ok().filter(|ext| unsafe { (ext.XShmQueryExtension)(display) } != 0);
        let cursor = XFixes::open().ok().filter(|fixes| {
            let (mut event_base, mut error_base) = (0, 0);
            // SAFETY: valid connection and out-pointers.
            unsafe { (fixes.XFixesQueryExtension)(display, &mut event_base, &mut error_base) != 0 }
        });
        let root = conn.root();
        Ok(Grabber { conn, target, root, root_visual, root_depth, shm, image: None, cursor })
    }

    fn grab(&mut self) -> Grab {
        match self.target {
            Target::Area(rect) => self.grab_area(rect),
            Target::Window(window) => self.grab_window(window),
        }
    }

    fn grab_area(&mut self, rect: Rect) -> Grab {
        match self.capture(self.root, rect, self.root_visual, self.root_depth) {
            Ok(mut frame) => {
                self.draw_cursor(&mut frame, rect.x, rect.y);
                Grab::Frame(frame)
            }
            Err(Failure::Unsupported) => Grab::Ended(ODD_COLORS.into()),
            // The rectangle no longer fits the root window: its monitor was unplugged.
            Err(Failure::Refused) => match self.conn.attributes(self.root) {
                Some(root) if !rect.fits_in(Rect::sized(root.width, root.height)) => Grab::Ended(SCREEN_GONE.into()),
                _ => Grab::Skip,
            },
        }
    }

    fn grab_window(&mut self, window: Window) -> Grab {
        // Read every frame: catches resizes, and fails once the window is destroyed (BadWindow).
        let Some(attributes) = self.conn.attributes(window) else {
            return Grab::Ended(WINDOW_CLOSED.into());
        };
        // Minimised or on another workspace: keep the last picture until it's back.
        if attributes.map_state != xlib::IsViewable {
            return Grab::Skip;
        }
        let size = Rect::sized(attributes.width, attributes.height);
        let position = self.conn.root_position(window);
        let mut frame = match self.capture(window, size, attributes.visual, attributes.depth) {
            Ok(frame) => frame,
            Err(Failure::Unsupported) => return Grab::Ended(ODD_COLORS.into()),
            // Without a compositor X only hands out the part of a window that's on screen.
            Err(Failure::Refused) => match position.and_then(|p| self.capture_on_screen_part(window, p, size)) {
                Some(frame) => frame,
                None => return Grab::Skip,
            },
        };
        if let Some((x, y)) = position {
            self.draw_cursor(&mut frame, x, y);
        }
        Grab::Frame(frame)
    }

    /// The window's on-screen part, placed in a black frame of the window's full size.
    fn capture_on_screen_part(&self, window: Window, (x, y): (i32, i32), size: Rect) -> Option<Frame> {
        let root = self.conn.attributes(self.root)?;
        let visible = Rect { x, y, ..size }.intersect(Rect::sized(root.width, root.height))?;
        let part = Rect { x: visible.x - x, y: visible.y - y, ..visible };
        let pixels = self.capture_plain(window, part).ok()?;
        let mut frame = Frame {
            width: size.width,
            height: size.height,
            stride: size.width as usize * 4,
            data: vec![0; size.width as usize * size.height as usize * 4],
        };
        paste(&mut frame, &pixels, part.x as usize, part.y as usize);
        Some(frame)
    }

    fn capture(&mut self, drawable: Drawable, rect: Rect, visual: *mut Visual, depth: c_int) -> Result<Frame, Failure> {
        if self.prepare_shm(rect.width, rect.height, visual, depth) {
            self.capture_shm(drawable, rect)
        } else {
            self.capture_plain(drawable, rect)
        }
    }

    /// Makes sure the shared image matches this size and format. False means plain XGetImage,
    /// for good once the server can't share memory with us.
    fn prepare_shm(&mut self, width: u32, height: u32, visual: *mut Visual, depth: c_int) -> bool {
        if self.shm.is_none() {
            return false;
        }
        if let Some(image) = &self.image
            && (image.width, image.height, image.visual, image.depth) == (width, height, visual, depth)
        {
            return true;
        }
        self.free_shm();
        self.image = self.create_shm(width, height, visual, depth);
        if self.image.is_none() {
            self.shm = None;
        }
        self.image.is_some()
    }

    fn create_shm(&self, width: u32, height: u32, visual: *mut Visual, depth: c_int) -> Option<ShmImage> {
        let ext = self.shm.as_ref()?;
        let display = self.conn.display;
        let mut segment =
            Box::new(XShmSegmentInfo { shmseg: 0, shmid: -1, shmaddr: ptr::null_mut(), readOnly: xlib::False });
        // SAFETY: valid connection and visual; `segment` is boxed, so the pointer the image keeps stays valid.
        let image = unsafe {
            (ext.XShmCreateImage)(
                display,
                visual,
                depth as c_uint,
                xlib::ZPixmap,
                ptr::null_mut(),
                &mut *segment,
                width,
                height,
            )
        };
        if image.is_null() {
            return None;
        }
        let mut shared = ShmImage { image, segment, attached: false, width, height, visual, depth };
        // SAFETY: XShmCreateImage filled in the row length and height.
        let size = unsafe { (*image).bytes_per_line as usize * (*image).height as usize };
        // SAFETY: plain System V call; failure is checked.
        let id = unsafe { libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600) };
        if id < 0 {
            shared.free(&self.conn, ext);
            return None;
        }
        // SAFETY: maps the segment we just made; failure is checked.
        let address = unsafe { libc::shmat(id, ptr::null(), 0) };
        if address as isize != -1 {
            shared.segment.shmid = id;
            shared.segment.shmaddr = address.cast();
            // SAFETY: the image is ours; it now points at the mapping.
            unsafe { (*image).data = address.cast() };
            // SAFETY: valid connection; the segment info is complete. Remote servers answer with
            // an error (they can't see our memory), which the sync below catches.
            let queued = unsafe { (ext.XShmAttach)(display, &mut *shared.segment) } != 0;
            let (_, failed) = self.conn.trapped(|| self.conn.sync());
            shared.attached = queued && !failed;
        }
        // Mark for removal now that the server has attached: the kernel frees it once both
        // sides let go, even if Yap crashes.
        // SAFETY: `id` is our segment; no buffer is passed for IPC_RMID.
        unsafe { libc::shmctl(id, libc::IPC_RMID, ptr::null_mut()) };
        if !shared.attached {
            shared.free(&self.conn, ext);
            return None;
        }
        Some(shared)
    }

    fn free_shm(&mut self) {
        if let (Some(image), Some(ext)) = (self.image.take(), &self.shm) {
            image.free(&self.conn, ext);
        }
    }

    fn capture_shm(&self, drawable: Drawable, rect: Rect) -> Result<Frame, Failure> {
        let (Some(ext), Some(shared)) = (&self.shm, &self.image) else { return Err(Failure::Refused) };
        // x11-dl declares this plane mask as 32 bits (C has unsigned long); all 32 planes is all
        // of them for any TrueColor depth.
        // SAFETY: the image is attached on this connection and is exactly rect's size.
        let (ok, failed) = self.conn.trapped(|| unsafe {
            (ext.XShmGetImage)(self.conn.display, drawable, shared.image, rect.x, rect.y, c_uint::MAX)
        });
        if ok == 0 || failed {
            return Err(Failure::Refused);
        }
        // SAFETY: the server has filled the image; it isn't touched again until the next grab.
        image_to_frame(unsafe { &*shared.image })
    }

    fn capture_plain(&self, drawable: Drawable, rect: Rect) -> Result<Frame, Failure> {
        // SAFETY: valid connection; the returned image is destroyed below.
        let (image, failed) = self.conn.trapped(|| unsafe {
            (self.conn.xlib.XGetImage)(
                self.conn.display,
                drawable,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                ALL_PLANES,
                xlib::ZPixmap,
            )
        });
        if image.is_null() {
            return Err(Failure::Refused);
        }
        // SAFETY: XGetImage returned a complete image.
        let frame = if failed { Err(Failure::Refused) } else { image_to_frame(unsafe { &*image }) };
        // SAFETY: the image came from XGetImage and isn't used after this.
        unsafe { (self.conn.xlib.XDestroyImage)(image) };
        frame
    }

    /// Draws the pointer onto a frame whose top-left corner sits at (left, top) on the root window.
    fn draw_cursor(&self, frame: &mut Frame, left: i32, top: i32) {
        let Some(fixes) = &self.cursor else { return };
        // SAFETY: valid connection; the image is freed below.
        let (cursor, _) = self.conn.trapped(|| unsafe { (fixes.XFixesGetCursorImage)(self.conn.display) });
        if cursor.is_null() {
            return;
        }
        // SAFETY: XFixes returned a complete cursor image.
        let image = unsafe { &*cursor };
        if !image.pixels.is_null() && image.width > 0 {
            let (width, height) = (usize::from(image.width), usize::from(image.height));
            // SAFETY: the cursor image holds width × height pixels.
            let pixels = unsafe { std::slice::from_raw_parts(image.pixels, width * height) };
            let x = i32::from(image.x) - i32::from(image.xhot) - left;
            let y = i32::from(image.y) - i32::from(image.yhot) - top;
            blend_cursor(frame, pixels, width, x, y);
        }
        // SAFETY: allocated by XFixesGetCursorImage and not used after this.
        unsafe { (self.conn.xlib.XFree)(cursor.cast()) };
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        // Before `conn` closes (fields drop after this).
        self.free_shm();
    }
}

fn image_to_frame(image: &XImage) -> Result<Frame, Failure> {
    if image.data.is_null() || image.width <= 0 || image.height <= 0 || image.bytes_per_line <= 0 {
        return Err(Failure::Refused);
    }
    let mask = |m: c_ulong| u32::try_from(m).map_err(|_| Failure::Unsupported);
    let layout = PixelLayout {
        bits_per_pixel: image.bits_per_pixel.max(0) as u32,
        msb_first: image.byte_order == xlib::MSBFirst,
        red_mask: mask(image.red_mask)?,
        green_mask: mask(image.green_mask)?,
        blue_mask: mask(image.blue_mask)?,
    };
    let (width, height, stride) = (image.width as usize, image.height as usize, image.bytes_per_line as usize);
    // SAFETY: an XImage's data is bytes_per_line × height bytes.
    let data = unsafe { std::slice::from_raw_parts(image.data.cast::<u8>(), stride * height) };
    to_bgra(data, width, height, stride, layout).ok_or(Failure::Unsupported)
}

// ---------------------------------------------------------------------------------------------
// Pixels

#[derive(Clone, Copy, Debug)]
struct PixelLayout {
    bits_per_pixel: u32,
    msb_first: bool,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
}

/// One color channel inside a packed pixel.
struct Channel {
    shift: u32,
    max: u32,
}

impl Channel {
    /// None for an empty or non-contiguous mask.
    fn new(mask: u32) -> Option<Channel> {
        let shift = mask.trailing_zeros();
        let max = mask.checked_shr(shift).unwrap_or(0);
        (max != 0 && max & max.wrapping_add(1) == 0).then_some(Channel { shift, max })
    }

    fn get(&self, pixel: u32) -> u8 {
        let value = u64::from((pixel >> self.shift) & self.max);
        let max = u64::from(self.max);
        ((value * 255 + max / 2) / max) as u8
    }
}

/// Converts packed TrueColor pixels to BGRA. None for formats we don't handle (palettes,
/// odd sizes). The usual 24/32-bit desktop is already B, G, R, X in memory and is just copied.
fn to_bgra(src: &[u8], width: usize, height: usize, stride: usize, layout: PixelLayout) -> Option<Frame> {
    let bytes = match layout.bits_per_pixel {
        16 => 2,
        24 => 3,
        32 => 4,
        _ => return None,
    };
    if width == 0 || height == 0 || stride < width * bytes || src.len() < stride * height {
        return None;
    }
    let size = (width as u32, height as u32);
    let masks = (layout.red_mask, layout.green_mask, layout.blue_mask);
    if bytes == 4 && !layout.msb_first && masks == (0xff_0000, 0xff00, 0xff) {
        return Some(Frame { width: size.0, height: size.1, stride, data: src[..stride * height].to_vec() });
    }
    let channels = [Channel::new(layout.blue_mask)?, Channel::new(layout.green_mask)?, Channel::new(layout.red_mask)?];
    let mut data = vec![0u8; width * height * 4];
    for (row, out) in data.chunks_exact_mut(width * 4).enumerate() {
        let pixels = src[row * stride..][..width * bytes].chunks_exact(bytes);
        for (pixel, out) in pixels.zip(out.as_chunks_mut::<4>().0) {
            let fold = |value: u32, &byte: &u8| (value << 8) | u32::from(byte);
            let value = if layout.msb_first { pixel.iter().fold(0, fold) } else { pixel.iter().rev().fold(0, fold) };
            for (out, channel) in out.iter_mut().zip(&channels) {
                *out = channel.get(value);
            }
            out[3] = 255;
        }
    }
    Some(Frame { width: size.0, height: size.1, stride: width * 4, data })
}

/// Blends a cursor (premultiplied ARGB, one per C long as XFixes gives it) onto a frame with
/// its top-left corner at (left, top); parts outside the frame are cut off.
fn blend_cursor(frame: &mut Frame, pixels: &[c_ulong], width: usize, left: i32, top: i32) {
    if width == 0 {
        return;
    }
    for (row, line) in pixels.chunks_exact(width).enumerate() {
        let y = i64::from(top) + row as i64;
        if y < 0 || y >= i64::from(frame.height) {
            continue;
        }
        for (column, &argb) in line.iter().enumerate() {
            let x = i64::from(left) + column as i64;
            let argb = argb as u32;
            let alpha = argb >> 24;
            if x < 0 || x >= i64::from(frame.width) || alpha == 0 {
                continue;
            }
            let at = y as usize * frame.stride + x as usize * 4;
            let Some(out) = frame.data.get_mut(at..at + 3) else { continue };
            for (i, out) in out.iter_mut().enumerate() {
                let source = (argb >> (8 * i)) & 0xff;
                *out = (source + (u32::from(*out) * (255 - alpha) + 127) / 255).min(255) as u8;
            }
        }
    }
}

/// Copies `src` into `dst` with its top-left corner at (x, y), cut off at dst's edges.
fn paste(dst: &mut Frame, src: &Frame, x: usize, y: usize) {
    let width = (src.width as usize).min((dst.width as usize).saturating_sub(x));
    let height = (src.height as usize).min((dst.height as usize).saturating_sub(y));
    for row in 0..height {
        let from = row * src.stride;
        let to = (y + row) * dst.stride + x * 4;
        dst.data[to..to + width * 4].copy_from_slice(&src.data[from..from + width * 4]);
    }
}

// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_is_detected_from_either_variable() {
        assert!(is_wayland(Some("wayland"), None));
        assert!(is_wayland(Some("Wayland"), None));
        assert!(is_wayland(None, Some("wayland-0")));
        // XWayland: DISPLAY is set too, but the session is still Wayland.
        assert!(is_wayland(Some("x11"), Some("wayland-0")));
        assert!(!is_wayland(Some("x11"), None));
        assert!(!is_wayland(Some("x11"), Some("")));
        assert!(!is_wayland(Some("tty"), None));
        assert!(!is_wayland(None, None));
    }

    fn monitor(x: i32, y: i32, width: u32, height: u32, primary: bool) -> Monitor {
        Monitor { rect: Rect { x, y, width, height }, primary }
    }

    #[test]
    fn screens_are_named_primary_first_then_left_to_right() {
        let sources = screen_sources(vec![
            monitor(4480, 0, 1920, 1080, false),
            monitor(1920, 0, 2560, 1440, true),
            monitor(0, 0, 1920, 1080, false),
        ]);
        let names: Vec<&str> = sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Screen 1 (2560 × 1440)", "Screen 2 (1920 × 1080)", "Screen 3 (1920 × 1080)"]);
        assert_eq!(sources[0].id, "m:1920,0,2560,1440");
        assert_eq!(sources[1].id, "m:0,0,1920,1080");
        assert_eq!(sources[2].id, "m:4480,0,1920,1080");
        assert!(sources.iter().all(|s| s.kind == SourceKind::Screen));
        assert_eq!((sources[0].width, sources[0].height), (2560, 1440));
    }

    #[test]
    fn ids_round_trip() {
        let sources = screen_sources(vec![monitor(-1280, 200, 1280, 1024, false)]);
        assert_eq!(
            parse_target(&sources[0].id),
            Some(Target::Area(Rect { x: -1280, y: 200, width: 1280, height: 1024 }))
        );
        assert_eq!(parse_target("w:12582919"), Some(Target::Window(12582919)));
        assert_eq!(parse_target("w:0"), None);
        assert_eq!(parse_target("w:abc"), None);
        assert_eq!(parse_target("m:0,0,0,1080"), None);
        assert_eq!(parse_target("m:0,0,1920"), None);
        assert_eq!(parse_target("m:0,0,1920,1080,1"), None);
        assert_eq!(parse_target("display:1"), None);
    }

    #[test]
    fn window_names() {
        assert_eq!(window_name("Inbox", Some("Thunderbird")), "Inbox — Thunderbird");
        assert_eq!(window_name("Mozilla Firefox", Some("firefox")), "Mozilla Firefox");
        assert_eq!(window_name(" notes.txt ", None), "notes.txt");
        assert_eq!(window_name("notes.txt", Some("  ")), "notes.txt");
        assert_eq!(class_name(b"navigator\0Firefox\0").as_deref(), Some("Firefox"));
        assert_eq!(class_name(b"xterm\0\0").as_deref(), Some("xterm"));
        assert_eq!(class_name(b"").as_deref(), None);
        assert_eq!(decode_text(xlib::XA_STRING, b"caf\xe9"), "café");
        assert_eq!(decode_text(0, "café".as_bytes()), "café");
        assert_eq!(clean_title(" Terminal\0"), "Terminal");
    }

    #[test]
    fn rectangles() {
        let screen = Rect::sized(1920, 1080);
        let window = Rect { x: 1800, y: -20, width: 400, height: 300 };
        assert_eq!(window.intersect(screen), Some(Rect { x: 1800, y: 0, width: 120, height: 280 }));
        assert_eq!(Rect { x: 1920, y: 0, width: 10, height: 10 }.intersect(screen), None);
        assert!(Rect { x: 0, y: 0, width: 1920, height: 1080 }.fits_in(screen));
        assert!(!Rect { x: 1920, y: 0, width: 1920, height: 1080 }.fits_in(screen));
    }

    fn layout(bits_per_pixel: u32, msb_first: bool, masks: (u32, u32, u32)) -> PixelLayout {
        PixelLayout { bits_per_pixel, msb_first, red_mask: masks.0, green_mask: masks.1, blue_mask: masks.2 }
    }

    #[test]
    fn usual_32_bit_pixels_are_copied_with_their_stride() {
        // 2 × 2 pixels, 12-byte rows (4 bytes of padding).
        let src: Vec<u8> = (0..24).collect();
        let frame = to_bgra(&src, 2, 2, 12, layout(32, false, (0xff_0000, 0xff00, 0xff))).unwrap();
        assert_eq!((frame.width, frame.height, frame.stride), (2, 2, 12));
        assert_eq!(frame.data, src);
    }

    #[test]
    fn big_endian_32_bit_pixels() {
        // X, R, G, B in memory.
        let src = [0, 10, 20, 30, 0, 40, 50, 60];
        let frame = to_bgra(&src, 2, 1, 8, layout(32, true, (0xff_0000, 0xff00, 0xff))).unwrap();
        assert_eq!(frame.data, [30, 20, 10, 255, 60, 50, 40, 255]);
    }

    #[test]
    fn swapped_masks_are_reordered() {
        // R, G, B, X in memory (red in the low byte).
        let src = [10, 20, 30, 0];
        let frame = to_bgra(&src, 1, 1, 4, layout(32, false, (0xff, 0xff00, 0xff_0000))).unwrap();
        assert_eq!(frame.data, [30, 20, 10, 255]);
    }

    #[test]
    fn packed_24_bit_pixels() {
        // B, G, R per pixel, one byte of row padding.
        let src = [1, 2, 3, 4, 5, 6, 0];
        let frame = to_bgra(&src, 2, 1, 7, layout(24, false, (0xff_0000, 0xff00, 0xff))).unwrap();
        assert_eq!((frame.stride, frame.data.as_slice()), (8, &[1, 2, 3, 255, 4, 5, 6, 255][..]));
    }

    #[test]
    fn rgb565_pixels_scale_to_full_range() {
        let pixels: [u16; 3] = [0xf800, 0x07e0, 0x001f];
        let src: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
        let frame = to_bgra(&src, 3, 1, 6, layout(16, false, (0xf800, 0x07e0, 0x001f))).unwrap();
        assert_eq!(frame.data, [0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255]);
    }

    #[test]
    fn ten_bit_pixels() {
        // Depth 30: 10 bits per channel, red on top.
        let pixel: u32 = (1023 << 20) | (512 << 10);
        let frame = to_bgra(&pixel.to_le_bytes(), 1, 1, 4, layout(32, false, (0x3ff0_0000, 0xffc00, 0x3ff))).unwrap();
        assert_eq!(frame.data, [0, 128, 255, 255]);
    }

    #[test]
    fn exotic_formats_are_refused() {
        assert!(to_bgra(&[0; 4], 4, 1, 4, layout(8, false, (0xe0, 0x1c, 0x03))).is_none());
        assert!(to_bgra(&[0; 4], 1, 1, 4, layout(32, false, (0, 0, 0))).is_none());
        assert!(to_bgra(&[0; 4], 1, 1, 4, layout(32, false, (0xf0f000, 0xff00, 0xff))).is_none());
        // Buffer too short for the claimed size.
        assert!(to_bgra(&[0; 4], 2, 1, 8, layout(32, false, (0xff_0000, 0xff00, 0xff))).is_none());
    }

    fn gray_frame(width: u32, height: u32, value: u8) -> Frame {
        Frame { width, height, stride: width as usize * 4, data: vec![value; (width * height * 4) as usize] }
    }

    #[test]
    fn cursor_blends_premultiplied_and_is_clipped() {
        let mut frame = gray_frame(2, 2, 100);
        // Opaque red, transparent, half-transparent white (premultiplied), off-frame.
        let cursor: [c_ulong; 4] = [0xffff_0000, 0x0000_0000, 0x8080_8080, 0xff00_ff00];
        blend_cursor(&mut frame, &cursor, 2, 1, 0);
        // (1,0) got opaque red; (1,1) got the half white; column 2 is off the frame.
        assert_eq!(&frame.data[4..8], &[0, 0, 255, 100]);
        assert_eq!(&frame.data[12..16], &[178, 178, 178, 100]);
        assert_eq!(&frame.data[0..4], &[100; 4]);
        assert_eq!(&frame.data[8..12], &[100; 4]);
        // Entirely outside: nothing changes, nothing panics.
        blend_cursor(&mut frame, &cursor, 2, -5, -5);
        blend_cursor(&mut frame, &cursor, 2, 10, 10);
        blend_cursor(&mut frame, &cursor, 0, 0, 0);
    }

    #[test]
    fn paste_cuts_at_the_edges() {
        let mut frame = gray_frame(3, 2, 0);
        paste(&mut frame, &gray_frame(2, 2, 9), 2, 1);
        assert_eq!(&frame.data[0..12], &[0; 12]);
        assert_eq!(&frame.data[12..20], &[0; 8]);
        assert_eq!(&frame.data[20..24], &[9; 4]);
    }

    #[test]
    fn setup_failure_goes_to_on_ended_once_start_stopped_waiting() {
        let setup = Setup::default();
        *lock(&setup.state) = SetupState::Abandoned;
        assert_eq!(setup.report(Err("gone".into())).as_deref(), Some("gone"));
        assert_eq!(setup.report(Ok(())), None);

        let setup = Setup::default();
        assert_eq!(setup.report(Err("gone".into())), None);
        assert_eq!(setup.wait(), Err("gone".to_string()));
    }

    #[test]
    fn fps_is_kept_sane() {
        let shared = Shared::new(0);
        assert_eq!(shared.interval(), Duration::from_secs(1));
        shared.set_fps(30);
        assert_eq!(shared.interval(), Duration::from_secs(1) / 30);
        shared.set_fps(10_000);
        assert_eq!(shared.interval(), Duration::from_secs(1) / 120);
        shared.stop();
        assert!(shared.sleep_until(Instant::now() + Duration::from_secs(60)));
    }
}
