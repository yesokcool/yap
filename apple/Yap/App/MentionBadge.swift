import Foundation
import os
import UserNotifications
import YapCore
#if os(macOS)
import AppKit
#endif

/// Your unread @mentions (replies to you count) on the app icon (iPhone)
/// and the Dock icon (Mac): the sum of the sidebar's "(@3)" badges, nothing
/// when there are none.
@MainActor
final class MentionBadge {
    /// What's on the icon now (nil until the first count is set).
    private(set) var shown: Int?

    /// The number the sidebar's badges add up to: the mentions of channels
    /// still in the list (an archived channel's leftover count shows nowhere).
    static func total(_ counts: [String: Int], channels: [Channel]) -> Int {
        channels.reduce(0) { $0 + max(0, counts[$1.id] ?? 0) }
    }

    /// The words on the Dock icon (none at 0).
    static func label(_ count: Int) -> String? {
        count > 0 ? (count > 99 ? "99+" : "\(count)") : nil
    }

    /// Put `count` on the icon, if it isn't there already.
    func show(_ count: Int) {
        guard count != shown else { return }
        shown = count
        Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "badge").info("mention badge: \(count)")
        #if os(macOS)
        NSApplication.shared.dockTile.badgeLabel = Self.label(count)
        #else
        // Needs the badge permission Yap asks for along with notifications;
        // without it nothing shows (and nothing is asked again).
        UNUserNotificationCenter.current().setBadgeCount(count) { _ in }
        #endif
    }
}

extension AppModel {
    /// The badge follows the counts wherever they change (a mention coming
    /// in, a channel read, Catch up's Mark all read, signing out), also off
    /// screen during a call, where SwiftUI may not pass changes on.
    func watchMentionBadge() {
        let client = client
        let total: Int? = withObservationTracking {
            switch client.phase {
            case .needsInvite, .signedOut, .blocked: return 0
            case .online: break
            // Not connected yet (a launch): keep what the icon says.
            case .connecting, .reconnecting, .waiting: return nil
            }
            // Until the server has said what came in while we were away
            // (unread.counts, asked after ready when something's unread), too.
            if client.unreadCounts == nil, !client.unreadChannels.isEmpty, client.mentionCounts.isEmpty { return nil }
            return MentionBadge.total(client.mentionCounts, channels: client.textChannels)
        } onChange: { [weak self] in
            Task { @MainActor in self?.watchMentionBadge() }
        }
        if let total { mentionBadge.show(total) }
    }
}
