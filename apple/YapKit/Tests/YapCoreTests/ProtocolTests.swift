import Foundation
import Testing
@testable import YapCore

@Suite("Wire protocol")
struct ProtocolTests {
    func json(_ message: ClientMessage) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: Data(message.frame().utf8)) as! [String: Any]
    }

    @Test func helloFormsMatchProtocol() throws {
        let token = try json(.hello(.token("tok"), client: "ios"))
        #expect(token["t"] as? String == "hello")
        #expect(token["v"] as? Int == 1)
        #expect(token["client"] as? String == "ios")
        #expect(token["token"] as? String == "tok")

        let login = try json(.hello(.login(name: "Ada", pin: "123456"), client: "ios"))
        #expect((login["login"] as? [String: String]) == ["name": "Ada", "pin": "123456"])
        #expect(login["token"] == nil)

        let signup = try json(.hello(.signup(name: "Ada", pin: "123456", color: "#5b6cff"), client: "ios"))
        #expect((signup["signup"] as? [String: String]) == ["name": "Ada", "pin": "123456", "color": "#5b6cff"])
    }

    @Test func clientMessagesEncode() throws {
        let send = try json(.send(channelId: "general", body: "hi", nonce: "n1", gif: nil))
        #expect(send["t"] as? String == "send")
        #expect(send["gif"] == nil)
        let history = try json(.history(channelId: "general", before: 5, beforeId: "x", limit: 50))
        #expect(history["before"] as? Int == 5)
        #expect(history["beforeId"] as? String == "x")
        let view = try json(.view(nil))
        #expect(view.keys.contains("view"))
        #expect(view["view"] is NSNull)
        let signal = try json(.signal(to: "s1", data: .candidate(IceCandidatePayload(candidate: "candidate:1", sdpMid: "0", sdpMLineIndex: 0))))
        let data = signal["data"] as? [String: Any]
        let candidate = data?["candidate"] as? [String: Any]
        #expect(candidate?["candidate"] as? String == "candidate:1")
        #expect(candidate?["sdpMLineIndex"] as? Int == 0)
        #expect(try json(.voiceJoin(channelId: "lounge", muted: true, deafened: false))["muted"] as? Bool == true)
        #expect(try json(.accountSignout)["t"] as? String == "account.signout")
        let react = try json(.react(id: "m1", emoji: "🎉", on: true))
        #expect(react["t"] as? String == "react" && react["emoji"] as? String == "🎉" && react["on"] as? Bool == true)
        #expect(try json(.ping(text: nil)).keys.sorted() == ["t"])
        let noAvatar = try json(.removeAvatar)
        #expect(noAvatar["t"] as? String == "profile" && noAvatar["avatar"] is NSNull)
    }

    @Test func readySnapshotDecodes() throws {
        let text = """
        {"t":"ready","v":1,"sid":"s1","uid":"u1","token":"tok","needsPin":false,"notify":{"mode":"off","topic":"x"},
         "channels":[{"id":"general","name":"general","kind":"text","position":0,"lastMessageAt":5},
                     {"id":"lounge","name":"Lounge","kind":"voice","position":0,"lastMessageAt":null},
                     {"id":"weird","name":"Future","kind":"stage","position":1,"lastMessageAt":null}],
         "users":[{"id":"u1","name":"Ada","color":"#5b6cff","lastSeen":1,"role":"owner","avatar":null},
                  {"id":"u2","name":"Ben","color":"#30a46c","lastSeen":1,"role":"superstar","avatar":"abc"}],
         "sessions":[{"sid":"s1","uid":"u1","client":"ios","voice":null,"view":null},
                     {"sid":"s2","uid":"u2","client":"web","voice":{"channelId":"lounge","muted":true,"deafened":false},"view":"board"}],
         "emojis":[{"id":"0123456789abcdef0123456789abcdef","name":"party","mime":"image/png","uid":"u1","createdAt":1}],
         "pending":[]}
        """
        guard case .ready(let ready) = try ServerMessage.decode(text) else { Issue.record("not ready"); return }
        #expect(ready.sid == "s1")
        #expect(ready.token == "tok")
        #expect(ready.channels.count == 3)
        #expect(ready.channels[2].kind == .other, "unknown channel kinds don't break decoding")
        #expect(ready.users[1].role == .member, "unknown roles fall back to member")
        #expect(ready.users[1].avatar == "abc")
        #expect(ready.sessions[1].voice?.muted == true)
        #expect(ready.emojis.first?.name == "party")
    }

    @Test func gifMessageDecodes() throws {
        let text = """
        {"t":"message","nonce":"n1","message":{"id":"m1","channelId":"general","uid":"u1","body":"","ts":1759600000000,
         "gif":{"provider":"klipy","slug":"hello-hi-662","title":"Hello","width":498,"height":280,
                "mp4":"https://static.klipy.com/ii/abc/MCCBoQlZ.mp4","webp":"https://static.klipy.com/ii/abc/eUbp2uNc.webp",
                "still":"https://static1.klipy.com/ii/abc/LyWpim71.jpg"}}}
        """
        guard case let .message(message, nonce) = try ServerMessage.decode(text) else { Issue.record("not a message"); return }
        #expect(nonce == "n1")
        let gif = try #require(message.gif)
        #expect(gif.slug == "hello-hi-662")
        #expect(gif.videoURL?.absoluteString == "https://static.klipy.com/ii/abc/MCCBoQlZ.mp4")
        #expect(gif.stillURL?.host() == "static1.klipy.com")
        #expect(abs(gif.aspectRatio - 498.0 / 280.0) < 0.0001)
        let fitted = gif.fittedSize()
        #expect(fitted.width == 300)
        #expect(fitted.height == 169)
        #expect(MessageFormatter.preview(message) == "GIF: Hello")
    }

    @Test func malformedGifKeepsTheMessage() throws {
        let text = #"{"t":"message","message":{"id":"m1","channelId":"general","uid":"u1","body":"hi","ts":1,"gif":"nope"}}"#
        guard case let .message(message, _) = try ServerMessage.decode(text) else { Issue.record("not a message"); return }
        #expect(message.body == "hi")
        #expect(message.gif == nil)
    }

    @Test func unknownAndIgnoredTypes() throws {
        for type in ["cursor", "draw.seg", "space.pos", "something.new"] {
            guard case .ignored(let t) = try ServerMessage.decode(#"{"t":"\#(type)","sid":"x"}"#) else {
                Issue.record("\(type) should be ignored")
                continue
            }
            #expect(t == type)
        }
    }

    @Test func signalPayloads() throws {
        let sdp = #"{"t":"signal","from":"s2","data":{"sdp":{"type":"offer","sdp":"v=0\r\n"}}}"#
        guard case let .signal(from, data) = try ServerMessage.decode(sdp) else { Issue.record("not a signal"); return }
        #expect(from == "s2")
        #expect(data == .sdp(SessionDescriptionPayload(type: "offer", sdp: "v=0\r\n")))
        let candidate = #"{"t":"signal","from":"s2","data":{"candidate":{"candidate":"candidate:1 1 udp 1 1.2.3.4 5 typ host","sdpMid":"0","sdpMLineIndex":0,"usernameFragment":"ab"}}}"#
        guard case let .signal(_, data2) = try ServerMessage.decode(candidate), case .candidate(let c)? = data2 else { Issue.record("no candidate"); return }
        #expect(c.usernameFragment == "ab")
        let junk = #"{"t":"signal","from":"s2","data":{"hello":1}}"#
        guard case let .signal(_, data3) = try ServerMessage.decode(junk) else { Issue.record("not a signal"); return }
        #expect(data3 == nil)
    }

    @Test func reactionsAndPendingDecode() throws {
        guard case let .reaction(messageId, channelId, emoji, uid, on) = try ServerMessage.decode(#"{"t":"reaction","messageId":"m","channelId":"c","emoji":"👍","uid":"u","on":true}"#) else {
            Issue.record("not a reaction")
            return
        }
        #expect([messageId, channelId, emoji, uid] == ["m", "c", "👍", "u"] && on)
        guard case let .pending(_, uid2, token) = try ServerMessage.decode(#"{"t":"pending","message":"wait","uid":"u9","token":"t"}"#) else {
            Issue.record("not pending")
            return
        }
        #expect(uid2 == "u9" && token == "t")
        guard case let .history(_, messages, _) = try ServerMessage.decode(#"{"t":"history","channelId":"c","hasMore":false,"messages":[{"id":"1","channelId":"c","uid":"u","body":"hi","ts":1,"reactions":[{"emoji":"🎉","uids":["a","b"]}]}]}"#) else {
            Issue.record("not history")
            return
        }
        #expect(messages.first?.reactions == [Reaction(emoji: "🎉", uids: ["a", "b"])])
    }

    @Test func minecraftStatusDecodes() throws {
        let json = #"{"name":"Minecraft","online":true,"address":"mc.example.com:25565","checkedAt":1791172655750,"ms":6,"version":"26.2","motd":"A Minecraft Server","players":{"online":0,"max":20,"names":[]}}"#
        let status = try JSONDecoder().decode(MinecraftStatus.self, from: Data(json.utf8))
        #expect(status.summary == "0/20 playing · 26.2")
        let offline = try JSONDecoder().decode(MinecraftStatus.self, from: Data(#"{"name":"Minecraft","online":false,"address":"a:1","checkedAt":1}"#.utf8))
        #expect(offline.summary == "Offline")
    }

    /// Minecraft and the board are optional server settings: a server without
    /// them leaves them out of `/api/config`, and that's not an error.
    @Test func configWithoutOptionalSettingsDecodes() throws {
        let bare = try JSONDecoder().decode(ServerConfig.self, from: Data(#"{"gifs":null,"links":[],"push":null}"#.utf8))
        #expect(bare.minecraft == nil && bare.gifs == nil)
        #expect(try JSONDecoder().decode(ServerConfig.self, from: Data("{}".utf8)) == ServerConfig())
    }

    /// One malformed Place doesn't empty the sidebar.
    @Test func aBadPlaceLeavesTheRest() throws {
        guard case .links(let links) = try ServerMessage.decode(#"{"t":"links","links":[{"id":"a","name":"Wiki","url":"https://wiki.example.com"},{"name":"no id"},{"id":"b","name":"Board","url":"","view":"board"}]}"#) else {
            Issue.record("links"); return
        }
        #expect(links.map(\.id) == ["a", "b"])
        guard case .ready(let ready) = try ServerMessage.decode(#"{"t":"ready","sid":"s","uid":"u","links":[{"id":"a","name":"Wiki","url":"https://wiki.example.com"},42]}"#) else {
            Issue.record("ready"); return
        }
        #expect(ready.links.map(\.id) == ["a"])
    }

    @Test func errorsDecode() throws {
        guard case let .error(code, message, re) = try ServerMessage.decode(#"{"t":"error","code":"locked","message":"Too many wrong tries. Try again in 2 min.","re":"hello"}"#) else {
            Issue.record("not an error")
            return
        }
        #expect(code == "locked")
        #expect(re == "hello")
        #expect(message.contains("2 min"))
    }

    @Test func inviteLinksParse() {
        let live = ServerEndpoint.parse(invite: "https://chat.example.com/#key=abc123")
        #expect(live?.baseURL.absoluteString == "https://chat.example.com")
        #expect(live?.key == "abc123")
        let dev = ServerEndpoint.parse(invite: " http://127.0.0.1:8789/#key=testkey\n")
        #expect(dev?.baseURL.absoluteString == "http://127.0.0.1:8789")
        #expect(dev?.webSocketURL.absoluteString == "ws://127.0.0.1:8789/ws", "the key never goes in a URL")
        #expect(dev?.webSocketProtocols == ["yap", "key.testkey"])
        #expect(dev?.request("/api/check").value(forHTTPHeaderField: "X-Yap-Key") == "testkey")
        #expect(ServerEndpoint.parse(invite: "https://chat.example.com/?key=q")?.key == "q")
        #expect(ServerEndpoint.parse(invite: "https://chat.example.com/") == nil)
        #expect(ServerEndpoint.parse(invite: "") == nil)
        #expect(live?.inviteLink == "https://chat.example.com/#key=abc123")
        #expect(live?.url("/api/config").absoluteString == "https://chat.example.com/api/config")
        #expect(live?.emojiURL("0123").absoluteString == "https://chat.example.com/emoji/0123")
        // No default server: a bare key says nothing about where to connect.
        #expect(ServerEndpoint.parse(invite: "bareKey42") == nil)
        #expect(ServerEndpoint.parse(invite: "chat.example.com") == nil, "a server without a key")
        // Without a scheme, https is assumed (the welcome screen's example).
        let typed = ServerEndpoint.parse(invite: "chat.example.com/#key=abc123")
        #expect(typed?.baseURL.absoluteString == "https://chat.example.com" && typed?.key == "abc123")
        #expect(ServerEndpoint.parse(invite: "chat.example.com:8443/?key=q")?.baseURL.absoluteString == "https://chat.example.com:8443")
    }

    @Test func friendlyAuthErrors() {
        // A wrong sign-in in the server's words, as the web shows it (the sign-on screen adds who can help).
        #expect(YapClient.friendly(code: "bad_login", message: "Wrong name or password.", attempted: nil) == "Wrong name or password.")
        let taken = YapClient.friendly(code: "name_taken", message: "x", attempted: .signup(name: "Ada", pin: "correct-horse-582039", color: "#000000"))
        #expect(taken.contains("already called Ada") && taken.hasSuffix("sign in with your password."))
        #expect(YapClient.friendly(code: "bad_token", message: "x", attempted: nil) == "This device was signed out. Sign in again with your name and password.")
        // The server's own words for the password rules and lockouts.
        #expect(YapClient.friendly(code: "locked", message: "Too many wrong tries. Try again in 4 min.", attempted: nil) == "Too many wrong tries. Try again in 4 min.")
        #expect(YapClient.friendly(code: "bad_pin", message: "Use a password of at least 8 characters.", attempted: nil) == "Use a password of at least 8 characters.")
        #expect(YapClient.friendly(code: "weak_pin", message: "That password is too easy to guess.", attempted: nil) == "That password is too easy to guess.")
        #expect(YapClient.friendly(code: "bad_pin", message: "", attempted: nil) == "Use a password of at least 8 characters.")
        for code in ["bad_token", "bad_login", "name_taken", "bad_pin", "weak_pin"] {
            #expect(!YapClient.friendly(code: code, message: "", attempted: .signup(name: "Ada", pin: "x", color: "#000000")).contains("PIN"), "\(code)")
        }
    }
}
