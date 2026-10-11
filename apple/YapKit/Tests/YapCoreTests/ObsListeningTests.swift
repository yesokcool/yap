import Foundation
import Testing
@testable import YapCore

/// OBS links that follow their account (PROTOCOL.md › Voice › OBS links in a
/// call): invisible while the account is in the call; while only the OBS
/// link is there, the account shows greyed out ("Ada's OBS is still
/// listening"), never as a call member.
@MainActor
@Suite("OBS links in a call")
struct ObsListeningTests {
    let ada = User(id: "ada", name: "Ada", color: "#fff")
    let ben = User(id: "ben", name: "Ben", color: "#fff")
    let me = User(id: "me", name: "Sam", color: "#fff")

    func client(_ sessions: [Session]) -> YapClient {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t"),
                               client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me",
                                            channels: [Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0),
                                                       Channel(id: "games", name: "Games", kind: .voice, position: 1)],
                                            users: [ada, ben, me],
                                            sessions: [Session(sid: "s0", uid: "me", client: "ios")] + sessions),
                        histories: [:])
        return client
    }

    func inVoice(_ channelId: String) -> VoiceState { VoiceState(channelId: channelId, muted: false, deafened: false) }

    func voice(_ client: YapClient, _ sid: String, _ channelId: String?) throws {
        let channel = channelId.map { "\"\($0)\"" } ?? "null"
        client.receiveForTesting(try ServerMessage.decode(#"{"t":"voice","sid":"\#(sid)","channelId":\#(channel),"muted":false,"deafened":false}"#))
    }

    @Test func invisibleWhileTheAccountIsInTheCall() {
        let client = client([Session(sid: "a1", uid: "ada", client: "web", voice: inVoice("lounge")),
                             Session(sid: "obs1", uid: "ada", client: "obs", voice: VoiceState(channelId: "lounge", muted: true, deafened: false))])
        #expect(client.voiceRoster(in: "lounge").map(\.sid) == ["a1"])
        #expect(client.obsStillListening(in: "lounge").isEmpty)
        #expect(client.voiceSessions(in: "lounge").map(\.sid).sorted() == ["a1", "obs1"], "the call still connects to it")
    }

    @Test func greyedOutOnceTheAccountLeaves() throws {
        let client = client([Session(sid: "a1", uid: "ada", client: "web", voice: inVoice("lounge")),
                             Session(sid: "obs1", uid: "ada", client: "obs", voice: VoiceState(channelId: "lounge", muted: true, deafened: false)),
                             Session(sid: "b1", uid: "ben", client: "ios", voice: inVoice("lounge"))])
        try voice(client, "a1", nil)
        #expect(client.voiceRoster(in: "lounge").map(\.uid) == ["ben"], "not a call member, not counted")
        #expect(client.obsStillListening(in: "lounge").map(\.uid) == ["ada"])
        #expect(client.voiceSessions(in: "lounge").map(\.sid).contains("obs1"), "and it still hears the call")
        #expect(client.voiceChannel(of: "ada") == nil, "Ada herself isn't in voice")
        // Back in the call: invisible again.
        try voice(client, "a1", "lounge")
        #expect(client.obsStillListening(in: "lounge").isEmpty)
        #expect(client.voiceRoster(in: "lounge").map(\.uid).sorted() == ["ada", "ben"])
    }

    /// Ada moved to another channel: her OBS link lingers where it was.
    @Test func lingersInTheChannelItWasIn() throws {
        let client = client([Session(sid: "a1", uid: "ada", client: "web", voice: inVoice("lounge")),
                             Session(sid: "obs1", uid: "ada", client: "obs", voice: VoiceState(channelId: "lounge", muted: true, deafened: false))])
        try voice(client, "a1", "games")
        #expect(client.obsStillListening(in: "lounge").map(\.uid) == ["ada"])
        #expect(client.obsStillListening(in: "games").isEmpty)
        #expect(client.voiceRoster(in: "games").map(\.uid) == ["ada"])
    }

    /// Two OBS links of one account show as one name; A–Z across people.
    @Test func onePerAccount() {
        let client = client([Session(sid: "obs2", uid: "ben", client: "obs", voice: inVoice("lounge")),
                             Session(sid: "obs1", uid: "ada", client: "obs", voice: inVoice("lounge")),
                             Session(sid: "obs3", uid: "ada", client: "obs", voice: inVoice("lounge"))])
        #expect(client.obsStillListening(in: "lounge").map(\.uid) == ["ada", "ben"])
        #expect(client.voiceRoster(in: "lounge").isEmpty)
    }

    /// After 5 minutes the server takes it out (`voice {channelId: null}`),
    /// then it stops being visible at all (`session.leave`).
    @Test func goneWhenTheServerTakesItOut() throws {
        let client = client([Session(sid: "obs1", uid: "ada", client: "obs", voice: inVoice("lounge"))])
        #expect(client.obsStillListening(in: "lounge").count == 1)
        try voice(client, "obs1", nil)
        #expect(client.obsStillListening(in: "lounge").isEmpty)
        client.receiveForTesting(.sessionLeave(sid: "obs1"))
        #expect(client.sessions["obs1"] == nil)
    }
}
