import CoreVideo
import Foundation
import os
import YapScreenLink
@preconcurrency import WebRTC

/// iPhone and iPad sharing: the screen comes from Yap's ReplayKit broadcast
/// extension (iOS asks what to share and shows the red pill), over the
/// App Group socket (`YapScreenLink`), into the same video source, relay and
/// encoder as the Mac's capture: "Watching: …", Smooth / Sharp / Max, a
/// still picture sent again every second, nothing encoded while nobody
/// watches. The extension scales each picture to the quality before
/// sending it, and sends one a second while nobody watches (so a new
/// viewer gets one at once). Picture only: the call's own sound must never
/// ride on a share, and the app's audio can't go on its own track here.
@MainActor
public final class BroadcastScreenFeeder: ScreenFeeder {
    private let connection: ScreenLinkConnection
    private var relay: FrameRelay?
    private var quality: ScreenQuality = .smooth
    private var delivering = false
    public var onEnded: (() -> Void)?
    /// ReplayKit shares the whole screen: nothing else to pick.
    public var canChangeContent: Bool { false }
    private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "screen")

    public init(connection: ScreenLinkConnection) {
        self.connection = connection
    }

    /// The socket the extension looks for, in the App Group's container
    /// (nil when this build has no App Group).
    public nonisolated static func socketPath(appGroup: String) -> String? {
        ScreenLink.socketPath(appGroup: appGroup)
    }

    /// Settings for the extension: the quality's size, and its frame rate
    /// while someone watches (one a second while nobody does).
    nonisolated static func settings(_ quality: ScreenQuality, delivering: Bool) -> ScreenLink.Settings {
        ScreenLink.Settings(maxLong: max(quality.maxWidth, quality.maxHeight), maxShort: min(quality.maxWidth, quality.maxHeight),
                            fps: delivering ? quality.fps : 1)
    }

    /// ReplayKit's turn (clockwise degrees) as WebRTC's.
    nonisolated static func rotation(_ degrees: Int) -> RTCVideoRotation {
        switch degrees {
        case 90: ._90
        case 180: ._180
        case 270: ._270
        default: ._0
        }
    }

    public func start(into source: RTCVideoSource, quality: ScreenQuality) async throws {
        let relay = FrameRelay(source: source)
        self.relay = relay
        self.quality = quality
        connection.onFrame = { buffer, rotation, _ in
            relay.push(buffer, rotation: Self.rotation(rotation))
        }
        connection.onClose = { [weak self] in
            // Stopped from Control Center or the red pill (or the extension went away).
            Task { @MainActor in self?.ended() }
        }
        connection.start()
        sendSettings()
        log.info("sharing from the broadcast extension")
    }

    public func setQuality(_ quality: ScreenQuality) {
        self.quality = quality
        sendSettings()
    }

    public func setDelivering(_ on: Bool) {
        delivering = on
        relay?.setDelivering(on)
        sendSettings()
    }

    public func refresh() { relay?.refresh() }

    public func changeContent() {}

    /// Stop sharing in Yap: the extension hears the link close and ends the broadcast.
    public func stop() {
        connection.onClose = nil
        connection.close()
        relay?.stop()
        relay = nil
    }

    /// Pictures that reached WebRTC (tests).
    var delivered: Int { relay?.delivered ?? 0 }

    private func sendSettings() {
        connection.send(Self.settings(quality, delivering: delivering))
    }

    private func ended() {
        guard relay != nil else { return }
        relay?.stop()
        relay = nil
        onEnded?()
    }
}
