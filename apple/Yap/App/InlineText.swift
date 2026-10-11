import SwiftUI
import YapCore

/// How a skin dresses the runs of a message's line (`MessageFormatter`'s
/// inlines): code, links, @mentions, spoilers, and a custom emoji whose
/// picture isn't in yet.
struct InlineTextStyle {
    var code: (inout AttributedString) -> Void
    var link: (inout AttributedString) -> Void
    var mention: (inout AttributedString) -> Void
    /// A run inside `||…||` (hidden or revealed: the skin knows which).
    var spoiler: (inout AttributedString) -> Void
    var missingEmoji: (inout AttributedString) -> Void = { _ in }
    /// Custom emoji: their size and how far below the baseline they sit.
    var emojiSide: CGFloat
    var emojiBaseline: CGFloat
}

/// One line of a message as a `Text`, the same in every skin: formatted
/// runs, links that open, mentions, spoilers, and custom emoji drawn inline
/// as pictures (or `:name:` until they load).
enum InlineText {
    @MainActor
    static func build(_ inlines: [Inline], style: InlineTextStyle, emoji: (_ id: String) -> PlatformImage?,
                      suffix: AttributedString? = nil) -> Text {
        var result = Text(verbatim: "")
        var run = AttributedString()
        func flush() {
            if !run.characters.isEmpty {
                let piece = Text(run)
                result = Text("\(result)\(piece)")
                run = AttributedString()
            }
        }
        for inline in inlines {
            switch inline {
            case let .text(string, inlineStyle):
                var a = AttributedString(string)
                a.inlinePresentationIntent = intent(inlineStyle)
                if inlineStyle.contains(.spoiler) { style.spoiler(&a) }
                run += a
            case .code(let code):
                var a = AttributedString(code)
                style.code(&a)
                run += a
            case let .link(url, label, inlineStyle):
                var a = AttributedString(label)
                a.link = url
                style.link(&a)
                a.inlinePresentationIntent = intent(inlineStyle)
                if inlineStyle.contains(.spoiler) { style.spoiler(&a) }
                run += a
            case .mention(let name):
                var a = AttributedString("@\(name)")
                style.mention(&a)
                run += a
            case let .emoji(name, id):
                if let image = emoji(id) {
                    flush()
                    let piece = Text(Image(platformImage: image)).baselineOffset(style.emojiBaseline)
                    result = Text("\(result)\(piece)")
                } else {
                    var a = AttributedString(":\(name):")
                    style.missingEmoji(&a)
                    run += a
                }
            }
        }
        if let suffix { run += suffix }
        flush()
        return result
    }

    /// Bold / italic / strikethrough as presentation intents.
    static func intent(_ style: InlineStyle) -> InlinePresentationIntent? {
        var intent: InlinePresentationIntent = []
        if style.contains(.bold) { intent.insert(.stronglyEmphasized) }
        if style.contains(.italic) { intent.insert(.emphasized) }
        if style.contains(.strike) { intent.insert(.strikethrough) }
        return intent.isEmpty ? nil : intent
    }
}
