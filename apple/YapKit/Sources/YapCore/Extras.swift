import Foundation

// The v0.36 extras (PROTOCOL.md › Extras): shapes and the pure rules every
// client applies the same way. Decoding stays tolerant, like Models.swift.

// MARK: - Away messages

/// `User.away`: an AIM-style away message. It ends by itself at `until`
/// (nil = "until I'm back"), or when its owner talks again.
public struct Away: Codable, Sendable, Hashable {
    public var text: String
    /// ms since epoch; nil = until I'm back.
    public var until: Int?
    /// When it was set (ms), filled in by the server.
    public var since: Int?

    public init(text: String, until: Int? = nil, since: Int? = nil) {
        self.text = text
        self.until = until
        self.since = since
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        text = try c.decodeIfPresent(String.self, forKey: .text) ?? ""
        until = try? c.decodeIfPresent(Int.self, forKey: .until)
        since = try? c.decodeIfPresent(Int.self, forKey: .since)
    }

    /// Clients hide it once `until` has passed (the server drops it soon after).
    public func isOn(at date: Date = Date()) -> Bool {
        guard !text.isEmpty else { return false }
        guard let until else { return true }
        return until > date.yapMillis
    }

    public static let maxLength = 80

    /// "at work til 6" → "Ada is away: at work til 6".
    public func line(name: String) -> String { "\(name) is away: \(text)" }
}

/// `away { uid, text, until }`: I @mentioned or replied to someone who's away.
public struct AwayNote: Sendable, Hashable {
    public var uid: String
    public var text: String
    public var until: Int?

    public init(uid: String, text: String, until: Int? = nil) {
        self.uid = uid
        self.text = text
        self.until = until
    }
}

// MARK: - Polls and dice

/// `Message.poll`: `votes[i]` = who picked option `i` (one vote each).
public struct Poll: Codable, Sendable, Hashable {
    public var options: [String]
    public var votes: [[String]]

    public init(options: [String], votes: [[String]]? = nil) {
        self.options = options
        self.votes = votes ?? Array(repeating: [], count: options.count)
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        options = (try? c.decodeIfPresent([String].self, forKey: .options)) ?? []
        let raw = (try? c.decodeIfPresent([[String]].self, forKey: .votes)) ?? []
        votes = options.indices.map { $0 < raw.count ? raw[$0] : [] }
    }

    /// The option `uid` picked, if any.
    public func choice(of uid: String) -> Int? { votes.firstIndex { $0.contains(uid) } }
    /// Everyone who voted.
    public var total: Int { votes.reduce(0) { $0 + $1.count } }
    public func count(_ option: Int) -> Int { option < votes.count ? votes[option].count : 0 }
    /// 0…1 of the votes for `option` (0 with no votes).
    public func share(_ option: Int) -> Double { total == 0 ? 0 : Double(count(option)) / Double(total) }

    /// My vote moved to `option` (nil = taken back), the way the server will.
    public func voting(_ uid: String, for option: Int?) -> Poll {
        var next = self
        next.votes = votes.map { $0.filter { $0 != uid } }
        if let option, option >= 0, option < next.votes.count { next.votes[option].append(uid) }
        return next
    }

    public static let minOptions = 2
    public static let maxOptions = 10
    public static let maxOptionLength = 80
    public static let maxQuestionLength = 300

    /// Why a poll can't be sent as typed, or nil when it's fine.
    public static func problem(question: String, options: [String]) -> String? {
        let q = question.trimmingCharacters(in: .whitespacesAndNewlines)
        let opts = options.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        if q.isEmpty { return "Ask a question." }
        if q.count > maxQuestionLength { return "Keep the question under \(maxQuestionLength) characters." }
        if opts.count < minOptions { return "A poll needs at least two answers." }
        if opts.count > maxOptions { return "Ten answers at most." }
        if opts.contains(where: { $0.count > maxOptionLength }) { return "Keep each answer under \(maxOptionLength) characters." }
        if Set(opts.map { $0.lowercased() }).count != opts.count { return "Each answer has to be different." }
        return nil
    }
}

/// `Message.roll`: rolled by the server, so nobody can fake it.
public struct Roll: Codable, Sendable, Hashable {
    /// "roll" or "flip".
    public var kind: String
    /// "2d6+1", "1-100", "coin"…
    public var spec: String
    /// Each die (a flip: 0 = heads, 1 = tails).
    public var rolls: [Int]
    public var total: Int

    public init(kind: String, spec: String, rolls: [Int], total: Int) {
        self.kind = kind
        self.spec = spec
        self.rolls = rolls
        self.total = total
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = (try? c.decodeIfPresent(String.self, forKey: .kind)) ?? "roll"
        spec = (try? c.decodeIfPresent(String.self, forKey: .spec)) ?? ""
        rolls = (try? c.decodeIfPresent([Int].self, forKey: .rolls)) ?? []
        total = (try? c.decodeIfPresent(Int.self, forKey: .total)) ?? rolls.reduce(0, +)
    }

    public var isFlip: Bool { kind == "flip" }
    /// "Heads" / "Tails" for a flip.
    public var coin: String { rolls.first == 1 ? "Tails" : "Heads" }
    /// The headline: the total, or Heads/Tails.
    public var headline: String { isFlip ? coin : "\(total)" }
    /// "2d6 · 3 + 5", "1–100" (empty for a flip).
    public var detail: String {
        if isFlip { return "" }
        let pretty = spec.replacingOccurrences(of: "-", with: "–")
        if rolls.count > 1 || spec.contains("+") || (spec.contains("-") && spec.contains("d")) {
            let dice = rolls.map(String.init).joined(separator: " + ")
            return "\(pretty) · \(dice)"
        }
        return pretty
    }
}

/// What the composer turns `/poll …` into (`/roll` and `/flip` go as they
/// are: the server rolls them).
public enum SlashCommand: Equatable, Sendable {
    case poll(question: String, options: [String])

    /// `/poll Question | A | B | C` → a poll; anything else → nil.
    public static func parse(_ text: String) -> SlashCommand? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.lowercased().hasPrefix("/poll ") || trimmed.lowercased() == "/poll" else { return nil }
        let rest = trimmed.dropFirst("/poll".count)
        let parts = rest.split(separator: "|", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        guard let question = parts.first else { return nil }
        return .poll(question: question, options: parts.dropFirst().filter { !$0.isEmpty })
    }

    /// The whole body is `/roll [spec]` or `/flip` (the server rolls it).
    static func isDice(_ text: String) -> Bool {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return t == "/flip" || t == "/roll" || t.hasPrefix("/roll ")
    }

    /// Composer hints for `/` at the start of an empty-ish draft.
    public static let hints: [(command: String, help: String)] = [
        ("/poll", "Question | Answer | Answer"),
        ("/roll", "1–100, or 20, or 2d6+1"),
        ("/flip", "Heads or tails"),
    ]
}

// MARK: - Doodles

/// Doodle reactions: a tiny drawing (`POST /api/doodle` → id) reacted as `<d:ID>`.
public enum Doodle {
    /// The drawing box (pixels); exported at 2×.
    public static let size = 48
    public static let scale = 2
    public static let maxFrames = 3
    public static let maxBytes = 64 * 1024

    /// `<d:ID>` → ID.
    public static func id(fromReaction emoji: String) -> String? {
        guard emoji.hasPrefix("<d:"), emoji.hasSuffix(">") else { return nil }
        let id = emoji.dropFirst(3).dropLast()
        guard !id.isEmpty, id.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "_" || $0 == "-" }) else { return nil }
        return String(id)
    }

    public static func reaction(_ id: String) -> String { "<d:\(id)>" }

    /// My doodle on a message, if I left one.
    static func mine(on message: Message, uid: String) -> Reaction? {
        message.doodles.first { $0.uids.contains(uid) }
    }
}

// MARK: - Media shelf, catch-up, hangs, quiet hours

/// `unread.counts` reply for one channel: messages by others since my read
/// mark (≤ 999), the oldest of them, and how many mention me (the one
/// mention rule, or a reply to me). Older servers don't count mentions:
/// `mentions` is nil then — unknown, not 0.
public struct UnreadCount: Decodable, Sendable, Hashable {
    public struct First: Decodable, Sendable, Hashable {
        public var id: String
        public var ts: Int
        public var uid: String
        public var body: String

        public init(id: String, ts: Int, uid: String, body: String) {
            self.id = id
            self.ts = ts
            self.uid = uid
            self.body = body
        }

        private enum CodingKeys: String, CodingKey { case id, ts, uid, body }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            id = try c.decode(String.self, forKey: .id)
            ts = (try? c.decodeIfPresent(Int.self, forKey: .ts)) ?? 0
            uid = (try? c.decodeIfPresent(String.self, forKey: .uid)) ?? ""
            body = (try? c.decodeIfPresent(String.self, forKey: .body)) ?? ""
        }
    }

    public var n: Int
    public var first: First?
    public var mentions: Int?

    public init(n: Int, first: First? = nil, mentions: Int? = nil) {
        self.n = n
        self.first = first
        self.mentions = mentions
    }

    private enum CodingKeys: String, CodingKey { case n, first, mentions }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        n = (try? c.decodeIfPresent(Int.self, forKey: .n)) ?? 0
        first = try? c.decodeIfPresent(First.self, forKey: .first)
        mentions = (try? c.decodeIfPresent(Int.self, forKey: .mentions)).flatMap { $0 }.map { max(0, $0) }
    }

    /// "12", "999+".
    public var label: String { n >= 999 ? "999+" : "\(n)" }
}

/// A plan made from When's good: "🗓 Game night is starting · Lounge".
public struct Hang: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    /// ms since epoch, on a half-hour boundary.
    public var at: Int
    public var title: String
    /// A voice channel to meet in, if any.
    public var channelId: String?
    public var uid: String
    public var createdAt: Int

    public init(id: String, at: Int, title: String, channelId: String? = nil, uid: String = "", createdAt: Int = 0) {
        self.id = id
        self.at = at
        self.title = title
        self.channelId = channelId
        self.uid = uid
        self.createdAt = createdAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        at = (try? c.decodeIfPresent(Int.self, forKey: .at)) ?? 0
        title = (try? c.decodeIfPresent(String.self, forKey: .title)) ?? "Hang"
        channelId = try? c.decodeIfPresent(String.self, forKey: .channelId)
        uid = (try? c.decodeIfPresent(String.self, forKey: .uid)) ?? ""
        createdAt = (try? c.decodeIfPresent(Int.self, forKey: .createdAt)) ?? 0
    }

    public var date: Date { Date(timeIntervalSince1970: Double(at) / 1000) }

    /// Started, and not two hours ago yet (the server keeps those in `ready.hangs`).
    public func isOn(at now: Date = Date()) -> Bool {
        let ms = now.yapMillis
        return at <= ms && ms - at < 2 * 3_600_000
    }

    public static let maxTitle = 80
    /// Between 5 minutes and 3 weeks from now.
    public static let soonest: TimeInterval = 5 * 60
    public static let latest: TimeInterval = 21 * 86_400
    public static let maxUpcoming = 5

    /// The next half-hour boundary at least 5 minutes from `now`.
    public static func firstSlot(after now: Date = Date()) -> Date {
        let half = 1_800.0
        let earliest = now.timeIntervalSince1970 + soonest
        return Date(timeIntervalSince1970: (earliest / half).rounded(.up) * half)
    }

    /// `date` snapped to its half hour (rounding to the nearest one).
    public static func snap(_ date: Date) -> Date {
        let half = 1_800.0
        return Date(timeIntervalSince1970: (date.timeIntervalSince1970 / half).rounded() * half)
    }
}

/// Quiet hours (PROTOCOL.md › Quiet hours): a nightly window in my time zone,
/// optionally letting @mentions and replies through, plus a snooze.
public struct QuietHours: Codable, Sendable, Hashable {
    /// Minutes after midnight, 0–1439 (the window may wrap past midnight).
    public var from: Int
    public var to: Int
    /// IANA zone, e.g. "Europe/Rome".
    public var tz: String
    /// Let @mentions and replies through.
    public var mentions: Bool
    /// ms: hold pushes until then, on top of the window (≤ a week ahead).
    public var until: Int?

    public init(from: Int = 23 * 60, to: Int = 8 * 60, tz: String = TimeZone.current.identifier, mentions: Bool = true, until: Int? = nil) {
        self.from = from
        self.to = to
        self.tz = tz
        self.mentions = mentions
        self.until = until
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        from = min(1439, max(0, (try? c.decodeIfPresent(Int.self, forKey: .from)) ?? 23 * 60))
        to = min(1439, max(0, (try? c.decodeIfPresent(Int.self, forKey: .to)) ?? 8 * 60))
        tz = (try? c.decodeIfPresent(String.self, forKey: .tz)) ?? TimeZone.current.identifier
        mentions = (try? c.decodeIfPresent(Bool.self, forKey: .mentions)) ?? true
        until = try? c.decodeIfPresent(Int.self, forKey: .until)
    }

    /// Inside the nightly window at `date` (in `tz`)?
    public func inWindow(at date: Date) -> Bool {
        guard from != to else { return false }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: tz) ?? .current
        let parts = calendar.dateComponents([.hour, .minute], from: date)
        let minute = (parts.hour ?? 0) * 60 + (parts.minute ?? 0)
        return from < to ? (minute >= from && minute < to) : (minute >= from || minute < to)
    }

    public func isSnoozed(at date: Date) -> Bool { (until ?? 0) > date.yapMillis }

    /// Should a notification (and its sound) be held back right now?
    /// `mention` = an @mention or a reply to me. Same rule as the server:
    /// quiet = snoozed or inside the window; mentions get through either
    /// way when `mentions` is on.
    public func holds(at date: Date = Date(), mention: Bool) -> Bool {
        guard isSnoozed(at: date) || inWindow(at: date) else { return false }
        return !(mention && mentions)
    }

    /// "until tomorrow morning": the window's end (or 8:00) after `date`.
    public func tomorrowMorning(after date: Date = Date()) -> Date {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: tz) ?? .current
        let end = from != to ? to : 8 * 60
        let start = calendar.startOfDay(for: date)
        var candidate = start.addingTimeInterval(TimeInterval(end * 60))
        while candidate <= date { candidate = calendar.date(byAdding: .day, value: 1, to: candidate) ?? candidate.addingTimeInterval(86_400) }
        return candidate
    }

    /// "11:00 PM" for minutes after midnight.
    static func label(_ minutes: Int) -> String {
        var c = DateComponents()
        c.hour = minutes / 60
        c.minute = minutes % 60
        let date = Calendar(identifier: .gregorian).date(from: c) ?? Date()
        return date.formatted(date: .omitted, time: .shortened)
    }
}

/// `notify { mode, topic, quiet }` (and `ready.notify`).
public struct NotifySettings: Decodable, Sendable, Equatable {
    /// "off" | "pings" | "all"
    public var mode: String
    public var topic: String?
    public var quiet: QuietHours?

    public init(mode: String = "pings", topic: String? = nil, quiet: QuietHours? = nil) {
        self.mode = mode
        self.topic = topic
        self.quiet = quiet
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        mode = (try? c.decodeIfPresent(String.self, forKey: .mode)) ?? "off"
        topic = try? c.decodeIfPresent(String.self, forKey: .topic)
        quiet = try? c.decodeIfPresent(QuietHours.self, forKey: .quiet)
    }

    private enum CodingKeys: String, CodingKey { case mode, topic, quiet }
}
