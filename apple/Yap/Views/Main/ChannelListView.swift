import SwiftUI
import YapCore
import YapVoice

/// The channel list: your Personal space, text channels, voice channels
/// (with who's in them), places and the Minecraft server. Lists end in
/// "New …" rows; the one menu holds the rest. iPhone: a plain white list
/// under a navy bar, with your profile card (→ Settings), big Mute / Deafen
/// buttons and the call at the top and the status bar at the bottom. Mac: a
/// Win98 pane (the profile card sits above it).
struct ChannelListView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    @State private var newChannelKind: ChannelKind?
    @State private var archiving: Channel?
    @State private var addingPlace = false
    @State private var removingPlace: PlaceLink?
    @State private var pinging = false

    var body: some View {
        content
            .pingPrompt(isPresented: $pinging)
            .onChange(of: model.client.createdChannel) { _, channel in
                // A text channel I just made opens right away.
                if let channel, channel.kind == .text { model.open(.channel(channel.id)) }
            }
            .sheet(item: $newChannelKind) { kind in
                NewChannelSheet(kind: kind)
            }
            .sheet(isPresented: $addingPlace) { NewPlaceSheet() }
            .question(isPresented: Binding(get: { removingPlace != nil }, set: { if !$0 { removingPlace = nil } }),
                      .removePlace(removingPlace?.name ?? "this place") { [client = model.client, removingPlace] in
                          if let removingPlace { client.removeLink(removingPlace.id) }
                      })
            .question(isPresented: Binding(get: { archiving != nil }, set: { if !$0 { archiving = nil } }),
                      .archive(archiving?.name ?? "this channel") { [client = model.client, archiving] in
                          if let archiving { client.archiveChannel(archiving.id) }
                      })
    }

    @ViewBuilder private var menuItems: some View {
        Button("Jump to…", systemImage: "magnifyingglass") { model.showingSwitcher = true }
        Button("Catch up", systemImage: "tray.full") {
            model.showingCatchUp = true
        }
        Button("Plan a hang…", systemImage: "calendar.badge.plus") { model.planningHang = true }
        Divider()
        Button("New text channel", systemImage: "text.bubble") { newChannelKind = .text }
        Button("New voice channel", systemImage: "speaker.wave.2") { newChannelKind = .voice }
        Divider()
        Button("Ping everyone…", systemImage: "megaphone") { pinging = true }
        Button("Invite someone…", systemImage: "person.badge.plus") { model.showSettings(.invite) }
    }

    #if os(iOS)
    private var content: some View {
        List {
            rows
        }
        .listStyle(.plain)
        .environment(\.defaultMinListRowHeight, 34)
        .scrollContentBackground(.hidden)
        .background(Theme.window)
        .safeAreaInset(edge: .top, spacing: 0) {
            VStack(spacing: 0) {
                AccountNotices98()
                HangNotice98()
                UserPanel()
                if model.voice.isInVoice {
                    VoiceBar()
                        .padding(.vertical, 5)
                        .bevel(.raised)
                }
            }
        }
        .win98NavigationBar("Yap")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                PeopleButton98()
                    .buttonStyle(.win98Compact)
                    .appScheme()
            }
            .win98Item()
            ToolbarItem(placement: .primaryAction) {
                ToolbarMenuButton { menuItems }
            }
            .win98Item()
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 0) {
                SidebarFooter98()
                StatusBar98()
            }
        }
    }
    #else
    private var content: some View {
        Window98(title: "Yap — Channels", systemImage: "bubble.left.and.bubble.right.fill") {
            Menu { menuItems } label: {
                Image(systemName: "ellipsis")
            }
            .menuStyle(.button)
            .menuIndicator(.hidden)
            .buttonStyle(TitleBarButtonStyle())
            .fixedSize()
            .help("More")
            .accessibilityLabel("More")
        } content: {
            VStack(spacing: 0) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        rows
                    }
                    .padding(.vertical, 2)
                }
                .well()
                SidebarFooter98()
            }
        }
    }
    #endif

    @ViewBuilder private var rows: some View {
        let client = model.client
        if client.unreadChannels.count >= CatchUp.showsFrom {
            Button {
                model.showingCatchUp = true
            } label: {
                SidebarLabel(title: "Catch up", systemImage: "tray.full.fill", bold: true) {
                    ViewThatFits {
                        Text("\(client.unreadChannels.count) channels")
                        Text("\(client.unreadChannels.count)")
                    }
                    .font(Theme.small)
                    .foregroundStyle(Theme.grayText)
                }
            }
            .buttonStyle(.plain)
            .help("What's new since you last looked")
            .modifier(PlainRow())
        }
        if let personal = client.personalChannel {
            link(.channel(personal.id)) { selected in
                PersonalChannelRow(channel: personal, selected: selected)
            }
        }
        group("Text channels")
        ForEach(client.textChannels) { channel in
            link(.channel(channel.id)) { selected in
                TextChannelRow(channel: channel, selected: selected)
            }
            .contextMenu {
                Button("Archive channel…", systemImage: "archivebox", role: .destructive) { archiving = channel }
            }
        }
        NewRow(title: "New channel…") { newChannelKind = .text }

        group("Voice channels")
        ForEach(client.voiceChannels) { channel in
            VoiceChannelRow(channel: channel, archive: { archiving = channel })
                .modifier(PlainRow())
            ForEach(client.voiceRoster(in: channel.id)) { session in
                VoiceMemberRow(session: session)
                    .modifier(PlainRow())
            }
        }
        NewRow(title: "New voice channel…") { newChannelKind = .voice }

        group("Places")
        ForEach(model.places) { place in
            placeRow(place)
                .contextMenu {
                    Button("Remove place…", systemImage: "trash", role: .destructive) { removingPlace = place }
                }
        }
        ForEach([WebPage.draw, .space], id: \.self) { page in
            link(.web(page)) { selected in
                PlaceRow(title: page.title, systemImage: page.systemImage,
                         viewers: client.sessions.values.filter { $0.view == page.place }, selected: selected)
            }
        }
        if client.endpoint?.watchURL != nil {
            Button { client.openWeb("watch", with: openURL) } label: {
                SidebarLabel(title: "Watch", systemImage: "play.rectangle.fill") {
                    if !client.watchAudience.isEmpty {
                        Text("\(client.watchAudience.count) watching").font(Theme.small)
                    }
                    Image(systemName: "arrow.up.right").font(.system(size: 9, weight: .bold)).foregroundStyle(Theme.grayText)
                }
            }
            .buttonStyle(.plain)
            .help(ScreenText.watchHelp)
            .accessibilityHint("Opens in the browser")
            .modifier(PlainRow())
            if let title = client.watchTitle {
                Text("▶ \(title)")
                    .font(Theme.small)
                    .foregroundStyle(Theme.grayText)
                    .lineLimit(1)
                    .padding(.leading, 30)
                    .padding(.trailing, 6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .modifier(PlainRow())
            }
            ForEach(client.watchAudience) { session in
                WatchMemberRow98(session: session).modifier(PlainRow())
            }
        }
        if client.endpoint?.whensGoodURL != nil {
            Button { client.openWeb("when", with: openURL) } label: {
                SidebarLabel(title: "When's good", systemImage: "calendar") {
                    Image(systemName: "arrow.up.right").font(.system(size: 9, weight: .bold)).foregroundStyle(Theme.grayText)
                }
            }
            .buttonStyle(.plain)
            .help("A weekly planner: paint when you're free (opens in the browser)")
            .accessibilityHint("Opens in the browser")
            .modifier(PlainRow())
        }
        ForEach(client.upcomingHangs()) { hang in
            HangRow98(hang: hang).modifier(PlainRow())
        }
        NewRow(title: "Plan a hang…") { model.planningHang = true }
        NewRow(title: "New place…") { addingPlace = true }

        if model.config?.minecraft != nil || model.minecraft != nil {
            group("Minecraft")
            MinecraftRow().modifier(PlainRow())
        }

        // Settings: the profile card at the bottom (UserPanel).
    }

    /// The board opens inside Yap; any other Place opens in the browser.
    @ViewBuilder
    private func placeRow(_ place: PlaceLink) -> some View {
        if place.view == "board" {
            link(.web(.board)) { selected in
                PlaceRow(title: place.name, systemImage: "square.grid.3x2.fill",
                         viewers: model.client.sessions.values.filter { $0.view == "board" }, selected: selected)
            }
        } else if let url = URL(string: place.url) {
            Button { openURL(url) } label: {
                SidebarLabel(title: place.name, systemImage: "arrow.up.forward.app.fill") {
                    Image(systemName: "arrow.up.right").font(.system(size: 9, weight: .bold)).foregroundStyle(Theme.grayText)
                }
            }
            .buttonStyle(.plain)
            .help(place.description ?? place.url)
            .accessibilityHint("Opens in the browser")
            .modifier(PlainRow())
        }
    }

    private func group(_ title: String) -> some View {
        Text(title)
            .font(Theme.chromeBold)
            .foregroundStyle(Theme.text)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.face)
            .overlay(alignment: .bottom) { Theme.shadow.frame(height: 1) }
            .accessibilityAddTraits(.isHeader)
            .modifier(PlainRow())
    }

    /// iPhone: a push. Mac: a selectable row (solid selection color).
    @ViewBuilder
    private func link<Label: View>(_ destination: Destination, @ViewBuilder label: (Bool) -> Label) -> some View {
        #if os(iOS)
        NavigationLink(value: destination) { label(false) }
            .listRowBackground(Theme.window)
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 12))
        #else
        let selected = model.selection == destination
        Button { model.selection = destination } label: { label(selected) }
            .buttonStyle(.plain)
        #endif
    }
}

/// Rows in the plain list: no separators, white background, no insets.
struct PlainRow: ViewModifier {
    func body(content: Content) -> some View {
        content
            #if os(iOS)
            .listRowBackground(Theme.window)
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets())
            #endif
    }
}

extension ChannelKind: @retroactive Identifiable {
    public var id: String { rawValue }
}

/// Icon + text, with the solid selection color when selected.
struct SidebarLabel<Trailing: View>: View {
    var title: String
    var systemImage: String
    var selected = false
    var bold = false
    var iconColor: Color = Theme.text
    @ViewBuilder var trailing: Trailing
    /// The icon grows with the text (Dynamic Type).
    @ScaledMetric(relativeTo: .footnote) private var iconSize: CGFloat = 11

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: systemImage)
                .font(.system(size: iconSize, weight: .bold))
                .foregroundStyle(selected ? Theme.selectionText : iconColor)
                .frame(width: iconSize + 7)
            Text(title)
                .font(bold ? Theme.chatBold : Theme.chat)
                .foregroundStyle(selected ? Theme.selectionText : Theme.text)
                .lineLimit(1)
                // With big text the name wins over the detail on the right.
                .layoutPriority(1)
            Spacer(minLength: 4)
            trailing
                .foregroundStyle(selected ? Theme.selectionText : Theme.text)
                .lineLimit(1)
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 3)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(selected ? Theme.selection : .clear)
        .contentShape(.rect)
    }
}

extension SidebarLabel where Trailing == EmptyView {
    init(title: String, systemImage: String, selected: Bool = false, bold: Bool = false, iconColor: Color = Theme.text) {
        self.init(title: title, systemImage: systemImage, selected: selected, bold: bold, iconColor: iconColor) { EmptyView() }
    }
}

struct TextChannelRow: View {
    @Environment(AppModel.self) private var model
    var channel: Channel
    var selected = false

    var body: some View {
        let unread = model.client.isUnread(channel)
        let mentions = model.client.mentionCounts[channel.id] ?? 0
        SidebarLabel(title: channel.name, systemImage: "text.bubble.fill", selected: selected, bold: unread || mentions > 0) {
            if mentions > 0 {
                Text("(@\(mentions))").font(Theme.chromeBold).foregroundStyle(selected ? Theme.selectionText : Theme.ownName)
                    .accessibilityLabel(mentions == 1 ? "1 mention" : "\(mentions) mentions")
            } else if unread {
                Text("new").font(Theme.small).foregroundStyle(selected ? Theme.selectionText : Theme.grayText)
                    .accessibilityLabel("Unread")
            }
        }
    }
}

/// Your Personal space: only you see it (notes, links and pictures between
/// your own devices).
struct PersonalChannelRow: View {
    @Environment(AppModel.self) private var model
    var channel: Channel
    var selected = false

    var body: some View {
        let unread = model.client.isUnread(channel)
        SidebarLabel(title: "Personal space", systemImage: "lock.fill", selected: selected, bold: unread) {
            if unread {
                Text("new").font(Theme.small).foregroundStyle(selected ? Theme.selectionText : Theme.grayText)
                    .accessibilityLabel("Unread")
            }
        }
        .help("Only you can see this. For moving things between your devices — not for passwords or private info.")
        .accessibilityHint("Only you can see it")
    }
}

struct VoiceChannelRow: View {
    @Environment(AppModel.self) private var model
    var channel: Channel
    var archive: () -> Void

    var body: some View {
        let joined = model.voice.wantedChannelId == channel.id
        Button {
            if joined {
                model.showingVoicePanel = true
            } else {
                Task { await model.voice.join(channel.id) }
            }
        } label: {
            SidebarLabel(title: channel.name, systemImage: joined ? "speaker.wave.3.fill" : "speaker.wave.2.fill", bold: joined,
                         iconColor: joined ? Theme.ok : Theme.text) {
                if joined {
                    Text(model.voice.isResuming ? "reconnecting…" : model.voice.isConnected ? "connected" : "connecting…")
                        .font(Theme.small)
                        .foregroundStyle(model.voice.isConnected ? Theme.ok : Theme.grayText)
                } else {
                    Text("Join").font(Theme.small).underline().foregroundStyle(Theme.link)
                }
            }
        }
        .buttonStyle(.plain)
        .contextMenu {
            if joined {
                Button("Leave Voice", systemImage: "phone.down") { model.voice.leave() }
            }
            Button("Archive channel…", systemImage: "archivebox", role: .destructive, action: archive)
        }
        .accessibilityHint(joined ? "Opens the call" : "Joins the voice channel")
    }
}

/// Someone in a voice channel, indented under it (an OBS link listens
/// without a row: `voiceRoster`).
struct VoiceMemberRow: View {
    @Environment(AppModel.self) private var model
    var session: Session

    var body: some View {
        let user = model.client.user(session.uid)
        let speaking = model.voice.engine.speaking.contains(session.sid)
        let inMyCall = model.voice.channelId != nil && session.voice?.channelId == model.voice.channelId
        let hint = session.sid == model.voice.ownSid ? nil : model.voice.hint(for: session.sid)
        let connecting = inMyCall && hint == nil && session.sid != model.voice.ownSid && model.voice.engine.peerStates[session.sid] != .connected
        return HStack(spacing: 5) {
            Avatar(user: user, size: 14, speaking: speaking)
            Text(user?.name ?? "Someone")
                .font(speaking ? Theme.chromeBold : Theme.chrome)
                .foregroundStyle(connecting ? Theme.grayText : Theme.text)
                .lineLimit(1)
            if speaking {
                Image(systemName: "speaker.wave.2.fill").font(.system(size: 9)).foregroundStyle(Theme.ok)
            }
            if session.voice?.screen == true {
                LiveBadge98(session: session)
            }
            if let hint {
                VoiceHint98(hint: hint).layoutPriority(-1)
            }
            Spacer()
            if session.voice?.deafened == true {
                Image(systemName: "speaker.slash.fill").foregroundStyle(Theme.danger).font(.system(size: 10))
            } else if session.voice?.muted == true {
                Image(systemName: "mic.slash.fill").foregroundStyle(Theme.danger).font(.system(size: 10))
            }
        }
        .padding(.leading, 28)
        .padding(.trailing, 6)
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .voiceHintHelp(hint, name: user?.name ?? "They")
        .liveBadgeAction(session)
        .voicePersonMenu(session)
    }
}

/// "LIVE": someone in voice is sharing their screen. Red, white, square;
/// opens their screen in the app (yours: stops your share).
struct LiveBadge98: View {
    @Environment(AppModel.self) private var model
    var session: Session

    var body: some View {
        let label = ScreenText.live(name: model.client.user(session.uid)?.name ?? "their", mine: session.sid == model.client.mySid)
        Button { model.watchScreen(session) } label: {
            Text("LIVE")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(.white)
                .padding(.horizontal, 3)
                .frame(height: 13)
                .background(Color(red: 0.75, green: 0, blue: 0))
        }
        .buttonStyle(.plain)
        .help(label)
        .accessibilityLabel(label)
    }
}

/// "v0.39.0 · what's new" (the server's version) and "Privacy", pinned under the channel list on a
/// strip with an etched line above (like the web's sidebar footer).
struct SidebarFooter98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 0) {
            Separator98()
            HStack(spacing: 10) {
                if let endpoint = model.client.endpoint {
                    FooterLink98(title: model.whatsNewTitle, url: endpoint.changelogURL)
                        .help("What's new on this server")
                    FooterLink98(title: "Privacy", url: endpoint.privacyURL)
                }
                Spacer(minLength: 0)
            }
            .font(Theme.small)
            .foregroundStyle(Theme.hint)
            .padding(.horizontal, 6)
            .padding(.top, 3)
            .padding(.bottom, 4)
        }
        .background(Theme.face)
    }
}

/// A quiet grey link that turns blue and underlined on hover.
private struct FooterLink98: View {
    var title: String
    var url: URL
    @State private var hovering = false

    var body: some View {
        Link(destination: url) {
            Text(title)
                .underline(hovering)
                .foregroundStyle(hovering ? Theme.link : Theme.hint)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

/// Someone in the Watch together audience, indented under Watch.
struct WatchMemberRow98: View {
    @Environment(AppModel.self) private var model
    var session: Session

    var body: some View {
        let user = model.client.user(session.uid)
        HStack(spacing: 5) {
            Avatar(user: user, size: 14)
            Text(user?.name ?? "Someone").font(Theme.chrome).foregroundStyle(Theme.text).lineLimit(1)
            Spacer()
        }
        .padding(.leading, 28)
        .padding(.trailing, 6)
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(user?.name ?? "Someone"), watching")
    }
}

struct PlaceRow: View {
    @Environment(AppModel.self) private var model
    var title: String
    var systemImage: String
    var viewers: [Session]
    var selected = false

    var body: some View {
        SidebarLabel(title: title, systemImage: systemImage, selected: selected) {
            if !viewers.isEmpty {
                Text("\(viewers.count) here").font(Theme.small)
            }
        }
    }
}

/// A quiet "New …" row at the end of a list (no toolbar + buttons).
struct NewRow: View {
    var title: String
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            SidebarLabel(title: title, systemImage: "plus", iconColor: Theme.grayText)
                .foregroundStyle(Theme.grayText)
        }
        .buttonStyle(.plain)
        .modifier(PlainRow())
    }
}

struct MinecraftRow: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let status = model.minecraft
        let address = status?.address ?? model.config?.minecraft?.address ?? ""
        Button {
            copy(address)
            model.client.notice = YapClient.Notice(text: "Copied \(address)")
        } label: {
            HStack(spacing: 6) {
                Rectangle()
                    .fill(status == nil ? Theme.grayText : status!.online ? Theme.ok : Theme.danger)
                    .frame(width: 8, height: 8)
                    .overlay(Bevel(style: .status))
                    .frame(width: 18)
                VStack(alignment: .leading, spacing: 0) {
                    Text(address).font(Theme.mono).foregroundStyle(Theme.text)
                    Text(status?.summary ?? "Checking…").font(Theme.small).foregroundStyle(Theme.grayText)
                    if let names = status?.players?.names, !names.isEmpty {
                        Text("On now: \(names.joined(separator: ", "))").font(Theme.small).foregroundStyle(Theme.grayText)
                    }
                }
                Spacer()
                Image(systemName: "doc.on.doc").font(.system(size: 10)).foregroundStyle(Theme.grayText)
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityHint("Copies the server address")
    }

    private func copy(_ text: String) {
        #if os(iOS)
        UIPasteboard.general.string = text
        #else
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        #endif
    }
}

struct NewChannelSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var kind: ChannelKind
    @State private var name = ""
    @FocusState private var focused: Bool

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 10) {
                Text("Name:").font(Theme.chrome).foregroundStyle(Theme.text)
                Field98 {
                    TextField("", text: $name, prompt: Text(kind == .voice ? "Hangout" : "movie night").foregroundStyle(Theme.grayText))
                        .focused($focused)
                        .onSubmit(create)
                }
                Text(kind == .voice ? "Anyone can hop in. Voice goes peer to peer, as high-quality Opus." : "Everyone sees it. Archive it later from its menu; messages are kept.")
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.grayText)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer()
            }
            .padding(14)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(Theme.face)
            .win98Sheet(kind == .voice ? "New voice channel" : "New channel") {
                CloseSheetButton(hasChanges: !name.isEmpty) { dismiss() }
            } done: {
                DoneButton(action: create)
                    .disabled(name.isBlank)
            }
            .onAppear { focused = true }
        }
        .presentationDetents([.medium])
        .interactiveDismissDisabled(!name.isEmpty)
        #if os(macOS)
        .frame(minWidth: 360, minHeight: 200)
        #endif
    }

    private func create() {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        model.client.createChannel(name: trimmed, kind: kind)
        dismiss()
    }
}

/// Add a Place for everyone: a name and an https address.
struct NewPlaceSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var address = ""
    @State private var error: String?

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 10) {
                Text("Name:").font(Theme.chrome).foregroundStyle(Theme.text)
                Field98 {
                    TextField("", text: $name, prompt: Text("Movie list").foregroundStyle(Theme.grayText))
                }
                Text("Address:").font(Theme.chrome).foregroundStyle(Theme.text)
                Field98 {
                    TextField("", text: $address, prompt: Text("https://…").foregroundStyle(Theme.grayText))
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        .keyboardType(.URL)
                        #endif
                        .onSubmit(add)
                }
                if let error { ErrorLine(text: error) }
                Text("Everyone sees it under Places. Links must start with https://.")
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.grayText)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer()
            }
            .padding(14)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(Theme.face)
            .win98Sheet("New place") {
                CloseSheetButton(hasChanges: !name.isEmpty || !address.isEmpty) { dismiss() }
            } done: {
                DoneButton(action: add)
                    .disabled(name.isBlank || address.isBlank)
            }
        }
        .presentationDetents([.medium])
        .interactiveDismissDisabled(!name.isEmpty || !address.isEmpty)
        #if os(macOS)
        .frame(minWidth: 380, minHeight: 260)
        #endif
    }

    private func add() {
        if model.client.addLink(name: name, url: address) {
            dismiss()
        } else {
            error = "That doesn't look like an https:// address."
        }
    }
}
