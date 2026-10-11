import Foundation

/// One standard (unicode) emoji from `emoji-data.json` (the web client's
/// file, built by tools/build-emoji-data.mjs; bundled here as a copy).
public struct StandardEmoji: Sendable, Hashable, Identifiable {
    public let char: String
    public let group: Int
    /// Slack/Discord-style shortcodes, first = preferred.
    public let codes: [String]
    public let words: String
    /// The five skin-toned versions (light → dark), when there are any.
    public let skins: [String]

    public var id: String { char }
    public var primaryCode: String { codes.first ?? "emoji" }

    public init(char: String, group: Int, codes: [String], words: String, skins: [String] = []) {
        self.char = char
        self.group = group
        self.codes = codes
        self.words = words
        self.skins = skins
    }

    /// The character in a skin tone (0 = default yellow, 1–5 light → dark).
    public func char(tone: Int) -> String {
        guard tone > 0, tone <= skins.count else { return char }
        return skins[tone - 1]
    }
}

/// Skin tone choices, as the hands the pickers show (same as the web).
public enum EmojiTone {
    public static let hands = ["✋", "✋🏻", "✋🏼", "✋🏽", "✋🏾", "✋🏿"]
    public static let storageKey = "yap.emojiTone"
}

/// The standard emoji set: groups, search, and `:shortcode:` lookup.
public final class EmojiCatalog: Sendable {
    public static let shared: EmojiCatalog = {
        guard let url = Bundle.module.url(forResource: "emoji-data", withExtension: "json"),
              let data = try? Data(contentsOf: url) else { return EmojiCatalog(groups: [], all: []) }
        return EmojiCatalog(data: data)
    }()

    public let groups: [String]
    public let all: [StandardEmoji]
    public let byCode: [String: StandardEmoji]
    public let byChar: [String: StandardEmoji]

    public init(groups: [String], all: [StandardEmoji]) {
        self.groups = groups
        self.all = all
        var byCode: [String: StandardEmoji] = [:]
        var byChar: [String: StandardEmoji] = [:]
        for emoji in all {
            for code in emoji.codes where byCode[code] == nil { byCode[code] = emoji }
            if byChar[emoji.char] == nil { byChar[emoji.char] = emoji }
        }
        self.byCode = byCode
        self.byChar = byChar
    }

    /// `{ groups: [String], emojis: [[char, group, "codes ...", "words ...", "toned ..."?]] }`
    public convenience init(data: Data) {
        let root = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        let groups = root["groups"] as? [String] ?? []
        let rows = root["emojis"] as? [[Any]] ?? []
        let all: [StandardEmoji] = rows.compactMap { row in
            guard row.count >= 3, let char = row[0] as? String, let group = row[1] as? Int, let codes = row[2] as? String else { return nil }
            return StandardEmoji(char: char, group: group, codes: codes.split(separator: " ").map(String.init),
                                 words: row.count > 3 ? (row[3] as? String ?? "") : "",
                                 skins: row.count > 4 ? (row[4] as? String ?? "").split(separator: " ").map(String.init) : [])
        }
        self.init(groups: groups, all: all)
    }

    public func emojis(inGroup group: Int) -> [StandardEmoji] {
        all.filter { $0.group == group }
    }

    /// Picker search: shortcode contains the query, or a keyword matches.
    public func search(_ query: String, limit: Int = 300) -> [StandardEmoji] {
        let q = query.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: ": "))
        guard !q.isEmpty else { return [] }
        var hits: [StandardEmoji] = []
        for emoji in all where emoji.codes.contains(where: { $0.contains(q) }) || emoji.words.contains(q) {
            hits.append(emoji)
            if hits.count >= limit { break }
        }
        return hits
    }

    /// Composer autocomplete for `:que`: prefix matches first, then contains.
    public func suggestions(for query: String, limit: Int = 8) -> [StandardEmoji] {
        let q = query.lowercased()
        guard q.count >= 2 else { return [] }
        let prefix = all.filter { $0.codes.contains { $0.hasPrefix(q) } }
        let contains = all.filter { e in !e.codes.contains { $0.hasPrefix(q) } && e.codes.contains { $0.contains(q) } }
        return Array((prefix + contains).prefix(limit))
    }
}

/// Custom emoji tokens (`<:name:id>`), `:name:` conversion on send, and the
/// "is this message only emoji" check (PROTOCOL.md › Custom emojis in text).
public enum EmojiCodec {
    static let tokenRegex = try! NSRegularExpression(pattern: "<:([A-Za-z0-9_]{2,32}):([a-f0-9]{32})>")
    static let codeSplitRegex = try! NSRegularExpression(pattern: "```[\\s\\S]*?```|`[^`\\n]+`")
    static let shortcodeRegex = try! NSRegularExpression(pattern: "(?<!<):([A-Za-z0-9_+-]{2,32}):(?![a-f0-9]{32}>)")

    public struct Token: Sendable, Hashable {
        public let name: String
        public let id: String
    }

    /// `:party:` → `<:party:id>` for custom emojis, `:thumbsup:` → 👍 for
    /// standard ones (custom names win), never inside code spans/blocks.
    /// `custom` is keyed by lowercased name.
    public static func encode(_ text: String, custom: [String: Emoji], catalog: EmojiCatalog = .shared, tone: Int = 0) -> String {
        let ns = text as NSString
        var out = ""
        var cursor = 0
        for match in codeSplitRegex.matches(in: text, range: NSRange(location: 0, length: ns.length)) {
            out += encodePlain(ns.substring(with: NSRange(location: cursor, length: match.range.location - cursor)), custom: custom, catalog: catalog, tone: tone)
            out += ns.substring(with: match.range)
            cursor = match.range.location + match.range.length
        }
        out += encodePlain(ns.substring(from: cursor), custom: custom, catalog: catalog, tone: tone)
        return out
    }

    private static func encodePlain(_ text: String, custom: [String: Emoji], catalog: EmojiCatalog, tone: Int) -> String {
        let ns = text as NSString
        var out = ""
        var cursor = 0
        for match in shortcodeRegex.matches(in: text, range: NSRange(location: 0, length: ns.length)) {
            out += ns.substring(with: NSRange(location: cursor, length: match.range.location - cursor))
            let name = ns.substring(with: match.range(at: 1))
            if let emoji = custom[name.lowercased()] {
                out += "<:\(emoji.name):\(emoji.id)>"
            } else if let standard = catalog.byCode[name.lowercased()] {
                out += standard.char(tone: tone)
            } else {
                out += ns.substring(with: match.range)
            }
            cursor = match.range.location + match.range.length
        }
        out += ns.substring(from: cursor)
        return out
    }

    /// Every `<:name:id>` token in a body, in order.
    public static func tokens(in text: String) -> [Token] {
        let ns = text as NSString
        return tokenRegex.matches(in: text, range: NSRange(location: 0, length: ns.length)).map {
            Token(name: ns.substring(with: $0.range(at: 1)), id: ns.substring(with: $0.range(at: 2)))
        }
    }

    /// Tokens back to plain `:name:` (notifications, previews).
    public static func plain(_ text: String) -> String {
        tokenRegex.stringByReplacingMatches(in: text, range: NSRange(location: 0, length: (text as NSString).length), withTemplate: ":$1:")
    }

    /// A short all-emoji message (unicode and/or custom, 1–8 of them) renders big.
    public static func isJumbo(_ text: String) -> Bool {
        let custom = tokens(in: text).count
        let rest = tokenRegex.stringByReplacingMatches(in: text, range: NSRange(location: 0, length: (text as NSString).length), withTemplate: " ")
        var count = custom
        for character in rest {
            if character.isWhitespace { continue }
            guard character.unicodeScalars.allSatisfy(isEmojiScalar) else { return false }
            guard character.unicodeScalars.contains(where: { isPictographic($0) || (0x1F1E6...0x1F1FF).contains($0.value) }) else { return false }
            count += 1
        }
        return count > 0 && count <= 8
    }

    /// Swift has no Extended_Pictographic; emoji that aren't plain digits/#/* is close enough.
    private static func isPictographic(_ scalar: Unicode.Scalar) -> Bool {
        let p = scalar.properties
        return p.isEmojiPresentation || (p.isEmoji && scalar.value > 0x238C)
    }

    private static func isEmojiScalar(_ scalar: Unicode.Scalar) -> Bool {
        let p = scalar.properties
        return isPictographic(scalar) || p.isEmojiModifier || (0x1F1E6...0x1F1FF).contains(scalar.value)
            || scalar.value == 0x200D || scalar.value == 0xFE0F || (0xE0020...0xE007F).contains(scalar.value)
    }
}
