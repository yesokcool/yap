import Foundation
import SwiftUI
import Testing
import WebKit
@testable import Yap
@testable import YapCore
@testable import YapVoice

/// The security review's fixes in the app's words (2026-10-24): the web
/// view never opens as someone else, a new password signs out the other
/// devices, and Hide my IP address says when it blocks the call.
@MainActor
@Suite("Security review")
struct SecurityAppTests {
    /// Whose sign-in the web view holds: wiped before a page opens for anyone else.
    @Test func theWebViewNeverOpensAsSomeoneElse() {
        let server = ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k")
        let ada = WebSignIn.owner(endpoint: server, uid: "ada")
        #expect(ada == "https://chat.example.com ada")
        #expect(WebSignIn.owner(endpoint: nil, uid: "ada") == "" && WebSignIn.owner(endpoint: server, uid: "") == "")
        #expect(WebSignIn.needsWipe(held: nil, owner: ada), "an older version may have left anyone's")
        #expect(!WebSignIn.needsWipe(held: "", owner: ada), "already wiped")
        #expect(!WebSignIn.needsWipe(held: ada, owner: ada))
        #expect(WebSignIn.needsWipe(held: "https://chat.example.com ben", owner: ada))
        #expect(WebSignIn.needsWipe(held: "https://other.example.com ada", owner: ada))
    }

    /// The real WebKit page: a link elsewhere and a `target=_blank` link go
    /// to the system; the page stays on the server.
    @Test func theWebViewHandsOtherSitesToTheSystem() async throws {
        final class Opened { var urls: [URL] = [] }
        let opened = Opened()
        let server = ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k")
        let page = WebView.makePage(appearance: .charcoal, endpoint: server, openURL: OpenURLAction { url in
            opened.urls.append(url)
            return .handled
        })
        let html = #"<a id="away" href="https://evil.example/away">a</a> <a id="blank" href="https://chat.example.com/media/x" target="_blank">b</a>"#
        for try await _ in page.load(html: html, baseURL: URL(string: "https://chat.example.com/")!) {}
        for (id, url) in [("away", "https://evil.example/away"), ("blank", "https://chat.example.com/media/x")] {
            _ = try await page.callJavaScript("document.getElementById(id).click()", arguments: ["id": id])
            for _ in 0..<40 where !opened.urls.contains(URL(string: url)!) { try await Task.sleep(for: .milliseconds(50)) }
            #expect(opened.urls.contains(URL(string: url)!), "\(id) went to the system")
        }
        #expect(page.url?.host() == "chat.example.com", "nothing else loaded in the view")
    }

    @Test func aNewPasswordSignsOutTheOthers() {
        #expect(PasswordText.othersSignedOut == "Your other devices will be signed out.")
        let changed = PasswordText.saved(onPin: false)
        #expect(PasswordText.saved(onPin: false, signedOut: nil) == changed && PasswordText.saved(onPin: false, signedOut: 0) == changed)
        #expect(PasswordText.saved(onPin: false, signedOut: 1) == changed + " Your other device was signed out.")
        #expect(PasswordText.saved(onPin: true, signedOut: 2).hasSuffix(" Your 2 other devices were signed out."))
        #expect(Question.resetPassword("Ada") {}.text?.hasSuffix("all their devices are signed out.") == true)
    }

    @Test func hideMyIPSaysWhenItBlocksTheCall() {
        #expect(VoiceOptions.relayMissing
            == "Hide my IP address is on, but this server has no voice relay, so you can't connect to anyone. Turn it off in Settings › Voice.")
        #expect(VoiceOptions.relayNote(available: false, on: true).contains("can't connect to anyone"))
        #expect(VoiceOptions.relayNote(available: false) == "Needs the voice relay, which isn't switched on for this server yet.")
        #expect(VoiceOptions.relayNote(available: true, on: true).hasPrefix("Sends your voice through the server's voice relay"))
        #expect(VoiceHintText.label(.hiddenNoRelay) == "can't connect (Hide my IP)")
        #expect(VoiceHintText.kind(.hiddenNoRelay) == .blocked)
        #expect(VoiceHintText.detail(.hiddenNoRelay, name: "Ada") == VoiceOptions.relayMissing)
    }
}
