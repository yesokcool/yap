#if os(iOS)
import Foundation
import Testing
@testable import YapCore
@testable import YapVoice
@preconcurrency import WebRTC

/// Round 10 on a real engine with a synthetic mic (iOS simulator; nothing
/// leaves the process): the level while muted (the "You're muted" nudge
/// needs it), push to talk's click and double-tap, "Too loud (clipping)",
/// and what the far side reports receiving from us (`remote-inbound-rtp`,
/// for "Your connection is dropping some of your audio").
@MainActor
@Suite("Talk cues on a real engine", .serialized)
final class TalkCuesLoopbackTests {
    private func waitUntil(_ what: String, timeout: Duration = .seconds(15), _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            if ContinuousClock.now > deadline {
                Issue.record("timed out waiting for: \(what)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(25))
        }
    }

    /// Alone in a channel (the loopback monitor measures the mic).
    private func alone(_ device: SyntheticAudioDevice, mode: VoiceEngine.InputMode) async throws -> VoiceEngine {
        let engine = VoiceEngine(audioDevice: device)
        engine.mySid = "s-me"
        engine.mode = mode
        engine.prepare(iceServers: [])
        engine.enter([])
        try await waitUntil("the mic's level") { engine.localLevel > 0.01 }
        return engine
    }

    /// Muting switches the track off (`isEnabled = false`), but WebRTC
    /// measures the level before that: the same counters voice activity
    /// reopens on. So talking while muted is seen, in every mode.
    @Test func talkingWhileMutedIsSeen() async throws {
        let device = SyntheticAudioDevice()
        let engine = try await alone(device, mode: .voiceActivity)
        defer { engine.stop() }
        var nudges = 0
        engine.onTalkWhileMuted = { nudges += 1 }
        try await Task.sleep(for: .milliseconds(500))
        #expect(nudges == 0, "not muted: nothing")

        engine.setMuted(true)
        try await Task.sleep(for: .seconds(1))
        let mutedDb = VoiceGate.dB(engine.localLevel)
        print("TALK muted: level \(Int(mutedDb)) dBFS, transmitting \(engine.transmitting), talking-while-muted readings \(nudges)")
        #expect(!engine.transmitting)
        #expect(mutedDb > -40, "the level before the mute still reads: \(mutedDb)")
        #expect(nudges > 10, "every reading while it lasts (the controller keeps one per 5 s)")

        // Deafened: you went away on purpose, no nudge (as in Mumble).
        engine.setDeafened(true)
        var before = nudges
        try await Task.sleep(for: .milliseconds(500))
        #expect(nudges == before, "deafened")
        engine.setDeafened(false)

        // Quiet: once the hold is over, no nudge.
        device.silent.withLock { $0 = true }
        try await Task.sleep(for: .seconds(1))
        before = nudges
        try await Task.sleep(for: .milliseconds(500))
        #expect(nudges == before, "silence isn't talking")

        // Always on: the same level decides.
        engine.mode = .openMic
        device.silent.withLock { $0 = false }
        try await waitUntil("talking on always on") { nudges > before }
        engine.setMuted(false)
        before = nudges
        try await Task.sleep(for: .milliseconds(300))
        #expect(nudges == before, "unmuted: nothing")
    }

    /// The click when push to talk opens and when it closes (after its
    /// 200 ms tail); a double-tap stays live until the next press.
    @Test func pushToTalkClicksAndDoubleTap() async throws {
        let engine = try await alone(SyntheticAudioDevice(), mode: .pushToTalk)
        defer { engine.stop() }
        var clicks: [Bool] = []
        engine.onPushToTalk = { clicks.append($0) }
        #expect(!engine.transmitting && clicks.isEmpty)

        engine.setPushToTalk(true)
        #expect(engine.transmitting && clicks == [true], "live at once, with a click")
        engine.setPushToTalk(false)
        #expect(engine.transmitting, "the tail")
        try await waitUntil("closed after the tail") { !engine.transmitting }
        #expect(clicks == [true, false])

        // Double-tap: live until the next press (well after the first press).
        try await Task.sleep(for: .milliseconds(450))
        engine.setPushToTalk(true)
        engine.setPushToTalk(false)
        try await Task.sleep(for: .milliseconds(120))
        engine.setPushToTalk(true)
        engine.setPushToTalk(false)
        #expect(engine.pushToTalkLatched)
        try await Task.sleep(for: .milliseconds(600))
        #expect(engine.transmitting && engine.pushToTalkDown, "still live after letting go")
        #expect(clicks == [true, false, true], "one click: it never closed in between")
        engine.setPushToTalk(true)
        #expect(!engine.pushToTalkLatched && engine.transmitting)
        engine.setPushToTalk(false)
        try await waitUntil("tap to stop") { !engine.transmitting }
        #expect(clicks == [true, false, true, false])

        // Muted: a press is "talking while muted", and no click (nothing opens).
        var nudges = 0
        engine.onTalkWhileMuted = { nudges += 1 }
        engine.setMuted(true)
        try await Task.sleep(for: .milliseconds(450)) // past the last press's double-tap window
        engine.setPushToTalk(true)
        #expect(nudges >= 1 && !engine.transmitting && clicks.count == 4)
        engine.setPushToTalk(false)
        try await Task.sleep(for: .milliseconds(300)) // past the tail
        engine.setMuted(false)
        #expect(clicks.count == 4)

        // Switching to another mode while latched: closes without a click.
        try await Task.sleep(for: .milliseconds(450))
        engine.setPushToTalk(true)
        engine.setPushToTalk(false)
        engine.setPushToTalk(true)
        engine.setPushToTalk(false)
        #expect(engine.pushToTalkLatched && clicks.count == 5)
        engine.mode = .voiceActivity
        engine.resetPushToTalk()
        #expect(!engine.pushToTalkLatched && !engine.pushToTalkDown && clicks.count == 5)
    }

    /// Peaks at full scale again and again: "Too loud (clipping)"; gone ~2 s after.
    @Test func clippedPeaksAreTooLoud() async throws {
        let device = SyntheticAudioDevice()
        let engine = try await alone(device, mode: .openMic)
        defer { engine.stop() }
        try await Task.sleep(for: .seconds(1.5))
        #expect(!engine.tooLoud, "the test chord peaks well under −1 dBFS")
        device.loud.withLock { $0 = true }
        try await waitUntil("too loud", timeout: .seconds(5)) { engine.tooLoud }
        device.loud.withLock { $0 = false }
        try await waitUntil("calm again", timeout: .seconds(6)) { !engine.tooLoud }
    }

    /// Two engines: each reads the other's RTCP report about its voice
    /// (fraction lost, packets lost, round trip). Nothing lost in-process, so
    /// "Your connection is dropping some of your audio" stays away.
    @Test func whatTheyReceiveFromUs() async throws {
        let a = VoiceEngine(audioDevice: SyntheticAudioDevice())
        let b = VoiceEngine(audioDevice: SyntheticAudioDevice())
        defer {
            a.stop()
            b.stop()
        }
        for (engine, sid) in [(a, "s-a"), (b, "s-b")] {
            engine.mySid = sid
            engine.mode = .openMic
        }
        a.onSignal = { [b] _, data in DispatchQueue.main.async { MainActor.assumeIsolated { b.handleSignal(data, from: "s-a") } } }
        b.onSignal = { [a] _, data in DispatchQueue.main.async { MainActor.assumeIsolated { a.handleSignal(data, from: "s-b") } } }
        a.prepare(iceServers: [])
        b.prepare(iceServers: [])
        a.enter(["s-b"])
        try await waitUntil("connected") { a.connectionState(of: "s-b") == .connected && b.connectionState(of: "s-a") == .connected }
        // Audio RTCP comes every ~5 s (randomized ±50 %).
        var stats = PeerStats(sid: "s-b")
        try await waitUntil("their report about our voice", timeout: .seconds(20)) {
            stats.remoteReports > 0 && stats.remoteRttMs != nil
        } refresh: {
            stats = await a.peerStats().first ?? stats
        }
        let pc = try #require(a.peerConnection("s-b"))
        for s in await pc.statistics().statistics.values where s.type == "remote-inbound-rtp" {
            print("TALK remote-inbound-rtp: " + s.values.keys.sorted().map { "\($0)=\(s.values[$0].map { "\($0)" } ?? "")" }.joined(separator: " "))
        }
        print("TALK they receive from us: fraction lost \(stats.remoteFractionLost ?? -1), lost \(stats.remotePacketsLost), rtt \(stats.remoteRttMs ?? -1) ms, reports \(stats.remoteReports), sent \(stats.packetsSent)")
        #expect(stats.remoteFractionLost == 0, "nothing lost in-process")
        #expect(stats.remotePacketsLost == 0)
        #expect((stats.remoteRttMs ?? -1) >= 0)
        #expect(!a.ownLinkWeak && !b.ownLinkWeak)
    }

    /// Someone's volume reaches their voice, and only their voice: at 0 %
    /// nothing of them plays here, at 100 % they're back. (WebRTC's ObjC SDK
    /// hands out new transceiver and source wrappers on every call, so the
    /// voice is found by position, never by identity, and what was applied is
    /// remembered rather than read back.)
    @Test func theirVolumeReachesTheirVoice() async throws {
        let device = SyntheticAudioDevice()
        let a = VoiceEngine(audioDevice: device)
        let b = VoiceEngine(audioDevice: SyntheticAudioDevice())
        defer {
            a.stop()
            b.stop()
        }
        for (engine, sid) in [(a, "s-a"), (b, "s-b")] {
            engine.mySid = sid
            engine.mode = .openMic
        }
        a.uidOf = { _ in "u-b" }
        a.onSignal = { [b] _, data in DispatchQueue.main.async { MainActor.assumeIsolated { b.handleSignal(data, from: "s-a") } } }
        b.onSignal = { [a] _, data in DispatchQueue.main.async { MainActor.assumeIsolated { a.handleSignal(data, from: "s-b") } } }
        a.prepare(iceServers: [])
        b.prepare(iceServers: [])
        a.enter(["s-b"])
        try await waitUntil("connected") { a.connectionState(of: "s-b") == .connected && b.connectionState(of: "s-a") == .connected }
        try await waitUntil("their voice plays here") { device.recentPlayoutRMS > 0.01 }
        // The test chord swells and fades every 2 s: average over that.
        func heard() async throws -> Double {
            var sum = 0.0
            for _ in 0..<40 {
                sum += device.recentPlayoutRMS
                try await Task.sleep(for: .milliseconds(50))
            }
            return sum / 40
        }
        func set(_ volume: Double) async throws -> Double {
            a.volumes = ["u-b": volume]
            a.applySettings(renegotiate: false)
            try await Task.sleep(for: .milliseconds(400))
            return try await heard()
        }
        let full = try await heard()
        let silent = try await set(0)
        let half = try await set(0.5)
        // The dip is for a screen's sound, never their voice.
        a.dipSharedMedia = true
        let dipped = try await heard()
        print("TALK their volume: 100 % → RMS \(full), 0 % → \(silent), 50 % → \(half), 50 % with the dip on → \(dipped)")
        #expect(silent < full * 0.02, "nothing of them at 0 %")
        #expect(half > full * 0.35 && half < full * 0.65, "half at 50 %")
        #expect(dipped > full * 0.35, "their voice isn't dipped")
        #expect(a.mediaDip == 0.5, "they're speaking, so a share's sound would be at half")
    }

    private func waitUntil(_ what: String, timeout: Duration, _ condition: () -> Bool, refresh: () async -> Void) async throws {
        let deadline = ContinuousClock.now + timeout
        while true {
            await refresh()
            if condition() { return }
            if ContinuousClock.now > deadline {
                Issue.record("timed out waiting for: \(what)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(250))
        }
    }
}
#endif
