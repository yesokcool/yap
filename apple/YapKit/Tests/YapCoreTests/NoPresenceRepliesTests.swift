import Foundation
import Testing
@testable import YapCore

/// v0.21 client rules: no presence (sessions only while in voice or on a
/// view), replies, 40k messages that collapse, erased deletes, and the
/// encrypted Personal space as the client sees it.
@MainActor
@Suite("Presence, replies, long messages, sealed notes")
struct NoPresenceRepliesTests {
    func json(_ message: ClientMessage) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: Data(message.frame().utf8)) as! [String: Any]
    }

    func client(vault: VaultWrap? = nil) -> YapClient {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t"), client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me",
                                            channels: [Channel(id: "general", name: "general", kind: .text, position: 0),
                                                       Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0),
                                                       Channel(id: "p_me", name: "Personal space", kind: .personal, position: -1)],
                                            users: [User(id: "me", name: "Me", color: "#fff"), User(id: "ada", name: "Ada", color: "#fff"),
                                                    User(id: "ben", name: "ben", color: "#fff")],
                                            sessions: [Session(sid: "s0", uid: "me", client: "ios")], vault: vault),
                        histories: ["general": [], "p_me": []])
        return client
    }

    @Test func framesEncode() throws {
        let reply = try json(.send(channelId: "general", body: "yes", nonce: "n", gif: nil, replyTo: "m1"))
        #expect(reply["replyTo"] as? String == "m1")
        #expect(try json(.send(channelId: "general", body: "x", nonce: nil, gif: nil))["replyTo"] == nil)
        let erase = try json(.delete(id: "m1", blobs: ["b1"]))
        #expect(erase["blobs"] as? [String] == ["b1"])
        #expect(try json(.delete(id: "m1")).keys.sorted() == ["id", "t"])
        let wrap = VaultWrap(iter: 600_000, salt: "s", iv: "i", ct: "c")
        let on = try json(.vaultSet(wrap))
        #expect(on["t"] as? String == "vault.set" && (on["wrap"] as? [String: Any])?["iter"] as? Int == 600_000)
        let off = try json(.vaultSet(nil))
        #expect(off.keys.contains("wrap") && off["wrap"] is NSNull)
    }

    @Test func sealingFrames() throws {
        let seal = try json(.personalSeal([SealedNote(id: "m1", body: "e1:abc")]))
        #expect(seal["t"] as? String == "personal.seal")
        #expect((seal["notes"] as? [[String: String]]) == [["id": "m1", "body": "e1:abc"]])
        #expect(try json(.personalPlain).keys.sorted() == ["t"])
        guard case .personalPlain(let notes) = try ServerMessage.decode(#"{"t":"personal.plain","notes":[{"id":"m1","body":"hi","image":{"id":"abc","width":2,"height":3}},{"id":"m2","body":""}]}"#) else {
            Issue.record("personal.plain"); return
        }
        #expect(notes == [PlainNote(id: "m1", body: "hi", image: ImageRef(id: "abc", width: 2, height: 3)), PlainNote(id: "m2", body: "")])

        // `edited {sealed}` drops the plain picture; editedAt 0 = never edited.
        let c = client()
        c.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me", channels: [Channel(id: "p_me", name: "Personal space", kind: .personal, position: -1)],
                                       users: [User(id: "me", name: "Me", color: "#fff")], sessions: []),
                   histories: ["p_me": [Message(id: "m1", channelId: "p_me", uid: "me", body: "hi", ts: 1, image: ImageRef(id: "abc", width: 2, height: 3))]])
        c.handle(try ServerMessage.decode(#"{"t":"edited","id":"m1","channelId":"p_me","body":"e1:xyz","editedAt":0,"sealed":true}"#))
        let m = try #require(c.histories["p_me"]?.messages.first)
        #expect(m.body == "e1:xyz" && m.image == nil && m.gif == nil && m.editedAt == nil)
    }

    @Test func accountFrames() throws {
        #expect(Secret.isPassword("8 chars!") && !Secret.isPassword("7 chars") && !Secret.isPassword(String(repeating: "x", count: 129)))
        #expect(Secret.canSignIn("582039"), "an old PIN still signs in")
        #expect(try json(.accountDevices)["t"] as? String == "account.devices")
        #expect(try json(.accountRevoke(id: "abc"))["id"] as? String == "abc")
        #expect(try json(.accountSignoutOthers).keys.sorted() == ["t"])
        guard case .devices(let devices) = try ServerMessage.decode(#"{"t":"devices","devices":[{"id":"d1","client":"ios","label":"Yap for iPhone","createdAt":1,"lastSeen":2,"scope":null,"current":true},{"id":"d2","client":"web","label":"Safari on Mac","createdAt":1,"lastSeen":3,"scope":"listen"}]}"#) else {
            Issue.record("devices"); return
        }
        #expect(devices.map(\.id) == ["d1", "d2"] && devices[0].current && !devices[1].current && devices[1].isListener)
        guard case .signinAlert(let device) = try ServerMessage.decode(#"{"t":"signin.alert","device":{"id":"d3","client":"web","label":"Chrome on Windows","createdAt":5,"lastSeen":5,"scope":null}}"#) else {
            Issue.record("signin.alert"); return
        }
        #expect(device.label == "Chrome on Windows")
        guard case .ready(let ready) = try ServerMessage.decode(#"{"t":"ready","v":1,"sid":"s","uid":"u","needsPin":false,"channels":[],"users":[],"sessions":[],"weakPin":true,"weakReason":"guessable","secretKind":"password"}"#) else {
            Issue.record("ready"); return
        }
        #expect(ready.weakPin && ready.weakReason == "guessable" && ready.secretKind == "password")
        guard case let .account(_, kind, signedOut) = try ServerMessage.decode(#"{"t":"account","hasPin":true,"kind":"password"}"#) else {
            Issue.record("account"); return
        }
        #expect(kind == "password" && signedOut == nil, "older servers don't say how many devices went")
    }

    @Test func searchQueries() {
        let q = SearchQuery(#"pizza "movie night" from:Ada has:link"#)
        #expect(q.words == ["pizza", "movie night"] && q.from == "Ada" && q.has == .link)
        #expect(q.matches(text: "Pizza before MOVIE NIGHT? https://x", author: "Ada", hasImage: false, hasGif: false))
        #expect(!q.matches(text: "pizza movie night https://x", author: "Ben", hasImage: false, hasGif: false))
        #expect(!q.matches(text: "pizza movie night", author: "Ada", hasImage: false, hasGif: false), "has:link")
        #expect(!q.matches(text: "pizza movie https://x night", author: "Ada", hasImage: false, hasGif: false), "phrase stays together")
        #expect(SearchQuery("has:picture").has == .image && SearchQuery("has:gifs").has == .gif)
        let (snippet, ranges) = SearchQuery("tiger").snippet(of: String(repeating: "word ", count: 50) + "purple Tiger river " + String(repeating: "x ", count: 100))
        #expect(snippet.hasPrefix("…") && snippet.hasSuffix("…"))
        #expect(ranges.count == 1 && snippet[ranges[0]] == "Tiger")
    }

    @Test func pinsAndSearchResultsDecode() throws {
        guard case .message(let m, _) = try ServerMessage.decode(#"{"t":"message","message":{"id":"m1","channelId":"general","uid":"ada","body":"hi","ts":1,"pinnedAt":9}}"#) else {
            Issue.record("message"); return
        }
        #expect(m.pinnedAt == 9)
        #expect(try json(.search(q: "hi", channelId: nil)).keys.sorted() == ["q", "t"])
        #expect(try json(.search(q: "hi", channelId: "general"))["channelId"] as? String == "general")
        let pin = try json(.pin(id: "m1", on: false))
        #expect(pin["t"] as? String == "pin" && pin["on"] as? Bool == false)
        #expect(try json(.pins(channelId: "general"))["channelId"] as? String == "general")

        let c = client()
        c.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me", channels: [Channel(id: "general", name: "general", kind: .text, position: 0)],
                                       users: [User(id: "me", name: "Me", color: "#fff")], sessions: []),
                   histories: ["general": [Message(id: "m1", channelId: "general", uid: "me", body: "hi", ts: 1)]])
        c.handle(try ServerMessage.decode(#"{"t":"pins","channelId":"general","messages":[]}"#))
        c.handle(try ServerMessage.decode(#"{"t":"pinned","id":"m1","channelId":"general","pinnedAt":5,"by":"me"}"#))
        #expect(c.histories["general"]?.messages.first?.pinnedAt == 5)
        #expect(c.pins["general"]?.map(\.id) == ["m1"])
        c.handle(try ServerMessage.decode(#"{"t":"pinned","id":"m1","channelId":"general","pinnedAt":null,"by":"me"}"#))
        #expect(c.histories["general"]?.messages.first?.pinnedAt == nil)
        #expect(c.pins["general"]?.isEmpty == true)

        c.runSearch("hi", in: nil)
        #expect(c.search?.loading == false, "no connection: done at once")
        c.handle(try ServerMessage.decode(#"{"t":"search.results","q":"hi","messages":[{"id":"m1","channelId":"general","uid":"me","body":"hi","ts":1}]}"#))
        #expect(c.search?.messages.map(\.id) == ["m1"])
        c.handle(try ServerMessage.decode(#"{"t":"search.results","q":"old","messages":[{"id":"x","channelId":"general","uid":"me","body":"old","ts":1}]}"#))
        #expect(c.search?.messages.map(\.id) == ["m1"], "stale results are ignored")
    }

    @Test func sealedNotesAreSearchedOnTheDevice() async throws {
        let (wrap, _) = try Vault.create(passcode: "purple tiger river", iterations: 2_000)
        let c = client(vault: wrap)
        try await c.unlockVault(passcode: "purple tiger river")
        #expect(c.send(text: "spare key under the blue pot", in: "p_me"))
        let note = try #require(c.pending(in: "p_me").first)
        // Pretend the server echoed it.
        c.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me", channels: [Channel(id: "p_me", name: "Personal space", kind: .personal, position: -1)],
                                       users: [User(id: "me", name: "Me", color: "#fff")], sessions: [], vault: wrap),
                   histories: ["p_me": [Message(id: "n1", channelId: "p_me", uid: "me", body: note.body, ts: 5)]])
        try await c.unlockVault(passcode: "purple tiger river")
        c.runSearch("blue pot", in: nil)
        #expect(c.search?.sealed.map(\.id) == ["n1"])
        c.runSearch("blue pot", in: "general")
        #expect(c.search?.sealed.isEmpty == true, "only when the scope includes the space")
        c.lockVault()
        c.runSearch("blue pot", in: nil)
        #expect(c.search?.sealed.isEmpty == true, "locked: nothing to search")
    }

    @Test func repliesDecodeAndCountAsMentions() throws {
        guard case .message(let m, _) = try ServerMessage.decode(#"{"t":"message","message":{"id":"m2","channelId":"general","uid":"ada","body":"same","ts":2,"reply":{"id":"m1","uid":"me","body":"line one\nline two","image":true}}}"#) else {
            Issue.record("message"); return
        }
        #expect(m.reply == Reply(id: "m1", uid: "me", body: "line one\nline two", image: true))
        #expect(m.reply?.preview == "line one…")
        let c = client()
        #expect(c.mentionsMe(m), "a reply to me is a mention")
        var mine = m
        mine.uid = "me"
        #expect(!c.mentionsMe(mine), "not when I answer myself")
        var toAda = m
        toAda.reply?.uid = "ada"
        #expect(!c.mentionsMe(toAda))
        guard case .message(let gone, _) = try ServerMessage.decode(#"{"t":"message","message":{"id":"m3","channelId":"general","uid":"ada","body":"?","ts":3,"reply":{"id":"m1","uid":"me","body":"","deleted":true}}}"#) else {
            Issue.record("deleted"); return
        }
        #expect(gone.reply?.deleted == true && gone.reply?.preview == "Original message was deleted")
        #expect(!c.mentionsMe(gone))
    }

    @Test func aReplyStartsItsOwnGroup() {
        let channel = Channel(id: "general", name: "general", kind: .text, position: 0)
        let a = Message(id: "a", channelId: "general", uid: "ada", body: "one", ts: 1000)
        let b = Message(id: "b", channelId: "general", uid: "ada", body: "two", ts: 2000, reply: Reply(id: "x", uid: "ben", body: "hi"))
        let rows = MessageRows.build(channel: channel, messages: [a, b], pending: [], hasMore: false, loading: false, unreadAfter: nil, myUid: "me")
        guard case .message(_, let grouped)? = rows.last else { Issue.record("rows"); return }
        #expect(!grouped)
    }

    @Test func pendingRepliesCarryTheQuote() {
        let c = client()
        let original = Message(id: "m1", channelId: "general", uid: "ada", body: "pizza?", ts: 1)
        #expect(c.send(text: "yes", in: "general", replyTo: original))
        #expect(c.pending(in: "general").first?.reply?.id == "m1")
        #expect(c.send(text: "note", in: "p_me", replyTo: original))
        #expect(c.pending(in: "p_me").first?.reply == nil, "no replies in the Personal space")
    }

    @Test func longMessages() {
        #expect(MessageLength.maxChars == 40_000)
        let long = String(repeating: "word ", count: 1000)
        #expect(MessageLength.isLong(long))
        let cut = MessageLength.collapsed(long)
        #expect(cut.count <= MessageLength.collapseChars + 1 && cut.hasSuffix("…"))
        #expect(!cut.dropLast().hasSuffix(" "))
        let lines = (1...40).map { "line \($0)" }.joined(separator: "\n")
        #expect(MessageLength.isLong(lines))
        #expect(MessageLength.collapsed(lines).split(separator: "\n").count == MessageLength.collapseLines)
        #expect(!MessageLength.isLong("short\nmessage"))
        let c = client()
        #expect(c.send(text: String(repeating: "a", count: 40_000), in: "general"))
        #expect(!c.send(text: String(repeating: "a", count: 40_001), in: "general"))
    }

    /// The server shows a session only while it's in voice or on a view:
    /// `session.join` → `voice`, then `voice null` → `session.leave`.
    @Test func presenceSequence() throws {
        let c = client()
        #expect(c.members.map(\.name) == ["Ada", "ben", "Me"], "everyone A–Z, case-insensitive")
        c.handle(try ServerMessage.decode(#"{"t":"session.join","session":{"sid":"s1","uid":"ada","client":"web","voice":null,"view":null}}"#))
        c.handle(try ServerMessage.decode(#"{"t":"voice","sid":"s1","channelId":"lounge","muted":false,"deafened":false}"#))
        #expect(c.voiceSessions(in: "lounge").map(\.sid) == ["s1"])
        #expect(c.voiceChannel(of: "ada")?.id == "lounge")
        c.handle(try ServerMessage.decode(#"{"t":"voice","sid":"s1","channelId":null,"muted":false,"deafened":false}"#))
        #expect(c.voiceSessions(in: "lounge").isEmpty)
        c.handle(try ServerMessage.decode(#"{"t":"session.leave","sid":"s1"}"#))
        #expect(c.sessions["s1"] == nil)
        #expect(c.voiceChannel(of: "ada") == nil)
        // A view works the same way.
        c.handle(try ServerMessage.decode(#"{"t":"session.join","session":{"sid":"s2","uid":"ben","client":"web","voice":null,"view":null}}"#))
        c.handle(try ServerMessage.decode(#"{"t":"view","sid":"s2","view":"board"}"#))
        #expect(c.sessions(of: "ben").first?.view == "board")
        c.handle(try ServerMessage.decode(#"{"t":"view","sid":"s2","view":null}"#))
        c.handle(try ServerMessage.decode(#"{"t":"session.leave","sid":"s2"}"#))
        #expect(c.sessions(of: "ben").isEmpty)
    }

    @Test func sealedPersonalSpace() async throws {
        let (wrap, _) = try Vault.create(passcode: "purple tiger river", iterations: 2_000)
        let c = client(vault: wrap)
        #expect(c.vaultLocked && c.isSealed("p_me") && !c.isSealed("general"))
        #expect(!c.send(text: "secret", in: "p_me"), "locked: nothing goes out")
        await #expect(throws: Vault.Failure.wrongPasscode) { try await c.unlockVault(passcode: "nope nope nope") }
        try await c.unlockVault(passcode: "purple tiger river")
        #expect(c.vaultUnlocked)

        #expect(c.send(text: "grab this", in: "p_me"))
        let pending = try #require(c.pending(in: "p_me").first)
        #expect(Vault.isEncryptedNote(pending.body) && !pending.body.contains("grab"))
        #expect(c.openNote(pending.body) == .note(Vault.Note(text: "grab this")))
        #expect(c.editableText(of: pending) == "grab this")
        #expect(c.send(text: "plain", in: "general"))
        #expect(c.pending(in: "general").first?.body == "plain", "other channels stay plain")

        // Locking forgets the key; the note stays sealed.
        c.lockVault()
        #expect(c.openNote(pending.body) == .locked)
        #expect(c.editableText(of: pending) == nil)
        #expect(c.openNote("just text") == .plain)
    }

    /// A locked Personal space shows a run of notes it can't open as one row.
    @Test func lockedNotesCollapse() {
        let channel = Channel(id: "p_me", name: "Personal space", kind: .personal, position: -1)
        let day = Date(timeIntervalSince1970: 1_800_000_000).yapMillis
        let notes = (0..<4).map { Message(id: "n\($0)", channelId: "p_me", uid: "me", body: $0 == 2 ? "plain" : "e1:x\($0)", ts: day + $0 * 1000) }
        let rows = MessageRows.build(channel: channel, messages: notes, pending: [], hasMore: false, loading: false, unreadAfter: nil, myUid: "me",
                                     locked: { $0.body.hasPrefix("e1:") })
        let kinds = rows.compactMap { row -> String? in
            switch row {
            case .lockedNotes(let first, let count): "locked \(first.id)×\(count)"
            case .message(let m, _): m.id
            default: nil
            }
        }
        #expect(kinds == ["locked n0×2", "n2", "locked n3×1"])
        #expect(rows.contains { $0.id == "locked:n0" })
    }
}
