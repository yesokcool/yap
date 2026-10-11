import Foundation
import Testing
@testable import Yap
@testable import YapCore

/// The composer's suggestions (`SuggestionEngine`).
@MainActor
@Suite("Composer suggestions")
struct SuggestionEngineTests {
    func client() -> YapClient {
        let client = YapClient(credentials: MemoryCredentialStore(), client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s", uid: "me", channels: [],
                                            users: [User(id: "me", name: "Me", color: "#fff"), User(id: "ada", name: "Ada", color: "#fff"),
                                                    User(id: "adam", name: "Adam", color: "#fff")],
                                            sessions: []), histories: [:])
        return client
    }

    @Test func peopleCommandsAndAccepting() {
        let client = client()
        var draft = "hi @ad"
        let (people, start) = SuggestionEngine.suggestions(for: draft, client: client)
        #expect(people.map(\.id) == ["p:ada", "p:adam"])
        SuggestionEngine.accept(people[0], from: start!, in: &draft)
        #expect(draft == "hi @Ada ")

        let (commands, _) = SuggestionEngine.suggestions(for: "/po", client: client)
        #expect(commands.map(\.id) == ["cmd:/poll"])
        #expect(SuggestionEngine.suggestions(for: "plain words", client: client).0.isEmpty)
        #expect(SuggestionEngine.suggestions(for: "@me", client: client).0.isEmpty, "never yourself")
    }
}
