import Foundation
import Security

/// What this device remembers about who it is. The device token and the
/// invite key are secrets (Keychain); name/color/uid and per-device
/// preferences aren't (UserDefaults).
@MainActor
public protocol CredentialStore: AnyObject {
    var endpoint: ServerEndpoint? { get set }
    var token: String? { get set }
    var uid: String? { get set }
    var name: String? { get set }
    var color: String? { get set }
    /// channelId -> newest ts the person has seen.
    var lastRead: [String: Int] { get set }
    /// This device's copy of the encrypted Personal space's key, tagged
    /// with the wrap it came from (`wrap.ct`). Keychain, this device only.
    func vaultKey(for uid: String) -> StoredVaultKey?
    func setVaultKey(_ key: StoredVaultKey?, for uid: String)
}

/// The space key (32 bytes) and the `wrap.ct` it was unwrapped from.
public struct StoredVaultKey: Codable, Sendable, Equatable {
    public var key: Data
    public var ct: String

    public init(key: Data, ct: String) {
        self.key = key
        self.ct = ct
    }
}

/// For tests and previews: nothing touches disk.
@MainActor
public final class MemoryCredentialStore: CredentialStore {
    public var endpoint: ServerEndpoint?
    public var token: String?
    public var uid: String?
    public var name: String?
    public var color: String?
    public var lastRead: [String: Int] = [:]
    public var vaultKeys: [String: StoredVaultKey] = [:]

    public init(endpoint: ServerEndpoint? = nil, token: String? = nil) {
        self.endpoint = endpoint
        self.token = token
    }

    public func vaultKey(for uid: String) -> StoredVaultKey? { vaultKeys[uid] }
    public func setVaultKey(_ key: StoredVaultKey?, for uid: String) { vaultKeys[uid] = key }
}

/// The real store. `service` keeps the Dev build's Keychain items apart from
/// the App Store build's (AppIdentity passes the bundle id).
@MainActor
public final class DeviceCredentialStore: CredentialStore {
    private let keychain: Keychain
    private let defaults: UserDefaults
    private let prefix = "yap."

    public init(service: String, defaults: UserDefaults = .standard) {
        keychain = Keychain(service: service)
        self.defaults = defaults
    }

    public var endpoint: ServerEndpoint? {
        get {
            guard let data = keychain.data(for: "endpoint") else { return nil }
            return try? JSONDecoder().decode(ServerEndpoint.self, from: data)
        }
        set {
            if let newValue, let data = try? JSONEncoder().encode(newValue) {
                keychain.set(data, for: "endpoint")
            } else {
                keychain.remove("endpoint")
            }
        }
    }

    public var token: String? {
        get { keychain.data(for: "token").map { String(decoding: $0, as: UTF8.self) } }
        set {
            if let newValue, !newValue.isEmpty { keychain.set(Data(newValue.utf8), for: "token") } else { keychain.remove("token") }
        }
    }

    public var uid: String? {
        get { defaults.string(forKey: prefix + "uid") }
        set { defaults.set(newValue, forKey: prefix + "uid") }
    }

    public var name: String? {
        get { defaults.string(forKey: prefix + "name") }
        set { defaults.set(newValue, forKey: prefix + "name") }
    }

    public var color: String? {
        get { defaults.string(forKey: prefix + "color") }
        set { defaults.set(newValue, forKey: prefix + "color") }
    }

    public var lastRead: [String: Int] {
        get { (defaults.dictionary(forKey: prefix + "lastRead") as? [String: Int]) ?? [:] }
        set { defaults.set(newValue, forKey: prefix + "lastRead") }
    }

    public func vaultKey(for uid: String) -> StoredVaultKey? {
        keychain.data(for: "vault." + uid).flatMap { try? JSONDecoder().decode(StoredVaultKey.self, from: $0) }
    }

    public func setVaultKey(_ key: StoredVaultKey?, for uid: String) {
        if let key, let data = try? JSONEncoder().encode(key) {
            keychain.set(data, for: "vault." + uid)
        } else {
            keychain.remove("vault." + uid)
        }
    }
}

/// A tiny generic-password wrapper. Items are this-device-only and readable
/// after first unlock, so a voice call that reconnects in the background
/// can still read the token.
public struct Keychain: Sendable {
    public let service: String

    public init(service: String) {
        self.service = service
    }

    private func query(_ account: String) -> [String: Any] {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        #if os(macOS)
        // The data-protection keychain needs a team-signed app; ad-hoc dev
        // builds fall back to the login keychain (see `set`).
        if Keychain.useDataProtection { q[kSecUseDataProtectionKeychain as String] = true }
        #endif
        return q
    }

    #if os(macOS)
    nonisolated(unsafe) static var useDataProtection = true
    #endif

    public func data(for account: String) -> Data? {
        var q = query(account)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &out)
        #if os(macOS)
        if status == errSecMissingEntitlement, Keychain.useDataProtection {
            Keychain.useDataProtection = false
            return data(for: account)
        }
        #endif
        return status == errSecSuccess ? out as? Data : nil
    }

    @discardableResult
    public func set(_ data: Data, for account: String) -> Bool {
        let q = query(account)
        let update: [String: Any] = [kSecValueData as String: data]
        var status = SecItemUpdate(q as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            var add = q
            add[kSecValueData as String] = data
            add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            status = SecItemAdd(add as CFDictionary, nil)
        }
        #if os(macOS)
        if status == errSecMissingEntitlement, Keychain.useDataProtection {
            Keychain.useDataProtection = false
            return set(data, for: account)
        }
        #endif
        return status == errSecSuccess
    }

    public func remove(_ account: String) {
        let status = SecItemDelete(query(account) as CFDictionary)
        #if os(macOS)
        if status == errSecMissingEntitlement, Keychain.useDataProtection {
            Keychain.useDataProtection = false
            remove(account)
        }
        #else
        _ = status
        #endif
    }
}
