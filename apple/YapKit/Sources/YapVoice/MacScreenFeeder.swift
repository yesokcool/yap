#if os(macOS)
import CoreMedia
import CoreVideo
import Foundation
import os
@preconcurrency import ScreenCaptureKit
@preconcurrency import WebRTC

/// The Mac's screen, through ScreenCaptureKit: the system's picker
/// (`SCContentSharingPicker`, a window, an app or a whole display) chooses
/// what's shared, and can change it later without stopping (same stream,
/// new content filter). Frames come as NV12 at the quality's size and frame
/// rate and go to WebRTC through a `FrameRelay` (hardware H.264 encodes them).
///
/// No sound: WebRTC's SDK sends the microphone on every audio track and
/// has no way to feed one from other audio, so the share's sound can't go
/// on its own track (PROTOCOL.md › Screen sharing) from this app yet.
@MainActor
public final class MacScreenFeeder: ScreenFeeder {
    public var onEnded: (() -> Void)?
    public var canChangeContent: Bool { stream != nil }

    private var relay: FrameRelay?
    private var stream: SCStream?
    private var filter: SCContentFilter?
    private var quality: ScreenQuality = .smooth
    private let output = CaptureOutput()
    private let observer = PickerObserver()
    private var pending: CheckedContinuation<SCContentFilter, any Error>?
    private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "screen")

    public init() {
        observer.feeder = self
        output.onStopped = { [weak self] error in
            MainActor.assumeIsolated { self?.streamStopped(error) }
        }
    }

    public func start(into source: RTCVideoSource, quality: ScreenQuality) async throws {
        self.quality = quality
        let picked = try await pick()
        let relay = FrameRelay(source: source)
        self.relay = relay
        output.relay = relay
        filter = picked
        let stream = SCStream(filter: picked, configuration: configuration(for: picked), delegate: output)
        try stream.addStreamOutput(output, type: .screen, sampleHandlerQueue: output.queue)
        do {
            try await stream.startCapture()
        } catch {
            relay.stop()
            self.relay = nil
            picker.isActive = false
            throw error
        }
        self.stream = stream
        log.info("sharing started")
    }

    public func setQuality(_ quality: ScreenQuality) {
        guard quality != self.quality else { return }
        self.quality = quality
        guard let stream, let filter else { return }
        Task { try? await stream.updateConfiguration(configuration(for: filter)) }
    }

    public func setDelivering(_ on: Bool) { relay?.setDelivering(on) }
    public func refresh() { relay?.refresh() }

    /// The system picker again, for this stream: another window or screen, same share.
    public func changeContent() {
        guard let stream else { return }
        picker.present(for: stream)
    }

    public func stop() {
        let stream = stream
        self.stream = nil
        filter = nil
        relay?.stop()
        relay = nil
        output.relay = nil
        picker.remove(observer)
        picker.isActive = false
        if let stream { Task { try? await stream.stopCapture() } }
        pending?.resume(throwing: CancellationError())
        pending = nil
    }

    // MARK: Picker

    private var picker: SCContentSharingPicker { .shared }

    private func pick() async throws -> SCContentFilter {
        var configuration = SCContentSharingPickerConfiguration()
        configuration.allowedPickerModes = [.singleWindow, .singleApplication, .singleDisplay]
        configuration.allowsChangingSelectedContent = true
        if let own = Bundle.main.bundleIdentifier { configuration.excludedBundleIDs = [own] }
        picker.defaultConfiguration = configuration
        picker.maximumStreamCount = 1
        picker.add(observer)
        picker.isActive = true
        return try await withCheckedThrowingContinuation { continuation in
            pending?.resume(throwing: CancellationError())
            pending = continuation
            picker.present()
        }
    }

    fileprivate func picked(_ filter: SCContentFilter, for stream: SCStream?) {
        if let stream, stream === self.stream {
            // Changing what's shared: same stream (and track), new content.
            self.filter = filter
            Task {
                try? await stream.updateContentFilter(filter)
                try? await stream.updateConfiguration(configuration(for: filter))
            }
            relay?.refresh()
            return
        }
        pending?.resume(returning: filter)
        pending = nil
    }

    fileprivate func cancelled(for stream: SCStream?) {
        guard stream == nil || stream !== self.stream else { return } // a change was cancelled: keep sharing
        pending?.resume(throwing: CancellationError())
        pending = nil
        if self.stream == nil { picker.isActive = false }
    }

    fileprivate func failed(_ error: any Error) {
        pending?.resume(throwing: error)
        pending = nil
    }

    private func streamStopped(_ error: (any Error)?) {
        guard stream != nil else { return }
        log.info("sharing ended by the system: \(error?.localizedDescription ?? "-", privacy: .public)")
        stop()
        onEnded?()
    }

    // MARK: Configuration

    /// The quality's size (scaled down to fit, never up) and frame rate, NV12, with the cursor.
    private func configuration(for filter: SCContentFilter) -> SCStreamConfiguration {
        let configuration = SCStreamConfiguration()
        let scale = CGFloat(filter.pointPixelScale)
        let size = quality.fit(width: Int(filter.contentRect.width * scale), height: Int(filter.contentRect.height * scale))
        configuration.width = size.width
        configuration.height = size.height
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(quality.fps))
        configuration.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
        configuration.colorSpaceName = CGColorSpace.sRGB
        configuration.showsCursor = true
        configuration.queueDepth = 6
        configuration.capturesAudio = false
        return configuration
    }
}

/// ScreenCaptureKit's callbacks (its own queues): frames to the relay, and
/// the stream stopping by itself (the menu bar's Stop, the window closed).
private final class CaptureOutput: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    let queue = DispatchQueue(label: "yap.screen.capture")
    private let lock = NSLock()
    private var _relay: FrameRelay?
    var relay: FrameRelay? {
        get { lock.withLock { _relay } }
        set { lock.withLock { _relay = newValue } }
    }
    /// Read on the main thread only.
    nonisolated(unsafe) var onStopped: ((any Error)?) -> Void = { _ in }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sampleBuffer.isValid, let buffer = sampleBuffer.imageBuffer else { return }
        // Only complete frames carry a new picture (idle ones repeat nothing).
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
           let raw = attachments.first?[.status] as? Int, let status = SCFrameStatus(rawValue: raw), status != .complete {
            return
        }
        relay?.push(buffer)
    }

    func stream(_ stream: SCStream, didStopWithError error: any Error) {
        let box = Unchecked(error)
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.onStopped(box.value) }
        }
    }
}

/// The system picker's answers, on any thread → the feeder on the main actor.
private final class PickerObserver: NSObject, SCContentSharingPickerObserver, @unchecked Sendable {
    /// Read and written on the main thread only.
    nonisolated(unsafe) weak var feeder: MacScreenFeeder?

    func contentSharingPicker(_ picker: SCContentSharingPicker, didUpdateWith filter: SCContentFilter, for stream: SCStream?) {
        let box = Unchecked((filter, stream))
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.feeder?.picked(box.value.0, for: box.value.1) }
        }
    }

    func contentSharingPicker(_ picker: SCContentSharingPicker, didCancelFor stream: SCStream?) {
        let box = Unchecked(stream)
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.feeder?.cancelled(for: box.value) }
        }
    }

    func contentSharingPickerStartDidFailWithError(_ error: any Error) {
        let box = Unchecked(error)
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.feeder?.failed(box.value) }
        }
    }
}
#endif
