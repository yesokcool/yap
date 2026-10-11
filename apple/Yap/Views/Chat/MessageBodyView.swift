import SwiftUI
import YapCore

/// A message body from `MessageFormatter`: bold/italic/strike, code, links,
/// @mentions, quotes, code blocks, and custom emoji inline as images (18 pt,
/// or 40 pt when the message is only 1–8 emoji). `prefix` (AIM's
/// "Name: ") runs inline into the first paragraph; `suffix` (" (edited)")
/// ends the last one.
struct MessageBodyView: View {
    @Environment(AppModel.self) private var model
    /// The raw message body (with `<:name:id>` tokens).
    var source: String
    var prefix: Text? = nil
    var suffix: Text? = nil
    /// `||Spoilers||` show as solid bars until the message is clicked.
    @State private var revealed = false

    var body: some View {
        let jumbo = EmojiCodec.isJumbo(source)
        let hidden = !revealed && Spoilers.contains(source)
        let blocks = source.isEmpty ? [] : MessageFormatter.parse(source, names: model.client.memberNames, hideSpoilers: hidden)
        let inlinePrefix: Bool = {
            if jumbo { return false }
            if case .paragraph? = blocks.first { return true }
            return false
        }()
        let inlineSuffix: Bool = {
            if jumbo { return false }
            if case .paragraph? = blocks.last { return true }
            return false
        }()
        VStack(alignment: .leading, spacing: 4) {
            if let prefix, !inlinePrefix {
                if blocks.isEmpty, let suffix {
                    Text("\(prefix)\(suffix)").font(Theme.chat)
                } else {
                    prefix.font(Theme.chat)
                }
            }
            ForEach(Array(blocks.enumerated()), id: \.offset) { index, block in
                switch block {
                case .paragraph(let inlines):
                    let body = text(inlines, jumbo: jumbo)
                    let ending: Text = index == blocks.count - 1 && inlineSuffix && suffix != nil ? Text("\(body)\(suffix!)") : body
                    Group {
                        if index == 0, inlinePrefix, let prefix {
                            Text("\(prefix)\(ending)")
                        } else {
                            ending
                        }
                    }
                    .font(jumbo ? .system(size: 40) : Theme.chat)
                    .foregroundStyle(Theme.text)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                case .quote(let inlines):
                    HStack(spacing: 6) {
                        Theme.grayText.frame(width: 2)
                        text(inlines, jumbo: false)
                            .font(Theme.chat)
                            .foregroundStyle(Theme.grayText)
                            .textSelection(.enabled)
                    }
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.leading, 12)
                case .codeBlock(let code):
                    ScrollView(.horizontal, showsIndicators: false) {
                        Text(code)
                            .font(Theme.mono)
                            .foregroundStyle(Theme.text)
                            .textSelection(.enabled)
                            .padding(6)
                    }
                    .background(Theme.codeBackground)
                    .overlay(Bevel(style: .sunken))
                    .padding(.leading, 12)
                }
            }
            if let suffix, !inlineSuffix, !blocks.isEmpty {
                suffix
            }
        }
        .spoilerReveal(hidden) { revealed = true }
    }

    private func text(_ inlines: [Inline], jumbo: Bool) -> Text {
        let style = InlineTextStyle(
            code: { a in
                a.font = Theme.mono
                a.backgroundColor = Theme.codeBackground
            },
            link: { a in
                a.foregroundColor = Theme.link
                a.underlineStyle = .single
            },
            mention: { a in
                a.font = Theme.chatBold
                a.foregroundColor = Theme.selectionText
                a.backgroundColor = Theme.selection
            },
            spoiler: spoiler,
            missingEmoji: { a in a.foregroundColor = Theme.grayText },
            emojiSide: jumbo ? 40 : 18, emojiBaseline: jumbo ? 0 : -4)
        return InlineText.build(inlines, style: style, emoji: { emojiImage($0, side: style.emojiSide) })
    }

    private func emojiImage(_ id: String, side: CGFloat) -> PlatformImage? {
        guard let endpoint = model.client.endpoint else { return nil }
        return model.images.inlineImage(for: endpoint.emojiURL(id), side: side)
    }

    /// Hidden: a solid gray bar (like the web's, a redacted line). Revealed: the code-well tint.
    private func spoiler(_ a: inout AttributedString) {
        if a.link == nil, !revealed {
            a.foregroundColor = Theme.grayText
            a.backgroundColor = Theme.grayText
        } else {
            a.backgroundColor = Theme.codeBackground
        }
    }

}
