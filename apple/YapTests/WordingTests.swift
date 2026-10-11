import Foundation
import Testing
@testable import Yap
@testable import YapCore

/// Words the app shares with the web (polish pass 2): the sign-on hint and
/// its wrong-PIN help, the join bars' summary, and the questions.
@MainActor
@Suite("Web wording")
struct WordingTests {
    @Test func signOnHintFollowsUseAPassword() {
        #expect(AuthText.hint(signup: true).contains("6-digit PIN"))
        #expect(AuthText.hint(signup: true, password: true).contains("password of 10 or more characters"))
        #expect(AuthText.hint(signup: false) == "Sign in with your name and PIN — same you, same messages, on any device.")
        #expect(AuthText.hint(signup: false, password: true) == "Sign in with your name and password — same you, same messages, on any device.")
    }

    @Test func wrongPinSaysWhoCanHelp() {
        let help = "Forgot your PIN? The owner or a mod can reset it."
        #expect(AuthText.help(for: YapClient.AuthError(code: "bad_login", message: "x", mode: .login)) == help)
        #expect(AuthText.help(for: YapClient.AuthError(code: "locked", message: "x", mode: .login)) == help)
        #expect(AuthText.help(for: YapClient.AuthError(code: "name_taken", message: "x", mode: .signup)) == nil)
        #expect(AuthText.help(for: nil) == nil)
    }

    @Test func joinBarsFoldIntoOne() {
        let dee = User(id: "d", name: "Dee", color: "#000000")
        let eve = User(id: "e", name: "Eve", color: "#000000")
        #expect(JoinRequests.summary([dee]) == ("Dee", " wants to join"))
        #expect(JoinRequests.summary([dee, eve]) == ("2 people", " want to join"))
    }

    @Test func questionsUseTheWebsWords() {
        #expect(Question.resetPin("Ada") {}.title == "Reset Ada’s PIN?")
        #expect(Question.turnOffInvite("for Dee") {}.title == "Turn off “for Dee”?")
        #expect(Question.cancelHang("Game night") {}.cancel == "Keep it")
        #expect(Question.deleteMessage {}.text == "It’s erased for good, for everyone.")
    }
}
