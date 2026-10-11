import Foundation
import SwiftUI
import YapCore
import YapVoice

// The v0.36 extras' non-visual glue (the views are in Views/Extras;
// PROTOCOL.md › Extras is the spec).

/// "Search messages for '…'" from the quick switcher: the open channel's
/// search (the Find window) picks it up.
struct SearchRequest: Equatable {
    var text: String
    let token = UUID()
}

// MARK: - Since you left

enum SinceYouLeft {
    /// "New since 9:41 PM" / "New since yesterday, 9:41 PM" / "New since Mon, Oct 5".
    static func label(_ ts: Int?) -> String {
        guard let ts, ts > 0 else { return "New messages" }
        let date = Date(timeIntervalSince1970: Double(ts) / 1000)
        let time = date.formatted(date: .omitted, time: .shortened)
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return "New since \(time)" }
        if calendar.isDateInYesterday(date) { return "New since yesterday, \(time)" }
        if let days = calendar.dateComponents([.day], from: date, to: .now).day, days < 7 {
            return "New since \(date.formatted(.dateTime.weekday(.wide))), \(time)"
        }
        return "New since \(date.formatted(.dateTime.month(.abbreviated).day()))"
    }

    /// The first unread message in what's loaded (after `unreadAfter`, not mine).
    @MainActor
    static func firstUnread(in channelId: String, after unreadAfter: Int?, client: YapClient) -> Message? {
        guard let unreadAfter else { return nil }
        return client.histories[channelId]?.messages.first { $0.ts > unreadAfter && $0.uid != client.uid }
    }

    /// Unread messages that are older than the loaded page: the line isn't
    /// there yet, but the pill can page back to it.
    @MainActor
    static func unreadBeyondPage(in channelId: String, after unreadAfter: Int?, client: YapClient) -> Bool {
        guard let unreadAfter, let history = client.histories[channelId], history.hasMore, !history.loading,
              let oldest = history.messages.first else { return false }
        return oldest.ts > unreadAfter && history.messages.contains { $0.uid != client.uid }
    }
}

/// The "Jump to first unread" pill's state, one per open channel. The pill shows while the "New since" line is off
/// screen, until you've seen it once or tapped it.
@MainActor
@Observable
final class UnreadPill {
    var lineVisible = false
    var seen = false
    var jumping = false

    func show(channelId: String, unreadAfter: Int?, client: YapClient) -> Bool {
        guard !seen, !lineVisible, unreadAfter != nil else { return false }
        return SinceYouLeft.firstUnread(in: channelId, after: unreadAfter, client: client) != nil
            || SinceYouLeft.unreadBeyondPage(in: channelId, after: unreadAfter, client: client)
    }

    func lineAppeared(_ visible: Bool) {
        lineVisible = visible
        if visible { seen = true }
    }

    /// Pages back if needed, then scrolls to the line.
    func jump(channelId: String, unreadAfter: Int?, client: YapClient, scroll: @escaping () -> Void) {
        guard let unreadAfter, !jumping else { return }
        jumping = true
        Task {
            _ = await client.ensureLoaded(after: unreadAfter, in: channelId)
            try? await Task.sleep(for: .milliseconds(120))
            scroll()
            seen = true
            jumping = false
        }
    }
}

extension AppModel {
    /// Catch up: open a channel at its first unread message.
    func catchUp(_ channel: Channel) {
        showingCatchUp = false
        if let first = client.unreadCounts?[channel.id]?.first {
            jump = JumpTarget(channelId: channel.id, messageId: first.id)
        }
        open(.channel(channel.id))
    }

    /// Everyone's chimes and the board's clips, fetched ahead so a join or a
    /// play sounds right away.
    func warmSounds() {
        guard let endpoint = client.endpoint else { return }
        for sound in client.sounds.values { sounds.warm(sound, endpoint: endpoint) }
    }
}

// MARK: - Quick switcher (⌘K)

/// One row of the quick switcher.
struct SwitcherItem: Identifiable, Equatable {
    enum Kind: Int { case channel, voice, place, person, action, search }
    enum Action: Equatable {
        case channel(String)
        case voice(String)
        case web(WebPage)
        case webPlace(String)
        case link(URL)
        case people
        case settings
        case search(String)
        case catchUp
        case planHang
    }

    var id: String
    var kind: Kind
    var title: String
    var subtitle: String?
    var systemImage: String
    var action: Action
}

enum Switcher {
    /// Everything to jump to, filtered by `query`; the last row is always
    /// "Search messages for …" when something's typed.
    @MainActor
    static func items(_ model: AppModel, query: String) -> [SwitcherItem] {
        let client = model.client
        var all: [SwitcherItem] = []
        if let personal = client.personalChannel {
            all.append(SwitcherItem(id: "c:\(personal.id)", kind: .channel, title: "Personal space", subtitle: "only you",
                                    systemImage: "lock.fill", action: .channel(personal.id)))
        }
        for channel in client.textChannels {
            all.append(SwitcherItem(id: "c:\(channel.id)", kind: .channel, title: channel.name,
                                    subtitle: client.isUnread(channel) ? "unread" : nil, systemImage: "number", action: .channel(channel.id)))
        }
        for channel in client.voiceChannels {
            let here = client.voiceSessions(in: channel.id).count
            all.append(SwitcherItem(id: "v:\(channel.id)", kind: .voice, title: channel.name,
                                    subtitle: model.voice.wantedChannelId == channel.id ? "you're here" : here > 0 ? "\(here) in voice" : "voice · join",
                                    systemImage: "speaker.wave.2.fill", action: .voice(channel.id)))
        }
        all.append(SwitcherItem(id: "p:draw", kind: .place, title: "Draw", subtitle: "place", systemImage: WebPage.draw.systemImage, action: .web(.draw)))
        all.append(SwitcherItem(id: "p:space", kind: .place, title: "Space", subtitle: "place", systemImage: WebPage.space.systemImage, action: .web(.space)))
        all.append(SwitcherItem(id: "p:watch", kind: .place, title: "Watch together", subtitle: "opens in the browser", systemImage: "play.rectangle", action: .webPlace("watch")))
        all.append(SwitcherItem(id: "p:when", kind: .place, title: "When's good", subtitle: "opens in the browser", systemImage: "calendar", action: .webPlace("when")))
        for place in model.places {
            if place.view == "board" {
                all.append(SwitcherItem(id: "p:board", kind: .place, title: place.name, subtitle: "place", systemImage: WebPage.board.systemImage, action: .web(.board)))
            } else if let url = URL(string: place.url) {
                all.append(SwitcherItem(id: "l:\(place.id)", kind: .place, title: place.name, subtitle: url.host(), systemImage: "link", action: .link(url)))
            }
        }
        for user in client.members {
            all.append(SwitcherItem(id: "u:\(user.id)", kind: .person, title: user.name,
                                    subtitle: user.awayNow().map { "away: \($0.text)" } ?? (client.voiceChannel(of: user.id).map { "in \($0.name)" } ?? "person"),
                                    systemImage: "person.fill", action: .people))
        }
        all.append(SwitcherItem(id: "a:settings", kind: .action, title: "Settings", subtitle: nil, systemImage: "gearshape", action: .settings))
        if !client.unreadChannels.isEmpty {
            all.append(SwitcherItem(id: "a:catchup", kind: .action, title: "Catch up", subtitle: "what's unread", systemImage: "tray.full", action: .catchUp))
        }
        all.append(SwitcherItem(id: "a:hang", kind: .action, title: "Plan a hang", subtitle: nil, systemImage: "calendar.badge.plus", action: .planHang))
        all.append(SwitcherItem(id: "a:people", kind: .action, title: "People", subtitle: nil, systemImage: "person.2", action: .people))

        let q = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !q.isEmpty else {
            // Nothing typed: channels and places, unread first.
            return all.filter { $0.kind == .channel || $0.kind == .voice || $0.kind == .place }
                .sorted { ($0.subtitle == "unread" ? 0 : 1, $0.kind.rawValue) < ($1.subtitle == "unread" ? 0 : 1, $1.kind.rawValue) }
        }
        let ranked = all.compactMap { item -> (SwitcherItem, Int)? in
            guard let score = score(q, item.title.lowercased()) else { return nil }
            return (item, score)
        }
        .sorted { ($0.1, $0.0.kind.rawValue, $0.0.title.count) < ($1.1, $1.0.kind.rawValue, $1.0.title.count) }
        .map(\.0)
        return Array(ranked.prefix(12)) + [SwitcherItem(id: "search", kind: .search, title: "Search messages for “\(query.trimmingCharacters(in: .whitespaces))”",
                                                        subtitle: nil, systemImage: "magnifyingglass", action: .search(query))]
    }

    /// Lower is better: exact, prefix, word prefix, contains, letters in order.
    static func score(_ q: String, _ title: String) -> Int? {
        if title == q { return 0 }
        if title.hasPrefix(q) { return 1 }
        if title.split(whereSeparator: { !$0.isLetter && !$0.isNumber }).contains(where: { $0.hasPrefix(q) }) { return 2 }
        if title.contains(q) { return 3 }
        var rest = Substring(title)
        for ch in q {
            guard let i = rest.firstIndex(of: ch) else { return nil }
            rest = rest[rest.index(after: i)...]
        }
        return 4
    }
}

extension AppModel {
    /// Open the open channel's search, everywhere, with `text` (the
    /// switcher's last row; ⇧⌘F, the web's shortcut, with nothing typed).
    func searchMessages(_ text: String = "") {
        // Signed in only, like the web's shortcut.
        guard client.mySid != nil else { return }
        // Search lives in a channel's screen: open one if none is.
        if currentChannelId == nil, let first = client.textChannels.first { open(.channel(first.id)) }
        searchRequest = SearchRequest(text: text)
    }

    /// Do what a switcher row says. `openSettings` is the Mac's Settings window.
    func perform(_ action: SwitcherItem.Action, openURL: OpenURLAction, openSettings: () -> Void) {
        showingSwitcher = false
        switch action {
        case .channel(let id): open(.channel(id))
        case .voice(let id):
            Task { await voice.join(id) }
        case .web(let page): open(.web(page))
        case .webPlace(let place): client.openWeb(place, with: openURL)
        case .link(let url): openURL(url)
        case .people: showingPeople = true
        case .settings:
            #if os(macOS)
            openSettings()
            #else
            open(.settings)
            #endif
        case .search(let text):
            searchMessages(text)
        case .catchUp:
            showingCatchUp = true
        case .planHang: planningHang = true
        }
    }

    /// The text channel on screen, if any (either platform).
    var currentChannelId: String? {
        #if os(macOS)
        if case .channel(let id)? = selection { return id }
        #else
        if case .channel(let id)? = path.last { return id }
        #endif
        return nil
    }
}

// MARK: - Presets

/// When an away message ends: the web's and the desktop app's four choices,
/// in their words. A stable id, not a position in the list.
enum AwayEnd: String, CaseIterable, Identifiable, Hashable {
    case back, hour, threeHours, morning

    var id: Self { self }

    var title: String {
        switch self {
        case .back: "Until my next message"
        case .hour: "For 1 hour"
        case .threeHours: "For 3 hours"
        case .morning: "Until tomorrow morning"
        }
    }

    /// The end time if you set it now (nil = until your next message;
    /// tomorrow morning is 9:00, like the web).
    func until(now: Date = Date()) -> Date? {
        switch self {
        case .back: nil
        case .hour: now.addingTimeInterval(3600)
        case .threeHours: now.addingTimeInterval(3 * 3600)
        case .morning: Calendar.current.date(bySettingHour: 9, minute: 0, second: 0, of: now.addingTimeInterval(86_400))
        }
    }
}

enum AwayPresets {
    /// The end-time choices: until my next message, 1 hour, 3 hours, tomorrow morning.
    static func choices(now: Date = Date()) -> [AwayEnd] { AwayEnd.allCases }

    static let suggestions = ["at work", "asleep 💤", "eating", "out with friends", "gaming, ping me", "brb"]

    /// "until 6:00 PM" / "until Tue 8:00 AM" / "".
    static func untilLabel(_ away: Away) -> String {
        guard let until = away.until else { return "" }
        let date = Date(timeIntervalSince1970: Double(until) / 1000)
        if Calendar.current.isDateInToday(date) { return "until \(date.formatted(date: .omitted, time: .shortened))" }
        return "until \(date.formatted(.dateTime.weekday(.abbreviated).hour().minute()))"
    }
}

enum HangFormat {
    /// "Tonight 8:30 PM", "Tomorrow 8:00 PM", "Sat, Oct 10 8:00 PM".
    static func when(_ hang: Hang, now: Date = .now) -> String {
        let date = hang.date
        let time = date.formatted(date: .omitted, time: .shortened)
        if hang.isOn(at: now) { return "on now" }
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return calendar.component(.hour, from: date) >= 17 ? "Tonight \(time)" : "Today \(time)" }
        if calendar.isDateInTomorrow(date) { return "Tomorrow \(time)" }
        return date.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day().hour().minute())
    }
}

extension View {
    /// `||Spoilers||`: while some are hidden, a tap anywhere on the text (or
    /// VoiceOver's "Reveal spoiler") shows them. Otherwise taps go to links
    /// as usual (the gesture is masked off, so the view keeps its identity).
    func spoilerReveal(_ hidden: Bool, reveal: @escaping () -> Void) -> some View {
        self
            .contentShape(Rectangle())
            .simultaneousGesture(TapGesture().onEnded { if hidden { reveal() } }, including: hidden ? .all : .subviews)
            .accessibilityActions {
                if hidden { Button("Reveal spoiler", action: reveal) }
            }
    }
}

extension AppModel {
    /// ▶ Watch together: queue the video (it plays now if nothing's on) and
    /// open Watch together, signed in.
    func watchTogether(_ videoId: String, openURL: OpenURLAction) {
        client.queueWatch(videoId: videoId)
        client.openWeb("watch", with: openURL)
    }

    /// A reply quote's original, wherever it lives: in this channel (`local`
    /// scrolls to it) or elsewhere (a Personal-space bookmark).
    func openQuote(_ reply: Reply, in channelId: String, local: ((String) -> Void)?) {
        if let other = reply.channelId, other != channelId {
            jump = JumpTarget(channelId: other, messageId: reply.id)
            open(.channel(other))
        } else {
            local?(reply.id)
        }
    }

    /// "Save to Personal" belongs in a message's menu (not in the space itself).
    func canSaveToPersonal(_ message: Message) -> Bool {
        guard let personal = client.personalChannel, !message.isPending else { return false }
        return message.channelId != personal.id
    }

    /// "in #general" for a bookmark's quote; nil when it's this channel.
    func quoteSource(_ reply: Reply, in channelId: String) -> String? {
        guard let other = reply.channelId, other != channelId else { return nil }
        return client.channelsById[other].map { $0.isPersonal ? "Personal space" : "#\($0.name)" } ?? "an archived channel"
    }
}

/// Poll wording that depends on the device.
enum PollText {
    /// "tap" on a phone, "click" on the Mac.
    static var takeBack: String {
        #if os(iOS)
        "tap"
        #else
        "click"
        #endif
    }
}

enum CatchUp {
    /// The sidebar's Catch up row shows once two or more channels have
    /// something new (one is just that channel's dot), like the web.
    static let showsFrom = 2
}
