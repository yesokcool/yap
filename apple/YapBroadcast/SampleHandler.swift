import CoreMedia
import Foundation
import os
import ReplayKit
import YapScreenLink

/// Yap's screen broadcast on iPhone and iPad. iOS runs this extension while
/// the red pill shows and hands it the screen; it scales each picture to
/// the share's quality (the app says which, and how often: the quality's
/// frame rate while someone watches, one a second while nobody does) and
/// passes it to the Yap app over the App Group socket (`YapScreenLink`).
/// The app encodes and sends it, exactly like the Mac's share.
///
/// Picture only: no sound leaves here (the call's own sound must never ride
/// on a share). It lives in 50 MB: one picture in flight, the rest skipped,
/// never queued; no WebRTC in here.
final class SampleHandler: RPBroadcastSampleHandler, @unchecked Sendable {
    private let lock = OSAllocatedUnfairLock(initialState: State())
    private let scaler = FrameScaler()
    private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap.broadcast", category: "broadcast")

    private struct State {
        var link: ScreenLinkClient?
        var settings = ScreenLink.Settings.initial
        var lastSent: CMTime = .invalid
    }

    /// Why the broadcast stopped, for iOS's "Screen Broadcasting has stopped" alert.
    private enum Stop {
        case notInCall, stoppedInYap, noApp

        var message: String {
            switch self {
            case .notInCall: "Join a voice channel in Yap first, then share your screen from the call."
            case .stoppedInYap: "Yap stopped sharing your screen."
            case .noApp: "Yap couldn't be reached. Open Yap and try again."
            }
        }
    }

    override func broadcastStarted(withSetupInfo setupInfo: [String: NSObject]?) {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "YapAppGroup") as? String,
              let path = ScreenLink.socketPath(appGroup: group) else {
            end(.noApp)
            return
        }
        let link: ScreenLinkClient
        do {
            link = try ScreenLinkClient.connect(path: path)
        } catch {
            // Yap only listens while it's in a call.
            log.info("Yap isn't listening: \(String(describing: error), privacy: .public)")
            end(.notInCall)
            return
        }
        link.onSettings = { [weak self] settings in self?.lock.withLock { $0.settings = settings } }
        link.onClose = { [weak self] in self?.end(.stoppedInYap) }
        lock.withLock { $0.link = link }
        link.start()
    }

    override func processSampleBuffer(_ sampleBuffer: CMSampleBuffer, with sampleBufferType: RPSampleBufferType) {
        // The screen only: app audio and the mic are never sent.
        guard sampleBufferType == .video, let image = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
        let time = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        let ready = lock.withLock { state -> (ScreenLinkClient, ScreenLink.Settings)? in
            guard let link = state.link, !link.isBusy else { return nil }
            // No faster than the app asked for.
            if state.lastSent.isValid, CMTimeGetSeconds(CMTimeSubtract(time, state.lastSent)) < 0.9 / Double(max(1, state.settings.fps)) { return nil }
            return (link, state.settings)
        }
        guard let (link, settings) = ready, let picture = scaler.fit(image, to: settings) else { return }
        let orientation = (CMGetAttachment(sampleBuffer, key: RPVideoSampleOrientationKey as CFString, attachmentModeOut: nil) as? NSNumber)?.uint32Value
        let seconds = CMTimeGetSeconds(time)
        let timestamp = seconds.isFinite && seconds > 0 ? UInt64(seconds * 1_000_000_000) : 0
        if link.send(picture, rotation: ScreenLink.rotation(forOrientation: orientation), timestampNs: timestamp) {
            lock.withLock { $0.lastSent = time }
        }
    }

    override func broadcastFinished() {
        // Stopped from Control Center or the red pill: Yap hears the link close.
        let link = lock.withLock { state -> ScreenLinkClient? in
            defer { state.link = nil }
            return state.link
        }
        link?.onClose = nil
        link?.close()
    }

    private func end(_ stop: Stop) {
        let link = lock.withLock { state -> ScreenLinkClient? in
            defer { state.link = nil }
            return state.link
        }
        link?.onClose = nil
        link?.close()
        finishBroadcastWithError(NSError(domain: "Yap", code: 1, userInfo: [NSLocalizedDescriptionKey: stop.message]))
    }
}
