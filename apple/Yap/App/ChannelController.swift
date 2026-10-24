import SwiftUI
import YapCore

/// One open text channel's screen state and behaviour: opening and closing
/// it, the DEBUG launch hooks, jumping to a message (and flashing it),
/// editing / replying / the staged picture, the composer's Send, Esc and ↑,
/// and the sheets the channel's menu opens. `ChannelView` draws it; changes
/// are instant (Win98 steps, it never animates).
@MainActor
@Observable
final class ChannelController {
    enum Picker: String, Identifiable {
        case emoji, gif
        var id: String { rawValue }
    }

    let channelId: String
    @ObservationIgnored let model: AppModel
    /// How long a jumped-to message stays highlighted.
    static let flashFor: Duration = .seconds(1.2)

    /// Where the "New since" line goes (the read mark from before opening).
    var unreadAfter: Int?
    var draft = ""
    var editing: MessageEdit?
    var replyingTo: Message?
    /// A GIF or picture waiting to go out with the draft as its caption.
    var staged: StagedAttachment?
    var picker: Picker?
    var reactingTo: Message?
    var pinging = false
    var archiving = false
    /// The message a jump just landed on.
    var flashed: String?
    var showingPins = false
    var showingShelf = false
    var makingPoll = false
    var pill = UnreadPill()
    /// "↓ New messages" while scrolled up.
    var newBelow = NewMessagesPill()

    @ObservationIgnored private var opened = false
    /// The kept draft went into the box (not while the Personal space was locked).
    @ObservationIgnored private var restoredDraft = true
    @ObservationIgnored private var debugDone: Set<String> = []
    /// The rows on screen (the rows report in), so a jump can tell it got there.
    @ObservationIgnored private var onScreen: Set<String> = []

    init(model: AppModel, channelId: String) {
        self.model = model
        self.channelId = channelId
        // What you'd typed here before going elsewhere (not into a locked
        // Personal space: its box is shut, and the words would show in it).
        if model.client.channelsById[channelId]?.isPersonal == true && model.client.vaultLocked {
            restoredDraft = false
        } else {
            draft = model.drafts.text(for: channelId)
        }
    }

    var client: YapClient { model.client }
    var channel: Channel? { client.channelsById[channelId] }
    var isPersonal: Bool { channel?.isPersonal == true }

    /// Send (or Save) can go.
    var canSend: Bool { MessageEdit.canSend(draft: draft, staged: staged != nil, editing: editing, client: client) }

    // MARK: Lifecycle

    /// On appear: open the channel (the first time) and run the DEBUG
    /// launch switches (`YAP_SEARCH` is the Find window's to show).
    func appear() {
        if !opened {
            opened = true
            unreadAfter = client.open(channel: channelId)
        } else {
            client.activeChannelId = channelId
        }
        let launch = DebugLaunch.current
        if launch.wants("gifs") { picker = .gif }
        if launch.wants("emoji") { picker = .emoji }
        if launch.showPins { showingPins = true }
        if launch.showShelf { showingShelf = true }
        if launch.newPoll { makingPoll = true }
        if let demo = launch.draft, draft.isEmpty, !debugDone.contains("draft") {
            debugDone.insert("draft")
            draft = demo
        }
        if launch.wants("staged"), staged == nil, let data = DebugLaunch.demoPicture() {
            stage(.picture(data))
            draft = "new icon, thoughts?"
        }
    }

    func disappear() {
        client.close(channel: channelId)
        // The draft waits here (an edit in progress gives back the draft behind it).
        let text = editing?.savedDraft ?? draft
        if restoredDraft || !text.isEmpty { model.drafts.keep(text, for: channelId) }
    }

    /// After a reconnect the history reloads; keep the screen's channel active.
    func phaseChanged(_ phase: YapClient.Phase) {
        guard phase == .online else { return }
        client.activeChannelId = channelId
    }

    /// The server turned one of my messages away (not "slow down": those go
    /// again by themselves): its text comes back if the box is empty.
    func takeReturnedDraft() {
        guard let returned = client.returnedDraft, returned.channelId == channelId else { return }
        client.returnedDraft = nil
        if draft.isEmpty, editing == nil { draft = returned.text }
    }

    /// DEBUG screenshots, once the history is in: pretend the last N messages
    /// are new, start a reply, jump to a message. The message may arrive a
    /// moment after the first page, so the lookups retry briefly.
    func historyChanged(loading: Bool?) {
        guard loading == false else { return }
        let launch = DebugLaunch.current
        if let n = launch.unreadLast, let messages = client.histories[channelId]?.messages,
           messages.count > n, unreadAfter == nil || unreadAfter == 0 {
            unreadAfter = messages[messages.count - n - 1].ts
        }
        if let demo = launch.demoReply, !debugDone.contains("reply") {
            debugDone.insert("reply")
            find(demo) { [weak self] message in self?.replyingTo = message }
        }
        if let demo = launch.demoEdit, !debugDone.contains("edit") {
            debugDone.insert("edit")
            find(demo) { [weak self] message in if message.uid == self?.client.uid { self?.beginEdit(message) } }
        }
        if let ask = launch.ask, !debugDone.contains("ask") {
            debugDone.insert("ask")
            if ask == "ping" { pinging = true }
            if ask == "archive" { archiving = true }
        }
        if let demo = launch.jump, !debugDone.contains("jump") {
            debugDone.insert("jump")
            find(demo) { [weak self] message in self?.model.jump(to: message) }
        }
        if let count = launch.demoBurst, !debugDone.contains("burst") {
            debugDone.insert("burst")
            // More than the server takes at once (10): the rest are slowed down.
            for n in 1...max(1, count) { client.send(text: "Quick one \(n) of \(count)", in: channelId) }
        }
    }

    private func find(_ text: String, then use: @escaping (Message) -> Void) {
        Task {
            for _ in 0..<20 {
                if let message = client.histories[channelId]?.messages.last(where: { $0.body.contains(text) }) {
                    use(message)
                    return
                }
                try? await Task.sleep(for: .milliseconds(250))
            }
        }
    }

    // MARK: Jumping

    /// A jump asked for elsewhere (search, a pin, the switcher, Catch up).
    func takeJump(_ target: JumpTarget?, proxy: ScrollViewProxy) {
        guard let target, target.channelId == channelId else { return }
        model.jump = nil
        pill.seen = true
        jump(to: target.messageId, proxy: proxy)
    }

    /// Scroll to a message (a reply's quote, a search result, a pin), paging
    /// back until it's loaded, then flash it.
    func jump(to id: String, proxy: ScrollViewProxy) {
        Task {
            guard await client.ensureLoaded(id, in: channelId) else {
                client.notice = YapClient.Notice(text: "That message is too far back (or it's gone).")
                return
            }
            try? await Task.sleep(for: .milliseconds(100))
            let row = "msg:\(id)"
            proxy.scrollTo(row, anchor: .center)
            flashed = id
            var flashUntil = ContinuousClock.now + Self.flashFor
            // Did it get there?
            try? await Task.sleep(for: .milliseconds(50))
            if !onScreen.contains(row) {
                await walk(to: row, proxy: proxy)
                proxy.scrollTo(row, anchor: .center)
                flashUntil = .now + Self.flashFor
            }
            try? await Task.sleep(until: flashUntil, clock: .continuous)
            if flashed == id { flashed = nil }
        }
    }

    func rowAppeared(_ id: String) { onScreen.insert(id) }
    func rowDisappeared(_ id: String) { onScreen.remove(id) }

    /// The lazy list knows where a row is only once it has laid it out; for
    /// any other row `scrollTo` goes by a guess. With tall rows laid out at
    /// the bottom (a poll, dice, a long message), the guess for the few rows
    /// just above them lands past the end, so the list didn't move at all.
    /// So when a jump misses, walk there a screenful at a time, each step to
    /// a row that is laid out (the message just past the screen's edge),
    /// until the row is on screen; then the last `scrollTo` is exact.
    private func walk(to row: String, proxy: ScrollViewProxy) async {
        var gap = Int.max
        for _ in 0..<60 {
            guard !onScreen.contains(row),
                  let ids = client.histories[channelId]?.messages.map({ "msg:\($0.id)" }),
                  let step = Self.step(toward: row, in: ids, onScreen: onScreen), step.gap < gap else { return }
            gap = step.gap
            let before = onScreen
            proxy.scrollTo(step.row, anchor: step.anchor)
            var waited = 0
            while onScreen == before, waited < 10 {
                try? await Task.sleep(for: .milliseconds(30))
                waited += 1
            }
        }
    }

    /// One step of a walk: the row just past the screen's edge on the
    /// target's side (`rows` in order), to be scrolled to the other edge, and
    /// how many rows are still between. Nil when the target is on screen or
    /// nothing is.
    static func step(toward target: String, in rows: [String], onScreen: Set<String>) -> (row: String, anchor: UnitPoint, gap: Int)? {
        guard let index = rows.firstIndex(of: target), !onScreen.contains(target) else { return nil }
        let shown = rows.indices.filter { onScreen.contains(rows[$0]) }
        guard let top = shown.first, let bottom = shown.last else { return nil }
        if index < top { return (rows[top - 1], .bottom, top - index) }
        if index > bottom { return (rows[bottom + 1], .top, index - bottom) }
        return nil
    }

    /// The message being edited comes into view if the edit bar (or the
    /// draft) pushed it off, scrolling as little as it takes (the web's
    /// renderEditBar).
    func showEditing(_ id: String?, proxy: ScrollViewProxy) {
        guard let id else { return }
        Task {
            // After the edit bar has changed the chat's height.
            try? await Task.sleep(for: .milliseconds(150))
            guard editing?.message.id == id else { return }
            proxy.scrollTo("msg:\(id)", anchor: nil)
        }
    }

    // MARK: Composing

    func beginEdit(_ message: Message) {
        if let current = editing { draft = current.savedDraft }
        guard let edit = MessageEdit.begin(message, client: client, draft: &draft) else {
            client.notice = YapClient.Notice(text: "Unlock your Personal space to edit that.")
            return
        }
        staged = nil
        replyingTo = nil
        editing = edit
    }

    func beginReply(_ message: Message) {
        if let current = editing {
            draft = current.savedDraft
            editing = nil
        }
        replyingTo = message
    }

    /// Into the tray above the composer (an edit in progress is cancelled).
    func stage(_ attachment: StagedAttachment) {
        if let current = editing {
            draft = current.savedDraft
            editing = nil
        }
        staged = attachment
    }

    func stagePicture(_ data: Data) {
        stage(.picture(data))
    }

    func cancelEdit() {
        guard let edit = editing else { return }
        draft = edit.savedDraft
        editing = nil
    }

    func dropStaged() { staged = nil }
    func dropReply() { replyingTo = nil }

    /// A picked emoji goes on the end of the draft (custom ones as `:name:`).
    func insert(_ pick: EmojiPick) {
        switch pick {
        case .standard(let emoji): draft += emoji.char(tone: EmojiToneSetting.current)
        case .custom(let emoji):
            if let last = draft.last, !last.isWhitespace { draft += " " }
            draft += ":\(emoji.name): "
        }
    }

    /// Send, or save the edit. A staged GIF/picture takes the draft as its caption.
    func send() {
        guard let channel else { return }
        if let edit = editing {
            if client.edit(edit.message, text: draft) {
                draft = edit.savedDraft
                editing = nil
            }
        } else if let attachment = staged {
            let caption = draft
            let reply = replyingTo
            staged = nil
            replyingTo = nil
            draft = ""
            Task {
                if await !attachment.send(caption: caption, in: channel.id, replyTo: reply, model: model), draft.isEmpty { draft = caption }
            }
        } else if client.send(text: draft, in: channel.id, replyTo: replyingTo) {
            draft = ""
            replyingTo = nil
        }
    }

    /// Esc: drop the staged picture, then the reply, then the edit.
    func escape() -> KeyPress.Result {
        if staged != nil {
            dropStaged()
            return .handled
        }
        if replyingTo != nil, editing == nil {
            dropReply()
            return .handled
        }
        guard editing != nil else { return .ignored }
        cancelEdit()
        return .handled
    }

    /// ↑ in an empty composer (Mac): edit my last message.
    func editLast() -> KeyPress.Result {
        guard draft.isEmpty, editing == nil, let last = client.lastOwnMessage(in: channelId),
              let edit = MessageEdit.begin(last, client: client, draft: &draft) else { return .ignored }
        editing = edit
        return .handled
    }

    /// Return sends; Shift- or Option-Return is a new line.
    func returnPressed(_ press: KeyPress) -> KeyPress.Result {
        if press.modifiers.contains(.shift) || press.modifiers.contains(.option) { return .ignored }
        send()
        return .handled
    }
}

/// A channel's "…" menu: pins, pictures, a poll,
/// then Ping / Invite / Copy link / Archive (or, in your Personal space, its
/// passcode and Lock: it has no link), and Search.
struct ChannelMenuItems: View {
    @Environment(AppModel.self) private var model
    var chat: ChannelController
    var search: (() -> Void)? = nil

    var body: some View {
        if let search {
            Button("Search…", systemImage: "magnifyingglass", action: search)
        }
        Button("Pinned messages…", systemImage: "pin") { chat.showingPins = true }
        Button("Pictures & GIFs…", systemImage: "photo.on.rectangle.angled") { chat.showingShelf = true }
        if !chat.isPersonal {
            Button("New poll…", systemImage: "chart.bar") { chat.makingPoll = true }
        }
        Divider()
        if chat.isPersonal {
            PersonalSpaceMenuItems()
        } else {
            Button("Ping everyone…", systemImage: "megaphone") { chat.pinging = true }
            Button("Invite someone…", systemImage: "person.badge.plus") { model.showSettings(.invite) }
            if chat.channel?.kind == .text {
                LinkMenuItems(link: .channel(chat.channelId))
            }
            Divider()
            Button("Archive channel…", systemImage: "archivebox", role: .destructive) { chat.archiving = true }
        }
    }
}

/// The prompts behind the channel menu: Ping everyone (with a note) and
/// Archive channel.
struct ChannelDialogs: ViewModifier {
    @Bindable var chat: ChannelController

    func body(content: Content) -> some View {
        content
            .pingPrompt(isPresented: $chat.pinging)
            .question(isPresented: $chat.archiving, .archive(chat.channel?.name ?? "this channel") { [chat] in
                chat.client.archiveChannel(chat.channelId)
            })
    }
}

extension View {
    func channelDialogs(_ chat: ChannelController) -> some View {
        modifier(ChannelDialogs(chat: chat))
    }

    /// A row of the channel's list tells the controller when it's on screen (for jumps).
    func reportsOnScreen(_ id: String, to chat: ChannelController) -> some View {
        onAppear { chat.rowAppeared(id) }
            .onDisappear { chat.rowDisappeared(id) }
    }
}
