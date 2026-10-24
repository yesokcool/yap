import Foundation

/// Where a Yap server lives and the invite key that opens it.
public struct ServerEndpoint: Codable, Sendable, Hashable {
    public var baseURL: URL
    public var key: String

    public init(baseURL: URL, key: String) {
        self.baseURL = baseURL
        self.key = key
    }

    /// Why an invite link doesn't open anything.
    public enum InviteProblem: Error, Equatable, Sendable {
        /// Not a link with a server and a key.
        case notALink
        /// `http://` to another computer: the key, your password and every
        /// message would cross the network readable.
        case plainHTTP

        public var message: String {
            switch self {
            case .notALink: "That doesn't look like a Yap invite link."
            case .plainHTTP: "That link starts with http://, so your password and messages would cross the network unencrypted. Ask for the https:// link."
            }
        }
    }

    /// Parses an invite link: `https://host/#key=K` (the key may also be a
    /// `?key=` query). Without a scheme, https is assumed
    /// (`chat.example.com/#key=K`). There's no default server, so a bare key
    /// isn't enough: the link says which server it opens.
    public static func parse(invite input: String) -> ServerEndpoint? {
        try? read(invite: input)
    }

    /// `parse(invite:)`, saying why not. Plain `http://` only for this
    /// computer (a test server; `isLoopback`).
    public static func read(invite input: String) throws(InviteProblem) -> ServerEndpoint {
        var value = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty, !value.contains(" ") else { throw .notALink }
        if !value.contains("://") { value = "https://" + value }
        guard let url = URL(string: value), let scheme = url.scheme?.lowercased(), scheme == "https" || scheme == "http",
              let host = url.host(), !host.isEmpty else { throw .notALink }
        if scheme == "http", !isLoopback(host) { throw .plainHTTP }
        var key: String?
        if let fragment = url.fragment(percentEncoded: false) {
            let items = URLComponents(string: "?" + fragment)?.queryItems ?? []
            key = items.first(where: { $0.name == "key" })?.value
        }
        if key == nil {
            key = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first(where: { $0.name == "key" })?.value
        }
        guard let key, !key.isEmpty else { throw .notALink }
        var base = URLComponents()
        base.scheme = scheme
        base.host = host.contains(":") ? "[\(host)]" : host // an IPv6 address keeps its brackets
        base.port = url.port
        guard let baseURL = base.url else { throw .notALink }
        return ServerEndpoint(baseURL: baseURL, key: key)
    }

    /// This computer: `localhost`, 127.0.0.0/8 or ::1.
    public static func isLoopback(_ host: String) -> Bool {
        var host = host.lowercased()
        if host.hasPrefix("["), host.hasSuffix("]") { host = String(host.dropFirst().dropLast()) }
        if host == "localhost" || host == "::1" || host == "0:0:0:0:0:0:0:1" { return true }
        let octets = host.split(separator: ".", omittingEmptySubsequences: false)
        guard octets.count == 4, octets.first == "127" else { return false }
        return octets.allSatisfy { octet in
            octet.count <= 3 && octet.allSatisfy { $0.isASCII && $0.isNumber } && (Int(octet) ?? 256) <= 255
        }
    }

    /// https, or plain http to this computer: the only servers the app talks
    /// to (an invite link, a saved server from an older version).
    public var isSafeToUse: Bool {
        switch baseURL.scheme?.lowercased() {
        case "https": baseURL.host()?.isEmpty == false
        case "http": baseURL.host().map(Self.isLoopback) ?? false
        default: false
        }
    }

    /// Unreserved URL characters (personal invite keys are `yi_` + base64url).
    static let keyCharacters = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))

    /// The link to hand to a friend.
    public var inviteLink: String {
        var c = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)!
        c.path = "/"
        c.percentEncodedFragment = "key=" + (key.addingPercentEncoding(withAllowedCharacters: Self.keyCharacters) ?? key)
        return c.string ?? baseURL.absoluteString
    }

    /// A URL on this server. The key never goes in a URL (it would end up in
    /// logs): HTTP calls carry it as `X-Yap-Key`, the socket as a subprotocol.
    public func url(_ path: String, query: [String: String] = [:]) -> URL {
        var c = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)!
        c.path = path
        if !query.isEmpty {
            c.queryItems = query.sorted(by: { $0.key < $1.key }).map { URLQueryItem(name: $0.key, value: $0.value) }
        }
        return c.url!
    }

    /// A request with the invite key in the `X-Yap-Key` header.
    public func request(_ path: String, query: [String: String] = [:], method: String = "GET") -> URLRequest {
        var request = URLRequest(url: url(path, query: query))
        request.httpMethod = method
        request.setValue(key, forHTTPHeaderField: "X-Yap-Key")
        request.cachePolicy = .reloadIgnoringLocalCacheData
        return request
    }

    /// `ws(s)://host/ws`
    public var webSocketURL: URL {
        var c = URLComponents(url: url("/ws"), resolvingAgainstBaseURL: false)!
        c.scheme = baseURL.scheme == "https" ? "wss" : "ws"
        return c.url!
    }

    /// The server answers with `yap`; `key.<K>` carries the invite key.
    public var webSocketProtocols: [String] { ["yap", "key.\(key)"] }

    /// `/emoji/<id>`: no key needed (ids are content hashes).
    public func emojiURL(_ id: String) -> URL { baseURL.appending(path: "emoji/\(id)") }
    /// `/media/<id>`: avatars and saved drawings.
    public func mediaURL(_ id: String) -> URL { baseURL.appending(path: "media/\(id)") }
    public var boardURL: URL { baseURL.appending(path: "board") }

    // MARK: The in-app web view

    /// Where a navigation in the in-app web view goes. The view only ever
    /// shows this server (Draw, Space, the board); it has no address bar, so
    /// another site in it would look like Yap.
    public enum WebNavigation: Equatable, Sendable {
        /// This server's own page.
        case allow
        /// A link elsewhere, or one that wants a new window: the system opens it (a browser, Mail).
        case openOutside
        case refuse
    }

    /// The server's origin as the web sees it (`location.origin`): no port when it's the scheme's own.
    public var webOrigin: String {
        let scheme = baseURL.scheme?.lowercased() ?? "https"
        var host = baseURL.host()?.lowercased() ?? ""
        if host.contains(":") { host = "[\(host)]" }
        let port = baseURL.port.flatMap { $0 == Self.defaultPort(scheme) ? nil : $0 }
        return "\(scheme)://\(host)" + (port.map { ":\($0)" } ?? "")
    }

    /// Same scheme, host and port as this server.
    public func isOwnOrigin(_ url: URL) -> Bool {
        guard let scheme = url.scheme?.lowercased(), scheme == "https" || scheme == "http",
              scheme == baseURL.scheme?.lowercased(),
              let host = url.host()?.lowercased(), host == baseURL.host()?.lowercased() else { return false }
        return (url.port ?? Self.defaultPort(scheme)) == (baseURL.port ?? Self.defaultPort(scheme))
    }

    /// The web view's rule: in the view, this server's pages only; other
    /// http(s) and mailto links, and every new window (`target=_blank`,
    /// `window.open`), go to the system. Frames inside the page (a YouTube
    /// player) are the page's own business.
    public func webNavigation(to url: URL, mainFrame: Bool, newWindow: Bool) -> WebNavigation {
        let scheme = url.scheme?.lowercased() ?? ""
        let outside = ["https", "http", "mailto"].contains(scheme) ? WebNavigation.openOutside : .refuse
        if newWindow { return outside }
        guard mainFrame else { return .allow }
        if isOwnOrigin(url) { return .allow }
        switch scheme {
        case "about": return url.absoluteString.lowercased() == "about:blank" ? .allow : .refuse
        // `blob:https://host/…`: something the page made itself.
        case "blob": return URL(string: String(url.absoluteString.dropFirst("blob:".count))).map(isOwnOrigin) == true ? .allow : .refuse
        default: return outside
        }
    }

    static func defaultPort(_ scheme: String) -> Int { scheme == "http" ? 80 : 443 }
}

// MARK: - HTTP payloads

/// `/api/check`: does this invite key open the server?
public struct KeyCheck: Decodable, Sendable {
    public var ok: Bool
}

public struct ServerConfig: Decodable, Sendable, Equatable {
    public struct Gifs: Decodable, Sendable, Equatable {
        public var provider: String
        public var appKey: String
    }

    public struct Minecraft: Decodable, Sendable, Equatable {
        public var name: String
        public var address: String
    }

    public var gifs: Gifs?
    public var minecraft: Minecraft?

    public init(gifs: Gifs? = nil, minecraft: Minecraft? = nil) {
        self.gifs = gifs
        self.minecraft = minecraft
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyKey.self)
        gifs = try? c.decodeIfPresent(Gifs.self, forKey: "gifs")
        minecraft = try? c.decodeIfPresent(Minecraft.self, forKey: "minecraft")
    }
}

public struct MinecraftStatus: Decodable, Sendable, Equatable {
    public struct Players: Decodable, Sendable, Equatable {
        public var online: Int
        public var max: Int
        public var names: [String]?
    }

    public var name: String?
    public var address: String
    public var online: Bool
    public var ms: Int?
    public var version: String?
    public var motd: String?
    public var players: Players?

    public init(name: String? = nil, address: String, online: Bool, ms: Int? = nil, version: String? = nil, motd: String? = nil, players: Players? = nil) {
        self.name = name
        self.address = address
        self.online = online
        self.ms = ms
        self.version = version
        self.motd = motd
        self.players = players
    }

    /// "3/20 playing · 1.21.4", or "Offline".
    public var summary: String {
        guard online else { return "Offline" }
        var parts: [String] = []
        if let players { parts.append("\(players.online)/\(players.max) playing") }
        if let version, !version.isEmpty { parts.append(version) }
        return parts.isEmpty ? "Online" : parts.joined(separator: " · ")
    }
}

/// `RTCIceServer` shape from `/api/ice`; `urls` may be a string or a list.
public struct IceServerConfig: Decodable, Sendable, Equatable {
    /// Does this list include a TURN relay (needed for relay-only voice)? Like the web's `hasRelay`.
    public static func hasRelay(_ servers: [IceServerConfig]) -> Bool {
        servers.contains { $0.urls.contains { $0.lowercased().hasPrefix("turn:") || $0.lowercased().hasPrefix("turns:") } }
    }

    public var urls: [String]
    public var username: String?
    public var credential: String?

    public init(urls: [String], username: String? = nil, credential: String? = nil) {
        self.urls = urls
        self.username = username
        self.credential = credential
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyKey.self)
        if let list = try? c.decode([String].self, forKey: "urls") {
            urls = list
        } else {
            urls = [try c.decode(String.self, forKey: "urls")]
        }
        username = try c.decodeIfPresent(String.self, forKey: "username")
        credential = try c.decodeIfPresent(String.self, forKey: "credential")
    }

    public static let fallback = [IceServerConfig(urls: ["stun:stun.cloudflare.com:3478"])]
}

public enum ServerAPIError: Error, Equatable, Sendable {
    case badKey
    case http(Int)
}

/// The handful of plain HTTP calls next to the socket.
public struct ServerAPI: Sendable {
    public var endpoint: ServerEndpoint
    public var session: URLSession

    public init(endpoint: ServerEndpoint, session: URLSession = .shared) {
        self.endpoint = endpoint
        self.session = session
    }

    private func get<T: Decodable>(_ path: String, _ type: T.Type) async throws -> T {
        let (data, response) = try await session.data(for: endpoint.request(path))
        let status = (response as? HTTPURLResponse)?.statusCode ?? 200
        if status == 401 { throw ServerAPIError.badKey }
        guard (200..<300).contains(status) else { throw ServerAPIError.http(status) }
        return try JSONDecoder().decode(T.self, from: data)
    }

    /// Is the invite key good? Call before connecting.
    public func check() async throws -> KeyCheck { try await get("/api/check", KeyCheck.self) }
    public func config() async throws -> ServerConfig { try await get("/api/config", ServerConfig.self) }
    public func minecraft() async throws -> MinecraftStatus { try await get("/api/minecraft", MinecraftStatus.self) }

    /// The server's Yap version (`/changelog.json › current`), what the
    /// sidebar's "what's new" link shows, like the web.
    public func serverVersion() async throws -> String {
        struct Changelog: Decodable { var current: String }
        return try await get("/changelog.json", Changelog.self).current
    }

    /// Your picture: a square image ≤ 512 KB (PNG/JPEG/WebP/GIF). The server
    /// broadcasts the new `User.avatar` to everyone (including us).
    ///
    /// Every upload carries our session id and `ut` = the `uploadToken` from
    /// our own `ready` (sids are visible to every member; the token isn't).
    public func uploadAvatar(_ image: Data, mime: String, sid: String, uploadToken: String) async throws {
        _ = try await upload("/api/avatar", image, mime: mime, query: ["sid": sid, "ut": uploadToken])
    }

    /// A custom emoji (≤ 512 KB) named `name`; the server broadcasts it.
    public func uploadEmoji(_ image: Data, name: String, mime: String, sid: String, uploadToken: String) async throws {
        _ = try await upload("/api/emoji", image, mime: mime, query: ["sid": sid, "ut": uploadToken, "name": name])
    }

    /// A picture for a message (≤ 1.9 MB; PNG, JPEG, WebP or GIF, sniffed
    /// from the bytes). Returns its media id; then `send` it as `image`.
    public func uploadImage(_ image: Data, mime: String, sid: String, uploadToken: String) async throws -> String {
        let body = try await upload("/api/image", image, mime: mime, query: ["sid": sid, "ut": uploadToken])
        guard let id = (try? JSONSerialization.jsonObject(with: body) as? [String: Any])?["id"] as? String else {
            throw UploadError(status: 0, message: "The server didn't return the picture.")
        }
        return id
    }

    /// An encrypted picture for the Personal space: opaque bytes (≤ 1.9 MB),
    /// fetched back from `/media/<id>` and decrypted on the device.
    public func uploadBlob(_ bytes: Data, sid: String, uploadToken: String) async throws -> String {
        let body = try await upload("/api/blob", bytes, mime: "application/octet-stream", query: ["sid": sid, "ut": uploadToken])
        guard let id = (try? JSONSerialization.jsonObject(with: body) as? [String: Any])?["id"] as? String else {
            throw UploadError(status: 0, message: "The server didn't return the picture.")
        }
        return id
    }

    /// A soundboard clip or my join chime: a PCM16 WAV (mono, 8–48 kHz; ≤ 5 s
    /// board, ≤ 2 s chime, ≤ 480 KB). The server measures its loudness and
    /// broadcasts `sound.added`.
    public func uploadSound(_ wav: Data, kind: String, name: String, emoji: String?, sid: String, uploadToken: String) async throws -> Sound {
        var query = ["sid": sid, "ut": uploadToken, "kind": kind, "name": name]
        if let emoji, !emoji.isEmpty { query["emoji"] = emoji }
        let body = try await upload("/api/sound", wav, mime: "audio/wav", query: query)
        struct Reply: Decodable { var sound: Sound }
        guard let sound = try? JSONDecoder().decode(Reply.self, from: body).sound else {
            throw UploadError(status: 0, message: "The server didn't return the sound.")
        }
        return sound
    }

    /// A doodle reaction's picture (GIF or PNG, ≤ 64 KB, ≤ 256 × 256) → its
    /// id; then react with `<d:ID>`.
    public func uploadDoodle(_ image: Data, mime: String, sid: String, uploadToken: String) async throws -> String {
        let body = try await upload("/api/doodle", image, mime: mime, query: ["sid": sid, "ut": uploadToken])
        guard let id = (try? JSONSerialization.jsonObject(with: body) as? [String: Any])?["id"] as? String else {
            throw UploadError(status: 0, message: "The server didn't return the doodle.")
        }
        return id
    }

    /// The largest picture `/api/image` takes.
    public static let maxImageBytes = 1_900_000

    private func upload(_ path: String, _ bytes: Data, mime: String, query: [String: String]) async throws -> Data {
        var request = endpoint.request(path, query: query, method: "POST")
        request.setValue(mime, forHTTPHeaderField: "Content-Type")
        let (data, response) = try await session.upload(for: request, from: bytes)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        // Refused (`{ ok: false, code, message }`): the day's upload allowance
        // used up (`quota`, 429), a picture too big (`too_big`)… in the server's words.
        let reply = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        guard (200..<300).contains(status), reply?["ok"] as? Bool != false else {
            let message = (reply?["message"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            throw UploadError(status: status, message: message ?? "Upload failed (HTTP \(status)).", code: reply?["code"] as? String)
        }
        return data
    }

    public struct UploadError: Error, LocalizedError, Sendable {
        public var status: Int
        public var message: String
        /// The server's reason (`quota`, `too_big`…), when it gave one.
        public var code: String?
        public var errorDescription: String? { message }

        public init(status: Int, message: String, code: String? = nil) {
            self.status = status
            self.message = message
            self.code = code
        }

        /// Not connected: what an upload (or anything else that needs the server right now) throws.
        public static let notConnected = UploadError(status: 0, message: "Not connected right now.")
    }

    /// Fetch right before joining voice (TURN credentials are short-lived).
    public func iceServers() async -> [IceServerConfig] {
        struct Body: Decodable { var iceServers: [IceServerConfig] }
        do {
            let servers = try await get("/api/ice", Body.self).iceServers
            return servers.isEmpty ? IceServerConfig.fallback : servers
        } catch {
            return IceServerConfig.fallback
        }
    }
}
