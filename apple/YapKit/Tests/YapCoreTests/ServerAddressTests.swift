import Foundation
import Testing
@testable import YapCore

/// Which servers the app talks to, and what its web view shows (security
/// review, 2026-10-24): https, or plain http only to this computer; in the
/// web view, the server's own pages only.
@MainActor
@Suite("Server addresses and the web view")
struct ServerAddressTests {
    @Test func plainHTTPOnlyToThisComputer() {
        for link in ["http://127.0.0.1:8789/#key=k", "http://localhost:8787/#key=k", "http://127.10.0.3/#key=k", "http://[::1]:8787/#key=k"] {
            #expect(ServerEndpoint.parse(invite: link) != nil, "\(link)")
        }
        for link in ["http://chat.example.com/#key=k", "http://192.168.1.20:8787/#key=k", "http://127.0.0.1.evil.example/#key=k",
                     "http://localhost.evil.example/#key=k", "http://10.0.0.5/#key=k", "http://127.1/#key=k", "http://[::2]/#key=k"] {
            #expect(throws: ServerEndpoint.InviteProblem.plainHTTP, "\(link)") { try ServerEndpoint.read(invite: link) }
        }
        #expect(throws: ServerEndpoint.InviteProblem.notALink) { try ServerEndpoint.read(invite: "bareKey42") }
        #expect(ServerEndpoint.InviteProblem.plainHTTP.message.contains("https://"))
        #expect(ServerEndpoint.parse(invite: "chat.example.com/#key=k")?.baseURL.scheme == "https", "https assumed")
        #expect(ServerEndpoint.isLoopback("LOCALHOST") && ServerEndpoint.isLoopback("127.255.255.254") && !ServerEndpoint.isLoopback("127.0.0.256"))
    }

    /// A plain-http server saved by an older version isn't used (nor its
    /// token sent): the invite screen asks for the https link.
    @Test func aSavedPlainHTTPServerIsntUsed() {
        let saved = ServerEndpoint(baseURL: URL(string: "http://chat.example.com")!, key: "k")
        let client = YapClient(credentials: MemoryCredentialStore(endpoint: saved, token: "t"), client: "test")
        #expect(client.endpoint == nil && client.phase == .needsInvite)
        #expect(client.authError?.code == "plain_http")
        client.useInvite(saved)
        #expect(client.endpoint == nil, "nor taken from anywhere else")
        let local = ServerEndpoint(baseURL: URL(string: "http://127.0.0.1:8789")!, key: "k")
        #expect(YapClient(credentials: MemoryCredentialStore(endpoint: local, token: "t"), client: "test").endpoint == local)
    }

    @Test func theWebViewStaysOnTheServer() {
        let server = ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k")
        func go(_ link: String, main: Bool = true, newWindow: Bool = false) -> ServerEndpoint.WebNavigation {
            server.webNavigation(to: URL(string: link)!, mainFrame: main, newWindow: newWindow)
        }
        #expect(go("https://chat.example.com/#key=k&place=draw&embed=1") == .allow)
        #expect(go("https://chat.example.com:443/board") == .allow)
        #expect(go("https://CHAT.example.com/changelog") == .allow)
        #expect(go("https://evil.example/login") == .openOutside, "another site never shows inside the app")
        #expect(go("http://chat.example.com/") == .openOutside, "http isn't the same origin")
        #expect(go("https://chat.example.com:8443/") == .openOutside)
        #expect(go("https://chat.example.com.evil.example/") == .openOutside)
        #expect(go("mailto:ada@example.com") == .openOutside)
        #expect(go("https://chat.example.com/media/abc", newWindow: true) == .openOutside, "target=_blank goes to the browser")
        #expect(go("https://www.youtube.com/watch?v=x", newWindow: true) == .openOutside)
        #expect(go("javascript:alert(1)", newWindow: true) == .refuse)
        #expect(go("https://www.youtube-nocookie.com/embed/x", main: false) == .allow, "a frame inside the page")
        #expect(go("about:blank") == .allow)
        #expect(go("about:srcdoc") == .refuse)
        #expect(go("data:text/html,<h1>Yap</h1>") == .refuse)
        #expect(go("javascript:alert(1)") == .refuse)
        #expect(go("blob:https://chat.example.com/1f2e") == .allow)
        #expect(go("blob:https://evil.example/1f2e") == .refuse)
        #expect(go("yap://x") == .refuse)
        #expect(server.webOrigin == "https://chat.example.com")
        #expect(ServerEndpoint(baseURL: URL(string: "http://127.0.0.1:8789")!, key: "k").webOrigin == "http://127.0.0.1:8789")
        #expect(ServerEndpoint(baseURL: URL(string: "https://chat.example.com:443")!, key: "k").webOrigin == "https://chat.example.com")
    }
}
