@preconcurrency import AVFoundation
@preconcurrency import CoreMedia
import CoreVideo
import Foundation
import os
@preconcurrency import WebRTC

/// Shows someone's screen: decoded frames from their video track go into an
/// `AVSampleBufferDisplayLayer` (letterboxed with `.resizeAspect`, never
/// cropped or stretched), which is also what picture-in-picture plays from.
/// Counts frames, so the viewer can tell connecting / playing / paused.
///
/// WebRTC calls `renderFrame` on its decoding thread; frames are handed to
/// the layer's renderer from a serial queue, never the main thread.
public final class ScreenRenderer: NSObject, RTCVideoRenderer, @unchecked Sendable {
    private struct State {
        var frames = 0
        var lastFrameAt: TimeInterval?
        var size: CGSize = .zero
        var format: CMVideoFormatDescription?
        var pool: CVPixelBufferPool?
        var poolSize: CGSize = .zero
    }

    /// Where the picture goes. Put it in a view (its `videoGravity` is aspect fit).
    public let displayLayer = AVSampleBufferDisplayLayer()
    private let videoRenderer: AVSampleBufferVideoRenderer
    /// Guarded by `lock` (WebRTC's thread writes, the main thread reads).
    private var state = State()
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "yap.screen.render")

    @MainActor
    public override init() {
        videoRenderer = displayLayer.sampleBufferRenderer
        super.init()
        displayLayer.videoGravity = .resizeAspect
        displayLayer.backgroundColor = CGColor(gray: 0, alpha: 1)
    }

    /// Frames shown so far, and when the last one came (system uptime).
    public var counters: (frames: Int, lastFrameAt: TimeInterval?) {
        lock.withLock { (state.frames, state.lastFrameAt) }
    }

    /// The picture's size in pixels (zero before the first frame).
    public var videoSize: CGSize { lock.withLock { state.size } }

    // MARK: RTCVideoRenderer

    public func setSize(_ size: CGSize) {}

    public func renderFrame(_ frame: RTCVideoFrame?) {
        guard let frame, let pixelBuffer = pixelBuffer(for: frame) else { return }
        let now = ProcessInfo.processInfo.systemUptime
        lock.withLock {
            state.frames += 1
            state.lastFrameAt = now
            state.size = CGSize(width: Int(frame.width), height: Int(frame.height))
        }
        guard let sample = sampleBuffer(for: pixelBuffer) else { return }
        // The renderer takes buffers from any thread; one queue keeps them in order.
        let work = Unchecked((videoRenderer, sample))
        queue.async {
            let (renderer, sample) = work.value
            if renderer.status == .failed || renderer.requiresFlushToResumeDecoding { renderer.flush() }
            renderer.enqueue(sample)
        }
    }

    /// Forget the picture (the share ended).
    public func clear() {
        let renderer = Unchecked(videoRenderer)
        lock.withLock { state.size = .zero }
        queue.async { renderer.value.flush(removingDisplayedImage: true, completionHandler: nil) }
    }

    // MARK: Frames → sample buffers

    /// Hardware-decoded H.264 arrives as a CVPixelBuffer already; software
    /// decoders (VP8/VP9) give I420, copied into an NV12 buffer.
    private func pixelBuffer(for frame: RTCVideoFrame) -> CVPixelBuffer? {
        if let buffer = frame.buffer as? RTCCVPixelBuffer, !buffer.requiresCropping() {
            return buffer.pixelBuffer
        }
        let i420 = frame.buffer.toI420()
        let width = Int(i420.width), height = Int(i420.height)
        guard width > 0, height > 0, let pool = pool(width: width, height: height) else { return nil }
        var out: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out) == kCVReturnSuccess, let out else { return nil }
        CVPixelBufferLockBaseAddress(out, [])
        defer { CVPixelBufferUnlockBaseAddress(out, []) }
        guard let y = CVPixelBufferGetBaseAddressOfPlane(out, 0), let uv = CVPixelBufferGetBaseAddressOfPlane(out, 1) else { return nil }
        let yStride = CVPixelBufferGetBytesPerRowOfPlane(out, 0)
        let uvStride = CVPixelBufferGetBytesPerRowOfPlane(out, 1)
        for row in 0..<height {
            memcpy(y + row * yStride, i420.dataY + row * Int(i420.strideY), width)
        }
        let chromaWidth = Int(i420.chromaWidth), chromaHeight = Int(i420.chromaHeight)
        for row in 0..<chromaHeight {
            let dst = (uv + row * uvStride).assumingMemoryBound(to: UInt8.self)
            let u = i420.dataU + row * Int(i420.strideU)
            let v = i420.dataV + row * Int(i420.strideV)
            for col in 0..<chromaWidth {
                dst[col * 2] = u[col]
                dst[col * 2 + 1] = v[col]
            }
        }
        return out
    }

    private func pool(width: Int, height: Int) -> CVPixelBufferPool? {
        lock.withLock {
            if let pool = state.pool, state.poolSize == CGSize(width: width, height: height) { return pool }
            let attributes: [String: Any] = [
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                kCVPixelBufferWidthKey as String: width,
                kCVPixelBufferHeightKey as String: height,
                kCVPixelBufferIOSurfacePropertiesKey as String: [:] as [String: Any],
            ]
            var pool: CVPixelBufferPool?
            CVPixelBufferPoolCreate(nil, nil, attributes as CFDictionary, &pool)
            state.pool = pool
            state.poolSize = CGSize(width: width, height: height)
            return pool
        }
    }

    private func sampleBuffer(for pixelBuffer: CVPixelBuffer) -> CMSampleBuffer? {
        let format: CMVideoFormatDescription? = lock.withLock {
            if let format = state.format, CMVideoFormatDescriptionMatchesImageBuffer(format, imageBuffer: pixelBuffer) { return format }
            var created: CMVideoFormatDescription?
            CMVideoFormatDescriptionCreateForImageBuffer(allocator: nil, imageBuffer: pixelBuffer, formatDescriptionOut: &created)
            state.format = created
            return created
        }
        guard let format else { return nil }
        var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: CMClockGetTime(CMClockGetHostTimeClock()), decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        guard CMSampleBufferCreateReadyWithImageBuffer(allocator: nil, imageBuffer: pixelBuffer, formatDescription: format,
                                                       sampleTiming: &timing, sampleBufferOut: &sample) == noErr, let sample else { return nil }
        // Live: show each frame as it comes, no timebase.
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true), CFArrayGetCount(attachments) > 0 {
            let dictionary = unsafeBitCast(CFArrayGetValueAtIndex(attachments, 0), to: CFMutableDictionary.self)
            CFDictionarySetValue(dictionary, Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(),
                                 Unmanaged.passUnretained(kCFBooleanTrue).toOpaque())
        }
        return sample
    }
}

/// A value handed to another queue that's safe there by design (WebRTC's and
/// AVFoundation's objects that document thread safety but aren't marked).
struct Unchecked<Value>: @unchecked Sendable {
    let value: Value
    init(_ value: Value) { self.value = value }
}
