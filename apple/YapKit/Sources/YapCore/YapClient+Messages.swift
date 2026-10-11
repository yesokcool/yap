import Foundation

/// Text channels: history paging, sending (text, polls, GIFs, pictures), editing, deleting, reactions, mentions, typing and read marks.
extension YapClient {
    public func loadLatest(_ channelId: String) {
        histories[channelId] = ChannelHistory()
        if !send(.history(channelId: channelId, before: nil, beforeId: nil, limit: Self.historyPage)) {
            histories[channelId]?.loading = false
        }
    }

    public func loadOlder(_ channelId: String) {
        guard var history = histories[channelId], !history.loading, history.hasMore, let oldest = history.messages.first else { return }
        history.loading = true
        history.loadingOlder = true
        histories[channelId] = history
        if !send(.history(channelId: channelId, before: oldest.ts, beforeId: oldest.id, limit: Self.historyPage)) {
            histories[channelId]?.loading = false
            histories[channelId]?.loadingOlder = false
        }
    }

    /// Opens a channel on screen; returns the read mark from before opening
    /// (where the "New" line goes).
    @discardableResult
    public func open(channel channelId: String) -> Int? {
        let before = lastRead[channelId]
        activeChannelId = channelId
        if histories[channelId] == nil || histories[channelId]?.messages.isEmpty == true && histories[channelId]?.loading == false {
            loadLatest(channelId)
        }
        return before
    }

    public func close(channel channelId: String) {
        if activeChannelId == channelId { activeChannelId = nil }
    }

    public var emojisByName: [String: Emoji] {
        Dictionary(emojis.values.map { ($0.name.lowercased(), $0) }, uniquingKeysWith: { a, _ in a })
    }

    /// Sends text (custom `:name:` and standard `:shortcode:` converted first),
    /// optionally as a reply. In an encrypted Personal space it's sealed
    /// first. Returns false when there's nothing to send, it's too long, or
    /// the space is locked.
    @discardableResult
    public func send(text: String, in channelId: String, replyTo: Message? = nil) -> Bool {
        if case let .poll(question, options)? = SlashCommand.parse(text) {
            return sendPoll(question: question, options: options, in: channelId)
        }
        let encoded = outgoing(text)
        guard !encoded.isBlank else { return false }
        guard fits(encoded) else { return false }
        let personal = channelsById[channelId]?.isPersonal == true
        var body = encoded
        if isSealed(channelId) {
            guard let sealed = seal(Vault.Note(text: encoded)) else { return false }
            body = sealed
        }
        let reply = personal ? nil : replyTo.map(Reply.init(quoting:))
        let nonce = UUID().uuidString
        lastTypingSent[channelId] = nil
        queue(Message(id: nonce, channelId: channelId, uid: uid, body: body, ts: Date().yapMillis, reply: reply, isPending: true), nonce: nonce)
        return true
    }

    /// A poll: the question is the body (custom `:name:` converted), 2–10
    /// answers. Not in a Personal space. False (with a notice) when it won't do.
    @discardableResult
    public func sendPoll(question: String, options: [String], in channelId: String) -> Bool {
        if channelsById[channelId]?.isPersonal == true {
            notice = Notice(text: "Polls are for channels with other people in them.")
            return false
        }
        if let problem = Poll.problem(question: question, options: options) {
            notice = Notice(text: problem)
            return false
        }
        let body = EmojiCodec.encode(question.trimmingCharacters(in: .whitespacesAndNewlines), custom: emojisByName, tone: emojiTone)
        let answers = options.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
        let nonce = UUID().uuidString
        queue(Message(id: nonce, channelId: channelId, uid: uid, body: body, ts: Date().yapMillis, poll: Poll(options: answers),
                      isPending: true), nonce: nonce)
        return true
    }

    /// Vote for `option` (nil = take my vote back). Shown right away; the
    /// server's `poll` tally follows.
    public func vote(_ message: Message, option: Int?) {
        guard let poll = message.poll, !message.isPending else { return }
        if let i = histories[message.channelId]?.messages.firstIndex(where: { $0.id == message.id }) {
            histories[message.channelId]!.messages[i].poll = poll.voting(uid, for: option)
        }
        send(.pollVote(id: message.id, option: option))
    }

    /// Bookmark a message into my Personal space (PROTOCOL.md › Save to
    /// Personal): a note replying to it, with an optional comment. False
    /// when there's no Personal space or it's locked.
    @discardableResult
    public func saveToPersonal(_ message: Message, comment: String = "") -> Bool {
        guard let personal = personalChannel, !message.isPending else { return false }
        var body = outgoing(comment)
        if isSealed(personal.id), !body.isEmpty {
            guard let sealed = seal(Vault.Note(text: body)) else { return false }
            body = sealed
        }
        let nonce = UUID().uuidString
        queue(Message(id: nonce, channelId: personal.id, uid: uid, body: body, ts: Date().yapMillis,
                      reply: Reply(quoting: message), isPending: true), nonce: nonce)
        if isSealed(personal.id), !savedToPersonalBefore() {
            notice = Notice(text: "Saved to your Personal space. Your notes are encrypted, but Yap's server can see which message you saved.")
        } else {
            notice = Notice(text: "Saved to your Personal space.")
        }
        return true
    }

    // MARK: Sending, in order (PROTOCOL.md › Messages)

    /// Every message goes through here: into the outbox (shown dimmed until
    /// its echo), then out — or into the line while sends go one at a time
    /// after a `slow_down`.
    func queue(_ message: Message, nonce: String) {
        outbox[nonce] = message
        sendLine.wrote(nonce)
        if sendLine.pacing {
            sendLine.hold(nonce)
        } else {
            transmit(nonce)
        }
    }

    /// The outbox copy, on the wire (false: no socket; it goes on `ready`).
    @discardableResult
    func transmit(_ nonce: String) -> Bool {
        guard let m = outbox[nonce] else { return false }
        guard send(.send(channelId: m.channelId, body: m.body, nonce: nonce, gif: m.gif, image: m.image, replyTo: m.reply?.id,
                         poll: m.poll?.options)) else { return false }
        sendLine.sent(nonce)
        return true
    }

    /// `error {re: "send"}`: the server answers sends in order, so it's
    /// about my oldest unanswered one. `slow_down`: it goes again (same
    /// nonce) in its turn, quietly. Anything else (or a sixth `slow_down`):
    /// the stuck copy goes, the reason shows, and the text comes back to an
    /// empty message box.
    func sendRefused(code: String, message: String) {
        switch sendLine.refused(code: code) {
        case .again?:
            startPacing()
        case .refused(let nonce)?:
            notice = Notice(text: message)
            sendLine.forget(nonce)
            awayNotes.removeValue(forKey: nonce)
            guard let m = outbox.removeValue(forKey: nonce) else { return }
            if activeChannelId == m.channelId, let text = returnableText(of: m), !text.isBlank {
                returnedDraft = ReturnedDraft(channelId: m.channelId, text: text)
            }
        case nil:
            notice = Notice(text: message)
        }
    }

    /// One at a time, 1.1 s apart, until nothing is waiting.
    func startPacing() {
        guard pacer == nil else { return }
        pacer = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: SendLine.resendAfter)
                guard let self, !Task.isCancelled else { return }
                guard let nonce = self.sendLine.next() else {
                    self.pacer = nil
                    return
                }
                // No socket: everything in the outbox goes again on `ready`.
                if !self.transmit(nonce) {
                    self.pacer = nil
                    return
                }
            }
        }
    }

    /// What didn't make it, in the order it was written. (By time alone, a
    /// burst written in the same millisecond came out in any order: twelve
    /// quick messages resent after a reconnect were stored 1, 11, 9, 8, 10, 2…)
    static func resendOrder(_ outbox: [String: Message], line: SendLine) -> [String] {
        outbox.sorted { a, b in
            (line.position(of: a.key) ?? .max, a.value.ts, a.key) < (line.position(of: b.key) ?? .max, b.value.ts, b.key)
        }.map(\.key)
    }

    /// A refused message's text as it was typed (a poll as its `/poll` line).
    func returnableText(of message: Message) -> String? {
        if let poll = message.poll {
            return (["/poll " + EmojiCodec.plain(message.body)] + poll.options).joined(separator: " | ")
        }
        return editableText(of: message)
    }

    /// The 40,000-character cap (a notice when it's over).
    func fits(_ body: String) -> Bool {
        guard body.utf16.count <= MessageLength.maxChars else {
            notice = Notice(text: "Messages max out at 40,000 characters.")
            return false
        }
        return true
    }

    /// A GIF, with an optional caption (custom `:name:` converted like `send`).
    /// In an encrypted Personal space the GIF rides inside the sealed note
    /// (and the caller shouldn't tell KLIPY: see `isSealed`).
    @discardableResult
    public func send(gif: Gif, caption: String = "", in channelId: String, replyTo: Message? = nil) -> Bool {
        let body = outgoing(caption)
        guard fits(body) else { return false }
        let nonce = UUID().uuidString
        if isSealed(channelId) {
            guard let sealed = seal(Vault.Note(text: body, gif: gif)) else { return false }
            queue(Message(id: nonce, channelId: channelId, uid: uid, body: sealed, ts: Date().yapMillis, isPending: true), nonce: nonce)
            return true
        }
        let reply = channelsById[channelId]?.isPersonal == true ? nil : replyTo.map(Reply.init(quoting:))
        queue(Message(id: nonce, channelId: channelId, uid: uid, body: body, ts: Date().yapMillis, gif: gif, reply: reply, isPending: true), nonce: nonce)
        return true
    }

    /// A picture (already prepared with `PictureEncoder`), with an optional
    /// caption: upload, then send it as `image`.
    /// In an encrypted Personal space the picture is sealed on the device,
    /// uploaded as an opaque blob, and the note says where it is.
    public func send(picture: PictureEncoder.Prepared, caption: String = "", in channelId: String, replyTo: Message? = nil) async throws {
        let (api, sid, token) = try uploadTarget()
        let body = outgoing(caption)
        guard body.utf16.count <= MessageLength.maxChars else { throw ServerAPI.UploadError(status: 0, message: "Messages max out at 40,000 characters.") }
        let nonce = UUID().uuidString
        let started = session
        if isSealed(channelId) {
            guard let key = vaultKey else { throw Vault.Failure.cantOpen }
            let sealedBytes = try Vault.seal(blob: picture.data, with: key)
            let id = try await api.uploadBlob(sealedBytes, sid: sid, uploadToken: token)
            try stillSignedIn(started)
            let image = Vault.SealedImage(id: id, width: picture.width, height: picture.height, mime: picture.mime)
            guard let sealed = seal(Vault.Note(text: body, image: image)) else { throw Vault.Failure.cantOpen }
            queue(Message(id: nonce, channelId: channelId, uid: uid, body: sealed, ts: Date().yapMillis, isPending: true), nonce: nonce)
            return
        }
        let id = try await api.uploadImage(picture.data, mime: picture.mime, sid: sid, uploadToken: token)
        try stillSignedIn(started)
        let image = ImageRef(id: id, width: picture.width, height: picture.height)
        let reply = channelsById[channelId]?.isPersonal == true ? nil : replyTo.map(Reply.init(quoting:))
        queue(Message(id: nonce, channelId: channelId, uid: uid, body: body, ts: Date().yapMillis, image: image, reply: reply, isPending: true), nonce: nonce)
    }

    /// Edit one of my messages (custom `:name:` converted like `send`). The
    /// server's `edited` broadcast updates every copy. False when there's
    /// nothing to save (empty text on a text-only message, or too long).
    ///
    /// An encrypted note is re-sealed with the new text, keeping its GIF or
    /// picture (needs the space unlocked).
    @discardableResult
    public func edit(_ message: Message, text: String) -> Bool {
        guard message.uid == uid, !message.isPending else { return false }
        let body = outgoing(text)
        guard fits(body) else { return false }
        if Vault.isEncryptedNote(message.body) {
            guard case .note(var note) = openNote(message.body) else { return false }
            if body.isBlank, note.gif == nil, note.image == nil { return false }
            if body == note.text { return true }
            note.text = body
            guard let sealed = seal(note) else { return false }
            return send(.edit(id: message.id, body: sealed))
        }
        if body.isBlank, message.gif == nil, message.image == nil { return false }
        if body == message.body { return true }
        return send(.edit(id: message.id, body: body))
    }

    /// The text to put in the composer when editing (an encrypted note's
    /// own text; nil while it can't be opened).
    public func editableText(of message: Message) -> String? {
        if Vault.isEncryptedNote(message.body) {
            guard case .note(let note) = openNote(message.body) else { return nil }
            return EmojiCodec.plain(note.text)
        }
        return EmojiCodec.plain(message.body)
    }

    /// My newest message in a channel (Up-arrow in an empty composer edits it).
    public func lastOwnMessage(in channelId: String) -> Message? {
        histories[channelId]?.messages.last { $0.uid == uid && !$0.isPending }
    }

    /// Typed text as it goes out: trailing whitespace dropped (leading
    /// spaces may be indentation), custom `:name:` emoji encoded.
    func outgoing(_ text: String) -> String {
        var trimmed = text
        while let last = trimmed.last, last.isWhitespace { trimmed.removeLast() }
        return EmojiCodec.encode(trimmed, custom: emojisByName, tone: emojiTone)
    }

    public func pending(in channelId: String) -> [Message] {
        outbox.values.filter { $0.channelId == channelId }.sorted { $0.ts < $1.ts }
    }

    /// Deletes a message; an encrypted note's picture is erased with it.
    public func delete(_ message: Message) {
        var blobs: [String]?
        if message.uid == uid, Vault.isEncryptedNote(message.body), case .note(let note) = openNote(message.body), let image = note.image {
            blobs = [image.id]
        }
        send(.delete(id: message.id, blobs: blobs))
    }

    public func react(to message: Message, emoji: String, on: Bool) {
        send(.react(id: message.id, emoji: emoji, on: on))
    }

    /// Tap on a reaction pill: add mine, or take it back.
    public func toggleReaction(_ emoji: String, on message: Message) {
        let mine = message.reactions.first { $0.emoji == emoji }?.uids.contains(uid) == true
        react(to: message, emoji: emoji, on: !mine)
    }

    /// 📣 Ping everyone online (2 per 30 s).
    public func pingEveryone(_ text: String? = nil) {
        let trimmed = text?.trimmingCharacters(in: .whitespacesAndNewlines)
        send(.ping(text: trimmed?.isEmpty == false ? trimmed : nil))
    }

    /// Member names, for @mention highlighting and autocomplete.
    public var memberNames: [String] { users.values.map(\.name) }

    /// Does this message @-mention me (or @everyone / @here)? The server's
    /// rule exactly (`Mentions`), so the highlight and the push agree.
    public func mentionsMe(_ m: Message) -> Bool {
        guard m.uid != uid else { return false }
        // A reply to me counts as a mention.
        if let reply = m.reply, reply.uid == uid, !reply.deleted { return true }
        guard !m.body.isEmpty else { return false }
        var people = users.values.map { (id: $0.id, name: $0.name) }
        if users[uid] == nil, let name = credentials.name { people.append((id: uid, name: name)) }
        let found = Mentions.find(in: m.body, among: people)
        return found.everyone || found.uids.contains(uid)
    }

    /// Call as the person types; throttled to one `typing` per 3.5 s.
    public func userTyped(_ text: String, in channelId: String) {
        guard !text.isBlank else { return }
        // Nobody else is in a Personal space.
        if channelsById[channelId]?.isPersonal == true { return }
        let now = Date()
        if let last = lastTypingSent[channelId], now.timeIntervalSince(last) < Self.typingSendEvery { return }
        lastTypingSent[channelId] = now
        send(.typing(channelId: channelId))
    }

    public func typers(in channelId: String, now: Date = Date()) -> [User] {
        (typing[channelId] ?? [:]).filter { $0.value > now && $0.key != uid }.compactMap { users[$0.key] }.sorted { $0.name < $1.name }
    }

    public func markRead(_ channelId: String, ts: Int? = nil) {
        if mentionCounts[channelId] != nil, channelId == activeChannelId { mentionCounts.removeValue(forKey: channelId) }
        guard let latest = ts ?? channelsById[channelId]?.lastMessageAt else { return }
        if (lastRead[channelId] ?? 0) >= latest { return }
        lastRead[channelId] = latest
        credentials.lastRead = lastRead
    }

    public func isUnread(_ channel: Channel) -> Bool {
        guard channel.hasMessages, let last = channel.lastMessageAt else { return false }
        if channel.id == activeChannelId && isAppActive { return false }
        return last > (lastRead[channel.id] ?? 0)
    }

    /// Your Personal space (only you ever see it), when the server has one.
    public var personalChannel: Channel? {
        channelsById.values.first { $0.isPersonal }
    }
}
