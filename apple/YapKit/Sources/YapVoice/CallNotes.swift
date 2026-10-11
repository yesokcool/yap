import Foundation
import Observation
import os

/// Takes the notes behind a call report (PROTOCOL.md › Voice › Call report)
/// while we're in a call: the engine's stats every 2 s and what happens
/// (joins and leaves, connection states, ICE restarts, our socket, the audio
/// route, interruptions, the network, settings). `VoiceController` feeds it;
/// the report goes to `store` as it grows, so a call cut short (iOS
/// stopping Yap) still leaves its notes behind.
@MainActor
@Observable
public final class CallNotes {
    /// The call we're in, or the last one (until the next starts).
    @ObservationIgnored public private(set) var report: CallReport?
    /// There's a report to copy: this call's, or one kept from before
    /// (Settings › Voice says "No call yet." otherwise).
    public private(set) var hasReport: Bool
    /// Wall-clock time (tests: a hand-cranked clock).
    @ObservationIgnored var now: () -> Date = Date.init
    @ObservationIgnored let store: CallReportStore?

    /// The last 2 s reading per session (counters are per connection; a new one starts from zero).
    @ObservationIgnored private var last: [String: (at: Date, stats: PeerStats)] = [:]
    /// Per person, since their last line in the timeline.
    @ObservationIgnored private var interval: [String: Interval] = [:]
    @ObservationIgnored private var nextSummary: TimeInterval = CallNotes.firstSummaries
    @ObservationIgnored private var lastSaved = Date.distantPast
    @ObservationIgnored private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "voice")

    /// A line per person every 10 s for the first minute, then every minute.
    static let firstSummaries: TimeInterval = 10
    static let laterSummaries: TimeInterval = 60

    struct Interval {
        var received = 0
        var lost = 0
        var samples: Double = 0
        var patched: Double = 0
        var bytesIn: Double = 0
        var bytesOut: Double = 0
        var seconds: Double = 0
        var rtt: [Double] = []
    }

    public init(store: CallReportStore?) {
        self.store = store
        // A report from before this launch on disk (listed once, not read).
        hasReport = !(store?.files().isEmpty ?? true)
    }

    /// In a call, taking notes.
    public var isTaking: Bool { report != nil && report?.ended == nil }

    var elapsed: TimeInterval { report.map { now().timeIntervalSince($0.started) } ?? 0 }

    // MARK: The call

    /// A new call (the last report is saved already).
    public func begin(_ fresh: CallReport) {
        if isTaking { end("Moved to \(fresh.channel).") }
        report = fresh
        if !hasReport { hasReport = true }
        last = [:]
        interval = [:]
        nextSummary = Self.firstSummaries
        save(force: true)
    }

    /// A line in the timeline, now.
    public func note(_ text: String) {
        guard isTaking else { return }
        let at = elapsed
        report?.add(text, at: at)
        report?.updated = now()
    }

    /// The call is over (`reason`, in plain words: "You left.").
    public func end(_ reason: String) {
        guard isTaking, var done = report else { return }
        let at = elapsed
        for i in done.people.indices {
            if let since = done.people[i].weakSince {
                done.people[i].weakSeconds += max(0, at - since)
                done.people[i].weakSince = nil
            }
        }
        done.add(reason, at: at)
        done.ended = now()
        done.updated = now()
        done.endReason = CallReport.scrub(reason)
        report = done
        save(force: true)
    }

    /// The report as text: the call we're in (`live`), else the last one kept.
    public func text(live: Bool) -> String? {
        if live, isTaking { return report?.text(now: now(), live: true) }
        if let report, report.ended != nil { return report.text() }
        return (store?.latest() ?? report)?.text()
    }

    // MARK: People

    @discardableResult
    func person(_ uid: String, _ name: String) -> Int? {
        guard isTaking else { return nil }
        return report?.person(uid, name: name)
    }

    public func joined(uid: String, name: String) {
        person(uid, name)
        note("\(name) joined")
    }

    public func left(uid: String, name: String, lost: Bool) {
        person(uid, name)
        note(lost ? "\(name)'s connection to the server dropped" : "\(name) left")
    }

    /// They came back under a new session and we kept the connection.
    public func reconnectKept(uid: String, name: String) {
        guard let i = person(uid, name) else { return }
        report?.people[i].reconnectsKept += 1
        note("\(name) reconnected; the call kept going")
    }

    public func connectionState(uid: String, name: String, _ state: VoiceEngine.PeerState?) {
        guard isTaking else { return }
        person(uid, name)
        note("\(name): \(state?.rawValue ?? "closed")")
    }

    public func iceRestart(uid: String, name: String, count: Int) {
        guard let i = person(uid, name) else { return }
        report?.people[i].iceRestarts += 1
        note("\(name): ICE restart \(count)")
    }

    /// A connection thrown away for a new one (`words`: how it happened).
    public func rebuilt(uid: String, name: String, words: String) {
        guard let i = person(uid, name) else { return }
        report?.people[i].rebuilds += 1
        note("\(name): \(words)")
    }

    public func weak(uid: String, name: String, _ on: Bool) {
        guard let i = person(uid, name), var p = report?.people[i] else { return }
        let at = elapsed
        if on, p.weakSince == nil {
            p.weakSince = at
        } else if !on, let since = p.weakSince {
            p.weakSeconds += max(0, at - since)
            p.weakSince = nil
        } else {
            return
        }
        report?.people[i] = p
        note("\(name): weak connection \(on ? "shown" : "gone")")
    }

    // MARK: Stats (every 2 s)

    public struct Reading {
        public var uid: String
        public var name: String
        public var connected: Bool
        public var stats: PeerStats

        public init(uid: String, name: String, connected: Bool, stats: PeerStats) {
            self.uid = uid
            self.name = name
            self.connected = connected
            self.stats = stats
        }
    }

    public func stats(_ readings: [Reading]) {
        guard isTaking, var r = report else { return }
        let at = now()
        for reading in readings {
            let s = reading.stats
            let i = r.person(reading.uid, name: reading.name)
            var p = r.people[i]
            // Deltas since the last reading of this connection (from zero for a new one).
            let before = last[s.sid].flatMap { $0.stats.voicePacketsReceived <= s.voicePacketsReceived && $0.stats.bytesSent <= s.bytesSent ? $0 : nil }
            let b = before?.stats ?? PeerStats(sid: s.sid)
            let received = s.voicePacketsReceived - b.voicePacketsReceived
            let lost = max(0, s.voicePacketsLost - b.voicePacketsLost)
            let copies = max(0, s.voicePacketsDiscarded - b.voicePacketsDiscarded)
            let samples = max(0, s.samplesReceived - b.samplesReceived)
            let patched = max(0, (s.concealedSamples - s.silentConcealedSamples) - (b.concealedSamples - b.silentConcealedSamples))
            p.packetsReceived += received
            p.packetsLost += lost
            p.copiesReceived += copies
            p.secondaryUsed += max(0, (s.fecPacketsReceived - b.fecPacketsReceived) - (s.fecPacketsDiscarded - b.fecPacketsDiscarded))
            p.samples += samples
            p.patched += patched
            last[s.sid] = (at, s)

            var span = interval[reading.uid] ?? Interval()
            span.received += received
            span.lost += lost
            span.samples += samples
            span.patched += patched
            if reading.connected, let before {
                let seconds = at.timeIntervalSince(before.at)
                if seconds > 0.5, seconds < 10 {
                    let bytesIn = s.voiceBytesReceived - b.voiceBytesReceived
                    let bytesOut = s.bytesSent - b.bytesSent
                    CallReport.append(bytesIn * 8 / 1000 / seconds, to: &p.kbpsIn)
                    CallReport.append(bytesOut * 8 / 1000 / seconds, to: &p.kbpsOut)
                    span.bytesIn += bytesIn
                    span.bytesOut += bytesOut
                    span.seconds += seconds
                    let emitted = s.jitterBufferEmitted - b.jitterBufferEmitted
                    if emitted > 0 { CallReport.append(1000 * (s.jitterBufferDelay - b.jitterBufferDelay) / emitted, to: &p.jitterBufferMs) }
                }
                if let rtt = s.rttMs {
                    CallReport.append(rtt, to: &p.rttMs)
                    span.rtt.append(rtt)
                }
            }
            interval[reading.uid] = span

            if reading.connected {
                let connection = Self.connection(local: s.localCandidateType, remote: s.remoteCandidateType)
                let codecOut = s.sendsRed ? "red+opus" : "opus"
                // libwebrtc names the codec inside either way; RED shows as a copy of
                // (nearly) every packet arriving and going unused.
                let codecIn = received >= 20 ? (Double(copies) >= 0.5 * Double(received) ? "red+opus" : "opus") : p.codecIn
                if let connection, connection != p.connection {
                    r.add("\(reading.name): \(p.connection == nil ? "" : "now ")\(connection)", at: at.timeIntervalSince(r.started))
                    p.connection = connection
                }
                if codecOut != p.codecOut || codecIn != p.codecIn, let codecIn {
                    r.add("\(reading.name): sound \(codecOut) out, \(codecIn) in", at: at.timeIntervalSince(r.started))
                    p.codecOut = codecOut
                    p.codecIn = codecIn
                }
                p.fmtpAsked = s.opusFmtpAsked ?? p.fmtpAsked
            }
            r.people[i] = p
        }
        r.updated = at
        report = r
        summarize()
        save(force: false)
    }

    /// "direct (host ↔ srflx)" / "relay (relay ↔ host)": candidate types only.
    static func connection(local: String?, remote: String?) -> String? {
        guard let local, let remote else { return nil }
        let relay = local == "relay" || remote == "relay"
        return "\(relay ? "relay" : "direct") (\(local) ↔ \(remote))"
    }

    /// The per-person lines: every 10 s for the first minute, then every minute.
    private func summarize() {
        let at = elapsed
        guard at >= nextSummary, var r = report else { return }
        nextSummary += at < 60 ? Self.firstSummaries : Self.laterSummaries
        if nextSummary <= at { nextSummary = at + (at < 60 ? Self.firstSummaries : Self.laterSummaries) }
        for person in r.people {
            guard let span = interval[person.uid], span.seconds > 0 || span.received > 0 else { continue }
            var parts: [String] = []
            if let rtt = CallReport.median(span.rtt) { parts.append("rtt \(Int(rtt.rounded())) ms") }
            parts.append("lost \(CallReport.percent(Double(span.lost), of: Double(span.received + span.lost)))")
            parts.append("patched \(CallReport.percent(span.patched, of: span.samples))")
            if span.seconds > 0 {
                parts.append("\(Int((span.bytesIn * 8 / 1000 / span.seconds).rounded())) kbps in, \(Int((span.bytesOut * 8 / 1000 / span.seconds).rounded())) out")
            }
            r.add("\(person.name): " + parts.joined(separator: ", "), at: at)
        }
        interval = [:]
        report = r
        save(force: true)
    }

    private func save(force: Bool) {
        guard let report, let store else { return }
        // At least every 10 s while it grows (a stopped app keeps what it had).
        guard force || now().timeIntervalSince(lastSaved) >= 10 else { return }
        lastSaved = now()
        do {
            try store.save(report)
        } catch {
            log.error("couldn't keep the call report: \(error.localizedDescription, privacy: .public)")
        }
    }
}

/// The last three call reports, in the app's own data folder (Application
/// Support), as JSON. Nothing else reads or sends them.
public struct CallReportStore: Sendable {
    public let directory: URL
    public static let keep = 3

    public init(directory: URL) {
        self.directory = directory
    }

    /// `Application Support/<bundle id>/Call reports`.
    public static var standard: CallReportStore? {
        guard let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else { return nil }
        let id = Bundle.main.bundleIdentifier ?? "yap"
        return CallReportStore(directory: base.appending(path: id).appending(path: "Call reports"))
    }

    private func file(for report: CallReport) -> URL {
        directory.appending(path: "call-\(Int(report.started.timeIntervalSince1970 * 1000)).json")
    }

    public func save(_ report: CallReport) throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let data = try JSONEncoder().encode(report)
        // (The default protection, until first unlock: a call in a locked phone still writes.)
        try data.write(to: file(for: report), options: .atomic)
        for old in files().dropFirst(Self.keep) { try? FileManager.default.removeItem(at: old) }
    }

    /// Newest first.
    func files() -> [URL] {
        let names = (try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
        return names.filter { $0.lastPathComponent.hasPrefix("call-") && $0.pathExtension == "json" }
            .sorted { $0.lastPathComponent.localizedStandardCompare($1.lastPathComponent) == .orderedDescending }
    }

    public func all() -> [CallReport] {
        files().compactMap { try? JSONDecoder().decode(CallReport.self, from: Data(contentsOf: $0)) }
    }

    public func latest() -> CallReport? {
        files().lazy.compactMap { try? JSONDecoder().decode(CallReport.self, from: Data(contentsOf: $0)) }.first
    }
}
