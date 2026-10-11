#if os(iOS)
import Foundation
import Testing
@testable import YapCore
@testable import YapVoice

/// iPhone calls dropped when the screen went off: WebRTC stops the audio
/// device the moment its last audio stream goes, and that happened for a
/// split second whenever the people in the call changed. On a real iPhone
/// the voice-processing unit then stops and the audio session is let go; in
/// the background iOS pauses Yap and won't start the mic again. Here two
/// engines on synthetic audio devices (no mic, no server) go through the
/// changes that did it, and our device must never go fully quiet until we
/// leave.
@MainActor
@Suite("The call's sound keeps running while people come and go", .serialized)
final class AudioAnchorTests {
    let device = SyntheticAudioDevice()
    let me: VoiceEngine
    let friend = VoiceEngine(audioDevice: SyntheticAudioDevice())

    init() {
        me = VoiceEngine(audioDevice: device)
        me.mySid = "s-me"
        friend.mySid = "s-friend"
        me.uidOf = { _ in "friend" }
        friend.uidOf = { _ in "me" }
        // Signals in order, like the socket (one main-queue hop each).
        me.onSignal = { [friend] _, data in
            DispatchQueue.main.async { MainActor.assumeIsolated { friend.handleSignal(data, from: "s-me") } }
        }
        friend.onSignal = { [me] _, data in
            DispatchQueue.main.async { MainActor.assumeIsolated { me.handleSignal(data, from: "s-friend") } }
        }
    }

    private func waitUntil(_ what: String, timeout: Duration = .seconds(15), _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            if ContinuousClock.now > deadline {
                Issue.record("timed out waiting for: \(what)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    private func connected() -> Bool {
        me.connectionState(of: "s-friend") == .connected && friend.connectionState(of: "s-me") == .connected
    }

    /// Nobody's sound changes for a moment (the device would show a stop).
    private func settle() async throws {
        try await Task.sleep(for: .milliseconds(600))
    }

    @Test func theSoundNeverStopsMidCall() async throws {
        defer {
            me.stop()
            friend.stop()
        }
        // Alone in the channel (the level monitor and the anchor).
        me.prepare(iceServers: [])
        #expect(me.holdsAudio, "the anchor starts with the call")
        try await waitUntil("our sound runs") { device.isRunningIO }
        try await settle()

        // Someone joins: they offer, the level monitor closes.
        friend.prepare(iceServers: [])
        friend.enter(["s-me"])
        try await waitUntil("connected") { connected() }
        try await settle()
        #expect(device.ioStops == 0, "someone joining while I was alone stopped the sound")

        // Their connection is rebuilt (an ICE restart that didn't help, a fresh start).
        me.restart("s-friend")
        try await waitUntil("connected again") { connected() }
        try await settle()
        #expect(device.ioStops == 0, "rebuilding the only connection stopped the sound")

        // They leave: I'm alone again.
        me.drop("s-friend")
        friend.drop("s-me")
        try await settle()
        #expect(device.ioStops == 0, "the last person leaving stopped the sound")

        // Our socket gave up on the call (everyone hung up, a newcomer's join next).
        friend.enter(["s-me"])
        try await waitUntil("connected once more") { connected() }
        me.hangUpAll()
        friend.drop("s-me")
        try await settle()
        #expect(device.ioStops == 0, "hanging up on everyone stopped the sound")
        #expect(device.isRunningIO)

        // Leaving for real lets the sound stop.
        me.stop()
        #expect(!me.holdsAudio)
        try await waitUntil("our sound stops after leaving") { !device.isRunningIO }
        #expect(device.ioStops == 1, "stopped \(device.ioStops) times")
    }
}
#endif
