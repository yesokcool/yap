import Foundation
import Testing
@testable import YapCore

/// Passwords instead of PINs (PROTOCOL.md › Identity): what the client
/// checks before sending (only a new password's length), and why
/// "Choose a password" comes up (`ready.weakPin` + `ready.weakReason`, or
/// `secretKind` from a server that doesn't say why).
@MainActor
@Suite("Passwords")
struct PasswordTests {
    static let server = ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k")

    static func ready(weak: Bool, kind: String?, reason: String? = nil) -> ReadyPayload {
        var ready = ReadyPayload(sid: "s0", uid: "me", channels: [Channel(id: "general", name: "general", kind: .text, position: 0)],
                                 users: [User(id: "me", name: "Me", color: "#5b6cff")], sessions: [])
        ready.weakPin = weak
        ready.secretKind = kind
        ready.weakReason = reason
        return ready
    }

    static func nudge(weak: Bool = true, kind: String?, reason: String?) -> YapClient.PasswordNudge? {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: server, token: "t"), client: "test")
        client.loadDemo(ready: ready(weak: weak, kind: kind, reason: reason), histories: [:])
        return client.passwordNudge
    }

    @Test func onlyANewPasswordHasAShape() {
        #expect(Secret.minimum == 8 && Secret.maximum == 128)
        #expect(Secret.isPassword("correct-horse-582039"))
        #expect(Secret.isPassword("12345678"), "digits are fine; the server judges guessable ones")
        #expect(!Secret.isPassword("582039"), "a 6-digit PIN can't be a new password")
        #expect(!Secret.isPassword(String(repeating: "x", count: 129)))
        // Signing in takes what was set before: an old PIN, a short old password.
        #expect(Secret.canSignIn("582039") && Secret.canSignIn("correct-horse-582039") && Secret.canSignIn("a"))
        #expect(!Secret.canSignIn("") && !Secret.canSignIn(String(repeating: "x", count: 129)))
    }

    @Test func noNudgeWithoutWeakPin() {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: Self.server, token: "t"), client: "test")
        client.loadDemo(ready: Self.ready(weak: false, kind: "password"), histories: [:])
        #expect(client.passwordNudge == nil && !client.onPin)
    }

    @Test func anOldPinAsksForAPassword() {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: Self.server, token: "t"), client: "test")
        client.loadDemo(ready: Self.ready(weak: true, kind: "pin"), histories: [:])
        #expect(client.onPin && client.passwordNudge == .pin)
        // A password set (here or on another device's echo): nothing more to ask.
        client.receiveForTesting(try! ServerMessage.decode(#"{"t":"account","hasPin":true,"kind":"password"}"#))
        #expect(client.passwordNudge == nil && !client.onPin)
    }

    /// The server says why (`ready.weakReason`), and that picks the words.
    @Test func theServerSaysWhy() {
        #expect(Self.nudge(kind: "pin", reason: "pin") == .pin)
        #expect(Self.nudge(kind: "password", reason: "temporary") == .temporary)
        #expect(Self.nudge(kind: "password", reason: "guessable") == .guessable)
        #expect(Self.nudge(weak: false, kind: "password", reason: "temporary") == nil, "only with weakPin")
    }

    /// An older server sends weakPin without a reason: an old PIN if that's
    /// what I signed in with, else a guessable password. So does a reason
    /// this app doesn't know yet.
    @Test func anOlderServerDoesntSayWhy() {
        #expect(Self.nudge(kind: "pin", reason: nil) == .pin)
        #expect(Self.nudge(kind: "password", reason: nil) == .guessable)
        #expect(Self.nudge(kind: nil, reason: nil) == .guessable)
        #expect(Self.nudge(kind: "pin", reason: "someday") == .pin)
        #expect(Self.nudge(kind: "password", reason: "someday") == .guessable)
    }

    /// A reset's temporary password: asked for your own until a password is
    /// set; the reason comes from the server on every `ready`, so nothing
    /// is kept on the device.
    @Test func aTemporaryPasswordAsksForYourOwn() throws {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: Self.server, token: "t"), client: "test")
        let frame = #"{"t":"ready","sid":"s0","uid":"me","channels":[],"users":[],"sessions":[],"weakPin":true,"weakReason":"temporary","secretKind":"password"}"#
        client.receiveForTesting(try ServerMessage.decode(frame))
        #expect(client.weakReason == "temporary" && client.passwordNudge == .temporary)
        client.receiveForTesting(try ServerMessage.decode(#"{"t":"account","hasPin":true,"kind":"password"}"#))
        #expect(client.passwordNudge == nil && client.weakReason == nil)
    }

    @Test func theWireKeepsItsNames() throws {
        let login = try JSONSerialization.jsonObject(with: JSONEncoder().encode(ClientMessage.hello(.login(name: "Me", pin: "correct-horse-582039"), client: "ios"))) as? [String: Any]
        #expect((login?["login"] as? [String: Any])?["pin"] as? String == "correct-horse-582039")
        let change = try JSONSerialization.jsonObject(with: JSONEncoder().encode(ClientMessage.accountPin(pin: "correct-horse-582039", current: "582039"))) as? [String: Any]
        #expect(change?["t"] as? String == "account.pin" && change?["current"] as? String == "582039")
        guard case let .pinReset(uid, pin) = try ServerMessage.decode(#"{"t":"pin.reset","uid":"cleo","pin":"maple-river-stone-42"}"#) else {
            Issue.record("pin.reset"); return
        }
        #expect(uid == "cleo" && pin == "maple-river-stone-42")
    }

    @Test func waitingRoomWords() throws {
        guard case let .pending(message, _, _) = try ServerMessage.decode(#"{"t":"pending"}"#) else { Issue.record("pending"); return }
        #expect(message == "Waiting for the owner or a mod to let you in.")
    }
}
