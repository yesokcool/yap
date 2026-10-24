import Foundation
import Testing
import WebRTC
@testable import YapCore
@testable import YapVoice

/// Hide my IP address fails closed (security review, 2026-10-24): on, it's
/// relay-only even when the server has no relay or `/api/ice` didn't answer
/// (the web's turned itself off then, and connected directly); the call says
/// nobody can be reached.
@MainActor
@Suite("Hide my IP address")
struct HideMyIPTests {
    func controller() -> (VoiceController, YapClient) {
        let client = YapClient(credentials: MemoryCredentialStore(), client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s1", uid: "me", channels: [Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0)],
                                            users: [], sessions: [Session(sid: "s1", uid: "me")]), histories: [:])
        #if os(iOS)
        let engine = VoiceEngine(audioDevice: SyntheticAudioDevice())
        #else
        let engine = VoiceEngine()
        #endif
        let voice = VoiceController(client: client, engine: engine, defaults: UserDefaults(suiteName: "yap.tests.hideIP.\(UUID().uuidString)")!,
                                    requestsMicPermission: false)
        return (voice, client)
    }

    @Test func onMeansRelayOnlyRelayOrNot() {
        let (voice, client) = controller()
        _ = client
        voice.settings.relayOnly = true
        let relay = [IceServerConfig(urls: ["turn:turn.example.com:3478"], username: "u", credential: "c")]
        for ice in [relay, IceServerConfig.fallback, []] {
            voice.applyRelaySetting(ice)
            #expect(voice.engine.relayOnly)
            #expect(voice.engine.configuration().iceTransportPolicy == .relay, "never a direct connection")
        }
        #expect(voice.relayAvailable == false)
        voice.settings.relayOnly = false
        voice.applyRelaySetting(IceServerConfig.fallback)
        #expect(!voice.engine.relayOnly && voice.engine.configuration().iceTransportPolicy == .all)
    }

    @Test func noRelayIsSaid() {
        let (voice, client) = controller()
        voice.settings.relayOnly = true
        voice.applyRelaySetting(IceServerConfig.fallback)
        voice.engine.prepare(iceServers: IceServerConfig.fallback)
        #expect(!voice.relayMissing, "only in a call")
        // Our join lands.
        client.receiveForTesting(.voice(sid: "s1", channelId: "lounge", muted: false, deafened: false, screen: false))
        #expect(voice.relayMissing)
        #expect(voice.reportSettings().contains("Hide my IP address: on, but this server has no voice relay (nobody can be reached)"))
        voice.leave()
        #expect(!voice.relayMissing)
    }
}
