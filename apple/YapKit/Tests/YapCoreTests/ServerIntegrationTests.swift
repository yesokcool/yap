import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import YapCore

/// A local dev server for the integration tests (see README › Testing):
///   YAP_TEST_URL=http://127.0.0.1:8789 YAP_TEST_KEY=testkey YAP_TEST_ADMIN=testadmin swift test
/// Without these the suite is skipped.
enum TestServer {
    static let env = ProcessInfo.processInfo.environment
    static var configured: Bool { env["YAP_TEST_URL"] != nil && env["YAP_TEST_ADMIN"] != nil }
    static var baseURL: URL { URL(string: env["YAP_TEST_URL"] ?? "http://127.0.0.1:8789")! }
    static var endpoint: ServerEndpoint { ServerEndpoint(baseURL: baseURL, key: env["YAP_TEST_KEY"] ?? "") }
    static let run = String(UUID().uuidString.prefix(4)).lowercased()
    /// The made-up password every suite signs up with (`login.pin` / `signup.pin` on the wire).
    static let password = "correct-horse-582039"

    static func admin(_ path: String, method: String = "GET", query: [String: String] = [:], body: Data? = nil) async throws -> Data {
        var c = URLComponents(url: baseURL.appending(path: path), resolvingAgainstBaseURL: false)!
        c.queryItems = query.isEmpty ? nil : query.map { URLQueryItem(name: $0.key, value: $0.value) }
        var request = URLRequest(url: c.url!)
        request.httpMethod = method
        request.httpBody = body
        request.setValue("Bearer \(env["YAP_TEST_ADMIN"] ?? "")", forHTTPHeaderField: "Authorization")
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard status == 200 else { throw URLError(.badServerResponse, userInfo: ["status": status, "body": String(decoding: data, as: UTF8.self)]) }
        return data
    }

    /// The waiting room doesn't tell a newcomer their uid; the admin export does.
    static func uid(named name: String) async throws -> String {
        let data = try await admin("/api/admin/export", query: ["after": "0"])
        let root = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        let users = root?["users"] as? [[String: Any]] ?? []
        guard let id = users.first(where: { ($0["name"] as? String)?.lowercased() == name.lowercased() })?["id"] as? String else {
            throw URLError(.fileDoesNotExist)
        }
        return id
    }

    static func letIn(_ name: String, uid: String? = nil) async throws {
        let id: String
        if let uid, !uid.isEmpty { id = uid } else { id = try await self.uid(named: name) }
        _ = try await admin("/api/admin/role", method: "POST", query: ["uid": id, "role": "member"])
    }

    /// Tests only: give an account an old 6-digit PIN, as from before
    /// passwords (sign-up only makes passwords). Needs the test server's
    /// `--var TEST_PINS:1`.
    static func giveOldPin(_ uid: String, pin: String) async throws {
        _ = try await admin("/api/admin/test-pin", method: "POST", query: ["uid": uid, "pin": pin])
    }

    /// Only the owner and mods let people in: make someone a mod.
    static func makeMod(_ uid: String) async throws {
        _ = try await admin("/api/admin/role", method: "POST", query: ["uid": uid, "role": "mod"])
    }

    /// A tiny PNG that's unique per call (same bytes = "duplicate").
    static func uniquePNG() -> Data {
        let side = 8
        var pixels = [UInt8](repeating: 0, count: side * side * 4)
        for i in stride(from: 0, to: pixels.count, by: 4) {
            pixels[i] = UInt8.random(in: 0...255); pixels[i + 1] = UInt8.random(in: 0...255); pixels[i + 2] = 200; pixels[i + 3] = 255
        }
        let provider = CGDataProvider(data: Data(pixels) as CFData)!
        let image = CGImage(width: side, height: side, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: side * 4,
                            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
        let out = NSMutableData()
        let dest = CGImageDestinationCreateWithData(out, UTType.png.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(dest, image, nil)
        CGImageDestinationFinalize(dest)
        return out as Data
    }
}

@MainActor
func waitUntil(_ what: String, timeout: Duration = .seconds(10), _ condition: () -> Bool) async throws {
    let deadline = ContinuousClock.now + timeout
    while !condition() {
        if ContinuousClock.now > deadline {
            Issue.record("timed out waiting for: \(what)")
            throw CancellationError()
        }
        try await Task.sleep(for: .milliseconds(25))
    }
}

/// A client as a "device": its own credential store, like a separate phone.
@MainActor
func device(token: String? = nil) -> (YapClient, MemoryCredentialStore) {
    let store = MemoryCredentialStore(endpoint: TestServer.endpoint, token: token)
    return (YapClient(credentials: store, client: "test-ios"), store)
}

/// A member the admin made a mod (they let people in), once they know it.
@MainActor
func mod(_ name: String, color: String = "#5b6cff") async throws -> (YapClient, MemoryCredentialStore) {
    let (client, store) = try await member(name, color: color)
    try await TestServer.makeMod(client.uid)
    try await waitUntil("\(name) is a mod") { client.isMod }
    return (client, store)
}

/// Sign up, wait in the waiting room, get let in by the admin, reach `ready`.
@MainActor
func member(_ name: String, color: String = "#5b6cff") async throws -> (YapClient, MemoryCredentialStore) {
    let (client, store) = device()
    client.signUp(name: name, pin: TestServer.password, color: color)
    try await waitUntil("\(name) reaches the waiting room or ready") {
        if case .waiting = client.phase { return true }
        return client.phase == .online
    }
    if case .waiting = client.phase {
        #expect(store.token != nil, "pending carries a device token")
        #expect(!client.uid.isEmpty, "pending carries our uid")
        try await TestServer.letIn(name, uid: client.uid)
        try await waitUntil("\(name) let in") { client.phase == .online }
    }
    return (client, store)
}

@MainActor
@Suite("Dev server integration", .serialized, .enabled(if: TestServer.configured))
struct ServerIntegrationTests {
    @Test func keyCheckAndConfig() async throws {
        let api = ServerAPI(endpoint: TestServer.endpoint)
        #expect(try await api.check().ok)
        var bad = TestServer.endpoint
        bad.key = "nope"
        #expect(try await ServerAPI(endpoint: bad).check().ok == false)
        let config = try await api.config()
        #expect(config.gifs?.provider == "klipy")
        #expect(config.minecraft?.address.isEmpty == false)
        #expect(await api.iceServers().first?.urls.first?.hasPrefix("stun:") == true)
    }

    @Test func signupWaitingRoomApproveReady() async throws {
        // On a brand-new server the first person becomes the owner and skips
        // the waiting room; make sure someone's already in.
        _ = try await member("Host \(TestServer.run)")
        let name = "Ada \(TestServer.run)"
        let (client, store) = device()
        client.signUp(name: name, pin: TestServer.password, color: "#5b6cff")
        try await waitUntil("waiting room") { if case .waiting = client.phase { true } else { false } }
        #expect(store.token != nil)
        #expect(client.mySid == nil)
        let pendingUid = client.uid
        #expect(!pendingUid.isEmpty)
        #expect(pendingUid == (try await TestServer.uid(named: name)), "pending.uid is the account id")
        try await TestServer.letIn(name, uid: pendingUid)
        try await waitUntil("ready after approval") { client.phase == .online }
        #expect(client.mySid != nil)
        #expect(client.me?.name == name)
        #expect(client.me?.color == "#5b6cff")
        #expect(client.textChannels.contains { $0.id == "general" })
        #expect(client.voiceChannels.contains { $0.id == "lounge" })
        #expect(client.sessions[client.mySid!]?.client == "test-ios")
        #expect(!client.uid.isEmpty)
    }

    @Test func loginOnSecondDeviceAndTokenReconnect() async throws {
        let name = "Ben \(TestServer.run)"
        let (first, firstStore) = try await member(name, color: "#30a46c")
        let firstToken = try #require(firstStore.token)

        // A second device signs in with name + password (case-insensitive name).
        let (second, secondStore) = device()
        second.logIn(name: name.uppercased(), pin: TestServer.password)
        try await waitUntil("second device ready") { second.phase == .online }
        #expect(second.uid == first.uid, "same account")
        let secondToken = try #require(secondStore.token)
        #expect(secondToken != firstToken, "each device gets its own token")
        // No presence: the first device isn't told about the second one.
        try await Task.sleep(for: .milliseconds(300))
        #expect(first.sessions(of: first.uid).count == 1, "only its own session, from ready")

        // Relaunch: a fresh client with just the stored token.
        let (relaunched, _) = device(token: firstToken)
        relaunched.start()
        try await waitUntil("token reconnect") { relaunched.phase == .online }
        #expect(relaunched.uid == first.uid)

        // Sign out on the second device revokes only its token.
        second.signOut()
        #expect(second.phase == .signedOut)
        #expect(secondStore.token == nil)
        try await Task.sleep(for: .milliseconds(800))
        let (stale, staleStore) = device(token: secondToken)
        stale.start()
        try await waitUntil("revoked token rejected") { stale.phase == .signedOut }
        #expect(stale.authError?.code == "bad_token")
        #expect(staleStore.token == nil, "a revoked token is forgotten")
        #expect(first.phase == .online)
    }

    @Test func friendlySignInErrors() async throws {
        let name = "Cleo \(TestServer.run)"
        _ = try await member(name, color: "#d6409f")

        let (wrong, _) = device()
        wrong.logIn(name: name, pin: "000000")
        try await waitUntil("bad login") { wrong.authError != nil }
        #expect(wrong.authError?.code == "bad_login")
        #expect(wrong.authError?.message == "Wrong name or password.", "the server's words, as on the web")
        #expect(wrong.phase == .signedOut)

        let (taken, _) = device()
        taken.signUp(name: name.lowercased(), pin: TestServer.password, color: "#000000")
        try await waitUntil("name taken") { taken.authError != nil }
        #expect(taken.authError?.code == "name_taken")
        #expect(taken.authError?.mode == .signup)
        #expect(taken.authError?.message.contains("I have an account") == true)

        let (short, _) = device()
        short.signUp(name: "Nell \(TestServer.run)", pin: "12", color: "#000000")
        try await waitUntil("too short") { short.authError != nil }
        #expect(short.authError?.code == "bad_pin")

        // A 6-digit PIN can't start an account any more: the server says why, in its words.
        let (pin, _) = device()
        pin.signUp(name: "Tam \(TestServer.run)", pin: "582039", color: "#000000")
        try await waitUntil("no new PINs") { pin.authError != nil }
        #expect(pin.authError?.code == "bad_pin")
        #expect(pin.authError?.message == "Use a password of at least 8 characters.")
        #expect(pin.authError?.mode == .signup)
    }

    @Test func sendReceiveTextTypingAndHistory() async throws {
        let (ada, _) = try await member("Dee \(TestServer.run)")
        let (ben, _) = try await member("Eli \(TestServer.run)")
        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("histories loaded") { ada.histories["general"]?.loading == false && ben.histories["general"]?.loading == false }

        // Typing indicator.
        ben.userTyped("h", in: "general")
        try await waitUntil("typing shows") { ada.typers(in: "general").contains { $0.id == ben.uid } }

        // Text with formatting + a standard shortcode converted on send.
        let text = "hello **world** :thumbsup: \(TestServer.run)"
        #expect(ada.send(text: text + "   ", in: "general"))
        #expect(ada.pending(in: "general").count == 1, "optimistic copy")
        try await waitUntil("echo replaces the optimistic copy") { ada.outbox.isEmpty }
        try await waitUntil("ben receives") { ben.histories["general"]?.messages.contains { $0.body.hasSuffix(TestServer.run) && $0.uid == ada.uid } == true }
        let received = try #require(ben.histories["general"]?.messages.last { $0.uid == ada.uid })
        let thumbsUp = EmojiCatalog.shared.byCode["thumbsup"]!.char
        #expect(received.body == "hello **world** \(thumbsUp) \(TestServer.run)", "trailing whitespace trimmed, shortcode converted")
        #expect(ben.typers(in: "general").isEmpty, "a message clears typing")
        #expect(ada.lastRead["general"] == received.ts)

        // Unread for someone not looking at the channel.
        ben.close(channel: "general")
        ada.send(text: "are you there \(TestServer.run)", in: "general")
        try await waitUntil("unread for ben") { ben.isUnread(ben.channelsById["general"]!) }
        ben.open(channel: "general")
        #expect(!ben.isUnread(ben.channelsById["general"]!))

        // History paging: a fresh device loads the newest page, then older ones.
        for i in 0..<3 { ada.send(text: "page \(i) \(TestServer.run)", in: "general") }
        try await waitUntil("pages sent") { ada.outbox.isEmpty }
        let (pager, _) = device(token: nil)
        pager.logIn(name: "Eli \(TestServer.run)", pin: TestServer.password)
        try await waitUntil("pager ready") { pager.phase == .online }
        pager.open(channel: "general")
        try await waitUntil("newest page") { pager.histories["general"]?.loading == false }
        let newest = try #require(pager.histories["general"]?.messages)
        #expect(newest.last?.body == "page 2 \(TestServer.run)")
        #expect(zip(newest, newest.dropFirst()).allSatisfy { $0.ts < $1.ts }, "oldest first")
        if pager.histories["general"]?.hasMore == true {
            let count = newest.count
            pager.loadOlder("general")
            try await waitUntil("older page") { (pager.histories["general"]?.messages.count ?? 0) > count || pager.histories["general"]?.hasMore == false }
        }

        // Delete my own message: it disappears for the other device.
        let mine = try #require(ada.histories["general"]?.messages.last { $0.uid == ada.uid })
        ada.delete(mine)
        try await waitUntil("deleted for ben") { ben.histories["general"]?.messages.contains { $0.id == mine.id } == false }
    }

    @Test func customEmojiTokensRoundTrip() async throws {
        let (ada, _) = try await member("Fay \(TestServer.run)")
        let (ben, _) = try await member("Gus \(TestServer.run)")
        let emojiName = "yay_\(TestServer.run)"
        _ = try await TestServer.admin("/api/admin/emoji", method: "POST", query: ["name": emojiName], body: TestServer.uniquePNG())
        try await waitUntil("emoji broadcast") { ada.emojis.values.contains { $0.name == emojiName } }
        let emoji = try #require(ada.emojis.values.first { $0.name == emojiName })

        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("loaded") { ben.histories["general"]?.loading == false }
        ada.send(text: "party :\(emojiName.uppercased()): `:\(emojiName):`", in: "general")
        try await waitUntil("ben gets the emoji message") { ben.histories["general"]?.messages.contains { $0.body.contains(emoji.id) } == true }
        let body = try #require(ben.histories["general"]?.messages.last { $0.body.contains(emoji.id) }?.body)
        #expect(body == "party <:\(emojiName):\(emoji.id)> `:\(emojiName):`")
        #expect(EmojiCodec.tokens(in: body) == [EmojiCodec.Token(name: emojiName, id: emoji.id)])
        guard case .paragraph(let inlines)? = MessageFormatter.parse(body).first else { Issue.record("no paragraph"); return }
        #expect(inlines.contains(.emoji(name: emojiName, id: emoji.id)))
        #expect(inlines.contains(.code(":\(emojiName):")))

        // The image is served without a key.
        let (data, response) = try await URLSession.shared.data(from: TestServer.endpoint.emojiURL(emoji.id))
        #expect((response as? HTTPURLResponse)?.statusCode == 200)
        #expect(data.starts(with: [0x89, 0x50, 0x4E, 0x47]))

        // Jumbo when it's only emoji.
        ada.send(text: ":\(emojiName)::\(emojiName):", in: "general")
        try await waitUntil("jumbo") { ben.histories["general"]?.messages.contains { EmojiCodec.isJumbo($0.body) && $0.body.contains(emoji.id) } == true }
    }

    @Test func gifMessagesRoundTrip() async throws {
        let (ada, _) = try await member("Hal \(TestServer.run)")
        let (ben, _) = try await member("Ivy \(TestServer.run)")
        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("loaded") { ben.histories["general"]?.loading == false }
        let gif = Gif(slug: "hello-hi-\(TestServer.run)", title: "Hello", width: 498, height: 280,
                      mp4: "https://static.klipy.com/ii/abc/14/af/MCCBoQlZ.mp4",
                      webp: "https://static.klipy.com/ii/abc/14/af/eUbp2uNc.webp",
                      still: "https://static1.klipy.com/ii/abc/14/af/LyWpim71.jpg")
        ada.send(gif: gif, in: "general")
        try await waitUntil("ben gets the GIF") { ben.histories["general"]?.messages.contains { $0.gif?.slug == gif.slug } == true }
        let got = try #require(ben.histories["general"]?.messages.last { $0.gif?.slug == gif.slug })
        #expect(got.body.isEmpty)
        #expect(got.gif == gif)

        // The server only takes KLIPY's CDN.
        let bad = Gif(slug: "evil", title: "x", width: 10, height: 10, mp4: "https://evil.example.com/a.mp4")
        ada.send(gif: bad, in: "general")
        try await waitUntil("bad gif rejected") { ada.notice?.text.contains("GIF rejected") == true }
    }

    @Test func reactionsMentionsAndPings() async throws {
        let ada = try await member("Mo \(TestServer.run)").0
        let ben = try await member("Ned \(TestServer.run)").0
        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("loaded") { ada.histories["general"]?.loading == false && ben.histories["general"]?.loading == false }

        // Reactions: add, see it on the other device, take it back.
        ben.send(text: "react to me \(TestServer.run)", in: "general")
        try await waitUntil("ada has it") { ada.histories["general"]?.messages.contains { $0.body == "react to me \(TestServer.run)" } == true }
        let message = try #require(ada.histories["general"]?.messages.last { $0.body == "react to me \(TestServer.run)" })
        ada.toggleReaction("🎉", on: message)
        try await waitUntil("ben sees the reaction") {
            ben.histories["general"]?.messages.first { $0.id == message.id }?.reactions == [Reaction(emoji: "🎉", uids: [ada.uid])]
        }
        let updated = try #require(ada.histories["general"]?.messages.first { $0.id == message.id })
        ada.toggleReaction("🎉", on: updated)
        try await waitUntil("reaction removed") { ben.histories["general"]?.messages.first { $0.id == message.id }?.reactions.isEmpty == true }

        // History carries reactions.
        ben.react(to: message, emoji: "👍", on: true)
        try await waitUntil("ada sees 👍") { ada.histories["general"]?.messages.first { $0.id == message.id }?.reactions.first?.emoji == "👍" }
        let (late, _) = device()
        late.logIn(name: "Mo \(TestServer.run)", pin: TestServer.password)
        try await waitUntil("late device ready") { late.phase == .online }
        late.open(channel: "general")
        try await waitUntil("history with reactions") { late.histories["general"]?.messages.first { $0.id == message.id }?.reactions.first?.uids == [ben.uid] }

        // Mentions: @name (case-insensitive) and @here count; my own don't.
        var mentioned: [Message] = []
        ada.onMention = { mentioned.append($0) }
        ada.close(channel: "general")
        ben.send(text: "hey @\(ada.me!.name.lowercased()) look", in: "general")
        ben.send(text: "@here dinner", in: "general")
        ben.send(text: "email@\(ada.me!.name)x is not a mention", in: "general")
        try await waitUntil("two mentions") { mentioned.count == 2 }
        #expect(ada.mentionCounts["general"] == 2)
        ada.open(channel: "general")
        #expect(ada.mentionCounts["general"] == nil, "opening the channel clears them")
        let blocks = MessageFormatter.parse("hi @\(ada.me!.name)!", names: ada.memberNames)
        guard case .paragraph(let inlines)? = blocks.first else { Issue.record("no paragraph"); return }
        #expect(inlines.contains(.mention(ada.me!.name)))

        // Pings: everyone else hears it.
        var pings: [String] = []
        ada.onPing = { user, text in pings.append("\(user?.name ?? "?"): \(text)") }
        ben.pingEveryone("hop on")
        try await waitUntil("ping arrives") { !pings.isEmpty }
        #expect(pings.first == "\(ben.me!.name): hop on")
    }

    @Test func avatarUploadAndRemove() async throws {
        let (ada, _) = try await member("Pia \(TestServer.run)")
        let (ben, _) = try await member("Quin \(TestServer.run)")
        try await ada.uploadAvatar(TestServer.uniquePNG(), mime: "image/png")
        try await waitUntil("ben sees the picture") { ben.users[ada.uid]?.avatar != nil }
        let id = try #require(ben.users[ada.uid]?.avatar)
        let (data, response) = try await URLSession.shared.data(from: TestServer.endpoint.mediaURL(id))
        #expect((response as? HTTPURLResponse)?.statusCode == 200)
        #expect(!data.isEmpty)
        ada.removeAvatar()
        try await waitUntil("picture removed") { ben.users[ada.uid]?.avatar == nil }
    }

    /// Only the owner and mods see the waiting room and let people in; a
    /// member is never told, and trying anyway is `forbidden`.
    @Test func waitingRoomIsForOwnerAndMods() async throws {
        let (host, _) = try await mod("Jo \(TestServer.run)")
        let (bystander, _) = try await member("Ike \(TestServer.run)")
        let name = "Kai \(TestServer.run)"
        let (newcomer, _) = device()
        newcomer.signUp(name: name, pin: TestServer.password, color: "#f76b15")
        try await waitUntil("host sees the request") { host.waitingRoom.values.contains { $0.name == name } }
        let uid = try #require(host.waitingRoom.values.first { $0.name == name }?.id)
        try await Task.sleep(for: .milliseconds(300))
        #expect(bystander.waitingRoom.isEmpty, "members aren't told")
        bystander.notice = nil
        bystander.approve(uid)
        try await waitUntil("a member can't let people in") { bystander.notice != nil }
        if case .waiting = newcomer.phase {} else { Issue.record("still waiting after a member tried") }
        // A member who signs in while someone waits isn't told either (`ready.pending`).
        let (laptop, _) = device()
        laptop.logIn(name: "Ike \(TestServer.run)", pin: TestServer.password)
        try await waitUntil("laptop in") { laptop.phase == .online }
        #expect(laptop.waitingRoom.isEmpty)
        host.approve(uid)
        try await waitUntil("newcomer in") { newcomer.phase == .online }
        try await waitUntil("request cleared") { host.waitingRoom[uid] == nil }
        #expect(host.users[uid]?.name == name)
        // Everyone gets the joined line: the member who was never told meets
        // them in `join.resolved.user`; the newcomer gets "You're in" instead.
        let line = "\(name) joined — let in by Jo \(TestServer.run)"
        try await waitUntil("the member's joined line") { bystander.localNotes.contains { $0.plain == line } }
        try await waitUntil("the laptop's too") { laptop.localNotes.contains { $0.plain == line } }
        #expect(host.localNotes.contains { $0.plain == "\(name) joined — let in by you" })
        try await Task.sleep(for: .milliseconds(300))
        #expect(newcomer.localNotes.map(\.plain) == ["You're in. Say hi 👋"])

        // Turned away = banned: stop reconnecting.
        let (stranger, _) = device()
        let strangerName = "Lou \(TestServer.run)"
        stranger.signUp(name: strangerName, pin: TestServer.password, color: "#8d8d8d")
        try await waitUntil("host sees stranger") { host.waitingRoom.values.contains { $0.name == strangerName } }
        host.deny(host.waitingRoom.values.first { $0.name == strangerName }!.id)
        try await waitUntil("stranger blocked") { if case .blocked = stranger.phase { true } else { false } }
    }

    // MARK: v0.13–v0.17 parity

    @Test func editOwnMessages() async throws {
        let (ada, _) = try await member("Rae \(TestServer.run)")
        let (ben, _) = try await member("Sol \(TestServer.run)")
        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("loaded") { ben.histories["general"]?.loading == false && ada.histories["general"]?.loading == false }
        let text = "typo hre \(TestServer.run)"
        ada.send(text: text, in: "general")
        try await waitUntil("ada has her message") { ada.histories["general"]?.messages.contains { $0.body == text } == true }
        let original = try #require(ada.histories["general"]?.messages.last { $0.body == text })
        #expect(ada.lastOwnMessage(in: "general")?.id == original.id, "Up-arrow edits this one")

        #expect(ada.edit(original, text: "typo here \(TestServer.run)"))
        try await waitUntil("ben sees the edit") {
            ben.histories["general"]?.messages.first { $0.id == original.id }.map { $0.body == "typo here \(TestServer.run)" && $0.editedAt != nil } == true
        }
        #expect(ada.histories["general"]?.messages.first { $0.id == original.id }?.editedAt != nil)

        // Not someone else's, and not to nothing.
        let edited = try #require(ben.histories["general"]?.messages.first { $0.id == original.id })
        #expect(!ben.edit(edited, text: "hacked"), "the client only offers Edit on my own messages")
        ben.send(.edit(id: original.id, body: "hacked"))
        try await waitUntil("server refuses") { ben.notice?.text.contains("own messages") == true }
        #expect(!ada.edit(edited, text: "   "), "an empty edit on a text message is refused")

        // History carries editedAt.
        let (late, _) = device()
        late.logIn(name: "Rae \(TestServer.run)", pin: TestServer.password)
        try await waitUntil("late device ready") { late.phase == .online }
        late.open(channel: "general")
        try await waitUntil("history has the edit") { late.histories["general"]?.messages.first { $0.id == original.id }?.editedAt != nil }
    }

    @Test func picturesAndUploadTokens() async throws {
        let (ada, _) = try await member("Tia \(TestServer.run)")
        let (ben, _) = try await member("Uma \(TestServer.run)")
        let sid = try #require(ada.mySid)
        let token = try #require(ada.uploadToken, "ready carries an upload token")
        #expect(token.count == 32)
        let api = ServerAPI(endpoint: TestServer.endpoint)

        // A session id alone (visible to everyone) isn't enough any more.
        for wrong in ["", String(repeating: "0", count: 32)] {
            await #expect(throws: ServerAPI.UploadError.self) { _ = try await api.uploadImage(TestServer.uniquePNG(), mime: "image/png", sid: sid, uploadToken: wrong) }
            await #expect(throws: ServerAPI.UploadError.self) { try await api.uploadAvatar(TestServer.uniquePNG(), mime: "image/png", sid: sid, uploadToken: wrong) }
        }
        // Someone else's token doesn't work with my sid.
        let bensToken = try #require(ben.uploadToken)
        #expect(bensToken != token)
        await #expect(throws: ServerAPI.UploadError.self) { _ = try await api.uploadImage(TestServer.uniquePNG(), mime: "image/png", sid: sid, uploadToken: bensToken) }

        // A picture with a caption, end to end.
        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("loaded") { ben.histories["general"]?.loading == false }
        let picture = try PictureEncoder.prepare(TestServer.uniquePNG())
        let caption = "look \(TestServer.run)"
        try await ada.send(picture: picture, caption: caption, in: "general")
        try await waitUntil("ben gets the picture") { ben.histories["general"]?.messages.contains { $0.image != nil && $0.body == caption } == true }
        let got = try #require(ben.histories["general"]?.messages.last { $0.body == caption }?.image)
        #expect(got.width == 8 && got.height == 8)
        let (bytes, response) = try await URLSession.shared.data(from: TestServer.endpoint.mediaURL(got.id))
        #expect((response as? HTTPURLResponse)?.statusCode == 200)
        #expect(bytes.starts(with: [0x89, 0x50, 0x4E, 0x47]))

        // Avatars and emoji go through with the token.
        try await ada.uploadAvatar(TestServer.uniquePNG(), mime: "image/png")
        try await waitUntil("avatar set") { ben.users[ada.uid]?.avatar != nil }
        let emojiName = "tok_\(TestServer.run)"
        try await ada.uploadEmoji(TestServer.uniquePNG(), name: emojiName, mime: "image/png")
        try await waitUntil("emoji broadcast") { ben.emojis.values.contains { $0.name == emojiName } }
    }

    @Test func personalInvitesAndTheMainKey() async throws {
        let inviterName = "Vic \(TestServer.run)"
        let (ada, _) = try await mod(inviterName)
        ada.createInvite(label: "for Wes")
        try await waitUntil("invite made") { ada.createdInvite != nil }
        let created = try #require(ada.createdInvite)
        #expect(created.invite.label == "for Wes")
        let personal = try #require(ServerEndpoint.parse(invite: created.link))
        #expect(personal.key.hasPrefix("yi_"))
        #expect(personal.baseURL == TestServer.endpoint.baseURL)
        #expect(try await ServerAPI(endpoint: personal).check().ok, "the personal link opens the server")
        ada.refreshInvites()
        try await waitUntil("listed") { ada.invites.contains { $0.id == created.invite.id } }

        // A newcomer with the personal link: the waiting room says whose invite.
        let name = "Wes \(TestServer.run)"
        let store = MemoryCredentialStore(endpoint: personal)
        let newcomer = YapClient(credentials: store, client: "test-ios")
        newcomer.signUp(name: name, pin: TestServer.password, color: "#30a46c")
        try await waitUntil("waiting") { if case .waiting = newcomer.phase { true } else { false } }
        try await waitUntil("ada sees who invited them") { ada.waitingRoom.values.contains { $0.name == name } }
        let request = try #require(ada.waitingRoom.values.first { $0.name == name })
        #expect(request.invitedBy?.contains(inviterName) == true)
        ada.approve(request.id)
        try await waitUntil("newcomer in") { newcomer.phase == .online }
        // Members get the main key, so revoking the personal one never locks them out.
        #expect(newcomer.endpoint?.key == TestServer.endpoint.key)
        #expect(store.endpoint?.key == TestServer.endpoint.key)
        try await waitUntil("a use is counted") {
            ada.refreshInvites()
            return ada.invites.first { $0.id == created.invite.id }?.uses == 1
        }

        ada.revokeInvite(created.invite.id)
        try await waitUntil("turned off") { ada.invites.first { $0.id == created.invite.id }?.revoked == true }
        let relaunched = YapClient(credentials: store, client: "test-ios")
        relaunched.start()
        try await waitUntil("still gets in with the stored (main) key") { relaunched.phase == .online }
    }

    @Test func placesFromTheServer() async throws {
        let (ada, _) = try await member("Xan \(TestServer.run)")
        let (ben, _) = try await member("Yul \(TestServer.run)")
        #expect(ada.links.contains { $0.view == "board" }, "ready carries the Places")
        #expect(!ada.addLink(name: "Nope", url: "http://example.com"), "https only")
        let name = "Wiki \(TestServer.run)"
        #expect(ada.addLink(name: name, url: "example.com/\(TestServer.run)"))
        try await waitUntil("ben sees it") { ben.links.contains { $0.name == name } }
        let link = try #require(ben.links.first { $0.name == name })
        #expect(link.url == "https://example.com/\(TestServer.run)")
        ben.removeLink(link.id)
        try await waitUntil("removed for everyone") { !ada.links.contains { $0.id == link.id } }

        ada.refreshModLog()
        try await waitUntil("mod log has it") { ada.modLog?.contains { $0.action == "link.add" && $0.detail?.contains(name) == true } == true }
        let entry = try #require(ada.modLog?.first { $0.action == "link.remove" && $0.detail == name })
        #expect(ada.name(of: entry.actor) == ben.me?.name)
        #expect(entry.phrase { ada.name(of: $0) } == "removed \(name) from Places")
    }

    @Test func sharedVoiceQuality() async throws {
        let (ada, _) = try await member("Zia \(TestServer.run)")
        let (ben, _) = try await member("Abe \(TestServer.run)")
        defer { ada.setBitrate(nil, for: "lounge") }
        ada.setBitrate(96_000, for: "lounge")
        try await waitUntil("everyone sees 96k") { ben.channelsById["lounge"]?.bitrate == 96_000 && ada.channelsById["lounge"]?.bitrate == 96_000 }
        ada.setBitrate(12_345, for: "lounge")
        try await waitUntil("odd values refused") { ada.notice != nil }
        #expect(ben.channelsById["lounge"]?.bitrate == 96_000)
        ben.setBitrate(nil, for: "lounge")
        try await waitUntil("back to each person's own") { ada.channelsById["lounge"]?.bitrate == nil }
    }

    @Test func personalSpaceStaysWithYou() async throws {
        let name = "Bea \(TestServer.run)"
        let (phone, _) = try await member(name)
        let (mac, _) = device()
        mac.logIn(name: name, pin: TestServer.password)
        try await waitUntil("second device ready") { mac.phase == .online }
        let (other, _) = try await member("Cal \(TestServer.run)")

        let personal = try #require(phone.personalChannel, "ready carries my Personal space")
        #expect(personal.id == "p_\(phone.uid)")
        #expect(personal.kind == .personal && personal.hasMessages)
        #expect(mac.personalChannel?.id == personal.id, "same space on my other device")
        #expect(other.channelsById[personal.id] == nil, "nobody else even sees it")
        #expect(other.personalChannel?.id == "p_\(other.uid)")
        #expect(!phone.textChannels.contains { $0.id == personal.id })

        other.open(channel: "general")
        phone.open(channel: personal.id)
        try await waitUntil("loaded") { phone.histories[personal.id]?.loading == false }
        let note = "grab this on the mac \(TestServer.run)"
        #expect(phone.send(text: note, in: personal.id))
        try await waitUntil("the other device gets it") { (mac.channelsById[personal.id]?.lastMessageAt ?? 0) > 0 }
        #expect(mac.isUnread(mac.personalChannel!), "a note from my other device shows as unread")
        try await waitUntil("echo") { phone.histories[personal.id]?.messages.contains { $0.body == note && !$0.isPending } == true }
        #expect(!phone.isUnread(phone.personalChannel!), "not on the device that sent it")

        mac.open(channel: personal.id)
        try await waitUntil("history on the other device") { mac.histories[personal.id]?.messages.contains { $0.body == note } == true }
        #expect(!mac.isUnread(mac.personalChannel!))

        // Edits and reactions stay with me too.
        let sent = try #require(phone.histories[personal.id]?.messages.last { $0.body == note })
        #expect(phone.edit(sent, text: note + " (and this)"))
        try await waitUntil("edit reaches the mac") { mac.histories[personal.id]?.messages.first { $0.id == sent.id }?.editedAt != nil }

        // Someone else can't read it or post in it, and never got the note.
        other.send(.history(channelId: personal.id, before: nil, beforeId: nil, limit: 50))
        try await waitUntil("history refused") { other.notice != nil }
        other.notice = nil
        other.send(.send(channelId: personal.id, body: "sneaky", nonce: nil, gif: nil))
        try await waitUntil("send refused") { other.notice != nil }
        try await Task.sleep(for: .milliseconds(300))
        #expect(!other.histories.values.contains { $0.messages.contains { $0.body.contains(note) } })
        #expect(other.histories[personal.id] == nil)
        #expect(!(mac.histories[personal.id]?.messages.contains { $0.body == "sneaky" } ?? false))
    }

    // MARK: v0.21: no presence, replies, erased deletes, encrypted Personal space

    @Test func noPresenceUntilVoice() async throws {
        let (ada, _) = try await member("Dot \(TestServer.run)")
        let (ben, _) = try await member("Eve \(TestServer.run)")
        try await Task.sleep(for: .milliseconds(300))
        #expect(ada.sessions(of: ben.uid).isEmpty, "connected but idle: invisible")
        ben.send(.voiceJoin(channelId: "lounge", muted: true, deafened: false))
        try await waitUntil("ada sees ben in voice") { ada.voiceChannel(of: ben.uid)?.id == "lounge" }
        ben.send(.voiceLeave)
        try await waitUntil("and gone again") { ada.sessions(of: ben.uid).isEmpty }
    }

    @Test func repliesAndErasedDeletes() async throws {
        let (ada, _) = try await member("Fin \(TestServer.run)")
        let benName = "Gil \(TestServer.run)"
        let (ben, _) = try await member(benName)
        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("loaded") { ada.histories["general"]?.loading == false && ben.histories["general"]?.loading == false }
        let question = "pizza tonight? \(TestServer.run)"
        ada.send(text: question, in: "general")
        try await waitUntil("ben has it") { ben.histories["general"]?.messages.contains { $0.body == question } == true }
        let original = try #require(ben.histories["general"]?.messages.last { $0.body == question })

        var mentioned: [Message] = []
        ada.onMention = { mentioned.append($0) }
        let answer = "yes! \(TestServer.run)"
        #expect(ben.send(text: answer, in: "general", replyTo: original))
        try await waitUntil("the reply arrives with its quote") {
            ada.histories["general"]?.messages.first { $0.body == answer }?.reply?.id == original.id
        }
        let reply = try #require(ada.histories["general"]?.messages.first { $0.body == answer }?.reply)
        #expect(reply.uid == ada.uid && reply.body == question && !reply.deleted)
        try await waitUntil("a reply to ada is a mention") { mentioned.contains { $0.body == answer } }

        // Deleting erases it; the reply's quote then says so.
        let mine = try #require(ada.histories["general"]?.messages.first { $0.id == original.id })
        ada.delete(mine)
        try await waitUntil("gone for ben") { ben.histories["general"]?.messages.contains { $0.id == original.id } == false }
        let (late, _) = device()
        late.logIn(name: benName, pin: TestServer.password)
        try await waitUntil("late device ready") { late.phase == .online }
        late.open(channel: "general")
        try await waitUntil("history loaded") { late.histories["general"]?.loading == false }
        #expect(late.histories["general"]?.messages.contains { $0.id == original.id } == false, "erased, not served")
        let quoted = try #require(late.histories["general"]?.messages.first { $0.body == answer }?.reply)
        #expect(quoted.deleted && quoted.body.isEmpty)

        // 40,000 characters go through; one more doesn't.
        let long = String(repeating: "a", count: 39_990) + TestServer.run + "zzzzzz"
        #expect(ben.send(text: long, in: "general"))
        try await waitUntil("a 40k message arrives") { ada.histories["general"]?.messages.contains { $0.body.count == long.count } == true }
        #expect(!ben.send(text: long + "!", in: "general"))
    }

    @Test func encryptedPersonalSpaceAcrossDevices() async throws {
        let name = "Hux \(TestServer.run)"
        let passcode = "purple tiger river \(TestServer.run)"
        let (phone, _) = try await member(name)
        let macStore = MemoryCredentialStore(endpoint: TestServer.endpoint)
        let mac = YapClient(credentials: macStore, client: "test-mac")
        mac.logIn(name: name, pin: TestServer.password)
        try await waitUntil("second device ready") { mac.phase == .online }
        let personal = try #require(phone.personalChannel?.id)
        #expect(phone.vaultWrap == nil && !phone.isSealed(personal))

        try await phone.setUpVault(passcode: passcode, iterations: 100_000)
        #expect(phone.vaultUnlocked)
        try await waitUntil("the other device hears about it") { mac.vaultWrap != nil }
        #expect(mac.vaultLocked)

        phone.open(channel: personal)
        mac.open(channel: personal)
        try await waitUntil("loaded") { phone.histories[personal]?.loading == false && mac.histories[personal]?.loading == false }
        let secret = "spare key under the blue pot \(TestServer.run)"
        #expect(phone.send(text: secret, in: personal))
        try await waitUntil("the note reaches the mac") { mac.histories[personal]?.messages.contains { Vault.isEncryptedNote($0.body) } == true }
        let sealed = try #require(mac.histories[personal]?.messages.last { Vault.isEncryptedNote($0.body) })
        #expect(!sealed.body.contains("blue pot"), "the server only ever had ciphertext")
        #expect(mac.openNote(sealed.body) == .locked)
        await #expect(throws: Vault.Failure.wrongPasscode) { try await mac.unlockVault(passcode: "wrong wrong wrong") }
        try await mac.unlockVault(passcode: passcode)
        #expect(mac.openNote(sealed.body) == .note(Vault.Note(text: secret)))
        #expect(macStore.vaultKey(for: mac.uid)?.ct == mac.vaultWrap?.ct, "kept in the device store, tagged")

        // A picture goes up sealed through /api/blob.
        let picture = try PictureEncoder.prepare(TestServer.uniquePNG())
        try await phone.send(picture: picture, caption: "the pot", in: personal)
        try await waitUntil("the picture note arrives") {
            mac.histories[personal]?.messages.contains { if case .note(let n) = mac.openNote($0.body) { n.image != nil } else { false } } == true
        }
        let pictureNote = try #require(mac.histories[personal]?.messages.last { if case .note(let n) = mac.openNote($0.body) { n.image != nil } else { false } })
        guard case .note(let note) = mac.openNote(pictureNote.body), let image = note.image else { Issue.record("no picture"); return }
        #expect(note.text == "the pot" && image.width == 8 && image.mime == "image/png")
        // (A query string keeps this look out of the server's edge cache, so the
        // erase check below sees the store itself.)
        var peek = URLComponents(url: TestServer.endpoint.mediaURL(image.id), resolvingAgainstBaseURL: false)!
        peek.query = "peek=1"
        let (bytes, _) = try await URLSession.shared.data(from: peek.url!)
        #expect(!bytes.starts(with: [0x89, 0x50, 0x4E, 0x47]), "stored scrambled")
        #expect(try mac.openBlob(bytes) == picture.data)

        // Editing re-seals the text and keeps the picture.
        let mine = try #require(phone.histories[personal]?.messages.first { $0.id == pictureNote.id })
        #expect(phone.edit(mine, text: "the pot, left side"))
        try await waitUntil("the edit opens on the mac") {
            if let m = mac.histories[personal]?.messages.first(where: { $0.id == pictureNote.id }), case .note(let n) = mac.openNote(m.body) {
                return n.text == "the pot, left side" && n.image?.id == image.id
            }
            return false
        }

        // Deleting it erases the sealed picture too.
        let edited = try #require(phone.histories[personal]?.messages.first { $0.id == pictureNote.id })
        phone.delete(edited)
        try await waitUntil("gone on the mac") { mac.histories[personal]?.messages.contains { $0.id == pictureNote.id } == false }
        try await Task.sleep(for: .milliseconds(300))
        let (_, gone) = try await URLSession.shared.data(from: TestServer.endpoint.mediaURL(image.id))
        #expect((gone as? HTTPURLResponse)?.statusCode == 404, "blobs: [id] erased the picture")

        // A relaunch remembers the key; a passcode change elsewhere locks it again.
        let relaunched = YapClient(credentials: macStore, client: "test-mac")
        relaunched.start()
        try await waitUntil("relaunched") { relaunched.phase == .online }
        #expect(relaunched.vaultUnlocked)
        try await phone.changeVaultPasscode(old: passcode, new: passcode + " 2")
        try await waitUntil("asks again after a change elsewhere") { relaunched.vaultLocked }
        try await relaunched.unlockVault(passcode: passcode + " 2")
        #expect(relaunched.openNote(sealed.body) == .note(Vault.Note(text: secret)), "same key, new passcode")

        // Starting over: a new key; old notes stay locked.
        try await phone.setUpVault(passcode: "a whole new start", iterations: 100_000)
        try await waitUntil("the reset reaches the other device") { relaunched.vaultLocked }
        try await relaunched.unlockVault(passcode: "a whole new start")
        #expect(relaunched.openNote(sealed.body) == .unreadable)
    }

    @Test func turningEncryptionOnSealsEarlierNotes() async throws {
        let name = "Ivo \(TestServer.run)"
        let (phone, _) = try await member(name)
        let (mac, _) = device()
        mac.logIn(name: name, pin: TestServer.password)
        try await waitUntil("second device ready") { mac.phase == .online }
        let personal = try #require(phone.personalChannel?.id)
        phone.open(channel: personal)
        mac.open(channel: personal)
        try await waitUntil("loaded") { phone.histories[personal]?.loading == false && mac.histories[personal]?.loading == false }

        // Plain notes from before: text, and a picture.
        let text = "old plain note \(TestServer.run)"
        #expect(phone.send(text: text, in: personal))
        let picture = try PictureEncoder.prepare(TestServer.uniquePNG())
        try await phone.send(picture: picture, caption: "old picture", in: personal)
        try await waitUntil("both on the mac") {
            mac.histories[personal]?.messages.contains { $0.body == text } == true && mac.histories[personal]?.messages.contains { $0.image != nil } == true
        }
        let plainPicture = try #require(mac.histories[personal]?.messages.first { $0.image != nil }?.image)
        // Long notes too: sealed, eight of these are more than one 384 KB
        // frame, so they have to go back in several `personal.seal`s.
        // (Ten sends in a burst is the server's spam limit.)
        for i in 0..<8 {
            #expect(phone.send(text: "long note \(i) " + String(repeating: "x", count: 39_000), in: personal))
        }
        try await waitUntil("long notes on the mac") { (mac.histories[personal]?.messages.count ?? 0) >= 10 }

        try await phone.setUpVault(passcode: "seal the old ones \(TestServer.run)", iterations: 100_000)
        #expect(phone.notice?.text == "Your earlier notes are encrypted too.")
        try await waitUntil("the mac sees them sealed in place") {
            mac.histories[personal]?.messages.allSatisfy { Vault.isEncryptedNote($0.body) && $0.image == nil } == true
        }
        #expect(mac.histories[personal]?.messages.allSatisfy { $0.editedAt == nil } == true, "sealing isn't an edit")
        try await mac.unlockVault(passcode: "seal the old ones \(TestServer.run)")
        let opened = mac.histories[personal]?.messages.compactMap { m -> Vault.Note? in
            if case .note(let n) = mac.openNote(m.body) { return n }
            return nil
        } ?? []
        #expect(opened.contains { $0.text == text })
        let sealedPicture = try #require(opened.first { $0.text == "old picture" }?.image)
        #expect(sealedPicture.id != plainPicture.id && sealedPicture.width == 8)
        let (bytes, _) = try await URLSession.shared.data(from: TestServer.endpoint.mediaURL(sealedPicture.id))
        #expect(try mac.openBlob(bytes) == picture.data)
    }

    @Test func passwordsWeakPinsAndDevices() async throws {
        // Guessable passwords are refused at sign-up, with the server's reason.
        let (weak, _) = device()
        weak.signUp(name: "Jem \(TestServer.run)", pin: "12345678", color: "#000000")
        try await waitUntil("weak password refused") { weak.authError != nil }
        #expect(weak.authError?.code == "weak_pin")
        #expect(weak.authError?.mode == .signup)
        #expect(weak.authError?.message.isEmpty == false)

        // Sign up and sign in with a password.
        let name = "Kit \(TestServer.run)"
        let password = "correct horse \(TestServer.run) battery"
        let (phone, phoneStore) = device()
        phone.signUp(name: name, pin: password, color: "#30a46c")
        try await waitUntil("waiting room") { if case .waiting = phone.phase { true } else { false } }
        try await TestServer.letIn(name, uid: phone.uid)
        try await waitUntil("in") { phone.phase == .online }
        #expect(phone.secretKind == "password")
        #expect(phoneStore.token != nil)

        // A second device signs in with the password; the first one hears about it.
        let (laptop, _) = device()
        laptop.logIn(name: name, pin: password)
        try await waitUntil("laptop in") { laptop.phase == .online }
        try await waitUntil("the phone is told about the new sign-in") { phone.signinAlert != nil }

        // Devices: both listed, the current one marked; signing one out works.
        phone.refreshDevices()
        try await waitUntil("devices listed") { (phone.devices?.count ?? 0) >= 2 }
        #expect(phone.devices?.filter(\.current).count == 1)
        let other = try #require(phone.devices?.first { !$0.current })
        phone.revokeDevice(other.id)
        try await waitUntil("the laptop is signed out") { laptop.phase == .signedOut || laptop.authError?.code == "bad_token" }
        try await waitUntil("one device left") { phone.devices?.count == 1 }

        // Sign out everywhere else.
        let (tablet, _) = device()
        tablet.logIn(name: name, pin: password)
        try await waitUntil("tablet in") { tablet.phase == .online }
        phone.signOutOthers()
        try await waitUntil("the tablet is signed out too") { tablet.phase == .signedOut || tablet.authError?.code == "bad_token" }
        #expect(phone.phase == .online)
        #expect(phone.signinAlert == nil)

        // No going back to a PIN: refused in the server's words, nothing changes.
        phone.changePassword(to: "582039", current: password)
        try await waitUntil("refused") { if case .failed = phone.pinChange { true } else { false } }
        #expect(phone.pinChange == .failed("Use a password of at least 8 characters."))
        // A wrong current password is refused too.
        phone.changePassword(to: "new-\(password)", current: "not-\(password)")
        try await waitUntil("wrong current one") { if case .failed = phone.pinChange { true } else { false } }
        // A new password, with the right current one.
        phone.changePassword(to: TestServer.password, current: password)
        try await waitUntil("saved") { phone.pinChange == .saved }
        #expect(phone.secretKind == "password" && phone.passwordNudge == nil)
        let (watch, _) = device()
        watch.logIn(name: name, pin: TestServer.password)
        try await waitUntil("the new password signs in") { watch.phase == .online }
    }

    /// An account from before passwords: its 6-digit PIN still signs in,
    /// the app is asked once for a password, the PIN is the current one when
    /// setting it, and then only the password works.
    @Test func anOldPinStillSignsIn() async throws {
        let name = "Ola \(TestServer.run)"
        let (first, _) = try await member(name)
        try await TestServer.giveOldPin(first.uid, pin: "582039")

        let (phone, _) = device()
        phone.logIn(name: name, pin: "582039")
        try await waitUntil("in with the old PIN") { phone.phase == .online }
        #expect(phone.uid == first.uid)
        #expect(phone.secretKind == "pin" && phone.onPin && phone.weakPin)
        #expect(phone.weakReason == "pin" && phone.passwordNudge == .pin)

        let password = "olive-lantern-\(TestServer.run)"
        phone.changePassword(to: password, current: "582039")
        try await waitUntil("password set") { phone.pinChange == .saved }
        #expect(phone.secretKind == "password" && !phone.onPin && phone.passwordNudge == nil)

        let (stale, _) = device()
        stale.logIn(name: name, pin: "582039")
        try await waitUntil("the PIN is gone") { stale.authError != nil }
        #expect(stale.authError?.code == "bad_login")
        let (laptop, _) = device()
        laptop.logIn(name: name, pin: password)
        try await waitUntil("the password signs in") { laptop.phase == .online }
        #expect(laptop.secretKind == "password" && laptop.passwordNudge == nil)
    }

    /// Owner/mods reset a password: a temporary one (three words, two
    /// digits) comes back; signing in with it asks for your own, the server
    /// saying why (`ready.weakReason`), also after a relaunch; a member
    /// can't reset anyone's.
    @Test func aResetHandsOutATemporaryPassword() async throws {
        let (host, _) = try await mod("Rex \(TestServer.run)")
        let name = "Wren \(TestServer.run)"
        let (wren, _) = try await member(name)
        let (bystander, _) = try await member("Val \(TestServer.run)")

        bystander.notice = nil
        bystander.resetPassword(for: wren.uid)
        try await waitUntil("a member can't reset") { bystander.notice != nil }
        #expect(bystander.pinReset == nil)

        host.resetPassword(for: wren.uid)
        try await waitUntil("the temporary password") { host.pinReset != nil }
        let temporary = try #require(host.pinReset?.pin)
        #expect(host.pinReset?.name == name)
        #expect(temporary.wholeMatch(of: /[a-z]+-[a-z]+-[a-z]+-[0-9]{2}/) != nil, "\(temporary)")

        let (phone, phoneStore) = device()
        phone.logIn(name: name, pin: temporary)
        try await waitUntil("in with the temporary password") { phone.phase == .online }
        #expect(phone.weakPin && phone.weakReason == "temporary" && phone.passwordNudge == .temporary)
        // Still temporary after a relaunch: the same device, its stored token.
        let relaunched = YapClient(credentials: phoneStore, client: "test-ios")
        relaunched.start()
        try await waitUntil("relaunched") { relaunched.phase == .online }
        #expect(relaunched.passwordNudge == .temporary)

        phone.changePassword(to: "quiet-harbor-\(TestServer.run)", current: temporary)
        try await waitUntil("own password") { phone.pinChange == .saved }
        #expect(phone.passwordNudge == nil)
        // The next sign-in on this device isn't asked again: it's your own password now.
        let again = YapClient(credentials: phoneStore, client: "test-ios")
        again.start()
        try await waitUntil("relaunched again") { again.phase == .online }
        #expect(!again.weakPin && again.weakReason == nil && again.passwordNudge == nil)
    }

    @Test func signInPassForTheWebApp() async throws {
        // Draw / Space open the web app with a one-time pass instead of name + password.
        let (phone, phoneStore) = try await member("Lux \(TestServer.run)")
        let pass = try #require(await phone.handoffPass())
        #expect(pass != phoneStore.token)

        // The web view says hello with the pass: in as the same person, with its own lasting token.
        let (web, webStore) = device(token: pass)
        web.start()
        try await waitUntil("signed in with the pass") { web.phase == .online }
        #expect(web.uid == phone.uid)
        #expect(webStore.token != nil && webStore.token != pass, "the pass is swapped for a real token")
        #expect(phone.signinAlert == nil, "no new-sign-in alert for your own app")

        // Spent: a second hello with it is refused.
        try await Task.sleep(for: .milliseconds(300))
        let (again, _) = device(token: pass)
        again.start()
        try await waitUntil("a spent pass is refused") { again.phase == .signedOut }
        #expect(again.authError?.code == "bad_token")
    }

    @Test func searchPinsAndJumpingBack() async throws {
        let tag = "zq\(TestServer.run)"
        let adaName = "Lia \(TestServer.run)"
        let (ada, _) = try await member(adaName)
        let (ben, _) = try await member("Max \(TestServer.run)")
        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("loaded") { ada.histories["general"]?.loading == false && ben.histories["general"]?.loading == false }
        ada.send(text: "movie night \(tag) friday", in: "general")
        ben.send(text: "\(tag) link https://example.com/\(tag)", in: "general")
        try await waitUntil("both arrive") { (ada.histories["general"]?.messages.filter { $0.body.contains(tag) }.count ?? 0) == 2 }

        ada.runSearch(tag, in: "general")
        try await waitUntil("results") { ada.search?.loading == false }
        #expect(ada.search?.messages.count == 2)
        #expect(ada.search?.messages.first?.uid == ben.uid, "newest first")
        ada.runSearch("\(tag) from:\(adaName.prefix(3))", in: nil)
        try await waitUntil("from: results") { ada.search?.loading == false }
        #expect(ada.search?.messages.map(\.uid) == [ada.uid])
        ada.runSearch("\(tag) has:link", in: nil)
        try await waitUntil("has:link results") { ada.search?.loading == false }
        #expect(ada.search?.messages.map(\.uid) == [ben.uid])

        // Pins: live to everyone, listed, carried in history.
        // This run's own message: earlier runs on the same dev server left theirs too.
        let movie = try #require(ada.histories["general"]?.messages.last { $0.body.contains("movie night \(tag)") })
        ben.loadPins("general")
        try await waitUntil("pins loaded") { ben.pins["general"] != nil }
        ada.pin(movie, on: true)
        try await waitUntil("ben sees the pin") { ben.histories["general"]?.messages.first { $0.id == movie.id }?.pinnedAt != nil }
        try await waitUntil("and in his list") { ben.pins["general"]?.first?.id == movie.id }
        ben.pin(movie, on: false)
        try await waitUntil("unpinned for ada") { ada.histories["general"]?.messages.first { $0.id == movie.id }?.pinnedAt == nil }

        // Jumping back: tiny pages, so the first message is several pages away.
        let old = YapClient.historyPage
        YapClient.historyPage = 2
        defer { YapClient.historyPage = old }
        for i in 1...6 { ben.send(text: "filler \(i) \(tag)", in: "general") }
        try await waitUntil("fillers arrive") { (ada.histories["general"]?.messages.filter { $0.body.hasPrefix("filler") && $0.body.contains(tag) }.count ?? 0) == 6 }
        let (late, _) = device()
        late.logIn(name: adaName, pin: TestServer.password)
        try await waitUntil("late device") { late.phase == .online }
        late.open(channel: "general")
        try await waitUntil("first page") { late.histories["general"]?.loading == false }
        #expect(late.histories["general"]?.messages.contains { $0.id == movie.id } == false, "not on the first page")
        #expect(await late.ensureLoaded(movie.id, in: "general"))
        #expect(late.histories["general"]?.messages.contains { $0.id == movie.id } == true)
        #expect(await late.ensureLoaded("no-such-message", in: "general", maxPages: 3) == false)
    }
}
