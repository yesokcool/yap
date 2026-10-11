//! macOS screen capture through ScreenCaptureKit (macOS 12.3+; sound needs macOS 13+).
//!
//! Frames and sound arrive on two serial dispatch queues we create. A delegate turns a stream
//! that stops by itself (permission revoked, stopped from the menu bar, display gone) into
//! `on_ended`, and a small watchdog thread catches a shared window being closed.
//!
//! What's shared is chosen two ways:
//! - macOS 14+: Apple's own picker (`SCContentSharingPicker`). Yap never lists screens or
//!   windows itself there, so macOS 15+ doesn't keep asking whether Yap may "bypass the
//!   private window picker". The picked content filter goes into one stream that lives for
//!   the whole share ("held"): idle at 1 fps while nobody watches, the share's quality while
//!   someone does. "Share something else" re-opens the picker for that stream and swaps its
//!   filter in place, and the menu bar's stop ends the share in Yap.
//! - macOS 12.3–13: our own list (`sources`), a stream per viewing session, as before.

use super::*;

use std::mem::{offset_of, size_of};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadMarker, Message};
use objc2_core_audio_types::{
    kAudioFormatFlagIsFloat, kAudioFormatLinearPCM, AudioBuffer, AudioBufferList,
};
use objc2_core_foundation::{
    CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType, CGRect, Type,
};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, kCGNullWindowID, kCGWindowNumber, kCGWindowOwnerPID, CGDirectDisplayID,
    CGDisplayCopyDisplayMode, CGDisplayIsOnline, CGDisplayMode, CGMainDisplayID,
    CGPreflightScreenCaptureAccess, CGWindowID, CGWindowListCopyWindowInfo, CGWindowListOption,
};
use objc2_core_media::{
    kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
    CMAudioFormatDescriptionGetStreamBasicDescription, CMBlockBuffer, CMSampleBuffer, CMTime,
};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, kCVReturnSuccess, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferIsPlanar, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSError, NSNumber, NSOperatingSystemVersion, NSProcessInfo};
use objc2_screen_capture_kit::{
    SCContentFilter, SCContentSharingPicker, SCContentSharingPickerConfiguration,
    SCContentSharingPickerMode, SCContentSharingPickerObserver, SCDisplay, SCFrameStatus,
    SCShareableContent, SCShareableContentStyle, SCStream, SCStreamConfiguration,
    SCStreamDelegate, SCStreamErrorCode, SCStreamErrorDomain, SCStreamFrameInfoStatus,
    SCStreamOutput, SCStreamOutputType, SCWindow,
};

const PERMISSION_MESSAGE: &str =
    "Yap needs Screen Recording permission: System Settings › Privacy & Security › Screen Recording.";
const TOO_OLD_MESSAGE: &str = "Screen sharing needs macOS 12.3 or newer.";
/// Ids of what Apple's picker handed us start with this ("picked:3").
const PICKED: &str = "picked:";
/// A held stream nobody watches still runs (so the share and its menu-bar indicator stay
/// up), at this many frames a second, and nothing is copied out of it.
const IDLE_FPS: u32 = 1;
/// How long we wait for macOS to list screens and windows.
const LIST_TIMEOUT: Duration = Duration::from_secs(3);
/// How long start() waits to hear whether the stream started (it keeps going after that).
const START_WAIT: Duration = Duration::from_millis(300);
/// How long stop() waits for macOS to confirm the stream stopped.
const STOP_WAIT: Duration = Duration::from_millis(500);
/// How often the watchdog checks that the shared window or screen still exists.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);
/// Windows smaller than this (in points) are skipped: tool tips, badges, status items.
const MIN_WINDOW_POINTS: f64 = 50.0;
const SAMPLE_RATE: u32 = 48_000;
/// Frames ScreenCaptureKit may hold for us before it drops new ones.
const QUEUE_DEPTH: isize = 5;

// ---------------------------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------------------------

pub fn sources() -> Result<Vec<Source>, String> {
    if !screen_capture_kit_available() {
        return Err(TOO_OLD_MESSAGE.into());
    }
    let content = shareable_content(true)?;
    let own_pid = own_pid();

    let mut displays: Vec<DisplayInfo> = displays_of(&content)
        .iter()
        .map(|d| DisplayInfo::of(d))
        .collect();
    // The main screen first, then the rest in the order macOS gives them.
    let main_id = CGMainDisplayID();
    displays.sort_by_key(|d| d.id != main_id);

    let mut list = Vec::new();
    for (index, display) in displays.iter().enumerate() {
        let (width, height) = display.pixel_size();
        list.push(Source {
            id: Target::Display(display.id).id(),
            name: screen_name(index + 1, width, height),
            kind: SourceKind::Screen,
        });
    }
    for window in windows_of(&content) {
        let window = WindowInfo::of(&window);
        if window.is_listed(own_pid) {
            list.push(Source {
                id: Target::Window(window.id).id(),
                name: window_name(&window.title, &window.app_name),
                kind: SourceKind::Window,
            });
        }
    }
    Ok(list)
}

pub fn start(
    source: &Source,
    options: Options,
    on_frame: OnFrame,
    on_audio: Option<OnAudio>,
    on_ended: OnEnded,
) -> Result<Box<dyn Capture>, String> {
    if !screen_capture_kit_available() {
        return Err(TOO_OLD_MESSAGE.into());
    }
    if source.id.starts_with(PICKED) {
        return start_held(source, options, on_frame, on_audio);
    }
    let target = Target::parse(&source.id).ok_or("That screen or window can't be shared.")?;
    // All windows, not just on-screen ones: one minimised (or moved to another Space) since the
    // picker was shown is still found.
    let content = shareable_content(false)?;
    let (filter, pixel_size) = make_filter(&content, target)?;

    let sound = options.sound && on_audio.is_some() && sound_supported();
    let shared = Shared::new(Some(on_frame), if sound { on_audio } else { None }, false);
    let config = make_config(pixel_size, &options, sound);
    let run = Running::open(&filter, &config, shared, sound)?;
    let mut capture = MacCapture {
        run,
        pixel_size,
        sound,
        watchdog: None,
        done: false,
    };

    let started = capture
        .run
        .stream
        .start(&capture.run.shared)
        // Only now does on_ended take over: anything that ended the stream before this point is
        // returned as an error instead.
        .and_then(|()| capture.run.shared.arm(on_ended));
    if let Err(message) = started {
        capture.stop();
        return Err(message);
    }
    capture.watchdog = spawn_watchdog(
        target,
        capture.run.shared.clone(),
        capture.run.stream.clone(),
    );
    Ok(Box::new(capture))
}

pub fn sound_supported() -> bool {
    // capturesAudio and excludesCurrentProcessAudio arrived in macOS 13.0.
    screen_capture_kit_available() && os_at_least(13, 0)
}

// ---------------------------------------------------------------------------------------------
// The running capture
// ---------------------------------------------------------------------------------------------

/// One SCStream with our output on two queues of our own. Closed once (by `close`, or when
/// dropped).
struct Running {
    stream: Arc<StreamHandle>,
    output: Retained<CaptureOutput>,
    shared: Arc<Shared>,
    // The stream delivers on these; keep them for as long as the stream lives.
    _video_queue: DispatchRetained<DispatchQueue>,
    audio_queue: Option<DispatchRetained<DispatchQueue>>,
    closed: AtomicBool,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.close();
    }
}

impl Running {
    /// Makes the stream (not started yet) with our output for pictures, and for sound if
    /// `sound` (a stream that can't take sound shares without it rather than not at all).
    fn open(
        filter: &SCContentFilter,
        config: &SCStreamConfiguration,
        shared: Arc<Shared>,
        sound: bool,
    ) -> Result<Running, String> {
        let output = CaptureOutput::new(shared.clone());
        let delegate = ProtocolObject::<dyn SCStreamDelegate>::from_ref(&*output);
        // SAFETY: filter, config and delegate are valid; we keep the delegate alive (see close()).
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                filter,
                config,
                Some(delegate),
            )
        };

        let stream_output = ProtocolObject::<dyn SCStreamOutput>::from_ref(&*output);
        let video_queue = DispatchQueue::new("yap.screen.video", None);
        // SAFETY: a serial queue we own; the output is kept alive by Running.
        unsafe {
            stream.addStreamOutput_type_sampleHandlerQueue_error(
                stream_output,
                SCStreamOutputType::Screen,
                Some(&video_queue),
            )
        }
        .map_err(|error| describe_error(&error))?;

        let audio_queue = if sound {
            let queue = DispatchQueue::new("yap.screen.audio", None);
            // SAFETY: as above.
            unsafe {
                stream.addStreamOutput_type_sampleHandlerQueue_error(
                    stream_output,
                    SCStreamOutputType::Audio,
                    Some(&queue),
                )
            }
            .ok()
            .map(|()| queue)
        } else {
            None
        };
        Ok(Running {
            stream: Arc::new(StreamHandle {
                stream,
                stop_requested: AtomicBool::new(false),
                stop_done: Arc::new((Mutex::new(false), Condvar::new())),
            }),
            output,
            shared,
            _video_queue: video_queue,
            audio_queue,
            closed: AtomicBool::new(false),
        })
    }

    fn update_config(&self, config: &SCStreamConfiguration) {
        // SAFETY: a valid configuration; no completion handler. If macOS refuses the change the
        // stream keeps its old size and frame rate, which is still a working share.
        unsafe {
            self.stream
                .stream
                .updateConfiguration_completionHandler(config, None)
        };
    }

    /// Stops the stream (waiting a moment for macOS to confirm) and lets go of the app's
    /// callbacks. From here on no frame, sound or on_ended reaches the app.
    fn close(&self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.stream.request_stop();
        let confirmed = self.stream.wait_stopped(STOP_WAIT);
        let output = ProtocolObject::<dyn SCStreamOutput>::from_ref(&*self.output);
        // SAFETY: removing outputs we added; errors (e.g. never added) don't matter here.
        unsafe {
            let _ = self
                .stream
                .stream
                .removeStreamOutput_type_error(output, SCStreamOutputType::Screen);
            if self.audio_queue.is_some() {
                let _ = self
                    .stream
                    .stream
                    .removeStreamOutput_type_error(output, SCStreamOutputType::Audio);
            }
        }
        if !confirmed {
            // macOS didn't confirm in time and may still call the delegate, which it might not
            // retain. Keep that small object alive for good rather than risk a crash.
            std::mem::forget(self.output.clone());
        }
        self.shared.release_callbacks();
    }
}

/// A capture from our own list (macOS 12.3–13): its own stream, for one viewing session.
struct MacCapture {
    run: Running,
    /// The source's full size in pixels; options scale down from this.
    pixel_size: (u32, u32),
    /// Whether sound was turned on at start (sound changes later are ignored, as the spec allows).
    sound: bool,
    /// Dropping this tells the watchdog thread to finish.
    watchdog: Option<mpsc::Sender<()>>,
    done: bool,
}

impl Capture for MacCapture {
    fn set_options(&mut self, options: Options) {
        if self.done {
            return;
        }
        self.run
            .update_config(&make_config(self.pixel_size, &options, self.sound));
    }

    fn stop(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        // Wake the watchdog so it exits; we don't join it, it never blocks for long.
        self.watchdog = None;
        self.run.close();
    }
}

impl Drop for MacCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The SCStream plus what we need to stop it from any thread.
struct StreamHandle {
    stream: Retained<SCStream>,
    stop_requested: AtomicBool,
    /// Set by the stop completion handler.
    stop_done: Arc<(Mutex<bool>, Condvar)>,
}

// SAFETY: SCStream's methods are safe to call from any thread (they report back through
// completion handlers); we only call start/stop/update/remove on it.
unsafe impl Send for StreamHandle {}
// SAFETY: see above.
unsafe impl Sync for StreamHandle {}

impl StreamHandle {
    /// Starts the stream and waits briefly for the result. A failure that arrives after we stop
    /// waiting goes to `on_ended` instead.
    fn start(&self, shared: &Arc<Shared>) -> Result<(), String> {
        type Slot = Arc<Mutex<Option<mpsc::Sender<Result<(), String>>>>>;
        let (tx, rx) = mpsc::channel();
        let slot: Slot = Arc::new(Mutex::new(Some(tx)));
        let handler = {
            let slot = slot.clone();
            let shared = shared.clone();
            RcBlock::new(move |error: *mut NSError| {
                // SAFETY: ScreenCaptureKit passes a valid NSError or null.
                let result = match unsafe { error.as_ref() } {
                    Some(error) => Err(describe_error(error)),
                    None => Ok(()),
                };
                let waiting = slot.lock().ok().and_then(|mut slot| slot.take());
                match waiting {
                    Some(tx) => {
                        let _ = tx.send(result);
                    }
                    None => {
                        if let Err(message) = result {
                            shared.end_by_itself(message);
                        }
                    }
                }
            })
        };
        // SAFETY: a valid stream and completion handler.
        unsafe {
            self.stream
                .startCaptureWithCompletionHandler(Some(&handler))
        };
        if let Ok(result) = rx.recv_timeout(START_WAIT) {
            return result;
        }
        // Still starting: stop waiting, but take an answer that slipped in meanwhile.
        if let Ok(mut slot) = slot.lock() {
            slot.take();
        }
        rx.try_recv().unwrap_or(Ok(()))
    }

    /// Starts the stream without waiting: a failure goes to `on_ended` (already armed).
    fn start_in_background(&self, shared: &Arc<Shared>) {
        let shared = shared.clone();
        let handler = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: ScreenCaptureKit passes a valid NSError or null.
            if let Some(error) = unsafe { error.as_ref() } {
                shared.end_by_itself(describe_error(error));
            }
        });
        // SAFETY: a valid stream and completion handler.
        unsafe {
            self.stream
                .startCaptureWithCompletionHandler(Some(&handler))
        };
    }

    /// Asks the stream to stop (once); doesn't wait.
    fn request_stop(&self) {
        if self.stop_requested.swap(true, Ordering::SeqCst) {
            return;
        }
        let done = self.stop_done.clone();
        let handler = RcBlock::new(move |_error: *mut NSError| {
            // An error here means it was already stopped, which is just as good.
            let (lock, cvar) = &*done;
            if let Ok(mut stopped) = lock.lock() {
                *stopped = true;
            }
            cvar.notify_all();
        });
        // SAFETY: a valid stream and completion handler.
        unsafe { self.stream.stopCaptureWithCompletionHandler(Some(&handler)) };
    }

    /// Waits up to `timeout` for the stop to be confirmed.
    fn wait_stopped(&self, timeout: Duration) -> bool {
        let (lock, cvar) = &*self.stop_done;
        let Ok(guard) = lock.lock() else { return false };
        match cvar.wait_timeout_while(guard, timeout, |stopped| !*stopped) {
            Ok((stopped, _)) => *stopped,
            Err(_) => false,
        }
    }
}

/// State shared between the app-facing handle and the ScreenCaptureKit callbacks.
struct Shared {
    /// Set once the capture is over (stopped by us or ended by itself).
    stopped: AtomicBool,
    /// None while a held stream has nobody attached: its pictures aren't copied then.
    on_frame: Mutex<Option<OnFrame>>,
    audio: Mutex<AudioSink>,
    end: Mutex<EndState>,
    /// Held streams only: the newest picture while nobody's attached (just a reference to
    /// macOS's buffer, nothing copied), handed over at once when someone is. ScreenCaptureKit
    /// only delivers changes, so a still screen would otherwise show nothing.
    last: Option<Mutex<Option<Sample>>>,
}

/// A picture from ScreenCaptureKit, kept by reference.
struct Sample(CFRetained<CMSampleBuffer>);
// SAFETY: a sample buffer is immutable once delivered and CoreMedia's retain/release is
// thread-safe; we only read it (copy_frame locks the pixel buffer read-only).
unsafe impl Send for Sample {}

struct AudioSink {
    on_audio: Option<OnAudio>,
    /// Reused storage for the AudioBufferList (u64 for alignment).
    list: Vec<u64>,
    /// Reused interleaved output.
    samples: Vec<f32>,
}

struct EndState {
    /// Installed by start() once the stream is running.
    callback: Option<OnEnded>,
    /// Why the stream ended if that happened before start() finished.
    early: Option<String>,
}

impl Shared {
    /// `held`: a stream from Apple's picker that outlives viewing sessions (see `Held`).
    fn new(on_frame: Option<OnFrame>, on_audio: Option<OnAudio>, held: bool) -> Arc<Shared> {
        Arc::new(Shared {
            stopped: AtomicBool::new(false),
            on_frame: Mutex::new(on_frame),
            audio: Mutex::new(AudioSink {
                on_audio,
                list: Vec::new(),
                samples: Vec::new(),
            }),
            end: Mutex::new(EndState {
                callback: None,
                early: None,
            }),
            last: held.then(|| Mutex::new(None)),
        })
    }

    /// Someone watches a held stream: pictures (and sound, if given) go to them from now on,
    /// starting with the newest one we have.
    fn attach(&self, mut on_frame: OnFrame, on_audio: Option<OnAudio>) {
        if let Ok(mut audio) = self.audio.lock() {
            audio.on_audio = on_audio;
        }
        let Ok(mut slot) = self.on_frame.lock() else {
            return;
        };
        let last = self.last.as_ref().and_then(|l| l.lock().ok()?.take());
        if let Some(frame) = last.and_then(|s| copy_frame(&s.0)) {
            let _ = catch_unwind(AssertUnwindSafe(|| on_frame(frame)));
        }
        *slot = Some(on_frame);
    }

    /// Nobody watches anymore: keep the stream, drop the callbacks.
    fn detach(&self) {
        if let Ok(mut audio) = self.audio.lock() {
            audio.on_audio = None;
        }
        if let Ok(mut slot) = self.on_frame.lock() {
            *slot = None;
        }
    }

    /// The content changed (something else is shared): the kept picture is stale.
    fn forget_last(&self) {
        if let Some(mut last) = self.last.as_ref().and_then(|l| l.lock().ok()) {
            *last = None;
        }
    }

    /// The capture ended without the app asking: tell the app once.
    fn end_by_itself(&self, message: String) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let Ok(mut end) = self.end.lock() else { return };
        match end.callback.take() {
            Some(callback) => {
                drop(end);
                let _ = catch_unwind(AssertUnwindSafe(|| callback(message)));
            }
            None => end.early = Some(message),
        }
    }

    /// Hands `on_ended` over, unless the capture already ended while starting.
    fn arm(&self, on_ended: OnEnded) -> Result<(), String> {
        let mut end = self
            .end
            .lock()
            .map_err(|_| "Screen sharing couldn't start.".to_string())?;
        if let Some(message) = end.early.take() {
            return Err(message);
        }
        end.callback = Some(on_ended);
        Ok(())
    }

    /// Drops the app's callbacks so whatever they hold is released. Skips any that a callback
    /// is running right now (it would deadlock); those go when the last reference does.
    fn release_callbacks(&self) {
        if let Ok(mut on_frame) = self.on_frame.try_lock() {
            on_frame.take();
        }
        if let Ok(mut audio) = self.audio.try_lock() {
            audio.on_audio.take();
        }
        if let Ok(mut end) = self.end.try_lock() {
            end.callback.take();
        }
        if let Some(mut last) = self.last.as_ref().and_then(|l| l.try_lock().ok()) {
            last.take();
        }
    }

    fn handle_video(&self, sample: &CMSampleBuffer) {
        if !frame_is_complete(sample) {
            return;
        }
        let Ok(mut on_frame) = self.on_frame.lock() else {
            return;
        };
        match on_frame.as_mut() {
            Some(on_frame) => {
                if let Some(frame) = copy_frame(sample) {
                    // A panic must not unwind into ScreenCaptureKit.
                    let _ = catch_unwind(AssertUnwindSafe(|| on_frame(frame)));
                }
            }
            None => {
                if let Some(mut last) = self.last.as_ref().and_then(|l| l.lock().ok()) {
                    *last = Some(Sample(Type::retain(sample)));
                }
            }
        }
    }

    fn handle_audio(&self, sample: &CMSampleBuffer) {
        let Ok(mut sink) = self.audio.lock() else {
            return;
        };
        let AudioSink {
            on_audio,
            list,
            samples,
        } = &mut *sink;
        let Some(on_audio) = on_audio.as_mut() else {
            return;
        };
        if read_audio(sample, list, samples) && !samples.is_empty() {
            let _ = catch_unwind(AssertUnwindSafe(|| on_audio(samples)));
        }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and CaptureOutput doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[ivars = Arc<Shared>]
    struct CaptureOutput;

    // SAFETY: NSObjectProtocol has no requirements beyond being an NSObject.
    unsafe impl NSObjectProtocol for CaptureOutput {}

    // SAFETY: the method signature matches the protocol.
    unsafe impl SCStreamOutput for CaptureOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_did_output(
            &self,
            _stream: &SCStream,
            sample: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            let shared = self.ivars();
            if shared.stopped.load(Ordering::SeqCst) {
                return;
            }
            if kind == SCStreamOutputType::Screen {
                shared.handle_video(sample);
            } else if kind == SCStreamOutputType::Audio {
                shared.handle_audio(sample);
            }
        }
    }

    // SAFETY: the method signature matches the protocol.
    unsafe impl SCStreamDelegate for CaptureOutput {
        #[unsafe(method(stream:didStopWithError:))]
        fn stream_did_stop(&self, _stream: &SCStream, error: &NSError) {
            self.ivars().end_by_itself(describe_error(error));
        }
    }
);

impl CaptureOutput {
    fn new(shared: Arc<Shared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(shared);
        // SAFETY: plain NSObject init.
        unsafe { msg_send![super(this), init] }
    }
}

/// Checks once a second that the source still exists. ScreenCaptureKit doesn't reliably stop
/// (or tell us) when a shared window is closed, so we look ourselves.
fn spawn_watchdog(
    target: Target,
    shared: Arc<Shared>,
    stream: Arc<StreamHandle>,
) -> Option<mpsc::Sender<()>> {
    let (tx, rx) = mpsc::channel::<()>();
    thread::Builder::new()
        .name("yap-capture-watchdog".into())
        .spawn(move || {
            // Ends when stop() drops the sender (Disconnected) or the source goes away.
            while let Err(RecvTimeoutError::Timeout) = rx.recv_timeout(WATCH_INTERVAL) {
                if shared.stopped.load(Ordering::SeqCst) {
                    break;
                }
                if !target.still_exists() {
                    shared.end_by_itself(target.gone_message().into());
                    stream.request_stop();
                    break;
                }
            }
        })
        .ok()
        .map(|_| tx)
}

// ---------------------------------------------------------------------------------------------
// macOS 14+: Apple's picker, and the one stream that lives for the whole share
// ---------------------------------------------------------------------------------------------

/// True when what's shared is chosen in Apple's own picker (macOS 14+).
pub fn system_picker() -> bool {
    screen_capture_kit_available()
        && os_at_least(14, 0)
        && AnyClass::get(c"SCContentSharingPicker").is_some()
}

/// Apple's picker is one per app, and so is this.
struct Picker {
    /// Added to the picker the first time it opens, kept for good.
    observer: Option<Retained<PickerObserver>>,
    /// Waiting for a pick (Share screen).
    pending: Option<OnPicked>,
    next: u64,
    /// Picked but not shared yet, newest last (a few at most).
    picked: Vec<(String, Retained<SCContentFilter>)>,
    /// The stream of the share going on now.
    held: Option<Held>,
}

// SAFETY: the ScreenCaptureKit objects in here are only used through calls that are fine from
// any thread (content filters don't change; SCStream answers through completion handlers);
// calls on the picker itself go to the main thread (`on_main`).
unsafe impl Send for Picker {}

static PICKER: Mutex<Picker> = Mutex::new(Picker {
    observer: None,
    pending: None,
    next: 0,
    picked: Vec::new(),
    held: None,
});

fn picker_state() -> std::sync::MutexGuard<'static, Picker> {
    PICKER.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs `f` on the main thread (the picker is UI): right away if we're on it, else soon.
fn on_main(f: impl FnOnce() + Send + 'static) {
    if MainThreadMarker::new().is_some() {
        f();
    } else {
        DispatchQueue::main().exec_async(f);
    }
}

fn shared_picker() -> Retained<SCContentSharingPicker> {
    // SAFETY: a class property; we only get here once system_picker() found the class.
    unsafe { SCContentSharingPicker::sharedPicker() }
}

fn ensure_observer(picker: &SCContentSharingPicker) {
    let mut state = picker_state();
    if state.observer.is_none() {
        let observer = PickerObserver::new();
        // SAFETY: a valid observer, kept alive for good in `state` (the picker may not retain it).
        unsafe { picker.addObserver(ProtocolObject::from_ref(&*observer)) };
        state.observer = Some(observer);
    }
}

/// Share screen: Apple's picker, for a new share (a window, an app or a whole screen; Yap's
/// own windows aren't offered). `on_picked` gets what was picked (a `Source` for
/// `hold`/`start`), None if the picker was cancelled, or why it couldn't open.
pub fn pick(on_picked: OnPicked) {
    if !system_picker() {
        return on_picked(Err("Apple's screen picker needs macOS 14 or newer.".into()));
    }
    let previous = picker_state().pending.replace(on_picked);
    if let Some(previous) = previous {
        previous(Ok(None));
    }
    let own_windows = own_window_ids();
    on_main(move || {
        let picker = shared_picker();
        ensure_observer(&picker);
        let ids: Vec<Retained<NSNumber>> = own_windows.iter().map(|&id| NSNumber::new_u32(id)).collect();
        // SAFETY: plain setters (macOS 14.0+, checked above) on objects we hold.
        unsafe {
            let config = SCContentSharingPickerConfiguration::<AnyObject>::new();
            config.setAllowedPickerModes(
                SCContentSharingPickerMode::SingleWindow
                    | SCContentSharingPickerMode::SingleApplication
                    | SCContentSharingPickerMode::SingleDisplay,
            );
            config.setAllowsChangingSelectedContent(true);
            config.setExcludedWindowIDs(&NSArray::from_retained_slice(&ids));
            picker.setDefaultConfiguration(&config);
            picker.setMaximumStreamCount(Some(&NSNumber::new_isize(1)));
            picker.setActive(true);
            picker.present();
        }
    });
}

/// Share something else: Apple's picker for the running share's stream; what's picked
/// replaces what it shows, without stopping. False when there's no such stream.
pub fn pick_again() -> bool {
    let Some(stream) = picker_state().held.as_ref().map(|h| h.run.stream.clone()) else {
        return false;
    };
    on_main(move || {
        let picker = shared_picker();
        ensure_observer(&picker);
        // SAFETY: a live stream of ours; macOS 14.0+.
        unsafe {
            picker.setActive(true);
            picker.presentPickerForStream(&stream.stream);
        }
    });
    true
}

/// The share starts with something from Apple's picker (or its quality or sound changed):
/// its stream runs from now until `release`, idle while nobody watches. `on_ended` hears it
/// end by itself (stopped from the menu bar, the window closed…). Other ids: nothing to do.
pub fn hold(source: &Source, options: Options, on_ended: OnEnded) {
    if !source.id.starts_with(PICKED) {
        return;
    }
    let mut state = picker_state();
    if let Some(held) = state.held.as_mut().filter(|h| h.id == source.id) {
        // While someone watches, the share's worker sets the options itself.
        held.options = options;
        if !held.attached {
            held.apply_config();
        }
        return;
    }
    let previous = state.held.take();
    let filter = state
        .picked
        .iter()
        .position(|(id, _)| *id == source.id)
        .map(|i| state.picked.remove(i).1);
    drop(state);
    if let Some(previous) = previous {
        previous.close_soon();
    }
    let Some(filter) = filter else {
        return on_ended("That screen or window isn't there anymore.".into());
    };
    match Held::open(source.id.clone(), &filter, options) {
        Ok(held) => {
            // Nothing has started yet, so nothing can have ended early.
            let _ = held.run.shared.arm(on_ended);
            held.run.stream.start_in_background(&held.run.shared);
            log::info!("screen: sharing {:?} (picked in macOS), idle until someone watches", source.name);
            picker_state().held = Some(held);
        }
        Err(why) => on_ended(why),
    }
}

/// The share stopped (or moved on to something else): stop its stream, forget the pick.
pub fn release(source: &Source) {
    if !source.id.starts_with(PICKED) {
        return;
    }
    let mut state = picker_state();
    state.picked.retain(|(id, _)| *id != source.id);
    let held = state.held.take_if(|h| h.id == source.id);
    let idle = state.held.is_none() && state.pending.is_none();
    drop(state);
    if let Some(held) = held {
        log::info!("screen: stopping the picked stream");
        held.close_soon();
    }
    if idle {
        deactivate();
    }
}

/// Nothing to pick for: macOS stops offering Yap in its screen-sharing menu.
fn deactivate() {
    if cfg!(test) {
        // (Unit tests never touch the system picker.)
        return;
    }
    on_main(|| {
        // SAFETY: a plain setter, macOS 14.0+ (only picked ids lead here).
        unsafe { shared_picker().setActive(false) };
    });
}

/// Someone watches a held stream: the share's worker gets its pictures.
fn start_held(
    source: &Source,
    options: Options,
    on_frame: OnFrame,
    on_audio: Option<OnAudio>,
) -> Result<Box<dyn Capture>, String> {
    let mut state = picker_state();
    let held = state
        .held
        .as_mut()
        .filter(|h| h.id == source.id)
        .ok_or("The shared screen or window is gone.")?;
    held.attach(on_frame, if options.sound { on_audio } else { None }, options);
    Ok(Box::new(HeldCapture {
        id: source.id.clone(),
        done: false,
    }))
}

fn with_held(id: &str, f: impl FnOnce(&mut Held)) {
    if let Some(held) = picker_state().held.as_mut().filter(|h| h.id == id) {
        f(held);
    }
}

/// The stream of a share picked in Apple's picker. It runs from the pick until the share
/// stops: macOS's filter goes into one stream, once, and "Share something else" changes
/// that stream's content (`swap`) rather than starting another.
struct Held {
    id: String,
    run: Running,
    /// What it shows, in pixels; options scale down from this.
    pixel_size: (u32, u32),
    /// The share's quality and sound: the stream runs at them while someone watches.
    options: Options,
    attached: bool,
    /// The computer's sound can be captured (macOS 13+). A held stream always captures it,
    /// so Share sound can change without a new stream; it's only passed on when asked for.
    sound: bool,
    watchdog: Option<mpsc::Sender<()>>,
}

// SAFETY: as for `Picker`.
unsafe impl Send for Held {}

impl Held {
    fn open(id: String, filter: &SCContentFilter, options: Options) -> Result<Held, String> {
        let sound = sound_supported();
        let pixel_size = picked_size(filter);
        let idle = Options {
            fps: IDLE_FPS,
            ..options
        };
        let run = Running::open(
            filter,
            &make_config(pixel_size, &idle, sound),
            Shared::new(None, None, true),
            sound,
        )?;
        let watchdog = picked_target(filter)
            .and_then(|t| spawn_watchdog(t, run.shared.clone(), run.stream.clone()));
        Ok(Held {
            id,
            run,
            pixel_size,
            options,
            attached: false,
            sound,
            watchdog,
        })
    }

    fn apply_config(&self) {
        let fps = if self.attached {
            self.options.fps
        } else {
            IDLE_FPS
        };
        let options = Options {
            fps,
            ..self.options
        };
        self.run
            .update_config(&make_config(self.pixel_size, &options, self.sound));
    }

    fn attach(&mut self, on_frame: OnFrame, on_audio: Option<OnAudio>, options: Options) {
        self.options = options;
        self.attached = true;
        self.apply_config();
        self.run.shared.attach(on_frame, on_audio);
    }

    fn detach(&mut self) {
        self.attached = false;
        self.run.shared.detach();
        self.apply_config();
    }

    /// Something else was picked for this stream: same stream, new content.
    fn swap(&mut self, filter: &SCContentFilter) {
        self.pixel_size = picked_size(filter);
        // SAFETY: a valid filter for our live stream. No completion handler: if macOS refuses,
        // the old content stays, which is still a working share.
        unsafe {
            self.run
                .stream
                .stream
                .updateContentFilter_completionHandler(filter, None)
        };
        self.apply_config();
        self.run.shared.forget_last();
        self.watchdog = picked_target(filter)
            .and_then(|t| spawn_watchdog(t, self.run.shared.clone(), self.run.stream.clone()));
    }

    /// Stops the stream on a thread of its own (macOS takes a moment to confirm).
    fn close_soon(self) {
        let Held { run, watchdog, .. } = self;
        drop(watchdog);
        run.shared.stopped.store(true, Ordering::SeqCst);
        // (If no thread can be made, dropping `run` closes it right here.)
        let _ = thread::Builder::new()
            .name("yap-capture-stop".into())
            .spawn(move || run.close());
    }
}

/// What the share's worker holds while someone watches a held stream. Stopping it only
/// detaches: the stream goes back to idle.
struct HeldCapture {
    id: String,
    done: bool,
}

impl Capture for HeldCapture {
    fn set_options(&mut self, options: Options) {
        if !self.done {
            with_held(&self.id, |h| {
                h.options = options;
                h.apply_config();
            });
        }
    }

    fn stop(&mut self) {
        if !self.done {
            self.done = true;
            with_held(&self.id, Held::detach);
        }
    }
}

impl Drop for HeldCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What a picked filter shows, in pixels.
fn picked_size(filter: &SCContentFilter) -> (u32, u32) {
    // SAFETY: plain getters on a valid filter, macOS 14.0+.
    let (rect, scale) = unsafe { (filter.contentRect(), filter.pointPixelScale() as f64) };
    to_pixels(&rect, if scale > 0.0 { scale } else { 1.0 })
}

/// The window or screen a picked filter shows, for the watchdog. Only macOS 15.2+ says; for
/// an app or on older systems there's no watchdog (ScreenCaptureKit's own stop still counts).
fn picked_target(filter: &SCContentFilter) -> Option<Target> {
    if !os_at_least(15, 2) {
        return None;
    }
    // SAFETY: getters that exist since macOS 15.2 (checked above), on a valid filter.
    unsafe {
        match filter.style() {
            SCShareableContentStyle::Window => filter
                .includedWindows()
                .firstObject()
                .map(|w| Target::Window(w.windowID())),
            SCShareableContentStyle::Display => filter
                .includedDisplays()
                .firstObject()
                .map(|d| Target::Display(d.displayID())),
            _ => None,
        }
    }
}

/// The `Source` the app gets for a pick (its name only goes to the log).
fn picked_source(id: String, filter: &SCContentFilter) -> Source {
    let (width, height) = picked_size(filter);
    // SAFETY: a plain getter on a valid filter, macOS 14.0+.
    let style = unsafe { filter.style() };
    let (kind, what) = match style {
        SCShareableContentStyle::Display => (SourceKind::Screen, "a screen"),
        SCShareableContentStyle::Application => (SourceKind::Window, "an app"),
        _ => (SourceKind::Window, "a window"),
    };
    Source {
        id,
        name: format!("{what} picked in macOS ({width} × {height})"),
        kind,
    }
}

fn is_held_stream(state: &Picker, stream: Option<&SCStream>) -> bool {
    match (stream, state.held.as_ref()) {
        (Some(stream), Some(held)) => ptr::eq(stream, &*held.run.stream.stream),
        _ => false,
    }
}

fn picker_updated(filter: &SCContentFilter, stream: Option<&SCStream>) {
    let mut state = picker_state();
    if is_held_stream(&state, stream) {
        if let Some(held) = state.held.as_mut() {
            log::info!("screen: sharing something else (picked in macOS)");
            held.swap(filter);
        }
        return;
    }
    let Some(on_picked) = state.pending.take() else {
        log::info!("screen: macOS picked something nobody asked for; ignored");
        return;
    };
    state.next += 1;
    let id = format!("{PICKED}{}", state.next);
    let source = picked_source(id.clone(), filter);
    state.picked.push((id, filter.retain()));
    if state.picked.len() > 3 {
        state.picked.remove(0);
    }
    drop(state);
    log::info!("screen: picked {}", source.name);
    on_picked(Ok(Some(source)));
}

fn picker_cancelled(stream: Option<&SCStream>) {
    let mut state = picker_state();
    if is_held_stream(&state, stream) {
        // Changing what's shared was cancelled: keep sharing.
        return;
    }
    let on_picked = state.pending.take();
    let idle = state.held.is_none();
    drop(state);
    if idle {
        deactivate();
    }
    if let Some(on_picked) = on_picked {
        on_picked(Ok(None));
    }
}

fn picker_failed(error: &NSError) {
    let mut state = picker_state();
    let on_picked = state.pending.take();
    let idle = state.held.is_none();
    drop(state);
    if idle {
        deactivate();
    }
    if let Some(on_picked) = on_picked {
        on_picked(Err(format!(
            "macOS couldn't open its screen picker: {}",
            error.localizedDescription()
        )));
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and PickerObserver doesn't implement Drop.
    #[unsafe(super(NSObject))]
    struct PickerObserver;

    // SAFETY: NSObjectProtocol has no requirements beyond being an NSObject.
    unsafe impl NSObjectProtocol for PickerObserver {}

    // SAFETY: the method signatures match the protocol. A panic must not unwind into macOS.
    unsafe impl SCContentSharingPickerObserver for PickerObserver {
        #[unsafe(method(contentSharingPicker:didCancelForStream:))]
        fn did_cancel(&self, _picker: &SCContentSharingPicker, stream: Option<&SCStream>) {
            let _ = catch_unwind(AssertUnwindSafe(|| picker_cancelled(stream)));
        }

        #[unsafe(method(contentSharingPicker:didUpdateWithFilter:forStream:))]
        fn did_update(
            &self,
            _picker: &SCContentSharingPicker,
            filter: &SCContentFilter,
            stream: Option<&SCStream>,
        ) {
            let _ = catch_unwind(AssertUnwindSafe(|| picker_updated(filter, stream)));
        }

        #[unsafe(method(contentSharingPickerStartDidFailWithError:))]
        fn did_fail(&self, error: &NSError) {
            let _ = catch_unwind(AssertUnwindSafe(|| picker_failed(error)));
        }
    }
);

impl PickerObserver {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: plain NSObject init.
        unsafe { msg_send![super(this), init] }
    }
}

/// Our own windows (the chat, a popped-out screen), so Apple's picker leaves them out. Only
/// window numbers and owners: nothing about other apps' windows, and nothing captured.
fn own_window_ids() -> Vec<CGWindowID> {
    let Some(list) = CGWindowListCopyWindowInfo(CGWindowListOption::OptionAll, kCGNullWindowID)
    else {
        return Vec::new();
    };
    // SAFETY: CoreGraphics documents this as an array of dictionaries.
    let list: CFRetained<CFArray<CFDictionary<CFString, CFType>>> =
        unsafe { CFRetained::cast_unchecked(list) };
    // SAFETY: extern CFString constants from CoreGraphics.
    let (owner_key, number_key) = unsafe { (kCGWindowOwnerPID, kCGWindowNumber) };
    let number = |window: &CFDictionary<CFString, CFType>, key: &CFString| {
        window
            .get(key)
            .and_then(|v| v.downcast::<CFNumber>().ok())
            .and_then(|n| n.as_i64())
    };
    let pid = i64::from(own_pid());
    list.iter()
        .filter(|w| number(w, owner_key) == Some(pid))
        .filter_map(|w| number(&w, number_key))
        .filter_map(|n| CGWindowID::try_from(n).ok())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Sources and filters
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Display(CGDirectDisplayID),
    Window(CGWindowID),
}

impl Target {
    fn id(self) -> String {
        match self {
            Target::Display(id) => format!("display:{id}"),
            Target::Window(id) => format!("window:{id}"),
        }
    }

    fn parse(id: &str) -> Option<Target> {
        if let Some(n) = id.strip_prefix("display:") {
            n.parse().ok().map(Target::Display)
        } else if let Some(n) = id.strip_prefix("window:") {
            n.parse().ok().map(Target::Window)
        } else {
            None
        }
    }

    fn still_exists(self) -> bool {
        match self {
            Target::Display(id) => CGDisplayIsOnline(id),
            // Lists just this window if it still exists (minimised or on another Space counts).
            Target::Window(id) => {
                CGWindowListCopyWindowInfo(CGWindowListOption::OptionIncludingWindow, id)
                    .is_some_and(|windows| windows.count() > 0)
            }
        }
    }

    fn gone_message(self) -> &'static str {
        match self {
            Target::Display(_) => "The shared screen was disconnected.",
            Target::Window(_) => "The shared window was closed.",
        }
    }
}

fn screen_name(number: usize, width: u32, height: u32) -> String {
    format!("Screen {number} ({width} × {height})")
}

fn window_name(title: &str, app: &str) -> String {
    let (title, app) = (title.trim(), app.trim());
    if app.is_empty() || app == title {
        title.to_string()
    } else {
        format!("{title} — {app}")
    }
}

fn own_pid() -> i32 {
    std::process::id() as i32
}

/// What we need to know about a display. SCDisplay sizes are in points.
struct DisplayInfo {
    id: CGDirectDisplayID,
    /// Where it sits on the desktop, in points.
    frame: CGRect,
    /// Pixels per point (2 on Retina).
    scale: f64,
}

impl DisplayInfo {
    fn of(display: &SCDisplay) -> DisplayInfo {
        // SAFETY: plain property getters on a valid SCDisplay.
        let (id, frame) = unsafe { (display.displayID(), display.frame()) };
        DisplayInfo {
            id,
            frame,
            scale: display_scale(id),
        }
    }

    fn pixel_size(&self) -> (u32, u32) {
        to_pixels(&self.frame, self.scale)
    }
}

/// What we need to know about a window.
#[derive(Clone)]
struct WindowInfo {
    id: CGWindowID,
    title: String,
    app_name: String,
    /// None when no app owns it (system windows).
    pid: Option<i32>,
    /// In points.
    frame: CGRect,
    on_screen: bool,
    layer: isize,
}

impl WindowInfo {
    fn of(window: &SCWindow) -> WindowInfo {
        // SAFETY: plain property getters on a valid SCWindow and its owning app.
        unsafe {
            let app = window.owningApplication();
            WindowInfo {
                id: window.windowID(),
                title: window.title().map(|t| t.to_string()).unwrap_or_default(),
                app_name: app
                    .as_ref()
                    .map(|a| a.applicationName().to_string())
                    .unwrap_or_default(),
                pid: app.as_ref().map(|a| a.processID()),
                frame: window.frame(),
                on_screen: window.isOnScreen(),
                layer: window.windowLayer(),
            }
        }
    }

    /// Worth showing in the picker: someone else's visible, titled, normal-sized app window.
    /// Layer 0 is ordinary app windows; the menu bar, Dock and overlays sit above it.
    fn is_listed(&self, own_pid: i32) -> bool {
        self.on_screen
            && self.layer == 0
            && self.pid.is_some_and(|pid| pid != own_pid)
            && !self.title.trim().is_empty()
            && self.frame.size.width >= MIN_WINDOW_POINTS
            && self.frame.size.height >= MIN_WINDOW_POINTS
    }

    fn pixel_size(&self, displays: &[DisplayInfo]) -> (u32, u32) {
        to_pixels(&self.frame, scale_at(displays, &self.frame))
    }
}

fn to_pixels(frame: &CGRect, scale: f64) -> (u32, u32) {
    (
        (frame.size.width * scale).round() as u32,
        (frame.size.height * scale).round() as u32,
    )
}

/// Pixels per point of a display, from its current mode.
fn display_scale(id: CGDirectDisplayID) -> f64 {
    let mode = CGDisplayCopyDisplayMode(id);
    let points = CGDisplayMode::width(mode.as_deref());
    let pixels = CGDisplayMode::pixel_width(mode.as_deref());
    if points > 0 && pixels > 0 {
        pixels as f64 / points as f64
    } else {
        1.0
    }
}

/// The scale of the display under the middle of `frame`, else the largest one (sharper is safer).
fn scale_at(displays: &[DisplayInfo], frame: &CGRect) -> f64 {
    let x = frame.origin.x + frame.size.width / 2.0;
    let y = frame.origin.y + frame.size.height / 2.0;
    displays
        .iter()
        .find(|d| {
            let r = &d.frame;
            x >= r.origin.x
                && x < r.origin.x + r.size.width
                && y >= r.origin.y
                && y < r.origin.y + r.size.height
        })
        .map(|d| d.scale)
        .unwrap_or_else(|| displays.iter().map(|d| d.scale).fold(1.0, f64::max))
}

fn displays_of(content: &SCShareableContent) -> Vec<Retained<SCDisplay>> {
    // SAFETY: plain property getter on a valid SCShareableContent.
    unsafe { content.displays() }.to_vec()
}

fn windows_of(content: &SCShareableContent) -> Vec<Retained<SCWindow>> {
    // SAFETY: plain property getter on a valid SCShareableContent.
    unsafe { content.windows() }.to_vec()
}

/// Builds the content filter for `target` and returns the source's size in pixels.
fn make_filter(
    content: &SCShareableContent,
    target: Target,
) -> Result<(Retained<SCContentFilter>, (u32, u32)), String> {
    let displays = displays_of(content);
    let infos: Vec<DisplayInfo> = displays.iter().map(|d| DisplayInfo::of(d)).collect();
    match target {
        Target::Display(id) => {
            let index = infos
                .iter()
                .position(|d| d.id == id)
                .ok_or("That screen isn't connected anymore.")?;
            // Leave Yap's own windows out, so the people in the call don't see a hall of
            // mirrors (or the chat).
            let own_pid = own_pid();
            // SAFETY: plain property getter on a valid SCShareableContent.
            let apps = unsafe { content.applications() }.to_vec();
            // SAFETY: plain property getter on a valid SCRunningApplication.
            let ours: Vec<_> = apps
                .into_iter()
                .filter(|a| unsafe { a.processID() } == own_pid)
                .collect();
            let ours = NSArray::from_retained_slice(&ours);
            let no_windows = NSArray::<SCWindow>::new();
            // SAFETY: a valid display and arrays.
            let filter = unsafe {
                SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                    SCContentFilter::alloc(),
                    &displays[index],
                    &ours,
                    &no_windows,
                )
            };
            Ok((filter, infos[index].pixel_size()))
        }
        Target::Window(id) => {
            let window = windows_of(content)
                .into_iter()
                .find(|w| WindowInfo::of(w).id == id)
                .ok_or("That window was closed.")?;
            let size = WindowInfo::of(&window).pixel_size(&infos);
            // SAFETY: a valid window.
            let filter = unsafe {
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
            };
            Ok((filter, size))
        }
    }
}

fn make_config(
    source_size: (u32, u32),
    options: &Options,
    sound: bool,
) -> Retained<SCStreamConfiguration> {
    let (width, height) = fit_inside(source_size, (options.max_width, options.max_height));
    // SAFETY: a fresh configuration object; every setter here exists since macOS 12.3, and the
    // sound ones since 13.0 (only used when sound_supported()).
    unsafe {
        let config = SCStreamConfiguration::new();
        config.setWidth(width as usize);
        config.setHeight(height as usize);
        config.setScalesToFit(true);
        config.setPixelFormat(kCVPixelFormatType_32BGRA);
        // Tag frames as sRGB so colors look the same on the other end (Macs default to P3).
        config.setColorSpaceName(kCGColorSpaceSRGB);
        config.setMinimumFrameInterval(CMTime::new(1, options.fps.clamp(1, 240) as i32));
        config.setShowsCursor(true);
        config.setQueueDepth(QUEUE_DEPTH);
        if sound {
            config.setCapturesAudio(true);
            config.setExcludesCurrentProcessAudio(true);
            config.setSampleRate(SAMPLE_RATE as isize);
            config.setChannelCount(2);
        }
        config
    }
}

/// The largest size that fits inside `max` with the source's aspect ratio, never bigger than
/// the source. Rounded down to even numbers, which H.264 needs. A max of 0 means "no limit".
fn fit_inside(source: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (width, height) = (source.0.max(1), source.1.max(1));
    let max_width = if max.0 == 0 { width } else { max.0.min(width) };
    let max_height = if max.1 == 0 {
        height
    } else {
        max.1.min(height)
    };
    let scale = (max_width as f64 / width as f64).min(max_height as f64 / height as f64);
    let fit = |size: u32, limit: u32| {
        let scaled = ((size as f64 * scale).round() as u32).min(limit);
        (scaled & !1).max(2)
    };
    (fit(width, max_width), fit(height, max_height))
}

// ---------------------------------------------------------------------------------------------
// ScreenCaptureKit plumbing
// ---------------------------------------------------------------------------------------------

fn os_at_least(major: isize, minor: isize) -> bool {
    let version = NSOperatingSystemVersion {
        majorVersion: major,
        minorVersion: minor,
        patchVersion: 0,
    };
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(version)
}

/// True when ScreenCaptureKit can be used. Checked before touching any of its classes or
/// constants. The class lookup also covers a weak-linked framework that isn't there.
fn screen_capture_kit_available() -> bool {
    os_at_least(12, 3) && AnyClass::get(c"SCStream").is_some()
}

/// SCShareableContent is an immutable snapshot, so passing it between threads is fine.
struct SendContent(Retained<SCShareableContent>);
// SAFETY: see above; we only read from it.
unsafe impl Send for SendContent {}

/// Asks macOS what can be shared. The first call is also what makes macOS ask for (and list
/// Yap under) Screen Recording permission.
fn shareable_content(on_screen_only: bool) -> Result<Retained<SCShareableContent>, String> {
    let (tx, rx) = mpsc::channel::<Result<SendContent, String>>();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: ScreenCaptureKit passes a valid object (or null) for each.
            let result = match unsafe { (Retained::retain(content), error.as_ref()) } {
                (Some(content), _) => Ok(SendContent(content)),
                (None, error) => Err(list_error(error)),
            };
            // If sources() already gave up waiting, nobody is listening; that's fine.
            let _ = tx.send(result);
        },
    );
    // SAFETY: a valid completion handler; ScreenCaptureKit copies it.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true,
            on_screen_only,
            &handler,
        );
    }
    match rx.recv_timeout(LIST_TIMEOUT) {
        Ok(result) => result.map(|content| content.0),
        Err(_) => {
            Err("macOS didn't list the screens and windows in time. Please try again.".into())
        }
    }
}

fn list_error(error: Option<&NSError>) -> String {
    let declined = error.is_some_and(|e| is_sck_error(e, SCStreamErrorCode::UserDeclined));
    if declined || !CGPreflightScreenCaptureAccess() {
        return PERMISSION_MESSAGE.into();
    }
    match error {
        Some(error) => format!(
            "Couldn't list screens and windows: {}",
            error.localizedDescription()
        ),
        None => "Couldn't list screens and windows.".into(),
    }
}

fn is_sck_error(error: &NSError, code: SCStreamErrorCode) -> bool {
    // SAFETY: an extern NSString constant; we only get here once ScreenCaptureKit is available.
    let domain = unsafe { SCStreamErrorDomain };
    error.code() == code.0 && *error.domain() == *domain
}

/// Plain words for why a stream failed or stopped.
fn describe_error(error: &NSError) -> String {
    if is_sck_error(error, SCStreamErrorCode::UserDeclined) {
        PERMISSION_MESSAGE.into()
    } else if is_sck_error(error, SCStreamErrorCode::UserStopped) {
        "Screen sharing was stopped from the macOS menu bar.".into()
    } else if is_sck_error(error, SCStreamErrorCode::SystemStoppedStream) {
        "macOS stopped the screen sharing.".into()
    } else if is_sck_error(error, SCStreamErrorCode::NoCaptureSource) {
        "The shared screen or window is gone.".into()
    } else {
        format!("Screen sharing stopped: {}", error.localizedDescription())
    }
}

/// Only "Complete" frames carry a new picture; Idle/Blank/Suspended/... are skipped.
fn frame_is_complete(sample: &CMSampleBuffer) -> bool {
    // SAFETY: a valid sample buffer for the duration of the callback.
    let Some(attachments) = (unsafe { sample.sample_attachments_array(false) }) else {
        return false;
    };
    // SAFETY: CoreMedia documents this as an array of CFDictionary, one per sample.
    let attachments: CFRetained<CFArray<CFDictionary<CFString, CFType>>> =
        unsafe { CFRetained::cast_unchecked(attachments) };
    let Some(info) = attachments.get(0) else {
        return false;
    };
    // SAFETY: an extern NSString constant; we only get here once ScreenCaptureKit is available.
    let key: &CFString = unsafe { SCStreamFrameInfoStatus }.as_ref();
    let Some(status) = info.get(key) else {
        return false;
    };
    let Ok(status) = status.downcast::<CFNumber>() else {
        return false;
    };
    status.as_isize() == Some(SCFrameStatus::Complete.0)
}

/// Copies the BGRA picture out of the sample buffer.
fn copy_frame(sample: &CMSampleBuffer) -> Option<Frame> {
    // SAFETY: a valid sample buffer for the duration of the callback.
    let image = unsafe { sample.image_buffer() }?;
    if CVPixelBufferGetPixelFormatType(&image) != kCVPixelFormatType_32BGRA
        || CVPixelBufferIsPlanar(&image)
    {
        return None;
    }
    // SAFETY: a read-only lock, balanced by the unlock below.
    if unsafe { CVPixelBufferLockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly) }
        != kCVReturnSuccess
    {
        return None;
    }
    let width = CVPixelBufferGetWidth(&image);
    let height = CVPixelBufferGetHeight(&image);
    let stride = CVPixelBufferGetBytesPerRow(&image);
    let base = CVPixelBufferGetBaseAddress(&image).cast::<u8>();
    let frame = (!base.is_null() && width > 0 && height > 0 && stride >= width * 4).then(|| {
        // Every row but the last is `stride` long; the last only needs its pixels.
        let len = stride * (height - 1) + width * 4;
        // SAFETY: while locked, the buffer holds `height` rows of `stride` bytes from `base`.
        let mut data = unsafe { std::slice::from_raw_parts(base, len) }.to_vec();
        data.resize(stride * height, 0);
        Frame {
            width: width as u32,
            height: height as u32,
            stride,
            data,
        }
    });
    // SAFETY: matches the lock above.
    unsafe { CVPixelBufferUnlockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly) };
    frame
}

/// Reads a sound sample buffer into `out` as interleaved stereo f32. False if it isn't the
/// 48 kHz float audio we asked for.
fn read_audio(sample: &CMSampleBuffer, list: &mut Vec<u64>, out: &mut Vec<f32>) -> bool {
    // SAFETY: a valid sample buffer for the duration of the callback.
    let Some(format) = (unsafe { sample.format_description() }) else {
        return false;
    };
    // SAFETY: returns null for non-audio formats; otherwise valid while `format` lives.
    let Some(asbd) =
        (unsafe { CMAudioFormatDescriptionGetStreamBasicDescription(&format).as_ref() })
    else {
        return false;
    };
    let is_float32 = asbd.mFormatID == kAudioFormatLinearPCM
        && asbd.mFormatFlags & kAudioFormatFlagIsFloat != 0
        && asbd.mBitsPerChannel == 32;
    // Another rate would play at the wrong speed; ScreenCaptureKit honours the 48 kHz we ask for.
    if !is_float32 || asbd.mSampleRate.round() as u32 != SAMPLE_RATE {
        return false;
    }

    // First ask how big the AudioBufferList is (one AudioBuffer per channel when planar).
    let mut needed = 0usize;
    // SAFETY: size query only; null outputs are allowed.
    let status = unsafe {
        sample.audio_buffer_list_with_retained_block_buffer(
            &mut needed,
            ptr::null_mut(),
            0,
            None,
            None,
            0,
            ptr::null_mut(),
        )
    };
    let header = offset_of!(AudioBufferList, mBuffers);
    if status != 0 || needed < header + size_of::<AudioBuffer>() {
        return false;
    }
    list.clear();
    list.resize(needed.div_ceil(size_of::<u64>()), 0);
    let list_ptr = list.as_mut_ptr().cast::<AudioBufferList>();
    let mut block: *mut CMBlockBuffer = ptr::null_mut();
    // SAFETY: `list` has room for `needed` bytes and is 8-byte aligned.
    let status = unsafe {
        sample.audio_buffer_list_with_retained_block_buffer(
            ptr::null_mut(),
            list_ptr,
            needed,
            None,
            None,
            kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            &mut block,
        )
    };
    // We own the returned block buffer; it keeps the sample memory alive until dropped.
    // SAFETY: a +1 reference handed to us by CoreMedia.
    let _block = NonNull::new(block).map(|block| unsafe { CFRetained::from_raw(block) });
    if status != 0 {
        return false;
    }

    // SAFETY: CoreMedia filled the header; the count is capped to what fits in `needed`.
    let count = (unsafe { (*list_ptr).mNumberBuffers } as usize)
        .min((needed - header) / size_of::<AudioBuffer>());
    // SAFETY: `count` AudioBuffers follow the header inside `list`.
    let buffers = unsafe {
        std::slice::from_raw_parts(
            ptr::addr_of!((*list_ptr).mBuffers).cast::<AudioBuffer>(),
            count,
        )
    };
    let mut planes: Vec<(&[f32], usize)> = Vec::with_capacity(buffers.len());
    for buffer in buffers {
        let data = buffer.mData.cast::<f32>().cast_const();
        if data.is_null() || !data.is_aligned() {
            continue;
        }
        let len = buffer.mDataByteSize as usize / size_of::<f32>();
        // SAFETY: mData holds mDataByteSize bytes of f32 samples, alive while `_block` is.
        planes.push((
            unsafe { std::slice::from_raw_parts(data, len) },
            buffer.mNumberChannels as usize,
        ));
    }
    interleave_stereo(&planes, out);
    true
}

/// Turns audio buffers into interleaved stereo (L R L R …). Each buffer comes with its channel
/// count: planar audio is one channel per buffer, interleaved audio is one buffer with all
/// channels. Mono is copied to both sides; channels past the second are dropped.
fn interleave_stereo(buffers: &[(&[f32], usize)], out: &mut Vec<f32>) {
    out.clear();
    // The first two channels as (samples, offset, step).
    let mut channels: Vec<(&[f32], usize, usize)> = Vec::with_capacity(2);
    for &(samples, count) in buffers {
        let step = count.max(1);
        for offset in 0..step.min(2) {
            if channels.len() < 2 {
                channels.push((samples, offset, step));
            }
        }
    }
    let Some(&left) = channels.first() else {
        return;
    };
    let right = channels.get(1).copied().unwrap_or(left);
    let frames = channels
        .iter()
        .map(|(samples, _, step)| samples.len() / step)
        .min()
        .unwrap_or(0);
    out.reserve(frames * 2);
    for frame in 0..frames {
        out.push(left.0[frame * left.2 + left.1]);
        out.push(right.0[frame * right.2 + right.1]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit_inside((2560, 1440), (1920, 1080)), (1920, 1080));
        assert_eq!(fit_inside((2560, 1600), (1920, 1080)), (1728, 1080));
        assert_eq!(fit_inside((1280, 720), (1920, 1080)), (1280, 720));
        assert_eq!(fit_inside((1080, 1920), (1920, 1080)), (608, 1080)); // portrait: 607.5 rounds up
        assert_eq!(fit_inside((3024, 1964), (1280, 720)), (1108, 720));
    }

    #[test]
    fn fit_rounds_to_even_and_handles_odd_inputs() {
        assert_eq!(fit_inside((1001, 501), (4000, 4000)), (1000, 500));
        assert_eq!(fit_inside((1, 1), (1920, 1080)), (2, 2));
        assert_eq!(fit_inside((0, 0), (1920, 1080)), (2, 2));
        assert_eq!(fit_inside((2560, 1440), (0, 0)), (2560, 1440));
        assert_eq!(fit_inside((2560, 1440), (0, 720)), (1280, 720));
        let (w, h) = fit_inside((3000, 1000), (1000, 1000));
        assert!(w <= 1000 && h <= 1000 && w % 2 == 0 && h % 2 == 0);
    }

    #[test]
    fn interleaves_planar_stereo() {
        let left = [1.0, 2.0, 3.0];
        let right = [-1.0, -2.0, -3.0];
        let mut out = Vec::new();
        interleave_stereo(&[(&left, 1), (&right, 1)], &mut out);
        assert_eq!(out, [1.0, -1.0, 2.0, -2.0, 3.0, -3.0]);
    }

    #[test]
    fn duplicates_mono_and_keeps_interleaved() {
        let mut out = Vec::new();
        interleave_stereo(&[(&[0.5, 0.25], 1)], &mut out);
        assert_eq!(out, [0.5, 0.5, 0.25, 0.25]);

        interleave_stereo(&[(&[1.0, -1.0, 2.0, -2.0], 2)], &mut out);
        assert_eq!(out, [1.0, -1.0, 2.0, -2.0]);

        // Interleaved 3 channels: keep the first two of each frame.
        interleave_stereo(&[(&[1.0, -1.0, 9.0, 2.0, -2.0, 9.0], 3)], &mut out);
        assert_eq!(out, [1.0, -1.0, 2.0, -2.0]);

        // Mismatched planes: stop at the shorter one. Extra planes are ignored.
        interleave_stereo(
            &[(&[1.0, 2.0, 3.0], 1), (&[-1.0, -2.0], 1), (&[9.0], 1)],
            &mut out,
        );
        assert_eq!(out, [1.0, -1.0, 2.0, -2.0]);

        interleave_stereo(&[], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn source_ids_round_trip() {
        for target in [Target::Display(69733378), Target::Window(4242)] {
            assert_eq!(Target::parse(&target.id()), Some(target));
        }
        assert_eq!(Target::parse("window:abc"), None);
        assert_eq!(Target::parse("hwnd:12"), None);
    }

    #[test]
    fn names() {
        assert_eq!(screen_name(1, 2560, 1440), "Screen 1 (2560 × 1440)");
        assert_eq!(window_name("Inbox", "Mail"), "Inbox — Mail");
        assert_eq!(window_name(" Notes ", ""), "Notes");
        assert_eq!(window_name("Finder", "Finder"), "Finder");
    }

    #[test]
    fn scale_picks_the_display_under_the_window() {
        let displays = [
            DisplayInfo {
                id: 1,
                frame: rect(0.0, 1512.0, 982.0),
                scale: 2.0,
            },
            DisplayInfo {
                id: 2,
                frame: rect(1512.0, 2560.0, 1440.0),
                scale: 1.0,
            },
        ];
        assert_eq!(scale_at(&displays, &rect(100.0, 500.0, 300.0)), 2.0);
        assert_eq!(scale_at(&displays, &rect(2000.0, 500.0, 300.0)), 1.0);
        assert_eq!(scale_at(&displays, &rect(-5000.0, 10.0, 10.0)), 2.0); // nowhere: the sharpest
        assert_eq!(displays[0].pixel_size(), (3024, 1964));
    }

    // These touch the Objective-C runtime but capture nothing, ask for no permission and never
    // open Apple's picker (an empty content filter stands in for a picked one).
    #[test]
    fn picker_observer_registers_and_picks_arrive_once() {
        // Registering the class checks (in debug builds) our method signatures against
        // SCContentSharingPickerObserver.
        let observer = PickerObserver::new();
        let _ = ProtocolObject::<dyn SCContentSharingPickerObserver>::from_ref(&*observer);

        // SAFETY: a plain empty filter, never used to capture.
        let filter = unsafe { SCContentFilter::new() };
        // A pick nobody asked for (the Control Center menu) is ignored.
        picker_updated(&filter, None);
        assert!(picker_state().picked.is_empty());

        let (tx, rx) = mpsc::channel();
        picker_state().pending = Some(Box::new(move |r| tx.send(r).unwrap()));
        picker_updated(&filter, None);
        let source = rx.try_recv().unwrap().unwrap().expect("a pick");
        assert!(source.id.starts_with(PICKED));
        assert!(source.name.contains("picked in macOS"));
        assert_eq!(picker_state().picked.len(), 1);
        assert!(picker_state().pending.is_none());
        // Its stream isn't running: watching it fails in words, and the capture handle of a
        // stream that's gone does nothing.
        let options = Options { max_width: 1920, max_height: 1080, fps: 60, sound: false };
        assert!(start_held(&source, options, Box::new(|_| {}), None).is_err());
        let mut gone = HeldCapture { id: source.id.clone(), done: false };
        gone.set_options(options);
        gone.stop();
        // Not shared after all: forgotten.
        release(&source);
        assert!(picker_state().picked.is_empty());

        // Cancelled: the asker hears None.
        let (tx, rx) = mpsc::channel();
        picker_state().pending = Some(Box::new(move |r| tx.send(r).unwrap()));
        picker_cancelled(None);
        assert!(matches!(rx.try_recv().unwrap(), Ok(None)));
        // Only the last asker hears the next pick; an earlier one hears None.
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        picker_state().pending = Some(Box::new(move |r| tx1.send(r.map(|s| s.is_some())).unwrap()));
        let earlier = picker_state().pending.replace(Box::new(move |r| tx2.send(r.map(|s| s.is_some())).unwrap()));
        earlier.unwrap()(Ok(None));
        picker_updated(&filter, None);
        assert_eq!(rx1.try_recv().unwrap(), Ok(false));
        assert_eq!(rx2.try_recv().unwrap(), Ok(true));
        let leftover: Vec<String> = picker_state().picked.iter().map(|(id, _)| id.clone()).collect();
        for id in leftover {
            release(&Source { id, name: String::new(), kind: SourceKind::Window });
        }
        assert!(picker_state().picked.is_empty());
    }

    #[test]
    fn a_held_stream_hands_pictures_over_only_while_attached() {
        let shared = Shared::new(None, None, true);
        assert!(shared.last.is_some());
        let (tx, rx) = mpsc::channel();
        shared.attach(Box::new(move |f: Frame| tx.send(f.width).unwrap()), None);
        // Nothing kept yet, so nothing handed over; from now on pictures go to the viewer.
        assert!(rx.try_recv().is_err());
        assert!(shared.on_frame.lock().unwrap().is_some());
        shared.detach();
        assert!(shared.on_frame.lock().unwrap().is_none());
        assert!(shared.audio.lock().unwrap().on_audio.is_none());
        shared.forget_last();
        assert!(shared.last.as_ref().unwrap().lock().unwrap().is_none());
        // A stream from our own list keeps nothing.
        assert!(Shared::new(None, None, false).last.is_none());
    }

    #[test]
    fn output_class_registers_and_ends_once() {
        // Registering the class checks (in debug builds) our method signatures against the
        // SCStreamOutput/SCStreamDelegate protocols.
        let shared = Shared::new(Some(Box::new(|_| {})), None, false);
        let output = CaptureOutput::new(shared.clone());
        let _ = ProtocolObject::<dyn SCStreamOutput>::from_ref(&*output);
        let _ = ProtocolObject::<dyn SCStreamDelegate>::from_ref(&*output);

        // Ending before start() finishes is reported by arm(), not through on_ended.
        shared.end_by_itself("early".into());
        assert_eq!(
            shared.arm(Box::new(|_| panic!("not called"))),
            Err("early".into())
        );

        // After arming, on_ended runs exactly once.
        let shared = Shared::new(Some(Box::new(|_| {})), None, false);
        let (tx, rx) = mpsc::channel();
        assert!(shared
            .arm(Box::new(move |message| tx.send(message).unwrap()))
            .is_ok());
        shared.end_by_itself("first".into());
        shared.end_by_itself("second".into());
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), ["first"]);
    }

    #[test]
    fn errors_in_plain_words() {
        // SAFETY: an extern NSString constant from ScreenCaptureKit (present on this Mac).
        let domain = unsafe { SCStreamErrorDomain };
        let declined = NSError::new(SCStreamErrorCode::UserDeclined.0, domain);
        assert_eq!(describe_error(&declined), PERMISSION_MESSAGE);
        let stopped = NSError::new(SCStreamErrorCode::UserStopped.0, domain);
        assert!(describe_error(&stopped).contains("menu bar"));
        let other = NSError::new(
            SCStreamErrorCode::UserDeclined.0,
            objc2_foundation::ns_string!("Other"),
        );
        assert!(describe_error(&other).starts_with("Screen sharing stopped: "));
    }

    #[test]
    fn window_filtering() {
        let window = WindowInfo {
            id: 7,
            title: "Inbox".into(),
            app_name: "Mail".into(),
            pid: Some(100),
            frame: rect(10.0, 800.0, 600.0),
            on_screen: true,
            layer: 0,
        };
        assert!(window.is_listed(1));
        assert!(!window.is_listed(100)); // ours
        let not_listed = |changed: WindowInfo| !changed.is_listed(1);
        assert!(not_listed(WindowInfo {
            title: "  ".into(),
            ..window.clone()
        }));
        assert!(not_listed(WindowInfo {
            layer: 25,
            ..window.clone()
        }));
        assert!(not_listed(WindowInfo {
            pid: None,
            ..window.clone()
        }));
        assert!(not_listed(WindowInfo {
            on_screen: false,
            ..window.clone()
        }));
        assert!(not_listed(WindowInfo {
            frame: rect(0.0, 30.0, 600.0),
            ..window.clone()
        }));
        let displays = [DisplayInfo {
            id: 1,
            frame: rect(0.0, 1512.0, 982.0),
            scale: 2.0,
        }];
        assert_eq!(window.pixel_size(&displays), (1600, 1200));
    }

    fn rect(x: f64, width: f64, height: f64) -> CGRect {
        CGRect {
            origin: objc2_core_foundation::CGPoint { x, y: 0.0 },
            size: objc2_core_foundation::CGSize { width, height },
        }
    }
}
