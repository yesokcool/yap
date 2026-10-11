import CoreVideo
import Foundation
import Testing
@testable import YapVoice
@preconcurrency import WebRTC

/// Screen sharing's rules (PROTOCOL.md › Screen sharing), without a network:
/// what a viewer sees over time, who's watching, the presets, H.264 first.
@Suite("Screen sharing")
struct ScreenShareTests {
    // MARK: Watching: connecting → playing → paused → stopped → closed

    @Test func watchingGoesThroughItsStatesOnTime() {
        var watch = ScreenWatch(sid: "s1", uid: "ana")
        #expect(watch.phase == .connecting)
        #expect(watch.status(name: "Ana") == "Connecting to Ana's screen…")
        // No frame yet: it keeps saying connecting, however long it takes.
        #expect(watch.tick(at: 10) == .connecting)

        watch.frame(at: 10.2)
        #expect(watch.phase == .playing && watch.status(name: "Ana") == nil)
        #expect(watch.tick(at: 13.1) == .playing, "2.9 s since the last frame: still playing")
        #expect(watch.tick(at: 13.2) == .paused, "3 s without a frame")
        #expect(watch.status(name: "Ana") == "Ana's screen is paused.")

        watch.frame(at: 14)
        #expect(watch.phase == .playing, "a frame brings it back")

        watch.sharerStopped(at: 20)
        #expect(watch.phase == .stopped && watch.status(name: "Ana") == "Ana stopped sharing.")
        watch.frame(at: 20.5)
        #expect(watch.phase == .stopped, "a late frame doesn't undo the end")
        #expect(watch.tick(at: 21.9) == .stopped)
        #expect(watch.tick(at: 22) == .closed, "closes itself 2 s after they stop")
        #expect(watch.status(name: "Ana") == "Ana stopped sharing.")
    }

    @Test func stoppingBeforeTheFirstFrameStillCloses() {
        var watch = ScreenWatch(sid: "s1", uid: "ana")
        watch.sharerStopped(at: 5)
        watch.sharerStopped(at: 6) // said twice: the 2 s count from the first
        #expect(watch.tick(at: 7) == .closed)
    }

    @Test func polledCountersOnlyMoveForward() {
        var watch = ScreenWatch(sid: "s1", uid: "ana")
        watch.frames(0, lastAt: 1)
        #expect(watch.phase == .connecting, "no frames counted yet")
        watch.frames(12, lastAt: 2)
        #expect(watch.phase == .playing && watch.frames == 12 && watch.lastFrameAt == 2)
        watch.frames(12, lastAt: 9) // the same count again: nothing new came
        #expect(watch.lastFrameAt == 2)
        #expect(watch.tick(at: 5) == .paused)
    }

    // MARK: Who's watching

    @Test func watchingList() {
        #expect(ScreenAudience.text(names: []) == "No one's watching yet")
        #expect(ScreenAudience.text(names: ["Ana"]) == "Watching: Ana")
        #expect(ScreenAudience.text(names: ["Ben", "ana", "Ben"]) == "Watching: ana, Ben", "A–Z, each person once (two devices)")
    }

    // MARK: Presets: the web's SCREEN_QUALITY

    @Test func presetsMatchTheWeb() {
        #expect(ScreenQuality.allCases.map(\.title) == ["Smooth", "Sharp", "Max"])
        #expect(ScreenQuality.smooth.maxWidth == 1920 && ScreenQuality.smooth.maxHeight == 1080 && ScreenQuality.smooth.fps == 60 && ScreenQuality.smooth.bitrate == 8_000_000)
        #expect(ScreenQuality.sharp.maxWidth == 2560 && ScreenQuality.sharp.maxHeight == 1440 && ScreenQuality.sharp.fps == 30 && ScreenQuality.sharp.bitrate == 6_000_000)
        #expect(ScreenQuality.max.maxWidth == 3840 && ScreenQuality.max.maxHeight == 2160 && ScreenQuality.max.fps == 60 && ScreenQuality.max.bitrate == 25_000_000)
        #expect(ScreenQuality.smooth.degradation == .maintainFramerate)
        #expect(ScreenQuality.sharp.degradation == .maintainResolution)
        #expect(ScreenQuality.max.degradation == .balanced)
    }

    @Test func aShareIsScaledDownToFitNeverUp() {
        // A 5K display at Smooth: 1920 wide, same shape.
        #expect(ScreenQuality.smooth.fit(width: 5120, height: 2880) == (1920, 1080))
        // A tall window keeps its shape and fits the height.
        #expect(ScreenQuality.smooth.fit(width: 1000, height: 2000) == (540, 1080))
        // Small stays small (even sizes for the encoder).
        #expect(ScreenQuality.max.fit(width: 801, height: 601) == (800, 600))
    }

    // MARK: H.264 first

    @Test func h264ConstrainedBaselineGoesFirst() {
        typealias C = ScreenCodecs.Codec
        let offered = [
            C(name: "VP8"),
            C(name: "H264", parameters: ["profile-level-id": "640c1f", "packetization-mode": "1"]),
            C(name: "VP9", parameters: ["profile-id": "0"]),
            C(name: "H264", parameters: ["profile-level-id": "42e01f", "packetization-mode": "0"]),
            C(name: "H264", parameters: ["profile-level-id": "42e01f", "packetization-mode": "1", "level-asymmetry-allowed": "1"]),
            C(name: "rtx", parameters: ["apt": "96"]),
        ]
        let ordered = ScreenCodecs.ordered(offered) { $0 }
        #expect(ordered[0] == offered[4], "constrained baseline, packetization-mode=1")
        #expect(ordered[1] == offered[1], "then other H.264 in mode 1")
        #expect(ordered[2] == offered[3], "then H.264 in mode 0")
        #expect(ordered.dropFirst(3).map(\.name) == ["VP8", "VP9", "rtx"], "the rest keep their order")
        #expect(ScreenCodecs.isConstrainedBaseline("42e01f") && ScreenCodecs.isConstrainedBaseline("42C01F"))
        #expect(!ScreenCodecs.isConstrainedBaseline("42001f"), "plain baseline isn't constrained")
        #expect(!ScreenCodecs.isConstrainedBaseline("640c1f") && !ScreenCodecs.isConstrainedBaseline(nil))
    }

    // MARK: The renderer counts what it shows

    @MainActor
    @Test func theRendererCountsFramesAndKnowsTheirSize() throws {
        let renderer = ScreenRenderer()
        #expect(renderer.counters.frames == 0 && renderer.counters.lastFrameAt == nil)
        let pool = SyntheticScreenFeeder.pool(width: 320, height: 180)
        for n in 1...3 {
            let buffer = try #require(SyntheticScreenFeeder.draw(frame: n, width: 320, height: 180, pool: pool))
            renderer.renderFrame(RTCVideoFrame(buffer: RTCCVPixelBuffer(pixelBuffer: buffer), rotation: ._0, timeStampNs: Int64(n) * 33_000_000))
        }
        // A software decoder's I420 frame is converted, not dropped.
        let i420 = RTCI420Buffer(width: 64, height: 36)
        renderer.renderFrame(RTCVideoFrame(buffer: i420, rotation: ._0, timeStampNs: 0))
        #expect(renderer.counters.frames == 4 && renderer.counters.lastFrameAt != nil)
        #expect(renderer.videoSize == CGSize(width: 64, height: 36))
        #expect(renderer.displayLayer.videoGravity == .resizeAspect, "letterboxed, never cropped or stretched")
        renderer.clear()
        #expect(renderer.videoSize == .zero)
    }

    // MARK: Settings

    @Test func shareSettingsAreRememberedAndOptional() throws {
        var settings = VoiceSettings()
        #expect(settings.shareQuality == .smooth && settings.screenVolumes.isEmpty)
        settings.shareQuality = .sharp
        settings.screenVolumes["ana"] = 0.4
        settings.volumes["ana"] = 1.5
        let back = try JSONDecoder().decode(VoiceSettings.self, from: JSONEncoder().encode(settings))
        #expect(back.shareQuality == .sharp && back.screenVolumes["ana"] == 0.4 && back.volumes["ana"] == 1.5, "separate from their voice")
        // Settings saved by an older version have neither.
        let old = try JSONDecoder().decode(VoiceSettings.self, from: Data(#"{"mode":"voiceActivity","bitrate":128000,"volumes":{"ana":0.5}}"#.utf8))
        #expect(old.shareQuality == .smooth && old.screenVolumes.isEmpty && old.volumes["ana"] == 0.5)
    }
}
