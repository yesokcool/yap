import Foundation
import Testing
@testable import Yap
@testable import YapCore

/// Passwords instead of PINs, in the app's words (the same in every
/// client), the "Choose a password" prompt's once-per-device memory, and
/// the waiting room being the owner's and mods' alone.
@MainActor
@Suite("Passwords and the waiting room")
struct PasswordWordingTests {
    @Test func thePromptsThreeTexts() {
        #expect(PasswordText.promptTitle == "Choose a password")
        #expect(PasswordText.prompt(.pin) == "Yap uses passwords now. Your PIN still works until you choose one.")
        #expect(PasswordText.prompt(.temporary) == "You signed in with a temporary password. Choose your own.")
        #expect(PasswordText.prompt(.guessable) == "Your password is easy to guess. Choose a stronger one.")
        #expect(PasswordText.choose == "Choose a password" && PasswordText.later == "Later")
    }

    @Test func settingsAccountWords() {
        #expect(PasswordText.change(onPin: false) == "Change password")
        #expect(PasswordText.change(onPin: true) == "Set a password")
        #expect(PasswordText.current(onPin: false) == "Current password")
        #expect(PasswordText.current(onPin: true) == "Current password or PIN")
        #expect(PasswordText.new == "New password")
        #expect(AccountText.howToSignIn.hasSuffix("sign in with your name and password."))
        #expect(AccountText.signOutMessage == "You can sign back in with your name and password.")
    }

    @Test func aResetsResult() {
        #expect(PasswordText.reset == "Reset password…")
        #expect(PasswordText.resetResult(name: "Cleo", password: "maple-river-stone-42")
            == "Cleo’s temporary password: maple-river-stone-42. Send it to Cleo; they’ll choose their own after signing in.")
    }

    /// "PIN" only where someone may still be on one: the prompt's PIN line,
    /// "Current password or PIN", and a wrong sign-in's "An old PIN works
    /// here too." (the web's words).
    @Test func noPinLeftInTheWords() {
        let words = [AuthText.hint(signup: true), AuthText.hint(signup: false), AuthText.waiting("Dee"), AuthText.waitingLine,
                     AuthText.help(for: YapClient.AuthError(code: "locked", message: "", mode: .login)) ?? "",
                     PasswordText.field, PasswordText.rule, PasswordText.show, PasswordText.change(onPin: true),
                     PasswordText.change(onPin: false), PasswordText.current(onPin: false), PasswordText.new,
                     PasswordText.saved(onPin: true), PasswordText.saved(onPin: false), PasswordText.prompt(.temporary),
                     PasswordText.prompt(.guessable), PasswordText.reset, PasswordText.temporaryTitle, PasswordText.copy,
                     PasswordText.resetResult(name: "Cleo", password: "x"), AccountText.howToSignIn, AccountText.signOutMessage,
                     Question.resetPassword("Ada") {}.title, Question.resetPassword("Ada") {}.text ?? "", JoinRequests.note]
        for text in words { #expect(!text.contains("PIN"), "\(text)") }
    }

    @Test func theWaitingRoomSaysWhoLetsYouIn() {
        #expect(AuthText.waitingLine == "Waiting for the owner or a mod to let you in.")
        #expect(AuthText.waiting("Dee").contains(AuthText.waitingLine))
        #expect(!AuthText.waiting("Dee").contains("Everyone here"), "only the owner and mods get the push now")
    }

    /// Later puts that prompt off on this device for good; a different
    /// reason (a reset's temporary password) still asks once.
    @Test func laterPutsItOffForGood() throws {
        let defaults = try #require(UserDefaults(suiteName: "yap.tests.passwordPrompt"))
        defaults.removePersistentDomain(forName: "yap.tests.passwordPrompt")
        #expect(PasswordPrompt.putOff(defaults).isEmpty)
        #expect(PasswordPrompt.showing(.pin, putOff: PasswordPrompt.putOff(defaults)) == .pin)
        #expect(PasswordPrompt.showing(nil, putOff: []) == nil)
        PasswordPrompt.save(["pin"], defaults)
        #expect(PasswordPrompt.showing(.pin, putOff: PasswordPrompt.putOff(defaults)) == nil)
        #expect(PasswordPrompt.showing(.temporary, putOff: PasswordPrompt.putOff(defaults)) == .temporary)
        defaults.removePersistentDomain(forName: "yap.tests.passwordPrompt")
    }

    /// Members never see Let in / Turn away, even if an older server tells them.
    @Test func onlyOwnerAndModsSeeTheWaitingRoom() {
        let dee = User(id: "dee", name: "Dee", color: "#12a594", role: .pending)
        func client(role: Role) -> YapClient {
            let client = YapClient(credentials: MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t"), client: "test")
            client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me", channels: [Channel(id: "general", name: "general", kind: .text, position: 0)],
                                                users: [User(id: "me", name: "Me", color: "#5b6cff", role: role)], sessions: [], pending: [dee]),
                            histories: [:])
            return client
        }
        #expect(JoinRequests.waiting(client(role: .member)).isEmpty)
        #expect(JoinRequests.waiting(client(role: .mod)).map(\.id) == ["dee"])
        #expect(JoinRequests.waiting(client(role: .owner)).map(\.id) == ["dee"])
        #expect(JoinRequests.note == "Only the owner and mods can let people in. Turning someone away bans them.")
    }
}
