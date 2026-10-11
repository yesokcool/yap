import Foundation

/// A quiet line in a channel for this session only — nothing stored, never
/// a popup (PROTOCOL.md › Since you left): "Cleo joined — invited by Ada"
/// when someone is let in, "You're in. Say hi 👋" when I am.
public struct LocalNote: Identifiable, Hashable, Sendable {
    public enum Kind: Hashable, Sendable {
        /// Someone was just let in from the waiting room.
        case joined(name: String, color: String, invitedBy: String?, letInBy: String)
        /// I was just let in.
        case welcome
    }

    /// One note per key ("join:<uid>", "welcome"); a newer one replaces it.
    public var key: String
    public var channelId: String
    /// When it happened (ms): it sits among the messages by time.
    public var ts: Int
    public var kind: Kind

    public var id: String { key }

    public init(key: String, channelId: String, ts: Int, kind: Kind) {
        self.key = key
        self.channelId = channelId
        self.ts = ts
        self.kind = kind
    }

    /// What follows the name: "joined — invited by Ada", "joined — let in by you".
    public var joinedText: String? {
        guard case let .joined(_, _, invitedBy, letInBy) = kind else { return nil }
        return invitedBy.map { "joined — invited by \($0)" } ?? "joined — let in by \(letInBy)"
    }

    /// The whole line, for VoiceOver and plain text.
    public var plain: String {
        switch kind {
        case let .joined(name, _, _, _): "\(name) \(joinedText ?? "joined")"
        case .welcome: "You're in. Say hi 👋"
        }
    }
}

/// One row of a channel's message list: the same layout as the web client
/// (day dividers, consecutive messages grouped under one header) plus an
/// unread marker.
public enum MessageRow: Identifiable, Hashable, Sendable {
    /// Older history exists; reaching this row loads the next page.
    case loadMore(channelId: String, loading: Bool)
    /// The very beginning of the channel.
    case channelStart(channelId: String, name: String)
    case day(key: String, date: Date)
    /// "New" line above the first message that arrived since you last looked.
    case unread
    case message(Message, grouped: Bool)
    /// Encrypted notes this device can't open yet, one after another: one
    /// quiet row ("3 encrypted notes…") instead of the same line each.
    case lockedNotes(first: Message, count: Int)
    /// This session's own quiet line ("Cleo joined — invited by Ada"), where it happened.
    case note(LocalNote)

    public var id: String {
        switch self {
        case .loadMore(let channelId, _): "more:\(channelId)"
        case .channelStart(let channelId, _): "start:\(channelId)"
        case .day(let key, _): "day:\(key)"
        case .unread: "unread"
        case .message(let m, _): "msg:\(m.id)"
        case .lockedNotes(let first, _): "locked:\(first.id)"
        case .note(let note): "note:\(note.key)"
        }
    }
}

public enum MessageRows {
    /// Messages within this window from the same person share a header.
    public static let groupWindowMs = 7 * 60_000

    /// `locked`: notes that can't be opened here (they collapse into runs).
    /// `notes`: this session's quiet lines, placed among the messages by time
    /// (newer than the last message: at the end); a message after one starts
    /// a new group.
    public static func build(channel: Channel, messages: [Message], pending: [Message], hasMore: Bool, loading: Bool,
                             unreadAfter: Int?, myUid: String, calendar: Calendar = .current,
                             locked: (Message) -> Bool = { _ in false }, notes: [LocalNote] = []) -> [MessageRow] {
        var rows: [MessageRow] = []
        rows.append(hasMore ? .loadMore(channelId: channel.id, loading: loading) : .channelStart(channelId: channel.id, name: channel.name))
        var previous: Message?
        var markedUnread = false
        var notes = notes.filter { $0.channelId == channel.id }.sorted { ($0.ts, $0.key) < ($1.ts, $1.key) }
        for message in messages + pending {
            var afterNote = false
            while let note = notes.first, note.ts < message.ts, !message.isPending {
                rows.append(.note(note))
                notes.removeFirst()
                afterNote = true
            }
            let newDay = previous.map { !calendar.isDate($0.date, inSameDayAs: message.date) } ?? true
            if newDay {
                let comps = calendar.dateComponents([.year, .month, .day], from: message.date)
                rows.append(.day(key: "\(comps.year ?? 0)-\(comps.month ?? 0)-\(comps.day ?? 0)", date: message.date))
            }
            if locked(message) {
                // Same day, right after another locked note: the run grows.
                if !newDay, case let .lockedNotes(first, count)? = rows.last {
                    rows[rows.count - 1] = .lockedNotes(first: first, count: count + 1)
                } else {
                    rows.append(.lockedNotes(first: message, count: 1))
                }
                previous = message
                continue
            }
            var showMarker = false
            if let unreadAfter, !markedUnread, !message.isPending, message.ts > unreadAfter, message.uid != myUid {
                rows.append(.unread)
                markedUnread = true
                showMarker = true
            }
            // A reply starts its own group, so its quote sits under a header.
            let grouped = !newDay && !showMarker && !afterNote && message.reply == nil
                && previous.map { $0.uid == message.uid && message.ts - $0.ts < groupWindowMs } == true
            rows.append(.message(message, grouped: grouped))
            previous = message
        }
        rows += notes.map(MessageRow.note)
        return rows
    }
}
