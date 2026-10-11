import Foundation

/// The v0.36 extras: sounds, doodles, the media shelf, Catch up, away messages, chimes, quiet hours, hangs, Watch together.
extension YapClient {
    /// "Ada is away: …" goes under my newest message (what triggered it).
    func attachAway(_ note: AwayNote) {
        let sent = histories.values.flatMap(\.messages).filter { $0.uid == uid } + outbox.values.map { $0 }
        guard let newest = sent.max(by: { $0.ts < $1.ts }) else {
            notice = Notice(text: Away(text: note.text).line(name: name(of: note.uid)))
            return
        }
        var list = awayNotes[newest.id] ?? []
        list.removeAll { $0.uid == note.uid }
        list.append(note)
        awayNotes[newest.id] = list
    }

    /// Board sounds, oldest first (the order people added them).
    public var boardSounds: [Sound] {
        sounds.values.filter { !$0.isChime }.sorted { ($0.createdAt, $0.id) < ($1.createdAt, $1.id) }
    }

    /// Play a soundboard clip for everyone in my voice channel.
    public func playSound(_ sound: Sound) { send(.soundPlay(id: sound.id)) }
    public func deleteSound(_ sound: Sound) { send(.soundDelete(id: sound.id)) }

    /// Upload a clip (a WAV from `SoundClip`): a board sound, or my chime
    /// (which also becomes my `User.chime`).
    @discardableResult
    public func uploadSound(_ wav: Data, kind: String, name: String, emoji: String? = nil) async throws -> Sound {
        let (api, sid, token) = try uploadTarget()
        let started = session
        let sound = try await api.uploadSound(wav, kind: kind, name: name, emoji: emoji, sid: sid, uploadToken: token)
        try stillSignedIn(started)
        sounds[sound.id] = sound
        if sound.isChime, var me = users[uid] {
            me.chime = "s:\(sound.id)"
            users[uid] = me
        }
        return sound
    }

    /// React with a doodle: upload it (GIF/PNG ≤ 64 KB), then `<d:ID>`.
    public func react(to message: Message, doodle: Data, mime: String) async throws {
        let (api, sid, token) = try uploadTarget()
        let started = session
        let id = try await api.uploadDoodle(doodle, mime: mime, sid: sid, uploadToken: token)
        try stillSignedIn(started)
        react(to: message, emoji: Doodle.reaction(id), on: true)
    }

    /// Pictures and GIFs in a channel (the media shelf), newest first.
    /// Asked for before the connection is ready, it's asked again on `ready`.
    public func loadShelf(_ channelId: String) {
        shelves[channelId] = Shelf()
        guard isConnected else {
            shelfWanted.insert(channelId)
            return
        }
        if !send(.shelf(channelId: channelId, before: nil)) { shelves[channelId]?.loading = false }
    }

    public func loadMoreShelf(_ channelId: String) {
        guard var shelf = shelves[channelId], !shelf.loading, shelf.hasMore, let oldest = shelf.items.last else { return }
        shelf.loading = true
        shelves[channelId] = shelf
        if !send(.shelf(channelId: channelId, before: oldest.ts)) { shelves[channelId]?.loading = false }
    }

    /// Channels with something unread (text + my Personal space), newest activity first.
    public var unreadChannels: [Channel] {
        channelsById.values.filter(isUnread).sorted { ($0.lastMessageAt ?? 0) > ($1.lastMessageAt ?? 0) }
    }

    /// Catch up: ask how much is unread where (≤ 50 channels). Also asked on
    /// every `ready`, so mentions that came while I was away get their badge.
    public func requestUnreadCounts() {
        let since = Dictionary(unreadChannels.prefix(50).map { ($0.id, lastRead[$0.id] ?? 0) }, uniquingKeysWith: { a, _ in a })
        guard !since.isEmpty else {
            unreadCounts = [:]
            return
        }
        unreadCounts = nil
        send(.unreadCounts(since: since))
    }

    /// `unread.counts` came back: mentions that arrived while I was away
    /// become the channel's mention badge, like live ones (never lower than
    /// what's already counted; older servers send no `mentions`).
    func onUnreadCounts(_ counts: [String: UnreadCount]) {
        unreadCounts = counts
        for (channelId, count) in counts {
            guard let mentions = count.mentions, mentions > 0 else { continue }
            if channelId == activeChannelId, isAppActive { continue }
            if mentions > (mentionCounts[channelId] ?? 0) { mentionCounts[channelId] = mentions }
        }
    }

    /// How many of a channel's new messages mention me: what the counts
    /// said, or what came in live, whichever is more.
    public func mentionsForMe(in channelId: String) -> Int {
        max(unreadCounts?[channelId]?.mentions ?? 0, mentionCounts[channelId] ?? 0)
    }

    /// Catch up's list: the unread channels, those that mention me first
    /// (most first), then the newest (the web's order).
    public var catchUpChannels: [Channel] {
        func newest(_ channel: Channel) -> Int { unreadCounts?[channel.id]?.first?.ts ?? channel.lastMessageAt ?? 0 }
        return unreadChannels.sorted { a, b in
            let (ma, mb) = (mentionsForMe(in: a.id), mentionsForMe(in: b.id))
            if ma != mb { return ma > mb }
            return (newest(a), a.id) > (newest(b), b.id)
        }
    }

    /// Catch up › Mark all read: every channel counts as read up to its
    /// newest message, and no mention badges are left.
    public func markAllRead() {
        for channel in channelsById.values where channel.kind != .voice {
            guard let latest = channel.lastMessageAt else { continue }
            lastRead[channel.id] = max(lastRead[channel.id] ?? 0, latest)
        }
        credentials.lastRead = lastRead
        mentionCounts = [:]
        unreadCounts = [:]
    }

    /// The first text channel (where "Cleo joined" lines go), by position.
    public var firstTextChannel: Channel? { textChannels.first }

    /// A quiet line at the end of a channel, for this session only (one per key; the newest 20).
    func addLocalNote(_ note: LocalNote) {
        localNotes = Array((localNotes.filter { $0.key != note.key } + [note]).suffix(20))
    }

    /// The quiet lines in one channel, oldest first.
    public func localNotes(in channelId: String) -> [LocalNote] {
        localNotes.filter { $0.channelId == channelId }
    }

    // Away messages, chimes, quiet hours, hangs

    /// Set my away message (≤ 80 characters; `until` nil = until I'm back).
    public func setAway(_ text: String, until: Date?) {
        let trimmed = String(text.trimmingCharacters(in: .whitespacesAndNewlines).prefix(Away.maxLength))
        guard !trimmed.isEmpty else { return clearAway() }
        let away = Away(text: trimmed, until: until?.yapMillis, since: Date().yapMillis)
        if var me = users[uid] {
            me.away = away
            users[uid] = me
        }
        send(.away(text: trimmed, until: away.until))
    }

    public func clearAway() {
        if var me = users[uid] {
            me.away = nil
            users[uid] = me
        }
        send(.awayOff)
    }

    /// My join chime: a built-in id, `s:<soundId>`, or nil = default.
    public func setChime(_ chime: String?) {
        if var me = users[uid] {
            me.chime = chime
            users[uid] = me
        }
        send(.chime(chime))
    }

    /// The sound behind an `s:` chime, if we know it.
    func chimeSound(_ choice: ChimeChoice) -> Sound? {
        if case .clip(let id) = choice { return sounds[id] }
        return nil
    }

    /// Quiet hours (nil = off). Applied here right away; the server holds pushes.
    public func setQuietHours(_ quiet: QuietHours?) {
        var settings = notifySettings ?? NotifySettings()
        settings.quiet = quiet
        notifySettings = settings
        send(.notifyQuiet(quiet))
    }

    /// Phone pushes via ntfy: "off" | "pings" | "all".
    public func setPushMode(_ mode: String) {
        var settings = notifySettings ?? NotifySettings()
        settings.mode = mode
        notifySettings = settings
        send(.notifySettings(mode: mode))
    }

    /// Should a local notification (and its chime) wait right now?
    public func quietHolds(mention: Bool, at date: Date = Date()) -> Bool {
        notifySettings?.quiet?.holds(at: date, mention: mention) ?? false
    }

    /// Hangs still to come (or on right now), soonest first.
    public func upcomingHangs(now: Date = Date()) -> [Hang] {
        hangs.filter { $0.at > now.yapMillis || $0.isOn(at: now) }
    }

    /// Plan a hang at a half-hour boundary (5 min … 3 weeks from now).
    public func planHang(at date: Date, title: String, channelId: String?) {
        let trimmed = String(title.trimmingCharacters(in: .whitespacesAndNewlines).prefix(Hang.maxTitle))
        guard !trimmed.isEmpty else { return }
        send(.hangSet(at: Hang.snap(date).yapMillis, title: trimmed, channelId: channelId))
    }

    public func cancelHang(_ hang: Hang) { send(.hangCancel(id: hang.id)) }

    /// ▶ Watch together: queue it (plays now if nothing's on).
    public func queueWatch(videoId: String) { send(.watchQueue(add: videoId)) }
}
