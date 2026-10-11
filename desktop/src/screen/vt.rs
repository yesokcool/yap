//! H.264 through VideoToolbox, the Mac's hardware encoder: captured BGRA
//! frames → scaled → NV12 → constrained-baseline H.264 (Annex B). The same
//! kind of stream `codec.rs` makes with OpenH264, encoded off the CPU.

use super::capture::Frame;
pub use super::codec::Encoded;
use super::codec::fit;

use std::ffi::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::{
    CMSampleBuffer, CMTime, CMTimeFlags, CMVideoFormatDescriptionGetH264ParameterSetAtIndex, kCMSampleAttachmentKey_NotSync, kCMVideoCodecType_H264,
};
use objc2_core_video::{
    CVAttachmentMode, CVPixelBuffer, CVPixelBufferGetBaseAddressOfPlane, CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetHeightOfPlane,
    CVPixelBufferGetWidthOfPlane, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferPool, CVPixelBufferUnlockBaseAddress,
    kCVImageBufferColorPrimaries_ITU_R_709_2, kCVImageBufferColorPrimariesKey, kCVImageBufferTransferFunction_ITU_R_709_2, kCVImageBufferTransferFunctionKey,
    kCVImageBufferYCbCrMatrix_ITU_R_601_4, kCVImageBufferYCbCrMatrixKey, kCVPixelBufferHeightKey, kCVPixelBufferIOSurfacePropertiesKey,
    kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVReturnSuccess,
};
use objc2_video_toolbox::{
    VTCompressionSession, VTEncodeInfoFlags, VTSessionCopyProperty, VTSessionSetProperty, kVTCompressionPropertyKey_AllowFrameReordering,
    kVTCompressionPropertyKey_AverageBitRate, kVTCompressionPropertyKey_ColorPrimaries, kVTCompressionPropertyKey_ExpectedFrameRate,
    kVTCompressionPropertyKey_MaxH264SliceBytes, kVTCompressionPropertyKey_MaxKeyFrameInterval, kVTCompressionPropertyKey_ProfileLevel,
    kVTCompressionPropertyKey_RealTime, kVTCompressionPropertyKey_TransferFunction, kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
    kVTCompressionPropertyKey_YCbCrMatrix, kVTEncodeFrameOptionKey_ForceKeyFrame, kVTProfileLevel_H264_Baseline_AutoLevel,
    kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder,
};

/// Presentation times tick at 90 kHz (RTP's video clock).
const CLOCK: i32 = 90_000;
/// Slices up to ~1100 bytes where the encoder can (each fits one RTP packet, no FU-A).
/// Apple Silicon's can't, hardware or software; the RTP packetizer fragments then.
const MAX_SLICE: i32 = 1100;
/// The size of the session `new` makes to find out whether VideoToolbox works here.
const PROBE: (u32, u32) = (1280, 720);
const START: [u8; 4] = [0, 0, 0, 1];

/// H.264 through VideoToolbox (the Mac's hardware encoder).
pub struct HwEncoder {
    /// Made for one picture size; remade when the size changes (or it breaks).
    session: Option<Session>,
    bitrate: u32,
    fps: u32,
    /// Is the latest session on the hardware encoder?
    hardware: bool,
    /// The next frame's presentation time, in `CLOCK` ticks.
    pts: i64,
}

impl HwEncoder {
    /// None when VideoToolbox can't make a hardware (or any) H.264 session here.
    pub fn new(bitrate: u32, fps: u32) -> Option<Self> {
        let fps = fps.max(1);
        match Session::new(PROBE, bitrate, fps) {
            Ok(session) => {
                log::info!("H.264: VideoToolbox, {} encoder", if session.hardware { "hardware" } else { "software" });
                Some(Self { hardware: session.hardware, session: Some(session), bitrate, fps, pts: 0 })
            }
            Err(e) => {
                log::warn!("H.264: no VideoToolbox encoder: {e}");
                None
            }
        }
    }

    /// Is it encoding on the hardware encoder (not VideoToolbox's software one)?
    pub fn hardware(&self) -> bool {
        self.hardware
    }

    /// Takes effect on the next encode (on the running session when it can be).
    pub fn set_rate(&mut self, bitrate: u32, fps: u32) {
        self.bitrate = bitrate;
        self.fps = fps.max(1);
    }

    /// Change the running session's bitrate (no new keyframe): how a share
    /// backs off when a viewer's connection loses packets.
    pub fn set_bitrate_live(&mut self, bps: u32) {
        self.bitrate = bps;
        if let Some(s) = self.session.as_mut()
            && let Err(e) = s.set_rate(bps, s.rate.1)
        {
            // The next encode tries again (and starts over if it still fails).
            log::warn!("H.264: {e}");
        }
    }

    /// Scale `frame` to fit `max`, encode it (a keyframe if `keyframe`).
    /// Synchronous: returns this frame's NAL units (Annex B, SPS and PPS in
    /// front of a keyframe), or none if VideoToolbox dropped the frame.
    pub fn encode(&mut self, frame: &Frame, max: (u32, u32), keyframe: bool) -> Result<Encoded, String> {
        let (fw, fh) = (frame.width as usize, frame.height as usize);
        if fw == 0 || fh == 0 || frame.stride < fw * 4 || frame.data.len() < frame.stride * (fh - 1) + fw * 4 {
            return Err("H.264 encode: an empty or short frame".into());
        }
        let (w, h) = fit(frame.width, frame.height, max.0, max.1);
        let rate = (self.bitrate, self.fps);
        if let Some(s) = self.session.as_mut().filter(|s| s.size == (w, h) && s.rate != rate)
            && let Err(e) = s.set_rate(rate.0, rate.1)
        {
            log::warn!("H.264: {e}; starting a new session");
            self.session = None;
        }
        if self.session.as_ref().is_none_or(|s| s.size != (w, h)) {
            // Let go of the old one first: the hardware has only so many sessions.
            self.session = None;
            let s = Session::new((w, h), rate.0, rate.1)?;
            self.hardware = s.hardware;
            self.session = Some(s);
        }
        let s = self.session.as_mut().expect("session");
        let step = i64::from(CLOCK as u32 / rate.1).max(1);
        let pts = self.pts;
        self.pts += step;
        match s.encode(frame, pts, step, keyframe) {
            Ok((data, keyframe)) => Ok(Encoded { data, keyframe, width: w, height: h }),
            Err(e) => {
                // A broken session (the GPU reset, the Mac slept) is remade on the next frame.
                self.session = None;
                Err(e)
            }
        }
    }
}

/// What the output callback hands back for one frame.
enum Output {
    Coded { data: Vec<u8>, keyframe: bool },
    Dropped,
    Failed(String),
}

type Sink = Mutex<Vec<Output>>;

fn lock(sink: &Sink) -> MutexGuard<'_, Vec<Output>> {
    sink.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One VideoToolbox session, for one picture size.
struct Session {
    vt: CFRetained<VTCompressionSession>,
    /// NV12 pictures of `size`, for the session's input.
    pool: CFRetained<CVPixelBufferPool>,
    /// Where the output callback leaves each frame (VideoToolbox holds a pointer to it until `drop`).
    sink: Arc<Sink>,
    size: (u32, u32),
    /// The bitrate and fps it's set to.
    rate: (u32, u32),
    hardware: bool,
}

// SAFETY: VideoToolbox sessions and CoreVideo pools may be used from any thread (one at a
// time, which `&mut self` ensures), and the sink is a Mutex.
unsafe impl Send for Session {}

impl Session {
    fn new(size: (u32, u32), bitrate: u32, fps: u32) -> Result<Self, String> {
        let (w, h) = size;
        let pixels = nv12_attributes(w, h);
        // SAFETY: an extern constant (there since macOS 10.9).
        let hw = unsafe { kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder };
        // Prefer the hardware encoder; VideoToolbox's software one where there's none.
        let spec = CFDictionary::<CFString, CFBoolean>::from_slices(&[hw], &[CFBoolean::new(true)]);
        let sink = Arc::<Sink>::default();
        let mut out = ptr::null_mut();
        // SAFETY: valid dictionaries and out-pointer; the callback's ref-con is `sink`, which the
        // session holds and outlives VideoToolbox's use of it (see Drop).
        let status = unsafe {
            VTCompressionSession::create(
                None,
                w as i32,
                h as i32,
                kCMVideoCodecType_H264,
                Some(spec.as_ref()),
                Some(pixels.as_ref()),
                None,
                Some(on_output),
                Arc::as_ptr(&sink).cast_mut().cast(),
                NonNull::from(&mut out),
            )
        };
        // SAFETY: a Create function: the session (if any) is ours to release.
        let vt = NonNull::new(out).map(|p| unsafe { CFRetained::from_raw(p) });
        let vt = vt.filter(|_| status == 0).ok_or_else(|| format!("can't make an H.264 session ({status})"))?;
        let mut out = ptr::null_mut();
        // SAFETY: valid attributes and out-pointer.
        let status = unsafe { CVPixelBufferPool::create(None, None, Some(pixels.as_ref()), NonNull::from(&mut out)) };
        // SAFETY: a Create function: the pool (if any) is ours to release.
        let pool = NonNull::new(out).map(|p| unsafe { CFRetained::from_raw(p) });
        let pool = pool.filter(|_| status == kCVReturnSuccess).ok_or_else(|| format!("can't make NV12 pictures ({status})"))?;
        let mut s = Self { vt, pool, sink, size, rate: (0, 0), hardware: false };
        s.configure()?;
        s.set_rate(bitrate, fps)?;
        // SAFETY: a live session. (Optional: it only makes the first frame quicker.)
        unsafe { s.vt.prepare_to_encode_frames() };
        s.hardware = s.using_hardware();
        Ok(s)
    }

    fn configure(&self) -> Result<(), String> {
        // SAFETY: extern constants, all there since macOS 10.9.
        let (profile, baseline, realtime, reorder, key_interval, slice, matrix, primaries, transfer) = unsafe {
            (
                kVTCompressionPropertyKey_ProfileLevel,
                kVTProfileLevel_H264_Baseline_AutoLevel,
                kVTCompressionPropertyKey_RealTime,
                kVTCompressionPropertyKey_AllowFrameReordering,
                kVTCompressionPropertyKey_MaxKeyFrameInterval,
                kVTCompressionPropertyKey_MaxH264SliceBytes,
                kVTCompressionPropertyKey_YCbCrMatrix,
                kVTCompressionPropertyKey_ColorPrimaries,
                kVTCompressionPropertyKey_TransferFunction,
            )
        };
        // Constrained Baseline: what every WebRTC peer takes (`profile-level-id=42e01f`).
        // Plain Baseline before macOS 12 (Apple's encoder uses no Baseline-only tools anyway).
        let mut status = 0;
        let profiled = constrained_baseline().into_iter().chain([baseline]).any(|p| {
            status = self.set(profile, p);
            status == 0
        });
        if !profiled {
            return Err(format!("no Baseline profile ({status})"));
        }
        let (on, off) = (CFBoolean::new(true), CFBoolean::new(false));
        // Keyframes only when asked (a viewer joins or loses packets).
        let never = CFNumber::new_i32(i32::MAX);
        let slice_bytes = CFNumber::new_i32(MAX_SLICE);
        // Tagged BT.601 limited range, the matrix `to_nv12` uses (the primaries and
        // transfer are a screen's, sRGB ≈ BT.709).
        // SAFETY: extern constants, all there since macOS 10.8.
        let (bt601, bt709, bt709_transfer) =
            unsafe { (kCVImageBufferYCbCrMatrix_ITU_R_601_4, kCVImageBufferColorPrimaries_ITU_R_709_2, kCVImageBufferTransferFunction_ITU_R_709_2) };
        let wanted: [(&str, &CFString, &CFType); 7] = [
            ("real time", realtime, on),
            ("no B-frames", reorder, off),
            ("no periodic keyframes", key_interval, &never),
            ("slice size", slice, &slice_bytes),
            ("BT.601 matrix", matrix, bt601),
            ("BT.709 primaries", primaries, bt709),
            ("BT.709 transfer", transfer, bt709_transfer),
        ];
        // Not fatal: Baseline already has no B-frames, and the rest only tunes the stream.
        for (what, key, value) in wanted {
            match self.set(key, value) {
                0 => {}
                e => log::debug!("H.264: VideoToolbox won't set {what} ({e})"),
            }
        }
        Ok(())
    }

    /// The target bitrate and the frame rate, on a new or running session.
    ///
    /// No DataRateLimits cap: with one set (even only when the session is made), the
    /// hardware encoder ignores later AverageBitRate changes, and a lowered cap never goes
    /// back up, so backing off would stick (seen on Apple Silicon, macOS 27). AverageBitRate
    /// alone tracks the target both ways within a frame or two (400 kbps → ~375, 8 Mbps → ~7.9).
    fn set_rate(&mut self, bitrate: u32, fps: u32) -> Result<(), String> {
        // SAFETY: extern constants, all there since macOS 10.9.
        let (average, expected) = unsafe { (kVTCompressionPropertyKey_AverageBitRate, kVTCompressionPropertyKey_ExpectedFrameRate) };
        let status = self.set(average, &CFNumber::new_i32(bitrate.min(i32::MAX as u32) as i32));
        if status != 0 {
            return Err(format!("can't set the bitrate ({status})"));
        }
        if fps != self.rate.1 {
            match self.set(expected, &CFNumber::new_i32(fps as i32)) {
                0 => {}
                e => log::debug!("H.264: VideoToolbox won't take the frame rate ({e})"),
            }
        }
        self.rate = (bitrate, fps);
        Ok(())
    }

    /// Set one session property; VideoToolbox's status (0 when it took it).
    fn set(&self, key: &CFString, value: &CFType) -> i32 {
        // SAFETY: a live session; VideoToolbox checks the key and the value's type itself.
        unsafe { VTSessionSetProperty(&self.vt, key, Some(value)) }
    }

    fn using_hardware(&self) -> bool {
        let mut out: *const CFType = ptr::null();
        // SAFETY: a live session and an extern constant key (macOS 10.9+); the value is copied out to `out`.
        let status = unsafe { VTSessionCopyProperty(&self.vt, kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder, None, (&raw mut out).cast()) };
        // SAFETY: a Copy function: the value (if any) is ours to release.
        let value = NonNull::new(out.cast_mut()).map(|p| unsafe { CFRetained::from_raw(p) });
        status == 0 && value.and_then(|v| v.downcast::<CFBoolean>().ok()).is_some_and(|b| b.as_bool())
    }

    /// Encode one picture and wait for it: (Annex B, keyframe).
    fn encode(&mut self, frame: &Frame, pts: i64, step: i64, keyframe: bool) -> Result<(Vec<u8>, bool), String> {
        let picture = self.picture(frame)?;
        // SAFETY: an extern constant (macOS 10.8+).
        let force = unsafe { kVTEncodeFrameOptionKey_ForceKeyFrame };
        let options = keyframe.then(|| CFDictionary::<CFString, CFBoolean>::from_slices(&[force], &[CFBoolean::new(true)]));
        let options: Option<&CFDictionary> = options.as_deref().map(|d| d.as_ref());
        let time = |value| CMTime { value, timescale: CLOCK, flags: CMTimeFlags::Valid, epoch: 0 };
        // SAFETY: a live session, an NV12 picture of its size, and a CFString → CFBoolean dictionary.
        let status = unsafe { self.vt.encode_frame(&picture, time(pts), time(step), options, ptr::null_mut(), ptr::null_mut()) };
        if status != 0 {
            return Err(format!("H.264 encode: VideoToolbox error {status}"));
        }
        // Everything up to this frame comes out now (through `on_output`).
        // SAFETY: a live session.
        let status = unsafe { self.vt.complete_frames(time(pts)) };
        if status != 0 {
            return Err(format!("H.264 encode: VideoToolbox error {status}"));
        }
        let mut data = Vec::new();
        let mut key = None;
        for out in std::mem::take(&mut *lock(&self.sink)) {
            match out {
                Output::Coded { data: d, keyframe } => {
                    key.get_or_insert(keyframe);
                    data.extend_from_slice(&d);
                }
                Output::Dropped => {}
                Output::Failed(e) => return Err(e),
            }
        }
        Ok((data, key.unwrap_or(false)))
    }

    /// `frame`, scaled and converted into an NV12 picture from the pool.
    fn picture(&self, frame: &Frame) -> Result<CFRetained<CVPixelBuffer>, String> {
        let mut out = ptr::null_mut();
        // SAFETY: a live pool and a valid out-pointer.
        let status = unsafe { CVPixelBufferPool::create_pixel_buffer(None, &self.pool, NonNull::from(&mut out)) };
        // SAFETY: a Create function: the buffer (if any) is ours to release.
        let pic = NonNull::new(out).map(|p| unsafe { CFRetained::from_raw(p) });
        let pic = pic.filter(|_| status == kCVReturnSuccess).ok_or_else(|| format!("H.264 encode: no picture ({status})"))?;
        let (w, h) = (self.size.0 as usize, self.size.1 as usize);
        let planes = [(0, w, h), (1, w / 2, h / 2)];
        if planes.iter().any(|&(i, pw, ph)| CVPixelBufferGetWidthOfPlane(&pic, i) != pw || CVPixelBufferGetHeightOfPlane(&pic, i) != ph) {
            return Err("H.264 encode: an unexpected picture layout".into());
        }
        // SAFETY: a buffer only we hold; unlocked below.
        let status = unsafe { CVPixelBufferLockBaseAddress(&pic, CVPixelBufferLockFlags::empty()) };
        if status != kCVReturnSuccess {
            return Err(format!("H.264 encode: can't write the picture ({status})"));
        }
        let (y, y_stride) = (CVPixelBufferGetBaseAddressOfPlane(&pic, 0).cast::<u8>(), CVPixelBufferGetBytesPerRowOfPlane(&pic, 0));
        let (uv, uv_stride) = (CVPixelBufferGetBaseAddressOfPlane(&pic, 1).cast::<u8>(), CVPixelBufferGetBytesPerRowOfPlane(&pic, 1));
        let ok = !y.is_null() && !uv.is_null() && y_stride >= w && uv_stride >= w;
        if ok {
            // SAFETY: while locked, plane 0 holds `h` rows of `w` bytes, `y_stride` apart, and
            // plane 1 `h / 2` rows of `w` bytes (U V pairs), `uv_stride` apart (checked above).
            let (y, uv) =
                unsafe { (std::slice::from_raw_parts_mut(y, y_stride * (h - 1) + w), std::slice::from_raw_parts_mut(uv, uv_stride * (h / 2 - 1) + w)) };
            to_nv12(frame, (w, h), (y, y_stride), (uv, uv_stride));
        }
        // SAFETY: matches the lock above.
        unsafe { CVPixelBufferUnlockBaseAddress(&pic, CVPixelBufferLockFlags::empty()) };
        if !ok {
            return Err("H.264 encode: an unexpected picture layout".into());
        }
        // SAFETY: extern constants (macOS 10.4+) of the types CoreVideo expects for these keys.
        unsafe {
            pic.set_attachment(kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_601_4, CVAttachmentMode::ShouldPropagate);
            pic.set_attachment(kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_ITU_R_709_2, CVAttachmentMode::ShouldPropagate);
            pic.set_attachment(kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_ITU_R_709_2, CVAttachmentMode::ShouldPropagate);
        }
        Ok(pic)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // An invalid time: every frame still pending.
        let all = CMTime { value: 0, timescale: 0, flags: CMTimeFlags::empty(), epoch: 0 };
        // SAFETY: a live session. Once invalidated, VideoToolbox no longer calls back into `sink`.
        unsafe {
            self.vt.complete_frames(all);
            self.vt.invalidate();
        }
    }
}

/// NV12 (BT.601 limited range), IOSurface-backed (the hardware encoder reads it without a copy).
fn nv12_attributes(w: u32, h: u32) -> CFRetained<CFDictionary<CFString, CFType>> {
    // SAFETY: extern constants (macOS 10.4+).
    let (format, width, height, iosurface) =
        unsafe { (kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey, kCVPixelBufferHeightKey, kCVPixelBufferIOSurfacePropertiesKey) };
    let nv12 = CFNumber::new_i32(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange as i32);
    let (w, h) = (CFNumber::new_i32(w as i32), CFNumber::new_i32(h as i32));
    let surface = CFDictionary::<CFString, CFType>::empty();
    CFDictionary::from_slices(&[format, width, height, iosurface], &[&nv12, &w, &h, &surface])
}

/// Constrained Baseline (macOS 12+), looked up at run time: a plain reference
/// to the symbol would stop Yap from starting at all on macOS 11.
fn constrained_baseline() -> Option<&'static CFString> {
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    // RTLD_DEFAULT ((void *)-2): every image loaded, VideoToolbox among them.
    let everywhere = ptr::without_provenance_mut(-2isize as usize);
    // SAFETY: a NUL-terminated name.
    let symbol = unsafe { dlsym(everywhere, c"kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel".as_ptr()) };
    // SAFETY: the symbol is a `const CFStringRef` VideoToolbox keeps for the life of the process.
    NonNull::new(symbol.cast::<*const CFString>()).and_then(|p| unsafe { p.read().as_ref() })
}

/// VideoToolbox's output callback: once per frame, on its own thread or inside `complete_frames`.
unsafe extern "C-unwind" fn on_output(sink: *mut c_void, _frame: *mut c_void, status: i32, flags: VTEncodeInfoFlags, sample: *mut CMSampleBuffer) {
    // SAFETY: the ref-con is the session's `Arc<Sink>`, alive until the session is invalidated.
    let sink = unsafe { &*sink.cast_const().cast::<Sink>() };
    // SAFETY: VideoToolbox passes a valid sample buffer (or null) for the length of the call.
    let sample = unsafe { sample.as_ref() };
    // Never unwind into VideoToolbox.
    let out = catch_unwind(AssertUnwindSafe(|| match sample {
        _ if status != 0 => Output::Failed(format!("H.264 encode: VideoToolbox error {status}")),
        Some(sample) if !flags.contains(VTEncodeInfoFlags::FrameDropped) => match annex_b(sample) {
            Ok((data, keyframe)) => Output::Coded { data, keyframe },
            Err(e) => Output::Failed(e),
        },
        _ => Output::Dropped,
    }))
    .unwrap_or_else(|_| Output::Failed("H.264 encode: reading the output failed".into()));
    lock(sink).push(out);
}

/// One encoded frame (length-prefixed NAL units) → Annex B, with the SPS and
/// PPS in front when it's a keyframe; and whether it is one.
fn annex_b(sample: &CMSampleBuffer) -> Result<(Vec<u8>, bool), String> {
    let keyframe = is_sync(sample);
    // SAFETY: a valid sample buffer.
    let (format, block) = unsafe { (sample.format_description(), sample.data_buffer()) };
    let (format, block) = format.zip(block).ok_or("H.264 encode: an empty sample")?;
    let (mut count, mut prefix): (usize, c_int) = (0, 0);
    // SAFETY: a valid format description; null for the outputs we don't want.
    let status = unsafe { CMVideoFormatDescriptionGetH264ParameterSetAtIndex(&format, 0, ptr::null_mut(), ptr::null_mut(), &mut count, &mut prefix) };
    if status != 0 || !(1..=4).contains(&prefix) {
        return Err(format!("H.264 encode: no parameter sets ({status})"));
    }
    // SAFETY: a valid block buffer.
    let len = unsafe { block.data_length() };
    let mut avcc = vec![0u8; len];
    // SAFETY: `avcc` has room for `len` bytes.
    let status = unsafe { block.copy_data_bytes(0, len, NonNull::from(avcc.as_mut_slice()).cast()) };
    if status != 0 {
        return Err(format!("H.264 encode: can't read the output ({status})"));
    }
    let mut out = Vec::with_capacity(len + 64);
    if keyframe {
        for i in 0..count {
            let (mut set, mut size) = (ptr::null(), 0);
            // SAFETY: as above.
            let status = unsafe { CMVideoFormatDescriptionGetH264ParameterSetAtIndex(&format, i, &mut set, &mut size, ptr::null_mut(), ptr::null_mut()) };
            if status != 0 || set.is_null() {
                return Err(format!("H.264 encode: no parameter set {i} ({status})"));
            }
            out.extend_from_slice(&START);
            // SAFETY: `size` bytes inside `format`, which we hold.
            out.extend_from_slice(unsafe { std::slice::from_raw_parts(set, size) });
        }
    }
    avcc_to_annex_b(&avcc, prefix as usize, &mut out)?;
    Ok((out, keyframe))
}

/// Length-prefixed NAL units (`prefix`-byte big-endian lengths) → start codes,
/// appended to `out`. SEI is left out: VideoToolbox's carries its own user data
/// (and HRD timing), which no peer needs; a keyframe then reads SPS, PPS, IDR.
fn avcc_to_annex_b(mut avcc: &[u8], prefix: usize, out: &mut Vec<u8>) -> Result<(), String> {
    const SEI: u8 = 6;
    while !avcc.is_empty() {
        let (len, rest) = avcc.split_at_checked(prefix).ok_or("H.264 encode: a cut-off NAL unit")?;
        let len = len.iter().fold(0usize, |n, &b| n << 8 | b as usize);
        let (nal, rest) = rest.split_at_checked(len).ok_or("H.264 encode: a cut-off NAL unit")?;
        if nal.first().is_some_and(|h| h & 0x1f != SEI) {
            out.extend_from_slice(&START);
            out.extend_from_slice(nal);
        }
        avcc = rest;
    }
    Ok(())
}

/// A sync sample (a keyframe) unless its attachments say NotSync.
fn is_sync(sample: &CMSampleBuffer) -> bool {
    // SAFETY: a valid sample buffer. No attachments at all means a sync sample.
    let Some(attachments) = (unsafe { sample.sample_attachments_array(false) }) else {
        return true;
    };
    // SAFETY: CoreMedia documents this as an array of CFDictionary, one per sample.
    let attachments: CFRetained<CFArray<CFDictionary<CFString, CFType>>> = unsafe { CFRetained::cast_unchecked(attachments) };
    let Some(first) = attachments.get(0) else { return true };
    // SAFETY: an extern constant (macOS 10.7+).
    let not_sync = unsafe { kCMSampleAttachmentKey_NotSync };
    !first.get(not_sync).and_then(|v| v.downcast::<CFBoolean>().ok()).is_some_and(|b| b.as_bool())
}

/// BGRA (any stride) → NV12 at `size` (BT.601, limited range): the same math as
/// `codec::convert`, with U and V interleaved in one plane. Planes may be padded.
fn to_nv12(src: &Frame, (dw, dh): (usize, usize), (y_plane, y_stride): (&mut [u8], usize), (uv_plane, uv_stride): (&mut [u8], usize)) {
    let (sw, sh) = (src.width as usize, src.height as usize);
    // Source columns/rows for each output column/row: the box's first and last pixel.
    let xs: Vec<(usize, usize)> = (0..dw).map(|x| (x * sw / dw, (((x + 1) * sw / dw).max(x * sw / dw + 1) - 1).min(sw - 1))).collect();
    let ys: Vec<(usize, usize)> = (0..dh).map(|y| (y * sh / dh, (((y + 1) * sh / dh).max(y * sh / dh + 1) - 1).min(sh - 1))).collect();
    let px = |x: usize, y: usize| -> (i32, i32, i32) {
        let i = y * src.stride + x * 4;
        (src.data[i + 2] as i32, src.data[i + 1] as i32, src.data[i] as i32)
    };
    let sample = |x: usize, y: usize| -> (i32, i32, i32) {
        let (x0, x1) = xs[x];
        let (y0, y1) = ys[y];
        if x0 == x1 && y0 == y1 {
            return px(x0, y0);
        }
        let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        ((a.0 + b.0 + c.0 + d.0 + 2) >> 2, (a.1 + b.1 + c.1 + d.1 + 2) >> 2, (a.2 + b.2 + c.2 + d.2 + 2) >> 2)
    };
    let luma = |(r, g, b): (i32, i32, i32)| (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
    for cy in 0..dh / 2 {
        let y = cy * 2;
        let (top, bottom) = y_plane[y * y_stride..].split_at_mut(y_stride);
        let uv_row = &mut uv_plane[cy * uv_stride..cy * uv_stride + dw];
        for cx in 0..dw / 2 {
            let x = cx * 2;
            let p = [sample(x, y), sample(x + 1, y), sample(x, y + 1), sample(x + 1, y + 1)];
            top[x] = luma(p[0]);
            top[x + 1] = luma(p[1]);
            bottom[x] = luma(p[2]);
            bottom[x + 1] = luma(p[3]);
            let (r, g, b) = ((p[0].0 + p[1].0 + p[2].0 + p[3].0) >> 2, (p[0].1 + p[1].1 + p[2].1 + p[3].1) >> 2, (p[0].2 + p[1].2 + p[2].2 + p[3].2) >> 2);
            uv_row[x] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            uv_row[x + 1] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::capture::pattern;
    use super::*;
    use openh264::OpenH264API;
    use openh264::decoder::{Decoder, DecoderConfig, Flush};
    use openh264::formats::YUVSource;
    use std::time::{Duration, Instant};

    /// A different decoder from the encoder: what a peer would see.
    fn decoder() -> Decoder {
        Decoder::with_api_config(OpenH264API::from_source(), DecoderConfig::new().flush_after_decode(Flush::Flush)).unwrap()
    }

    /// One access unit → (width, height, RGBA), if a picture came out.
    fn decode(dec: &mut Decoder, au: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
        let yuv = dec.decode(au).unwrap()?;
        let (w, h) = yuv.dimensions();
        let mut rgba = vec![0u8; w * h * 4];
        yuv.write_rgba8(&mut rgba);
        Some((w as u32, h as u32, rgba))
    }

    /// NAL units without their start codes.
    fn nals(au: &[u8]) -> Vec<&[u8]> {
        openh264::nal_units(au).map(|n| &n[n.iter().position(|&b| b == 1).unwrap() + 1..]).collect()
    }

    fn types(au: &[u8]) -> Vec<u8> {
        nals(au).iter().map(|n| n[0] & 0x1f).collect()
    }

    fn white(rgba: &[u8]) -> bool {
        rgba[0] > 200 && rgba[1] > 200 && rgba[2] > 200
    }

    /// The pattern with faint noise everywhere: costly at a high bitrate, cheap
    /// at a low one. (Full-range noise costs about the same at any bitrate.)
    fn noisy(n: u64) -> Frame {
        let mut f = pattern::draw(1280, 720, n);
        let mut seed = n.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        for px in f.data.as_chunks_mut::<4>().0 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            for (c, r) in px[..3].iter_mut().zip(seed.to_le_bytes()) {
                *c = c.saturating_add(r % 32);
            }
        }
        f
    }

    #[test]
    fn converts_colours() {
        // Pure red, green, blue, white → BT.601 limited-range Y, U, V.
        for (bgra, yuv) in [
            ([0u8, 0, 255, 255], [82u8, 90, 240]),
            ([0, 255, 0, 255], [145, 54, 34]),
            ([255, 0, 0, 255], [41, 240, 110]),
            ([255, 255, 255, 255], [235, 128, 128]),
        ] {
            // Padded rows on both sides: 6 pixels in a 32-byte stride, planes 8 bytes wide.
            let mut data = vec![0u8; 32 * 4];
            for row in data.as_chunks_mut::<32>().0 {
                for px in row[..24].as_chunks_mut::<4>().0 {
                    *px = bgra;
                }
            }
            let frame = Frame { width: 6, height: 4, stride: 32, data };
            let (mut y, mut uv) = (vec![7u8; 8 * 4], vec![7u8; 8 * 2]);
            to_nv12(&frame, (6, 4), (&mut y, 8), (&mut uv, 8));
            for row in 0..4 {
                assert!(y[row * 8..row * 8 + 6].iter().all(|&v| v.abs_diff(yuv[0]) <= 1), "{bgra:?} → Y {:?}", &y[..8]);
                assert_eq!(&y[row * 8 + 6..row * 8 + 8], &[7, 7], "padding left alone");
            }
            for row in 0..2 {
                let uv = &uv[row * 8..row * 8 + 8];
                assert!(uv[..6].as_chunks::<2>().0.iter().all(|p| p[0].abs_diff(yuv[1]) <= 1 && p[1].abs_diff(yuv[2]) <= 1), "{bgra:?} → UV {uv:?}");
                assert_eq!(&uv[6..], &[7, 7], "padding left alone");
            }
        }
    }

    #[test]
    fn length_prefixes_become_start_codes() {
        let mut out = Vec::new();
        // An SEI (dropped), an IDR slice and a P slice, with 4-byte lengths.
        avcc_to_annex_b(&[0, 0, 0, 2, 0x06, 0x05, 0, 0, 0, 3, 0x65, 0xaa, 0xbb, 0, 0, 0, 1, 0x41], 4, &mut out).unwrap();
        assert_eq!(out, [0, 0, 0, 1, 0x65, 0xaa, 0xbb, 0, 0, 0, 1, 0x41]);
        assert!(avcc_to_annex_b(&[0, 0, 0, 9, 0x65], 4, &mut Vec::new()).is_err(), "cut off");
        assert!(avcc_to_annex_b(&[0, 0], 4, &mut Vec::new()).is_err(), "cut-off length");
    }

    /// The pattern survives VideoToolbox → OpenH264 at the right size: SPS,
    /// PPS and IDR first, Constrained Baseline, and no keyframes unasked.
    #[test]
    fn round_trip() {
        let mut enc = HwEncoder::new(2_000_000, 30).expect("VideoToolbox H.264 on this Mac");
        let mut dec = decoder();
        let mut decoded = 0;
        for n in 0..10 {
            let e = enc.encode(&pattern::draw(1280, 720, n), (1920, 1080), n == 0).unwrap();
            assert_eq!((e.width, e.height), (1280, 720));
            if n == 0 {
                assert!(e.keyframe, "starts with a keyframe");
                assert_eq!(&types(&e.data)[..3], &[7, 8, 5], "SPS, PPS, then IDR: {:?}", types(&e.data));
                assert!(e.data.starts_with(&START), "Annex B");
                let sps = nals(&e.data)[0];
                assert_eq!(sps[1], 66, "profile_idc: Baseline");
                assert_ne!(sps[2] & 0x40, 0, "constraint_set1: Constrained Baseline");
            } else {
                assert!(!e.keyframe && !types(&e.data).contains(&5), "frame {n}: no keyframe unasked");
            }
            let (w, h, rgba) = decode(&mut dec, &e.data).expect("a picture per frame");
            assert_eq!((w, h), (1280, 720));
            // The first colour bar is white(ish).
            assert!(white(&rgba), "{:?}", &rgba[..4]);
            decoded += 1;
        }
        assert_eq!(decoded, 10);
    }

    #[test]
    fn keyframes_on_request() {
        let mut enc = HwEncoder::new(2_000_000, 30).unwrap();
        for n in 0..6 {
            let e = enc.encode(&pattern::draw(1280, 720, n), (1920, 1080), n == 0 || n == 4).unwrap();
            assert_eq!(e.keyframe, n == 0 || n == 4, "frame {n}");
            if e.keyframe {
                assert_eq!(&types(&e.data)[..3], &[7, 8, 5], "frame {n}: {:?}", types(&e.data));
            } else {
                assert!(!types(&e.data).iter().any(|&t| t == 5 || t == 7), "frame {n}: {:?}", types(&e.data));
            }
        }
    }

    #[test]
    fn downscales_while_encoding() {
        let mut enc = HwEncoder::new(4_000_000, 30).unwrap();
        let mut dec = decoder();
        let e = enc.encode(&pattern::draw(1280, 720, 0), (640, 640), true).unwrap();
        assert_eq!((e.width, e.height), (640, 360));
        let (w, h, rgba) = decode(&mut dec, &e.data).unwrap();
        assert_eq!((w, h), (640, 360));
        assert!(white(&rgba), "{:?}", &rgba[..4]);
        // A new size is a new session, which starts with a keyframe even unasked.
        let e = enc.encode(&pattern::draw(1280, 720, 1), (1920, 1080), false).unwrap();
        assert_eq!((e.width, e.height), (1280, 720));
        assert!(e.keyframe && types(&e.data).starts_with(&[7, 8, 5]));
        assert_eq!(decode(&mut dec, &e.data).map(|d| (d.0, d.1)), Some((1280, 720)));
    }

    /// A bitrate change on the running session: no keyframe, still decodable,
    /// and the frames really shrink (then grow back).
    #[test]
    fn live_bitrate_change() {
        let mut enc = HwEncoder::new(8_000_000, 30).unwrap();
        let mut dec = decoder();
        let mut sizes = Vec::new();
        for n in 0..60 {
            match n {
                20 => enc.set_bitrate_live(400_000),
                40 => enc.set_bitrate_live(8_000_000),
                _ => {}
            }
            let e = enc.encode(&noisy(n), (1920, 1080), n == 0).unwrap();
            assert_eq!(e.keyframe, n == 0, "frame {n}");
            assert_eq!(types(&e.data).contains(&5), n == 0, "frame {n}");
            assert!(decode(&mut dec, &e.data).is_some(), "frame {n} decodes");
            sizes.push(e.data.len());
        }
        let average = |r: std::ops::Range<usize>| sizes[r.clone()].iter().sum::<usize>() / r.len();
        let (high, low, again) = (average(10..20), average(30..40), average(50..60));
        assert!(low * 4 < high, "{high} → {low} bytes per frame");
        assert!(again > low * 4, "{low} → {again} bytes per frame");
    }

    /// `set_rate` lands on the next encode without a new session (so no keyframe).
    #[test]
    fn rate_change_on_next_frame() {
        let mut enc = HwEncoder::new(2_000_000, 30).unwrap();
        enc.encode(&pattern::draw(1280, 720, 0), (1920, 1080), true).unwrap();
        enc.set_rate(1_000_000, 15);
        let e = enc.encode(&pattern::draw(1280, 720, 1), (1920, 1080), false).unwrap();
        assert!(!e.keyframe && !e.data.is_empty());
        assert_eq!(enc.session.as_ref().map(|s| s.rate), Some((1_000_000, 15)));
    }

    #[test]
    fn refuses_broken_frames() {
        let mut enc = HwEncoder::new(2_000_000, 30).unwrap();
        let short = Frame { width: 64, height: 64, stride: 256, data: vec![0; 100] };
        assert!(enc.encode(&short, (1920, 1080), true).is_err());
        let empty = Frame { width: 0, height: 0, stride: 0, data: Vec::new() };
        assert!(enc.encode(&empty, (1920, 1080), true).is_err());
    }

    #[test]
    fn can_move_between_threads() {
        fn send<T: Send>() {}
        send::<HwEncoder>();
    }

    /// `cargo test --release -- --ignored --nocapture timing`
    #[test]
    #[ignore = "a timing, not a test"]
    fn timing() {
        for (source, max) in [((1920, 1080), (1920, 1080)), ((2560, 1600), (1920, 1080))] {
            let mut enc = HwEncoder::new(6_000_000, 30).unwrap();
            let frames: Vec<Frame> = (0..8).map(|n| pattern::draw(source.0, source.1, n)).collect();
            let (mut total, mut convert) = (Duration::ZERO, Duration::ZERO);
            let runs = 120;
            for n in 0..runs + 5 {
                let f = &frames[n % frames.len()];
                let t = Instant::now();
                let e = enc.encode(f, max, n == 0).unwrap();
                // The first few warm the session up.
                if n >= 5 {
                    total += t.elapsed();
                }
                assert!(!e.data.is_empty());
                if n == 5 {
                    let (w, h) = (e.width as usize, e.height as usize);
                    let (mut y, mut uv) = (vec![0u8; w * h], vec![0u8; w * h / 2]);
                    let t = Instant::now();
                    for _ in 0..20 {
                        to_nv12(f, (w, h), (&mut y, w), (&mut uv, w));
                    }
                    convert = t.elapsed() / 20;
                }
            }
            println!(
                "{}×{} → {:?}: {:.2} ms per frame ({:.2} ms of it BGRA → NV12), hardware {}",
                source.0,
                source.1,
                fit(source.0, source.1, max.0, max.1),
                total.as_secs_f64() * 1000.0 / runs as f64,
                convert.as_secs_f64() * 1000.0,
                enc.hardware()
            );
        }
    }
}
