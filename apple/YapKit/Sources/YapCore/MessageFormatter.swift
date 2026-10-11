import Foundation

/// Bold / italic / strikethrough flags on a run of text.
public struct InlineStyle: OptionSet, Sendable, Hashable {
    public let rawValue: Int
    public init(rawValue: Int) { self.rawValue = rawValue }
    public static let bold = InlineStyle(rawValue: 1)
    public static let italic = InlineStyle(rawValue: 2)
    public static let strike = InlineStyle(rawValue: 4)
    /// Inside `||spoiler||`: drawn as a solid block until the message is
    /// revealed (then lightly tinted).
    public static let spoiler = InlineStyle(rawValue: 8)
}

public enum Inline: Sendable, Hashable {
    case text(String, InlineStyle)
    case code(String)
    case link(URL, String, InlineStyle)
    case emoji(name: String, id: String)
    /// `@name`, `@everyone`, `@here` (the name as typed, without the @).
    case mention(String)
}

public enum MessageBlock: Sendable, Hashable {
    case paragraph([Inline])
    case quote([Inline])
    case codeBlock(String)
}

/// The web client's tiny formatter (web/js/format.js `renderBody`), as data:
/// ```blocks```, `code`, **bold**, *italic*, _italic_, ~~strike~~, "> quotes",
/// bare links, `<:name:id>` custom emoji and `||spoilers||`. Code is set
/// aside first so nothing inside it gets formatted.
public enum MessageFormatter {
    // Private-use characters as placeholders and style markers; they can't
    // be typed, and a message containing them just loses them.
    private static let slotOpen: Character = "\u{E000}"
    private static let slotClose: Character = "\u{E001}"
    private static let boldOn: Character = "\u{E010}", boldOff: Character = "\u{E011}"
    private static let italicOn: Character = "\u{E012}", italicOff: Character = "\u{E013}"
    private static let strikeOn: Character = "\u{E014}", strikeOff: Character = "\u{E015}"
    private static let spoilerOn: Character = "\u{E016}", spoilerOff: Character = "\u{E017}"

    private enum Slot {
        case codeBlock(String)
        case code(String)
        case emoji(String, String)
        case link(URL, String)
        case mention(String)
    }

    private static let codeBlockRegex = try! NSRegularExpression(pattern: "```(?:[\\w+-]*\\n)?([\\s\\S]*?)```")
    private static let inlineCodeRegex = try! NSRegularExpression(pattern: "`([^`\\n]+)`")
    private static let emojiRegex = try! NSRegularExpression(pattern: "<:([A-Za-z0-9_]{2,32}):([a-f0-9]{32})>")
    private static let linkRegex = try! NSRegularExpression(pattern: "\\bhttps?://[^\\s<\\x{E000}-\\x{E0FF}]+")
    private static let trailingPunct = try! NSRegularExpression(pattern: "[).,!?;:'\"*_~]+$")
    private static let boldRegex = try! NSRegularExpression(pattern: "\\*\\*(?=\\S)([\\s\\S]*?\\S)\\*\\*")
    private static let starItalicRegex = try! NSRegularExpression(pattern: "(^|[^*\\w])\\*(?=\\S)([^*\\n]*?\\S)\\*(?![*\\w])", options: [.anchorsMatchLines])
    private static let underscoreItalicRegex = try! NSRegularExpression(pattern: "(^|[^_\\w])_(?=\\S)([^_\\n]*?\\S)_(?![_\\w])", options: [.anchorsMatchLines])
    private static let strikeRegex = try! NSRegularExpression(pattern: "~~(?=\\S)([\\s\\S]*?\\S)~~")
    private static let quoteRegex = try! NSRegularExpression(pattern: "^> ?(.*)$")

    /// One compiled @mention regex per member list (longest names first, so
    /// "@Sam Lee" beats "@Sam"), like web/js/format.js; the same rule as
    /// `Mentions`. Word characters are ASCII (JavaScript's `\w`), and a
    /// placeholder for code, a link or an emoji counts as one, like the web's.
    private static let mentionCache = MentionCache()

    final class MentionCache: @unchecked Sendable {
        private let lock = NSLock()
        private var key = ""
        private var regex: NSRegularExpression?

        func regex(for names: [String]) -> NSRegularExpression {
            let who = (["everyone", "here"] + names).map(NSRegularExpression.escapedPattern(for:)).sorted { ($0.count, $1) > ($1.count, $0) }
            let key = who.joined(separator: "\u{0}")
            lock.lock()
            defer { lock.unlock() }
            if key != self.key || regex == nil {
                regex = try? NSRegularExpression(pattern: "(^|[^A-Za-z0-9_@\\x{E000}\\x{E001}])@(\(who.joined(separator: "|")))(?![A-Za-z0-9_\\x{E000}])", options: [.caseInsensitive])
                self.key = key
            }
            return regex!
        }
    }

    /// `names`: member names, for @mention highlighting. `hideSpoilers`:
    /// everything inside `||…||` comes back as plain `.text` runs styled
    /// `.spoiler` (links, emoji, code and mentions too, so nothing leaks or
    /// can be tapped); otherwise spoiler text keeps its formatting and is
    /// only marked `.spoiler`.
    public static func parse(_ body: String, names: [String] = [], hideSpoilers: Bool = false) -> [MessageBlock] {
        // Drop any private-use characters a sender might have pasted in.
        var s = String(body.unicodeScalars.filter { !(0xE000...0xE0FF).contains($0.value) })
        var slots: [Slot] = []

        func extract(_ regex: NSRegularExpression, _ make: (NSString, NSTextCheckingResult) -> (slot: Slot, lead: String, trail: String)?) {
            let ns = s as NSString
            var out = ""
            var cursor = 0
            for match in regex.matches(in: s, range: NSRange(location: 0, length: ns.length)) {
                out += ns.substring(with: NSRange(location: cursor, length: match.range.location - cursor))
                if let made = make(ns, match) {
                    out += made.lead + "\(slotOpen)\(slots.count)\(slotClose)" + made.trail
                    slots.append(made.slot)
                } else {
                    out += ns.substring(with: match.range)
                }
                cursor = match.range.location + match.range.length
            }
            out += ns.substring(from: cursor)
            s = out
        }

        extract(codeBlockRegex) { ns, m in
            let code = ns.substring(with: m.range(at: 1)).trimmingCharacters(in: CharacterSet(charactersIn: "\n"))
            return (.codeBlock(code), "", "")
        }
        extract(inlineCodeRegex) { ns, m in (.code(ns.substring(with: m.range(at: 1))), "", "") }
        // Spoilers outside code: markers, so links stop at the closing ||.
        if s.contains("||") {
            s = Spoilers.regex.stringByReplacingMatches(in: s, range: NSRange(location: 0, length: (s as NSString).length),
                                                        withTemplate: "\(spoilerOn)$1\(spoilerOff)")
        }
        extract(emojiRegex) { ns, m in (.emoji(ns.substring(with: m.range(at: 1)), ns.substring(with: m.range(at: 2))), "", "") }
        extract(linkRegex) { ns, m in
            let raw = ns.substring(with: m.range)
            let rawNS = raw as NSString
            let trail = trailingPunct.firstMatch(in: raw, range: NSRange(location: 0, length: rawNS.length)).map { rawNS.substring(with: $0.range) } ?? ""
            let href = String(raw.dropLast(trail.count))
            guard let url = URL(string: href) else { return nil }
            return (.link(url, href), "", trail)
        }
        // @mentions after links, so "@name" inside a URL stays part of the link.
        extract(mentionCache.regex(for: names)) { ns, m in
            (.mention(ns.substring(with: m.range(at: 2))), ns.substring(with: m.range(at: 1)), "")
        }

        func replace(_ regex: NSRegularExpression, _ template: String) {
            s = regex.stringByReplacingMatches(in: s, range: NSRange(location: 0, length: (s as NSString).length), withTemplate: template)
        }
        replace(boldRegex, "\(boldOn)$1\(boldOff)")
        replace(starItalicRegex, "$1\(italicOn)$2\(italicOff)")
        replace(underscoreItalicRegex, "$1\(italicOn)$2\(italicOff)")
        replace(strikeRegex, "\(strikeOn)$1\(strikeOff)")

        // Lines → blocks. Styles may span lines (bold over a newline), so the
        // style state carries across them.
        var blocks: [MessageBlock] = []
        var paragraph: [Inline] = []
        var quote: [Inline] = []
        var style: InlineStyle = []

        func flushParagraph() {
            trimNewlines(&paragraph)
            if !paragraph.isEmpty { blocks.append(.paragraph(paragraph)) }
            paragraph = []
        }
        func flushQuote() {
            trimNewlines(&quote)
            if !quote.isEmpty { blocks.append(.quote(quote)) }
            quote = []
        }

        let lines = s.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
        for (index, rawLine) in lines.enumerated() {
            var line = rawLine
            var isQuote = false
            let lineNS = line as NSString
            if let m = quoteRegex.firstMatch(in: line, range: NSRange(location: 0, length: lineNS.length)) {
                line = lineNS.substring(with: m.range(at: 1))
                isQuote = true
            }
            if isQuote { flushParagraph() } else { flushQuote() }
            var target: [Inline] = isQuote ? quote : paragraph
            var buffer = ""
            func flushText() {
                if !buffer.isEmpty { append(&target, .text(buffer, style)) }
                buffer = ""
            }
            var iterator = Array(line).makeIterator()
            while let ch = iterator.next() {
                switch ch {
                case boldOn: flushText(); style.insert(.bold)
                case boldOff: flushText(); style.remove(.bold)
                case italicOn: flushText(); style.insert(.italic)
                case italicOff: flushText(); style.remove(.italic)
                case strikeOn: flushText(); style.insert(.strike)
                case strikeOff: flushText(); style.remove(.strike)
                case spoilerOn: flushText(); style.insert(.spoiler)
                case spoilerOff: flushText(); style.remove(.spoiler)
                case slotOpen:
                    flushText()
                    var digits = ""
                    while let d = iterator.next(), d != slotClose { digits.append(d) }
                    guard let i = Int(digits), i < slots.count else { continue }
                    if hideSpoilers, style.contains(.spoiler) {
                        // Hidden: just its letters, as part of the block.
                        let plain: String = switch slots[i] {
                        case .code(let code), .codeBlock(let code): code
                        case .emoji(let name, _): ":\(name):"
                        case .link(_, let text): text
                        case .mention(let name): "@\(name)"
                        }
                        append(&target, .text(plain, style))
                        continue
                    }
                    switch slots[i] {
                    case .code(let code): target.append(.code(code))
                    case .emoji(let name, let id): target.append(.emoji(name: name, id: id))
                    case .link(let url, let text): target.append(.link(url, text, style))
                    case .mention(let name): target.append(.mention(name))
                    case .codeBlock(let code):
                        if isQuote { quote = target; flushQuote() } else { paragraph = target; flushParagraph() }
                        blocks.append(.codeBlock(code))
                        target = []
                    }
                default:
                    buffer.append(ch)
                }
            }
            flushText()
            if index < lines.count - 1 { append(&target, .text("\n", style)) }
            if isQuote { quote = target } else { paragraph = target }
        }
        flushQuote()
        flushParagraph()
        return blocks
    }

    /// Merge adjacent text runs with the same style (fewer `Text` pieces).
    private static func append(_ list: inout [Inline], _ inline: Inline) {
        if case let .text(new, newStyle) = inline, case let .text(old, oldStyle)? = list.last, newStyle == oldStyle {
            list[list.count - 1] = .text(old + new, oldStyle)
        } else {
            list.append(inline)
        }
    }

    private static func trimNewlines(_ list: inout [Inline]) {
        while case let .text(text, style)? = list.first, text.first == "\n" {
            let trimmed = String(text.drop(while: { $0 == "\n" }))
            if trimmed.isEmpty { list.removeFirst() } else { list[0] = .text(trimmed, style); break }
        }
        while case let .text(text, style)? = list.last, text.last == "\n" {
            var trimmed = text
            while trimmed.last == "\n" { trimmed.removeLast() }
            if trimmed.isEmpty { list.removeLast() } else { list[list.count - 1] = .text(trimmed, style); break }
        }
    }

    /// One line of plain text for a preview: formatting markers dropped,
    /// custom emoji as `:name:`, mentions as `@name`, spoilers as ▒▒▒,
    /// lines joined with spaces.
    public static func flatten(_ body: String) -> String {
        var out: [String] = []
        for block in parse(body, hideSpoilers: true) {
            switch block {
            case .paragraph(let inlines), .quote(let inlines):
                var line = ""
                var inSpoiler = false
                for inline in inlines {
                    switch inline {
                    case let .text(text, style):
                        if style.contains(.spoiler) {
                            if !inSpoiler { line += Spoilers.mask }
                            inSpoiler = true
                            continue
                        }
                        inSpoiler = false
                        line += text
                    case .code(let code): inSpoiler = false; line += code
                    case let .link(_, text, _): inSpoiler = false; line += text
                    case let .emoji(name, _): inSpoiler = false; line += ":\(name):"
                    case .mention(let name): inSpoiler = false; line += "@\(name)"
                    }
                }
                out.append(line)
            case .codeBlock(let code):
                out.append(code)
            }
        }
        return out.joined(separator: " ").split(whereSeparator: \.isNewline).joined(separator: " ")
    }

    /// Plain text for previews/notifications: one line, formatting markers
    /// dropped, emoji tokens as `:name:`, spoilers as ▒▒▒, a poll as "📊 question".
    public static func preview(_ message: Message) -> String {
        if message.body.isEmpty, let gif = message.gif { return "GIF: \(gif.title.isEmpty ? "a GIF" : gif.title)" }
        let text = flatten(message.body)
        if message.poll != nil { return "📊 \(text)" }
        return text
    }
}
