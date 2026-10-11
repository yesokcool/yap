import Foundation

/// People, channels, invites, Places and the mod log; profile pictures.
extension YapClient {
    /// Upload a square picture (≤ 512 KB). The `user` broadcast updates everyone.
    public func uploadAvatar(_ image: Data, mime: String) async throws {
        let (api, sid, token) = try uploadTarget()
        try await api.uploadAvatar(image, mime: mime, sid: sid, uploadToken: token)
    }

    /// Add a custom emoji (≤ 512 KB; clients scale static images to 128 px first).
    func uploadEmoji(_ image: Data, name: String, mime: String) async throws {
        let (api, sid, token) = try uploadTarget()
        try await api.uploadEmoji(image, name: name, mime: mime, sid: sid, uploadToken: token)
    }

    public func removeAvatar() { send(.removeAvatar) }

    public var textChannels: [Channel] {
        channelsById.values.filter { $0.kind == .text }.sorted { ($0.position, $0.name) < ($1.position, $1.name) }
    }

    public var voiceChannels: [Channel] {
        channelsById.values.filter { $0.kind == .voice }.sorted { ($0.position, $0.name) < ($1.position, $1.name) }
    }

    public func createChannel(name: String, kind: ChannelKind) {
        pendingCreate = (name, kind)
        createdChannel = nil
        send(.channelCreate(name: name, kind: kind))
    }

    public func archiveChannel(_ id: String) { send(.channelArchive(channelId: id)) }

    /// Shared voice quality for a channel (bits/s), or nil = each person's own.
    public func setBitrate(_ bitrate: Int?, for channelId: String) {
        send(.channelBitrate(channelId: channelId, bitrate: bitrate))
    }

    public func createInvite(label: String) {
        createdInvite = nil
        send(.inviteCreate(label: label.trimmingCharacters(in: .whitespacesAndNewlines)))
    }

    public func refreshInvites() { send(.inviteList) }
    public func revokeInvite(_ id: String) { send(.inviteRevoke(id: id)) }

    /// Owner only: a new main invite link. Old main links stop working for
    /// anyone not signed in yet; everyone signed in keeps working and their
    /// apps get the new key (`key`). The new link comes back as `newMainLink`.
    public func resetMainLink() {
        newMainLink = nil
        mainResetAsked = send(.inviteResetMain)
        if !mainResetAsked { notice = Notice(text: "Not connected right now. Try again in a moment.") }
    }

    /// A new main key from the server (`key`, or `ready.key`): it becomes
    /// this device's link, for HTTP calls now and the socket's next connect.
    func adoptMainKey(_ key: String) {
        guard !key.isEmpty, var endpoint, endpoint.key != key else { return }
        endpoint.key = key
        self.endpoint = endpoint
        credentials.endpoint = endpoint
        connection?.retarget(url: endpoint.webSocketURL, protocols: endpoint.webSocketProtocols)
    }

    /// `key { key }`: the main link was reset (by me, or by the owner on
    /// another device). Saved silently; whoever asked here sees the new link.
    func mainKeyReset(_ key: String) {
        adoptMainKey(key)
        if mainResetAsked, let endpoint {
            mainResetAsked = false
            newMainLink = endpoint.inviteLink
        }
    }

    /// Adds a sidebar link (https only, like the server). False if the URL won't do.
    @discardableResult
    public func addLink(name: String, url: String) -> Bool {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        var url = url.trimmingCharacters(in: .whitespacesAndNewlines)
        if !url.contains("://") { url = "https://" + url }
        guard !name.isEmpty, let parsed = URL(string: url), parsed.scheme?.lowercased() == "https", parsed.host() != nil else { return false }
        return send(.linkAdd(name: name, url: url))
    }

    public func removeLink(_ id: String) { send(.linkRemove(id: id)) }
    public func refreshModLog() { send(.modlog) }

    public var me: User? { users[uid] }
    public var myRole: Role { me?.role ?? .member }
    public var isMod: Bool { myRole.canModerate }
    /// Only the owner resets the main link.
    public var isOwner: Bool { myRole == .owner }

    public func user(_ uid: String) -> User? { users[uid] }

    /// A uid as a name for lists like the mod log ("The owner (admin key)",
    /// "Someone"), the web's `nameOf`.
    public func name(of uid: String?) -> String {
        guard let uid else { return "Someone" }
        if uid == "admin" { return "The owner (admin key)" }
        return users[uid]?.name ?? "Someone"
    }

    /// Watch together's audience (sessions with `watching`), A–Z.
    public var watchAudience: [Session] {
        sessions.values.filter { $0.watching && !$0.isListener }.sorted { a, b in
            (users[a.uid]?.name ?? "", a.sid) < (users[b.uid]?.name ?? "", b.sid)
        }
    }

    /// "▶ Title" for the sidebar: only while someone is watching and a video is on.
    public var watchTitle: String? {
        guard !watchAudience.isEmpty, let watch, watch.videoId != nil else { return nil }
        return watch.title ?? "a video"
    }

    /// What's playing in Watch together: there's nothing to ask for until
    /// someone is in the audience.
    func askForWatchIfNeeded() {
        guard !watchRequested, mySid != nil, sessions.values.contains(where: \.watching) else { return }
        watchRequested = send(.watchGet)
    }

    /// Keep listing a session whose socket dropped while voice waits for it
    /// to resume (`voice` is where it was).
    public func holdSession(_ session: Session) {
        sessions[session.sid] = session
        heldSessions.insert(session.sid)
    }

    /// Forget a session voice was holding, or one that another session just
    /// took over (a resume): it's gone, whatever the server says next.
    public func releaseSession(_ sid: String) {
        heldSessions.remove(sid)
        sessions.removeValue(forKey: sid)
    }

    /// Every session in a voice channel, OBS links included: the call
    /// connects to all of them (an OBS link has to hear it). What people see
    /// is `voiceRoster(in:)`.
    public func voiceSessions(in channelId: String) -> [Session] {
        sessions.values.filter { $0.voice?.channelId == channelId }.sorted { a, b in
            (users[a.uid]?.name ?? "", a.sid) < (users[b.uid]?.name ?? "", b.sid)
        }
    }

    /// Who shows in a voice channel (its rows, the call sheet, every count):
    /// everyone but OBS links, which listen without showing up (the web's
    /// `s.client !== 'obs'`).
    public func voiceRoster(in channelId: String) -> [Session] {
        voiceSessions(in: channelId).filter { !$0.isListener }
    }

    /// Everyone here, A–Z. There's no online/offline (PROTOCOL.md ›
    /// Presence): the server only shows sessions in voice or on a shared view.
    public var members: [User] {
        users.values.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    /// OBS links still listening in a voice channel their account isn't in
    /// (the server keeps one there up to 5 minutes after its account left:
    /// PROTOCOL.md › Voice › OBS links in a call), one per account, A–Z.
    /// Shown greyed out — "<name>'s OBS is still listening" — and never a call
    /// member: no volume, no speaking ring, not counted. While the account
    /// is in the channel, its OBS link shows nowhere.
    public func obsStillListening(in channelId: String) -> [Session] {
        let present = Set(voiceRoster(in: channelId).map(\.uid))
        var seen = Set<String>()
        return voiceSessions(in: channelId).filter { $0.isListener && !present.contains($0.uid) && seen.insert($0.uid).inserted }
    }

    /// The voice channel someone is in right now, if any (OBS listeners don't count).
    public func voiceChannel(of uid: String) -> Channel? {
        sessions(of: uid).compactMap(\.voice).first.flatMap { channelsById[$0.channelId] }
    }

    /// Sessions of one person (phone + desktop…), not counting OBS listeners.
    public func sessions(of uid: String) -> [Session] {
        sessions.values.filter { $0.uid == uid && !$0.isListener }
    }

    public func approve(_ uid: String) { send(.memberApprove(uid: uid)) }
    public func deny(_ uid: String) { send(.memberDeny(uid: uid)) }
    public func ban(_ uid: String) { send(.memberBan(uid: uid)) }
    public func setRole(_ uid: String, mod: Bool) { send(.memberRole(uid: uid, role: mod ? "mod" : "member")) }
    /// Owner/mods: a temporary password for them comes back as `pinReset`.
    public func resetPassword(for uid: String) { send(.memberResetPin(uid: uid)) }

    public func updateProfile(name: String? = nil, color: String? = nil) {
        if let name { credentials.name = name }
        if let color { credentials.color = color }
        send(.profile(name: name, color: color))
    }
}
