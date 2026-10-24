import Foundation
import WebKit
import YapCore

/// The in-app web view's own sign-in (Draw, Space, the board). Its web app
/// swaps the one-time pass for a lasting device token kept in its storage,
/// and ignores later passes, so whoever it's signed in as stays: on a shared
/// iPad or Mac the next person opened Draw as the last one. Signing out
/// (here, by the server, a new server) wipes what WebKit keeps for the app
/// (it only ever shows Yap); a page never opens signed in as someone else.
/// The persistent store stays: a private one would make a new device on the
/// server every time a page opens.
enum WebSignIn {
    /// Whose sign-in the web view holds: "<server> <uid>", "" once wiped, nil
    /// if no version has said (an older one may have left anyone's).
    static let key = "yap.webSignIn"

    static func held(_ defaults: UserDefaults = .standard) -> String? { defaults.string(forKey: key) }

    /// "<server> <uid>" for the account signed in now ("" when nobody is).
    static func owner(endpoint: ServerEndpoint?, uid: String) -> String {
        guard let endpoint, !uid.isEmpty else { return "" }
        return "\(endpoint.baseURL.absoluteString) \(uid)"
    }

    /// Wipe before a page opens for `owner`: the web view may hold someone
    /// else's sign-in (another account or server), or nobody said whose.
    static func needsWipe(held: String?, owner: String) -> Bool {
        guard let held else { return true }
        return !held.isEmpty && held != owner
    }
}

/// An open page, held weakly (the view owns it).
struct WeakWebPage {
    weak var page: WebKit.WebPage?
}

extension AppModel {
    /// Signed out: close the open pages, wipe the web view's data, and say nobody's signed in there.
    func forgetWebSignIn() {
        webSession += 1
        wipeWebData()
    }

    /// Right before a page opens (and before its pass is asked for, which a
    /// wipe would take with it): wipe if it holds someone else's sign-in.
    func readyWebSignIn() async {
        await webWipe?.value
        let owner = WebSignIn.owner(endpoint: client.endpoint, uid: client.uid)
        if WebSignIn.needsWipe(held: WebSignIn.held(), owner: owner) {
            wipeWebData()
            await webWipe?.value
        }
        UserDefaults.standard.set(owner, forKey: WebSignIn.key)
    }

    func register(_ page: WebKit.WebPage) {
        webPages.removeAll { $0.page == nil }
        webPages.append(WeakWebPage(page: page))
    }

    /// Wipe at once, take the open pages to a blank one (so none writes its
    /// token back), then wipe again.
    private func wipeWebData() {
        let pages = webPages.compactMap(\.page)
        webPages.removeAll()
        let previous = webWipe
        webWipe = Task {
            await previous?.value
            await Self.removeWebsiteData()
            for page in pages { await Self.blank(page) }
            if !pages.isEmpty { await Self.removeWebsiteData() }
            UserDefaults.standard.set("", forKey: WebSignIn.key)
        }
    }

    private static func removeWebsiteData() async {
        await WKWebsiteDataStore.default().removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast)
    }

    /// Off to about:blank (the old page unloads), waiting at most 2 s.
    private static func blank(_ page: WebKit.WebPage) async {
        page.stopLoading()
        guard let blank = URL(string: "about:blank") else { return }
        let once = Once()
        await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
            once.continuation = done
            Task {
                do { for try await event in page.load(URLRequest(url: blank)) where event == .finished { break } } catch {}
                once.finish()
            }
            Task {
                try? await Task.sleep(for: .seconds(2))
                once.finish()
            }
        }
    }

    /// Resumes the wait once, whichever comes first.
    private final class Once {
        var continuation: CheckedContinuation<Void, Never>?
        func finish() {
            continuation?.resume()
            continuation = nil
        }
    }
}
