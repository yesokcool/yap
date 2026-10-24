import Foundation
import Testing
@testable import YapCore

/// Signing out — here, or from another device (the token stops working) —
/// leaves nothing of the old session behind for whoever signs in next.
@MainActor
@Suite("Ending a session")
struct SessionTests {
    final class FakeVoice: VoiceCoordinator {
        var signedOut = 0
        func clientDidBecomeReady() {}
        func clientConnectionDropped() {}
        func clientDidSignOut() { signedOut += 1 }
        func clientVoiceChanged(_ change: VoiceChange) {}
        func clientSessionLeft(sid: String) {}
        func clientReceivedSignal(_ data: SignalData, from sid: String) {}
    }

    func signedIn(_ store: MemoryCredentialStore, voice: FakeVoice) -> YapClient {
        let client = YapClient(credentials: store, client: "test")
        client.voice = voice
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me",
                                            channels: [Channel(id: "general", name: "general", kind: .text, position: 0)],
                                            users: [User(id: "me", name: "Me", color: "#fff"), User(id: "ada", name: "Ada", color: "#fff")],
                                            sessions: []),
                        histories: ["general": []])
        client.receiveForTesting(try! ServerMessage.decode(#"{"t":"account","hasPin":true,"kind":"password"}"#))
        store.setVaultKey(StoredVaultKey(key: Data(repeating: 1, count: 32), ct: "c"), for: "me")
        // A mention while looking elsewhere, a pins list asked for.
        client.receiveForTesting(.message(Message(id: "m1", channelId: "general", uid: "ada", body: "@Me hi", ts: 5), nonce: nil))
        client.loadPins("general")
        return client
    }

    func expectClean(_ client: YapClient, _ store: MemoryCredentialStore, _ voice: FakeVoice) {
        #expect(voice.signedOut == 1)
        #expect(store.token == nil)
        #expect(store.vaultKey(for: "me") == nil)
        #expect(client.uid.isEmpty)
        #expect(client.mentionCounts.isEmpty)
        #expect(client.secretKind == nil)
        #expect(client.channelsById.isEmpty && client.users.isEmpty && client.histories.isEmpty)
        #expect(client.pins.isEmpty && client.pinChange == .idle)
    }

    @Test func signOutHere() {
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t")
        let voice = FakeVoice()
        let client = signedIn(store, voice: voice)
        #expect(client.mentionCounts["general"] == 1 && client.secretKind == "password")
        client.signOut()
        expectClean(client, store, voice)
        #expect(client.phase == .signedOut && client.authError == nil)
    }

    /// Revoked from another device: the reconnect's hello comes back
    /// `bad_token`. Same clean slate, plus a friendly reason on the sign-in screen.
    @Test func signedOutElsewhere() {
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t")
        let voice = FakeVoice()
        let client = signedIn(store, voice: voice)
        client.receiveForTesting(.error(code: "bad_token", message: "Unknown device.", re: "hello"))
        expectClean(client, store, voice)
        #expect(client.phase == .signedOut)
        #expect(client.authError?.code == "bad_token" && client.authError?.mode == .login)
    }

    /// Closed with 4001 (signed out from another device, or by a new
    /// password — mine or a mod's reset): the same clean slate at once,
    /// without a reconnect to hear `bad_token`; the app wipes its web view.
    @Test func signedOutByTheServer() {
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t")
        let voice = FakeVoice()
        let client = signedIn(store, voice: voice)
        var wiped = 0
        client.onSignedOut = { wiped += 1 }
        client.connectionClosed(.signedOut)
        expectClean(client, store, voice)
        #expect(client.phase == .signedOut && wiped == 1)
        #expect(client.authError?.code == "bad_token" && client.authError?.mode == .login)
    }

    /// Every way a sign-in ends tells the app (it signs out its web view too).
    @Test func everyEndingTellsTheApp() {
        let server = ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k")
        var wiped = 0
        let here = signedIn(MemoryCredentialStore(endpoint: server, token: "t"), voice: FakeVoice())
        here.onSignedOut = { wiped += 1 }
        here.signOut()
        #expect(wiped == 1)
        let elsewhere = signedIn(MemoryCredentialStore(endpoint: server, token: "t"), voice: FakeVoice())
        elsewhere.onSignedOut = { wiped += 1 }
        elsewhere.receiveForTesting(.error(code: "bad_token", message: "", re: "hello"))
        #expect(wiped == 2)
        let forgetting = signedIn(MemoryCredentialStore(endpoint: server, token: "t"), voice: FakeVoice())
        forgetting.onSignedOut = { wiped += 1 }
        forgetting.forgetServer()
        #expect(wiped == 3 && forgetting.phase == .needsInvite)
        // Another server (a new key for the same one keeps the sign-in: `newKeySameServerKeepsTheDevice`).
        let moving = YapClient(credentials: MemoryCredentialStore(endpoint: server, token: "t"), client: "test")
        moving.onSignedOut = { wiped += 1 }
        moving.useInvite(ServerEndpoint(baseURL: URL(string: "https://other.example.com")!, key: "k"))
        #expect(wiped == 4)
    }

    /// A wrong password on the sign-in screen isn't a session ending.
    @Test func badLoginKeepsNothingToClear() {
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"))
        let voice = FakeVoice()
        let client = YapClient(credentials: store, client: "test")
        client.voice = voice
        client.logIn(name: "Me", pin: "correct-horse-582039")
        client.receiveForTesting(.error(code: "bad_login", message: "", re: "hello"))
        #expect(voice.signedOut == 0)
        #expect(client.authError?.code == "bad_login")
    }

    /// A rotated invite key for the same server keeps this device signed in.
    @Test func newKeySameServerKeepsTheDevice() {
        let server = URL(string: "https://chat.example.com")!
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: server, key: "old"), token: "t")
        let client = YapClient(credentials: store, client: "test")
        client.useInvite(ServerEndpoint(baseURL: server, key: "new"))
        #expect(store.token == "t" && store.endpoint?.key == "new")
        #expect(client.phase == .connecting)
        client.useInvite(ServerEndpoint(baseURL: URL(string: "https://other.example.com")!, key: "k"))
        #expect(store.token == nil && client.phase == .signedOut)
    }

    /// A deleted message leaves pins, search results and the media shelf too.
    @Test func deletedLeavesEveryList() {
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t")
        let voice = FakeVoice()
        let client = signedIn(store, voice: voice)
        let m = Message(id: "m9", channelId: "general", uid: "ada", body: "pinned", ts: 9)
        client.receiveForTesting(.pins(channelId: "general", messages: [m]))
        client.runSearch("pinned", in: nil)
        client.receiveForTesting(.searchResults(q: "pinned", channelId: nil, messages: [m]))
        client.receiveForTesting(.shelf(channelId: "general", items: [m], hasMore: false))
        #expect(client.pins["general"]?.count == 1 && client.search?.messages.count == 1 && client.shelves["general"]?.items.count == 1)
        client.receiveForTesting(.deleted(id: "m9", channelId: "general"))
        #expect(client.pins["general"]?.isEmpty == true)
        #expect(client.search?.messages.isEmpty == true)
        #expect(client.shelves["general"]?.items.isEmpty == true)
    }
}
