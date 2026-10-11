import SwiftUI
import YapCore

/// What the emoji picker hands back.
enum EmojiPick {
    case standard(StandardEmoji)
    case custom(Emoji)

    /// As a reaction: the character (in your skin tone), or the `<:name:id>` token.
    var reactionValue: String {
        switch self {
        case .standard(let e): e.char(tone: EmojiToneSetting.current)
        case .custom(let e): "<:\(e.name):\(e.id)>"
        }
    }
}

/// Your skin tone for standard emoji (0 = default yellow, 1–5 light → dark),
/// per device like the web's.
enum EmojiToneSetting {
    static var current: Int { min(5, max(0, UserDefaults.standard.integer(forKey: EmojiTone.storageKey))) }
}

/// Recently used emoji, per device (the web keeps the same list in localStorage).
enum RecentEmoji {
    static let key = "yap.recentEmoji"
    static func all() -> [String] { UserDefaults.standard.stringArray(forKey: key) ?? [] }
    static func remember(_ key: String) {
        let keys = [key] + all().filter { $0 != key }
        UserDefaults.standard.set(Array(keys.prefix(27)), forKey: self.key)
    }
}

/// What the emoji pickers list: recents, the server's custom emoji
/// A–Z, and search across custom and standard emoji.
enum EmojiBrowser {
    @MainActor
    static func recents(_ client: YapClient) -> [EmojiPick] {
        let catalog = EmojiCatalog.shared
        return RecentEmoji.all().compactMap { key in
            if key.hasPrefix("u:") { return catalog.byChar[String(key.dropFirst(2))].map(EmojiPick.standard) }
            if key.hasPrefix("c:") { return client.emojis[String(key.dropFirst(2))].map(EmojiPick.custom) }
            return nil
        }
    }

    @MainActor
    static func custom(_ client: YapClient) -> [EmojiPick] {
        client.emojis.values.sorted { $0.name.lowercased() < $1.name.lowercased() }.map(EmojiPick.custom)
    }

    /// Custom emoji whose name contains the words, then standard ones.
    @MainActor
    static func results(for query: String, client: YapClient) -> [EmojiPick] {
        let q = query.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: ": "))
        let custom = client.emojis.values.filter { $0.name.lowercased().contains(q) }.map(EmojiPick.custom)
        return custom + EmojiCatalog.shared.search(q).map(EmojiPick.standard)
    }

    /// A pick goes to the front of Recently used.
    static func remember(_ pick: EmojiPick) {
        switch pick {
        case .standard(let e): RecentEmoji.remember("u:\(e.char)")
        case .custom(let e): RecentEmoji.remember("c:\(e.id)")
        }
    }
}

/// Standard emoji (the bundled emoji-data.json, shown in the system font)
/// plus the server's custom ones, with search and recents.
struct EmojiPickerView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    var title = "Emoji"
    var onPick: (EmojiPick) -> Void
    @State private var query = ""
    @AppStorage(EmojiTone.storageKey) private var tone = 0

    private let columns = [GridItem(.adaptive(minimum: 40), spacing: 4)]
    private let catalog = EmojiCatalog.shared

    var body: some View {
        NavigationStack {
            VStack(spacing: 6) {
                Field98 {
                    TextField("", text: $query, prompt: Text("Find an emoji").foregroundStyle(Theme.grayText))
                        .autocorrectionDisabled()
                }
                HStack(spacing: 3) {
                    Text("Skin tone:").font(Theme.chrome).foregroundStyle(Theme.text)
                    ForEach(Array(EmojiTone.hands.enumerated()), id: \.offset) { index, hand in
                        Button { tone = index } label: { Text(hand).font(.system(size: 15)) }
                            .buttonStyle(Win98ToggleButtonStyle(isOn: tone == index, compact: true))
                            .accessibilityLabel(index == 0 ? "Default skin tone" : "Skin tone \(index)")
                            .accessibilityAddTraits(tone == index ? .isSelected : [])
                    }
                    Spacer(minLength: 0)
                }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14, pinnedViews: [.sectionHeaders]) {
                    if query.isEmpty {
                        let recents = EmojiBrowser.recents(model.client)
                        if !recents.isEmpty { section("Recently used", recents) }
                        let custom = EmojiBrowser.custom(model.client)
                        if custom.isEmpty {
                            Section {
                                Text("No custom emoji yet — upload them from the web app.")
                                    .font(Theme.chrome).foregroundStyle(Theme.grayText).padding(.horizontal, 10)
                            } header: { header("Custom") }
                        } else {
                            section("Custom", custom)
                        }
                        ForEach(Array(catalog.groups.enumerated()), id: \.offset) { index, name in
                            section(name, catalog.emojis(inGroup: index).map(EmojiPick.standard))
                        }
                    } else {
                        let results = EmojiBrowser.results(for: query, client: model.client)
                        if results.isEmpty {
                            Text("No emoji matches “\(query)”.").font(Theme.chrome).foregroundStyle(Theme.grayText).padding(10)
                        } else {
                            section("Results", results)
                        }
                    }
                }
                .padding(.bottom, 8)
            }
            .well()
            }
            .padding(6)
            .background(Theme.face)
            .win98Sheet(title) {
                CloseSheetButton { dismiss() }
            }
        }
        .presentationDetents([.medium, .large])
        #if os(macOS)
        .frame(minWidth: 420, minHeight: 480)
        #endif
    }

    private func section(_ title: String, _ picks: [EmojiPick]) -> some View {
        Section {
            LazyVGrid(columns: columns, spacing: 4) {
                ForEach(Array(picks.enumerated()), id: \.offset) { _, pick in
                    Button {
                        EmojiBrowser.remember(pick)
                        onPick(pick)
                        dismiss()
                    } label: {
                        tile(pick)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 6)
        } header: {
            header(title)
        }
    }

    private func header(_ title: String) -> some View {
        Text(title)
            .font(Theme.chromeBold)
            .foregroundStyle(Theme.text)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.face)
            .overlay(alignment: .bottom) { Theme.shadow.frame(height: 1) }
    }

    @ViewBuilder private func tile(_ pick: EmojiPick) -> some View {
        Group {
            switch pick {
            case .standard(let e):
                Text(e.char(tone: tone)).font(.system(size: 28))
                    .accessibilityLabel(":\(e.primaryCode):")
            case .custom(let e):
                ReactionGlyph(emoji: pick.reactionValue, size: 30)
                    .accessibilityLabel(":\(e.name):")
            }
        }
        .frame(width: 40, height: 40)
        .contentShape(.rect)
    }

}
