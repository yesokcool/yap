import Foundation
import Testing
@testable import YapCore

@Suite("Emoji + formatting")
struct FormattingTests {
    let party = Emoji(id: "0123456789abcdef0123456789abcdef", name: "Party")
    var custom: [String: Emoji] { ["party": party] }

    @Test func standardCatalogLoads() {
        let catalog = EmojiCatalog.shared
        #expect(catalog.all.count > 1500)
        #expect(catalog.groups.first == "Smileys")
        #expect(catalog.byCode["thumbsup"] != nil)
        #expect(catalog.byCode["joy"]?.char == "😂")
        #expect(catalog.search("pizza").contains { $0.char == "🍕" })
        #expect(catalog.suggestions(for: "thumbsu").first?.codes.contains("thumbsup") == true)
    }

    @Test func encodeCustomAndStandardShortcodes() {
        #expect(EmojiCodec.encode("yay :party: and :PARTY:", custom: custom) == "yay <:Party:0123456789abcdef0123456789abcdef> and <:Party:0123456789abcdef0123456789abcdef>")
        #expect(EmojiCodec.encode("lol :joy: `:joy:`", custom: custom) == "lol 😂 `:joy:`", "never inside code")
        #expect(EmojiCodec.encode("```\n:joy:\n``` :joy:", custom: custom) == "```\n:joy:\n``` 😂")
        #expect(EmojiCodec.encode(":party::party:", custom: custom).components(separatedBy: "<:Party:").count == 3)
        #expect(EmojiCodec.encode(":nope_not_real:", custom: custom) == ":nope_not_real:")
        // Already-encoded tokens are left alone.
        let token = "<:Party:0123456789abcdef0123456789abcdef>"
        #expect(EmojiCodec.encode(token, custom: custom) == token)
        // Custom names win over standard ones.
        let joy = Emoji(id: "ffffffffffffffffffffffffffffffff", name: "joy")
        #expect(EmojiCodec.encode(":joy:", custom: ["joy": joy]) == "<:joy:ffffffffffffffffffffffffffffffff>")
    }

    @Test func decodeTokens() {
        let body = "hi <:party:0123456789abcdef0123456789abcdef> there <:x:ab>"
        #expect(EmojiCodec.tokens(in: body) == [EmojiCodec.Token(name: "party", id: "0123456789abcdef0123456789abcdef")])
        #expect(EmojiCodec.plain(body) == "hi :party: there <:x:ab>")
    }

    @Test func jumbo() {
        #expect(EmojiCodec.isJumbo("😂"))
        #expect(EmojiCodec.isJumbo("👍🏽 🎉"))
        #expect(EmojiCodec.isJumbo("<:party:0123456789abcdef0123456789abcdef><:party:0123456789abcdef0123456789abcdef>"))
        #expect(EmojiCodec.isJumbo("👨‍👩‍👧"))
        #expect(!EmojiCodec.isJumbo("lol 😂"))
        #expect(!EmojiCodec.isJumbo("123"))
        #expect(!EmojiCodec.isJumbo(""))
        #expect(!EmojiCodec.isJumbo(String(repeating: "😂", count: 9)))
    }

    @Test func formatterBasics() {
        let blocks = MessageFormatter.parse("some **bold**, *it* _also_ ~~gone~~ `code` and https://example.com/x?a=1&b=2.")
        guard case .paragraph(let inlines)? = blocks.first else { Issue.record("no paragraph"); return }
        #expect(inlines.contains(.text("bold", .bold)))
        #expect(inlines.contains(.text("it", .italic)))
        #expect(inlines.contains(.text("also", .italic)))
        #expect(inlines.contains(.text("gone", .strike)))
        #expect(inlines.contains(.code("code")))
        #expect(inlines.contains(.link(URL(string: "https://example.com/x?a=1&b=2")!, "https://example.com/x?a=1&b=2", [])))
        if case .text(let last, _)? = inlines.last { #expect(last == ".") }
    }

    @Test func formatterCodeIsLiteral() {
        let blocks = MessageFormatter.parse("look:\n```swift\nlet **x** = 1\n```\nafter")
        #expect(blocks == [
            .paragraph([.text("look:", [])]),
            .codeBlock("let **x** = 1"),
            .paragraph([.text("after", [])]),
        ])
        #expect(MessageFormatter.parse("`**not bold**`") == [.paragraph([.code("**not bold**")])])
    }

    @Test func formatterEmojiQuotesAndNewlines() {
        let blocks = MessageFormatter.parse("> quoted <:party:0123456789abcdef0123456789abcdef>\nreply\nline two")
        #expect(blocks == [
            .quote([.text("quoted ", []), .emoji(name: "party", id: "0123456789abcdef0123456789abcdef")]),
            .paragraph([.text("reply\nline two", [])]),
        ])
        // Bold across a link keeps the link bold.
        #expect(MessageFormatter.parse("**see https://a.b**") == [.paragraph([.text("see ", .bold), .link(URL(string: "https://a.b")!, "https://a.b", .bold)])])
        // snake_case_words aren't italic.
        #expect(MessageFormatter.parse("snake_case_word") == [.paragraph([.text("snake_case_word", [])])])
        // Private-use characters from a sender can't fake markers.
        #expect(MessageFormatter.parse("a\u{E010}b") == [.paragraph([.text("ab", [])])])
    }

    @Test func messageRowsGroupAndDivide() {
        let channel = Channel(id: "general", name: "general", kind: .text, position: 0)
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let day1 = 1_759_600_000_000 // 2025-10-04 17:46 UTC
        let messages = [
            Message(id: "1", channelId: "general", uid: "a", body: "hi", ts: day1),
            Message(id: "2", channelId: "general", uid: "a", body: "again", ts: day1 + 60_000),
            Message(id: "3", channelId: "general", uid: "b", body: "yo", ts: day1 + 120_000),
            Message(id: "4", channelId: "general", uid: "b", body: "later", ts: day1 + 120_000 + 8 * 60_000),
            Message(id: "5", channelId: "general", uid: "b", body: "tomorrow", ts: day1 + 86_400_000),
        ]
        let rows = MessageRows.build(channel: channel, messages: messages, pending: [], hasMore: false, loading: false,
                                     unreadAfter: day1 + 60_000, myUid: "a", calendar: calendar)
        let ids = rows.map(\.id)
        #expect(ids == ["start:general", "day:2025-10-4", "msg:1", "msg:2", "unread", "msg:3", "msg:4", "day:2025-10-5", "msg:5"])
        func grouped(_ id: String) -> Bool? {
            for row in rows { if case let .message(m, g) = row, m.id == id { return g } }
            return nil
        }
        #expect(grouped("1") == false)
        #expect(grouped("2") == true)
        #expect(grouped("3") == false, "after the unread line")
        #expect(grouped("4") == false, "more than 7 minutes later")
        #expect(grouped("5") == false, "new day")
    }

    /// One blank check everywhere: a newline-only hang title is as empty as spaces.
    @Test func blankText() {
        #expect("".isBlank && "   ".isBlank && "\n\n".isBlank && " \t\n ".isBlank)
        #expect(!"a".isBlank && !"  hi  ".isBlank && !"\n🎉".isBlank)
    }

    @Test func mentionsHighlight() {
        let blocks = MessageFormatter.parse("@sam lee and @Sam, @everyone — mail@sam.dev https://x.dev/@sam", names: ["Sam", "Sam Lee"])
        guard case .paragraph(let inlines)? = blocks.first else { Issue.record("no paragraph"); return }
        let mentions = inlines.compactMap { if case .mention(let n) = $0 { n } else { nil } }
        #expect(mentions == ["sam lee", "Sam", "everyone"], "longest name wins; emails and URLs aren't mentions")
    }

    @Test func reactionsApply() {
        var m = Message(id: "1", channelId: "c", uid: "a", body: "x", ts: 1)
        m.applyReaction(emoji: "🎉", uid: "a", on: true)
        m.applyReaction(emoji: "🎉", uid: "b", on: true)
        m.applyReaction(emoji: "🎉", uid: "b", on: true)
        m.applyReaction(emoji: "<:party:0123456789abcdef0123456789abcdef>", uid: "a", on: true)
        #expect(m.reactions == [Reaction(emoji: "🎉", uids: ["a", "b"]), Reaction(emoji: "<:party:0123456789abcdef0123456789abcdef>", uids: ["a"])])
        m.applyReaction(emoji: "🎉", uid: "a", on: false)
        m.applyReaction(emoji: "🎉", uid: "b", on: false)
        #expect(m.reactions.map(\.emoji) == ["<:party:0123456789abcdef0123456789abcdef>"])
    }
}
