import SwiftUI
import UniformTypeIdentifiers
import YapCore
import YapVoice

// The Windows 98 skin's windows for the extras: Run… (the quick
// switcher), Catch Up, the media shelf, New Poll, Plan a Hang, and adding
// a sound. Each is a Win98 window (navy title bar, grey face, sunken wells).

/// The skin root's sheets for the extras (both platforms).
struct ExtrasSheets98: ViewModifier {
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        @Bindable var model = model
        content
            .sheet(isPresented: $model.showingSwitcher) { Switcher98() }
            .sheet(isPresented: $model.showingCatchUp) { CatchUp98() }
            .sheet(isPresented: $model.planningHang) { PlanHang98() }
            .sheet(item: $model.doodleTarget) { message in DoodleWindow98(message: message) }
    }
}

// MARK: - Run… (quick switcher)

/// Ctrl/⌘K: a Run-style box. Type to filter; Enter opens the top hit, ↑/↓
/// move, Esc closes; the last row searches messages.
struct Switcher98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    #if os(macOS)
    @Environment(\.openSettings) private var openSettings
    #endif
    @State private var query = ""
    @State private var selected = 0
    @FocusState private var focused: Bool

    var body: some View {
        let items = Switcher.items(model, query: query)
        NavigationStack {
            VStack(alignment: .leading, spacing: 6) {
                Text("Type the name of a channel, place or person, and Yap will open it for you.")
                    .font(Theme.chrome)
                    .foregroundStyle(Theme.text)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 6) {
                    Text("Open:").font(Theme.chrome).foregroundStyle(Theme.text)
                    Field98 {
                        TextField("", text: $query)
                            .focused($focused)
                            .autocorrectionDisabled()
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            .submitLabel(.go)
                            #endif
                            .onSubmit { run(items) }
                            .onKeyPress(.downArrow) { selected = min(items.count - 1, selected + 1); return .handled }
                            .onKeyPress(.upArrow) { selected = max(0, selected - 1); return .handled }
                            .onKeyPress(.escape) { dismiss(); return .handled }
                    }
                }
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                                Button { perform(item) } label: {
                                    SidebarLabel(title: item.title, systemImage: item.systemImage, selected: index == selected) {
                                        if let subtitle = item.subtitle {
                                            Text(subtitle).font(Theme.small).lineLimit(1)
                                        }
                                    }
                                }
                                .buttonStyle(.plain)
                                .id(item.id)
                            }
                        }
                        .padding(2)
                    }
                    .well()
                    .onChange(of: selected) { _, index in if items.indices.contains(index) { proxy.scrollTo(items[index].id) } }
                }
                HStack {
                    Spacer()
                    Button("OK") { run(items) }.buttonStyle(.win98Default).disabled(items.isEmpty)
                    Button("Cancel") { dismiss() }.buttonStyle(.win98)
                }
            }
            .padding(10)
            .background(Theme.face)
            .win98Sheet("Jump to") {
                CloseSheetButton { dismiss() }
            }
        }
        .onChange(of: query) { selected = 0 }
        .onAppear {
            focused = true
            if let text = DebugLaunch.current.switcherText, query.isEmpty { query = text }
        }
        .presentationDetents([.medium, .large])
        #if os(macOS)
        .frame(width: 440, height: 380)
        #endif
    }

    private func run(_ items: [SwitcherItem]) {
        guard items.indices.contains(selected) else { return }
        perform(items[selected])
    }

    private func perform(_ item: SwitcherItem) {
        dismiss()
        #if os(macOS)
        model.perform(item.action, openURL: openURL, openSettings: { openSettings() })
        #else
        model.perform(item.action, openURL: openURL, openSettings: {})
        #endif
    }
}

// MARK: - Catch up

/// The unread channels, how many new, and the first new line. Click one to
/// open it at the "New since" line. Channels that mention you come first;
/// Mark all read clears it all.
struct CatchUp98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let client = model.client
        NavigationStack {
            VStack(alignment: .leading, spacing: 6) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        if client.unreadChannels.isEmpty {
                            Text("You're all caught up.").font(Theme.chrome).foregroundStyle(Theme.grayText).padding(8)
                        }
                        ForEach(client.catchUpChannels) { channel in
                            let mentions = client.mentionsForMe(in: channel.id)
                            Button { model.catchUp(channel) } label: {
                                VStack(alignment: .leading, spacing: 2) {
                                    HStack {
                                        Text(channel.isPersonal ? "Personal space" : channel.name).font(Theme.chatBold).foregroundStyle(Theme.text)
                                        if mentions > 0 {
                                            Text("(@\(mentions)) for you").font(Theme.chromeBold).foregroundStyle(Theme.ownName)
                                                .accessibilityLabel("\(mentions) for you")
                                        }
                                        Spacer()
                                        Text(client.unreadCounts?[channel.id].map { "\($0.label) new" } ?? (client.unreadCounts == nil ? "…" : ""))
                                            .font(Theme.chromeBold)
                                            .foregroundStyle(Theme.ownName)
                                    }
                                    if let first = client.unreadCounts?[channel.id]?.first {
                                        Text("\(Text(client.name(of: first.uid)).bold()): \(MessageFormatter.flatten(first.body))")
                                            .font(Theme.chrome)
                                            .foregroundStyle(Theme.grayText)
                                            .lineLimit(2)
                                    } else if channel.isPersonal {
                                        Text("Notes from your other devices").font(Theme.chrome).foregroundStyle(Theme.grayText)
                                    }
                                }
                                .padding(.horizontal, 6)
                                .padding(.vertical, 5)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(.rect)
                            }
                            .buttonStyle(.plain)
                            Separator98()
                        }
                    }
                }
                .well()
                if !client.unreadChannels.isEmpty {
                    HStack {
                        Spacer()
                        Button("Mark all read") {
                            client.markAllRead()
                            dismiss()
                        }
                        .buttonStyle(.win98)
                    }
                }
            }
            .padding(8)
            .background(Theme.face)
            .win98Sheet("Catch up") {
                CloseSheetButton { dismiss() }
            }
            .onAppear { client.requestUnreadCounts() }
        }
        .presentationDetents([.medium, .large])
        #if os(macOS)
        .frame(minWidth: 420, minHeight: 340)
        #endif
    }
}

// MARK: - Media shelf

/// Pictures and GIFs in a channel, as a grid of sunken tiles; click one to
/// jump to its message.
struct Shelf98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var channelId: String

    var body: some View {
        let shelf = model.client.shelves[channelId]
        NavigationStack {
            ScrollView {
                if let shelf, shelf.items.isEmpty, !shelf.loading {
                    Text("No pictures or GIFs here yet.").font(Theme.chrome).foregroundStyle(Theme.grayText).padding(12)
                }
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 96), spacing: 6)], spacing: 6) {
                    ForEach(shelf?.items ?? []) { message in
                        Button {
                            dismiss()
                            model.jump(to: message)
                        } label: {
                            ShelfTile98(message: message)
                        }
                        .buttonStyle(.plain)
                        .onAppear { if message.id == shelf?.items.last?.id { model.client.loadMoreShelf(channelId) } }
                    }
                }
                .padding(6)
                if shelf?.loading != false { ProgressView().padding() }
            }
            .well()
            .padding(6)
            .background(Theme.face)
            .win98Sheet("Pictures & GIFs in \(channelLabel(channelId, client: model.client))") {
                CloseSheetButton { dismiss() }
            }
            .onAppear { model.client.loadShelf(channelId) }
        }
        .presentationDetents([.medium, .large])
        #if os(macOS)
        .frame(minWidth: 500, minHeight: 420)
        #endif
    }
}

struct ShelfTile98: View {
    @Environment(AppModel.self) private var model
    var message: Message

    var body: some View {
        Theme.window
            .aspectRatio(1, contentMode: .fit)
            .overlay {
                if let image = message.image, let endpoint = model.client.endpoint {
                    if let picture = model.images.picture(for: endpoint.mediaURL(image.id)) {
                        Image(platformImage: picture).resizable().scaledToFill()
                    } else {
                        ProgressView()
                    }
                } else if let gif = message.gif {
                    AsyncImage(url: gif.stillURL ?? gif.imageURL) { $0.resizable().scaledToFill() } placeholder: { Color.clear }
                        .overlay(alignment: .bottomLeading) { Tooltip98(text: "GIF").padding(3) }
                }
            }
            .clipped()
            .padding(2)
            .overlay(Bevel(style: .sunken))
            .accessibilityLabel(message.gif != nil ? "GIF from \(model.client.name(of: message.uid))" : "Picture from \(model.client.name(of: message.uid))")
    }
}

// MARK: - New poll

struct Poll98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var channelId: String
    @State private var question = ""
    @State private var options = ["", ""]

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Question:").font(Theme.chrome).foregroundStyle(Theme.text)
                    Field98 { TextField("", text: $question) }
                    Text("Answers:").font(Theme.chrome).foregroundStyle(Theme.text)
                    ForEach(options.indices, id: \.self) { index in
                        HStack(spacing: 6) {
                            Text("\(index + 1).").font(Theme.chrome).foregroundStyle(Theme.text).frame(width: 20, alignment: .trailing)
                            Field98 { TextField("", text: $options[index]) }
                            if options.count > 2 {
                                Button("Remove") { options.remove(at: index) }.buttonStyle(.win98Compact)
                            }
                        }
                    }
                    if options.count < Poll.maxOptions {
                        Button("Add answer") { options.append("") }.buttonStyle(.win98)
                    }
                    Text("Everyone can vote once and change their mind. Tip: type /poll Question | A | B in any message.")
                        .font(Theme.small)
                        .foregroundStyle(Theme.grayText)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(12)
            }
            .background(Theme.face)
            .win98Sheet("New poll") {
                CloseSheetButton(hasChanges: hasChanges) { dismiss() }
            } done: {
                DoneButton { if model.client.sendPoll(question: question, options: options, in: channelId) { dismiss() } }
                    .disabled(Poll.problem(question: question, options: options) != nil)
            }
        }
        .interactiveDismissDisabled(hasChanges)
        #if os(macOS)
        .frame(minWidth: 400, minHeight: 400)
        #endif
    }

    /// A question or any answer typed: closing asks first.
    private var hasChanges: Bool { !question.isBlank || options.contains { !$0.isBlank } }
}

// MARK: - Hangs

/// Plan a hang: title, when (on the half hour), and maybe a voice channel.
struct PlanHang98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var title = ""
    @State private var date = Hang.firstSlot()
    @State private var channelId: String?

    var body: some View {
        let client = model.client
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    GroupBox98(title: "New hang") {
                        Text("What:").font(Theme.chrome).foregroundStyle(Theme.text)
                        Field98 { TextField("", text: $title, prompt: Text("Game night")) }
                        Text("When:").font(Theme.chrome).foregroundStyle(Theme.text)
                        DateField98(selection: $date, range: Hang.firstSlot()...Date().addingTimeInterval(Hang.latest), components: [.date, .hourAndMinute])
                        Text("Starts at \(Hang.snap(date).formatted(date: .abbreviated, time: .shortened)) (on the half hour).")
                            .font(Theme.small)
                            .foregroundStyle(Theme.grayText)
                        Text("Where:").font(Theme.chrome).foregroundStyle(Theme.text)
                        VStack(alignment: .leading, spacing: 0) {
                            Radio98(title: "Anywhere", isOn: channelId == nil) { channelId = nil }
                            ForEach(client.voiceChannels) { channel in
                                Radio98(title: channel.name, isOn: channelId == channel.id) { channelId = channel.id }
                            }
                        }
                    }
                    Text("When it's time, everyone online gets a notice, and the people who marked it free in When's good get one push.")
                        .font(Theme.small)
                        .foregroundStyle(Theme.grayText)
                        .fixedSize(horizontal: false, vertical: true)
                    if !client.upcomingHangs().isEmpty {
                        GroupBox98(title: "Coming up") {
                            ForEach(client.upcomingHangs()) { hang in HangRow98(hang: hang) }
                        }
                    }
                }
                .padding(12)
            }
            .background(Theme.face)
            .win98Sheet("Plan a hang") {
                CloseSheetButton(hasChanges: !title.isEmpty) { dismiss() }
            } done: {
                DoneButton {
                    client.planHang(at: date, title: title, channelId: channelId)
                    dismiss()
                }
                .disabled(title.isBlank || client.upcomingHangs().count >= Hang.maxUpcoming)
            }
        }
        .interactiveDismissDisabled(!title.isEmpty)
        #if os(macOS)
        .frame(minWidth: 380, minHeight: 420)
        #endif
    }
}

/// "🗓 Game night — Tonight 8:30 PM, Lounge"; the maker (or a mod) cancels it.
struct HangRow98: View {
    @Environment(AppModel.self) private var model
    var hang: Hang
    @State private var cancelling = false

    var body: some View {
        let client = model.client
        SidebarLabel(title: hang.title, systemImage: "calendar", iconColor: hang.isOn() ? Theme.ok : Theme.text) {
            Text([HangFormat.when(hang), hang.channelId.flatMap { client.channelsById[$0]?.name }].compactMap { $0 }.joined(separator: ", "))
                .font(Theme.small)
                .foregroundStyle(Theme.grayText)
                .lineLimit(1)
        }
        .help("Planned by \(client.name(of: hang.uid))")
        .contextMenu {
            if hang.uid == client.uid || client.isMod {
                Button("Cancel hang…", systemImage: "xmark", role: .destructive) { cancelling = true }
            }
        }
        .question(isPresented: $cancelling, .cancelHang(hang.title) { [hang] in client.cancelHang(hang) })
    }
}

/// A hang that's starting: a raised bar with Join voice.
struct HangNotice98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let client = model.client
        if let hang = client.hangStarting {
            HStack(spacing: 6) {
                Text("🗓")
                VStack(alignment: .leading, spacing: 0) {
                    Text("\(hang.title) is starting").font(Theme.chromeBold).foregroundStyle(Theme.text).lineLimit(1)
                    if let place = hang.channelId.flatMap({ client.channelsById[$0]?.name }) {
                        Text(place).font(Theme.small).foregroundStyle(Theme.grayText)
                    }
                }
                Spacer(minLength: 4)
                if let channel = hang.channelId, model.voice.wantedChannelId != channel {
                    Button("Join Voice") {
                        Task { await model.voice.join(channel) }
                        client.hangStarting = nil
                    }
                    .buttonStyle(.win98Default)
                }
                Button { client.hangStarting = nil } label: { Image(systemName: "xmark").font(.system(size: 9, weight: .black)) }
                    .buttonStyle(.win98Compact)
                    .accessibilityLabel("Dismiss")
            }
            .padding(6)
            .bevel(.raised)
        }
    }
}

// MARK: - Adding a sound

/// Pick a clip, trim it (a sunken waveform), name it, upload.
struct SoundUpload98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var upload: SoundUpload
    @State private var importing = false

    init(kind: SoundUpload.Kind) {
        _upload = State(initialValue: SoundUpload(kind: kind))
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    Text(upload.kind == .chime
                         ? "Pick a sound file (a voice memo, an m4a, mp3 or wav). Your chime can be up to 2 seconds."
                         : "Pick a sound file (a voice memo, an m4a, mp3 or wav). Soundboard clips can be up to 5 seconds.")
                        .font(Theme.chrome)
                        .foregroundStyle(Theme.text)
                        .fixedSize(horizontal: false, vertical: true)
                    Button(upload.clip == nil ? "Browse…" : "Browse for Another…") { importing = true }
                        .buttonStyle(.win98)
                    if let clip = upload.clip {
                        GroupBox98(title: "Trim") {
                            ClipOutline98(clip: clip, start: upload.start, length: upload.length)
                                .frame(height: 40)
                            HStack {
                                Text("Start: \(String(format: "%.2f", upload.start)) s").font(Theme.chrome).foregroundStyle(Theme.text)
                                Spacer()
                                Text("Length: \(String(format: "%.1f", upload.length)) s").font(Theme.chrome).foregroundStyle(Theme.text)
                            }
                            Slider(value: $upload.start, in: 0...max(0.01, upload.latestStart))
                            Button("▶ Preview") { upload.preview(on: model.sounds) }.buttonStyle(.win98)
                        }
                        if upload.kind == .board {
                            Text("Name:").font(Theme.chrome).foregroundStyle(Theme.text)
                            Field98 { TextField("", text: $upload.name) }
                                .onChange(of: upload.name) { _, new in
                                    if new.count > Soundboard.maxNameLength { upload.name = String(new.prefix(Soundboard.maxNameLength)) }
                                }
                            Text("Emoji (optional):").font(Theme.chrome).foregroundStyle(Theme.text)
                            Field98 { TextField("", text: $upload.emoji) }
                                .frame(width: 80)
                                .onChange(of: upload.emoji) { _, new in if new.count > 1 { upload.emoji = String(new.suffix(1)) } }
                        }
                    }
                    if let error = upload.error { ErrorLine(text: error) }
                }
                .padding(12)
            }
            .background(Theme.face)
            .win98Sheet(upload.kind == .chime ? "Your join chime" : "Add a sound") {
                CloseSheetButton(hasChanges: upload.clip != nil) { dismiss() }
            } done: {
                DoneButton { Task { if await upload.upload(model.client) { dismiss() } } }
                    .disabled(!upload.canUpload)
            }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.audio]) { result in
                if case .success(let url) = result { upload.load(url) }
            }
        }
        .interactiveDismissDisabled(upload.clip != nil)
        #if os(macOS)
        .frame(minWidth: 420, minHeight: 400)
        #endif
    }
}

/// The clip's loudness in a sunken well; the kept part in navy.
struct ClipOutline98: View {
    var clip: SoundClip
    var start: Double
    var length: Double

    var body: some View {
        GeometryReader { geo in
            let bars = clip.outline(max(1, Int(geo.size.width / 3)))
            HStack(alignment: .center, spacing: 1) {
                ForEach(Array(bars.enumerated()), id: \.offset) { index, level in
                    let t = Double(index) / Double(max(1, bars.count)) * clip.seconds
                    Rectangle()
                        .fill(t >= start && t <= start + length ? Theme.selection : Theme.grayText)
                        .frame(width: 2, height: max(1, (geo.size.height - 6) * CGFloat(level)))
                }
            }
            .frame(width: geo.size.width, height: geo.size.height)
        }
        .background(Theme.window)
        .overlay(Bevel(style: .sunken))
        .accessibilityHidden(true)
    }
}
