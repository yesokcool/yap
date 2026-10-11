import Foundation
#if os(iOS)
import UIKit
#endif

/// Which build this is, and every identifier that follows from it.
///
/// The bundle id is `YAP_BUNDLE_ID` from `Config/Yap.xcconfig` (your own
/// value goes in `Config/Local.xcconfig`). The **Dev** configuration (scheme
/// "Yap DEV") installs beside the real app as `<YAP_BUNDLE_ID>.dev` ("Yap
/// Dev", the DEV-banded icon) with its own Keychain items and UserDefaults,
/// so testing never signs the real app out. Nothing in the source hardcodes
/// the identifiers; they're derived from the running bundle here
/// (`AppIdentityTests` pins them).
enum AppIdentity {
    nonisolated static let bundleIdentifier: String = Bundle.main.bundleIdentifier ?? "yap"

    /// The App Store build's id: `YAP_BUNDLE_ID`, which the Info.plist carries
    /// as `YapBundleIdentifier` (else this bundle's id without ".dev").
    nonisolated static let productionBundleIdentifier: String = {
        if let id = Bundle.main.object(forInfoDictionaryKey: "YapBundleIdentifier") as? String, !id.isEmpty { return id }
        return bundleIdentifier.hasSuffix(".dev") ? String(bundleIdentifier.dropLast(4)) : bundleIdentifier
    }()

    /// "Dev" for the sandbox build, nil for the real app.
    nonisolated static let buildFlavor: String? = bundleIdentifier.hasSuffix(".dev") ? "Dev" : nil
    nonisolated static var isDevBuild: Bool { buildFlavor != nil }

    /// The App Group the iPhone app shares with its screen-broadcast
    /// extension (`YAP_APP_GROUP`, carried in the Info.plist as `YapAppGroup`).
    nonisolated static var appGroup: String? {
        (Bundle.main.object(forInfoDictionaryKey: "YapAppGroup") as? String).flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Keychain service for the device token + invite key.
    nonisolated static var keychainService: String { bundleIdentifier }

    /// The `client` label the server shows for this device: "ios" (iPhone),
    /// "ipad" or "mac", so Settings › Account can say which one it is.
    @MainActor static var clientLabel: String {
        #if os(macOS)
        "mac"
        #else
        UIDevice.current.userInterfaceIdiom == .pad ? "ipad" : "ios"
        #endif
    }

    /// "1.0": the marketing version alone (the sidebar footer's "v1.0").
    nonisolated static var shortVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "1.0"
    }

    nonisolated static var version: String {
        let v = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "1.0"
        let b = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "1"
        return "\(v) (\(b))"
    }
}
