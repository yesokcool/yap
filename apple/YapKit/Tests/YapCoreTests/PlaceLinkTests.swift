import Foundation
import Testing
@testable import YapCore

/// Links to places and channels (PROTOCOL.md › Links to places and channels):
/// what Copy link makes, and which tapped links the app opens itself.
@Suite("Links to places and channels")
struct PlaceLinkTests {
    let server = ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "secret-key")

    private func read(_ link: String, on endpoint: ServerEndpoint? = nil) -> YapLink? {
        YapLink(URL(string: link)!, on: endpoint ?? server)
    }

    @Test func copyLinkMakesTheWebsAddress() {
        #expect(server.link(to: .place("space")).absoluteString == "https://chat.example.com/#place=space")
        #expect(server.link(to: .place("board")).absoluteString == "https://chat.example.com/#place=board")
        #expect(server.link(to: .channel("general")).absoluteString == "https://chat.example.com/#channel=general")
        #expect(server.link(to: .channel("a1b2c3", message: "m9")).absoluteString == "https://chat.example.com/#channel=a1b2c3&message=m9")
        for place in YapLink.places {
            #expect(!server.link(to: .place(place)).absoluteString.contains("secret-key"), "never the invite key")
        }
        // The origin as the web shows it: no default port; a test server keeps its own.
        let withPort = ServerEndpoint(baseURL: URL(string: "https://chat.example.com:443")!, key: "k")
        #expect(withPort.link(to: .place("draw")).absoluteString == "https://chat.example.com/#place=draw")
        let local = ServerEndpoint(baseURL: URL(string: "http://127.0.0.1:8789")!, key: "k")
        #expect(local.link(to: .place("when")).absoluteString == "http://127.0.0.1:8789/#place=when")
    }

    @Test func idsArePercentEncodedBothWays() {
        let odd = ["café talk", "a&b=c", "100%", "plus+one", "slash/hash#", "ünï"]
        for id in odd {
            let link = server.link(to: .channel(id, message: id))
            #expect(link.fragment(percentEncoded: true)?.contains(" ") == false, "\(link)")
            #expect(YapLink(link, on: server) == .channel(id, message: id), "\(id) round trips")
        }
        #expect(server.link(to: .channel("café")).absoluteString == "https://chat.example.com/#channel=caf%C3%A9")
        // The web encodes with encodeURIComponent, and URLSearchParams reads `+` as a space.
        #expect(read("https://chat.example.com/#channel=caf%C3%A9%20talk") == .channel("café talk"))
        #expect(read("https://chat.example.com/#channel=a%26b%3Dc&message=x%2By") == .channel("a&b=c", message: "x+y"))
        #expect(read("https://chat.example.com/#channel=movie+night") == .channel("movie night"))
    }

    @Test func ownServersLinksOpenInTheApp() {
        #expect(read("https://chat.example.com/#place=space") == .place("space"))
        #expect(read("https://chat.example.com/#place=when") == .place("when"))
        #expect(read("https://chat.example.com#place=draw") == .place("draw"), "no slash before the #")
        #expect(read("https://CHAT.example.com:443/#place=watch") == .place("watch"), "same origin, spelled differently")
        #expect(read("https://chat.example.com/#channel=general") == .channel("general"))
        #expect(read("https://chat.example.com/#channel=general&message=m1") == .channel("general", message: "m1"))
        #expect(read("https://chat.example.com/#message=m1&channel=general") == .channel("general", message: "m1"), "in any order")
        #expect(read("https://chat.example.com/#channel=general&message=") == .channel("general"), "an empty message is none")
        #expect(read("https://chat.example.com/#place=space&channel=general") == .place("space"), "a place first, as the web does")
        #expect(read("https://chat.example.com/#place=&channel=general") == .channel("general"))
        #expect(read("https://chat.example.com/?utm=x#place=space") == .place("space"), "a query doesn't matter")
    }

    @Test func anUnknownPlaceIsStillALinkHere() {
        // The app opens it and says the server doesn't have it, rather than handing it to the browser.
        #expect(read("https://chat.example.com/#place=arcade") == .place("arcade"))
        #expect(!YapLink.places.contains("arcade"))
        #expect(YapLink.places.sorted() == ["board", "draw", "space", "watch", "when"])
    }

    @Test func otherLinksKeepOpeningInTheBrowser() {
        for link in [
            "https://other.example.com/#place=space", // another server
            "https://chat.example.com.evil.example/#place=space",
            "http://chat.example.com/#place=space", // http isn't the same origin
            "https://chat.example.com:8443/#place=space",
            "https://chat.example.com/board#place=space", // another page
            "https://chat.example.com/changelog",
            "https://chat.example.com/",
            "https://chat.example.com/#",
            "https://chat.example.com/#place=",
            "https://chat.example.com/#channel=",
            "https://chat.example.com/#message=m1",
            "https://chat.example.com/#embed=1",
            "mailto:ada@example.com",
        ] {
            #expect(read(link) == nil, "\(link)")
        }
    }

    @Test func inviteAndSignInLinksKeepTheirOwnWay() {
        for link in [
            "https://chat.example.com/#key=secret-key",
            "https://chat.example.com/#key=secret-key&place=when",
            "https://chat.example.com/#place=draw&key=k",
            "https://chat.example.com/#key=k&place=draw&handoff=p&name=Ada",
            "https://chat.example.com/#place=draw&handoff=p",
            "https://chat.example.com/#channel=general&handoff=p",
            "https://chat.example.com/?key=k#place=space",
            "https://chat.example.com/?handoff=p#channel=general",
        ] {
            #expect(read(link) == nil, "\(link)")
        }
        // What the app itself opens for When's good is a sign-in link, so it never comes back to the app.
        let signedIn = server.inviteLink + "&place=when&handoff=pass"
        #expect(read(signedIn) == nil)
    }

    @Test func aLinkMadeHereReadsBackAsTheSame() {
        let links: [YapLink] = YapLink.places.map { .place($0) } + [.channel("general"), .channel("a1b2c3d4e5f6", message: "m_42")]
        for link in links {
            #expect(YapLink(server.link(to: link), on: server) == link, "\(link)")
            let other = ServerEndpoint(baseURL: URL(string: "https://other.example.com")!, key: "k")
            #expect(YapLink(server.link(to: link), on: other) == nil, "\(link) isn't another server's")
        }
    }
}
