import Foundation
import Testing
@testable import YapCore

/// Screen sharing and Watch together on the wire: who's sharing (the LIVE
/// badge), a viewer's want/stop, the sharer's `voice.state {screen}`, and
/// who's in the Watch together audience and what's on.
@MainActor
@Suite("Screen sharing and Watch together")
struct ScreenWatchTests {
    func client() -> YapClient {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t"), client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me",
                                            channels: [Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0)],
                                            users: [User(id: "me", name: "Me", color: "#fff"), User(id: "ada", name: "Ada", color: "#fff"),
                                                    User(id: "ben", name: "Ben", color: "#fff")],
                                            sessions: [Session(sid: "s0", uid: "me", client: "ios")]),
                        histories: [:])
        return client
    }

    @Test func screenFlagDecodes() throws {
        guard case let .voice(_, channelId, _, _, screen, _, _, _) = try ServerMessage.decode(#"{"t":"voice","sid":"s1","channelId":"lounge","muted":false,"deafened":false,"screen":true}"#) else {
            Issue.record("not a voice message")
            return
        }
        #expect(channelId == "lounge" && screen)
        guard case let .voice(_, _, _, _, older, _, _, _) = try ServerMessage.decode(#"{"t":"voice","sid":"s1","channelId":"lounge","muted":false,"deafened":false}"#) else {
            Issue.record("not a voice message")
            return
        }
        #expect(!older, "servers without screen sharing omit it")
        let session = try JSONDecoder().decode(Session.self, from: Data(#"{"sid":"s1","uid":"ada","client":"web","voice":{"channelId":"lounge","muted":false,"deafened":false,"screen":true},"view":null,"watching":true}"#.utf8))
        #expect(session.voice?.screen == true && session.watching)
        let plain = try JSONDecoder().decode(Session.self, from: Data(#"{"sid":"s2","uid":"ben","client":"web","voice":{"channelId":"lounge","muted":true,"deafened":false},"view":null}"#.utf8))
        #expect(plain.voice?.screen == false && !plain.watching)
    }

    @Test func sharingShowsAndStops() throws {
        let c = client()
        c.handle(try ServerMessage.decode(#"{"t":"session.join","session":{"sid":"s1","uid":"ada","client":"web","voice":null,"view":null}}"#))
        c.handle(try ServerMessage.decode(#"{"t":"voice","sid":"s1","channelId":"lounge","muted":false,"deafened":false,"screen":true}"#))
        #expect(c.voiceSessions(in: "lounge").first?.voice?.screen == true)
        c.handle(try ServerMessage.decode(#"{"t":"voice","sid":"s1","channelId":"lounge","muted":true,"deafened":false}"#))
        #expect(c.voiceSessions(in: "lounge").first?.voice?.screen == false)
    }

    /// A viewer's `{screen: "want"|"stop"}` (PROTOCOL.md › Screen sharing)
    /// decodes and encodes exactly as the web sends it; anything else in
    /// `data` is still dropped.
    @Test func screenSignals() throws {
        guard case let .signal(from, data) = try ServerMessage.decode(#"{"t":"signal","from":"s1","data":{"screen":"want"}}"#) else {
            Issue.record("not a signal")
            return
        }
        #expect(from == "s1" && data == .screen(true))
        guard case let .signal(_, stop) = try ServerMessage.decode(#"{"t":"signal","from":"s1","data":{"screen":"stop"}}"#) else {
            Issue.record("not a signal")
            return
        }
        #expect(stop == .screen(false))
        guard case let .signal(_, odd) = try ServerMessage.decode(#"{"t":"signal","from":"s1","data":{"screen":"maybe"}}"#) else {
            Issue.record("not a signal")
            return
        }
        #expect(odd == nil, "only want and stop")
        let want = try JSONSerialization.jsonObject(with: Data(ClientMessage.signal(to: "s1", data: .screen(true)).frame().utf8)) as! [String: Any]
        #expect((want["data"] as? [String: String]) == ["screen": "want"] && want["to"] as? String == "s1")
        let c = client()
        c.handle(.signal(from: "s1", data: nil))
        // Not sharing: voice.state says nothing about a screen (older servers and apps see what they always did).
        let state = try JSONSerialization.jsonObject(with: Data(ClientMessage.voiceState(muted: false, deafened: false).frame().utf8)) as! [String: Any]
        #expect(state["screen"] == nil)
        let sharing = try JSONSerialization.jsonObject(with: Data(ClientMessage.voiceState(muted: true, deafened: false, screen: true).frame().utf8)) as! [String: Any]
        #expect(sharing["screen"] as? Bool == true && sharing["muted"] as? Bool == true)
        // A resumed join keeps the share going (`screen` only rides with `prev`).
        let resumed = try JSONSerialization.jsonObject(with: Data(ClientMessage.voiceJoin(channelId: "lounge", muted: false, deafened: false, resume: true, prev: "old", proof: "k", screen: true).frame().utf8)) as! [String: Any]
        #expect(resumed["screen"] as? Bool == true && resumed["prev"] as? String == "old")
        let fresh = try JSONSerialization.jsonObject(with: Data(ClientMessage.voiceJoin(channelId: "lounge", muted: false, deafened: false, resume: true, screen: true).frame().utf8)) as! [String: Any]
        #expect(fresh["screen"] == nil, "a fresh join announces the share with voice.state once it lands")
        #expect(try ClientMessage.watchGet.frame().contains(#""t":"watch.get""#))
        #expect(try ClientMessage.deviceHandoff.frame() == #"{"t":"device.handoff"}"#)
        guard case .deviceHandoff(let token) = try ServerMessage.decode(#"{"t":"device.handoff","token":"abc_D-9"}"#) else {
            Issue.record("device.handoff decodes")
            return
        }
        #expect(token == "abc_D-9")
    }

    @Test func watchAudienceAndTitle() throws {
        let c = client()
        #expect(c.watchAudience.isEmpty && c.watchTitle == nil)
        c.handle(try ServerMessage.decode(#"{"t":"session.join","session":{"sid":"s1","uid":"ben","client":"web","voice":null,"view":null,"watching":true}}"#))
        c.handle(try ServerMessage.decode(#"{"t":"session.join","session":{"sid":"s2","uid":"ada","client":"web","voice":null,"view":null}}"#))
        c.handle(try ServerMessage.decode(#"{"t":"watching","sid":"s2","on":true}"#))
        #expect(c.watchAudience.map(\.uid) == ["ada", "ben"], "A–Z")
        #expect(c.watchTitle == nil, "nothing known yet")
        c.handle(try ServerMessage.decode(#"{"t":"watch.state","state":{"videoId":"dQw4w9WgXcQ","title":"Never Gonna","playing":true,"position":12.5,"at":1000,"by":"ben","queue":[{"id":"abc","title":null,"by":"ada"}]},"now":2000}"#))
        #expect(c.watch?.queue.count == 1 && c.watch?.position == 12.5)
        #expect(c.watchTitle == "Never Gonna")
        c.handle(try ServerMessage.decode(#"{"t":"watching","sid":"s1","on":false}"#))
        c.handle(try ServerMessage.decode(#"{"t":"watching","sid":"s2","on":false}"#))
        #expect(c.watchAudience.isEmpty && c.watchTitle == nil, "no audience, no title")
        // Nothing playing: no title even with an audience.
        c.handle(try ServerMessage.decode(#"{"t":"watching","sid":"s2","on":true}"#))
        c.handle(try ServerMessage.decode(#"{"t":"watch.state","state":{"videoId":null,"title":null,"playing":false,"position":0,"at":0,"by":null,"queue":[]},"now":0}"#))
        #expect(c.watchTitle == nil)
    }
}
