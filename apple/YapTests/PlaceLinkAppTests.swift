import Foundation
import SwiftUI
import Testing
import WebKit
@testable import Yap
@testable import YapCore

/// Links to places and channels in the app (PROTOCOL.md › Links to places
/// and channels): where a tapped one goes, the words, and the in-app web
/// view handing one to the app.
@MainActor
@Suite("Links to places and channels (app)")
struct PlaceLinkAppTests {
    static let server = ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k")

    private func client(board: Bool) -> YapClient {
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: Self.server, token: "t"), client: "test")
        let links = board ? [PlaceLink(id: "b", name: "Gaming board", url: "/board", view: "board")] : []
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "me", channels: [
            Channel(id: "general", name: "general", kind: .text, position: 0),
            Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0),
            Channel(id: "p_me", name: "Personal", kind: .personal, position: 0),
        ], users: [User(id: "me", name: "Sam", color: "#5b6cff", role: .member)], sessions: [], links: links), histories: [:])
        return client
    }

    /// The way the channel list opens each one.
    @Test func aLinkGoesWhereItsRowGoes() {
        let client = client(board: true)
        func target(_ link: YapLink) -> LinkTarget { LinkTarget.of(link, in: client) }
        #expect(target(.place("draw")) == .page(.draw))
        #expect(target(.place("space")) == .page(.space))
        #expect(target(.place("board")) == .page(.board))
        #expect(target(.place("watch")) == .browser("watch"), "Watch opens in the browser, signed in, like its row")
        #expect(target(.place("when")) == .browser("when"))
        #expect(target(.channel("general")) == .channel("general", message: nil))
        #expect(target(.channel("general", message: "m1")) == .channel("general", message: "m1"))
        #expect(target(.channel("p_me")) == .channel("p_me", message: nil), "your own Personal space opens for you")
    }

    @Test func anUnknownPlaceOrAGoneChannelSaysSo() {
        let client = client(board: false)
        func target(_ link: YapLink) -> LinkTarget { LinkTarget.of(link, in: client) }
        #expect(target(.place("board")) == .notice("That link is to a place this server doesn't have."), "no board here")
        #expect(target(.place("arcade")) == .notice(LinkWords.noSuchPlace))
        #expect(target(.place("screen")) == .notice(LinkWords.noSuchPlace), "a screen share comes and goes: no link")
        #expect(target(.channel("archived1")) == .notice("That channel is gone (or isn't yours to see)."))
        #expect(target(.channel("p_someone")) == .notice(LinkWords.channelGone))
        #expect(target(.channel("lounge")) == .notice(LinkWords.channelGone), "a voice channel isn't a place to read")
    }

    @Test func copyLinkSaysTheWebsWords() {
        #expect(LinkWords.copied == "Link copied. Anyone here who opens it lands right here.")
        #expect(WebPage.board.link == .place("board") && WebPage.draw.link == .place("draw") && WebPage.space.link == .place("space"))
    }

    /// The real WebKit page: a link to a place or channel here, clicked in
    /// the page or opening a new window, goes to the app (`openURL`) and the
    /// page stays put. (The server's other pages load inside, as before:
    /// `ServerAddressTests`.)
    @Test func theWebViewHandsLinksHereToTheApp() async throws {
        final class Opened { var urls: [URL] = [] }
        let opened = Opened()
        let page = WebView.makePage(appearance: .charcoal, endpoint: Self.server, openURL: OpenURLAction { url in
            opened.urls.append(url)
            return .handled
        })
        let html = """
        <a id="place" href="https://chat.example.com/#place=when">a</a>
        <a id="channel" href="https://chat.example.com/#channel=general&amp;message=m1" target="_blank">b</a>
        """
        for try await _ in page.load(html: html, baseURL: URL(string: "https://chat.example.com/board")!) {}
        for (id, url) in [("place", "https://chat.example.com/#place=when"), ("channel", "https://chat.example.com/#channel=general&message=m1")] {
            _ = try await page.callJavaScript("document.getElementById(id).click()", arguments: ["id": id])
            for _ in 0..<40 where !opened.urls.contains(URL(string: url)!) { try await Task.sleep(for: .milliseconds(50)) }
            #expect(opened.urls.contains(URL(string: url)!), "\(id) went to the app")
        }
        #expect(opened.urls.count == 2)
        #expect(page.url?.path() == "/board", "the page didn't go anywhere")
    }
}
