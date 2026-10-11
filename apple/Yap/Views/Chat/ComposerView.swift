import SwiftUI
import YapCore

/// The compose box, AIM-style: emoji, GIF and picture buttons, a white
/// sunken field, and the default "Send" button. Typing `:sho` suggests emoji
/// (custom first) and `@na` suggests people; custom `:name:` and standard
/// `:shortcodes:` are converted when sending. While editing a message, a
/// yellow bar says so ("Cancel" or Esc puts the draft back) and Send saves;
/// while replying, "Replying to Name" with an ×.
struct ComposerView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dynamicTypeSize) private var typeSize
    /// The emoji / picture glyphs grow with the text.
    @ScaledMetric(relativeTo: .footnote) private var glyph: CGFloat = 13
    var channel: Channel
    /// The channel screen: draft, edit, reply, staged picture, Send / Esc / ↑.
    @Bindable var chat: ChannelController

    var body: some View {
        let locked = channel.isPersonal && model.client.vaultLocked
        VStack(alignment: .leading, spacing: 4) {
            if let staged = chat.staged {
                StagedTray98(staged: staged) { chat.dropStaged() }
            }
            if let replyingTo = chat.replyingTo, chat.editing == nil {
                HStack(spacing: 6) {
                    Image(systemName: "arrowshape.turn.up.left.fill").font(.system(size: 10, weight: .bold))
                    Text("Replying to \(Text(model.client.name(of: replyingTo.uid)).bold())").font(Theme.chrome)
                    let quoted = MessageContent(replyingTo, client: model.client).snippet
                    Text(quoted.isEmpty ? "" : "— \(quoted)")
                        .font(Theme.small)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    Button { chat.dropReply() } label: {
                        Image(systemName: "xmark").font(.system(size: 10, weight: .black))
                    }
                    .buttonStyle(.win98Compact)
                    .accessibilityLabel("Don't reply")
                }
                .foregroundStyle(Theme.tooltipText)
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(Theme.tooltip)
                .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
            }
            if chat.editing != nil {
                HStack(spacing: 6) {
                    Image(systemName: "pencil").font(.system(size: 11, weight: .bold))
                    Text("Editing your message").font(Theme.chrome)
                    Spacer()
                    Button("Cancel") { chat.cancelEdit() }
                        .buttonStyle(.win98Compact)
                }
                .foregroundStyle(Theme.tooltipText)
                .padding(.horizontal, 6)
                .padding(.vertical, 3)
                .background(Theme.tooltip)
                .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
            }
            Suggestions(draft: $chat.draft)
            if typeSize.isAccessibilitySize {
                // Big type: the field gets its own line, the buttons go under it.
                VStack(alignment: .leading, spacing: 4) {
                    field(locked: locked)
                    HStack(alignment: .center, spacing: 4) {
                        tools
                        Spacer(minLength: 4)
                        sendButton
                    }
                }
            } else {
                HStack(alignment: .center, spacing: 4) {
                    tools
                    field(locked: locked)
                    sendButton
                }
            }
        }
        .padding(.horizontal, 2)
    }

    /// Emoji, GIF, picture (and Paste in the Personal space).
    @ViewBuilder private var tools: some View {
        Button { chat.picker = .emoji } label: {
            Image(systemName: "face.smiling").font(.system(size: glyph, weight: .semibold))
        }
        .buttonStyle(.win98Compact)
        .accessibilityLabel("Emoji")

        Button { chat.picker = .gif } label: {
            Text("GIF").font(Theme.chromeBold)
        }
        .buttonStyle(.win98Compact)
        .disabled(model.config?.gifs == nil)
        .accessibilityLabel("GIF")

        PicturePickerButton(onPick: chat.stagePicture) {
            Image(systemName: "photo").font(.system(size: glyph - 1, weight: .semibold))
        }
        .buttonStyle(.win98Compact)
        .disabled(chat.editing != nil)
        .accessibilityLabel("Send a picture")

        #if os(iOS)
        if channel.isPersonal {
            PasteToNote(onPicture: chat.stagePicture, onText: { chat.draft += $0 })
                .labelStyle(.iconOnly)
                .buttonBorderShape(.roundedRectangle(radius: 0))
                .tint(Theme.shadow)
                .disabled(chat.editing != nil)
        }
        #endif
    }

    private func field(locked: Bool) -> some View {
        Field98 {
            TextField("", text: $chat.draft, prompt: Text(locked ? PersonalSpace.lockedPlaceholder : channel.isPersonal ? PersonalSpace.placeholder : "Message \(channel.name)").foregroundStyle(Theme.grayText), axis: .vertical)
                .lineLimit(1...6)
                .onChange(of: chat.draft) { _, text in model.client.userTyped(text, in: channel.id) }
                .onKeyPress(.return, phases: .down) { chat.returnPressed($0) }
                .onKeyPress(.escape) { chat.escape() }
                #if os(macOS)
                .onKeyPress(.upArrow) { chat.editLast() }
                #endif
        }
    }

    private var sendButton: some View {
        Button(chat.editing == nil ? "Send" : "Save", action: chat.send)
            #if os(macOS)
            .buttonStyle(.win98Default)
            #else
            .buttonStyle(Win98ButtonStyle(isDefault: true, compact: true))
            #endif
            .disabled(!chat.canSend)
    }

}

/// The tray above the compose box: what's about to go out, with × to drop it.
struct StagedTray98: View {
    var staged: StagedAttachment
    var drop: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            StagedThumbnail(staged: staged)
                .frame(width: 44, height: 44)
                .clipped()
                .padding(2)
                .background(Theme.window)
                .overlay(Bevel(style: .sunken))
            VStack(alignment: .leading, spacing: 1) {
                Text(staged.title).font(Theme.chromeBold).foregroundStyle(Theme.text)
                Text("\(staged.subtitle) · Enter to send").font(Theme.small).foregroundStyle(Theme.grayText).lineLimit(1)
                if let draft = staged.picture {
                    Check98(title: "Full res", isOn: Binding(get: { draft.full }, set: { draft.full = $0 }))
                        .help(PictureDraft.fullResHelp)
                }
            }
            Spacer(minLength: 4)
            Button(action: drop) {
                Image(systemName: "xmark").font(.system(size: 11, weight: .black)).frame(minWidth: 16)
            }
            .buttonStyle(.win98Compact)
            .accessibilityLabel("Don't send this")
            .help("Don't send this (Esc)")
        }
        .padding(4)
        .bevel(.raised)
    }
}

/// Inline suggestions above the field for the word being typed.
struct Suggestions: View {
    @Environment(AppModel.self) private var model
    @Binding var draft: String

    typealias Item = SuggestionEngine.Item

    var body: some View {
        let (items, start) = SuggestionEngine.suggestions(for: draft, client: model.client)
        if !items.isEmpty, let start {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 3) {
                    ForEach(items) { item in
                        Button { SuggestionEngine.accept(item, from: start, in: &draft) } label: { chip(item) }
                            .buttonStyle(.win98Compact)
                    }
                }
                .padding(2)
            }
            .background(Theme.tooltip)
            .overlay(Rectangle().stroke(Color.black, lineWidth: 1))
        }
    }

    @ViewBuilder private func chip(_ item: Item) -> some View {
        HStack(spacing: 4) {
            switch item {
            case .custom(let emoji):
                ReactionGlyph(emoji: "<:\(emoji.name):\(emoji.id)>", size: 14)
                Text(":\(emoji.name):")
            case .standard(let emoji):
                Text(emoji.char(tone: EmojiToneSetting.current))
                Text(":\(emoji.primaryCode):")
            case .person(let user):
                Text("@\(user.name)").bold()
            case let .command(command, help):
                Text(command).bold()
                Text(help).foregroundStyle(Theme.grayText)
            }
        }
        .font(Theme.chrome)
    }

}
