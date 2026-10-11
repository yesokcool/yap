//! Windows screen capture.
//!
//! Picture: Windows.Graphics.Capture (Windows 10 1903+), copied off the GPU into BGRA frames at
//! native size. Sound: WASAPI process loopback of everything the PC plays except Yap's own
//! process tree (Windows 10 2004+), so the call is never sent back into the call.
//!
//! Threads: "yap-capture-video" owns every capture object; the WinRT callbacks only nudge it
//! through a channel. "yap-capture-audio" (when sound is on) pulls sound packets.
//!
//! The parent module gates this file with `#[cfg(windows)]`. The pure helpers at the top build
//! anywhere so their tests can run off Windows; everything that calls Windows is in `imp`.
//! Paths into the `windows` crate are written `::windows::` because this module is named
//! `windows` too.

use super::*;

use std::time::{Duration, Instant};

#[cfg(windows)]
pub use imp::{sound_supported, sources, start};

// ---- Pure helpers ----

/// Process loopback capture exists from Windows 10 2004 (build 19041). Microsoft's docs list
/// build 20348, but it works from 19041.
const MIN_LOOPBACK_BUILD: u32 = 19041;

/// Sound always goes out as stereo.
const CHANNELS: usize = 2;

/// "Screen 1 (2560 × 1440)", or just "Screen 1" when the size is unknown.
fn screen_name(number: usize, width: u32, height: u32) -> String {
    if width == 0 || height == 0 {
        format!("Screen {number}")
    } else {
        format!("Screen {number} ({width} × {height})")
    }
}

/// "Title — app", where app is the program's file name without ".exe". Just the title when
/// the app name adds nothing (it's already in the title, or it's the host of a Store app).
fn window_name(title: &str, exe_path: Option<&str>) -> String {
    let title = title.trim();
    let Some(app) = exe_path.and_then(app_name) else {
        return title.to_string();
    };
    if app.eq_ignore_ascii_case("ApplicationFrameHost") || title.to_lowercase().contains(&app.to_lowercase()) {
        return title.to_string();
    }
    format!("{title} — {app}")
}

/// `C:\Program Files\App\App.exe` → `App`.
fn app_name(path: &str) -> Option<&str> {
    let file = path.rsplit(['\\', '/']).next()?;
    let stem = file
        .len()
        .checked_sub(4)
        .and_then(|cut| Some((file.get(..cut)?, file.get(cut..)?)))
        .filter(|(_, extension)| extension.eq_ignore_ascii_case(".exe"))
        .map_or(file, |(stem, _)| stem);
    (!stem.is_empty()).then_some(stem)
}

/// Source ids are "m:<HMONITOR>" for screens and "w:<HWND>" for windows.
fn parse_source_id(id: &str) -> Option<(SourceKind, usize)> {
    let (kind, handle) = id.split_once(':')?;
    let kind = match kind {
        "m" => SourceKind::Screen,
        "w" => SourceKind::Window,
        _ => return None,
    };
    let handle = handle.parse::<usize>().ok().filter(|&handle| handle != 0)?;
    Some((kind, handle))
}

fn frame_interval(fps: u32) -> Duration {
    Duration::from_secs(1) / fps.clamp(1, 240)
}

/// When the next frame may go out, after one went out at `now` that was due at `due`. Keeps an
/// even pace while frames keep coming, without a burst after a quiet spell.
fn next_due(due: Instant, now: Instant, interval: Duration) -> Instant {
    let next = due + interval;
    if next > now { next } else { now + interval }
}

/// Packs `rows` rows of `row_bytes` each out of a buffer whose rows start `pitch` bytes apart.
fn copy_rows(src: &[u8], pitch: usize, row_bytes: usize, rows: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(row_bytes * rows);
    for row in src.chunks(pitch).take(rows) {
        out.extend_from_slice(&row[..row_bytes]);
    }
    out
}

/// The registry's "CurrentBuildNumber" text, e.g. "22631".
fn parse_build(text: &str) -> Option<u32> {
    text.trim_matches(|c: char| c == '\0' || c.is_whitespace()).parse().ok()
}

fn build_supports_sound(build: u32) -> bool {
    build >= MIN_LOOPBACK_BUILD
}

/// What WASAPI hands us: 32-bit float, or 16-bit integers if float was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SampleFormat {
    F32,
    I16,
}

impl SampleFormat {
    /// The WAVEFORMATEX format tag (WAVE_FORMAT_IEEE_FLOAT or WAVE_FORMAT_PCM, from mmreg.h).
    fn tag(self) -> u16 {
        match self {
            SampleFormat::F32 => 3,
            SampleFormat::I16 => 1,
        }
    }

    fn bits(self) -> u16 {
        match self {
            SampleFormat::F32 => 32,
            SampleFormat::I16 => 16,
        }
    }

    fn bytes_per_frame(self) -> usize {
        CHANNELS * usize::from(self.bits() / 8)
    }

    /// Appends the samples in `bytes` (little-endian) to `out` as f32 in -1.0..1.0.
    fn convert(self, bytes: &[u8], out: &mut Vec<f32>) {
        match self {
            SampleFormat::F32 => {
                out.extend(bytes.as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)));
            }
            SampleFormat::I16 => {
                out.extend(bytes.as_chunks::<2>().0.iter().map(|&b| f32::from(i16::from_le_bytes(b)) / 32768.0));
            }
        }
    }
}

// ---- Windows ----

#[cfg(windows)]
mod imp {
    use super::*;

    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
    use std::thread::{self, JoinHandle};

    use ::windows::Foundation::{TimeSpan, TypedEventHandler};
    use ::windows::Graphics::Capture::{
        Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureAccess, GraphicsCaptureAccessKind,
        GraphicsCaptureItem, GraphicsCaptureSession,
    };
    use ::windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use ::windows::Graphics::DirectX::DirectXPixelFormat;
    use ::windows::Graphics::SizeInt32;
    use ::windows::Win32::Foundation::{
        CloseHandle, E_FAIL, E_NOINTERFACE, ERROR_SUCCESS, HANDLE, HMODULE, HWND, LPARAM, RECT,
    };
    use ::windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
    use ::windows::Win32::Graphics::Direct3D11::{
        D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
        D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice,
        ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    };
    use ::windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    use ::windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use ::windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIDevice};
    use ::windows::Win32::Graphics::Gdi::{
        DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, HDC, HMONITOR,
        MONITORINFO, MONITORINFOEXW,
    };
    use ::windows::Win32::Media::Audio::{
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
        AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
        IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient,
        IAudioClient, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
        WAVEFORMATEX,
    };
    use ::windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use ::windows::Win32::System::Com::{BLOB, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use ::windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    use ::windows::Win32::System::Threading::{
        CreateEventW, GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW, WaitForSingleObject,
    };
    use ::windows::Win32::System::Variant::VT_BLOB;
    use ::windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use ::windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetShellWindow, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, MONITORINFOF_PRIMARY, WS_EX_TOOLWINDOW,
    };
    use ::windows::core::{
        BOOL, Error, HRESULT, IInspectable, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result as WinResult, implement,
        w,
    };

    const PIXEL_FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;
    const POOL_BUFFERS: i32 = 2;
    /// How long start() waits for the capture to come up before returning anyway.
    const START_WAIT: Duration = Duration::from_millis(500);
    /// How long the Windows 11 "no yellow border" request may take before we go on without it.
    const BORDER_WAIT: Duration = Duration::from_millis(300);
    const SAMPLE_RATE: u32 = 48_000;
    /// The sound buffer: 100 ms, in 100-nanosecond units.
    const SOUND_BUFFER: i64 = 1_000_000;
    const ACTIVATION_WAIT: Duration = Duration::from_secs(5);
    /// Sound waits wake at least this often so a stop is noticed quickly.
    const SOUND_POLL: Duration = Duration::from_millis(100);

    const START_FAILED: &str = "Screen sharing couldn't start.";
    const DEVICE_LOST: &str = "Screen sharing stopped because the graphics driver restarted.";

    fn gone_message(kind: &SourceKind) -> &'static str {
        match kind {
            SourceKind::Screen => "That screen isn't connected any more.",
            SourceKind::Window => "That window was closed.",
        }
    }

    // ---- sources() ----

    pub fn sources() -> Result<Vec<Source>, String> {
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err("Screen sharing needs Windows 10 version 1903 or newer.".into());
        }
        let mut list = screens();
        list.extend(app_windows());
        Ok(list)
    }

    fn screens() -> Vec<Source> {
        let mut handles: Vec<HMONITOR> = Vec::new();
        // SAFETY: the callback runs only during this call and gets a pointer to `handles`.
        let _ = unsafe { EnumDisplayMonitors(None, None, Some(push_monitor), LPARAM(&raw mut handles as isize)) };
        let mut screens: Vec<(bool, usize, u32, u32)> = handles
            .into_iter()
            .filter_map(|monitor| {
                let (primary, width, height) = monitor_details(monitor)?;
                Some((primary, monitor.0 as usize, width, height))
            })
            .collect();
        // Primary first; the others keep Windows' order (the sort is stable).
        screens.sort_by_key(|&(primary, ..)| !primary);
        screens
            .into_iter()
            .enumerate()
            .map(|(index, (_, handle, width, height))| Source {
                id: format!("m:{handle}"),
                name: screen_name(index + 1, width, height),
                kind: SourceKind::Screen,
                width,
                height,
            })
            .collect()
    }

    unsafe extern "system" fn push_monitor(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the Vec that screens() passed in, alive for the whole enumeration.
        let handles = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        handles.push(monitor);
        true.into()
    }

    /// (is primary, width, height) in real pixels, or None if the monitor is gone.
    fn monitor_details(monitor: HMONITOR) -> Option<(bool, u32, u32)> {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `info` is a MONITORINFOEXW and cbSize says so, so Windows also fills in the device name.
        if !unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast::<MONITORINFO>()) }.as_bool() {
            return None;
        }
        let primary = info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0;
        // The display mode is in real pixels whatever Yap's DPI awareness; the rectangle is the fallback.
        let mut mode = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
        // SAFETY: szDevice is the NUL-terminated name GetMonitorInfoW just wrote; `mode` has dmSize set.
        let found = unsafe { EnumDisplaySettingsW(PCWSTR(info.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode) };
        let (width, height) =
            if found.as_bool() { (mode.dmPelsWidth, mode.dmPelsHeight) } else { rect_size(&info.monitorInfo.rcMonitor) };
        Some((primary, width, height))
    }

    fn rect_size(rect: &RECT) -> (u32, u32) {
        ((rect.right - rect.left).max(0) as u32, (rect.bottom - rect.top).max(0) as u32)
    }

    fn app_windows() -> Vec<Source> {
        let mut handles: Vec<HWND> = Vec::new();
        // SAFETY: the callback runs only during this call and gets a pointer to `handles`.
        let _ = unsafe { EnumWindows(Some(push_window), LPARAM(&raw mut handles as isize)) };
        // SAFETY: neither call has preconditions.
        let (own_pid, desktop) = unsafe { (GetCurrentProcessId(), GetShellWindow()) };
        let mut exe_paths: HashMap<u32, Option<String>> = HashMap::new();
        handles
            .into_iter()
            .filter(|&hwnd| hwnd != desktop)
            .filter_map(|hwnd| {
                let pid = shareable_window_pid(hwnd)?;
                if pid == own_pid {
                    return None;
                }
                let title = window_title(hwnd)?;
                let (width, height) = window_size(hwnd);
                if width == 0 || height == 0 {
                    return None;
                }
                let exe = exe_paths.entry(pid).or_insert_with(|| process_path(pid));
                Some(Source {
                    id: format!("w:{}", hwnd.0 as usize),
                    name: window_name(&title, exe.as_deref()),
                    kind: SourceKind::Window,
                    width,
                    height,
                })
            })
            .collect()
    }

    unsafe extern "system" fn push_window(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the Vec that app_windows() passed in, alive for the whole enumeration.
        let handles = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
        handles.push(hwnd);
        true.into()
    }

    /// The owning process of a window worth offering: visible, not minimised, not a tool window
    /// and not cloaked (hidden by Windows, e.g. on another virtual desktop or a suspended app).
    fn shareable_window_pid(hwnd: HWND) -> Option<u32> {
        // SAFETY: read-only queries; a stale handle only makes them fail.
        let (visible, minimised, ex_style) =
            unsafe { (IsWindowVisible(hwnd).as_bool(), IsIconic(hwnd).as_bool(), GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) };
        if !visible || minimised || ex_style & WS_EX_TOOLWINDOW.0 != 0 || is_cloaked(hwnd) {
            return None;
        }
        let mut pid = 0u32;
        // SAFETY: `pid` is a valid out-pointer.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        (pid != 0).then_some(pid)
    }

    fn is_cloaked(hwnd: HWND) -> bool {
        let mut cloaked = 0u32;
        // SAFETY: DWMWA_CLOAKED writes one DWORD into `cloaked`.
        let asked = unsafe {
            DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, (&raw mut cloaked).cast::<c_void>(), size_of::<u32>() as u32)
        };
        asked.is_ok() && cloaked != 0
    }

    fn window_title(hwnd: HWND) -> Option<String> {
        // SAFETY: read-only query.
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return None;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        // SAFETY: the slice tells Windows how much room there is.
        let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        let title = String::from_utf16_lossy(&buffer[..(copied.max(0) as usize).min(buffer.len())]);
        let title = title.trim();
        (!title.is_empty()).then(|| title.to_string())
    }

    /// The window's visible size in real pixels (what the capture will be).
    fn window_size(hwnd: HWND) -> (u32, u32) {
        let mut rect = RECT::default();
        // SAFETY: DWMWA_EXTENDED_FRAME_BOUNDS writes one RECT into `rect`.
        let from_dwm = unsafe {
            DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, (&raw mut rect).cast::<c_void>(), size_of::<RECT>() as u32)
        };
        // SAFETY: `rect` is a valid out-pointer.
        if from_dwm.is_err() && unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
            return (0, 0);
        }
        rect_size(&rect)
    }

    fn process_path(pid: u32) -> Option<String> {
        // SAFETY: query-only access; the handle is closed below.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        // SAFETY: `buffer` has room for `length` characters; Windows writes back how many it used.
        let named =
            unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut length) };
        // SAFETY: `process` came from OpenProcess above and is closed exactly once.
        let _ = unsafe { CloseHandle(process) };
        named.ok()?;
        Some(String::from_utf16_lossy(buffer.get(..length as usize)?))
    }

    // ---- sound_supported() ----

    pub fn sound_supported() -> bool {
        static SUPPORTED: OnceLock<bool> = OnceLock::new();
        *SUPPORTED.get_or_init(|| windows_build().is_some_and(build_supports_sound))
    }

    /// The Windows build number, e.g. 22631. From the registry, because GetVersionEx reports an
    /// old version to programs without a compatibility manifest.
    fn windows_build() -> Option<u32> {
        let mut buffer = [0u16; 32];
        let mut size = size_of_val(&buffer) as u32;
        // SAFETY: `buffer` has room for `size` bytes; RRF_RT_REG_SZ makes Windows NUL-terminate the text.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
                w!("CurrentBuildNumber"),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast::<c_void>()),
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        parse_build(&String::from_utf16_lossy(buffer.get(..size as usize / 2)?))
    }

    // ---- start() ----

    type Ended = Arc<Mutex<Option<OnEnded>>>;
    /// start() takes the sender back if it stops waiting; then the video thread reports a late
    /// failure through on_ended instead.
    type Ready = Arc<Mutex<Option<mpsc::Sender<Result<(), String>>>>>;

    enum Msg {
        Frame,
        Closed,
        Fps(u32),
        Stop,
    }

    pub fn start(
        source: &Source,
        options: Options,
        on_frame: OnFrame,
        on_audio: Option<OnAudio>,
        on_ended: OnEnded,
    ) -> Result<Box<dyn Capture>, String> {
        let Some((kind, handle)) = parse_source_id(&source.id) else {
            return Err(gone_message(&source.kind).into());
        };
        if !target_exists(&kind, handle) {
            return Err(gone_message(&kind).into());
        }

        let ended: Ended = Arc::new(Mutex::new(Some(on_ended)));
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let ready: Ready = Arc::new(Mutex::new(Some(ready_tx)));
        let video = {
            let (tx, ready, ended, fps) = (tx.clone(), ready.clone(), ended.clone(), options.fps);
            thread::Builder::new()
                .name("yap-capture-video".into())
                .spawn(move || video_thread((kind, handle), fps, on_frame, (tx, rx), ready, ended))
                .map_err(|_| START_FAILED.to_string())?
        };

        // Wait a little so most failures come back here as an Err.
        let answer = match ready_rx.recv_timeout(START_WAIT) {
            Ok(answer) => answer,
            Err(RecvTimeoutError::Timeout) => {
                if lock(&ready).take().is_some() {
                    // Still starting; a later failure goes to on_ended.
                    Ok(())
                } else {
                    // It finished starting just now; its answer is on the way.
                    ready_rx.recv().unwrap_or_else(|_| Err(START_FAILED.to_string()))
                }
            }
            Err(RecvTimeoutError::Disconnected) => Err(START_FAILED.to_string()),
        };
        if let Err(message) = answer {
            let _ = video.join();
            return Err(message);
        }

        let audio_stop = Arc::new(AtomicBool::new(false));
        let audio = match on_audio {
            Some(on_audio) if options.sound && sound_supported() => {
                let stop = audio_stop.clone();
                thread::Builder::new()
                    .name("yap-capture-audio".into())
                    .spawn(move || audio_thread(on_audio, stop))
                    .map_err(|error| log::warn!("screen capture: no sound thread: {error}"))
                    .ok()
            }
            _ => None,
        };

        Ok(Box::new(WindowsCapture { tx, video: Some(video), audio, audio_stop, ended }))
    }

    fn target_exists(kind: &SourceKind, handle: usize) -> bool {
        match kind {
            // SAFETY: IsWindow takes any value and only says whether it names a window.
            SourceKind::Window => unsafe { IsWindow(Some(HWND(handle as *mut c_void))) }.as_bool(),
            SourceKind::Screen => monitor_details(HMONITOR(handle as *mut c_void)).is_some(),
        }
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Calls on_ended once, unless stop() came first.
    fn end(ended: &Ended, message: String) {
        // Taken out first so the callback runs unlocked (it may well call stop()).
        let callback = lock(ended).take();
        if let Some(callback) = callback {
            callback(message);
        }
    }

    struct WindowsCapture {
        tx: mpsc::Sender<Msg>,
        video: Option<JoinHandle<()>>,
        audio: Option<JoinHandle<()>>,
        audio_stop: Arc<AtomicBool>,
        ended: Ended,
    }

    impl Capture for WindowsCapture {
        fn set_options(&mut self, options: Options) {
            // Frames go out at native size (the caller scales) and sound changes are ignored,
            // so only the frame rate matters here.
            let _ = self.tx.send(Msg::Fps(options.fps));
        }

        fn stop(&mut self) {
            // A stop that was asked for isn't an ending "by itself": on_ended must not fire now.
            drop(lock(&self.ended).take());
            let _ = self.tx.send(Msg::Stop);
            self.audio_stop.store(true, Ordering::Relaxed);
            join(self.video.take());
            join(self.audio.take());
        }
    }

    impl Drop for WindowsCapture {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn join(handle: Option<JoinHandle<()>>) {
        let Some(handle) = handle else { return };
        // stop() may be called from inside one of our callbacks. That thread can't join itself;
        // it sees the stop and finishes on its own.
        if handle.thread().id() != thread::current().id() {
            let _ = handle.join();
        }
    }

    /// COM for the current thread (multithreaded apartment), undone when dropped.
    struct ComScope(bool);

    impl ComScope {
        fn multithreaded() -> Self {
            // SAFETY: per-thread setup, balanced in Drop when it succeeded.
            ComScope(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
        }
    }

    impl Drop for ComScope {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: balances the successful CoInitializeEx above, on the same thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    // ---- Picture ----

    fn video_thread(
        (kind, handle): (SourceKind, usize),
        fps: u32,
        mut on_frame: OnFrame,
        (tx, rx): (mpsc::Sender<Msg>, mpsc::Receiver<Msg>),
        ready: Ready,
        ended: Ended,
    ) {
        // Declared first so COM is torn down after every capture object is gone.
        let _com = ComScope::multithreaded();
        let started = Video::start(&kind, handle, fps, tx);
        let waiting = lock(&ready).take();
        let mut video = match (started, waiting) {
            (Ok(video), waiting) => {
                if let Some(waiting) = waiting {
                    let _ = waiting.send(Ok(()));
                }
                video
            }
            (Err(message), Some(waiting)) => {
                let _ = waiting.send(Err(message));
                return;
            }
            (Err(message), None) => {
                end(&ended, message);
                return;
            }
        };
        if let Some(message) = video.run(&rx, &mut on_frame) {
            // Let go of the capture before telling the caller.
            drop(video);
            end(&ended, message);
        }
    }

    struct Staging {
        texture: ID3D11Texture2D,
        width: u32,
        height: u32,
    }

    struct Video {
        item: GraphicsCaptureItem,
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        winrt_device: IDirect3DDevice,
        pool: Direct3D11CaptureFramePool,
        session: GraphicsCaptureSession,
        pool_size: SizeInt32,
        staging: Option<Staging>,
        frame_token: i64,
        closed_token: i64,
        interval: Duration,
        closed_message: &'static str,
    }

    impl Video {
        fn start(kind: &SourceKind, handle: usize, fps: u32, tx: mpsc::Sender<Msg>) -> Result<Video, String> {
            let closed_message = match kind {
                SourceKind::Screen => "The shared screen was disconnected.",
                SourceKind::Window => "The shared window was closed.",
            };
            let item = capture_item(kind, handle).map_err(|error| {
                log::warn!("screen capture: no capture item: {error}");
                gone_message(kind).to_string()
            })?;
            let (device, context) = d3d_device().map_err(|error| failed("graphics device", error))?;
            let winrt_device = winrt_device(&device).map_err(|error| failed("WinRT device", error))?;
            let pool_size = item.Size().map_err(|error| failed("item size", error))?;
            let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt_device, PIXEL_FORMAT, POOL_BUFFERS, pool_size)
                .map_err(|error| failed("frame pool", error))?;
            let session = pool.CreateCaptureSession(&item).map_err(|error| failed("session", error))?;

            // The callbacks run on Windows' thread pool; they only wake our thread.
            let frame_tx = tx.clone();
            let on_frame = TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                let _ = frame_tx.send(Msg::Frame);
                Ok(())
            });
            let frame_token = pool.FrameArrived(&on_frame).map_err(|error| failed("frame event", error))?;
            let on_closed = TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                let _ = tx.send(Msg::Closed);
                Ok(())
            });
            let closed_token = match item.Closed(&on_closed) {
                Ok(token) => token,
                Err(error) => {
                    let _ = pool.RemoveFrameArrived(frame_token);
                    return Err(failed("closed event", error));
                }
            };

            // From here on, Drop cleans up.
            let mut video = Video {
                item,
                device,
                context,
                winrt_device,
                pool,
                session,
                pool_size,
                staging: None,
                frame_token,
                closed_token,
                interval: frame_interval(fps),
                closed_message,
            };
            // Extras that older Windows versions lack; capture works without them.
            let _ = video.session.SetIsCursorCaptureEnabled(true);
            hide_border(&video.session);
            video.set_fps(fps);
            video.session.StartCapture().map_err(|error| failed("start", error))?;
            Ok(video)
        }

        fn set_fps(&mut self, fps: u32) {
            self.interval = frame_interval(fps);
            // Newer Windows 11 can skip the extra frames itself; elsewhere we drop them below.
            let ticks = i64::try_from(self.interval.as_nanos() / 100).unwrap_or(i64::MAX);
            let _ = self.session.SetMinUpdateInterval(TimeSpan { Duration: ticks });
        }

        /// Hands out frames until stopped. Returns why it ended by itself, if it did.
        fn run(&mut self, rx: &mpsc::Receiver<Msg>, on_frame: &mut OnFrame) -> Option<String> {
            // The newest frame not handed out yet. Frames that come too soon wait here and are
            // replaced by newer ones, so the last change before a quiet spell still goes out.
            let mut pending: Option<Direct3D11CaptureFrame> = None;
            let mut due = Instant::now();
            let mut failing = false;
            loop {
                let msg = if pending.is_some() {
                    match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                        Ok(msg) => Some(msg),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => Some(Msg::Stop),
                    }
                } else {
                    Some(rx.recv().unwrap_or(Msg::Stop))
                };
                match msg {
                    Some(Msg::Frame) => {
                        if let Some(newer) = self.newest_frame()
                            && let Some(older) = pending.replace(newer)
                        {
                            let _ = older.Close();
                        }
                    }
                    Some(Msg::Fps(fps)) => self.set_fps(fps),
                    Some(Msg::Closed) => return Some(self.closed_message.to_string()),
                    Some(Msg::Stop) => return None,
                    None => {}
                }

                let now = Instant::now();
                if now < due {
                    continue;
                }
                let Some(frame) = pending.take() else { continue };
                let delivered = self.deliver(&frame, on_frame);
                let _ = frame.Close();
                match delivered {
                    Ok(()) => {
                        failing = false;
                        due = next_due(due, now, self.interval);
                    }
                    Err(_) if self.device_lost() => return Some(DEVICE_LOST.to_string()),
                    Err(error) => {
                        if !failing {
                            log::warn!("screen capture: couldn't read a frame: {error}");
                        }
                        failing = true;
                    }
                }
            }
        }

        /// Takes every frame waiting in the pool and keeps the newest, so the pool always has a
        /// free buffer for the next one.
        fn newest_frame(&self) -> Option<Direct3D11CaptureFrame> {
            let mut newest: Option<Direct3D11CaptureFrame> = None;
            while let Ok(frame) = self.pool.TryGetNextFrame() {
                if let Some(older) = newest.replace(frame) {
                    let _ = older.Close();
                }
            }
            newest
        }

        fn deliver(&mut self, frame: &Direct3D11CaptureFrame, on_frame: &mut OnFrame) -> WinResult<()> {
            let content = frame.ContentSize()?;
            let resized = content.Width != self.pool_size.Width || content.Height != self.pool_size.Height;
            if resized && content.Width > 0 && content.Height > 0 {
                // The window changed size: later frames come at the new size; this one is cut to fit.
                self.pool.Recreate(&self.winrt_device, PIXEL_FORMAT, POOL_BUFFERS, content)?;
                self.pool_size = content;
            }

            let texture = frame_texture(frame)?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            // SAFETY: `desc` is a valid out-pointer.
            unsafe { texture.GetDesc(&mut desc) };
            let width = u32::try_from(content.Width).unwrap_or(0).min(desc.Width);
            let height = u32::try_from(content.Height).unwrap_or(0).min(desc.Height);
            if width == 0 || height == 0 {
                return Ok(());
            }

            let staging = self.staging(width, height)?;
            let region = D3D11_BOX { left: 0, top: 0, front: 0, right: width, bottom: height, back: 1 };
            // SAFETY: both textures live on our device, share the BGRA format, and `region` fits in both.
            unsafe { self.context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &texture, 0, Some(&region)) };
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            // SAFETY: the staging texture has CPU read access; it is unmapped below.
            unsafe { self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
            let row_bytes = width as usize * 4;
            let pitch = mapped.RowPitch as usize;
            let length = pitch * (height as usize - 1) + row_bytes;
            // SAFETY: while mapped, pData holds `height` rows of at least `row_bytes`, `pitch` bytes apart.
            let pixels = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, length) };
            let data = copy_rows(pixels, pitch, row_bytes, height as usize);
            // SAFETY: matches the Map above; `pixels` is not used after this.
            unsafe { self.context.Unmap(&staging, 0) };

            on_frame(Frame { width, height, stride: row_bytes, data });
            Ok(())
        }

        /// A CPU-readable texture of this size, made again only when the size changes.
        fn staging(&mut self, width: u32, height: u32) -> WinResult<ID3D11Texture2D> {
            if let Some(staging) = &self.staging
                && staging.width == width
                && staging.height == height
            {
                return Ok(staging.texture.clone());
            }
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut texture = None;
            // SAFETY: `desc` describes a plain staging texture; `texture` is a valid out-pointer.
            unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut texture))? };
            let texture = texture.ok_or_else(|| Error::from(E_FAIL))?;
            self.staging = Some(Staging { texture: texture.clone(), width, height });
            Ok(texture)
        }

        fn device_lost(&self) -> bool {
            // SAFETY: plain query on our own device.
            unsafe { self.device.GetDeviceRemovedReason() }.is_err()
        }
    }

    impl Drop for Video {
        fn drop(&mut self) {
            let _ = self.pool.RemoveFrameArrived(self.frame_token);
            let _ = self.item.RemoveClosed(self.closed_token);
            let _ = self.session.Close();
            let _ = self.pool.Close();
        }
    }

    fn failed(step: &str, error: Error) -> String {
        log::warn!("screen capture: {step} failed: {error}");
        START_FAILED.to_string()
    }

    fn capture_item(kind: &SourceKind, handle: usize) -> WinResult<GraphicsCaptureItem> {
        let interop = ::windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let raw = handle as *mut c_void;
        match kind {
            // SAFETY: the handle came from sources() and start() checked it; a stale one just fails.
            SourceKind::Screen => unsafe { interop.CreateForMonitor(HMONITOR(raw)) },
            // SAFETY: as above.
            SourceKind::Window => unsafe { interop.CreateForWindow(HWND(raw)) },
        }
    }

    fn d3d_device() -> WinResult<(ID3D11Device, ID3D11DeviceContext)> {
        let mut last_error = Error::from(E_FAIL);
        // WARP (software) covers machines without a usable graphics driver.
        for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
            let (mut device, mut context) = (None, None);
            // SAFETY: out-pointers to locals; the default adapter and feature levels.
            let made = unsafe {
                D3D11CreateDevice(
                    None::<&IDXGIAdapter>,
                    driver,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
            };
            match (made, device, context) {
                (Ok(()), Some(device), Some(context)) => {
                    // The capture system uses this device from its own threads too.
                    if let Ok(multithread) = context.cast::<ID3D11Multithread>() {
                        // SAFETY: only turns on D3D's internal locking.
                        let _ = unsafe { multithread.SetMultithreadProtected(true) };
                    }
                    return Ok((device, context));
                }
                (Err(error), ..) => last_error = error,
                _ => {}
            }
        }
        Err(last_error)
    }

    fn winrt_device(device: &ID3D11Device) -> WinResult<IDirect3DDevice> {
        let dxgi: IDXGIDevice = device.cast()?;
        // SAFETY: wraps a live DXGI device for WinRT and returns a new reference.
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? };
        inspectable.cast()
    }

    fn frame_texture(frame: &Direct3D11CaptureFrame) -> WinResult<ID3D11Texture2D> {
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        // SAFETY: asks the surface for the D3D11 texture behind it (a new reference).
        unsafe { access.GetInterface() }
    }

    /// Windows 11 draws a yellow border around what's being captured; ask it not to. Older
    /// versions lack the switch, which is fine.
    fn hide_border(session: &GraphicsCaptureSession) {
        if let Ok(request) = GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Borderless) {
            let (done_tx, done_rx) = mpsc::channel();
            let done = move |_| {
                let _ = done_tx.send(());
            };
            if request.when(done).is_ok() {
                // Instant for desktop apps; never hold up the capture for it.
                let _ = done_rx.recv_timeout(BORDER_WAIT);
            }
        }
        let _ = session.SetIsBorderRequired(false);
    }

    // ---- Sound ----

    fn audio_thread(on_audio: OnAudio, stop: Arc<AtomicBool>) {
        let _com = ComScope::multithreaded();
        if let Err(error) = capture_sound(on_audio, &stop) {
            log::warn!("screen capture: sound stopped: {error}");
        }
    }

    fn capture_sound(mut on_audio: OnAudio, stop: &AtomicBool) -> WinResult<()> {
        // Made before the client so it is closed after the client is released.
        let event = Event::new()?;
        let Some((client, format)) = loopback_client(stop)? else { return Ok(()) };
        // SAFETY: `event` outlives the client's use of it (the client is stopped first).
        unsafe { client.SetEventHandle(event.0)? };
        // SAFETY: plain COM call on an initialised client.
        let capture: IAudioCaptureClient = unsafe { client.GetService()? };
        // SAFETY: as above.
        unsafe { client.Start()? };
        let pumped = pump(&capture, format, &event, stop, &mut on_audio);
        // SAFETY: as above; stopping a stopped client is harmless.
        let _ = unsafe { client.Stop() };
        pumped
    }

    fn pump(
        capture: &IAudioCaptureClient,
        format: SampleFormat,
        event: &Event,
        stop: &AtomicBool,
        on_audio: &mut OnAudio,
    ) -> WinResult<()> {
        let mut samples: Vec<f32> = Vec::new();
        while !stop.load(Ordering::Relaxed) {
            // SAFETY: `event` is a live event handle; the timeout keeps stop() responsive when nothing plays.
            unsafe { WaitForSingleObject(event.0, SOUND_POLL.as_millis() as u32) };
            // SAFETY: plain COM call on a started capture client.
            while unsafe { capture.GetNextPacketSize()? } > 0 {
                let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                // SAFETY: out-pointers to locals.
                unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)? };
                let count = frames as usize * CHANNELS;
                samples.clear();
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    samples.resize(count, 0.0);
                } else {
                    // SAFETY: until ReleaseBuffer, `data` holds `frames` frames in the format we asked for.
                    let bytes = unsafe { std::slice::from_raw_parts(data, frames as usize * format.bytes_per_frame()) };
                    format.convert(bytes, &mut samples);
                }
                // SAFETY: gives back exactly what GetBuffer handed out; `bytes` is no longer used.
                unsafe { capture.ReleaseBuffer(frames)? };
                if !samples.is_empty() {
                    on_audio(&samples);
                }
            }
        }
        Ok(())
    }

    /// A WASAPI client for "everything this PC plays except Yap", set up for 48 kHz stereo.
    /// None if stop() came while Windows was still preparing it.
    fn loopback_client(stop: &AtomicBool) -> WinResult<Option<(IAudioClient, SampleFormat)>> {
        let Some(client) = activate_loopback(stop)? else { return Ok(None) };
        match initialize(&client, SampleFormat::F32) {
            Ok(()) => return Ok(Some((client, SampleFormat::F32))),
            Err(error) => log::info!("screen capture: float sound refused ({error}); trying 16-bit"),
        }
        // A client whose Initialize failed can't be used again, so ask for a new one.
        let Some(client) = activate_loopback(stop)? else { return Ok(None) };
        initialize(&client, SampleFormat::I16)?;
        Ok(Some((client, SampleFormat::I16)))
    }

    fn initialize(client: &IAudioClient, format: SampleFormat) -> WinResult<()> {
        let block_align = format.bytes_per_frame() as u16;
        let wave = WAVEFORMATEX {
            wFormatTag: format.tag(),
            nChannels: CHANNELS as u16,
            nSamplesPerSec: SAMPLE_RATE,
            nAvgBytesPerSec: SAMPLE_RATE * u32::from(block_align),
            nBlockAlign: block_align,
            wBitsPerSample: format.bits(),
            cbSize: 0,
        };
        let flags = AUDCLNT_STREAMFLAGS_LOOPBACK
            | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        // SAFETY: `wave` is a complete WAVEFORMATEX (no extra bytes) that outlives the call.
        unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, SOUND_BUFFER, 0, &wave, None) }
    }

    /// Asks Windows for a loopback client that leaves out this process and its children.
    fn activate_loopback(stop: &AtomicBool) -> WinResult<Option<IAudioClient>> {
        // Boxed: if we give up waiting, Windows may still read them, so they are leaked then.
        let mut params = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    // SAFETY: no preconditions.
                    TargetProcessId: unsafe { GetCurrentProcessId() },
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
                },
            },
        });
        let mut blob = Box::new(PROPVARIANT::default());
        // SAFETY: fills in the blob variant of a zeroed PROPVARIANT, pointing at `params`, which
        // stays alive (or is leaked) until Windows is done with it.
        unsafe {
            let variant = &mut *blob.Anonymous.Anonymous;
            variant.vt = VT_BLOB;
            variant.Anonymous.blob = BLOB {
                cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                pBlobData: (&raw mut *params).cast::<u8>(),
            };
        }

        let (done_tx, done_rx) = mpsc::channel();
        let handler: IActivateAudioInterfaceCompletionHandler = ActivationDone(done_tx).into();
        // SAFETY: the pointers stay valid until the activation finishes (we wait, or leak them).
        let operation = unsafe {
            ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, &IAudioClient::IID, Some(&*blob), &handler)?
        };

        let give_up = Instant::now() + ACTIVATION_WAIT;
        let finished = loop {
            if stop.load(Ordering::Relaxed) || Instant::now() >= give_up {
                break false;
            }
            match done_rx.recv_timeout(SOUND_POLL) {
                Ok(()) => break true,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break false,
            }
        };
        if !finished {
            Box::leak(params);
            Box::leak(blob);
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            return Err(Error::new(E_FAIL, "Windows didn't set up sound capture in time"));
        }

        let mut result = HRESULT(0);
        let mut activated: Option<IUnknown> = None;
        // SAFETY: out-pointers to locals; the operation has finished.
        unsafe { operation.GetActivateResult(&mut result, &mut activated)? };
        result.ok()?;
        let client = activated.ok_or_else(|| Error::from(E_NOINTERFACE))?.cast()?;
        Ok(Some(client))
    }

    /// Wakes the audio thread when Windows has finished activating the loopback client.
    /// `#[implement]` also makes it agile (IAgileObject plus the free-threaded marshaler), which
    /// Windows requires because it calls back from one of its own threads.
    #[implement(IActivateAudioInterfaceCompletionHandler)]
    struct ActivationDone(mpsc::Sender<()>);

    impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationDone_Impl {
        fn ActivateCompleted(&self, _operation: Ref<'_, IActivateAudioInterfaceAsyncOperation>) -> WinResult<()> {
            let _ = self.0.send(());
            Ok(())
        }
    }

    /// An auto-reset event, closed when dropped.
    struct Event(HANDLE);

    impl Event {
        fn new() -> WinResult<Self> {
            // SAFETY: an unnamed auto-reset event with default security.
            unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.map(Event)
        }
    }

    impl Drop for Event {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateEventW and is closed exactly once.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn screen_names() {
        assert_eq!(screen_name(1, 2560, 1440), "Screen 1 (2560 × 1440)");
        assert_eq!(screen_name(2, 0, 0), "Screen 2");
    }

    #[test]
    fn window_names() {
        assert_eq!(window_name("Inbox", Some(r"C:\Program Files\Mail\mail.exe")), "Inbox — mail");
        assert_eq!(window_name("  Inbox  ", Some(r"C:\Apps\MAIL.EXE")), "Inbox — MAIL");
        // The app is already in the title.
        assert_eq!(window_name("notes.txt - Notepad", Some(r"C:\Windows\notepad.exe")), "notes.txt - Notepad");
        assert_eq!(window_name("Yap - Google Chrome", Some(r"C:\Chrome\chrome.exe")), "Yap - Google Chrome");
        // Store apps all run inside ApplicationFrameHost, which says nothing useful.
        assert_eq!(window_name("Settings", Some(r"C:\Windows\System32\ApplicationFrameHost.exe")), "Settings");
        assert_eq!(window_name("Settings", None), "Settings");
    }

    #[test]
    fn app_names() {
        assert_eq!(app_name(r"C:\a\Code.exe"), Some("Code"));
        assert_eq!(app_name("C:/a/tool"), Some("tool"));
        assert_eq!(app_name(r"C:\a\Ünïcødé.exe"), Some("Ünïcødé"));
        assert_eq!(app_name(r"C:\a\é.exe"), Some("é"));
        assert_eq!(app_name(r"C:\a\"), None);
        assert_eq!(app_name(".exe"), None);
    }

    #[test]
    fn source_ids() {
        assert_eq!(parse_source_id("m:123"), Some((SourceKind::Screen, 123)));
        assert_eq!(parse_source_id("w:42"), Some((SourceKind::Window, 42)));
        assert_eq!(parse_source_id("x:1"), None);
        assert_eq!(parse_source_id("m:"), None);
        assert_eq!(parse_source_id("m:0"), None);
        assert_eq!(parse_source_id("w:abc"), None);
        assert_eq!(parse_source_id("42"), None);
    }

    #[test]
    fn pacing() {
        assert_eq!(frame_interval(30), Duration::from_nanos(33_333_333));
        assert_eq!(frame_interval(0), Duration::from_secs(1));
        let t = Instant::now();
        // A little late: keep the even pace.
        assert_eq!(next_due(t, t + ms(5), ms(33)), t + ms(33));
        // After a quiet spell: a full interval from now, no burst.
        assert_eq!(next_due(t, t + ms(1000), ms(33)), t + ms(1033));
    }

    #[test]
    fn rows_are_packed() {
        // Two rows of 6 bytes, 8 bytes apart; the last row has no padding.
        let src = [1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12];
        assert_eq!(copy_rows(&src, 8, 6, 2), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        assert_eq!(copy_rows(&src[..6], 6, 6, 1), vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn sixteen_bit_to_float() {
        let mut out = Vec::new();
        let bytes = [0x00, 0x80, 0x00, 0x00, 0xff, 0x7f, 0x00, 0x40, 0x99];
        SampleFormat::I16.convert(&bytes, &mut out);
        assert_eq!(out, vec![-1.0, 0.0, 32767.0 / 32768.0, 0.5]);
    }

    #[test]
    fn float_passthrough() {
        let mut out = vec![9.0];
        let bytes: Vec<u8> = [0.25f32, -1.0].iter().flat_map(|s| s.to_le_bytes()).collect();
        SampleFormat::F32.convert(&bytes, &mut out);
        assert_eq!(out, vec![9.0, 0.25, -1.0]);
    }

    #[test]
    fn sample_formats() {
        assert_eq!((SampleFormat::F32.tag(), SampleFormat::F32.bytes_per_frame()), (3, 8));
        assert_eq!((SampleFormat::I16.tag(), SampleFormat::I16.bytes_per_frame()), (1, 4));
    }

    #[test]
    fn builds() {
        assert_eq!(parse_build("22631\0"), Some(22631));
        assert_eq!(parse_build(" 19041 "), Some(19041));
        assert_eq!(parse_build("abc"), None);
        assert!(!build_supports_sound(18363));
        assert!(build_supports_sound(19041));
        assert!(build_supports_sound(26100));
    }
}
