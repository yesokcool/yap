import SwiftUI
import WebKit
import YapCore

/// The server's board (optional, served by Yap at /board) and the web-only
/// places (Draw, Space) in an in-app browser; voice keeps going. Live
/// cursors are web-only. The web app keeps its own sign-in (its storage),
/// so Draw/Space load with a one-time sign-in pass and open on that place.
struct WebPageView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    var page: WebPage

    var body: some View {
        Group {
            if let endpoint = model.client.endpoint {
                let url = page.url(for: endpoint)
                #if os(macOS)
                Window98(title: model.title(of: page), systemImage: "globe") {
                    Button { model.openInBrowser(page, with: openURL) } label: { Image(systemName: "safari") }
                        .buttonStyle(TitleBarButtonStyle())
                        .help("Open in the browser")
                        .accessibilityLabel("Open in the browser")
                } content: {
                    WebView(url: url, place: page.place).well()
                }
                #else
                WebView(url: url, place: page.place)
                    .modifier(InCallBar98())
                    .toolbar {
                        ToolbarItem(placement: .primaryAction) {
                            ToolbarMenuButton {
                                Button("Open in Safari", systemImage: "safari") { model.openInBrowser(page, with: openURL) }
                            }
                        }
                        .win98Item()
                    }
                #endif
            } else {
                Text("Not connected.").font(Theme.chrome).foregroundStyle(Theme.grayText)
            }
        }
        #if os(iOS)
        .win98NavigationBar(model.title(of: page))
        #else
        .navigationTitle(model.title(of: page))
        #endif
        .onAppear {
            if page == .board { model.client.send(.view("board")) }
        }
        .onDisappear {
            if page == .board { model.client.send(.view(nil)) }
        }
    }
}

/// SwiftUI's WebKit view (iOS 26 / macOS 26). With a web-app `place` it
/// asks for a one-time sign-in pass first, so the page opens signed in.
///
/// Two fixes over the bare view:
/// - WebKit's view reaches up under the navigation bar (like a scroll view),
///   but the web app is a fixed full-height layout, so its top strip (Space's
///   room bar, Draw's tools) sat hidden under the bar. A hairline keeps it off
///   the safe-area edge, so it starts right below the bar.
/// - The web app keeps its own light/dark in its storage; the app's goes in
///   first (`yap.settings.theme`, as `web/js/theme.js` reads it before first
///   paint), so a dark app shows a dark Draw. The web's "Add to Home Screen"
///   bar stays dismissed: it means nothing inside the app.
struct WebView: View {
    @Environment(AppModel.self) private var model
    @AppStorage(Appearance.storageKey) private var appearance: Appearance = .charcoal
    var url: URL
    var place: String? = nil
    @State private var page: WebKit.WebPage?

    var body: some View {
        Group {
            if let page {
                WebKit.WebView(page)
            } else {
                Color.clear
            }
        }
        .padding(.top, 1)
        .task(id: url) {
            let page = page ?? Self.makePage(appearance: appearance)
            self.page = page
            var load = url
            if let place, let signedIn = await model.client.signedInWebURL(place: place) { load = signedIn }
            // The web app shows just the place: no sidebar or header of its own (web/js/theme.js).
            if place != nil, let embedded = URL(string: load.absoluteString + "&embed=1") { load = embedded }
            page.load(URLRequest(url: load))
        }
    }

    /// A page whose web app starts in the app's light/dark.
    static func makePage(appearance: Appearance) -> WebKit.WebPage {
        let configuration = WebKit.WebPage.Configuration()
        let theme = switch appearance {
        case .charcoal: "dark"
        case .standard: "light"
        case .system: "system"
        }
        let source = """
        try {
          const s = JSON.parse(localStorage.getItem('yap.settings') || '{}');
          if (s.theme !== '\(theme)') {
            s.theme = '\(theme)';
            localStorage.setItem('yap.settings', JSON.stringify(s));
          }
          // "Put Yap on your Home Screen" means nothing inside the app.
          localStorage.setItem('yap.homeDismissed', String(Date.now()));
        } catch (e) {}
        """
        configuration.userContentController.addUserScript(
            WKUserScript(source: source, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        return WebKit.WebPage(configuration: configuration)
    }
}
