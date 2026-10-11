import SwiftUI
import YapCore
import YapVoice

/// One text channel, AIM-style: a white sunken chat log (day lines, the
/// "New" line, `Name: message`), a typing line, and the compose box. The
/// one menu has Ping Everyone and Archive (in your Personal space: the
/// passcode and Lock).
struct ChannelView: View {
    @Environment(AppModel.self) private var model
    let channelId: String
    @State private var chat: ChannelController
    @State private var finding = false
    /// Find, opened from Jump to… with these words.
    @State private var findText: String?

    init(model: AppModel, channelId: String) {
        self.channelId = channelId
        _chat = State(initialValue: ChannelController(model: model, channelId: channelId))
    }

    var body: some View {
        @Bindable var chat = chat
        let client = model.client
        let channel = client.channelsById[channelId]
        frame(title: channel?.name ?? "Archived", personal: channel?.isPersonal == true) {
            if let channel {
                messages(channel)
            } else {
                Text("This channel was archived. Its messages are kept, but it's hidden for everyone.")
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.grayText)
                    .padding()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .well()
            }
        }
        .channelDialogs(chat)
        .sheet(item: $chat.picker) { kind in
            switch kind {
            case .emoji:
                EmojiPickerView { pick in chat.insert(pick) }
            case .gif:
                GifPickerView { gif, query in
                    chat.stage(.gif(gif, query: query))
                }
            }
        }
        .sheet(item: $chat.reactingTo) { message in
            EmojiPickerView(title: "Add reaction") { pick in
                client.react(to: message, emoji: pick.reactionValue, on: true)
            }
        }
        .sheet(isPresented: $finding, onDismiss: { findText = nil }) { FindWindow98(channelId: channelId, initialText: findText) }
        .sheet(isPresented: $chat.showingPins) { PinsWindow98(channelId: channelId) }
        .sheet(isPresented: $chat.showingShelf) { Shelf98(channelId: channelId) }
        .sheet(isPresented: $chat.makingPoll) { Poll98(channelId: channelId) }
        .onChange(of: model.searchRequest, initial: true) { _, request in
            // Jump to…'s "Search messages for …": Find, everywhere, with those words.
            guard let request else { return }
            model.searchRequest = nil
            findText = request.text
            finding = true
        }
        .onAppear {
            chat.appear()
            if DebugLaunch.current.search != nil { finding = true }
        }
        .onChange(of: client.histories[channelId]?.loading, initial: true) { _, loading in chat.historyChanged(loading: loading) }
        .onDisappear { chat.disappear() }
        .onChange(of: client.phase) { _, phase in chat.phaseChanged(phase) }
        .onChange(of: client.returnedDraft) { chat.takeReturnedDraft() }
    }

    @ViewBuilder private var menuItems: some View {
        ChannelMenuItems(chat: chat) { finding = true }
    }

    /// iPhone: a navy navigation bar. Mac: a Win98 pane with title-bar
    /// buttons. Personal space: its state ("end-to-end encrypted" / "locked"
    /// / "only you · not encrypted") under the title.
    @ViewBuilder
    private func frame<Content: View>(title: String, personal: Bool, @ViewBuilder content: () -> Content) -> some View {
        let note = personal ? PersonalSpace.headerNote(model.client) : nil
        #if os(iOS)
        content()
            .padding(.horizontal, 2)
            .background(Theme.face)
            .modifier(InCallBar98())
            .win98NavigationBar(title, subtitle: note)
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    ToolbarMenuButton { menuItems }
                }
                .win98Item()
            }
            .background {
                // ⌘F with a keyboard (iPad): Find, like the Mac's title-bar button.
                Button("Find messages") { finding = true }
                    .keyboardShortcut("f", modifiers: .command)
                    .opacity(0)
                    .accessibilityHidden(true)
            }
        #else
        Window98(title: note.map { "\(title) — \($0)" } ?? title, systemImage: personal ? "lock.fill" : "text.bubble.fill") {
            HStack(spacing: 2) {
                Menu { menuItems } label: { Image(systemName: "ellipsis") }
                    .menuStyle(.button)
                    .menuIndicator(.hidden)
                    .buttonStyle(TitleBarButtonStyle())
                    .fixedSize()
                    .help("More")
                    .accessibilityLabel("More")
                Button { finding = true } label: { Image(systemName: "magnifyingglass") }
                    .buttonStyle(TitleBarButtonStyle())
                    .keyboardShortcut("f", modifiers: .command)
                    .help("Find messages (⌘F)")
                    .accessibilityLabel("Find messages")
                PeopleButton98(titleBar: true)
                    .buttonStyle(TitleBarButtonStyle())
            }
        } content: {
            content()
        }
        #endif
    }

    @ViewBuilder
    private func messages(_ channel: Channel) -> some View {
        let client = model.client
        let history = client.histories[channelId]
        let rows = MessageRows.build(channel: channel, messages: history?.messages ?? [], pending: client.pending(in: channelId),
                                     hasMore: history?.hasMore ?? true, loading: history?.loading ?? true,
                                     unreadAfter: chat.unreadAfter, myUid: client.uid,
                                     locked: { channel.isPersonal && client.openNote($0.body) == .locked },
                                     notes: client.localNotes(in: channelId))
        let locked = channel.isPersonal && client.vaultLocked
        VStack(spacing: 4) {
            if !channel.isPersonal {
                JoinRequests98()
            }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        if history == nil || (history!.loading && history!.messages.isEmpty) {
                            ProgressView().frame(maxWidth: .infinity).padding(.vertical, 40)
                        } else {
                            ForEach(rows) { row in
                                rowView(row, jump: { chat.jump(to: $0, proxy: proxy) })
                                    .reportsOnScreen(row.id, to: chat)
                            }
                        }
                    }
                    .padding(.vertical, 4)
                }
                .defaultScrollAnchor(.bottom)
                .defaultScrollAnchor(.bottom, for: .sizeChanges)
                .scrollDismissesKeyboard(.interactively)
                .well()
                .onChange(of: model.jump, initial: true) { _, target in chat.takeJump(target, proxy: proxy) }
                .onChange(of: chat.editing?.message.id) { _, id in chat.showEditing(id, proxy: proxy) }
                .overlay(alignment: .top) {
                    if chat.pill.show(channelId: channelId, unreadAfter: chat.unreadAfter, client: model.client) {
                        Button {
                            chat.pill.jump(channelId: channelId, unreadAfter: chat.unreadAfter, client: model.client) {
                                proxy.scrollTo("unread", anchor: .top)
                            }
                        } label: {
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.up").font(.system(size: 9, weight: .bold))
                                Text(chat.pill.jumping ? "Loading…" : "Jump to first unread")
                            }
                        }
                        .buttonStyle(.win98)
                        .padding(.top, 6)
                    }
                }
            }

            if !channel.isPersonal {
                TypingIndicator(channelId: channelId)
            }
            if locked {
                LockedCard98()
            }
            ComposerView(channel: channel, chat: chat)
                .disabled(locked)
        }
        .padding(.vertical, 4)
        .pastesPictures { chat.stagePicture($0) }
    }

    @ViewBuilder
    private func rowView(_ row: MessageRow, jump: @escaping (String) -> Void) -> some View {
        switch row {
        case .loadMore(_, let loading):
            HStack {
                Spacer()
                if loading { ProgressView().controlSize(.small) } else { Text("Loading older messages…").font(Theme.small).foregroundStyle(Theme.grayText) }
                Spacer()
            }
            .padding(.vertical, 10)
            .onAppear { model.client.loadOlder(channelId) }
        case .channelStart(_, let name):
            if model.client.channelsById[channelId]?.isPersonal == true {
                PersonalStart98()
            } else {
                Text("*** This is the very beginning of \(name). ***")
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.grayText)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 10)
            }
        case .day(_, let date):
            DayDivider(date: date)
        case .unread:
            UnreadDivider(label: SinceYouLeft.label(chat.unreadAfter))
                .onScrollVisibilityChange(threshold: 0.5) { chat.pill.lineAppeared($0) }
        case .message(let message, let grouped):
            let personal = model.client.channelsById[channelId]?.isPersonal == true
            let editing = chat.editing?.message.id == message.id
            MessageRowView(message: message, grouped: grouped, react: { chat.reactingTo = message },
                           edit: { chat.beginEdit(message) },
                           reply: personal ? nil : { chat.beginReply(message) },
                           jumpToReply: jump)
                .background(chat.flashed == message.id ? Theme.selection.opacity(0.35) : editing ? Theme.selection.opacity(0.16) : .clear)
                // The message you're editing: Windows' dotted focus box.
                .overlay {
                    if editing {
                        Rectangle().strokeBorder(Theme.text, style: StrokeStyle(lineWidth: 1, dash: [1, 1]))
                    }
                }
                .accessibilityValue(editing ? "Editing" : "")
        case .lockedNotes(_, let count):
            Text(PersonalSpace.lockedRun(count))
                .font(Theme.chat.italic())
                .foregroundStyle(Theme.grayText)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
        case .note(let note):
            LocalNote98(note: note)
        }
    }
}

/// This session's quiet line ("Cleo joined — invited by Ada", "You're in.
/// Say hi 👋"): small and gray, the name in their color, like the web.
struct LocalNote98: View {
    var note: LocalNote

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Image(systemName: note.symbol)
                .font(.system(size: 10, weight: .bold))
            Group {
                switch note.kind {
                case let .joined(name, color, _, _):
                    Text("\(Text(name).bold().foregroundStyle(Color.yap(hex: color) ?? Theme.text)) \(note.joinedText ?? "joined")")
                case .welcome:
                    Text("\(Text("You're in.").bold().foregroundStyle(Theme.text)) Say hi 👋")
                }
            }
            .fixedSize(horizontal: false, vertical: true)
        }
        .font(Theme.small)
        .foregroundStyle(Theme.grayText)
        .padding(.horizontal, 8)
        .padding(.top, 6)
        .padding(.bottom, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(note.plain)
    }
}

extension LocalNote {
    /// The web's icons: a person joining, sparkles for you.
    var symbol: String {
        switch kind {
        case .joined: "person.badge.plus"
        case .welcome: "sparkles"
        }
    }
}

/// The top of your Personal space: what it's for, then whether it's
/// encrypted (with Unlock / Encrypt buttons).
struct PersonalStart98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let client = model.client
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Image(systemName: "lock.fill").font(.system(size: 12, weight: .bold))
                Text("Your Personal space").font(Theme.chatBold)
            }
            .foregroundStyle(Theme.text)
            Text(PersonalSpace.intro)
                .font(Theme.chat)
                .foregroundStyle(Theme.text)
                .fixedSize(horizontal: false, vertical: true)
            if client.vaultUnlocked {
                note(icon: "lock.fill", bold: "End-to-end encrypted", text: String(PersonalSpace.encryptedNote.dropFirst("End-to-end encrypted".count)))
            } else if client.vaultWrap != nil {
                // Locked: the card above the compose box says so (with Unlock).
                EmptyView()
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    note(icon: "exclamationmark.triangle.fill", bold: "", text: PersonalSpace.notEncryptedWarning)
                    Button("Encrypt it with a passcode…") { model.vaultSheet = .setup }
                        .buttonStyle(.win98)
                }
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 10)
    }

    private func note(icon: String, bold: String, text: String) -> some View {
        HStack(alignment: .top, spacing: 6) {
            Image(systemName: icon)
            Text("\(Text(bold).bold())\(text)").fixedSize(horizontal: false, vertical: true)
        }
        .font(Theme.chrome)
        .foregroundStyle(Theme.tooltipText)
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(Theme.tooltip)
        .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
        .accessibilityElement(children: .combine)
    }
}

/// Above the compose box while the Personal space is locked here.
struct LockedCard98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "lock.fill").font(.system(size: 14, weight: .bold))
            VStack(alignment: .leading, spacing: 1) {
                Text("Locked on this device").font(Theme.chromeBold)
                Text("Your notes are encrypted. Enter your passcode to read and write.").font(Theme.small).foregroundStyle(Theme.grayText)
            }
            Spacer(minLength: 4)
            Button("Unlock…") { model.vaultSheet = .unlock }
                .buttonStyle(.win98Default)
        }
        .foregroundStyle(Theme.text)
        .padding(6)
        .bevel(.raised)
    }
}

struct DayDivider: View {
    var date: Date

    var body: some View {
        HStack(spacing: 8) {
            Separator98()
            Text(Stamps.day(date))
                .font(Theme.small)
                .foregroundStyle(Theme.grayText)
                .fixedSize()
            Separator98()
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 8)
        .accessibilityElement(children: .combine)
    }
}

/// "New since 9:41 PM": a red rule above the first message since you left.
struct UnreadDivider: View {
    var label = "New messages"

    var body: some View {
        HStack(spacing: 6) {
            Theme.danger.frame(height: 1)
            Text(label)
                .font(Theme.small.bold())
                .foregroundStyle(Theme.danger)
                .fixedSize()
            Theme.danger.frame(height: 1)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .accessibilityLabel(label)
    }
}

/// "Ada is typing…" in gray, like AIM's status line.
struct TypingIndicator: View {
    @Environment(AppModel.self) private var model
    var channelId: String

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let typers = model.client.typers(in: channelId, now: context.date)
            Text(typers.isEmpty ? " " : text(typers))
                .font(Theme.small)
                .italic()
                .foregroundStyle(Theme.grayText)
                .lineLimit(1)
                .padding(.horizontal, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func text(_ typers: [User]) -> String {
        switch typers.count {
        case 1: "\(typers[0].name) is typing…"
        case 2: "\(typers[0].name) and \(typers[1].name) are typing…"
        default: "Several people are typing…"
        }
    }
}
