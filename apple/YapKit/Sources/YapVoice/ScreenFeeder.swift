import CoreVideo
import Foundation
import os
@preconcurrency import WebRTC

/// Where a screen share's pictures come from: the Mac's screen through
/// ScreenCaptureKit (`MacScreenFeeder`), an iPhone's or iPad's through the
/// broadcast extension (`BroadcastScreenFeeder`), or a synthetic pattern
/// for tests (`SyntheticScreenFeeder`). Real capture never runs in tests.
@MainActor
public protocol ScreenFeeder: AnyObject {
    /// Ask what to share (where there's a picker) and start pushing frames
    /// into `source`. Throws `CancellationError` if the person cancels.
    func start(into source: RTCVideoSource, quality: ScreenQuality) async throws
    /// Smooth / Sharp / Max, without stopping.
    func setQuality(_ quality: ScreenQuality)
    /// Someone's watching: hand frames to the encoder. Nobody: encode nothing.
    func setDelivering(_ on: Bool)
    /// A new viewer: the current picture again right away (their encoder
    /// starts with a keyframe, so they see something within a moment).
    func refresh()
    func stop()
    /// Picking another window or screen without stopping is possible.
    var canChangeContent: Bool { get }
    func changeContent()
    /// The system ended the share (its own Stop button, the window closed).
    var onEnded: (() -> Void)? { get set }
}

/// Hands frames to WebRTC: the newest picture goes to the source while
/// someone watches, and is sent again every second while the screen holds
/// still (capture only delivers changes; a viewer who just joined, or one
/// whose "paused" timer is running, still gets a picture). Thread-safe:
/// capture callbacks push from their own queue.
final class FrameRelay: @unchecked Sendable {
    private let source: RTCVideoSource
    private let capturer: RTCVideoCapturer
    private let lock = NSLock()
    private var last: CVPixelBuffer?
    private var lastRotation: RTCVideoRotation = ._0
    private var lastSentAt: TimeInterval = 0
    private var delivering = false
    private var timer: DispatchSourceTimer?
    private let queue = DispatchQueue(label: "yap.screen.relay")
    /// A still screen is sent again this often.
    static let repeatEvery: TimeInterval = 1

    init(source: RTCVideoSource) {
        self.source = source
        capturer = RTCVideoCapturer(delegate: source)
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + 0.5, repeating: 0.5)
        timer.setEventHandler { [weak self] in self?.repeatIfStill() }
        timer.resume()
        self.timer = timer
    }

    deinit { timer?.cancel() }

    func stop() {
        timer?.cancel()
        timer = nil
        lock.withLock { last = nil }
    }

    func setDelivering(_ on: Bool) {
        lock.withLock { delivering = on }
        if on { refresh() }
    }

    /// A new picture from the capture (`rotation`: how far it must turn
    /// clockwise to be upright; WebRTC turns it, or tells the far side to).
    func push(_ buffer: CVPixelBuffer, rotation: RTCVideoRotation = ._0) {
        let send = lock.withLock { () -> Bool in
            last = buffer
            lastRotation = rotation
            return delivering
        }
        if send { deliver(buffer, rotation: rotation) }
    }

    /// The current picture again, now.
    func refresh() {
        guard let (buffer, rotation) = lock.withLock({ delivering ? last.map { ($0, lastRotation) } : nil }) else { return }
        deliver(buffer, rotation: rotation)
    }

    private func repeatIfStill() {
        let still = lock.withLock { () -> (CVPixelBuffer, RTCVideoRotation)? in
            guard delivering, let last, ProcessInfo.processInfo.systemUptime - lastSentAt >= Self.repeatEvery else { return nil }
            return (last, lastRotation)
        }
        if let (buffer, rotation) = still { deliver(buffer, rotation: rotation) }
    }

    private func deliver(_ buffer: CVPixelBuffer, rotation: RTCVideoRotation) {
        let now = ProcessInfo.processInfo.systemUptime
        lock.withLock {
            lastSentAt = now
            deliveredCount += 1
        }
        let frame = RTCVideoFrame(buffer: RTCCVPixelBuffer(pixelBuffer: buffer), rotation: rotation, timeStampNs: Int64(now * 1_000_000_000))
        source.capturer(capturer, didCapture: frame)
    }

    /// Pictures handed to WebRTC so far (tests).
    var delivered: Int { lock.withLock { deliveredCount } }
    private var deliveredCount = 0
}

/// A moving test card (bars sliding across a gradient) instead of a screen:
/// the interop test and DEBUG runs share it, so nothing real is captured.
@MainActor
public final class SyntheticScreenFeeder: ScreenFeeder {
    public let width: Int
    public let height: Int
    public let fps: Int
    private var relay: FrameRelay?
    private var timer: DispatchSourceTimer?
    public var onEnded: (() -> Void)?
    public var canChangeContent: Bool { false }
    /// Frames drawn so far (tests).
    public private(set) var framesMade = 0
    private let counter = OSAllocatedUnfairLock(initialState: 0)

    public init(width: Int = 640, height: Int = 360, fps: Int = 30) {
        self.width = width
        self.height = height
        self.fps = fps
    }

    public func start(into source: RTCVideoSource, quality: ScreenQuality) async throws {
        let relay = FrameRelay(source: source)
        self.relay = relay
        let pool = Self.pool(width: width, height: height)
        let (width, height) = (width, height)
        let counter = counter
        let timer = DispatchSource.makeTimerSource(queue: DispatchQueue(label: "yap.screen.synthetic"))
        timer.schedule(deadline: .now(), repeating: 1 / Double(max(1, min(fps, quality.fps))))
        timer.setEventHandler {
            let n = counter.withLock { value -> Int in
                value += 1
                return value
            }
            if let buffer = Self.draw(frame: n, width: width, height: height, pool: pool) { relay.push(buffer) }
        }
        timer.resume()
        self.timer = timer
    }

    public func setQuality(_ quality: ScreenQuality) {}
    public func setDelivering(_ on: Bool) { relay?.setDelivering(on) }
    public func refresh() { relay?.refresh() }
    public func changeContent() {}

    public func stop() {
        timer?.cancel()
        timer = nil
        relay?.stop()
        relay = nil
        framesMade = counter.withLock { $0 }
    }

    /// NV12, like the screen's own capture.
    nonisolated static func pool(width: Int, height: Int) -> CVPixelBufferPool? {
        let attributes: [String: Any] = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height,
            kCVPixelBufferIOSurfacePropertiesKey as String: [:] as [String: Any],
        ]
        var pool: CVPixelBufferPool?
        CVPixelBufferPoolCreate(nil, nil, attributes as CFDictionary, &pool)
        return pool
    }

    /// A gradient with a bright bar moving across it, and a color wash in the chroma.
    nonisolated static func draw(frame: Int, width: Int, height: Int, pool: CVPixelBufferPool?) -> CVPixelBuffer? {
        guard let pool else { return nil }
        var out: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out) == kCVReturnSuccess, let buffer = out else { return nil }
        CVPixelBufferLockBaseAddress(buffer, [])
        defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
        guard let y = CVPixelBufferGetBaseAddressOfPlane(buffer, 0)?.assumingMemoryBound(to: UInt8.self),
              let uv = CVPixelBufferGetBaseAddressOfPlane(buffer, 1)?.assumingMemoryBound(to: UInt8.self) else { return nil }
        let yStride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0)
        let uvStride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 1)
        let bar = (frame * 8) % width
        for row in 0..<height {
            let line = y + row * yStride
            for col in 0..<width {
                let inBar = col >= bar && col < bar + width / 10
                line[col] = inBar ? 235 : UInt8(16 + (col + row) * 200 / (width + height))
            }
        }
        for row in 0..<height / 2 {
            let line = uv + row * uvStride
            for col in 0..<width / 2 {
                line[col * 2] = UInt8((col * 255) / max(1, width / 2))
                line[col * 2 + 1] = UInt8((row * 255) / max(1, height / 2))
            }
        }
        return buffer
    }
}
