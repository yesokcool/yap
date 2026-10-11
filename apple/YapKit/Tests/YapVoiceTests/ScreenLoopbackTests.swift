#if os(iOS)
import Foundation
import Testing
@testable import YapCore
@testable import YapVoice
@preconcurrency import WebRTC

/// Screen sharing over a real WebRTC connection inside one process: two
/// engines (synthetic mics, a synthetic screen, no server, nothing
/// captured) wired signal to signal. A viewer asks, the sharer's video
/// arrives as H.264 and decodes; stopping stops the encoding for that
/// viewer; asking again reuses the same transceiver.
@MainActor
@Suite("Screen sharing between two engines", .serialized)
final class ScreenLoopbackTests {
    let sharer = VoiceEngine(audioDevice: SyntheticAudioDevice())
    let viewer = VoiceEngine(audioDevice: SyntheticAudioDevice())

    init() {
        sharer.mySid = "s-sharer"
        viewer.mySid = "s-viewer"
        sharer.uidOf = { _ in "viewer" }
        viewer.uidOf = { _ in "sharer" }
        // Signals in order, like the socket (one main-queue hop each).
        sharer.onSignal = { [viewer] _, data in
            DispatchQueue.main.async { MainActor.assumeIsolated { viewer.handleSignal(data, from: "s-sharer") } }
        }
        viewer.onSignal = { [sharer] _, data in
            DispatchQueue.main.async { MainActor.assumeIsolated { sharer.handleSignal(data, from: "s-viewer") } }
        }
    }

    private func waitUntil(_ what: String, timeout: Duration = .seconds(15), _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            if ContinuousClock.now > deadline {
                Issue.record("timed out waiting for: \(what)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(50))
        }
    }

    @Test func shareWatchStopAndWatchAgain() async throws {
        defer {
            sharer.stop()
            viewer.stop()
        }
        sharer.prepare(iceServers: [])
        viewer.prepare(iceServers: [])
        viewer.enter(["s-sharer"]) // the newcomer offers
        try await waitUntil("connected") { sharer.connectionState(of: "s-viewer") == .connected && viewer.connectionState(of: "s-sharer") == .connected }

        // Sharing, nobody watching: nothing goes out.
        let feeder = SyntheticScreenFeeder(width: 640, height: 360, fps: 30)
        try await feeder.start(into: sharer.makeScreenSource(), quality: .smooth)
        defer { feeder.stop() }
        sharer.onScreenViewers = { [sharer] joined in
            feeder.setDelivering(!sharer.screenViewers.isEmpty)
            if joined { feeder.refresh() }
        }
        sharer.startScreen(quality: .smooth)
        #expect(sharer.screenQuality == .smooth && sharer.screenViewers.isEmpty)
        #expect(!sharer.sendsScreen(to: "s-viewer"), "no viewer, no video")

        // The viewer asks: video arrives, H.264, and decodes.
        let asked = ContinuousClock.now
        viewer.watchScreen("s-sharer", true)
        try await waitUntil("the sharer sees its viewer") { sharer.screenViewers == ["s-viewer"] }
        #expect(sharer.sendsScreen(to: "s-viewer"))
        try await waitUntil("the video track arrives") { viewer.remoteScreens["s-sharer"] != nil }
        let renderer = ScreenRenderer()
        viewer.remoteScreens["s-sharer"]?.add(renderer)
        try await waitUntil("decoded frames") { renderer.counters.frames >= 30 }
        let firstPicture = ContinuousClock.now - asked
        let seen = try #require(await viewer.peerStats().first)
        let sent = try #require(await sharer.peerStats().first)
        print("SCREEN loopback: first frames after \(firstPicture); viewer decoded \(seen.videoFramesDecoded) frames \(seen.videoFrameWidth)x\(seen.videoFrameHeight) \(seen.inboundVideoCodec ?? "?") [\(seen.inboundVideoFmtp ?? "")]; sharer encoded \(sent.videoFramesEncoded) with \(sent.videoEncoder ?? "?") \(sent.outboundVideoCodec ?? "?")")
        #expect(seen.inboundVideoCodec == "video/H264", "H.264 is the screen codec")
        #expect(seen.inboundVideoFmtp?.contains("packetization-mode=1") == true)
        #expect(seen.inboundVideoFmtp?.contains("profile-level-id=42e01f") == true, "constrained baseline")
        // Smooth keeps the frame rate: while the bandwidth estimate ramps up
        // WebRTC may send it smaller at first, but never cropped or stretched.
        let size = renderer.videoSize
        #expect(size.width > 0 && size.width <= 640 && abs(size.width * 9 - size.height * 16) < 32, "the share's shape: \(size)")
        #expect(firstPicture < .seconds(5), "a picture soon after asking: \(firstPicture)")

        // Quality changes on the fly (no renegotiation, same track).
        sharer.setScreenQuality(.sharp)
        #expect(sharer.screenQuality == .sharp && sharer.sendsScreen(to: "s-viewer"))

        // Stop watching: the sharer drops them and stops encoding for them.
        viewer.watchScreen("s-sharer", false)
        try await waitUntil("the sharer lets them go") { sharer.screenViewers.isEmpty && !sharer.sendsScreen(to: "s-viewer") }
        try await waitUntil("the viewer's track goes") { viewer.remoteScreens["s-sharer"] == nil }
        try await Task.sleep(for: .milliseconds(800))
        let after = renderer.counters.frames
        try await Task.sleep(for: .seconds(1.5))
        #expect(renderer.counters.frames == after, "nothing more comes after Stop watching")

        // Ask again: the same transceiver carries it (no new m-line each time).
        let sections = sharer.peerStatsSections()
        viewer.watchScreen("s-sharer", true)
        try await waitUntil("video again") { viewer.remoteScreens["s-sharer"] != nil }
        let again = ScreenRenderer()
        viewer.remoteScreens["s-sharer"]?.add(again)
        try await waitUntil("frames again") { again.counters.frames >= 10 }
        #expect(sharer.peerStatsSections() == sections, "the transceiver is reused")

        // The sharer stops: the video goes for the viewer too.
        sharer.stopScreen()
        try await waitUntil("share ended") { viewer.remoteScreens["s-sharer"] == nil }
        #expect(sharer.screenViewers.isEmpty)
    }
}

extension VoiceEngine {
    /// How many m-lines the first connection has (tests).
    func peerStatsSections() -> Int {
        guard let sid = peerSids.first, let sdp = peerLocalDescription(sid) else { return 0 }
        return sdp.components(separatedBy: "\nm=").count - 1
    }
}
#endif
