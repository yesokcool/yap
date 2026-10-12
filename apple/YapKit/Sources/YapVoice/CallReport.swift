import Foundation

/// What the app noticed about one call (PROTOCOL.md › Voice › Call report):
/// a header, the people, a timeline. Kept on this device only (the last
/// three, `CallReportStore`) and copied as plain text by the person who was
/// in it. Never an IP address, a message or a key: nothing here reads an
/// address, and every piece of free text goes through `CallReport.scrub`.
public struct CallReport: Codable, Sendable, Equatable {
    public struct Line: Codable, Sendable, Equatable {
        /// Seconds since we joined.
        public var at: TimeInterval
        public var text: String
    }

    /// One person who was in the call with us (by account: a reconnect's new session is the same person).
    public struct Person: Codable, Sendable, Equatable {
        public var uid: String
        public var name: String
        /// "direct (host ↔ srflx)" / "relay (relay ↔ host)": candidate types only.
        public var connection: String?
        /// "red+opus" or "opus", each way.
        public var codecOut: String?
        public var codecIn: String?
        /// The Opus fmtp we asked them for (our description).
        public var fmtpAsked: String?
        /// 2 s readings.
        public var kbpsIn: [Double] = []
        public var kbpsOut: [Double] = []
        public var rttMs: [Double] = []
        public var jitterBufferMs: [Double] = []
        /// Counted over the whole call (every connection with them).
        public var packetsReceived = 0
        public var packetsLost = 0
        public var copiesReceived = 0
        public var secondaryUsed = 0
        public var samples: Double = 0
        public var patched: Double = 0
        public var iceRestarts = 0
        public var rebuilds = 0
        public var reconnectsKept = 0
        public var weakSeconds: Double = 0
        /// Since when "weak connection" shows (nil = not now).
        public var weakSince: TimeInterval?
        /// What they report receiving from us (`remote-inbound-rtp`): our
        /// packets sent to them, how many of those they lost, and the round
        /// trip measured with their reports. Optional: reports kept by an
        /// older version don't have them.
        public var sentToThem: Int?
        public var theyLost: Int?
        public var theirRttMs: [Double]?

        public init(uid: String, name: String) {
            self.uid = uid
            self.name = name
        }
    }

    /// At most this many timeline lines (the oldest go first; the summary never does).
    public static let maxLines = 2000
    /// At most this many 2 s readings per series (~5½ hours).
    static let maxReadings = 10_000

    public var started: Date
    /// Last time notes were taken (an unfinished report ends here).
    public var updated: Date
    public var ended: Date?
    /// "You left.", "The channel was archived.", …
    public var endReason: String?
    public var app: String
    public var device: String
    public var channel: String
    /// "Quality: 128 kbps (yours)", "Input: voice activity, sensitivity −50 dB", …
    public var settings: [String]
    /// "In: iPhone Microphone · Out: Speaker"
    public var audio: String?
    public var people: [Person] = []
    public var timeline: [Line] = []
    public var droppedLines = 0

    public init(started: Date, app: String, device: String, channel: String, settings: [String], audio: String?) {
        self.started = started
        self.updated = started
        self.app = CallReport.scrub(app)
        self.device = CallReport.scrub(device)
        self.channel = CallReport.scrub(channel)
        self.settings = settings.map(CallReport.scrub)
        self.audio = audio.map(CallReport.scrub)
    }

    // MARK: Taking notes

    public mutating func add(_ text: String, at: TimeInterval) {
        timeline.append(Line(at: max(0, at), text: Self.scrub(text)))
        if timeline.count > Self.maxLines {
            let extra = timeline.count - Self.maxLines
            timeline.removeFirst(extra)
            droppedLines += extra
        }
    }

    /// The person with this account (added on first sight).
    public mutating func person(_ uid: String, name: String) -> Int {
        if let i = people.firstIndex(where: { $0.uid == uid }) {
            if !name.isEmpty { people[i].name = Self.scrub(name) }
            return i
        }
        people.append(Person(uid: uid, name: Self.scrub(name)))
        return people.count - 1
    }

    static func append(_ value: Double, to series: inout [Double]) {
        series.append(value)
        if series.count > maxReadings { series.removeFirst(series.count - maxReadings) }
    }

    // MARK: Plain text

    /// The report as text. `now`: a call still going is cut there ("still in the call").
    public func text(now: Date? = nil, live: Bool = false, calendar: Calendar = .current, timeZone: TimeZone = .current) -> String {
        let end = ended ?? (live ? (now ?? updated) : updated)
        var out: [String] = ["Yap call report"]
        out.append("App: \(app)")
        out.append("Device: \(device)")
        out.append("Channel: \(channel)")
        out.append("Started: \(Self.startText(started, calendar: calendar, timeZone: timeZone)) (local time); lasted \(Self.duration(end.timeIntervalSince(started)))")
        if let endReason, ended != nil {
            out.append("Ended: \(endReason)")
        } else if live {
            out.append("Ended: not yet (copied during the call)")
        } else {
            out.append("Ended: unknown. Yap stopped taking notes at \(Self.stamp(updated.timeIntervalSince(started))) (it may have been closed or stopped by the system).")
        }
        out += settings
        if let audio { out.append("Audio: \(audio)") }

        out.append("")
        out.append("People")
        if people.isEmpty { out.append("Nobody else was in the call.") }
        for person in people {
            out.append(person.name)
            out += Self.summary(person, until: end.timeIntervalSince(started)).map { "  " + $0 }
        }

        out.append("")
        out.append("Timeline")
        if droppedLines > 0 { out.append("(\(droppedLines) earlier lines left out: only the last \(Self.maxLines) are kept)") }
        out += timeline.map { "\(Self.stamp($0.at)) \($0.text)" }
        // Belt and braces: whatever slipped into free text, no addresses leave.
        return Self.scrub(out.joined(separator: "\n")) + "\n"
    }

    static func summary(_ p: Person, until: TimeInterval) -> [String] {
        var lines: [String] = []
        lines.append("Connection: \(p.connection ?? "never connected")")
        if p.codecOut != nil || p.codecIn != nil {
            lines.append("Codec: \(p.codecOut ?? "?") out, \(p.codecIn ?? "?") in")
        }
        if let fmtp = p.fmtpAsked { lines.append("Opus we asked for: \(fmtp)") }
        if let inKbps = median(p.kbpsIn), let outKbps = median(p.kbpsOut) {
            lines.append("Sound: \(Int(inKbps.rounded())) kbps in, \(Int(outKbps.rounded())) out (median)")
        }
        if let rtt = median(p.rttMs), let worst = p.rttMs.max() {
            lines.append("Round trip: \(Int(rtt.rounded())) ms (median), \(Int(worst.rounded())) ms at worst")
        }
        let expected = p.packetsReceived + p.packetsLost
        if expected > 0 {
            lines.append("Lost packets: \(percent(Double(p.packetsLost), of: Double(expected))) of \(expected)")
        }
        if p.samples > 0 {
            lines.append("Audio patched over: \(percent(p.patched, of: p.samples))")
        }
        if expected > 0 {
            // libwebrtc doesn't count what RED rebuilt; it does count the copies that arrive.
            lines.append(p.copiesReceived > 0
                ? "RED rebuilt: not counted by WebRTC (copies arrived: \(p.copiesReceived))"
                : "RED rebuilt: no copies arrived")
        }
        if p.secondaryUsed > 0 { lines.append("Rebuilt from FEC/RED (WebRTC's count): \(p.secondaryUsed) packets") }
        if let jb = median(p.jitterBufferMs) { lines.append("Jitter buffer: \(Int(jb.rounded())) ms (median)") }
        if let line = theyLostLine(p) { lines.append(line) }
        lines.append("ICE restarts: \(p.iceRestarts); connection rebuilds: \(p.rebuilds); reconnects kept: \(p.reconnectsKept)")
        let weak = p.weakSeconds + (p.weakSince.map { max(0, until - $0) } ?? 0)
        lines.append("Weak connection shown: \(weak > 0 ? duration(weak) : "never")")
        return lines
    }

    /// "From you: they lost 0.4 % of yours (their round trip 38 ms median)",
    /// from their reports (the web's words).
    static func theyLostLine(_ p: Person) -> String? {
        guard let sent = p.sentToThem else { return nil }
        guard sent > 0, (p.theirRttMs?.isEmpty == false) || (p.theyLost ?? 0) > 0 else { return "From you: no reports from them" }
        let rtt = median(p.theirRttMs ?? []).map { " (their round trip \(Int($0.rounded())) ms median)" } ?? ""
        return "From you: they lost \(percent(Double(p.theyLost ?? 0), of: Double(sent))) of yours" + rtt
    }

    // MARK: Formatting helpers

    static func median(_ values: [Double]) -> Double? {
        guard !values.isEmpty else { return nil }
        let sorted = values.sorted()
        let mid = sorted.count / 2
        return sorted.count % 2 == 0 ? (sorted[mid - 1] + sorted[mid]) / 2 : sorted[mid]
    }

    static func percent(_ part: Double, of whole: Double) -> String {
        guard whole > 0 else { return "0 %" }
        let value = 100 * part / whole
        return value == 0 ? "0 %" : String(format: value < 10 ? "%.1f %%" : "%.0f %%", value)
    }

    /// "+mm:ss" since joining, "+h:mm:ss" past an hour.
    public static func stamp(_ seconds: TimeInterval) -> String { "+" + clock(seconds) }

    /// How long: "4:12", "1:02:05".
    static func duration(_ seconds: TimeInterval) -> String {
        let total = max(0, Int(seconds.rounded(.down)))
        let h = total / 3600, m = (total % 3600) / 60, s = total % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }

    /// "mm:ss", "h:mm:ss" past an hour.
    static func clock(_ seconds: TimeInterval) -> String {
        let total = max(0, Int(seconds.rounded(.down)))
        let h = total / 3600, m = (total % 3600) / 60, s = total % 60
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%02d:%02d", m, s)
    }

    static func startText(_ date: Date, calendar: Calendar, timeZone: TimeZone) -> String {
        var calendar = calendar
        calendar.timeZone = timeZone
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = timeZone
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "EEE d MMM yyyy, HH:mm:ss"
        return formatter.string(from: date)
    }

    // MARK: No addresses, ever

    private static let patterns: [(NSRegularExpression, String)] = [
        // STUN/TURN server addresses.
        (try! NSRegularExpression(pattern: #"(?i)\b(?:stuns?|turns?):[^\s,;)]+"#), "[server]"),
        // mDNS candidate names (a random UUID, but still an address).
        (try! NSRegularExpression(pattern: #"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.local\b"#), "[address]"),
        // IPv4 (and the tail of an IPv4-mapped IPv6).
        (try! NSRegularExpression(pattern: #"(?<![\d.])(?:\d{1,3}\.){3}\d{1,3}(?![\d.])"#), "[address]"),
        // IPv6 with "::" (fe80::1, 2001:db8::5, ::1, ::ffff:[address]); times never have "::".
        (try! NSRegularExpression(pattern: #"(?i)(?<![0-9a-z:])(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4})*)?::(?:[0-9a-f]{1,4}(?::[0-9a-f]{1,4})*)?(?:%[0-9a-z]+)?(?::?\[address\])?(?![0-9a-z:])"#), "[address]"),
        // Full IPv6: eight groups.
        (try! NSRegularExpression(pattern: #"(?i)(?<![0-9a-z:])(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}(?:%[0-9a-z]+)?(?![0-9a-z:])"#), "[address]"),
    ]

    /// Anything that looks like an IP address (or a STUN/TURN server) becomes "[address]".
    public static func scrub(_ text: String) -> String {
        var text = text
        for (regex, replacement) in patterns {
            text = regex.stringByReplacingMatches(in: text, range: NSRange(text.startIndex..., in: text), withTemplate: replacement)
        }
        return text
    }
}
