import CoreVideo
import Foundation
import os
import Testing
@testable import YapScreenLink
@testable import YapVoice
@preconcurrency import WebRTC

/// iPhone and iPad sharing, the app's half: what the broadcast extension
/// sends goes into the screen source like the Mac's capture, and the app
/// tells the extension the quality and when nobody's watching. (ReplayKit
/// itself can't run in a test: a stand-in client plays the extension.)
@MainActor
@Suite("Sharing from iPhone and iPad: the app's side", .serialized)
final class BroadcastFeederTests {
    /// A Unix socket's path fits in 103 bytes; the simulator's temporary folder is longer.
    let path: String = {
        let name = "yb-\(UUID().uuidString.prefix(8))"
        let temporary = (NSTemporaryDirectory() as NSString).appendingPathComponent(name)
        return temporary.utf8.count <= 100 ? temporary : "/tmp/" + name
    }()

    deinit { unlink(path) }

    private func waitUntil(_ what: String, timeout: Duration = .seconds(5), _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            if ContinuousClock.now > deadline {
                Issue.record("timed out waiting for: \(what)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(10))
        }
    }

    @Test func theExtensionIsToldTheQualityAndWhenSomeoneWatches() {
        #expect(BroadcastScreenFeeder.settings(.smooth, delivering: true) == .init(maxLong: 1920, maxShort: 1080, fps: 60))
        #expect(BroadcastScreenFeeder.settings(.sharp, delivering: true) == .init(maxLong: 2560, maxShort: 1440, fps: 30))
        #expect(BroadcastScreenFeeder.settings(.max, delivering: true) == .init(maxLong: 3840, maxShort: 2160, fps: 60))
        #expect(BroadcastScreenFeeder.settings(.smooth, delivering: false).fps == 1, "one picture a second while nobody watches")
        #expect(BroadcastScreenFeeder.rotation(0) == ._0 && BroadcastScreenFeeder.rotation(90) == ._90)
        #expect(BroadcastScreenFeeder.rotation(180) == ._180 && BroadcastScreenFeeder.rotation(270) == ._270)
    }

    private func picture(width: Int = 640, height: Int = 360) -> CVPixelBuffer {
        let pool = ScreenLinkPixels.pool(width: width, height: height, format: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange)!
        var out: CVPixelBuffer?
        CVPixelBufferPoolCreatePixelBuffer(nil, pool, &out)
        return out!
    }

    @Test func picturesReachTheEncoderAndEitherSideCanStop() async throws {
        let engine = VoiceEngine()
        let source = engine.makeScreenSource()
        let server = ScreenLinkServer(path: path)
        let arrived = OSAllocatedUnfairLock<ScreenLinkConnection?>(initialState: nil)
        server.onConnection = { connection in arrived.withLock { $0 = connection } }
        try server.start()
        defer { server.stop() }

        // The extension connects and listens for settings.
        let settings = OSAllocatedUnfairLock<[ScreenLink.Settings]>(initialState: [])
        let extensionClosed = OSAllocatedUnfairLock(initialState: false)
        let client = try ScreenLinkClient.connect(path: path)
        client.onSettings = { s in settings.withLock { $0.append(s) } }
        client.onClose = { extensionClosed.withLock { $0 = true } }
        client.start()
        try await waitUntil("the app accepts") { arrived.withLock { $0 } != nil }
        let feeder = BroadcastScreenFeeder(connection: try #require(arrived.withLock { $0 }))
        var ended = 0
        feeder.onEnded = { ended += 1 }
        try await feeder.start(into: source, quality: .smooth)
        #expect(!feeder.canChangeContent, "ReplayKit shares the whole screen")

        // Nobody watching: the extension sends one a second, nothing is encoded.
        try await waitUntil("first settings") { settings.withLock { $0.last } == BroadcastScreenFeeder.settings(.smooth, delivering: false) }
        #expect(client.send(picture(), rotation: 0, timestampNs: 1))
        try await Task.sleep(for: .milliseconds(200))
        #expect(feeder.delivered == 0, "nothing goes to the encoder while nobody watches")

        // Someone watches: full rate, and the latest picture right away.
        feeder.setDelivering(true)
        try await waitUntil("watching settings") { settings.withLock { $0.last } == BroadcastScreenFeeder.settings(.smooth, delivering: true) }
        try await waitUntil("the held picture goes at once") { feeder.delivered >= 1 }
        try await waitUntil("free") { !client.isBusy }
        let before = feeder.delivered
        #expect(client.send(picture(), rotation: 90, timestampNs: 2))
        try await waitUntil("a new picture") { feeder.delivered > before }

        // Sharp, without stopping.
        feeder.setQuality(.sharp)
        try await waitUntil("sharp settings") { settings.withLock { $0.last } == BroadcastScreenFeeder.settings(.sharp, delivering: true) }

        // Stopped from Control Center: the extension hangs up, the share ends in Yap.
        client.close()
        try await waitUntil("the share ends") { ended == 1 }
        feeder.stop()
        #expect(ended == 1)
    }

    @Test func stopSharingInYapEndsTheBroadcast() async throws {
        let server = ScreenLinkServer(path: path)
        let arrived = OSAllocatedUnfairLock<ScreenLinkConnection?>(initialState: nil)
        server.onConnection = { connection in arrived.withLock { $0 = connection } }
        try server.start()
        defer { server.stop() }
        let extensionClosed = OSAllocatedUnfairLock(initialState: false)
        let client = try ScreenLinkClient.connect(path: path)
        client.onClose = { extensionClosed.withLock { $0 = true } }
        client.start()
        try await waitUntil("the app accepts") { arrived.withLock { $0 } != nil }
        let feeder = BroadcastScreenFeeder(connection: try #require(arrived.withLock { $0 }))
        var ended = 0
        feeder.onEnded = { ended += 1 }
        try await feeder.start(into: VoiceEngine().makeScreenSource(), quality: .smooth)
        feeder.stop()
        try await waitUntil("the extension hears it (and ends the broadcast)") { extensionClosed.withLock { $0 } }
        try await Task.sleep(for: .milliseconds(100))
        #expect(ended == 0, "stopping in Yap isn't the system's stop")
    }
}
