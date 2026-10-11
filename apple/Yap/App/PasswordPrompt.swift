import Foundation
import YapCore

/// "Choose a password" (PROTOCOL.md › Identity, `ready.weakPin`): one quiet
/// bar above the nameplate with Choose a password and Later, never a popup.
/// Later puts it off on this device for good (UserDefaults, so signing out
/// and back in doesn't bring it back); Settings › Account keeps Set a
/// password / Change password. What's put off is the reason it came up
/// for: a mod's reset later hands out a temporary password, and that one
/// is asked about once more.
enum PasswordPrompt {
    static let key = "yap.passwordPromptLater"

    /// The reasons Later put off on this device.
    static func putOff(_ defaults: UserDefaults = .standard) -> Set<String> {
        Set(defaults.stringArray(forKey: key) ?? [])
    }

    static func save(_ reasons: Set<String>, _ defaults: UserDefaults = .standard) {
        defaults.set(reasons.sorted(), forKey: key)
    }

    /// The prompt to show for this nudge, unless Later put it off.
    static func showing(_ nudge: YapClient.PasswordNudge?, putOff: Set<String>) -> YapClient.PasswordNudge? {
        guard let nudge, !putOff.contains(nudge.rawValue) else { return nil }
        return nudge
    }
}
