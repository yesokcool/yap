import SwiftUI
import YapCore
import YapVoice

/// The join requests above a text channel's chat (the web's
/// `renderRequests`): one bar each — "Dee wants to join · invited by Ada",
/// the passed-on-link warning, Let in / Turn away — until they'd crowd
/// the chat, then one "3 people want to join" bar with Review (People).
/// Only the owner and mods let people in (PROTOCOL.md › Identity › Waiting
/// room): members never see these, even from an older server that still
/// tells them. `JoinRequests98` draws the bars.
enum JoinRequests {
    /// Who's waiting, A–Z, for the owner and mods; nobody until we're
    /// online (the buttons need a session).
    @MainActor
    static func waiting(_ client: YapClient) -> [User] {
        guard client.mySid != nil, client.isMod else { return [] }
        return client.waitingRoom.values.sorted { $0.name < $1.name }
    }

    /// Above the waiting list in People.
    static let note = "Only the owner and mods can let people in. Turning someone away bans them."

    /// From how many people they fold into one bar: on a phone two bars
    /// (or one under the call bar) already take a lot of the screen.
    @MainActor
    static func summaryFrom(_ model: AppModel) -> Int {
        #if os(iOS)
        model.voice.isInVoice ? 1 : 2
        #else
        model.voice.isInVoice ? 2 : 3
        #endif
    }

    /// "3 people want to join" (one: "Dee wants to join").
    static func summary(_ waiting: [User]) -> (bold: String, rest: String) {
        waiting.count == 1 ? (waiting[0].name, " wants to join") : ("\(waiting.count) people", " want to join")
    }
}
