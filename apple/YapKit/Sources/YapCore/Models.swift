import Foundation

// Shapes from PROTOCOL.md / worker/protocol.ts. Decoding is deliberately
// tolerant: a field the server adds later, or an enum value this build
// doesn't know, must never fail a whole `ready` snapshot.

public enum ChannelKind: String, Codable, Sendable, Hashable {
    /// `personal` = your own Personal space (id `p_<your uid>`; only ever sent to you).
    case text, voice, personal, other

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = ChannelKind(rawValue: raw) ?? .other
    }
}

public struct Channel: Codable, Sendable, Hashable, Identifiable {
    /// Text-like: messages, history, edits, reactions, pictures (a text
    /// channel or your Personal space).
    public var hasMessages: Bool { kind == .text || kind == .personal }
    public var isPersonal: Bool { kind == .personal }

    public var id: String
    public var name: String
    public var kind: ChannelKind
    public var position: Int
    /// Newest message time (ms since epoch); nil when empty. Drives unread dots.
    public var lastMessageAt: Int?
    /// Voice channels: the Opus bitrate everyone in it uses (bits/s); nil =
    /// each person's own setting.
    public var bitrate: Int?

    public init(id: String, name: String, kind: ChannelKind, position: Int, lastMessageAt: Int? = nil, bitrate: Int? = nil) {
        self.id = id
        self.name = name
        self.kind = kind
        self.position = position
        self.lastMessageAt = lastMessageAt
        self.bitrate = bitrate
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? id
        kind = try c.decodeIfPresent(ChannelKind.self, forKey: .kind) ?? .other
        position = try c.decodeIfPresent(Int.self, forKey: .position) ?? 0
        lastMessageAt = try c.decodeIfPresent(Int.self, forKey: .lastMessageAt)
        bitrate = try? c.decodeIfPresent(Int.self, forKey: .bitrate)
    }
}

/// `pending` = in the waiting room; `banned` = turned away / removed.
public enum Role: String, Codable, Sendable, Hashable {
    case owner, mod, member, pending, banned

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = Role(rawValue: raw) ?? .member
    }

    /// Owner, mod or member: shown in member lists.
    public var isListed: Bool { self == .owner || self == .mod || self == .member }
    public var canModerate: Bool { self == .owner || self == .mod }
}

public struct User: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var name: String
    /// "#rrggbb"
    public var color: String
    public var role: Role
    /// Media id of their picture (served at /media/<id>), if any.
    public var avatar: String?
    /// Waiting-room entries only: whose invite they used, e.g. "Ada (for Ben)".
    public var invitedBy: String?
    /// Waiting-room entries only: the personal link they came with — its
    /// maker, its label, and who else signed up with it before.
    public var invite: InviteInfo?
    /// An AIM-style away message while it's on (PROTOCOL.md › Extras).
    public var away: Away?
    /// Their join chime: a built-in id, `s:<soundId>`, or nil = `default`.
    public var chime: String?

    public init(id: String, name: String, color: String, role: Role = .member, avatar: String? = nil,
                invitedBy: String? = nil, invite: InviteInfo? = nil, away: Away? = nil, chime: String? = nil) {
        self.id = id
        self.name = name
        self.color = color
        self.role = role
        self.avatar = avatar
        self.invitedBy = invitedBy
        self.invite = invite
        self.away = away
        self.chime = chime
    }

    /// The away message to show right now (nil once it's past `until`).
    public func awayNow(_ now: Date = Date()) -> Away? {
        guard let away, away.isOn(at: now) else { return nil }
        return away
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? "Someone"
        color = try c.decodeIfPresent(String.self, forKey: .color) ?? "#8d8d8d"
        role = try c.decodeIfPresent(Role.self, forKey: .role) ?? .member
        avatar = try c.decodeIfPresent(String.self, forKey: .avatar)
        invitedBy = try? c.decodeIfPresent(String.self, forKey: .invitedBy)
        invite = try? c.decodeIfPresent(InviteInfo.self, forKey: .invite)
        away = try? c.decodeIfPresent(Away.self, forKey: .away)
        chime = try? c.decodeIfPresent(String.self, forKey: .chime)
    }
}

/// A waiting-room entry's personal invite (`User.invite`, PROTOCOL.md ›
/// Identity › Waiting room): who made the link, its label, and the other
/// accounts that signed up with the same link before (oldest first, ≤ 10).
/// Links get passed on, so a non-empty `usedBy` is worth a word.
public struct InviteInfo: Codable, Sendable, Hashable {
    public var by: String
    public var label: String
    public var usedBy: [String]

    public init(by: String, label: String = "", usedBy: [String] = []) {
        self.by = by
        self.label = label
        self.usedBy = usedBy
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        by = (try? c.decodeIfPresent(String.self, forKey: .by)) ?? ""
        label = (try? c.decodeIfPresent(String.self, forKey: .label)) ?? ""
        usedBy = ((try? c.decodeIfPresent([String].self, forKey: .usedBy)) ?? []).filter { !$0.isEmpty }
    }

    /// "This link was also used by Ben — check with Ada." (nil when nobody else used it).
    public var warning: String? {
        guard !usedBy.isEmpty else { return nil }
        return "This link was also used by \(Self.names(usedBy)) — check with \(by.isEmpty ? "whoever made it" : by)."
    }

    /// Names as a sentence: "Ada", "Ada and Ben", "Ada, Ben and Cleo", "Ada, Ben and 2 others" (the web's `nameList`).
    public static func names(_ names: [String]) -> String {
        switch names.count {
        case 0...2: names.joined(separator: " and ")
        case 3: "\(names[0]), \(names[1]) and \(names[2])"
        default: "\(names[0]), \(names[1]) and \(names.count - 2) others"
        }
    }
}

public struct VoiceState: Codable, Sendable, Hashable {
    public var channelId: String
    public var muted: Bool
    public var deafened: Bool
    /// Sharing their screen in the call (PROTOCOL.md › Screen sharing): a
    /// LIVE badge, and Watch screen.
    public var screen: Bool
    /// Their app keeps calls across reconnects (PROTOCOL.md › Voice ›
    /// Keeping calls across reconnects).
    public var resume: Bool
    /// The earlier session whose call this one continues.
    public var prev: String?

    public init(channelId: String, muted: Bool, deafened: Bool, screen: Bool = false, resume: Bool = false, prev: String? = nil) {
        self.channelId = channelId
        self.muted = muted
        self.deafened = deafened
        self.screen = screen
        self.resume = resume
        self.prev = prev
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        channelId = try c.decode(String.self, forKey: .channelId)
        muted = try c.decodeIfPresent(Bool.self, forKey: .muted) ?? false
        deafened = try c.decodeIfPresent(Bool.self, forKey: .deafened) ?? false
        screen = (try? c.decodeIfPresent(Bool.self, forKey: .screen)) ?? false
        resume = (try? c.decodeIfPresent(Bool.self, forKey: .resume)) ?? false
        prev = try? c.decodeIfPresent(String.self, forKey: .prev)
    }
}

/// One live connection. A person can have several (phone + desktop).
public struct Session: Codable, Sendable, Hashable, Identifiable {
    public var sid: String
    public var uid: String
    public var client: String
    public var voice: VoiceState?
    /// Shared view this session is on (e.g. "board"), nil for chat.
    public var view: String?
    /// In the Watch together audience (PROTOCOL.md › Watch together).
    public var watching: Bool

    public var id: String { sid }

    /// A listen-only OBS link (`client: "obs"`): the call still connects to
    /// it, but it never shows in voice (no row, no chimes, not counted) and
    /// never counts as that person being online.
    public var isListener: Bool { client == "obs" }

    public init(sid: String, uid: String, client: String = "", voice: VoiceState? = nil, view: String? = nil, watching: Bool = false) {
        self.sid = sid
        self.uid = uid
        self.client = client
        self.voice = voice
        self.view = view
        self.watching = watching
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        sid = try c.decode(String.self, forKey: .sid)
        uid = try c.decode(String.self, forKey: .uid)
        client = try c.decodeIfPresent(String.self, forKey: .client) ?? ""
        voice = try c.decodeIfPresent(VoiceState.self, forKey: .voice)
        view = try c.decodeIfPresent(String.self, forKey: .view)
        watching = (try? c.decodeIfPresent(Bool.self, forKey: .watching)) ?? false
    }
}

/// Watch together's one shared YouTube player (PROTOCOL.md › Watch together).
/// The apps only show what's on; watching happens on the web for now.
public struct WatchState: Decodable, Sendable, Equatable {
    public struct Queued: Decodable, Sendable, Equatable {
        public var id: String
        public var title: String?
        public var by: String?
    }

    public var videoId: String?
    public var title: String?
    public var playing: Bool
    /// Seconds into the video at server time `at`.
    public var position: Double
    public var at: Int
    public var by: String?
    public var queue: [Queued]

    public init(videoId: String? = nil, title: String? = nil, playing: Bool = false, position: Double = 0, at: Int = 0,
                by: String? = nil, queue: [Queued] = []) {
        self.videoId = videoId
        self.title = title
        self.playing = playing
        self.position = position
        self.at = at
        self.by = by
        self.queue = queue
    }

    private enum CodingKeys: String, CodingKey { case videoId, title, playing, position, at, by, queue }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        videoId = try? c.decodeIfPresent(String.self, forKey: .videoId)
        title = try? c.decodeIfPresent(String.self, forKey: .title)
        playing = (try? c.decodeIfPresent(Bool.self, forKey: .playing)) ?? false
        position = (try? c.decodeIfPresent(Double.self, forKey: .position)) ?? 0
        at = (try? c.decodeIfPresent(Int.self, forKey: .at)) ?? 0
        by = try? c.decodeIfPresent(String.self, forKey: .by)
        queue = (try? c.decodeIfPresent([Queued].self, forKey: .queue)) ?? []
    }
}

/// A GIF picked from KLIPY. URLs are KLIPY's own (static*.klipy.com), used
/// exactly as their API returned them; their terms require it.
public struct Gif: Codable, Sendable, Hashable {
    public var provider: String
    public var slug: String
    public var title: String
    public var width: Int
    public var height: Int
    public var mp4: String?
    public var webm: String?
    public var webp: String?
    public var gif: String?
    /// First-frame JPEG, for posters and reduced motion.
    public var still: String?

    public init(provider: String = "klipy", slug: String, title: String, width: Int, height: Int,
                mp4: String? = nil, webm: String? = nil, webp: String? = nil, gif: String? = nil, still: String? = nil) {
        self.provider = provider
        self.slug = slug
        self.title = title
        self.width = width
        self.height = height
        self.mp4 = mp4
        self.webm = webm
        self.webp = webp
        self.gif = gif
        self.still = still
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        provider = try c.decodeIfPresent(String.self, forKey: .provider) ?? "klipy"
        slug = try c.decodeIfPresent(String.self, forKey: .slug) ?? ""
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        width = max(1, try c.decodeIfPresent(Int.self, forKey: .width) ?? 1)
        height = max(1, try c.decodeIfPresent(Int.self, forKey: .height) ?? 1)
        mp4 = try c.decodeIfPresent(String.self, forKey: .mp4)
        webm = try c.decodeIfPresent(String.self, forKey: .webm)
        webp = try c.decodeIfPresent(String.self, forKey: .webp)
        gif = try c.decodeIfPresent(String.self, forKey: .gif)
        still = try c.decodeIfPresent(String.self, forKey: .still)
    }

    public var aspectRatio: Double { Double(width) / Double(height) }
    /// AVPlayer plays mp4 (not webm), so that's the only video we use.
    public var videoURL: URL? { mp4.flatMap(URL.init(string:)) }
    /// Animated image fallback when there's no mp4.
    public var imageURL: URL? { (webp ?? gif).flatMap(URL.init(string:)) }
    public var stillURL: URL? { still.flatMap(URL.init(string:)) }

    /// Display size inside a `maxSide` box, keeping the aspect ratio (Yap uses 300).
    public func fittedSize(maxSide: Double = 300) -> (width: Double, height: Double) {
        let scale = min(1, maxSide / Double(width), maxSide / Double(height))
        return ((Double(width) * scale).rounded(), (Double(height) * scale).rounded())
    }
}

/// A custom emoji. `id` = first 32 hex chars of the image's SHA-256.
public struct Emoji: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var name: String
    public var mime: String
    public var uid: String
    public var createdAt: Int

    public init(id: String, name: String, mime: String = "image/png", uid: String = "", createdAt: Int = 0) {
        self.id = id
        self.name = name
        self.mime = mime
        self.uid = uid
        self.createdAt = createdAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        name = try c.decode(String.self, forKey: .name)
        mime = try c.decodeIfPresent(String.self, forKey: .mime) ?? "image/png"
        uid = try c.decodeIfPresent(String.self, forKey: .uid) ?? ""
        createdAt = try c.decodeIfPresent(Int.self, forKey: .createdAt) ?? 0
    }
}

/// One emoji's reactions on a message: a character, or a custom `<:name:id>` token.
public struct Reaction: Codable, Sendable, Hashable, Identifiable {
    public var emoji: String
    public var uids: [String]
    public var id: String { emoji }

    public init(emoji: String, uids: [String]) {
        self.emoji = emoji
        self.uids = uids
    }
}

public struct Message: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var channelId: String
    public var uid: String
    /// May be empty when the message is just a GIF.
    public var body: String
    /// ms since epoch; strictly increasing server-wide (a safe sort key).
    public var ts: Int
    public var gif: Gif?
    /// A picture (`POST /api/image`), served at `/media/<id>`.
    public var image: ImageRef?
    /// In first-reaction order (as the server sends them).
    public var reactions: [Reaction] = []
    /// Set once the author has edited it (ms since epoch).
    public var editedAt: Int?
    /// The message this one answers (filled in by the server).
    public var reply: Reply?
    /// When it was pinned to its channel (ms since epoch), if it is.
    public var pinnedAt: Int?
    /// A poll: the body is its question.
    public var poll: Poll?
    /// `/roll` or `/flip`, rolled by the server; the body is the result text.
    public var roll: Roll?
    /// Client-only: an optimistic copy waiting for the server's echo.
    public var isPending: Bool = false

    enum CodingKeys: String, CodingKey { case id, channelId, uid, body, ts, gif, image, reactions, editedAt, reply, pinnedAt, poll, roll }

    public init(id: String, channelId: String, uid: String, body: String, ts: Int, gif: Gif? = nil, image: ImageRef? = nil,
                reactions: [Reaction] = [], editedAt: Int? = nil, reply: Reply? = nil, poll: Poll? = nil, roll: Roll? = nil,
                isPending: Bool = false) {
        self.id = id
        self.channelId = channelId
        self.uid = uid
        self.body = body
        self.ts = ts
        self.gif = gif
        self.image = image
        self.reactions = reactions
        self.editedAt = editedAt
        self.reply = reply
        self.poll = poll
        self.roll = roll
        self.isPending = isPending
    }

    /// Apply a `reaction` event (add/remove one person's emoji).
    public mutating func applyReaction(emoji: String, uid: String, on: Bool) {
        if let i = reactions.firstIndex(where: { $0.emoji == emoji }) {
            var uids = reactions[i].uids.filter { $0 != uid }
            if on { uids.append(uid) }
            if uids.isEmpty { reactions.remove(at: i) } else { reactions[i].uids = uids }
        } else if on {
            reactions.append(Reaction(emoji: emoji, uids: [uid]))
        }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        channelId = try c.decode(String.self, forKey: .channelId)
        uid = try c.decode(String.self, forKey: .uid)
        body = try c.decodeIfPresent(String.self, forKey: .body) ?? ""
        ts = try c.decodeIfPresent(Int.self, forKey: .ts) ?? 0
        // A malformed GIF shouldn't hide the rest of the message.
        gif = try? c.decodeIfPresent(Gif.self, forKey: .gif)
        image = try? c.decodeIfPresent(ImageRef.self, forKey: .image)
        reactions = (try? c.decodeIfPresent([Reaction].self, forKey: .reactions)) ?? []
        editedAt = try? c.decodeIfPresent(Int.self, forKey: .editedAt)
        reply = try? c.decodeIfPresent(Reply.self, forKey: .reply)
        pinnedAt = try? c.decodeIfPresent(Int.self, forKey: .pinnedAt)
        poll = try? c.decodeIfPresent(Poll.self, forKey: .poll)
        roll = try? c.decodeIfPresent(Roll.self, forKey: .roll)
    }

    public var date: Date { Date(timeIntervalSince1970: Double(ts) / 1000) }

    /// Doodle reactions (`<d:ID>`), apart from the emoji ones.
    public var doodles: [Reaction] { reactions.filter { Doodle.id(fromReaction: $0.emoji) != nil } }
    /// Emoji reactions (characters and custom `<:name:id>` tokens).
    public var emojiReactions: [Reaction] { reactions.filter { Doodle.id(fromReaction: $0.emoji) == nil } }
}

/// What a reply quotes: the original's author and first 200 characters (or
/// that it had a GIF/picture, or that it's gone).
public struct Reply: Codable, Sendable, Hashable {
    public var id: String
    public var uid: String
    public var body: String
    public var gif: Bool
    public var image: Bool
    public var deleted: Bool
    /// Where the original lives (every `reply` carries it since v0.36; a
    /// Personal-space bookmark points into another channel).
    public var channelId: String?

    public init(id: String, uid: String, body: String, gif: Bool = false, image: Bool = false, deleted: Bool = false,
                channelId: String? = nil) {
        self.id = id
        self.uid = uid
        self.body = body
        self.gif = gif
        self.image = image
        self.deleted = deleted
        self.channelId = channelId
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        uid = try c.decodeIfPresent(String.self, forKey: .uid) ?? ""
        body = try c.decodeIfPresent(String.self, forKey: .body) ?? ""
        gif = (try? c.decodeIfPresent(Bool.self, forKey: .gif)) ?? false
        image = (try? c.decodeIfPresent(Bool.self, forKey: .image)) ?? false
        deleted = (try? c.decodeIfPresent(Bool.self, forKey: .deleted)) ?? false
        channelId = try? c.decodeIfPresent(String.self, forKey: .channelId)
    }

    /// A quote of `message` for my own pending copy, the way the server makes it.
    public init(quoting message: Message) {
        self.init(id: message.id, uid: message.uid, body: String(message.body.prefix(200)),
                  gif: message.gif != nil, image: message.image != nil, channelId: message.channelId)
    }

    /// "first line…" of the quote as plain text (formatting markers dropped,
    /// custom emoji as `:name:`, spoilers hidden).
    public var preview: String {
        if deleted { return "Original message was deleted" }
        let lines = body.split(whereSeparator: \.isNewline)
        let first = lines.first.map { MessageFormatter.flatten(String($0)) } ?? ""
        if first.isEmpty { return image ? "Picture" : gif ? "GIF" : "…" }
        var short = first.count > 80 ? String(first.prefix(80)) : first
        if first.count > 80 || lines.count > 1 || body.count >= 200 { short += "…" }
        return short
    }
}

/// Messages longer than this many characters or lines start collapsed.
public enum MessageLength {
    /// The server's cap on a message (an encrypted note may be longer).
    public static let maxChars = 40_000
    public static let maxSealedChars = 240_000
    public static let collapseChars = 3_000
    public static let collapseLines = 30

    public static func isLong(_ text: String) -> Bool {
        text.count > collapseChars || text.reduce(0) { $0 + ($1.isNewline ? 1 : 0) } >= collapseLines
    }

    /// The first ~3000 characters / 30 lines, cut at a space so a custom
    /// emoji token or a link isn't split.
    public static func collapsed(_ text: String) -> String {
        var lines = 0
        var end = text.startIndex
        var count = 0
        for i in text.indices {
            if count >= collapseChars || lines >= collapseLines { break }
            if text[i].isNewline { lines += 1 }
            count += 1
            end = text.index(after: i)
        }
        var cut = String(text[..<end])
        if end < text.endIndex, let space = cut.lastIndex(where: \.isWhitespace), cut.distance(from: space, to: cut.endIndex) < 200 {
            cut = String(cut[..<space])
        }
        while let last = cut.last, last.isWhitespace { cut.removeLast() }
        return cut + "…"
    }
}

/// A picture in a message: stored on the server, served at `/media/<id>`.
public struct ImageRef: Codable, Sendable, Hashable {
    public var id: String
    public var width: Int
    public var height: Int

    public init(id: String, width: Int, height: Int) {
        self.id = id
        self.width = width
        self.height = height
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        width = max(1, try c.decodeIfPresent(Int.self, forKey: .width) ?? 1)
        height = max(1, try c.decodeIfPresent(Int.self, forKey: .height) ?? 1)
    }

    /// Display size inside a `maxWidth` × `maxHeight` box, keeping the aspect ratio.
    public func fittedSize(maxWidth: Double, maxHeight: Double) -> (width: Double, height: Double) {
        let scale = min(1, maxWidth / Double(width), maxHeight / Double(height))
        return ((Double(width) * scale).rounded(), (Double(height) * scale).rounded())
    }
}

/// A personal invite link. The key itself is only shown once, when it's made.
public struct Invite: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var label: String
    /// uid of whoever made it.
    public var by: String
    public var createdAt: Int
    public var uses: Int
    public var lastUsedAt: Int?
    public var revoked: Bool

    public init(id: String, label: String, by: String, createdAt: Int, uses: Int = 0, lastUsedAt: Int? = nil, revoked: Bool = false) {
        self.id = id
        self.label = label
        self.by = by
        self.createdAt = createdAt
        self.uses = uses
        self.lastUsedAt = lastUsedAt
        self.revoked = revoked
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        label = try c.decodeIfPresent(String.self, forKey: .label) ?? "Invite"
        by = try c.decodeIfPresent(String.self, forKey: .by) ?? ""
        createdAt = try c.decodeIfPresent(Int.self, forKey: .createdAt) ?? 0
        uses = try c.decodeIfPresent(Int.self, forKey: .uses) ?? 0
        lastUsedAt = try? c.decodeIfPresent(Int.self, forKey: .lastUsedAt)
        revoked = try c.decodeIfPresent(Bool.self, forKey: .revoked) ?? false
    }
}

/// A sidebar Place: an outside https link, or (`view`) something that opens
/// inside Yap (today only `"board"`).
public struct PlaceLink: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    public var name: String
    public var url: String
    public var view: String?
    public var description: String?

    public init(id: String, name: String, url: String, view: String? = nil, description: String? = nil) {
        self.id = id
        self.name = name
        self.url = url
        self.view = view
        self.description = description
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? "Link"
        url = try c.decodeIfPresent(String.self, forKey: .url) ?? ""
        id = try c.decode(String.self, forKey: .id)
        view = try? c.decodeIfPresent(String.self, forKey: .view)
        description = try? c.decodeIfPresent(String.self, forKey: .description)
    }
}

/// One of my signed-in devices (Settings › Account).
public struct Device: Codable, Sendable, Hashable, Identifiable {
    /// First 16 hex chars of the token's hash: enough to pick it, useless to sign in with.
    public var id: String
    public var client: String
    /// "Chrome on Windows", "Yap for iPhone"…
    public var label: String
    public var createdAt: Int
    public var lastSeen: Int
    /// nil = full account; "listen" = an OBS link.
    public var scope: String?
    /// The device asking.
    public var current: Bool

    public init(id: String, client: String, label: String, createdAt: Int = 0, lastSeen: Int = 0, scope: String? = nil, current: Bool = false) {
        self.id = id
        self.client = client
        self.label = label
        self.createdAt = createdAt
        self.lastSeen = lastSeen
        self.scope = scope
        self.current = current
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        client = try c.decodeIfPresent(String.self, forKey: .client) ?? ""
        label = try c.decodeIfPresent(String.self, forKey: .label) ?? client
        createdAt = try c.decodeIfPresent(Int.self, forKey: .createdAt) ?? 0
        lastSeen = try c.decodeIfPresent(Int.self, forKey: .lastSeen) ?? 0
        scope = try? c.decodeIfPresent(String.self, forKey: .scope)
        current = (try? c.decodeIfPresent(Bool.self, forKey: .current)) ?? false
    }

    public var isListener: Bool { scope == "listen" }
}

/// A sign-in secret (PROTOCOL.md › Identity): a password of 8–128
/// characters. Accounts from before passwords may still sign in with their
/// old PIN, so signing in takes anything up to 128 characters and the
/// server judges it; only a new password has a shape to check here (the
/// server also refuses guessable ones, `weak_pin`).
public enum Secret {
    /// The shortest new password the server takes ("8 characters or more.").
    public static let minimum = 8
    public static let maximum = 128

    /// Long enough to send as a new password.
    public static func isPassword(_ s: String) -> Bool { (minimum...maximum).contains(s.count) }

    /// Something to sign in with: a password, or an old PIN.
    public static func canSignIn(_ s: String) -> Bool { (1...maximum).contains(s.count) }
}

/// One line of the moderation log: who did what to whom (never message text).
public struct ModLogEntry: Codable, Sendable, Hashable, Identifiable {
    public var ts: Int
    /// A uid, or "admin" for the admin API.
    public var actor: String?
    public var action: String
    public var target: String?
    public var detail: String?
    public var id: String { "\(ts)|\(action)|\(actor ?? "")|\(target ?? "")" }

    public init(ts: Int, actor: String?, action: String, target: String? = nil, detail: String? = nil) {
        self.ts = ts
        self.actor = actor
        self.action = action
        self.target = target
        self.detail = detail
    }

    public var date: Date { Date(timeIntervalSince1970: Double(ts) / 1000) }

    /// "let Ben in", "made an invite link (for Cleo)"… (after the actor's
    /// name; the web's words, `LOG_WORDS` in web/js/app/settings-ui.js).
    /// `name` turns a uid into a name.
    public func phrase(_ name: (String?) -> String) -> String {
        let detail = self.detail.flatMap { $0.isEmpty ? nil : $0 }
        let who = target == "admin" ? "the owner (admin key)" : name(target)
        switch action {
        case "member.approve": return "let \(who) in"
        case "member.deny": return "turned \(who) away"
        case "member.ban": return "banned \(who)"
        case "member.role":
            if detail == "banned" { return "banned \(who)" }
            if detail == "pending" { return "sent \(who) back to the waiting room" }
            // From the apps only the owner sets roles, and "member" means taking a mod's role away.
            if detail == "member" && actor != "admin" { return "made \(who) a member again (no longer a mod)" }
            let role = ["mod": "a mod", "member": "a member", "owner": "the owner"][detail ?? ""] ?? detail ?? "a member"
            return "made \(who) \(role)"
        case "member.resetPin": return "reset \(who)'s password"
        case "member.merge": return "merged a duplicate account into \(who)'s"
        case "member.unmerge": return "split a merged account back out"
        case "owner.claim": return "became the owner"
        case "message.delete": return "deleted a message by \(who)"
        case "channel.create": return "made a \(detail ?? "channel")"
        case "channel.archive": return detail.map { "archived the channel \($0)" } ?? "archived the channel"
        case "channel.unarchive": return "brought back the \(detail ?? "channel")"
        case "channel.bitrate":
            // detail: `"Lounge" → 256 kbps` or `"Lounge" → personal settings`
            guard let detail, let arrow = detail.range(of: " → ", options: .backwards), detail.hasPrefix("\""),
                  detail[..<arrow.lowerBound].hasSuffix("\"") else { return "changed a voice channel's quality" }
            let channel = detail[..<arrow.lowerBound]
            let to = detail[arrow.upperBound...]
            return to == "personal settings" ? "let everyone in \(channel) pick their own voice quality again"
                : "set the voice quality in \(channel) to \(to) for everyone"
        case "emoji.delete": return detail.map { "deleted the emoji \($0)" } ?? "deleted the emoji"
        case "invite.create": return "made an invite link" + (detail.map { " (\($0))" } ?? "")
        case "invite.revoke": return "turned off an invite link" + (detail.map { " (\($0))" } ?? "")
        case "invite.resetMain": return "reset the main link"
        case "link.add":
            if let detail, let arrow = detail.range(of: " → ", options: .backwards), arrow.lowerBound > detail.startIndex {
                return "added \(detail[..<arrow.lowerBound]) (\(detail[arrow.upperBound...])) to Places"
            }
            return "added \(detail ?? "a link") to Places"
        case "link.remove": return "removed \(detail ?? "a link") from Places"
        case "obs.create": return "made an OBS link"
        case "obs.revoke": return (Int(detail ?? "") ?? 0) > 1 ? "turned off their \(detail ?? "") OBS links" : "turned off their OBS link"
        case "sound.delete": return "deleted a soundboard clip by \(who)"
        default:
            // An action this app doesn't know yet (a newer server): its name in words.
            let words = action.replacingOccurrences(of: ".", with: " ").replacingOccurrences(of: "_", with: " ")
            return "did “\(words)”" + (detail.map { " (\($0))" } ?? "")
        }
    }
}

public extension Date {
    /// ms since epoch, the protocol's time unit.
    var yapMillis: Int { Int((timeIntervalSince1970 * 1000).rounded()) }
}
