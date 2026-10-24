import SwiftUI
import WebKit
import YapCore

/// The server's board (optional, served by Yap at /board) and the web-only
/// places (Draw, Space) in an in-app browser; voice keeps going. Live
/// cursors are web-only. The web app keeps its own sign-in (its storage),
/// so Draw/Space load with a one-time sign-in pass and open on that place.
/// Copy link: the title bar's link button (Mac), the "…" menu (iPhone).
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
                    HStack(spacing: 2) {
                        // Like the web's header: a link to this place for anyone here.
                        Button { model.copyLink(page.link) } label: { Image(systemName: "link") }
                            .buttonStyle(TitleBarButtonStyle())
                            .help(LinkWords.help)
                            .accessibilityLabel("Copy link")
                        Button { model.openInBrowser(page, with: openURL) } label: { Image(systemName: "safari") }
                            .buttonStyle(TitleBarButtonStyle())
                            .help("Open in the browser")
                            .accessibilityLabel("Open in the browser")
                    }
                } content: {
                    WebView(url: url, place: page.place, endpoint: endpoint).well()
                }
                #else
                WebView(url: url, place: page.place, endpoint: endpoint)
                    .modifier(InCallBar98())
                    .toolbar {
                        ToolbarItem(placement: .primaryAction) {
                            ToolbarMenuButton {
                                Button("Open in Safari", systemImage: "safari") { model.openInBrowser(page, with: openURL) }
                                LinkMenuItems(link: page.link)
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
/// asks for a one-time sign-in pass right before loading (it's good for a
/// few minutes), so the page opens signed in, never as someone else
/// (`WebSignIn`).
///
/// It only ever shows this server: a link elsewhere, or one for a new
/// window, opens in the browser (`ServerEndpoint.webNavigation`), and a link
/// to a place or channel here opens in the app (both through `openURL`,
/// which is `OpensYapLinks`'s).
///
/// Two fixes over the bare view:
/// - WebKit's view reaches up under the navigation bar (like a scroll view),
///   but the web app is a fixed full-height layout, so its top strip (Space's
///   room bar, Draw's tools) sat hidden under the bar. A hairline keeps it off
///   the safe-area edge, so it starts right below the bar.
/// - The web app keeps its own light/dark in its storage; the app's goes in
///   first (`yap.settings.theme`, as `web/js/theme.js` reads it before first
///   paint), so a dark app shows a dark Draw. The web's "Add to Home Screen"
///   bar stays dismissed: it means nothing inside the app. Only on Yap's own
///   pages.
struct WebView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var openURL
    @AppStorage(Appearance.storageKey) private var appearance: Appearance = .charcoal
    var url: URL
    var place: String? = nil
    var endpoint: ServerEndpoint
    @State private var page: WebKit.WebPage?
    /// The sign-in (`AppModel.webSession`) the page was made for.
    @State private var pageSession = 0

    /// A new address, or a wiped sign-in: load again.
    private struct Load: Hashable {
        var url: URL
        var session: Int
    }

    var body: some View {
        Group {
            if let page {
                WebKit.WebView(page)
            } else {
                Color.clear
            }
        }
        .padding(.top, 1)
        .task(id: Load(url: url, session: model.webSession)) {
            // A sign-out's wipe first (a pass asked for before it would go with it).
            await model.readyWebSignIn()
            guard !Task.isCancelled else { return }
            let page: WebKit.WebPage
            if let current = self.page, pageSession == model.webSession {
                page = current
            } else {
                page = Self.makePage(appearance: appearance, endpoint: endpoint, openURL: openURL)
                model.register(page)
                pageSession = model.webSession
            }
            self.page = page
            var load = url
            if let place, let signedIn = await model.client.signedInWebURL(place: place) { load = signedIn }
            guard !Task.isCancelled else { return }
            // The web app shows just the place: no sidebar or header of its own (web/js/theme.js).
            if place != nil, let embedded = URL(string: load.absoluteString + "&embed=1") { load = embedded }
            page.load(URLRequest(url: load))
        }
    }

    /// A page whose web app starts in the app's light/dark, and that stays on this server.
    static func makePage(appearance: Appearance, endpoint: ServerEndpoint, openURL: OpenURLAction) -> WebKit.WebPage {
        let configuration = WebKit.WebPage.Configuration()
        let theme = switch appearance {
        case .charcoal: "dark"
        case .standard: "light"
        case .system: "system"
        }
        // The origin as a JS string literal (JSON-escaped).
        let origin = (try? JSONEncoder().encode(endpoint.webOrigin)).map { String(decoding: $0, as: UTF8.self) } ?? "''"
        let source = """
        if (location.origin === \(origin)) try {
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
        return WebKit.WebPage(configuration: configuration, navigationDecider: YapOnly(endpoint: endpoint, openURL: openURL))
    }
}

/// The web view's navigation rule (`ServerEndpoint.webNavigation`): this
/// server's pages in the view, everything else to the system.
struct YapOnly: WebKit.WebPage.NavigationDeciding {
    let endpoint: ServerEndpoint
    let openURL: OpenURLAction

    func decidePolicy(for action: WebKit.WebPage.NavigationAction,
                      preferences: inout WebKit.WebPage.NavigationPreferences) async -> WKNavigationActionPolicy {
        guard let url = action.request.url else { return .cancel }
        // A link to a place or channel here, clicked in the page (the board's,
        // say): the app goes there, as from a message (`OpensYapLinks`), not
        // the whole web app inside this view. Loads the page makes itself
        // (its sign-in link, a reload) aren't clicks.
        if action.navigationType == .linkActivated, action.target?.isMainFrame == true, YapLink(url, on: endpoint) != nil {
            openURL(url)
            return .cancel
        }
        // No target frame: a new window (`target=_blank`, `window.open`).
        switch endpoint.webNavigation(to: url, mainFrame: action.target?.isMainFrame ?? true, newWindow: action.target == nil) {
        case .allow:
            return .allow
        case .openOutside:
            openURL(url)
            return .cancel
        case .refuse:
            return .cancel
        }
    }
}
