import Foundation

/// Who a message @-mentions: the one rule the server (pushes, away
/// replies) and every client share, ported from web/js/mentions.js. A
/// member's name in any case, longest names first and each mention used
/// once (so "@Sam Lee" is Sam Lee, never also Sam), not right after a
/// letter, digit, `_` or `@` (no e-mail addresses), and never inside code,
/// links or custom emoji tokens (`<:name:id>`). Those are covered up first
/// and count as a word, so one right against the name blocks it like a
/// letter would: "`x`@ben" and "<:wave:…>@ben" mention nobody,
/// "<:wave:…> @ben" mentions Ben. `MessageFormatter` highlights by the
/// same rule (its placeholders count as words on both sides).
///
/// Like the web, it works on UTF-16 and counts only ASCII letters, digits
/// and `_` as word characters, so a message pings exactly who the server
/// pushes to.
public enum Mentions {
    public struct Found: Equatable, Sendable {
        /// Everyone named, longest names first.
        public var uids: [String]
        /// It says @everyone or @here.
        public var everyone: Bool

        public init(uids: [String] = [], everyone: Bool = false) {
            self.uids = uids
            self.everyone = everyone
        }
    }

    private static let codeBlock = try! NSRegularExpression(pattern: "```[\\s\\S]*?```")
    private static let code = try! NSRegularExpression(pattern: "`[^`\\n]+`")
    private static let link = try! NSRegularExpression(pattern: "\\bhttps?://[^\\s<]+")
    private static let customEmoji = try! NSRegularExpression(pattern: "<:[A-Za-z0-9_]{2,32}:[a-f0-9]{32}>")

    public static func find(in body: String, among users: [User]) -> Found {
        find(in: body, among: users.map { ($0.id, $0.name) })
    }

    public static func find(in body: String, among users: [(id: String, name: String)]) -> Found {
        // Code, custom emojis and links covered up (in the web's order):
        // same length, word characters on both sides, like the formatter's
        // placeholders.
        let covered = NSMutableString(string: body)
        for regex in [codeBlock, code, customEmoji, link] {
            for match in regex.matches(in: covered as String, range: NSRange(location: 0, length: covered.length)) {
                covered.replaceCharacters(in: match.range, with: String(repeating: "x", count: match.range.length))
            }
        }
        var text = Array((covered as String).lowercased().utf16)

        /// Where `needle` stands as a mention.
        func at(_ needle: [UInt16]) -> [Int] {
            guard !needle.isEmpty, needle.count <= text.count else { return [] }
            var found: [Int] = []
            for i in 0...(text.count - needle.count) where text[i..<(i + needle.count)].elementsEqual(needle) {
                let before = i > 0 ? text[i - 1] : nil
                let after = i + needle.count < text.count ? text[i + needle.count] : nil
                if !(before.map { isWord($0) || $0 == atSign } ?? false), !(after.map(isWord) ?? false) { found.append(i) }
            }
            return found
        }

        let everyone = !at(Array("@everyone".utf16)).isEmpty || !at(Array("@here".utf16)).isEmpty
        var uids: [String] = []
        let named = users.filter { !$0.name.isEmpty }.sorted { $0.name.utf16.count > $1.name.utf16.count }
        for user in named {
            let needle = Array("@\(user.name.lowercased())".utf16)
            let found = at(needle)
            guard !found.isEmpty else { continue }
            uids.append(user.id)
            // Taken: a shorter name can't match inside these any more.
            for i in found { text.replaceSubrange(i..<(i + needle.count), with: repeatElement(coverUnit, count: needle.count)) }
        }
        return Found(uids: uids, everyone: everyone)
    }

    private static let atSign = UInt16(UInt8(ascii: "@"))
    private static let coverUnit = UInt16(UInt8(ascii: "x"))

    /// JavaScript's `\w`: ASCII letters, digits and `_`.
    static func isWord(_ unit: UInt16) -> Bool {
        switch unit {
        case 0x30...0x39, 0x41...0x5A, 0x61...0x7A, 0x5F: true
        default: false
        }
    }
}
