import Foundation
import Testing
@testable import YapCore

/// Resetting the main link (PROTOCOL.md › Resetting the main link): the
/// owner's `invite.resetMain`, everyone's `key`, `ready.key` on a device that
/// still holds the old link, and what a wrong key looks like now (4003).
@MainActor
@Suite("Resetting the main link")
struct MainLinkTests {
    static let server = URL(string: "https://chat.example.com")!

    func signedIn(role: Role = .owner, key: String = "old") -> (YapClient, MemoryCredentialStore) {
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: Self.server, key: key), token: "t")
        let client = YapClient(credentials: store, client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me",
                                            channels: [Channel(id: "general", name: "general", kind: .text, position: 0)],
                                            users: [User(id: "me", name: "Sam", color: "#fff", role: role)],
                                            sessions: [], key: key),
                        histories: [:])
        return (client, store)
    }

    @Test func frames() throws {
        let frame = try JSONSerialization.jsonObject(with: Data(ClientMessage.inviteResetMain.frame().utf8)) as? [String: Any]
        #expect(frame?["t"] as? String == "invite.resetMain")
        #expect(frame?.keys.sorted() == ["t"], "nothing else on the wire")
        guard case .key(let key) = try ServerMessage.decode(#"{"t":"key","key":"N3w_main-key"}"#) else {
            Issue.record("key"); return
        }
        #expect(key == "N3w_main-key")
    }

    /// The owner resets: the `key` that comes back becomes the stored link,
    /// and the new link is there to copy.
    @Test func ownerResetsAndSeesTheNewLink() {
        let (client, store) = signedIn()
        #expect(client.isOwner)
        client.connection = YapConnection(url: client.endpoint!.webSocketURL, protocols: client.endpoint!.webSocketProtocols)
        client.resetMainLink()
        // Offline (the connection never opened): nothing went, and it says so.
        #expect(client.notice?.text == "Not connected right now. Try again in a moment.")
        #expect(client.newMainLink == nil)
        client.mainResetAsked = true // as if the frame had gone out
        client.receiveForTesting(.key("fresh_key"))
        #expect(store.endpoint?.key == "fresh_key", "saved as this device's link")
        #expect(client.endpoint?.key == "fresh_key")
        #expect(client.newMainLink == "https://chat.example.com/#key=fresh_key")
        #expect(client.connection?.protocols == ["yap", "key.fresh_key"], "the next connect uses it")
        #expect(client.connection?.url == client.endpoint?.webSocketURL)
        client.newMainLink = nil
        // Another reset from the owner's other device: saved, no link shown here.
        client.receiveForTesting(.key("third_key"))
        #expect(store.endpoint?.key == "third_key" && client.newMainLink == nil)
    }

    /// Everyone else gets the new key silently.
    @Test func membersSaveItSilently() {
        let (client, store) = signedIn(role: .member)
        #expect(!client.isOwner && !client.isMod)
        client.receiveForTesting(.key("fresh_key"))
        #expect(store.endpoint?.key == "fresh_key")
        #expect(client.newMainLink == nil && client.notice == nil)
        // An empty key is never adopted.
        client.receiveForTesting(.key(""))
        #expect(store.endpoint?.key == "fresh_key")
    }

    /// A mod isn't the owner: the server says no, in its own words.
    @Test func refusedResetSaysWhy() {
        let (client, _) = signedIn(role: .mod)
        #expect(client.isMod && !client.isOwner)
        client.mainResetAsked = true
        client.receiveForTesting(.error(code: "forbidden", message: "Only the owner can do that.", re: "invite.resetMain"))
        #expect(client.notice?.text == "Only the owner can do that.")
        #expect(client.mainResetAsked == false)
        client.receiveForTesting(.key("fresh_key"))
        #expect(client.newMainLink == nil, "no link for a reset that was turned down")
    }

    /// A device that still holds the old link (it was away during the reset)
    /// gets in with its token and takes the current key from `ready`.
    @Test func readyCarriesTheCurrentKey() {
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: Self.server, key: "old"), token: "t")
        let client = YapClient(credentials: store, client: "test")
        client.connection = YapConnection(url: client.endpoint!.webSocketURL, protocols: client.endpoint!.webSocketProtocols)
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me", channels: [], users: [User(id: "me", name: "Sam", color: "#fff")],
                                            sessions: [], key: "current"),
                        histories: [:])
        #expect(store.endpoint?.key == "current" && client.endpoint?.key == "current")
        #expect(client.connection?.protocols == ["yap", "key.current"])
        #expect(client.newMainLink == nil)
    }

    /// `/api/check` turns an old link down, but a device signed in to that
    /// server connects anyway (the socket lets `hello { token }` through).
    @Test func aSignedInDeviceDoesntStopAtTheKeyCheck() {
        let (client, _) = signedIn(role: .member)
        #expect(client.signsInWithToken(at: ServerEndpoint(baseURL: Self.server, key: "stale")))
        #expect(!client.signsInWithToken(at: ServerEndpoint(baseURL: URL(string: "https://other.example.com")!, key: "stale")))
        let fresh = YapClient(credentials: MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: Self.server, key: "old")), client: "test")
        #expect(!fresh.signsInWithToken(at: ServerEndpoint(baseURL: Self.server, key: "stale")), "no token: the key has to be right")
    }

    /// 4003 (or an older server's 401) = the invite key is wrong for real.
    @Test func wrongKeyCloses() {
        #expect(YapConnection.endedForWrongKey(status: nil, closeCode: 4003))
        #expect(YapConnection.endedForWrongKey(status: 401, closeCode: 0))
        #expect(!YapConnection.endedForWrongKey(status: 101, closeCode: 1006), "a dropped socket comes back")
        #expect(!YapConnection.endedForWrongKey(status: nil, closeCode: 4001), "signed out elsewhere isn't a key problem")
        #expect(!YapConnection.endedForWrongKey(status: nil, closeCode: 1000))
    }

    /// A wrong key ends on the invite screen, saying so.
    @Test func wrongKeyAsksForANewLink() {
        let (client, store) = signedIn(role: .member)
        client.connectionClosed(.badKey)
        #expect(client.phase == .needsInvite)
        #expect(client.authError?.message == "That invite link doesn't work anymore. Paste the new one.")
        #expect(store.token == "t", "the device token stays: a new link for this server keeps the device")
    }

    @Test func modLogSaysIt() {
        #expect(ModLogEntry(ts: 1, actor: "me", action: "invite.resetMain").phrase { _ in "Sam" } == "reset the main link")
    }

    @Test func signingOutForgetsTheNewLink() {
        let (client, _) = signedIn()
        client.mainResetAsked = true
        client.receiveForTesting(.key("fresh_key"))
        #expect(client.newMainLink != nil)
        client.signOut()
        #expect(client.newMainLink == nil && client.mainResetAsked == false)
    }
}
