#if os(iOS)
import ActivityKit
import Foundation

/// The voice call on the Lock Screen and in the Dynamic Island (iPhone).
/// In both targets: the app starts and updates it (`LiveCall`), YapLive
/// draws it (`CallActivityViews`).
nonisolated struct CallActivityAttributes: ActivityAttributes {
    /// When you joined: the title bar's clock counts up from it by itself.
    var joinedAt: Date

    nonisolated struct ContentState: Codable, Hashable, Sendable {
        var channel: String
        /// Who's in the call, A–Z like the call sheet (OBS links left out),
        /// at most `LiveCall.shown` of them.
        var people: [Person]
        /// Everyone past those: "+2".
        var more: Int
        var muted: Bool
        /// Deafened is muted too (the mic sends nothing), as everywhere in Yap.
        var deafened: Bool
        /// "Connecting…" or "Reconnecting…" in place of the clock.
        var status: String?
        /// The × was tapped: "Leave Lounge?" until Leave, Stay or a few seconds pass.
        var askingToLeave: Bool
        /// Settings › Look: Charcoal (true), Windows Standard (false), the system's (nil).
        var dark: Bool?

        var quiet: Bool { muted || deafened }
        var count: Int { people.count + more }
        var anyoneTalking: Bool { people.contains { $0.talking } }
    }

    nonisolated struct Person: Codable, Hashable, Sendable {
        var name: String
        var me: Bool
        var talking: Bool
    }
}
#endif
