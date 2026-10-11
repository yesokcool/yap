import Foundation
import Testing
@testable import YapCore

/// The server's mention rule (web/js/mentions.js), with the same examples
/// as test/unit.test.mjs: who gets pinged is exactly who the message highlights.
@Suite("Mentions")
struct MentionsTests {
    static let people = [User(id: "sam", name: "Sam", color: "#fff"), User(id: "samlee", name: "Sam Lee", color: "#fff"),
                         User(id: "ben", name: "Ben", color: "#fff"), User(id: "jo", name: "Jo", color: "#fff")]

    /// [message, who it mentions]
    static let cases: [(String, [String])] = [
        ("@Sam Lee hi", ["samlee"]), ("@sam lee and @SAM", ["samlee", "sam"]), ("@Sam Lee, @Sam Lee!", ["samlee"]), ("@Sam Leeds", ["sam"]),
        ("hi @ben!", ["ben"]), ("(@Ben)", ["ben"]), ("@ben\n@jo", ["ben", "jo"]), ("@Ben@Jo", ["ben"]),
        ("mail sam@ben.com", []), ("x@ben", []), ("@@ben", []), ("@bento", []), ("@ben_x", []), ("@ben2", []),
        ("`@ben` in code", []), ("```\n@ben\n```", []), ("https://youtube.com/@Ben", []), ("see https://x.com/a then @jo", ["jo"]), ("no one", []),
        // Custom emojis are covered up like code: one right against the @ makes it no mention (on both sides).
        ("<:wave:\(wave)>@ben", []), ("@ben<:wave:\(wave)>", []), ("<:wave:\(wave)> @ben", ["ben"]), ("@ben <:wave:\(wave)>", ["ben"]),
        ("`x`@ben", []), ("@ben`x`", []), ("`x` @ben", ["ben"]), ("☀️@ben", ["ben"]),
    ]
    static let wave = String(repeating: "a", count: 32)

    /// Who the formatter draws as a mention.
    static func highlighted(_ body: String) -> [String] {
        var names: [String] = []
        func walk(_ inlines: [Inline]) {
            for case .mention(let name) in inlines { names.append(name) }
        }
        for block in MessageFormatter.parse(body, names: people.map(\.name)) {
            switch block {
            case .paragraph(let inlines), .quote(let inlines): walk(inlines)
            case .codeBlock: break
            }
        }
        return Array(Set(names.compactMap { name in people.first { $0.name.lowercased() == name.lowercased() }?.id }))
    }

    @Test(arguments: cases)
    func pingedIsHighlighted(_ body: String, _ want: [String]) {
        #expect(Mentions.find(in: body, among: Self.people).uids.sorted() == want.sorted(), "\(body)")
        #expect(Self.highlighted(body).sorted() == want.sorted(), "highlight: \(body)")
    }

    @Test(arguments: [("@everyone game time", true), ("@here?", true), ("me@everyone.com", false), ("`@everyone`", false), ("@everyones", false)])
    func everyone(_ body: String, _ want: Bool) {
        #expect(Mentions.find(in: body, among: Self.people).everyone == want, "\(body)")
    }

    /// "@Sam Lee" pings Sam Lee, not Sam; code and links ping nobody.
    @MainActor
    @Test func mentionsMeFollowsTheRule() {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t"), client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "sam", channels: [Channel(id: "general", name: "general", kind: .text, position: 0)],
                                            users: Self.people, sessions: []),
                        histories: [:])
        func pings(_ body: String) -> Bool { client.mentionsMe(Message(id: "m", channelId: "general", uid: "ben", body: body, ts: 1)) }
        #expect(pings("@sam you there?"))
        #expect(pings("@here game time"))
        #expect(!pings("@Sam Lee you there?"))
        #expect(!pings("`@sam`"))
        #expect(!pings("https://youtube.com/@sam"))
        #expect(!pings("mail sam@sam.dev"))
    }
}
