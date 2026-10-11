import Foundation

/// Search, pins, and paging back until a message (or the first unread) is loaded.
extension YapClient {
    public struct SearchResults: Equatable, Sendable {
        public var q: String
        /// nil = everywhere.
        public var channelId: String?
        public var server: [Message] = []
        /// My encrypted Personal-space notes that match (searched here, decrypted).
        public var sealed: [Message] = []
        public var loading = true

        /// Everything, newest first.
        public var messages: [Message] {
            (server + sealed).sorted { $0.ts > $1.ts }
        }
    }

    /// Search one channel (or everywhere). Encrypted Personal-space notes
    /// never match on the server; when the space is unlocked here, its
    /// loaded notes are searched on the device and merged in.
    public func runSearch(_ q: String, in channelId: String?) {
        let trimmed = q.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            search = nil
            return
        }
        var results = SearchResults(q: String(trimmed.prefix(200)), channelId: channelId)
        results.sealed = searchSealed(SearchQuery(trimmed), in: channelId)
        search = results
        if !send(.search(q: results.q, channelId: channelId)) {
            search?.loading = false
        }
    }

    public func clearSearch() { search = nil }

    func searchSealed(_ query: SearchQuery, in channelId: String?) -> [Message] {
        guard !query.isEmpty, vaultUnlocked, let personal = personalChannel?.id, channelId == nil || channelId == personal else { return [] }
        let me = users[uid]?.name ?? credentials.name ?? ""
        return (histories[personal]?.messages ?? []).filter { message in
            guard case .note(let note) = openNote(message.body) else { return false }
            return query.matches(text: note.text, author: me, hasImage: note.image != nil, hasGif: note.gif != nil)
        }
    }

    public func pin(_ message: Message, on: Bool) { send(.pin(id: message.id, on: on)) }

    /// Ask for a channel's pinned messages (they arrive in `pins`).
    public func loadPins(_ channelId: String) {
        guard isConnected, send(.pins(channelId: channelId)) else {
            pinsWanted.insert(channelId)
            return
        }
    }

    /// Makes sure a message is in the loaded history, paging back (up to
    /// `maxPages` older pages) until it turns up. For jumping to a search
    /// result, a pin, or the message a reply quotes.
    public func ensureLoaded(_ messageId: String, in channelId: String, maxPages: Int = 20) async -> Bool {
        await pageBack(in: channelId, maxPages: maxPages) { history in
            history.messages.contains { $0.id == messageId } ? true : history.hasMore ? nil : false
        }
    }

    /// Pages back until the first message after `ts` is loaded (the "New
    /// since" line), up to `maxPages` older pages.
    public func ensureLoaded(after ts: Int, in channelId: String, maxPages: Int = 20) async -> Bool {
        await pageBack(in: channelId, maxPages: maxPages) { history in
            !history.hasMore || (history.messages.first?.ts ?? .max) <= ts ? true : nil
        }
    }

    /// Loads older pages of a channel until `done` has an answer (nil =
    /// keep going), the history runs out of pages to ask for, or the
    /// connection drops.
    private func pageBack(in channelId: String, maxPages: Int, done: (ChannelHistory) -> Bool?) async -> Bool {
        if histories[channelId] == nil { loadLatest(channelId) }
        var pages = 0
        while true {
            // Wait for any page in flight.
            var waited = 0
            while histories[channelId]?.loading == true, waited < 200 {
                try? await Task.sleep(for: .milliseconds(50))
                waited += 1
            }
            guard let history = histories[channelId] else { return false }
            if let answer = done(history) { return answer }
            guard pages < maxPages, isConnected else { return false }
            loadOlder(channelId)
            pages += 1
            try? await Task.sleep(for: .milliseconds(20))
        }
    }
}
