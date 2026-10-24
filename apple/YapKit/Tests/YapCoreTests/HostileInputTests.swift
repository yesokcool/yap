import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers
@testable import YapCore

/// What another member, or a broken or hostile server, can send: never a
/// crash (each of the numbers here trapped before), and the server's newer
/// replies understood (security review, 2026-10-24).
@MainActor
@Suite("Hostile input")
struct HostileInputTests {
    // MARK: Numbers that trapped

    /// The server relays `signal.data` untouched: one member's candidate
    /// with sdpMLineIndex 3000000000 crashed every Apple app in the call.
    @Test func candidateMLineIndex() throws {
        func index(_ i: Int?) -> Int32? { IceCandidatePayload(candidate: "candidate:1", sdpMid: "0", sdpMLineIndex: i).mLineIndex }
        #expect(index(nil) == 0, "absent: the first m-line, as before")
        #expect(index(0) == 0 && index(2) == 2 && index(63) == 63)
        #expect(index(64) == nil && index(-1) == nil && index(3_000_000_000) == nil && index(.max) == nil && index(.min) == nil)
        let frame = #"{"t":"signal","from":"s2","data":{"candidate":{"candidate":"candidate:1 1 udp 1 1.2.3.4 9 typ host","sdpMid":"0","sdpMLineIndex":3000000000}}}"#
        guard case let .signal(_, data) = try ServerMessage.decode(frame), case let .candidate(c)? = data else {
            Issue.record("signal"); return
        }
        #expect(c.mLineIndex == nil, "dropped, not trapped on")
        // Not a whole number: the signal is dropped as undecodable.
        guard case let .signal(_, odd) = try ServerMessage.decode(#"{"t":"signal","from":"s2","data":{"candidate":{"candidate":"x","sdpMLineIndex":1e30}}}"#) else {
            Issue.record("signal"); return
        }
        #expect(odd == nil)
    }

    @Test func timesAndCountsAreClampedWhereTheyreDecoded() throws {
        let hangs = try JSONDecoder().decode([Hang].self, from: Data(#"[{"id":"h","at":-9223372036854775808,"createdAt":9223372036854775807}]"#.utf8))
        #expect(hangs[0].at == -WireNumber.maxMillis && hangs[0].createdAt == WireNumber.maxMillis)
        #expect(!hangs[0].isOn(), "`ms - at` used to overflow")
        let message = try JSONDecoder().decode(Message.self, from: Data(#"{"id":"m","channelId":"g","uid":"u","body":"","ts":-9223372036854775808,"editedAt":9223372036854775807}"#.utf8))
        #expect(message.ts == -WireNumber.maxMillis && message.editedAt == WireNumber.maxMillis)
        let counts = try JSONDecoder().decode([String: UnreadCount].self, from: Data(#"{"a":{"n":9223372036854775807,"mentions":9223372036854775807},"b":{"n":-5,"mentions":1}}"#.utf8))
        #expect(counts["a"]?.mentions == 1_000_000_000 && counts["a"]?.n == 1_000_000_000 && counts["b"]?.n == 0)
        // The sidebar's badges summed for the icon: no overflow.
        #expect(counts.values.reduce(0) { $0 + ($1.mentions ?? 0) } == 1_000_000_001)
        let away = try JSONDecoder().decode(Away.self, from: Data(#"{"text":"brb","until":-9223372036854775808}"#.utf8))
        #expect(away.until == -WireNumber.maxMillis && !away.isOn())
    }

    @Test func aRollsTotalNeverTraps() throws {
        let roll = try JSONDecoder().decode(Roll.self, from: Data(#"{"kind":"roll","spec":"2d6","rolls":[9223372036854775807,1]}"#.utf8))
        #expect(roll.rolls.count == 2)
        let fair = try JSONDecoder().decode(Roll.self, from: Data(#"{"kind":"roll","spec":"2d6","rolls":[3,4]}"#.utf8))
        #expect(fair.total == 7)
    }

    /// `UInt32(iterations)` trapped past 4294967295, and a few billion would
    /// hang the unlock for hours: at most the server's 10 million.
    @Test func aVaultWrapsStretching() throws {
        let (wrap, _) = try Vault.create(passcode: "correct horse", iterations: 1_000)
        _ = try Vault.open(wrap, passcode: "correct horse")
        for iter in [0, 10_000_001, 4_294_967_296, Int.max, -1] {
            var bad = wrap
            bad.iter = iter
            #expect(throws: Vault.Failure.badWrap) { try Vault.open(bad, passcode: "correct horse") }
        }
    }

    /// AVAudioFormat aborts (an exception, not an error) at a few MHz.
    @Test func aClipsSampleRate() {
        let tone = (0..<2_400).map { Float(sin(Double($0) / 8)) * 0.5 }
        #expect(WAV.decode(WAV.encode(tone, sampleRate: 48_000))?.sampleRate == 48_000)
        #expect(WAV.decode(WAV.encode(tone, sampleRate: 8_000))?.sampleRate == 8_000)
        for rate in [3_000_000, 7_999, 1] {
            var wav = WAV.encode(tone, sampleRate: 48_000)
            // The fmt chunk's sample rate (byte 24, little-endian).
            withUnsafeBytes(of: UInt32(rate).littleEndian) { wav.replaceSubrange(24..<28, with: $0) }
            #expect(WAV.decode(wav) == nil, "\(rate) Hz")
        }
    }

    // MARK: Strings

    @Test func renamesInTheModLog() {
        func say(_ detail: String?) -> String { ModLogEntry(ts: 1, actor: "u1", action: "member.rename", target: "u1", detail: detail).phrase { _ in "Ada" } }
        #expect(say(#""Ada" → "Ada L.""#) == "changed their name from “Ada” to “Ada L.”")
        #expect(say(#""" → "x""#) == "changed their name from “” to “x”")
        #expect(say(#"" → ""#) == "changed their name", "the arrow's own quotes aren't names (this range trapped)")
        for odd in [nil, "", "\"", "\"\"", "Ada → Bea", "\" → \" → \"", "\"a\" → "] {
            _ = say(odd)
        }
    }

    // MARK: The account

    @Test func aNewPasswordSaysHowManyDevicesWent() throws {
        guard case let .account(_, _, two) = try ServerMessage.decode(#"{"t":"account","hasPin":true,"kind":"password","signedOut":2}"#),
              case let .account(_, _, odd) = try ServerMessage.decode(#"{"t":"account","hasPin":true,"kind":"password","signedOut":"lots"}"#) else {
            Issue.record("account"); return
        }
        #expect(two == 2 && odd == nil)
        let client = YapClient(credentials: MemoryCredentialStore(), client: "test")
        client.receiveForTesting(.account(token: nil, kind: "password", signedOut: 3))
        #expect(client.pinChange == .saved && client.passwordSignedOut == 3)
        client.receiveForTesting(.account(token: nil, kind: "password"))
        #expect(client.passwordSignedOut == nil, "an older server doesn't say")
    }

    // MARK: Signed out by the server

    /// Closed with 4001 ("signed out"): no reconnecting to hear `bad_token`.
    @Test func closeCodes() {
        #expect(YapConnection.reason(status: nil, closeCode: 4001) == .signedOut)
        #expect(YapConnection.reason(status: nil, closeCode: 4003) == .badKey)
        #expect(YapConnection.reason(status: 401, closeCode: 0) == .badKey)
        #expect(YapConnection.reason(status: 101, closeCode: 1006) == .lost)
        #expect(YapConnection.reason(status: nil, closeCode: 4000) == .lost, "resumed elsewhere")
    }

    // MARK: Uploads the server turns away

    @Test(.serialized, arguments: [(429, "quota"), (413, "too_big"), (200, "quota")])
    func anUploadRefusalInTheServersWords(status: Int, code: String) async throws {
        FakeUploads.reply = (status, #"{"ok":false,"code":"\#(code)","message":"That's all the uploading for today (300 MB a day each). Try again tomorrow."}"#)
        let api = ServerAPI(endpoint: ServerEndpoint(baseURL: URL(string: "https://uploads.example.com")!, key: "k"), session: FakeUploads.session())
        do {
            _ = try await api.uploadImage(Data([1, 2, 3]), mime: "image/png", sid: "s", uploadToken: "t")
            Issue.record("accepted")
        } catch let error as ServerAPI.UploadError {
            #expect(error.code == code)
            #expect(error.localizedDescription == "That's all the uploading for today (300 MB a day each). Try again tomorrow.")
        }
    }

    // MARK: Pictures stay within what the server stores

    @Test func picturesFitTheServersLimits() throws {
        #expect(PictureEncoder.fitsServer(width: 8192, height: 4882, animated: false))
        #expect(!PictureEncoder.fitsServer(width: 8193, height: 10, animated: false), "8192 px a side")
        #expect(!PictureEncoder.fitsServer(width: 8000, height: 6000, animated: false), "40 megapixels")
        #expect(!PictureEncoder.fitsServer(width: 2049, height: 10, animated: true), "an animation: 2048 a side")
        #expect(!PictureEncoder.fitsServer(width: .max, height: .max, animated: false))
        for (w, h) in [(8064, 6048), (12_000, 100), (9000, 9000)] {
            let side = PictureEncoder.largestSide(width: w, height: h)
            let scale = Double(side) / Double(max(w, h))
            #expect(side <= 8192 && Double(w) * scale * Double(h) * scale <= 40_000_000, "\(w)×\(h)")
        }
        // A tiny file of a very wide picture: Full res redraws it within the limits.
        let wide = try #require(Self.png(width: 9000, height: 8))
        let prepared = try PictureEncoder.prepare(wide, full: true)
        #expect(max(prepared.width, prepared.height) <= 8192 && prepared.mime == "image/jpeg")
    }

    static func png(width: Int, height: Int) -> Data? {
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return nil }
        context.setFillColor(CGColor(red: 0.2, green: 0.5, blue: 0.8, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        guard let image = context.makeImage() else { return nil }
        let out = NSMutableData()
        guard let dest = CGImageDestinationCreateWithData(out, UTType.png.identifier as CFString, 1, nil) else { return nil }
        CGImageDestinationAddImage(dest, image, nil)
        return CGImageDestinationFinalize(dest) ? out as Data : nil
    }
}

/// Answers every upload with `reply` (status, JSON body).
final class FakeUploads: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var reply: (Int, String) = (200, "{}")

    static func session() -> URLSession {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [FakeUploads.self]
        return URLSession(configuration: config)
    }

    override class func canInit(with request: URLRequest) -> Bool { request.url?.host() == "uploads.example.com" }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let (status, body) = Self.reply
        let response = HTTPURLResponse(url: request.url!, statusCode: status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}
