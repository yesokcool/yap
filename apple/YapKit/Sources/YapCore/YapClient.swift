import CryptoKit
import Foundation
import Observation

/// The whole client side of the protocol: connection, identity, and the
/// state every screen reads. One per app (or two in a test, as two "devices").
@MainActor
@Observable
public final class YapClient {
    public enum Phase: Equatable, Sendable {
        /// No server/invite key yet.
        case needsInvite
        /// Have an invite; need "I'm new" or "I have an account".
        case signedOut
        /// Socket opening / hello sent.
        case connecting
        case online
        /// Socket dropped; retrying with backoff.
        case reconnecting
        /// In the waiting room until a member lets us in.
        case waiting(String)
        /// Banned or turned away: no more reconnects.
        case blocked(title: String, message: String)
    }

    public enum AuthMode: Sendable { case signup, login }

    public struct AuthError: Equatable, Sendable {
        public var code: String
        public var message: String
        /// Which form the error belongs to.
        public var mode: AuthMode
    }

    public struct Notice: Identifiable, Equatable, Sendable {
        public let id = UUID()
        public var text: String
        public init(text: String) { self.text = text }
    }

    public struct ChannelHistory: Sendable, Equatable {
        public var messages: [Message] = []
        public var hasMore = true
        public var loading = true
        var loadingOlder = false
    }

    public enum PinChange: Equatable, Sendable {
        case idle, saving, saved
        case failed(String)
    }

    /// Why "Choose a password" comes up (`ready.weakPin`), which picks its
    /// words; the raw values are `ready.weakReason`'s.
    public enum PasswordNudge: String, Sendable {
        /// Still on a 6-digit PIN from before passwords.
        case pin
        /// Signed in with the temporary password an owner/mod reset handed out.
        case temporary
        /// A password the server now finds easy to guess.
        case guessable
    }

    /// A reset's temporary password (`pin.reset`; the wire keeps "pin").
    public struct PinReset: Identifiable, Equatable, Sendable {
        public let id = UUID()
        public var name: String
        public var pin: String
    }

    /// A message the server turned away, back for the message box.
    public struct ReturnedDraft: Identifiable, Equatable, Sendable {
        public let id = UUID()
        public var channelId: String
        public var text: String
    }

    /// A personal invite just made: its link is shown once (only a hash is kept).
    public struct CreatedInvite: Identifiable, Equatable, Sendable {
        public var invite: Invite
        public var link: String
        public var id: String { invite.id }
    }

    // MARK: Observable state

    public internal(set) var phase: Phase
    public internal(set) var authError: AuthError?
    public var notice: Notice?
    public internal(set) var endpoint: ServerEndpoint?
    /// Canonical account id (`ready.uid`). Empty until the first ready.
    public internal(set) var uid: String
    public internal(set) var mySid: String?
    /// `ready.uploadToken`: proves uploads come from this connection.
    @ObservationIgnored public internal(set) var uploadToken: String?
    /// `ready.resumeKey`: proves a later session continues this one's call.
    /// Kept until the next `ready` (voice reads it when the socket drops).
    @ObservationIgnored public internal(set) var resumeKey: String?
    /// Sessions whose socket dropped mid-call while voice keeps their
    /// connection, in case they resume ("reconnecting…"); they stay listed.
    public internal(set) var heldSessions: Set<String> = []
    public var pinChange: PinChange = .idle
    public var pinReset: PinReset?

    public internal(set) var channelsById: [String: Channel] = [:]
    public internal(set) var users: [String: User] = [:]
    public internal(set) var sessions: [String: Session] = [:]
    public internal(set) var emojis: [String: Emoji] = [:]
    public internal(set) var waitingRoom: [String: User] = [:]
    public internal(set) var histories: [String: ChannelHistory] = [:]
    /// nonce → optimistic copy, shown dimmed until the server echoes it.
    public internal(set) var outbox: [String: Message] = [:]
    /// The server turned a message away for good: its text, for the
    /// message box (only if that's empty and the channel's still open).
    public var returnedDraft: ReturnedDraft?
    /// My sends in flight, and the line `slow_down` puts them in.
    @ObservationIgnored var sendLine = SendLine()
    @ObservationIgnored var pacer: Task<Void, Never>?
    /// channelId → uid → when their "typing" expires.
    public internal(set) var typing: [String: [String: Date]] = [:]
    public internal(set) var lastRead: [String: Int]
    /// The channel I just created (UI opens it).
    public internal(set) var createdChannel: Channel?
    /// channelId → @mentions of me not yet seen (live ones, and those that
    /// came while I was away, from `unread.counts`).
    public internal(set) var mentionCounts: [String: Int] = [:]
    /// This session's quiet lines in channels ("Cleo joined — invited by
    /// Ada", "You're in. Say hi 👋"): never stored, never a popup.
    public internal(set) var localNotes: [LocalNote] = []
    /// Sidebar Places from the server (`ready.links`, live `links`).
    public internal(set) var links: [PlaceLink] = []
    /// My personal invites (everyone's for owner/mods), after `refreshInvites`.
    public internal(set) var invites: [Invite] = []
    /// The invite I just made, with its one-time link.
    public var createdInvite: CreatedInvite?
    /// The moderation log, after `refreshModLog`.
    public internal(set) var modLog: [ModLogEntry]?
    /// I signed in with an old PIN, a guessable password or a temporary
    /// one (`ready.weakPin`): ask once for a password (`passwordNudge`).
    public internal(set) var weakPin = false
    /// Why `weakPin` (`ready.weakReason`: "pin", "temporary" or "guessable");
    /// nil from servers from before it.
    public internal(set) var weakReason: String?
    /// What I last signed in with: "pin" or "password" (`ready.secretKind`).
    public internal(set) var secretKind: String?
    /// The latest search: what was asked, where, and the results (newest first).
    public internal(set) var search: SearchResults?
    /// channelId → its pinned messages (most recently pinned first), once asked for.
    public internal(set) var pins: [String: [Message]] = [:]
    /// What Watch together is playing, fetched (`watch.get`) once someone is
    /// in its audience; `watch.state` broadcasts keep it current.
    public internal(set) var watch: WatchState?
    @ObservationIgnored var watchRequested = false
    /// My signed-in devices, after `refreshDevices`.
    public internal(set) var devices: [Device]?
    /// "New sign-in: …" — my account just signed in somewhere else.
    public var signinAlert: Device?
    /// My Personal space's wrapped key (nil = not encrypted).
    public internal(set) var vaultWrap: VaultWrap?
    /// This device holds the space key, so notes open.
    public internal(set) var vaultUnlocked = false
    @ObservationIgnored var vaultKey: SymmetricKey?
    /// The `wrap.ct` the key in hand came from.
    @ObservationIgnored var vaultKeyTag: String?
    /// Waiting for `personal.plain` (sealing old notes).
    @ObservationIgnored var plainWaiter: (([PlainNote]) -> Void)?
    /// Sealing old notes: each `edited {sealed: true}` is one confirmed.
    @ObservationIgnored var sealWaiter: ((String) -> Void)?
    /// Waiting for `device.handoff` (signing the web app in).
    @ObservationIgnored var handoffWaiter: ((String?) -> Void)?
    /// What to go back to if the server refuses a `vault.set`.
    @ObservationIgnored var vaultBefore: (wrap: VaultWrap?, key: SymmetricKey?, tag: String?)?
    /// Encrypted note body → what it says (or that it won't open).
    @ObservationIgnored var noteCache: [String: NoteView] = [:]

    // MARK: Extras (v0.36)

    /// Soundboard sounds and the chimes people use (`ready.sounds`, live
    /// `sound.added` / `sound.deleted`), by id.
    public internal(set) var sounds: [String: Sound] = [:]
    /// Upcoming hangs, soonest first (and ones that started < 2 h ago).
    public internal(set) var hangs: [Hang] = []
    /// A hang that just started (`hang.now`): a calm notice with Join voice.
    public var hangStarting: Hang?
    /// My push settings and quiet hours (`ready.notify`, live `notify`).
    public internal(set) var notifySettings: NotifySettings?
    /// Catch up: unread counts per channel since my read marks, once asked
    /// for (`requestUnreadCounts`); nil while it hasn't come back.
    public internal(set) var unreadCounts: [String: UnreadCount]?
    /// Media shelves (pictures + GIFs), per channel, once asked for.
    public internal(set) var shelves: [String: Shelf] = [:]
    /// My message id (or optimistic nonce) → "Ada is away: …" lines under it.
    public internal(set) var awayNotes: [String: [AwayNote]] = [:]
    /// A soundboard clip played in my voice channel: (sound, who played it).
    @ObservationIgnored public var onSound: ((Sound, _ sid: String, _ uid: String) -> Void)?
    /// A hang is starting (`hang.now`).
    @ObservationIgnored public var onHangNow: ((Hang) -> Void)?
    /// I was just let in from the waiting room (the app chimes).
    @ObservationIgnored public var onAdmitted: (() -> Void)?
    /// Saved to Personal: the first time with encryption on, say what the server sees.
    @ObservationIgnored public var savedToPersonalBefore: () -> Bool = { true }

    public struct Shelf: Sendable, Equatable {
        public var items: [Message] = []
        public var hasMore = true
        public var loading = true
    }

    /// The text channel on screen (for unread + mark-read).
    public var activeChannelId: String? {
        didSet { if let activeChannelId { markRead(activeChannelId) } }
    }
    public var isAppActive = true {
        didSet {
            if isAppActive, let activeChannelId { markRead(activeChannelId) }
        }
    }

    /// Hooks for sounds and local notifications (the app decides what to do).
    @ObservationIgnored public var onPing: ((User?, String) -> Void)?
    /// A new message that @-mentions me (or @everyone / @here), from someone else.
    @ObservationIgnored public var onMention: ((Message) -> Void)?
    @ObservationIgnored public weak var voice: (any VoiceCoordinator)?
    /// Skin tone for `:shortcodes:` typed in messages (0 = default; the app keeps it in sync).
    @ObservationIgnored public var emojiTone = 0

    // MARK: Internals

    @ObservationIgnored let credentials: any CredentialStore
    @ObservationIgnored let clientLabel: String
    @ObservationIgnored let urlSession: URLSession
    @ObservationIgnored var connection: YapConnection?
    @ObservationIgnored var pendingHello: HelloAuth?
    @ObservationIgnored var lastTypingSent: [String: Date] = [:]
    @ObservationIgnored var pendingCreate: (name: String, kind: ChannelKind)?

    /// Messages per history page (tests use small pages).
    public static var historyPage = 50
    public static let typingSendEvery: TimeInterval = 3.5
    public static let typingShowFor: TimeInterval = 6

    /// `client` is the short label the server shows (`ios`, `mac`).
    public init(credentials: any CredentialStore, client: String, urlSession: URLSession = .shared) {
        self.credentials = credentials
        clientLabel = client
        self.urlSession = urlSession
        endpoint = credentials.endpoint
        uid = credentials.uid ?? ""
        lastRead = credentials.lastRead
        if credentials.endpoint == nil {
            phase = .needsInvite
        } else if credentials.token == nil {
            phase = .signedOut
        } else {
            phase = .connecting
        }
    }

    /// Bumped whenever the signed-in session ends; work that awaited across
    /// it (an upload) checks it before touching state or sending.
    @ObservationIgnored var session = 0

    /// Channels whose pins were asked for while offline; asked again once ready.
    @ObservationIgnored var pinsWanted: Set<String> = []

    @ObservationIgnored var shelfWanted: Set<String> = []
}
