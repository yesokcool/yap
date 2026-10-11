import Foundation
import Testing
@testable import YapCore

/// A plain socket for what the app itself never sends (an OBS link's
/// `hello`, `obs.create`) and to read close codes as the server sends them.
@MainActor
final class RawSocket {
    let task: URLSessionWebSocketTask
    private(set) var frames: [[String: Any]] = []
    private(set) var closed = false

    init(_ endpoint: ServerEndpoint) {
        task = URLSession.shared.webSocketTask(with: endpoint.webSocketURL, protocols: endpoint.webSocketProtocols)
        task.resume()
        Task { await self.receive() }
    }

    private func receive() async {
        while true {
            guard let message = try? await task.receive() else {
                closed = true
                return
            }
            if case .string(let text) = message, text != "pong",
               let frame = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] {
                frames.append(frame)
            }
        }
    }

    func send(_ frame: [String: Any]) async throws {
        let data = try JSONSerialization.data(withJSONObject: frame)
        try await task.send(.string(String(decoding: data, as: UTF8.self)))
    }

    /// The first frame (oldest first) that matches, taken out of the list.
    func next(_ what: String, timeout: Duration = .seconds(10), where match: @escaping ([String: Any]) -> Bool) async throws -> [String: Any] {
        try await waitUntil(what, timeout: timeout) { self.frames.contains(where: match) }
        return frames.remove(at: frames.firstIndex(where: match)!)
    }

    func type(_ t: String) -> ([String: Any]) -> Bool { { $0["t"] as? String == t } }

    func close() { task.cancel(with: .goingAway, reason: nil) }
}

/// OBS links that follow their account, against the dev server
/// (PROTOCOL.md › Voice › OBS links in a call).
@MainActor
@Suite("OBS links in a call (dev server)", .serialized, .enabled(if: TestServer.configured))
struct ObsListeningIntegrationTests {
    @Test func greyedOutWhileOnlyTheObsLinkIsThere() async throws {
        let (ada, adaStore) = try await member("Oda\(TestServer.run)", color: "#d6409f")
        let (ben, _) = try await member("Obe\(TestServer.run)", color: "#f76b15")
        // A voice channel of our own, so nothing else running sits in it.
        ada.createChannel(name: "obs-\(TestServer.run)", kind: .voice)
        try await waitUntil("the channel") { ada.createdChannel != nil }
        let channel = try #require(ada.createdChannel)
        try await waitUntil("Ben knows the channel") { ben.channelsById[channel.id] != nil }

        // Ada makes an OBS link (a second connection of hers: the app never does).
        let maker = RawSocket(TestServer.endpoint)
        try await maker.send(["t": "hello", "v": 1, "client": "web", "token": try #require(adaStore.token)])
        _ = try await maker.next("Ada's second connection", where: maker.type("ready"))
        try await maker.send(["t": "obs.create"])
        let obsToken = try #require(try await maker.next("obs.created", where: maker.type("obs.created"))["token"] as? String)
        maker.close()

        let obs = RawSocket(TestServer.endpoint)
        try await obs.send(["t": "hello", "v": 1, "client": "obs", "token": obsToken])
        let obsReady = try await obs.next("the OBS link's ready", where: obs.type("ready"))
        let obsSid = try #require(obsReady["sid"] as? String)

        // Ada isn't in the channel: the OBS link has to wait for her.
        try await obs.send(["t": "voice.join", "channelId": channel.id, "muted": true, "deafened": false])
        let refused = try await obs.next("not_in_voice", where: { $0["t"] as? String == "error" && $0["re"] as? String == "voice.join" })
        #expect(refused["code"] as? String == "not_in_voice")
        #expect(refused["message"] as? String == "Waiting for \(ada.me?.name ?? "") to join \(channel.name).")

        // Ada joins; now it may listen, and it shows nowhere.
        ada.send(.voiceJoin(channelId: channel.id, muted: true, deafened: false))
        try await waitUntil("Ada in the channel, for Ben") { ben.voiceRoster(in: channel.id).contains { $0.uid == ada.uid } }
        try await obs.send(["t": "voice.join", "channelId": channel.id, "muted": true, "deafened": false])
        try await waitUntil("the OBS link in the channel") { ben.voiceSessions(in: channel.id).contains { $0.sid == obsSid } }
        #expect(ben.obsStillListening(in: channel.id).isEmpty, "invisible while Ada is in the call")
        #expect(ben.voiceRoster(in: channel.id).map(\.uid) == [ada.uid])

        // Ada leaves: her OBS link keeps listening, greyed out, not a member.
        ada.send(.voiceLeave)
        try await waitUntil("Ada out, her OBS link still listening") { ben.obsStillListening(in: channel.id).map(\.sid) == [obsSid] }
        #expect(ben.voiceRoster(in: channel.id).isEmpty, "not counted")
        #expect(ben.voiceChannel(of: ada.uid) == nil)
        #expect(ben.voiceSessions(in: channel.id).contains { $0.sid == obsSid }, "the call still connects to it")

        // Back: invisible again.
        ada.send(.voiceJoin(channelId: channel.id, muted: true, deafened: false))
        try await waitUntil("Ada back") { ben.voiceRoster(in: channel.id).contains { $0.uid == ada.uid } }
        #expect(ben.obsStillListening(in: channel.id).isEmpty)

        // Gone for good when the grace runs out (a test server shortens the
        // 5 minutes: --var TEST_OBS_GRACE:<seconds>, told here as YAP_TEST_OBS_GRACE).
        if let grace = Int(TestServer.env["YAP_TEST_OBS_GRACE"] ?? ""), grace > 0 {
            ada.send(.voiceLeave)
            try await waitUntil("greyed out again") { ben.obsStillListening(in: channel.id).count == 1 }
            try await waitUntil("taken out after \(grace) s", timeout: .seconds(grace + 10)) { ben.obsStillListening(in: channel.id).isEmpty }
            #expect(!ben.voiceSessions(in: channel.id).contains { $0.sid == obsSid }, "not listening any more")
            let out = try await obs.next("the OBS link told it's out", where: { $0["t"] as? String == "voice" && $0["sid"] as? String == obsSid && $0["channelId"] is NSNull })
            #expect(out["channelId"] is NSNull)
        }

        obs.close()
        ada.send(.voiceLeave)
        ada.archiveChannel(channel.id)
        ada.signOut()
        ben.signOut()
    }
}
