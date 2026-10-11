import SwiftUI
import YapCore

/// "Search", Win98-style: a search box, This channel / Everywhere,
/// and the results (newest first) in a sunken list. Click one to jump to it.
/// It searches as you type (after 600 ms: the server allows a burst, then
/// one per 2 s), on Find / Return, and again after a reconnect if it was
/// typed while offline.
struct FindWindow98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var channelId: String
    /// ⇧⌘F or Jump to's "Search messages for …": everywhere, with these words (maybe none yet).
    var initialText: String? = nil
    @State private var query = ""
    @State private var scope: SearchScope = .channel
    /// What the last search asked (nil: it didn't go out, we were offline).
    @State private var searched: String?
    @FocusState private var focused: Bool

    var body: some View {
        let client = model.client
        NavigationStack {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 6) {
                    Field98 {
                        TextField("", text: $query, prompt: Text("Find… (from:Name, has:image, has:link)").foregroundStyle(Theme.grayText))
                            .autocorrectionDisabled()
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif
                            .focused($focused)
                            .onSubmit(find)
                    }
                    Button("Find", action: find)
                        .buttonStyle(.win98Default)
                        .disabled(query.isBlank)
                }
                HStack(spacing: 12) {
                    ForEach(SearchScope.allCases, id: \.self) { option in
                        Radio98(title: option == .channel ? "In \(channelLabel(channelId, client: client))" : "Everywhere", isOn: scope == option) {
                            scope = option
                            if !query.isEmpty { find() }
                        }
                    }
                }
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        if let search = client.search {
                            if search.messages.isEmpty {
                                Text(search.loading ? "Searching…" : "Nothing found.")
                                    .font(Theme.chrome)
                                    .foregroundStyle(Theme.grayText)
                                    .padding(8)
                            }
                            ForEach(search.messages) { message in
                                SearchResultRow98(message: message, query: search.q) {
                                    dismiss()
                                    model.jump(to: message)
                                }
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .well()
                StatusPanel(text: statusText)
            }
            .padding(6)
            .background(Theme.face)
            .win98Sheet("Search") {
                CloseSheetButton { dismiss() }
            }
            .onAppear {
                focused = true
                client.clearSearch()
                if let initialText {
                    query = initialText
                    scope = .everywhere
                    if !initialText.isBlank { find() }
                } else if let demo = DebugLaunch.current.search {
                    query = demo
                    scope = .everywhere
                    Task {
                        try? await Task.sleep(for: .seconds(1))
                        find()
                    }
                }
            }
        }
        .task(id: query) {
            // Search as you type, gently; a new keystroke cancels the wait.
            guard !query.isBlank else { return }
            try? await Task.sleep(for: .milliseconds(600))
            if !Task.isCancelled, query != searched { find() }
        }
        .onChange(of: model.client.phase) { _, phase in
            // Typed while offline: it didn't go out; ask again now.
            if phase == .online, !query.isBlank, searched != query { find() }
        }
        .presentationDetents([.large])
        #if os(macOS)
        .frame(minWidth: 520, minHeight: 460)
        #endif
    }

    private var statusText: String {
        guard let search = model.client.search else { return "Type a word, then Find." }
        if search.loading { return "Searching…" }
        let n = search.messages.count
        return n == 1 ? "1 message" : "\(n) messages\(n >= 50 ? " (newest 50)" : "")"
    }

    private func find() {
        guard !query.isBlank else { return }
        model.client.runSearch(query, in: scope == .channel ? channelId : nil)
        searched = model.client.isConnected ? query : nil
    }
}

/// One result: `#channel · Name · time`, then the excerpt with matches in
/// the selection color.
struct SearchResultRow98: View {
    @Environment(AppModel.self) private var model
    var message: Message
    var query: String
    var open: () -> Void
    @State private var hovering = false

    var body: some View {
        let client = model.client
        Button(action: open) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 4) {
                    Text(channelLabel(message.channelId, client: client)).font(Theme.chromeBold)
                    Text("·")
                    Text(client.name(of: message.uid)).font(Theme.chromeBold)
                        .foregroundStyle(message.uid == client.uid ? Theme.ownName : Theme.otherName)
                    Text("· \(Stamps.short(message.date))").font(Theme.small).foregroundStyle(Theme.grayText)
                    if message.pinnedAt != nil { Text("📌").font(Theme.small) }
                }
                .font(Theme.chrome)
                .foregroundStyle(Theme.text)
                Text(searchSnippet(message, client: client, query: query) { text, range in
                    text[range].backgroundColor = Theme.selection
                    text[range].foregroundColor = Theme.selectionText
                })
                .font(Theme.chat)
                .foregroundStyle(Theme.text)
                .lineLimit(3)
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(hovering ? Theme.selection.opacity(0.12) : .clear)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .accessibilityHint("Jumps to the message")
    }
}

/// A channel's pinned messages (most recently pinned first); click one to
/// jump to it, or unpin it.
struct PinsWindow98: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var channelId: String

    var body: some View {
        let client = model.client
        NavigationStack {
            VStack(spacing: 4) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        if let pins = client.pins[channelId] {
                            if pins.isEmpty {
                                Text("Nothing pinned yet. Pin a message from its menu.")
                                    .font(Theme.chrome)
                                    .foregroundStyle(Theme.grayText)
                                    .padding(8)
                            }
                            ForEach(pins) { message in
                                HStack(alignment: .top, spacing: 4) {
                                    SearchResultRow98(message: message, query: "") {
                                        dismiss()
                                        model.jump(to: message)
                                    }
                                    Button("Unpin") { client.pin(message, on: false) }
                                        .buttonStyle(.win98Compact)
                                        .padding(.top, 4)
                                        .padding(.trailing, 4)
                                }
                            }
                        } else {
                            ProgressView().padding(8)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .well()
            }
            .padding(6)
            .background(Theme.face)
            .win98Sheet("Pinned in \(channelLabel(channelId, client: client))") {
                CloseSheetButton { dismiss() }
            }
            .onAppear { client.loadPins(channelId) }
        }
        .presentationDetents([.medium, .large])
        #if os(macOS)
        .frame(minWidth: 480, minHeight: 380)
        #endif
    }
}
