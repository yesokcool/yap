import Foundation
import Testing
@testable import Yap
@testable import YapCore

/// The owner's Reset main link (PROTOCOL.md › Resetting the main link), in
/// the spec's words, and who gets it.
@MainActor
@Suite("Reset main link")
struct MainLinkAppTests {
    func client(role: Role) -> YapClient {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t"),
                               client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me", channels: [], users: [User(id: "me", name: "Sam", color: "#fff", role: role)], sessions: []),
                        histories: [:])
        return client
    }

    @Test func resetQuestion() {
        let question = Question.resetMainLink {}
        #expect(question.title == "Reset main link?")
        #expect(question.text == "The old link stops working for anyone who hasn't joined yet. Everyone already signed in keeps working, and their apps get the new link by themselves. Personal invite links keep working.")
        #expect(question.action == "Reset link" && question.cancel == "Cancel")
        #expect(question.danger, "destructive: Cancel is the default button")
    }

    @Test func onlyTheOwnerResets() {
        #expect(MainInvite.canReset(client(role: .owner)))
        #expect(!MainInvite.canReset(client(role: .mod)), "mods see and copy the main link, but don't reset it")
        #expect(!MainInvite.canReset(client(role: .member)))
        #expect(MainInvite.reset == "Reset main link…")
    }
}
