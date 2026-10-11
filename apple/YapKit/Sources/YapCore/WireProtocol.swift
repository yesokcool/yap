import Foundation

/// Protocol version this client speaks (PROTOCOL.md, `hello.v`).
public let yapProtocolVersion = 1

// MARK: - Voice signaling payload

/// `signal.data`: an SDP or an ICE candidate (PROTOCOL.md › Voice), or a
/// viewer asking a sharer for their screen (`{screen: "want"|"stop"}`,
/// PROTOCOL.md › Screen sharing).
public enum SignalData: Codable, Sendable, Equatable {
    case sdp(SessionDescriptionPayload)
    case candidate(IceCandidatePayload)
    /// true = "want" (send me your screen), false = "stop".
    case screen(Bool)

    private enum Keys: String, CodingKey { case sdp, candidate, screen }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        if let sdp = try c.decodeIfPresent(SessionDescriptionPayload.self, forKey: .sdp) {
            self = .sdp(sdp)
        } else if let candidate = try c.decodeIfPresent(IceCandidatePayload.self, forKey: .candidate) {
            self = .candidate(candidate)
        } else if let screen = try? c.decodeIfPresent(String.self, forKey: .screen), screen == "want" || screen == "stop" {
            self = .screen(screen == "want")
        } else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "signal without sdp, candidate or screen"))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        switch self {
        case .sdp(let sdp): try c.encode(sdp, forKey: .sdp)
        case .candidate(let candidate): try c.encode(candidate, forKey: .candidate)
        case .screen(let want): try c.encode(want ? "want" : "stop", forKey: .screen)
        }
    }
}

public struct SessionDescriptionPayload: Codable, Sendable, Equatable {
    /// "offer" | "answer"
    public var type: String
    public var sdp: String

    public init(type: String, sdp: String) {
        self.type = type
        self.sdp = sdp
    }
}

public struct IceCandidatePayload: Codable, Sendable, Equatable {
    public var candidate: String
    public var sdpMid: String?
    public var sdpMLineIndex: Int?
    public var usernameFragment: String?

    public init(candidate: String, sdpMid: String?, sdpMLineIndex: Int?, usernameFragment: String? = nil) {
        self.candidate = candidate
        self.sdpMid = sdpMid
        self.sdpMLineIndex = sdpMLineIndex
        self.usernameFragment = usernameFragment
    }
}

// MARK: - Client → server

/// How `hello` identifies us (PROTOCOL.md › Identity).
public enum HelloAuth: Sendable, Equatable {
    /// Normal reconnect with this device's token.
    case token(String)
    /// New device / signed out: name + password (or an old PIN; the wire
    /// still calls it `pin`).
    case login(name: String, pin: String)
    /// Brand-new person (lands in the waiting room): `pin` is a password.
    case signup(name: String, pin: String, color: String)
}

public enum ClientMessage: Encodable, Sendable {
    case hello(HelloAuth, client: String)
    case accountPin(pin: String, current: String?)
    case accountSignout
    /// My signed-in devices → `devices`; sign one out; sign out all the others.
    case accountDevices
    case accountRevoke(id: String)
    case accountSignoutOthers
    case memberResetPin(uid: String)
    case history(channelId: String, before: Int?, beforeId: String?, limit: Int?)
    /// `poll`: its answers (the body is the question).
    case send(channelId: String, body: String, nonce: String?, gif: Gif?, image: ImageRef? = nil, replyTo: String? = nil,
              poll: [String]? = nil)
    /// `blobs`: my own encrypted note's picture ids, erased with it.
    case delete(id: String, blobs: [String]? = nil)
    /// Turn on / change / reset the encrypted Personal space (nil = off).
    case vaultSet(VaultWrap?)
    /// My Personal-space notes that aren't encrypted yet → `personal.plain`.
    case personalPlain
    /// Their sealed versions (≤ 50 per frame), swapped in place by the server.
    case personalSeal([SealedNote])
    /// Edit your own message's text (a GIF or picture stays).
    case edit(id: String, body: String)
    /// Personal invite links: make one (the key comes back once), list, revoke.
    case inviteCreate(label: String)
    case inviteList
    case inviteRevoke(id: String)
    /// Sidebar Places: add an https link, or remove one.
    case linkAdd(name: String, url: String)
    case linkRemove(id: String)
    /// Voice quality for everyone in a channel (bits/s), or nil = each person's own.
    case channelBitrate(channelId: String, bitrate: Int?)
    /// The moderation log (anyone may read it).
    case modlog
    /// Search messages (one channel, or everywhere) → `search.results`.
    case search(q: String, channelId: String?)
    /// Pin / unpin a message → `pinned` to everyone who can see it.
    case pin(id: String, on: Bool)
    /// A channel's pinned messages → `pins`.
    case pins(channelId: String)
    /// What Watch together is playing → `watch.state`.
    case watchGet
    /// A one-time sign-in pass for the web app (Draw, Space…) → `device.handoff`.
    case deviceHandoff
    case typing(channelId: String)
    case profile(name: String?, color: String?)
    /// `profile { avatar: null }`: remove your picture.
    case removeAvatar
    /// Add/remove your reaction (`emoji` = a character or a `<:name:id>` token).
    case react(id: String, emoji: String, on: Bool)
    /// 📣 Everyone online gets a ping.
    case ping(text: String?)
    case channelCreate(name: String, kind: ChannelKind)
    case channelArchive(channelId: String)
    /// `resume`: I keep calls across reconnects. `prev` + `proof` (that
    /// session's `ready.resumeKey`): this join continues `prev`'s call
    /// (PROTOCOL.md › Voice › Keeping calls across reconnects).
    /// `screen`: a share we're continuing after a reconnect (with `prev`).
    case voiceJoin(channelId: String, muted: Bool, deafened: Bool, resume: Bool = false, prev: String? = nil, proof: String? = nil, screen: Bool = false)
    case voiceLeave
    /// `screen`: we're sharing our screen (PROTOCOL.md › Screen sharing).
    case voiceState(muted: Bool, deafened: Bool, screen: Bool = false)
    case emojiDelete(id: String)
    case emojiRename(id: String, name: String)
    case memberApprove(uid: String)
    case memberDeny(uid: String)
    case memberBan(uid: String)
    case memberRole(uid: String, role: String)
    case view(String?)
    case signal(to: String, data: SignalData)

    // The v0.36 extras (PROTOCOL.md › Extras).
    /// Vote in a poll (nil = take my vote back) → `poll` to everyone.
    case pollVote(id: String, option: Int?)
    /// Play a soundboard clip for everyone in my voice channel → `sound`.
    case soundPlay(id: String)
    case soundDelete(id: String)
    /// Pictures + GIFs in a channel, newest first → `shelf`.
    case shelf(channelId: String, before: Int?)
    /// Unread counts since my read marks → `unread.counts`.
    case unreadCounts(since: [String: Int])
    /// Quiet hours (nil = off) → `notify`.
    case notifyQuiet(QuietHours?)
    /// Phone pushes via ntfy: "off" | "pings" | "all" → `notify`.
    case notifySettings(mode: String)
    /// Plan a hang (ms on a half-hour boundary) → `hang`; cancel → `hang.cancelled`.
    case hangSet(at: Int, title: String, channelId: String?)
    case hangCancel(id: String)
    /// `profile { away: { text, until } | null }`.
    case away(text: String, until: Int?)
    case awayOff
    /// `profile { chime }`: a built-in id, `s:<soundId>`, or nil = default.
    case chime(String?)
    /// Queue a YouTube video in Watch together (plays now if nothing's on).
    case watchQueue(add: String)

    public var type: String {
        switch self {
        case .hello: "hello"
        case .accountPin: "account.pin"
        case .accountSignout: "account.signout"
        case .accountDevices: "account.devices"
        case .accountRevoke: "account.revoke"
        case .accountSignoutOthers: "account.signoutOthers"
        case .memberResetPin: "member.resetPin"
        case .history: "history"
        case .send: "send"
        case .delete: "delete"
        case .vaultSet: "vault.set"
        case .personalPlain: "personal.plain"
        case .personalSeal: "personal.seal"
        case .edit: "edit"
        case .inviteCreate: "invite.create"
        case .inviteList: "invite.list"
        case .inviteRevoke: "invite.revoke"
        case .linkAdd: "link.add"
        case .linkRemove: "link.remove"
        case .channelBitrate: "channel.bitrate"
        case .modlog: "modlog"
        case .search: "search"
        case .pin: "pin"
        case .pins: "pins"
        case .watchGet: "watch.get"
        case .deviceHandoff: "device.handoff"
        case .typing: "typing"
        case .profile, .removeAvatar: "profile"
        case .react: "react"
        case .ping: "ping"
        case .channelCreate: "channel.create"
        case .channelArchive: "channel.archive"
        case .voiceJoin: "voice.join"
        case .voiceLeave: "voice.leave"
        case .voiceState: "voice.state"
        case .emojiDelete: "emoji.delete"
        case .emojiRename: "emoji.rename"
        case .memberApprove: "member.approve"
        case .memberDeny: "member.deny"
        case .memberBan: "member.ban"
        case .memberRole: "member.role"
        case .view: "view"
        case .signal: "signal"
        case .pollVote: "poll.vote"
        case .soundPlay: "sound.play"
        case .soundDelete: "sound.delete"
        case .shelf: "shelf"
        case .unreadCounts: "unread.counts"
        case .notifyQuiet: "notify.quiet"
        case .notifySettings: "notify.settings"
        case .hangSet: "hang.set"
        case .hangCancel: "hang.cancel"
        case .away, .awayOff, .chime: "profile"
        case .watchQueue: "watch.queue"
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: AnyKey.self)
        try c.encode(type, forKey: "t")
        switch self {
        case let .hello(auth, client):
            try c.encode(yapProtocolVersion, forKey: "v")
            try c.encode(client, forKey: "client")
            switch auth {
            case .token(let token):
                try c.encode(token, forKey: "token")
            case let .login(name, pin):
                try c.encode(["name": name, "pin": pin], forKey: "login")
            case let .signup(name, pin, color):
                try c.encode(["name": name, "pin": pin, "color": color], forKey: "signup")
            }
        case let .accountPin(pin, current):
            try c.encode(pin, forKey: "pin")
            try c.encodeIfPresent(current, forKey: "current")
        case .accountSignout, .voiceLeave, .inviteList, .modlog, .personalPlain, .accountDevices, .accountSignoutOthers, .watchGet, .deviceHandoff:
            break
        case let .memberResetPin(uid), let .memberApprove(uid), let .memberDeny(uid), let .memberBan(uid):
            try c.encode(uid, forKey: "uid")
        case let .history(channelId, before, beforeId, limit):
            try c.encode(channelId, forKey: "channelId")
            try c.encodeIfPresent(before, forKey: "before")
            try c.encodeIfPresent(beforeId, forKey: "beforeId")
            try c.encodeIfPresent(limit, forKey: "limit")
        case let .send(channelId, body, nonce, gif, image, replyTo, poll):
            try c.encode(channelId, forKey: "channelId")
            try c.encode(body, forKey: "body")
            try c.encodeIfPresent(nonce, forKey: "nonce")
            try c.encodeIfPresent(gif, forKey: "gif")
            try c.encodeIfPresent(image, forKey: "image")
            try c.encodeIfPresent(replyTo, forKey: "replyTo")
            if let poll { try c.encode(["options": poll], forKey: "poll") }
        case let .delete(id, blobs):
            try c.encode(id, forKey: "id")
            if let blobs, !blobs.isEmpty { try c.encode(blobs, forKey: "blobs") }
        case let .search(q, channelId):
            try c.encode(q, forKey: "q")
            try c.encodeIfPresent(channelId, forKey: "channelId")
        case let .pin(id, on):
            try c.encode(id, forKey: "id")
            try c.encode(on, forKey: "on")
        case let .pins(channelId):
            try c.encode(channelId, forKey: "channelId")
        case let .personalSeal(notes):
            try c.encode(notes, forKey: "notes")
        case let .vaultSet(wrap):
            if let wrap { try c.encode(wrap, forKey: "wrap") } else { try c.encodeNil(forKey: "wrap") }
        case let .emojiDelete(id), let .inviteRevoke(id), let .linkRemove(id), let .accountRevoke(id):
            try c.encode(id, forKey: "id")
        case let .edit(id, body):
            try c.encode(id, forKey: "id")
            try c.encode(body, forKey: "body")
        case let .inviteCreate(label):
            try c.encode(label, forKey: "label")
        case let .linkAdd(name, url):
            try c.encode(name, forKey: "name")
            try c.encode(url, forKey: "url")
        case let .channelBitrate(channelId, bitrate):
            try c.encode(channelId, forKey: "channelId")
            if let bitrate { try c.encode(bitrate, forKey: "bitrate") } else { try c.encodeNil(forKey: "bitrate") }
        case let .typing(channelId), let .channelArchive(channelId):
            try c.encode(channelId, forKey: "channelId")
        case let .profile(name, color):
            try c.encodeIfPresent(name, forKey: "name")
            try c.encodeIfPresent(color, forKey: "color")
        case .removeAvatar:
            try c.encodeNil(forKey: "avatar")
        case let .react(id, emoji, on):
            try c.encode(id, forKey: "id")
            try c.encode(emoji, forKey: "emoji")
            try c.encode(on, forKey: "on")
        case let .ping(text):
            try c.encodeIfPresent(text, forKey: "text")
        case let .channelCreate(name, kind):
            try c.encode(name, forKey: "name")
            try c.encode(kind.rawValue, forKey: "kind")
        case let .voiceJoin(channelId, muted, deafened, resume, prev, proof, screen):
            try c.encode(channelId, forKey: "channelId")
            try c.encode(muted, forKey: "muted")
            try c.encode(deafened, forKey: "deafened")
            if resume { try c.encode(true, forKey: "resume") }
            if let prev, let proof {
                try c.encode(prev, forKey: "prev")
                try c.encode(proof, forKey: "proof")
                if screen { try c.encode(true, forKey: "screen") }
            }
        case let .voiceState(muted, deafened, screen):
            try c.encode(muted, forKey: "muted")
            try c.encode(deafened, forKey: "deafened")
            if screen { try c.encode(true, forKey: "screen") }
        case let .emojiRename(id, name):
            try c.encode(id, forKey: "id")
            try c.encode(name, forKey: "name")
        case let .memberRole(uid, role):
            try c.encode(uid, forKey: "uid")
            try c.encode(role, forKey: "role")
        case let .view(view):
            if let view { try c.encode(view, forKey: "view") } else { try c.encodeNil(forKey: "view") }
        case let .signal(to, data):
            try c.encode(to, forKey: "to")
            try c.encode(data, forKey: "data")
        case let .pollVote(id, option):
            try c.encode(id, forKey: "id")
            if let option { try c.encode(option, forKey: "option") } else { try c.encodeNil(forKey: "option") }
        case let .soundPlay(id), let .soundDelete(id), let .hangCancel(id):
            try c.encode(id, forKey: "id")
        case let .shelf(channelId, before):
            try c.encode(channelId, forKey: "channelId")
            try c.encodeIfPresent(before, forKey: "before")
        case let .unreadCounts(since):
            try c.encode(since, forKey: "since")
        case let .notifyQuiet(quiet):
            if let quiet { try c.encode(quiet, forKey: "quiet") } else { try c.encodeNil(forKey: "quiet") }
        case let .notifySettings(mode):
            try c.encode(mode, forKey: "mode")
        case let .hangSet(at, title, channelId):
            try c.encode(at, forKey: "at")
            try c.encode(title, forKey: "title")
            try c.encodeIfPresent(channelId, forKey: "channelId")
        case let .away(text, until):
            var away = c.nestedContainer(keyedBy: AnyKey.self, forKey: "away")
            try away.encode(text, forKey: "text")
            if let until { try away.encode(until, forKey: "until") } else { try away.encodeNil(forKey: "until") }
        case .awayOff:
            try c.encodeNil(forKey: "away")
        case let .chime(chime):
            if let chime { try c.encode(chime, forKey: "chime") } else { try c.encodeNil(forKey: "chime") }
        case let .watchQueue(add):
            try c.encode(add, forKey: "add")
        }
    }

    /// The JSON text frame for this message.
    public func frame() throws -> String {
        let data = try JSONEncoder().encode(self)
        return String(decoding: data, as: UTF8.self)
    }
}

// MARK: - Server → client

public struct ReadyPayload: Decodable, Sendable {
    public var sid: String
    public var uid: String
    public var token: String?
    public var channels: [Channel]
    public var users: [User]
    public var sessions: [Session]
    public var emojis: [Emoji]
    public var pending: [User]
    /// The server's main invite key (members get it, so a revoked personal
    /// invite never locks them out). Absent on open servers.
    public var key: String?
    /// Sidebar Places.
    public var links: [PlaceLink] = []
    /// Private to this connection: uploads send it as `ut` next to `sid`.
    public var uploadToken: String?
    /// This session's voice resume key: a later session proves with it that
    /// it continues this one's call (PROTOCOL.md › Voice › Keeping calls
    /// across reconnects). Absent on servers that predate it.
    public var resumeKey: String?
    /// The encrypted Personal space's wrapped key; nil = encryption off.
    public var vault: VaultWrap?
    /// I signed in with an old PIN, a guessable password or a reset's
    /// temporary one: ask once for a password (PROTOCOL.md › Identity).
    public var weakPin: Bool = false
    /// Why `weakPin` (it picks the prompt's words): "pin", "temporary" (a
    /// reset's temporary password) or "guessable". Sent only with `weakPin`;
    /// absent on servers from before it.
    public var weakReason: String?
    /// What I last signed in with: "pin" or "password".
    public var secretKind: String?
    /// Every live soundboard sound and every chime someone uses (v0.36).
    public var sounds: [Sound] = []
    /// Upcoming hangs (and ones that started < 2 h ago).
    public var hangs: [Hang] = []
    /// My push settings, with quiet hours.
    public var notify: NotifySettings?

    public init(sid: String, uid: String, token: String? = nil, channels: [Channel], users: [User],
                sessions: [Session], emojis: [Emoji] = [], pending: [User] = [], key: String? = nil, links: [PlaceLink] = [],
                vault: VaultWrap? = nil) {
        self.sid = sid
        self.uid = uid
        self.token = token
        self.channels = channels
        self.users = users
        self.sessions = sessions
        self.emojis = emojis
        self.pending = pending
        self.key = key
        self.links = links
        self.vault = vault
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyKey.self)
        sid = try c.decode(String.self, forKey: "sid")
        uid = try c.decode(String.self, forKey: "uid")
        token = try c.decodeIfPresent(String.self, forKey: "token")
        channels = try c.decodeIfPresent([Channel].self, forKey: "channels") ?? []
        users = try c.decodeIfPresent([User].self, forKey: "users") ?? []
        sessions = try c.decodeIfPresent([Session].self, forKey: "sessions") ?? []
        emojis = try c.decodeIfPresent([Emoji].self, forKey: "emojis") ?? []
        pending = try c.decodeIfPresent([User].self, forKey: "pending") ?? []
        key = try? c.decodeIfPresent(String.self, forKey: "key")
        links = ((try? c.decodeIfPresent([Lenient<PlaceLink>].self, forKey: "links")) ?? []).compactMap(\.value)
        uploadToken = try? c.decodeIfPresent(String.self, forKey: "uploadToken")
        resumeKey = try? c.decodeIfPresent(String.self, forKey: "resumeKey")
        vault = try? c.decodeIfPresent(VaultWrap.self, forKey: "vault")
        weakPin = (try? c.decodeIfPresent(Bool.self, forKey: "weakPin")) ?? false
        weakReason = try? c.decodeIfPresent(String.self, forKey: "weakReason")
        secretKind = try? c.decodeIfPresent(String.self, forKey: "secretKind")
        sounds = ((try? c.decodeIfPresent([Lenient<Sound>].self, forKey: "sounds")) ?? []).compactMap(\.value)
        hangs = ((try? c.decodeIfPresent([Lenient<Hang>].self, forKey: "hangs")) ?? []).compactMap(\.value)
        notify = try? c.decodeIfPresent(NotifySettings.self, forKey: "notify")
    }
}

public enum ServerMessage: Sendable {
    case ready(ReadyPayload)
    /// Waiting room; `uid` = our new account's id.
    case pending(message: String, uid: String?, token: String?)
    case account(token: String?, kind: String?)
    case devices([Device])
    /// One-time sign-in pass (15 min) for the web app.
    case deviceHandoff(token: String)
    /// My account just signed in on another device.
    case signinAlert(Device)
    case pinReset(uid: String, pin: String)
    case joinRequest(User)
    /// Someone was let in or turned away. `user` (when let in, from newer
    /// servers): who joined, for everyone who never saw the request.
    case joinResolved(uid: String, approved: Bool, by: String, user: User? = nil)
    case error(code: String, message: String, re: String?)
    case history(channelId: String, messages: [Message], hasMore: Bool)
    case message(Message, nonce: String?)
    case deleted(id: String, channelId: String)
    /// `sealed`: a Personal-space note was just encrypted in place (drop its
    /// GIF/picture: they're inside the note); `editedAt` 0 = never edited.
    case edited(id: String, channelId: String, body: String, editedAt: Int, sealed: Bool)
    /// My Personal-space notes that are still plain (for sealing).
    case personalPlain([PlainNote])
    /// Newest first, ≤ 50 (encrypted notes never match on the server).
    case searchResults(q: String, channelId: String?, messages: [Message])
    case pinned(id: String, channelId: String, pinnedAt: Int?, by: String)
    case pins(channelId: String, messages: [Message])
    /// Only to the maker; the key is shown once.
    case inviteCreated(Invite, key: String)
    case invites([Invite])
    case links([PlaceLink])
    case modlog([ModLogEntry])
    /// My Personal space's wrapped key changed (nil = encryption turned off).
    case vault(VaultWrap?)
    case typing(channelId: String, uid: String)
    case user(User)
    case sessionJoin(Session)
    case sessionLeave(sid: String)
    case channel(Channel)
    case channelArchived(channelId: String)
    /// `resume`: that session keeps calls across reconnects; `prev`: it
    /// continues that earlier session's call; `lost`: it left because its
    /// socket closed (no `voice.leave`), so it may come back as a `prev`.
    case voice(sid: String, channelId: String?, muted: Bool, deafened: Bool, screen: Bool,
               resume: Bool = false, prev: String? = nil, lost: Bool = false)
    /// Someone joined (`on`) or left the Watch together audience.
    case watching(sid: String, on: Bool)
    /// What Watch together is playing; `now` is the server's clock.
    case watchState(WatchState, now: Int)
    case emoji(Emoji)
    case emojiDeleted(id: String)
    case view(sid: String, view: String?)
    /// `data` is nil when it isn't an SDP or a candidate we understand.
    case signal(from: String, data: SignalData?)
    case ping(from: String, text: String)
    case reaction(messageId: String, channelId: String, emoji: String, uid: String, on: Bool)
    /// Someone voted (or took a vote back): the poll's whole tally.
    case poll(id: String, channelId: String, votes: [[String]])
    /// A soundboard clip played in my voice channel (me included).
    case sound(id: String, sid: String, uid: String)
    case soundAdded(Sound)
    case soundDeleted(id: String)
    /// I @mentioned or replied to someone who's away (only I get this).
    case away(AwayNote)
    case shelf(channelId: String, items: [Message], hasMore: Bool)
    case unreadCounts([String: UnreadCount])
    case hang(Hang)
    case hangCancelled(id: String)
    /// A hang is starting now.
    case hangNow(id: String)
    /// My push settings (with quiet hours).
    case notify(NotifySettings)
    /// cursor, draw.*, space.*, anything newer than this build: ignored.
    case ignored(type: String)

    public static func decode(_ text: String) throws -> ServerMessage {
        try decode(Data(text.utf8))
    }

    public static func decode(_ data: Data) throws -> ServerMessage {
        try JSONDecoder().decode(Wrapper.self, from: data).message
    }

    private struct Wrapper: Decodable {
        let message: ServerMessage

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: AnyKey.self)
            let t = try c.decode(String.self, forKey: "t")
            func str(_ key: AnyKey) throws -> String { try c.decode(String.self, forKey: key) }
            func optStr(_ key: AnyKey) throws -> String? { try c.decodeIfPresent(String.self, forKey: key) }
            func bool(_ key: AnyKey) throws -> Bool { try c.decodeIfPresent(Bool.self, forKey: key) ?? false }
            switch t {
            case "ready":
                message = .ready(try ReadyPayload(from: decoder))
            case "pending":
                message = .pending(message: try optStr("message") ?? "Waiting for the owner or a mod to let you in.", uid: try optStr("uid"), token: try optStr("token"))
            case "account":
                message = .account(token: try optStr("token"), kind: try? optStr("kind"))
            case "devices":
                message = .devices((try? c.decodeIfPresent([Device].self, forKey: "devices")) ?? [])
            case "device.handoff":
                message = .deviceHandoff(token: try str("token"))
            case "signin.alert":
                message = .signinAlert(try c.decode(Device.self, forKey: "device"))
            case "pin.reset":
                message = .pinReset(uid: try str("uid"), pin: try str("pin"))
            case "join.request":
                message = .joinRequest(try c.decode(User.self, forKey: "user"))
            case "join.resolved":
                message = .joinResolved(uid: try str("uid"), approved: try bool("approved"), by: try optStr("by") ?? "",
                                        user: try? c.decodeIfPresent(User.self, forKey: "user"))
            case "error":
                message = .error(code: try optStr("code") ?? "internal", message: try optStr("message") ?? "Something went wrong.", re: try optStr("re"))
            case "history":
                message = .history(channelId: try str("channelId"),
                                   messages: try c.decodeIfPresent([Message].self, forKey: "messages") ?? [],
                                   hasMore: try bool("hasMore"))
            case "message":
                message = .message(try c.decode(Message.self, forKey: "message"), nonce: try optStr("nonce"))
            case "deleted":
                message = .deleted(id: try str("id"), channelId: try str("channelId"))
            case "edited":
                message = .edited(id: try str("id"), channelId: try str("channelId"), body: try optStr("body") ?? "",
                                  editedAt: (try? c.decodeIfPresent(Int.self, forKey: "editedAt")) ?? 0, sealed: try bool("sealed"))
            case "search.results":
                message = .searchResults(q: try optStr("q") ?? "", channelId: try optStr("channelId"),
                                         messages: (try? c.decodeIfPresent([Message].self, forKey: "messages")) ?? [])
            case "pinned":
                message = .pinned(id: try str("id"), channelId: try str("channelId"),
                                  pinnedAt: try? c.decodeIfPresent(Int.self, forKey: "pinnedAt"), by: try optStr("by") ?? "")
            case "pins":
                message = .pins(channelId: try str("channelId"), messages: (try? c.decodeIfPresent([Message].self, forKey: "messages")) ?? [])
            case "personal.plain":
                message = .personalPlain((try? c.decodeIfPresent([PlainNote].self, forKey: "notes")) ?? [])
            case "invite.created":
                message = .inviteCreated(try c.decode(Invite.self, forKey: "invite"), key: try str("key"))
            case "invites":
                message = .invites((try? c.decodeIfPresent([Invite].self, forKey: "invites")) ?? [])
            case "links":
                message = .links(((try? c.decodeIfPresent([Lenient<PlaceLink>].self, forKey: "links")) ?? []).compactMap(\.value))
            case "vault":
                message = .vault(try? c.decodeIfPresent(VaultWrap.self, forKey: "wrap"))
            case "modlog":
                message = .modlog((try? c.decodeIfPresent([ModLogEntry].self, forKey: "entries")) ?? [])
            case "typing":
                message = .typing(channelId: try str("channelId"), uid: try str("uid"))
            case "user":
                message = .user(try c.decode(User.self, forKey: "user"))
            case "session.join":
                message = .sessionJoin(try c.decode(Session.self, forKey: "session"))
            case "session.leave":
                message = .sessionLeave(sid: try str("sid"))
            case "channel":
                message = .channel(try c.decode(Channel.self, forKey: "channel"))
            case "channel.archived":
                message = .channelArchived(channelId: try str("channelId"))
            case "voice":
                message = .voice(sid: try str("sid"), channelId: try optStr("channelId"), muted: try bool("muted"),
                                 deafened: try bool("deafened"), screen: (try? bool("screen")) ?? false,
                                 resume: (try? bool("resume")) ?? false, prev: try? optStr("prev"), lost: (try? bool("lost")) ?? false)
            case "watching":
                message = .watching(sid: try str("sid"), on: try bool("on"))
            case "watch.state":
                message = .watchState(try c.decode(WatchState.self, forKey: "state"), now: (try? c.decodeIfPresent(Int.self, forKey: "now")) ?? 0)
            case "emoji":
                message = .emoji(try c.decode(Emoji.self, forKey: "emoji"))
            case "emoji.deleted":
                message = .emojiDeleted(id: try str("id"))
            case "view":
                message = .view(sid: try str("sid"), view: try optStr("view"))
            case "signal":
                message = .signal(from: try str("from"), data: try? c.decode(SignalData.self, forKey: "data"))
            case "reaction":
                message = .reaction(messageId: try str("messageId"), channelId: try str("channelId"), emoji: try str("emoji"), uid: try str("uid"), on: try bool("on"))
            case "ping":
                message = .ping(from: try optStr("from") ?? "", text: try optStr("text") ?? "")
            case "poll":
                message = .poll(id: try str("id"), channelId: try str("channelId"),
                                votes: (try? c.decodeIfPresent([[String]].self, forKey: "votes")) ?? [])
            case "sound":
                message = .sound(id: try str("id"), sid: try optStr("sid") ?? "", uid: try optStr("uid") ?? "")
            case "sound.added":
                message = .soundAdded(try c.decode(Sound.self, forKey: "sound"))
            case "sound.deleted":
                message = .soundDeleted(id: try str("id"))
            case "away":
                message = .away(AwayNote(uid: try str("uid"), text: try optStr("text") ?? "",
                                         until: try? c.decodeIfPresent(Int.self, forKey: "until")))
            case "shelf":
                message = .shelf(channelId: try str("channelId"),
                                 items: ((try? c.decodeIfPresent([Lenient<Message>].self, forKey: "items")) ?? []).compactMap(\.value),
                                 hasMore: try bool("hasMore"))
            case "unread.counts":
                message = .unreadCounts((try? c.decodeIfPresent([String: UnreadCount].self, forKey: "counts")) ?? [:])
            case "hang":
                message = .hang(try c.decode(Hang.self, forKey: "hang"))
            case "hang.cancelled":
                message = .hangCancelled(id: try str("id"))
            case "hang.now":
                message = .hangNow(id: try str("id"))
            case "notify":
                message = .notify(try NotifySettings(from: decoder))
            default:
                message = .ignored(type: t)
            }
        }
    }
}

/// A Personal-space note written before encryption was turned on.
public struct PlainNote: Decodable, Sendable, Equatable {
    public var id: String
    public var body: String
    public var gif: Gif?
    public var image: ImageRef?

    public init(id: String, body: String, gif: Gif? = nil, image: ImageRef? = nil) {
        self.id = id
        self.body = body
        self.gif = gif
        self.image = image
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: AnyKey.self)
        id = try c.decode(String.self, forKey: "id")
        body = try c.decodeIfPresent(String.self, forKey: "body") ?? ""
        gif = try? c.decodeIfPresent(Gif.self, forKey: "gif")
        image = try? c.decodeIfPresent(ImageRef.self, forKey: "image")
    }
}

/// A note's sealed body, handed back with `personal.seal`.
public struct SealedNote: Codable, Sendable, Equatable {
    public var id: String
    public var body: String

    public init(id: String, body: String) {
        self.id = id
        self.body = body
    }

    /// `personal.seal` takes 50 notes a frame at most…
    public static let maxPerFrame = 50
    /// …and frames stay well under the server's 384 KB cap.
    public static let maxFrameBytes = 300_000

    /// Roughly what one note adds to a `personal.seal` frame (`{"id":…,"body":…},`).
    public static func frameBytes(id: String, body: String) -> Int {
        id.utf8.count + body.utf8.count + 24
    }
}

/// Decodes a list item without failing the whole list (a newer server's
/// unknown shape just drops that one item).
struct Lenient<T: Decodable & Sendable>: Decodable, Sendable {
    var value: T?
    init(from decoder: Decoder) throws { value = try? T(from: decoder) }
}

/// A string-keyed CodingKey, for frames whose keys depend on the type.
struct AnyKey: CodingKey, ExpressibleByStringLiteral {
    var stringValue: String
    var intValue: Int? { nil }
    init(stringValue: String) { self.stringValue = stringValue }
    init?(intValue: Int) { nil }
    init(stringLiteral value: String) { stringValue = value }
}
