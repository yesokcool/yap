import Foundation
import Testing
@testable import Yap
@testable import YapCore

/// Words the app shares with the web (polish pass 2): the sign-on hint and
/// its wrong-password help, the join bars' summary, and the questions.
@MainActor
@Suite("Web wording")
struct WordingTests {
    @Test func signOnHintSaysPassword() {
        #expect(AuthText.hint(signup: true) == "Pick a name friends will see and a password. It’s how you sign in on your other devices.")
        #expect(AuthText.hint(signup: false) == "Sign in with your name and password — same you, same messages, on any device.")
        #expect(PasswordText.field == "Password" && PasswordText.rule == "8 characters or more.")
    }

    /// The server's line, then who can help: together the web's words.
    @Test func wrongPasswordSaysWhoCanHelp() throws {
        let wrong = YapClient.friendly(code: "bad_login", message: "Wrong name or password.", attempted: nil)
        let help = try #require(AuthText.help(for: YapClient.AuthError(code: "bad_login", message: wrong, mode: .login)))
        #expect("\(wrong) \(help)" == "Wrong name or password. An old PIN works here too. Forgot it? The owner or a mod can reset it.")
        #expect(YapClient.friendly(code: "bad_login", message: "", attempted: nil) == "Wrong name or password.")
        let locked = YapClient.friendly(code: "locked", message: "Too many wrong tries. Try again in 4 min.", attempted: nil)
        let lockedHelp = try #require(AuthText.help(for: YapClient.AuthError(code: "locked", message: locked, mode: .login)))
        #expect("\(locked) \(lockedHelp)" == "Too many wrong tries. Try again in 4 min. Forgot your password? The owner or a mod can reset it.")
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
        #expect(Question.resetPassword("Ada") {}.title == "Reset Ada’s password?")
        #expect(Question.resetPassword("Ada") {}.action == "Reset password")
        #expect(Question.turnOffInvite("for Dee") {}.title == "Turn off “for Dee”?")
        #expect(Question.cancelHang("Game night") {}.cancel == "Keep it")
        #expect(Question.deleteMessage {}.text == "It’s erased for good, for everyone.")
    }
}
