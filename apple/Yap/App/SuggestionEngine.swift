import Foundation
import YapCore

/// The composer's suggestions for the word being typed: `/` commands at
/// the start, `@people`, and `:emoji` (custom ones first). The composer
/// draws the chips.
enum SuggestionEngine {
    enum Item: Identifiable {
        case custom(Emoji)
        case standard(StandardEmoji)
        case person(User)
        /// A slash command: `/poll` and what it takes.
        case command(String, String)
        var id: String {
            switch self {
            case .custom(let e): "c:\(e.id)"
            case .standard(let e): "s:\(e.char)"
            case .person(let u): "p:\(u.id)"
            case .command(let c, _): "cmd:\(c)"
            }
        }
    }

    /// The word at the end of the draft, if it's `:que` or `@que`.
    @MainActor
    static func suggestions(for draft: String, client: YapClient) -> ([Item], String.Index?) {
        // `/` at the start: the slash commands (/poll, /roll, /flip).
        if draft.hasPrefix("/"), !draft.contains(" ") {
            let typed = draft.lowercased()
            let hits = SlashCommand.hints.filter { $0.command.hasPrefix(typed) && $0.command != typed }.map { Item.command($0.command, $0.help) }
            return (hits, hits.isEmpty ? nil : draft.startIndex)
        }
        guard let last = draft.split(separator: " ", omittingEmptySubsequences: false).last,
              last.count >= 2, let first = last.first, first == ":" || first == "@" else { return ([], nil) }
        let query = last.dropFirst().lowercased()
        guard !query.contains(":") else { return ([], nil) }
        let start = draft.index(draft.endIndex, offsetBy: -last.count)
        if first == "@" {
            let people = client.users.values
                .filter { $0.id != client.uid && $0.name.lowercased().hasPrefix(query) }
                .sorted { $0.name < $1.name }
                .prefix(6)
                .map(Item.person)
            return (Array(people), start)
        }
        guard query.count >= 2 else { return ([], nil) }
        let custom = client.emojis.values
            .filter { $0.name.lowercased().contains(query) }
            .sorted { ($0.name.lowercased().hasPrefix(query) ? 0 : 1, $0.name) < ($1.name.lowercased().hasPrefix(query) ? 0 : 1, $1.name) }
            .prefix(5)
            .map(Item.custom)
        let standard = EmojiCatalog.shared.suggestions(for: String(query), limit: 8 - custom.count).map(Item.standard)
        return (custom + standard, start)
    }

    /// Puts the picked suggestion in place of the word being typed.
    static func accept(_ item: Item, from start: String.Index, in draft: inout String) {
        let replacement: String
        switch item {
        case .custom(let emoji):
            replacement = ":\(emoji.name): "
            RecentEmoji.remember("c:\(emoji.id)")
        case .standard(let emoji):
            replacement = "\(emoji.char(tone: EmojiToneSetting.current)) "
            RecentEmoji.remember("u:\(emoji.char)")
        case .person(let user):
            replacement = "@\(user.name) "
        case .command(let command, _):
            replacement = command + " "
        }
        draft.replaceSubrange(start..<draft.endIndex, with: replacement)
    }
}
