import CryptoKit
import Foundation
import Testing
@testable import YapCore

/// The encrypted Personal space's crypto: round trips here, and both
/// directions against the web's own `web/js/vault.js` run in Node.
@Suite("Encrypted Personal space")
struct VaultTests {
    static let fast = 2_000 // the real thing uses 600k; tests don't need to wait

    @Test func noteRoundTripsAndGivesNothingAway() throws {
        let (wrap, key) = try Vault.create(passcode: "purple tiger river", iterations: Self.fast)
        let body = try Vault.seal(Vault.Note(text: "wifi at the cabin is on the fridge"), with: key)
        #expect(Vault.isEncryptedNote(body))
        #expect(!body.contains("cabin"))
        #expect(try Vault.open(note: body, with: key).text == "wifi at the cabin is on the fridge")
        // Another device: same passcode, same key.
        let again = try Vault.open(wrap, passcode: "purple tiger river")
        #expect(try Vault.open(note: body, with: again).text == "wifi at the cabin is on the fridge")
    }

    @Test func wrapShape() throws {
        let (wrap, _) = try Vault.create(passcode: "purple tiger river", iterations: Self.fast)
        #expect(wrap.v == 1 && wrap.iter == Self.fast)
        #expect(Data(base64URL: wrap.salt)?.count == 16)
        #expect(Data(base64URL: wrap.iv)?.count == 12)
        #expect(Data(base64URL: wrap.ct)?.count == 48)
        #expect(!wrap.salt.contains("=") && !wrap.ct.contains("+") && !wrap.ct.contains("/"))
    }

    @Test func wrongPasscodeAndTampering() throws {
        let (wrap, key) = try Vault.create(passcode: "purple tiger river", iterations: Self.fast)
        #expect(throws: Vault.Failure.wrongPasscode) { try Vault.open(wrap, passcode: "purple tiger rivers") }
        let body = try Vault.seal(Vault.Note(text: "hi"), with: key)
        var chars = Array(body)
        chars[chars.count - 2] = chars[chars.count - 2] == "A" ? "B" : "A"
        #expect(throws: Vault.Failure.cantOpen) { try Vault.open(note: String(chars), with: key) }
        let other = try Vault.create(passcode: "another one entirely", iterations: Self.fast).key
        #expect(throws: Vault.Failure.cantOpen) { try Vault.open(note: body, with: other) }
    }

    @Test func newPasscodeSameKey() throws {
        let (wrap, key) = try Vault.create(passcode: "purple tiger river", iterations: Self.fast)
        let body = try Vault.seal(Vault.Note(text: "kept"), with: key)
        let next = try Vault.rewrap(wrap, old: "purple tiger river", new: "green ocean lamp")
        #expect(throws: Vault.Failure.wrongPasscode) { try Vault.open(next, passcode: "purple tiger river") }
        #expect(try Vault.open(note: body, with: Vault.open(next, passcode: "green ocean lamp")).text == "kept")
        #expect(next.iter == Vault.iterations, "a rewrap uses at least the real iteration count")
    }

    @Test func picturesAndNotePayloads() throws {
        let key = try Vault.create(passcode: "purple tiger river", iterations: Self.fast).key
        let bytes = Data((0..<5000).map { _ in UInt8.random(in: 0...255) })
        let sealed = try Vault.seal(blob: bytes, with: key)
        #expect(sealed.count == 12 + 5000 + 16)
        #expect(try Vault.open(blob: sealed, with: key) == bytes)
        let gif = Gif(slug: "x", title: "t", width: 1, height: 1, mp4: "https://static.klipy.com/a.mp4")
        let note = Vault.Note(text: "look", gif: gif, image: Vault.SealedImage(id: String(repeating: "a", count: 32), width: 10, height: 20, mime: "image/webp"))
        #expect(try Vault.open(note: Vault.seal(note, with: key), with: key) == note)
    }

    @Test func passcodesAreNormalized() throws {
        // "é" precomposed vs. "e" + combining accent: same passcode after NFKC.
        let (wrap, _) = try Vault.create(passcode: "caf\u{E9} au lait", iterations: Self.fast)
        _ = try Vault.open(wrap, passcode: "cafe\u{301} au lait")
    }

    @Test func passcodeRules() {
        #expect(Vault.problem(newPasscode: "short", again: "short") != nil)
        #expect(Vault.problem(newPasscode: "12345678901", again: "12345678901")?.contains("Numbers") == true)
        #expect(Vault.problem(newPasscode: "123456789012", again: "123456789012") == nil)
        #expect(Vault.problem(newPasscode: "purple tiger", again: "purple tigre")?.contains("match") == true)
        #expect(Vault.problem(newPasscode: "purple tiger", again: "purple tiger") == nil)
    }

    @Test func plainNotesStayPlain() {
        #expect(!Vault.isEncryptedNote("e1:"))
        #expect(!Vault.isEncryptedNote("hello"))
        #expect(!Vault.isEncryptedNote("e1:not base64!"))
    }

    // MARK: Cross-check with web/js/vault.js (macOS: needs Node)
    #if os(macOS)

    static var vaultJS: URL {
        URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appending(path: "../../../../web/js/vault.js").standardizedFileURL
    }

    static var node: String? {
        ["/opt/homebrew/bin/node", "/usr/local/bin/node", "/usr/bin/node"].first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// Runs an ES module in Node with `vault.js` imported as `V`; returns stdout.
    static func runNode(_ script: String, input: String = "") throws -> String {
        let dir = FileManager.default.temporaryDirectory.appending(path: "yap-vault-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = dir.appending(path: "check.mjs")
        try ("import * as V from \(String(reflecting: vaultJS.absoluteString));\n" + script).write(to: file, atomically: true, encoding: .utf8)
        let process = Process()
        process.executableURL = URL(fileURLWithPath: node!)
        process.arguments = [file.path]
        let stdin = Pipe(), stdout = Pipe(), stderr = Pipe()
        process.standardInput = stdin
        process.standardOutput = stdout
        process.standardError = stderr
        try process.run()
        stdin.fileHandleForWriting.write(Data(input.utf8))
        try stdin.fileHandleForWriting.close()
        let out = stdout.fileHandleForReading.readDataToEndOfFile()
        let err = stderr.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw NSError(domain: "node", code: Int(process.terminationStatus), userInfo: [NSLocalizedDescriptionKey: String(decoding: err, as: UTF8.self)])
        }
        return String(decoding: out, as: UTF8.self)
    }

    struct Exchange: Codable {
        var wrap: VaultWrap
        var note: String
        var blob: String
        var passcode: String
    }

    @Test(.enabled(if: VaultTests.node != nil && FileManager.default.fileExists(atPath: VaultTests.vaultJS.path)))
    func swiftSealsWebOpens() throws {
        let passcode = "caf\u{E9} purple tiger 42"
        let (wrap, key) = try Vault.create(passcode: passcode, iterations: Self.fast)
        let gif = Gif(slug: "hi", title: "Hi", width: 2, height: 1, mp4: "https://static.klipy.com/hi.mp4")
        let note = try Vault.seal(Vault.Note(text: "from swift 🔒 <:yap:0123456789abcdef0123456789abcdef>", gif: gif,
                                             image: Vault.SealedImage(id: String(repeating: "b", count: 32), width: 640, height: 480, mime: "image/jpeg")),
                                  with: key)
        let blob = try Vault.seal(blob: Data("picture bytes".utf8), with: key).base64URL
        // The web normalizes too, so the decomposed form must open it.
        let input = try JSONEncoder().encode(Exchange(wrap: wrap, note: note, blob: blob, passcode: "cafe\u{301} purple tiger 42"))
        let out = try Self.runNode("""
        const input = JSON.parse(await new Promise((r) => { let s = ''; process.stdin.on('data', (d) => (s += d)); process.stdin.on('end', () => r(s)); }));
        const key = await V.openVault(input.wrap, input.passcode);
        const note = await V.decryptNote(key, input.note);
        const blob = new TextDecoder().decode(await V.decryptBlob(key, V.unb64u(input.blob)));
        console.log(JSON.stringify({ note, blob, encrypted: V.isEncryptedNote(input.note) }));
        """, input: String(decoding: input, as: UTF8.self))
        let result = try JSONSerialization.jsonObject(with: Data(out.utf8)) as? [String: Any]
        let opened = result?["note"] as? [String: Any]
        #expect(opened?["text"] as? String == "from swift 🔒 <:yap:0123456789abcdef0123456789abcdef>")
        #expect((opened?["gif"] as? [String: Any])?["slug"] as? String == "hi")
        #expect((opened?["image"] as? [String: Any])?["mime"] as? String == "image/jpeg")
        #expect(result?["blob"] as? String == "picture bytes")
        #expect(result?["encrypted"] as? Bool == true)
    }

    @Test(.enabled(if: VaultTests.node != nil && FileManager.default.fileExists(atPath: VaultTests.vaultJS.path)))
    func webSealsSwiftOpens() throws {
        let out = try Self.runNode("""
        const passcode = 'green ocean lamp ﬁ';
        const { wrap, key } = await V.createVault(passcode, \(Self.fast));
        const note = await V.encryptNote(key, { text: 'from the web', image: { id: 'c'.repeat(32), width: 3, height: 4, mime: 'image/webp' } });
        const blob = V.b64u(await V.encryptBlob(key, new TextEncoder().encode('web picture')));
        const next = await V.rewrapVault(wrap, passcode, 'second passcode here');
        console.log(JSON.stringify({ wrap, note, blob, passcode, next }));
        """)
        struct WebOut: Codable { var wrap: VaultWrap; var note: String; var blob: String; var passcode: String; var next: VaultWrap }
        let web = try JSONDecoder().decode(WebOut.self, from: Data(out.utf8))
        // "ﬁ" (a ligature) normalizes to "fi" under NFKC on both sides.
        let key = try Vault.open(web.wrap, passcode: "green ocean lamp fi")
        let note = try Vault.open(note: web.note, with: key)
        #expect(note.text == "from the web")
        #expect(note.image == Vault.SealedImage(id: String(repeating: "c", count: 32), width: 3, height: 4, mime: "image/webp"))
        #expect(try Vault.open(blob: try #require(Data(base64URL: web.blob)), with: key) == Data("web picture".utf8))
        // The web's rewrap: same key under a new passcode.
        let again = try Vault.open(web.next, passcode: "second passcode here")
        #expect(try Vault.open(note: web.note, with: again).text == "from the web")
        #expect(throws: Vault.Failure.wrongPasscode) { try Vault.open(web.next, passcode: web.passcode) }
    }
    #endif
}
