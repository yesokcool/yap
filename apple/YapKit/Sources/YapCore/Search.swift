import Foundation

/// A search box's query, parsed like the server does (PROTOCOL.md › search):
/// every word must appear (case-insensitive), quotes keep a phrase
/// together, `from:Name` (name prefix) and `has:image|gif|link` filter. Used
/// to search encrypted Personal-space notes on the device and to highlight
/// matches in result snippets.
public struct SearchQuery: Equatable, Sendable {
    public var words: [String] = []
    public var from: String?
    public var has: Has?

    public enum Has: String, Sendable { case image, gif, link }

    public init(_ text: String) {
        let pattern = try! NSRegularExpression(pattern: #"(?:[^\s"]+|"[^"]*")+"#)
        let ns = text as NSString
        for match in pattern.matches(in: text, range: NSRange(location: 0, length: ns.length)) {
            let token = ns.substring(with: match.range)
            let lower = token.lowercased()
            if lower.hasPrefix("from:"), token.count > 5 {
                from = String(token.dropFirst(5)).trimmingCharacters(in: CharacterSet(charactersIn: "\""))
            } else if lower.hasPrefix("has:") {
                let what = lower.dropFirst(4)
                if what.hasPrefix("image") || what.hasPrefix("picture") { has = .image }
                else if what.hasPrefix("gif") { has = .gif }
                else if what.hasPrefix("link") { has = .link }
            } else {
                let word = lower.trimmingCharacters(in: CharacterSet(charactersIn: "\""))
                if !word.isEmpty { words.append(word) }
            }
        }
    }

    public var isEmpty: Bool { words.isEmpty && from == nil && has == nil }

    /// Does a note (text + what it carries) by `author` match?
    public func matches(text: String, author: String, hasImage: Bool, hasGif: Bool) -> Bool {
        if let from, !author.lowercased().hasPrefix(from.lowercased()) { return false }
        switch has {
        case .image: if !hasImage { return false }
        case .gif: if !hasGif { return false }
        case .link: if !text.lowercased().contains("http") { return false }
        case nil: break
        }
        let lower = text.lowercased()
        return words.allSatisfy { lower.contains($0) }
    }

    /// A short excerpt around the first match (or the start), and the ranges
    /// of every matched word in it, for highlighting.
    public func snippet(of text: String, radius: Int = 60) -> (text: String, matches: [Range<String.Index>]) {
        let plain = text.replacingOccurrences(of: "\n", with: " ")
        var start = plain.startIndex
        if let first = words.compactMap({ plain.range(of: $0, options: .caseInsensitive)?.lowerBound }).min() {
            start = plain.index(first, offsetBy: -radius, limitedBy: plain.startIndex) ?? plain.startIndex
        }
        let end = plain.index(start, offsetBy: radius * 3, limitedBy: plain.endIndex) ?? plain.endIndex
        var excerpt = String(plain[start..<end])
        if start > plain.startIndex { excerpt = "…" + excerpt }
        if end < plain.endIndex { excerpt += "…" }
        var ranges: [Range<String.Index>] = []
        for word in words where !word.isEmpty {
            var from = excerpt.startIndex
            while let r = excerpt.range(of: word, options: .caseInsensitive, range: from..<excerpt.endIndex) {
                ranges.append(r)
                from = r.upperBound
            }
        }
        return (excerpt, ranges)
    }
}
