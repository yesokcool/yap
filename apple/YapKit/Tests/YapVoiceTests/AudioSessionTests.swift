#if os(iOS)
import AVFoundation
import Foundation
import Testing
@testable import YapVoice

/// Outside calls the app's audio session is ambient and mixes with others,
/// so Yap's blips never stop someone's music and follow the silent switch.
/// WebRTC sets play and record for a call and doesn't put that back when the
/// call ends (it only lets the session go): measured here with the real
/// audio device, it was still play and record, voice chat, not mixable. The
/// app puts the everyday session back a moment after each call
/// (`AppModel.settleAudioSession`).
@MainActor
@Suite("The audio session after a call", .serialized)
struct AudioSessionTests {
    let session = AVAudioSession.sharedInstance()

    private func waitUntil(_ what: String, timeout: Duration = .seconds(5), _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            if ContinuousClock.now > deadline {
                Issue.record("timed out waiting for: \(what)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(50))
        }
    }

    @Test func aCallEndsAndTheEverydaySessionComesBack() async throws {
        AudioSessionSetup.everyday()
        #expect(!AudioSessionSetup.leftByACall)
        AudioSessionSetup.configure(highQualityBluetooth: false)
        let engine = VoiceEngine()
        engine.mySid = "s-me"
        // The call's anchor starts the audio device, as joining does.
        engine.prepare(iceServers: [])
        try await waitUntil("the call's session") { session.category == .playAndRecord }
        engine.stop()
        try await Task.sleep(for: .seconds(1))
        if AudioSessionSetup.leftByACall {
            // (What WebRTC does today; the app's settle step is for this.)
            #expect(!session.categoryOptions.contains(.mixWithOthers))
        }
        AudioSessionSetup.everyday()
        #expect(session.category == .ambient)
        #expect(session.categoryOptions.contains(.mixWithOthers))
        #expect(!AudioSessionSetup.leftByACall)
    }
}
#endif
