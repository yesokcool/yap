import CryptoKit
import Foundation

/// GIFs from KLIPY, called straight from the device (PROTOCOL.md › GIFs):
/// never proxied through Yap, media used exactly as returned, results shown
/// in KLIPY's order, "Powered by KLIPY" in the picker, and a share report
/// after sending so "Recent" follows the person across clients.
public struct KlipyClient: Sendable {
    public static let apiBase = URL(string: "https://api.klipy.com/api/v1")!
    public static let perPage = 24

    public enum Mode: String, Sendable, CaseIterable {
        case trending, recent, search
    }

    public struct Page: Sendable, Equatable {
        public var gifs: [Gif]
        public var hasNext: Bool
    }

    public enum Failure: Error, Equatable, Sendable {
        /// The server has no KLIPY key.
        case notConfigured
        /// Testing-mode keys get 100 requests/hour for the whole server.
        case rateLimited
        case http(Int, String?)
    }

    public var appKey: String?
    public var customerId: String
    public var locale: String
    public var session: URLSession

    /// `uid` is the account's canonical id (`ready.uid`).
    public init(appKey: String?, uid: String, locale: Locale = .current, session: URLSession = KlipyClient.mediaSession) {
        self.appKey = appKey
        customerId = KlipyClient.customerId(for: uid)
        self.locale = (locale.region?.identifier ?? "").lowercased()
        self.session = session
    }

    /// No disk cache: KLIPY's terms say media isn't to be cached or re-hosted.
    /// (A var only so DEBUG screenshot runs can swap in a fake KLIPY.)
    nonisolated(unsafe) public static var mediaSession: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.urlCache = nil
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        return URLSession(configuration: config)
    }()

    /// First 16 bytes (hex) of SHA-256("yap:" + uid): stable, anonymous, and
    /// the same on every client, so "Recent" follows the person.
    public static func customerId(for uid: String) -> String {
        let digest = SHA256.hash(data: Data("yap:\(uid)".utf8))
        return digest.prefix(16).map { String(format: "%02x", $0) }.joined()
    }

    private func endpoint(_ path: String, _ query: [String: String]) throws -> URL {
        guard let appKey, !appKey.isEmpty else { throw Failure.notConfigured }
        var c = URLComponents(url: Self.apiBase.appending(path: appKey).appending(path: "gifs").appending(path: path), resolvingAgainstBaseURL: false)!
        c.queryItems = query.sorted { $0.key < $1.key }.filter { !$0.value.isEmpty }.map { URLQueryItem(name: $0.key, value: $0.value) }
        return c.url!
    }

    public func page(_ mode: Mode, query: String = "", page: Int = 1) async throws -> Page {
        let url: URL
        switch mode {
        case .recent:
            url = try endpoint("recent/\(customerId)", ["page": "\(page)", "per_page": "\(Self.perPage)"])
        case .search:
            // content_filter is KLIPY's own safety filter; we may not filter results ourselves.
            url = try endpoint("search", ["q": query, "page": "\(page)", "per_page": "\(Self.perPage)", "customer_id": customerId, "locale": locale, "content_filter": "medium"])
        case .trending:
            url = try endpoint("trending", ["page": "\(page)", "per_page": "\(Self.perPage)", "customer_id": customerId, "locale": locale, "content_filter": "medium"])
        }
        let (data, response) = try await session.data(from: url)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 200
        if status == 429 { throw Failure.rateLimited }
        let body = try? JSONDecoder().decode(Envelope.self, from: data)
        guard (200..<300).contains(status), body?.result != false else { throw Failure.http(status, body?.errors?.message?.first) }
        return Page(gifs: (body?.data?.data ?? []).compactMap { $0.gif }, hasNext: body?.data?.hasNext ?? false)
    }

    /// Tell KLIPY a GIF was shared (feeds "Recent" and their ranking). Best effort.
    public func reportShare(slug: String, query: String?) async {
        guard let url = try? endpoint("share/\(slug)", [:]) else { return }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        var body: [String: String] = ["customer_id": customerId]
        if let query, !query.isEmpty { body["q"] = query }
        request.httpBody = try? JSONEncoder().encode(body)
        _ = try? await session.data(for: request)
    }

    // MARK: - Response shapes

    struct Envelope: Decodable {
        var result: Bool?
        var data: PageData?
        var errors: Errors?
    }

    struct Errors: Decodable {
        var message: [String]?
    }

    struct PageData: Decodable {
        var data: [Item]
        var hasNext: Bool?

        enum CodingKeys: String, CodingKey {
            case data
            case hasNext = "has_next"
        }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            data = (try? c.decode([LossyItem].self, forKey: .data))?.compactMap(\.item) ?? []
            hasNext = try? c.decodeIfPresent(Bool.self, forKey: .hasNext)
        }
    }

    struct LossyItem: Decodable {
        var item: Item?
        init(from decoder: Decoder) throws { item = try? Item(from: decoder) }
    }

    public struct Media: Decodable, Sendable {
        public var url: String
        public var width: Int?
        public var height: Int?
    }

    public struct Sizes: Decodable, Sendable {
        public var mp4: Media?
        public var webm: Media?
        public var webp: Media?
        public var gif: Media?
        public var jpg: Media?
    }

    public struct Item: Decodable, Sendable {
        public var slug: String
        public var title: String?
        public var file: [String: Sizes]?

        /// KLIPY item → the Gif we put in a message (medium size, URLs untouched).
        public var gif: Gif? {
            guard let md = file?["md"] ?? file?["hd"], let size = md.mp4 ?? md.webp ?? md.gif,
                  let width = size.width, let height = size.height, width > 0, height > 0 else { return nil }
            return Gif(provider: "klipy", slug: slug, title: title ?? "", width: width, height: height,
                       mp4: md.mp4?.url, webm: md.webm?.url, webp: md.webp?.url, gif: md.gif?.url, still: md.jpg?.url)
        }
    }
}
