import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import YapCore

/// Frames and helpers the web client gained in v0.13–v0.17: edits, pictures,
/// personal invites, server Places, shared voice quality, OBS listeners,
/// the mod log, and the upload token.
@Suite("Protocol parity (v0.13–v0.17)")
struct ParityProtocolTests {
    func json(_ message: ClientMessage) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: Data(message.frame().utf8)) as! [String: Any]
    }

    @Test func newClientFramesEncode() throws {
        let edit = try json(.edit(id: "m1", body: "fixed"))
        #expect(edit["t"] as? String == "edit" && edit["id"] as? String == "m1" && edit["body"] as? String == "fixed")
        let picture = try json(.send(channelId: "general", body: "look", nonce: "n", gif: nil, image: ImageRef(id: "abc", width: 640, height: 480)))
        #expect((picture["image"] as? [String: Any])?["id"] as? String == "abc")
        #expect((picture["image"] as? [String: Any])?["width"] as? Int == 640)
        #expect(try json(.inviteCreate(label: "for Cleo"))["label"] as? String == "for Cleo")
        #expect(try json(.inviteList).keys.sorted() == ["t"])
        #expect(try json(.inviteRevoke(id: "i1"))["t"] as? String == "invite.revoke")
        let link = try json(.linkAdd(name: "Wiki", url: "https://example.com"))
        #expect(link["t"] as? String == "link.add" && link["url"] as? String == "https://example.com")
        #expect(try json(.linkRemove(id: "l1"))["id"] as? String == "l1")
        let shared = try json(.channelBitrate(channelId: "lounge", bitrate: 96_000))
        #expect(shared["t"] as? String == "channel.bitrate" && shared["bitrate"] as? Int == 96_000)
        let personal = try json(.channelBitrate(channelId: "lounge", bitrate: nil))
        #expect(personal.keys.contains("bitrate") && personal["bitrate"] is NSNull, "null means each person's own")
        #expect(try json(.modlog)["t"] as? String == "modlog")
    }

    @Test func personalInviteLinksStayReadable() {
        let link = ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "yi_Ab-9_z").inviteLink
        #expect(link == "https://chat.example.com/#key=yi_Ab-9_z")
        #expect(ServerEndpoint.parse(invite: link)?.key == "yi_Ab-9_z")
    }

    @Test func skinTones() throws {
        let wave = try #require(EmojiCatalog.shared.byCode["wave"])
        #expect(wave.skins.count == 5)
        #expect(wave.char(tone: 0) == "👋" && wave.char(tone: 3) == "👋🏽" && wave.char(tone: 9) == "👋")
        #expect(EmojiCodec.encode("hi :wave:", custom: [:], tone: 5) == "hi 👋🏿")
        #expect(EmojiCodec.encode("hi :wave:", custom: [:]) == "hi 👋")
        let tea = try #require(EmojiCatalog.shared.byCode["tea"] ?? EmojiCatalog.shared.byCode["coffee"])
        #expect(tea.skins.isEmpty && tea.char(tone: 4) == tea.char, "no tones, no change")
    }

    @Test func newServerFramesDecode() throws {
        guard case let .edited(id, channelId, body, editedAt, sealed) = try ServerMessage.decode(#"{"t":"edited","id":"m1","channelId":"general","body":"new","editedAt":7}"#) else {
            Issue.record("edited"); return
        }
        #expect(id == "m1" && channelId == "general" && body == "new" && editedAt == 7 && !sealed)

        guard case let .inviteCreated(invite, key) = try ServerMessage.decode(
            #"{"t":"invite.created","key":"yi_abc","invite":{"id":"i1","label":"for Cleo","by":"u1","createdAt":1,"uses":0,"lastUsedAt":null,"revoked":false}}"#) else {
            Issue.record("invite.created"); return
        }
        #expect(key == "yi_abc" && invite.label == "for Cleo" && !invite.revoked)

        guard case .invites(let invites) = try ServerMessage.decode(#"{"t":"invites","invites":[{"id":"i1","label":"x","by":"u1","createdAt":1,"uses":2,"lastUsedAt":3,"revoked":true}]}"#) else {
            Issue.record("invites"); return
        }
        #expect(invites.first?.uses == 2 && invites.first?.revoked == true)

        guard case .links(let links) = try ServerMessage.decode(#"{"t":"links","links":[{"id":"l1","name":"Game Board","url":"https://games.example.com","view":"board"},{"id":"l2","name":"Wiki","url":"https://example.com"}]}"#) else {
            Issue.record("links"); return
        }
        #expect(links.map(\.id) == ["l1", "l2"] && links[0].view == "board" && links[1].view == nil)

        guard case .modlog(let entries) = try ServerMessage.decode(#"{"t":"modlog","entries":[{"ts":1,"actor":"u1","action":"member.approve","target":"u2","detail":null}]}"#) else {
            Issue.record("modlog"); return
        }
        #expect(entries.first?.phrase { $0 == "u2" ? "Ben" : "?" } == "let Ben in")
    }

    /// The mod log reads like the web's (`LOG_WORDS` in web/js/app/settings-ui.js).
    @Test func modLogUsesTheWebsWords() {
        let ben: (String?) -> String = { $0 == "u2" ? "Ben" : "Someone" }
        func say(_ action: String, _ detail: String? = nil, actor: String = "u1", target: String? = "u2") -> String {
            ModLogEntry(ts: 1, actor: actor, action: action, target: target, detail: detail).phrase(ben)
        }
        #expect(say("member.role", "banned") == "banned Ben")
        #expect(say("member.role", "pending") == "sent Ben back to the waiting room")
        #expect(say("member.role", "mod") == "made Ben a mod")
        #expect(say("member.role", "member") == "made Ben a member again (no longer a mod)")
        #expect(say("member.role", "member", actor: "admin") == "made Ben a member")
        #expect(say("member.merge") == "merged a duplicate account into Ben's")
        #expect(say("channel.bitrate", "\"Lounge\" → 256 kbps") == "set the voice quality in \"Lounge\" to 256 kbps for everyone")
        #expect(say("channel.bitrate", "\"Lounge\" → personal settings") == "let everyone in \"Lounge\" pick their own voice quality again")
        #expect(say("link.add", "Wiki → example.com") == "added Wiki (example.com) to Places")
        #expect(say("link.remove", "Wiki") == "removed Wiki from Places")
        #expect(say("invite.create", "") == "made an invite link")
        #expect(say("obs.revoke", "3") == "turned off their 3 OBS links")
        #expect(say("sound.delete") == "deleted a soundboard clip by Ben")
        #expect(say("member.resetPin", target: "admin") == "reset the owner (admin key)'s PIN")
        #expect(say("space.reset_all", "x") == "did “space reset all” (x)")
    }

    @Test func readyCarriesKeyLinksAndUploadToken() throws {
        let text = """
        {"t":"ready","v":1,"sid":"s1","uid":"u1","needsPin":false,"channels":[{"id":"lounge","name":"Lounge","kind":"voice","position":0,"lastMessageAt":null,"bitrate":96000}],
         "users":[],"sessions":[{"sid":"s9","uid":"u1","client":"obs","voice":{"channelId":"lounge","muted":true,"deafened":false},"view":null}],
         "emojis":[],"pending":[{"id":"u3","name":"Zed","color":"#fff","lastSeen":0,"role":"pending","avatar":null,"invitedBy":"Ada (for Zed)"}],
         "key":"mainkey","links":[{"id":"l1","name":"Board","url":"https://x.example","view":"board"}],"uploadToken":"0123456789abcdef0123456789abcdef"}
        """
        guard case .ready(let ready) = try ServerMessage.decode(text) else { Issue.record("ready"); return }
        #expect(ready.key == "mainkey")
        #expect(ready.links.first?.view == "board")
        #expect(ready.uploadToken == "0123456789abcdef0123456789abcdef")
        #expect(ready.channels.first?.bitrate == 96_000)
        #expect(ready.pending.first?.invitedBy == "Ada (for Zed)")
        #expect(ready.sessions.first?.isListener == true)
    }

    @Test func personalSpaceDecodes() throws {
        guard case .channel(let channel) = try ServerMessage.decode(#"{"t":"channel","channel":{"id":"p_u1","name":"Personal space","kind":"personal","position":-1,"lastMessageAt":3}}"#) else {
            Issue.record("channel"); return
        }
        #expect(channel.kind == .personal && channel.isPersonal && channel.hasMessages)
        #expect(Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0).hasMessages == false)
    }

    @Test func messagesCarryPicturesAndEdits() throws {
        let text = #"{"t":"message","message":{"id":"m1","channelId":"general","uid":"u1","body":"","ts":5,"image":{"id":"abc","width":4000,"height":3000},"editedAt":9}}"#
        guard case .message(let m, _) = try ServerMessage.decode(text) else { Issue.record("message"); return }
        #expect(m.image == ImageRef(id: "abc", width: 4000, height: 3000))
        #expect(m.editedAt == 9)
        let fitted = try #require(m.image).fittedSize(maxWidth: 360, maxHeight: 360)
        #expect(fitted.width == 360 && fitted.height == 270)
        // A malformed picture never hides the message.
        guard case .message(let broken, _) = try ServerMessage.decode(#"{"t":"message","message":{"id":"m2","channelId":"general","uid":"u1","body":"hi","ts":5,"image":"nope"}}"#) else {
            Issue.record("broken"); return
        }
        #expect(broken.image == nil && broken.body == "hi")
    }

    @MainActor
    @Test func obsListenersDontCountAsOnline() throws {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t"), client: "test")
        let ada = User(id: "u1", name: "Ada", color: "#fff")
        let me = User(id: "me", name: "Me", color: "#fff")
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me", channels: [Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0)],
                                            users: [ada, me],
                                            sessions: [Session(sid: "s0", uid: "me", client: "ios"),
                                                       Session(sid: "s1", uid: "u1", client: "obs", voice: VoiceState(channelId: "lounge", muted: true, deafened: false))]),
                        histories: [:])
        #expect(client.members.map(\.id) == ["u1", "me"], "everyone, A–Z")
        #expect(client.voiceChannel(of: "u1") == nil, "a listener doesn't put them in voice")
        #expect(client.sessions(of: "u1").isEmpty)
        #expect(client.voiceSessions(in: "lounge").map(\.sid) == ["s1"], "still shown in the channel, as a listener")
    }
}

@Suite("Picture encoder")
struct PictureEncoderTests {
    static func image(width: Int, height: Int, type: UTType, noisy: Bool = false) -> Data {
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        for i in stride(from: 0, to: pixels.count, by: 4) {
            pixels[i] = noisy ? UInt8.random(in: 0...255) : UInt8((i / 4) % 255)
            pixels[i + 1] = noisy ? UInt8.random(in: 0...255) : 120
            pixels[i + 2] = noisy ? UInt8.random(in: 0...255) : 200
            pixels[i + 3] = 255
        }
        let provider = CGDataProvider(data: Data(pixels) as CFData)!
        let cg = CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                         space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
                         provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
        let out = NSMutableData()
        let dest = CGImageDestinationCreateWithData(out, type.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(dest, cg, nil)
        CGImageDestinationFinalize(dest)
        return out as Data
    }

    @Test func smallPicturesGoAsTheyAre() throws {
        let png = Self.image(width: 64, height: 32, type: .png)
        let prepared = try PictureEncoder.prepare(png)
        #expect(prepared.mime == "image/png" && prepared.data == png)
        #expect(prepared.width == 64 && prepared.height == 32)
    }

    @Test func bigPicturesShrinkToJPEG() throws {
        let big = Self.image(width: 3000, height: 2000, type: .png, noisy: true)
        #expect(big.count > ServerAPI.maxImageBytes, "the test picture really is too big")
        let prepared = try PictureEncoder.prepare(big)
        #expect(prepared.mime == "image/jpeg")
        #expect(max(prepared.width, prepared.height) <= PictureEncoder.maxSide)
        #expect(prepared.data.count <= ServerAPI.maxImageBytes)
        #expect(abs(Double(prepared.width) / Double(prepared.height) - 1.5) < 0.01, "keeps the aspect ratio")
    }

    @Test func heicBecomesJPEG() throws {
        let heic = Self.image(width: 400, height: 300, type: .heic)
        try #require(!heic.isEmpty, "this OS can write HEIC")
        let prepared = try PictureEncoder.prepare(heic)
        #expect(prepared.mime == "image/jpeg")
        #expect(prepared.data.starts(with: [0xFF, 0xD8]))
        #expect(prepared.width == 400 && prepared.height == 300)
    }

    @Test func notAPicture() {
        #expect(throws: PictureEncoder.Failure.unreadable) { try PictureEncoder.prepare(Data("hello".utf8)) }
    }

    /// A smooth "photo" (gradients, a little grain) as a JPEG.
    static func photo(width: Int, height: Int) -> Data {
        var pixels = [UInt8](repeating: 255, count: width * height * 4)
        for y in 0..<height {
            for x in 0..<width {
                let i = (y * width + x) * 4
                let grain = Int.random(in: -6...6)
                pixels[i] = UInt8(clamping: x * 255 / width + grain)
                pixels[i + 1] = UInt8(clamping: y * 255 / height + grain)
                pixels[i + 2] = UInt8(clamping: 128 + grain)
            }
        }
        let provider = CGDataProvider(data: Data(pixels) as CFData)!
        let cg = CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                         space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
                         provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
        let out = NSMutableData()
        let dest = CGImageDestinationCreateWithData(out, UTType.jpeg.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(dest, cg, [kCGImageDestinationLossyCompressionQuality: 0.9] as CFDictionary)
        CGImageDestinationFinalize(dest)
        return out as Data
    }

    /// The default: ≤ 1600 px JPEG, small. Full res: the original, untouched.
    @Test func compressedByDefaultOriginalWithFullRes() throws {
        let original = Self.photo(width: 2400, height: 1800)
        try #require(original.count <= ServerAPI.maxImageBytes, "the test photo fits as it is")
        let compressed = try PictureEncoder.prepare(original)
        #expect(compressed.mime == "image/jpeg" && max(compressed.width, compressed.height) == 1600)
        #expect(compressed.data.count < original.count && compressed.data.count < 400_000, "\(compressed.data.count) bytes")
        let full = try PictureEncoder.prepare(original, full: true)
        #expect(full.data == original && full.width == 2400 && full.height == 1800)
    }

    /// Full res of a picture that's too big: the sharpest copy that fits.
    @Test func fullResFitsWhenTheOriginalCant() throws {
        let big = Self.image(width: 3000, height: 2000, type: .png, noisy: true)
        #expect(big.count > ServerAPI.maxImageBytes)
        let full = try PictureEncoder.prepare(big, full: true)
        let compressed = try PictureEncoder.prepare(big)
        #expect(full.mime == "image/jpeg" && full.data.count <= ServerAPI.maxImageBytes)
        #expect(max(full.width, full.height) >= max(compressed.width, compressed.height))
    }

    @Test func heicBecomesJPEGEvenAtFullRes() throws {
        let heic = Self.image(width: 400, height: 300, type: .heic)
        try #require(!heic.isEmpty, "this OS can write HEIC")
        let full = try PictureEncoder.prepare(heic, full: true)
        #expect(full.mime == "image/jpeg" && full.width == 400 && full.height == 300)
    }

    @Test func smallGIFsStayAsTheyAre() throws {
        let gif = Self.image(width: 40, height: 30, type: .gif)
        for full in [false, true] {
            let prepared = try PictureEncoder.prepare(gif, full: full)
            #expect(prepared.mime == "image/gif" && prepared.data == gif)
        }
    }

    @Test func sizeLabels() {
        #expect(PictureEncoder.sizeLabel(47_000) == "47 KB")
        #expect(PictureEncoder.sizeLabel(300) == "1 KB")
        #expect(PictureEncoder.sizeLabel(1_234_567) == "1.2 MB")
    }
}
