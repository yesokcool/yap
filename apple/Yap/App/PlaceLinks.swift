import SwiftUI
import YapCore
#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// Links to places and channels (PROTOCOL.md › Links to places and
/// channels): Copy link on every text channel and place (not voice, not the
/// Personal space, not a Places link to another site), `<server>/#place=space`
/// or `/#channel=<id>`, never the invite key. Such a link tapped in a message,
/// a Places row or the in-app web view opens here, the way the channel list
/// opens it (`OpensYapLinks`); an invite or sign-in link, another server's,
/// and everything else go to the system as before.
enum LinkWords {
    /// The web's words for its header's link button…
    static let copied = "Link copied. Anyone here who opens it lands right here."
    static let help = "Copy a link to this: anyone here who opens it lands right here"
    /// …and for a link it can't follow.
    static let noSuchPlace = "That link is to a place this server doesn't have."
    static let channelGone = "That channel is gone (or isn't yours to see)."
}

extension WebPage {
    /// Its link: `#place=board|draw|space`.
    var link: YapLink { .place(place ?? "board") }
}

/// Where a link to a place or channel goes in the app: where its row in the
/// channel list goes.
enum LinkTarget: Equatable {
    /// Draw, Space, the board: the in-app web view (signed in with a one-time pass).
    case page(WebPage)
    /// Watch, When's good: the web app in the browser, signed in, as their rows open it.
    case browser(String)
    /// A text channel (or your own Personal space), at a message when the link names one.
    case channel(String, message: String?)
    /// A place this server doesn't have, a channel that's gone: a notice says so.
    case notice(String)

    static func of(_ link: YapLink, in client: YapClient) -> LinkTarget {
        switch link {
        case .place("draw"): .page(.draw)
        case .place("space"): .page(.space)
        // The board only on a server with one (its Places link).
        case .place("board"): client.links.contains { $0.view == "board" } ? .page(.board) : .notice(LinkWords.noSuchPlace)
        case .place(let place) where place == "watch" || place == "when": .browser(place)
        case .place: .notice(LinkWords.noSuchPlace)
        case .channel(let id, let message):
            // Not a voice channel: a link is for reading. (Someone else's Personal space never reaches us.)
            client.channelsById[id]?.hasMessages == true ? .channel(id, message: message) : .notice(LinkWords.channelGone)
        }
    }
}

extension AppModel {
    /// The link to hand to someone here, nil without a server.
    func linkURL(for link: YapLink) -> URL? { client.endpoint?.link(to: link) }

    /// Copy link: on the clipboard, and a notice that says so.
    func copyLink(_ link: YapLink) {
        guard let url = linkURL(for: link) else { return }
        copyToPasteboard(url.absoluteString)
        client.notice = YapClient.Notice(text: LinkWords.copied)
    }

    /// A tapped link: this server's link to a place or channel opens here;
    /// anything else is the system's, as before. `outside` is the system's
    /// own opener (Watch and When's good open in the browser).
    func openLink(_ url: URL, outside: OpenURLAction) -> OpenURLAction.Result {
        guard let endpoint = client.endpoint, let link = YapLink(url, on: endpoint) else { return .systemAction }
        follow(link, outside: outside)
        return .handled
    }

    /// Go where a link points. Right after launch the channels may not be in
    /// yet: it waits for them (up to 30 s), like a tapped notification.
    func follow(_ link: YapLink, outside: OpenURLAction) {
        Task {
            for _ in 0..<150 where client.channelsById.isEmpty { try? await Task.sleep(for: .milliseconds(200)) }
            switch LinkTarget.of(link, in: client) {
            case .page(let page): open(.web(page))
            case .browser(let place): client.openWeb(place, with: outside)
            case .channel(let id, let message):
                // The channel screen pages back to the message, scrolls to it and flashes it.
                if let message { jump = JumpTarget(channelId: id, messageId: message) }
                open(.channel(id))
            case .notice(let text): client.notice = YapClient.Notice(text: text)
            }
        }
    }

    #if DEBUG
    /// `YAP_OPEN_LINK`: no view to ask for the system's opener.
    static let systemOpener = OpenURLAction { url in
        #if os(iOS)
        UIApplication.shared.open(url)
        #else
        NSWorkspace.shared.open(url)
        #endif
        return .handled
    }
    #endif
}

/// Every link tapped in the app comes here first (message text, a Places
/// row, the in-app web view's links that leave the page): this server's
/// links to places and channels open in the app (`AppModel.openLink`).
struct OpensYapLinks: ViewModifier {
    @Environment(AppModel.self) private var model
    /// The system's: what opens everything else (a browser, Mail).
    @Environment(\.openURL) private var system

    func body(content: Content) -> some View {
        content.environment(\.openURL, OpenURLAction { [model, system] url in model.openLink(url, outside: system) })
    }
}

/// Copy link (and Share link… on iPhone and iPad) in a row's context menu or a "…" menu.
struct LinkMenuItems: View {
    @Environment(AppModel.self) private var model
    var link: YapLink

    var body: some View {
        if let url = model.linkURL(for: link) {
            Button("Copy link", systemImage: "link") { model.copyLink(link) }
                .help(LinkWords.help)
            #if os(iOS)
            ShareLink(item: url) { Label("Share link…", systemImage: "square.and.arrow.up") }
            #endif
        }
    }
}
