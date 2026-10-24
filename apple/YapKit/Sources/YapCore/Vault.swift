import CommonCrypto
import CryptoKit
import Foundation

/// The encrypted Personal space's crypto (PROTOCOL.md › Encrypted Personal
/// space), byte-compatible with the web's `web/js/vault.js`:
///
///     space key K   32 random bytes, AES-256-GCM
///     wrap          KEK = PBKDF2-HMAC-SHA256(NFKC passcode, salt(16), iter) → 32 bytes;
///                   ct = AES-GCM(KEK, iv(12), K, AAD "yap:wrap:v1") = 48 bytes
///     note          "e1:" + b64url(iv ‖ AES-GCM(K, iv, JSON {text, gif?, image?}, AAD "yap:note:v1"))
///     picture       iv ‖ AES-GCM(K, iv, bytes, AAD "yap:blob:v1")
///
/// Everything is base64url without padding. Nothing here touches the
/// network or the Keychain.
public enum Vault {
    /// What the web uses when it makes a wrap; reading takes whatever `iter` says.
    public static let iterations = 600_000

    static let wrapAAD = Data("yap:wrap:v1".utf8)
    static let noteAAD = Data("yap:note:v1".utf8)
    static let blobAAD = Data("yap:blob:v1".utf8)

    public enum Failure: Error, LocalizedError, Equatable {
        case wrongPasscode
        case cantOpen
        case badWrap

        public var errorDescription: String? {
            switch self {
            case .wrongPasscode: "That passcode doesn’t open it."
            case .cantOpen: "This note was locked with another passcode."
            case .badWrap: "The stored key looks broken."
            }
        }
    }

    /// The decrypted contents of an encrypted note.
    public struct Note: Codable, Sendable, Hashable {
        public var text: String
        public var gif: Gif?
        public var image: SealedImage?

        public init(text: String, gif: Gif? = nil, image: SealedImage? = nil) {
            self.text = text
            self.gif = gif
            self.image = image
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            text = try c.decodeIfPresent(String.self, forKey: .text) ?? ""
            gif = try? c.decodeIfPresent(Gif.self, forKey: .gif)
            image = try? c.decodeIfPresent(SealedImage.self, forKey: .image)
        }
    }

    /// An encrypted picture: the blob's media id, size and type.
    public struct SealedImage: Codable, Sendable, Hashable {
        public var id: String
        public var width: Int
        public var height: Int
        public var mime: String?

        public init(id: String, width: Int, height: Int, mime: String?) {
            self.id = id
            self.width = width
            self.height = height
            self.mime = mime
        }

        public var ref: ImageRef { ImageRef(id: id, width: width, height: height) }
    }

    // MARK: Key and wrap

    /// A brand-new space key, wrapped with `passcode`.
    public static func create(passcode: String, iterations: Int = Vault.iterations) throws -> (wrap: VaultWrap, key: SymmetricKey) {
        let key = SymmetricKey(size: .bits256)
        let wrap = try wrapKey(key, passcode: passcode, iterations: iterations)
        return (wrap, key)
    }

    /// The stretching a wrap may ask for (the server takes 100k–10M): more
    /// than `UInt32` traps, and billions would hang the unlock for hours.
    public static let iterationRange = 1...10_000_000

    /// Unwrap with the passcode; `.wrongPasscode` if it doesn't fit.
    public static func open(_ wrap: VaultWrap, passcode: String) throws -> SymmetricKey {
        guard let salt = Data(base64URL: wrap.salt), let iv = Data(base64URL: wrap.iv), let ct = Data(base64URL: wrap.ct),
              ct.count > 16, iterationRange.contains(wrap.iter) else { throw Failure.badWrap }
        let kek = passcodeKey(passcode, salt: salt, iterations: wrap.iter)
        do {
            let box = try AES.GCM.SealedBox(nonce: AES.GCM.Nonce(data: iv), ciphertext: ct.dropLast(16), tag: ct.suffix(16))
            let raw = try AES.GCM.open(box, using: kek, authenticating: wrapAAD)
            guard raw.count == 32 else { throw Failure.badWrap }
            return SymmetricKey(data: raw)
        } catch let failure as Failure {
            throw failure
        } catch {
            throw Failure.wrongPasscode
        }
    }

    /// Same key, new passcode (notes don't change).
    public static func rewrap(_ wrap: VaultWrap, old: String, new: String) throws -> VaultWrap {
        let key = try open(wrap, passcode: old)
        return try wrapKey(key, passcode: new, iterations: max(wrap.iter, iterations))
    }

    /// Wrap a key we already hold (e.g. after unlocking) with a passcode.
    public static func wrapKey(_ key: SymmetricKey, passcode: String, iterations: Int = Vault.iterations) throws -> VaultWrap {
        let salt = randomBytes(16)
        let kek = passcodeKey(passcode, salt: salt, iterations: iterations)
        let raw = key.withUnsafeBytes { Data($0) }
        let box = try AES.GCM.seal(raw, using: kek, nonce: AES.GCM.Nonce(), authenticating: wrapAAD)
        return VaultWrap(v: 1, iter: iterations, salt: salt.base64URL, iv: Data(box.nonce).base64URL, ct: (box.ciphertext + box.tag).base64URL)
    }

    /// PBKDF2-HMAC-SHA256 over the NFKC-normalized passcode's UTF-8 → 32 bytes.
    static func passcodeKey(_ passcode: String, salt: Data, iterations: Int) -> SymmetricKey {
        let password = Data(passcode.precomposedStringWithCompatibilityMapping.utf8)
        var derived = [UInt8](repeating: 0, count: 32)
        let status = password.withUnsafeBytes { pw in
            salt.withUnsafeBytes { s in
                CCKeyDerivationPBKDF(CCPBKDFAlgorithm(kCCPBKDF2),
                                     pw.baseAddress?.assumingMemoryBound(to: CChar.self), password.count,
                                     s.baseAddress?.assumingMemoryBound(to: UInt8.self), salt.count,
                                     CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256), UInt32(clamping: iterations),
                                     &derived, derived.count)
            }
        }
        precondition(status == kCCSuccess, "PBKDF2 failed")
        return SymmetricKey(data: derived)
    }

    // MARK: Notes and pictures

    public static func isEncryptedNote(_ body: String) -> Bool {
        guard body.hasPrefix("e1:"), body.count > 3 else { return false }
        return body.dropFirst(3).allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-" || $0 == "_") }
    }

    public static func seal(_ note: Note, with key: SymmetricKey) throws -> String {
        let json = try JSONEncoder().encode(note)
        let box = try AES.GCM.seal(json, using: key, nonce: AES.GCM.Nonce(), authenticating: noteAAD)
        return "e1:" + (Data(box.nonce) + box.ciphertext + box.tag).base64URL
    }

    public static func open(note body: String, with key: SymmetricKey) throws -> Note {
        guard isEncryptedNote(body), let bytes = Data(base64URL: String(body.dropFirst(3))), bytes.count > 28 else { throw Failure.cantOpen }
        do {
            let plain = try openBytes(bytes, key: key, aad: noteAAD)
            return try JSONDecoder().decode(Note.self, from: plain)
        } catch {
            throw Failure.cantOpen
        }
    }

    public static func seal(blob: Data, with key: SymmetricKey) throws -> Data {
        let box = try AES.GCM.seal(blob, using: key, nonce: AES.GCM.Nonce(), authenticating: blobAAD)
        return Data(box.nonce) + box.ciphertext + box.tag
    }

    public static func open(blob: Data, with key: SymmetricKey) throws -> Data {
        guard blob.count > 28 else { throw Failure.cantOpen }
        do { return try openBytes(blob, key: key, aad: blobAAD) } catch { throw Failure.cantOpen }
    }

    /// `iv(12) ‖ ciphertext ‖ tag(16)`.
    static func openBytes(_ bytes: Data, key: SymmetricKey, aad: Data) throws -> Data {
        let data = Data(bytes)
        let box = try AES.GCM.SealedBox(nonce: AES.GCM.Nonce(data: data.prefix(12)),
                                        ciphertext: data.dropFirst(12).dropLast(16), tag: data.suffix(16))
        return try AES.GCM.open(box, using: key, authenticating: aad)
    }

    // MARK: Passcode rules (same as the web's)

    /// Why a new passcode won't do, or nil if it's fine.
    public static func problem(newPasscode pass: String, again: String) -> String? {
        if pass.count < 8 { return "Use at least 8 characters (a short phrase is easiest to remember)." }
        if pass.allSatisfy(\.isASCIIDigit), pass.count < 12 { return "Numbers alone are easy to guess. Add some words." }
        if pass != again { return "Those two don’t match." }
        return nil
    }

    static func randomBytes(_ count: Int) -> Data {
        var bytes = [UInt8](repeating: 0, count: count)
        _ = SecRandomCopyBytes(kSecRandomDefault, count, &bytes)
        return Data(bytes)
    }
}

/// The wrapped space key the server stores (`ready.vault`, `vault.set`).
public struct VaultWrap: Codable, Sendable, Hashable {
    public var v: Int
    public var iter: Int
    public var salt: String
    public var iv: String
    public var ct: String

    public init(v: Int = 1, iter: Int, salt: String, iv: String, ct: String) {
        self.v = v
        self.iter = iter
        self.salt = salt
        self.iv = iv
        self.ct = ct
    }
}

private extension Character {
    var isASCIIDigit: Bool { isASCII && isNumber }
}

public extension Data {
    /// base64url without padding (the vault's text encoding).
    var base64URL: String {
        base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    init?(base64URL text: String) {
        var s = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while s.count % 4 != 0 { s += "=" }
        self.init(base64Encoded: s)
    }
}
