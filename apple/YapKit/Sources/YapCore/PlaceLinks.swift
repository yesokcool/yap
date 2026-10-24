import Foundation

/// A link to a place or a text channel (PROTOCOL.md › Links to places and
/// channels): `https://<server>/#place=space`, `https://<server>/#channel=<id>`
/// (`&message=<id>` jumps to a message, as pushes do). No invite key and never
/// the Personal space: a link is for people who already use Yap. An invite or
/// sign-in link (`key=`, `handoff=`) isn't one.
public enum YapLink: Hashable, Sendable {
    /// `#place=…`: one of `places`, or one this app doesn't know (it says so).
    case place(String)
    /// `#channel=<id>`, at a message when there's one.
    case channel(String, message: String? = nil)

    /// What a link can open, in the web's words (`web/js/app/places.js` ›
    /// LINKABLE): not a screen share, which only exists while someone shares.
    /// The board only on a server with one.
    public static let places = ["board", "draw", "space", "when", "watch"]

    /// ASCII letters, digits and `-._~` stay as they are; everything else in an id is percent-encoded.
    static let idCharacters = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")

    /// `place=space`, `channel=<id>&message=<id>`.
    public var fragment: String {
        func encoded(_ value: String) -> String { value.addingPercentEncoding(withAllowedCharacters: Self.idCharacters) ?? value }
        switch self {
        case .place(let place): return "place=" + encoded(place)
        case .channel(let id, let message):
            return "channel=" + encoded(id) + (message.map { "&message=" + encoded($0) } ?? "")
        }
    }

    /// `url` as a link to a place or channel on `endpoint`'s server: the same
    /// origin, the path `/`, and `place` or `channel` in the fragment. Nil for
    /// anything else, which opens as before: another server or page, and an
    /// invite or sign-in link (`key=` or `handoff=`, in the fragment or the query).
    public init?(_ url: URL, on endpoint: ServerEndpoint) {
        guard endpoint.isOwnOrigin(url), ["", "/"].contains(url.path(percentEncoded: true)),
              let fragment = url.fragment(percentEncoded: true) else { return nil }
        let items = Self.items(fragment)
        let query = url.query(percentEncoded: true).map(Self.items) ?? []
        guard !(items + query).contains(where: { $0.name == "key" || $0.name == "handoff" }) else { return nil }
        // The web's order: a place first, then a channel; an empty one is no link.
        func value(_ name: String) -> String? { items.first(where: { $0.name == name })?.value.flatMap { $0.isEmpty ? nil : $0 } }
        if let place = value("place") {
            self = .place(place)
        } else if let channel = value("channel") {
            self = .channel(channel, message: value("message"))
        } else {
            return nil
        }
    }

    /// `a=1&b=x%20y` as the web's `URLSearchParams` reads it (`+` is a space);
    /// a value that isn't valid percent-encoding counts as none.
    static func items(_ encoded: String) -> [(name: String, value: String?)] {
        encoded.split(separator: "&", omittingEmptySubsequences: true).map { pair in
            let parts = pair.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            func decode(_ part: Substring) -> String? { part.replacingOccurrences(of: "+", with: " ").removingPercentEncoding }
            return (decode(parts[0]) ?? "", parts.count > 1 ? decode(parts[1]) : "")
        }
    }
}

extension ServerEndpoint {
    /// The link to hand to someone here: the server's origin as the web shows
    /// it (`location.origin`), then the place or channel. Never the invite key.
    public func link(to link: YapLink) -> URL {
        URL(string: "\(webOrigin)/#\(link.fragment)") ?? baseURL
    }
}
