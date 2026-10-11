import Foundation

/// Server frames: `handle(_:)` dispatches each one; `ready`, history pages and new messages.
extension YapClient {
    func handle(_ message: ServerMessage) {
        switch message {
        case .ready(let ready): onReady(ready)
        case let .pending(text, newUid, token):
            if let token { credentials.token = token }
            if let newUid {
                uid = newUid
                credentials.uid = newUid
            }
            pendingHello = nil
            authError = nil
            phase = .waiting(text)
        case let .error(code, text, re): onError(code: code, message: text, re: re)
        case let .history(channelId, messages, hasMore): onHistory(channelId, messages, hasMore)
        case let .message(m, nonce): onMessage(m, nonce: nonce)
        case let .deleted(id, channelId):
            // Erased everywhere it's shown: the chat, pins, search results, the media shelf.
            histories[channelId]?.messages.removeAll { $0.id == id }
            pins[channelId]?.removeAll { $0.id == id }
            shelves[channelId]?.items.removeAll { $0.id == id }
            search?.server.removeAll { $0.id == id }
            search?.sealed.removeAll { $0.id == id }
        case .vault(let wrap):
            syncVault(wrap)
        case let .edited(id, channelId, body, editedAt, sealed):
            if sealed { sealWaiter?(id) }
            guard let i = histories[channelId]?.messages.firstIndex(where: { $0.id == id }) else { return }
            histories[channelId]!.messages[i].body = body
            histories[channelId]!.messages[i].editedAt = editedAt > 0 ? editedAt : nil
            if sealed {
                histories[channelId]!.messages[i].gif = nil
                histories[channelId]!.messages[i].image = nil
            }
        case .personalPlain(let notes):
            plainWaiter?(notes)
            plainWaiter = nil
        case let .searchResults(q, channelId, messages):
            guard var current = search, current.q == q, current.channelId == channelId else { return }
            current.server = messages
            current.loading = false
            search = current
        case let .pinned(id, channelId, pinnedAt, _):
            if let i = histories[channelId]?.messages.firstIndex(where: { $0.id == id }) {
                histories[channelId]!.messages[i].pinnedAt = pinnedAt
            }
            if var list = pins[channelId] {
                list.removeAll { $0.id == id }
                if let pinnedAt {
                    if var message = histories[channelId]?.messages.first(where: { $0.id == id }) {
                        message.pinnedAt = pinnedAt
                        list.insert(message, at: 0)
                    } else {
                        send(.pins(channelId: channelId))
                    }
                }
                pins[channelId] = list
            }
        case let .pins(channelId, messages):
            pins[channelId] = messages
        case let .inviteCreated(invite, key):
            invites.removeAll { $0.id == invite.id }
            invites.insert(invite, at: 0)
            if let endpoint {
                var personal = endpoint
                personal.key = key
                createdInvite = CreatedInvite(invite: invite, link: personal.inviteLink)
            }
        case .invites(let list):
            invites = list
        case .links(let list):
            links = list
        case .modlog(let entries):
            modLog = entries
        case let .typing(channelId, uid):
            guard uid != self.uid else { return }
            typing[channelId, default: [:]][uid] = Date().addingTimeInterval(Self.typingShowFor)
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(Self.typingShowFor + 0.1))
                self?.pruneTyping()
            }
        case .user(let user):
            if user.role.isListed { users[user.id] = user } else { users.removeValue(forKey: user.id) }
            if user.id == uid {
                credentials.name = user.name
                credentials.color = user.color
            }
        case .sessionJoin(let session):
            sessions[session.sid] = session
            askForWatchIfNeeded()
        case .sessionLeave(let sid):
            // Someone whose call voice is keeping across their reconnect stays listed.
            if !heldSessions.contains(sid) { sessions.removeValue(forKey: sid) }
            voice?.clientSessionLeft(sid: sid)
        case .channel(let channel):
            channelsById[channel.id] = channel
            voice?.clientChannelChanged(channel)
            if let pendingCreate, pendingCreate.kind == channel.kind {
                self.pendingCreate = nil
                createdChannel = channel
            }
        case .channelArchived(let channelId):
            channelsById.removeValue(forKey: channelId)
            if activeChannelId == channelId { activeChannelId = nil }
        case let .voice(sid, channelId, muted, deafened, screen, resume, prev, lost):
            let before = sessions[sid]?.voice
            sessions[sid]?.voice = channelId.map { VoiceState(channelId: $0, muted: muted, deafened: deafened, screen: screen, resume: resume, prev: prev) }
            voice?.clientVoiceChanged(VoiceChange(sid: sid, channelId: channelId, before: before, lost: lost, prev: prev))
        case .emoji(let emoji):
            emojis[emoji.id] = emoji
        case .emojiDeleted(let id):
            emojis.removeValue(forKey: id)
        case let .account(token, kind):
            // A new password: nothing more to ask.
            if let token { credentials.token = token }
            weakPin = false
            weakReason = nil
            if let kind { secretKind = kind }
            pinChange = .saved
        case .devices(let list):
            devices = list
        case .deviceHandoff(let token):
            handoffWaiter?(token)
            handoffWaiter = nil
        case .signinAlert(let device):
            signinAlert = device
        case let .pinReset(uid, pin):
            pinReset = PinReset(name: users[uid]?.name ?? "them", pin: pin)
        case .joinRequest(let user):
            waitingRoom[user.id] = user
        case let .joinResolved(uid, approved, by, user):
            // The owner and mods knew them from the waiting room; everyone
            // else meets them here (`join.resolved.user`, from newer servers).
            // Not about me: just let in, I get "You're in" instead.
            let waiting = waitingRoom.removeValue(forKey: uid)
            guard uid != self.uid, let who = waiting ?? (approved ? user ?? users[uid] : nil) else { return }
            let actor = by == self.uid ? "you" : by == "admin" ? "the owner" : (users[by]?.name ?? "someone")
            if approved {
                // Let in: a calm line in the first channel, never a popup.
                guard let first = firstTextChannel else { return }
                let invitedBy = who.invite.map(\.by).flatMap { $0.isEmpty ? nil : $0 } ?? who.invitedBy.map(Self.inviter)
                addLocalNote(LocalNote(key: "join:\(who.id)", channelId: first.id, ts: Date().yapMillis,
                                       kind: .joined(name: who.name, color: who.color, invitedBy: invitedBy, letInBy: actor)))
            } else if by != self.uid {
                // Turned away: just a word.
                notice = Notice(text: "\(actor.prefix(1).uppercased() + actor.dropFirst()) turned away \(who.name).")
            }
        case let .view(sid, view):
            sessions[sid]?.view = view
        case let .watching(sid, on):
            sessions[sid]?.watching = on
            askForWatchIfNeeded()
        case let .watchState(state, _):
            watch = state
        case let .signal(from, data):
            if let data { voice?.clientReceivedSignal(data, from: from) }
        case let .ping(from, text):
            let user = users[from]
            notice = Notice(text: "📣 \(user?.name ?? "Someone") pinged everyone\(text.isEmpty ? "!" : ": \(text)")")
            if from != uid { onPing?(user, text) }
        case let .reaction(messageId, channelId, emoji, uid, on):
            guard let i = histories[channelId]?.messages.firstIndex(where: { $0.id == messageId }) else { return }
            histories[channelId]!.messages[i].applyReaction(emoji: emoji, uid: uid, on: on)
        case let .poll(id, channelId, votes):
            guard let i = histories[channelId]?.messages.firstIndex(where: { $0.id == id }),
                  let poll = histories[channelId]!.messages[i].poll else { return }
            histories[channelId]!.messages[i].poll = Poll(options: poll.options, votes: poll.options.indices.map { $0 < votes.count ? votes[$0] : [] })
        case let .sound(id, sid, uid):
            if let sound = sounds[id] { onSound?(sound, sid, uid) }
        case .soundAdded(let sound):
            sounds[sound.id] = sound
        case .soundDeleted(let id):
            // Soft: a chime someone uses keeps playing; a board sound leaves the board.
            if sounds[id]?.isChime == false { sounds.removeValue(forKey: id) }
        case .away(let note):
            attachAway(note)
        case let .shelf(channelId, items, hasMore):
            var shelf = shelves[channelId] ?? Shelf()
            let known = Set(shelf.items.map(\.id))
            shelf.items += items.filter { !known.contains($0.id) }
            shelf.hasMore = hasMore
            shelf.loading = false
            shelves[channelId] = shelf
        case .unreadCounts(let counts):
            onUnreadCounts(counts)
        case .hang(let hang):
            hangs.removeAll { $0.id == hang.id }
            hangs.append(hang)
            hangs.sort { $0.at < $1.at }
        case .hangCancelled(let id):
            hangs.removeAll { $0.id == id }
            if hangStarting?.id == id { hangStarting = nil }
        case .hangNow(let id):
            guard let hang = hangs.first(where: { $0.id == id }) else { return }
            hangStarting = hang
            onHangNow?(hang)
        case .notify(let settings):
            notifySettings = settings
        case .ignored:
            break
        }
    }

    /// "Ada (for Cleo)" → "Ada": who made the invite.
    static func inviter(_ invitedBy: String) -> String {
        guard let paren = invitedBy.range(of: " (", options: .backwards), invitedBy.hasSuffix(")") else { return invitedBy }
        return String(invitedBy[..<paren.lowerBound])
    }

    func onReady(_ ready: ReadyPayload) {
        // Let in just now (we were in the waiting room).
        let admitted = if case .waiting = phase { true } else { false }
        if let token = ready.token { credentials.token = token }
        // Keep the main key: a personal invite we came in with may be revoked later.
        if let key = ready.key, !key.isEmpty, var endpoint, endpoint.key != key {
            endpoint.key = key
            self.endpoint = endpoint
            credentials.endpoint = endpoint
        }
        links = ready.links
        uploadToken = ready.uploadToken
        resumeKey = ready.resumeKey
        sounds = Dictionary(ready.sounds.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        hangs = ready.hangs.sorted { $0.at < $1.at }
        if let notify = ready.notify { notifySettings = notify }
        unreadCounts = nil
        shelves = shelves.filter { shelfWanted.contains($0.key) }
        weakPin = ready.weakPin
        weakReason = ready.weakPin ? ready.weakReason : nil
        secretKind = ready.secretKind ?? secretKind
        uid = ready.uid
        syncVault(ready.vault)
        pendingHello = nil
        authError = nil
        // (uid is set above, before the vault looks up my key.)
        credentials.uid = ready.uid
        mySid = ready.sid
        channelsById = Dictionary(ready.channels.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        // A device that has never read anything (a newcomer, or me on a new
        // device) starts caught up, not with every channel unread since the
        // beginning (an empty channel gets 1: anything new counts).
        if lastRead.isEmpty {
            for channel in ready.channels where channel.kind != .voice {
                lastRead[channel.id] = max(1, channel.lastMessageAt ?? 1)
            }
            credentials.lastRead = lastRead
        }
        users = Dictionary(ready.users.filter { $0.role.isListed }.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        sessions = Dictionary(ready.sessions.map { ($0.sid, $0) }, uniquingKeysWith: { a, _ in a })
        // (Voice holds again whoever it's still keeping, in clientDidBecomeReady.)
        heldSessions = []
        emojis = Dictionary(ready.emojis.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        waitingRoom = Dictionary(ready.pending.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        if let me = users[uid] {
            credentials.name = me.name
            credentials.color = me.color
        }
        // Fresh history after every (re)connect, so nothing missed while away.
        histories = [:]
        typing = [:]
        phase = .online
        if let active = activeChannelId {
            if channelsById[active]?.hasMessages == true { loadLatest(active) } else { activeChannelId = nil }
        }
        // What didn't make it goes again, in the order it was written.
        pacer?.cancel()
        pacer = nil
        sendLine.reconnected()
        for nonce in Self.resendOrder(outbox, line: sendLine) {
            transmit(nonce)
        }
        // Pins a view asked for while offline (or the open lists, fresh).
        for channelId in pinsWanted.union(pins.keys) where channelsById[channelId] != nil {
            send(.pins(channelId: channelId))
        }
        pinsWanted = []
        for channelId in shelfWanted where channelsById[channelId] != nil {
            send(.shelf(channelId: channelId, before: nil))
        }
        shelfWanted = []
        watchRequested = false
        askForWatchIfNeeded()
        // How much is new where, with mentions that came while I was away.
        if !unreadChannels.isEmpty { requestUnreadCounts() }
        if admitted, let first = firstTextChannel {
            addLocalNote(LocalNote(key: "welcome", channelId: first.id, ts: Date().yapMillis, kind: .welcome))
            onAdmitted?()
        }
        voice?.clientDidBecomeReady()
    }

    func onHistory(_ channelId: String, _ messages: [Message], _ hasMore: Bool) {
        guard var history = histories[channelId] else { return }
        if history.loadingOlder {
            let known = Set(history.messages.map(\.id))
            history.messages = messages.filter { !known.contains($0.id) } + history.messages
        } else {
            // Keep anything that arrived live while the page was loading.
            let ids = Set(messages.map(\.id))
            let newest = messages.last?.ts ?? 0
            history.messages = messages + history.messages.filter { !ids.contains($0.id) && $0.ts > newest }
        }
        history.hasMore = hasMore
        history.loading = false
        history.loadingOlder = false
        histories[channelId] = history
        if channelId == activeChannelId, isAppActive { markRead(channelId) }
    }

    func onMessage(_ m: Message, nonce: String?) {
        if let nonce {
            sendLine.echoed(nonce)
            outbox.removeValue(forKey: nonce)
            if let notes = awayNotes.removeValue(forKey: nonce) { awayNotes[m.id] = notes }
        }
        if var channel = channelsById[m.channelId] {
            channel.lastMessageAt = max(channel.lastMessageAt ?? 0, m.ts)
            channelsById[m.channelId] = channel
        }
        if histories[m.channelId] != nil, !(histories[m.channelId]!.messages.contains { $0.id == m.id }) {
            histories[m.channelId]!.messages.append(m)
        }
        typing[m.channelId]?.removeValue(forKey: m.uid)
        if nonce == nil, mentionsMe(m) {
            if m.channelId != activeChannelId || !isAppActive { mentionCounts[m.channelId, default: 0] += 1 }
            onMention?(m)
        }
        // My own message reads the channel — except a note to myself from
        // another device (no nonce) in my Personal space: that's news here.
        let fromElsewhere = nonce == nil && channelsById[m.channelId]?.isPersonal == true
        if m.uid == uid, !fromElsewhere {
            markRead(m.channelId, ts: m.ts)
        } else if m.channelId == activeChannelId, isAppActive {
            markRead(m.channelId)
        }
    }

    func pruneTyping() {
        let now = Date()
        for (channel, map) in typing {
            let live = map.filter { $0.value > now }
            if live.count != map.count { typing[channel] = live }
        }
    }
}
