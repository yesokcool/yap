import Foundation

// Pure rules for message text every client applies the same way: blank
// text, YouTube links for Watch together, and spoilers.

extension StringProtocol {
    /// Empty, or nothing but spaces and line breaks: the one "is there
    /// anything here?" check (a name, a hang's title, a draft).
    public var isBlank: Bool { allSatisfy(\.isWhitespace) }
}

// MARK: - YouTube → Watch together

public enum YouTube {
    private static let urlRegex = try! NSRegularExpression(pattern: "\\bhttps?://[^\\s<]+")
    private static let trailing = try! NSRegularExpression(pattern: "[).,!?;:'\"]+$")

    /// The first YouTube video a message links to (its "▶ Watch together"
    /// button), the web's rule (`web/js/format.js` › `youtubeLinkIn`).
    public static func firstVideoID(in body: String) -> String? {
        let ns = body as NSString
        for match in urlRegex.matches(in: body, range: NSRange(location: 0, length: ns.length)) {
            var url = ns.substring(with: match.range)
            if let t = trailing.firstMatch(in: url, range: NSRange(location: 0, length: (url as NSString).length)) {
                url = (url as NSString).substring(to: t.range.location)
            }
            if let id = videoID(url) { return id }
        }
        return nil
    }

    /// The 11-character id in a YouTube link (watch, youtu.be, shorts,
    /// music, embed, live, nocookie), or a bare id; nil otherwise.
    public static func videoID(_ input: String) -> String? {
        let text = input.trimmingCharacters(in: .whitespaces)
        if isID(text) { return text }
        let withScheme = text.range(of: "^[a-z][a-z0-9+.-]*://", options: [.regularExpression, .caseInsensitive]) != nil ? text : "https://" + text
        guard let url = URLComponents(string: withScheme), let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
              var host = url.host?.lowercased() else { return nil }
        for prefix in ["www.", "m.", "music."] where host.hasPrefix(prefix) {
            host = String(host.dropFirst(prefix.count))
            break
        }
        let parts = url.path.split(separator: "/").map(String.init)
        var id: String?
        if host == "youtu.be" {
            id = parts.first
        } else if host == "youtube.com" || host == "youtube-nocookie.com" {
            if url.path == "/watch" {
                id = url.queryItems?.first { $0.name == "v" }?.value
            } else if parts.count >= 2, ["shorts", "embed", "live", "v", "e"].contains(parts[0]) {
                id = parts[1]
            }
        }
        guard let id, isID(id) else { return nil }
        return id
    }

    private static func isID(_ s: String) -> Bool {
        s.count == 11 && s.allSatisfy { ($0.isASCII && ($0.isLetter || $0.isNumber)) || $0 == "_" || $0 == "-" }
    }
}

// MARK: - Spoilers

/// `||like this||`: hidden until tapped (per message, not remembered).
public enum Spoilers {
    /// What notifications and one-line previews show instead.
    public static let mask = "▒▒▒"
    private static let codeBlock = try! NSRegularExpression(pattern: "```[\\s\\S]*?```")
    private static let inlineCode = try! NSRegularExpression(pattern: "`[^`\\n]+`")
    /// The rule every client shares: two pipes, at least one character, no
    /// `||` inside, no newline, two pipes. Several per line are separate.
    static let regex = try! NSRegularExpression(pattern: "\\|\\|((?:(?!\\|\\|)[^\\n])+)\\|\\|")

    /// Does the body have a spoiler (outside code)?
    public static func contains(_ body: String) -> Bool {
        guard body.contains("||") else { return false }
        let text = withoutCode(body)
        return regex.firstMatch(in: text, range: NSRange(location: 0, length: (text as NSString).length)) != nil
    }

    /// Spoilers replaced by ▒▒▒ (code spans keep their `||`).
    public static func hidden(_ body: String) -> String {
        guard body.contains("||") else { return body }
        // Protect code, mask spoilers, put the code back.
        var slots: [String] = []
        var text = body
        for regex in [codeBlock, inlineCode] {
            let ns = text as NSString
            var out = ""
            var cursor = 0
            for m in regex.matches(in: text, range: NSRange(location: 0, length: ns.length)) {
                out += ns.substring(with: NSRange(location: cursor, length: m.range.location - cursor))
                out += "\u{E020}\(slots.count)\u{E021}"
                slots.append(ns.substring(with: m.range))
                cursor = m.range.location + m.range.length
            }
            out += ns.substring(from: cursor)
            text = out
        }
        text = regex.stringByReplacingMatches(in: text, range: NSRange(location: 0, length: (text as NSString).length), withTemplate: mask)
        for (i, code) in slots.enumerated() {
            text = text.replacingOccurrences(of: "\u{E020}\(i)\u{E021}", with: code)
        }
        return text
    }

    /// The body with code spans blanked (for finding links/spoilers outside code).
    static func withoutCode(_ body: String) -> String {
        guard body.contains("`") else { return body }
        var text = body
        for regex in [codeBlock, inlineCode] {
            text = regex.stringByReplacingMatches(in: text, range: NSRange(location: 0, length: (text as NSString).length), withTemplate: " ")
        }
        return text
    }
}
