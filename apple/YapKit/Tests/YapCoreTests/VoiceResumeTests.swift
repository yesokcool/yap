import Foundation
import Testing
@testable import YapCore

/// PROTOCOL.md › Voice › Keeping calls across reconnects, on the wire, and
/// the client keeping someone listed while voice waits for them.
@MainActor
@Suite("Keeping calls across reconnects: wire and listing")
struct VoiceResumeTests {
    func json(_ message: ClientMessage) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: Data(message.frame().utf8)) as! [String: Any]
    }

    @Test func joinSaysResumeAndCarriesTheProof() throws {
        let plain = try json(.voiceJoin(channelId: "lounge", muted: false, deafened: false))
        #expect(plain["resume"] == nil && plain["prev"] == nil, "older apps' join is unchanged")
        let fresh = try json(.voiceJoin(channelId: "lounge", muted: false, deafened: false, resume: true))
        #expect(fresh["resume"] as? Bool == true)
        #expect(fresh["prev"] == nil && fresh["proof"] == nil)
        let resumed = try json(.voiceJoin(channelId: "lounge", muted: true, deafened: false, resume: true, prev: "old", proof: "k".padding(toLength: 43, withPad: "k", startingAt: 0)))
        #expect(resumed["t"] as? String == "voice.join")
        #expect(resumed["prev"] as? String == "old")
        #expect((resumed["proof"] as? String)?.count == 43)
        #expect(resumed["muted"] as? Bool == true)
    }

    @Test func voiceEventsAndReadyDecodeTheResumeFields() throws {
        guard case let .voice(sid, channelId, _, _, _, resume, prev, lost) = try ServerMessage.decode(#"{"t":"voice","sid":"new","channelId":"lounge","muted":false,"deafened":false,"resume":true,"prev":"old"}"#) else {
            Issue.record("not a voice event")
            return
        }
        #expect(sid == "new" && channelId == "lounge" && resume && prev == "old" && !lost)
        guard case let .voice(_, gone, _, _, _, _, _, isLost) = try ServerMessage.decode(#"{"t":"voice","sid":"old","channelId":null,"muted":false,"deafened":false,"lost":true}"#) else {
            Issue.record("not a voice event")
            return
        }
        #expect(gone == nil && isLost)
        guard case let .voice(_, _, _, _, _, oldResume, oldPrev, oldLost) = try ServerMessage.decode(#"{"t":"voice","sid":"x","channelId":"lounge","muted":false,"deafened":false}"#) else {
            Issue.record("not a voice event")
            return
        }
        #expect(!oldResume && oldPrev == nil && !oldLost, "an older server's events")

        guard case let .ready(ready) = try ServerMessage.decode(#"{"t":"ready","sid":"s1","uid":"u1","resumeKey":"abc","channels":[],"users":[],"sessions":[{"sid":"s2","uid":"u2","client":"web","voice":{"channelId":"lounge","muted":false,"deafened":false,"resume":true,"prev":"s0"}}]}"#) else {
            Issue.record("not ready")
            return
        }
        #expect(ready.resumeKey == "abc")
        #expect(ready.sessions.first?.voice?.resume == true)
        #expect(ready.sessions.first?.voice?.prev == "s0")
    }

    @Test func aHeldSessionStaysListedThroughItsLeaveUntilReleased() {
        let client = YapClient(credentials: MemoryCredentialStore(), client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "me", uid: "u1", channels: [Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0)],
                                            users: [], sessions: [Session(sid: "s2", uid: "u2", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false, resume: true))]),
                        histories: [:])
        #expect(client.resumeKey == nil, "this server sent none")
        // Their socket drops: the client clears them, voice holds them.
        client.receiveForTesting(.voice(sid: "s2", channelId: nil, muted: false, deafened: false, screen: false, lost: true))
        client.holdSession(Session(sid: "s2", uid: "u2", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false, resume: true)))
        client.receiveForTesting(.sessionLeave(sid: "s2"))
        #expect(client.voiceSessions(in: "lounge").map(\.sid) == ["s2"], "still listed (reconnecting…)")
        client.releaseSession("s2")
        #expect(client.voiceSessions(in: "lounge").isEmpty)
        #expect(client.heldSessions.isEmpty)
    }

    @Test func voiceEventsReachTheCoordinatorWithWhatTheyReplace() {
        final class Recorder: VoiceCoordinator {
            var changes: [VoiceChange] = []
            func clientDidBecomeReady() {}
            func clientConnectionDropped() {}
            func clientDidSignOut() {}
            func clientVoiceChanged(_ change: VoiceChange) { changes.append(change) }
            func clientSessionLeft(sid: String) {}
            func clientReceivedSignal(_ data: SignalData, from sid: String) {}
        }
        let client = YapClient(credentials: MemoryCredentialStore(), client: "test")
        let recorder = Recorder()
        client.voice = recorder
        client.loadDemo(ready: ReadyPayload(sid: "me", uid: "u1", channels: [Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0)],
                                            users: [], sessions: [Session(sid: "s2", uid: "u2", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false, resume: true))]),
                        histories: [:])
        client.receiveForTesting(.voice(sid: "s2", channelId: nil, muted: false, deafened: false, screen: false, lost: true))
        let change = recorder.changes.last
        #expect(change?.previous == "lounge")
        #expect(change?.before?.resume == true, "whether they could have come back")
        #expect(change?.lost == true)
    }
}
