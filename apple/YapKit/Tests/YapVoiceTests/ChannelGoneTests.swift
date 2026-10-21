import Foundation
import Testing
@testable import YapCore
@testable import YapVoice

/// The call's channel was archived while our socket was down: the next
/// `ready` doesn't have it, and we're out of the call the same way as when
/// it's archived while we're connected. The share used to keep going (the
/// Mac's capture and the iPhone's broadcast running, "Sharing" in the call
/// bar and no new share possible) and the app wasn't told why the call ended.
@MainActor
@Suite("A call whose channel went while the socket was down")
struct ChannelGoneTests {
    @Test func theShareAndTheCallEndAndTheAppIsTold() async throws {
        let lounge = Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0)
        let general = Channel(id: "general", name: "general", kind: .text, position: 1)
        let client = YapClient(credentials: MemoryCredentialStore(), client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s1", uid: "me", channels: [lounge, general], users: [],
                                            sessions: [Session(sid: "s1", uid: "me")]), histories: [:])
        let defaults = UserDefaults(suiteName: "yap.tests.channelGone.\(UUID().uuidString)")!
        #if os(iOS)
        let engine = VoiceEngine(audioDevice: SyntheticAudioDevice())
        #else
        let engine = VoiceEngine()
        #endif
        let voice = VoiceController(client: client, engine: engine, defaults: defaults, requestsMicPermission: false)
        var events: [VoiceEvent] = []
        var endedByServer = 0
        voice.onEvent = { events.append($0) }
        voice.onEndedByServer = { endedByServer += 1 }
        // Our join lands (the server's echo of it).
        client.receiveForTesting(.voice(sid: "s1", channelId: "lounge", muted: false, deafened: false, screen: false))
        #expect(voice.channelId == "lounge")
        let feeder = SyntheticScreenFeeder(width: 64, height: 36, fps: 5)
        try await voice.startSharing(feeder)
        #expect(voice.sharing != nil)

        // The socket drops; meanwhile someone archives Lounge.
        voice.clientConnectionDropped()
        client.loadDemo(ready: ReadyPayload(sid: "s2", uid: "me", channels: [general], users: [],
                                            sessions: [Session(sid: "s2", uid: "me")]), histories: [:])

        #expect(!voice.isInVoice)
        #expect(voice.sharing == nil, "the share stops with the call")
        #expect(voice.watch == nil)
        #expect(endedByServer == 1, "the app says why the call ended")
        #expect(events.last == .left)
        voice.leave()
    }
}
