import ImageIO
import SwiftUI
import YapCore

/// What a message shows: its own text/GIF/picture, or — for an
/// encrypted Personal-space note — what it decrypts to (or why it can't).
struct MessageContent {
    var text: String
    var gif: Gif?
    var image: ImageRef?
    var sealedImage: Vault.SealedImage?
    /// "🔒 Encrypted…" when the note can't be shown.
    var sealedNotice: String?

    @MainActor
    init(_ message: Message, client: YapClient) {
        switch client.openNote(message.body) {
        case .plain:
            text = message.body
            gif = message.gif
            image = message.image
        case .note(let note):
            text = note.text
            gif = note.gif
            sealedImage = note.image
        case .locked:
            text = ""
            sealedNotice = "🔒 Encrypted. Unlock your Personal space to read it."
        case .unreadable:
            text = ""
            sealedNotice = "🔒 Can’t open this note (it was locked with another passcode)."
        }
    }

    /// The text to show: collapsed to ~3000 characters / 30 lines unless expanded.
    func shownText(expanded: Bool) -> String {
        isLong && !expanded ? MessageLength.collapsed(text) : text
    }

    var isLong: Bool { MessageLength.isLong(text) }

    /// The web's fold button: "Show more (3,214 characters)" / "Show less".
    func foldLabel(expanded: Bool) -> String {
        expanded ? "Show less" : "Show more (\(text.count.formatted()) characters)"
    }
}

/// A message being edited in the composer, and the draft it replaced.
struct MessageEdit: Equatable {
    var message: Message
    var savedDraft: String

    /// Puts the message's text in the composer (custom emoji as `:name:`;
    /// an encrypted note's own text), keeping what was typed so Cancel can
    /// bring it back. Nil while an encrypted note can't be opened.
    @MainActor
    static func begin(_ message: Message, client: YapClient, draft: inout String) -> MessageEdit? {
        guard let text = client.editableText(of: message) else { return nil }
        let edit = MessageEdit(message: message, savedDraft: draft)
        draft = text
        return edit
    }

    /// Send / Save can go: there's text, a staged GIF or picture, or this
    /// edits a message whose GIF or picture stays (its caption may be emptied).
    @MainActor
    static func canSend(draft: String, staged: Bool, editing: MessageEdit?, client: YapClient) -> Bool {
        if staged || !draft.isBlank { return true }
        guard let message = editing?.message else { return false }
        if message.gif != nil || message.image != nil { return true }
        if case .note(let note) = client.openNote(message.body) { return note.gif != nil || note.image != nil }
        return false
    }
}

/// The encrypted Personal space's sheet: turn it on, unlock, change the
/// passcode, or start over (a new key; old notes stay locked).
enum VaultMode: String, Identifiable {
    case setup, unlock, change, reset
    var id: String { rawValue }

    var title: String {
        switch self {
        case .setup: "Encrypt your Personal space"
        case .unlock: "Unlock your Personal space"
        case .change: "Change your passcode"
        case .reset: "Start over?"
        }
    }

    var text: String {
        switch self {
        case .setup:
            "Pick a passcode only you know: a short phrase works best (like \"purple tiger river 42\"), at least 8 characters. Yap can't see it and can't recover it — forget it and your encrypted notes are gone. Each of your devices asks for it once."
        case .unlock:
            "Enter your Personal space passcode. This device remembers it until you lock it."
        case .change:
            "Your notes stay as they are; only the passcode that opens them changes. Your other devices will ask for the new one."
        case .reset:
            "Without the passcode, nobody can open your old encrypted notes — not you, not Yap. Starting over makes a new passcode for new notes; the old ones stay locked."
        }
    }

    var action: String {
        switch self {
        case .setup: "Encrypt"
        case .unlock: "Unlock"
        case .change: "Change"
        case .reset: "Start over"
        }
    }

    var asksAgain: Bool { self != .unlock }
}

/// The sheet's form state and the work it starts.
@MainActor
@Observable
final class VaultForm {
    var mode: VaultMode
    var old = ""
    var pass = ""
    var again = ""
    var error: String?
    var working = false

    init(mode: VaultMode) { self.mode = mode }

    var canSubmit: Bool {
        !working && !pass.isEmpty && (mode != .change || !old.isEmpty) && (!mode.asksAgain || !again.isEmpty)
    }

    /// True when done (the sheet closes).
    func submit(_ client: YapClient) async -> Bool {
        if mode.asksAgain, let problem = Vault.problem(newPasscode: pass, again: again) {
            error = problem
            return false
        }
        working = true
        defer { working = false }
        do {
            switch mode {
            case .unlock: try await client.unlockVault(passcode: pass)
            case .change:
                try await client.changeVaultPasscode(old: old, new: pass)
                client.notice = YapClient.Notice(text: "Passcode changed.")
            case .setup, .reset:
                try await client.setUpVault(passcode: pass)
                client.notice = YapClient.Notice(text: "Your Personal space is encrypted. New notes are sealed with your passcode.")
            }
            return true
        } catch Vault.Failure.wrongPasscode {
            self.error = "That passcode doesn’t open it."
        } catch {
            self.error = "Something went wrong: \(error.localizedDescription)"
        }
        return false
    }
}

/// The Personal space's wording (the web's).
enum PersonalSpace {
    static let intro = "Only you can see this. Send yourself links, pictures and notes to grab on your other devices."
    static let placeholder = "Note to self: a link, a picture, anything"
}

/// The Personal space's menu: encrypt it, or change the passcode / lock it.
struct PersonalSpaceMenuItems: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let client = model.client
        if client.vaultWrap == nil {
            Button("Encrypt it with a passcode…", systemImage: "lock") { model.vaultSheet = .setup }
        } else if client.vaultUnlocked {
            Button("Lock on this device", systemImage: "lock.fill") { client.lockVault() }
            Button("Change passcode…", systemImage: "key") { model.vaultSheet = .change }
        } else {
            Button("Unlock…", systemImage: "lock.open") { model.vaultSheet = .unlock }
        }
    }
}

/// The Personal space's header note and what the start card says.
extension PersonalSpace {
    @MainActor
    static func headerNote(_ client: YapClient) -> String {
        client.vaultWrap == nil ? "only you · not encrypted" : client.vaultUnlocked ? "end-to-end encrypted" : "locked"
    }

    static let encryptedNote = "End-to-end encrypted with your passcode: this device and your others that know it can read these notes; Yap's server and its backups only ever see scrambled text. Forget the passcode and they're gone for good. (Someone holding your unlocked device can still read it.)"

    /// A run of notes this device can't open yet (one row, not one line each).
    static func lockedRun(_ count: Int) -> String {
        count == 1 ? "🔒 An encrypted note. Unlock your Personal space to read it."
                   : "🔒 \(count) encrypted notes. Unlock your Personal space to read them."
    }
    static let notEncryptedWarning = "Not encrypted yet: Yap's server (and its backups) can read it like any other message."
    static let lockedPlaceholder = "Unlock your Personal space to write"
}

extension ImageStore {
    /// An encrypted Personal-space picture: fetched from `/media/<id>`,
    /// opened with `open`, then downsampled.
    func sealedPicture(for url: URL, maxPixel: Int = 900, open: @escaping @MainActor (Data) throws -> Data) -> PlatformImage? {
        let key = "sealed:\(url.absoluteString)#\(maxPixel)"
        if let hit = pictures[key] { return hit }
        loadSealedPicture(url, key: key, maxPixel: maxPixel, open: open)
        return nil
    }
}

/// When's good (the weekly planner) and Watch together are web-only for
/// now: open them in the browser.
extension ServerEndpoint {
    var whensGoodURL: URL? { URL(string: inviteLink + "&place=when") }
    var watchURL: URL? { URL(string: inviteLink + "&place=watch") }
    var privacyURL: URL { baseURL.appending(path: "privacy") }
    /// "What's new": the web's changelog page.
    var changelogURL: URL { baseURL.appending(path: "changelog") }
    /// The web app, at `place` if given (`draw`, `space`, `when`, `watch`…).
    /// With a one-time `pass` it opens signed in: the web keeps its own
    /// sign-in, apart from the app's, and would otherwise ask for name + password
    /// and land on chat. The fragment never reaches the server.
    func webURL(place: String? = nil, pass: String? = nil, name: String? = nil) -> URL? {
        var link = inviteLink
        for (key, value) in [("place", place), ("handoff", pass), ("name", name)] {
            guard let value, let encoded = value.addingPercentEncoding(withAllowedCharacters: .alphanumerics) else { continue }
            link += "&\(key)=\(encoded)"
        }
        return URL(string: link)
    }
}

extension YapClient {
    /// The web app signed in as you (PROTOCOL.md › `device.handoff`). A web
    /// app that's already signed in keeps its own sign-in and ignores the pass.
    func signedInWebURL(place: String? = nil) async -> URL? {
        guard let endpoint else { return nil }
        let pass = await handoffPass()
        return endpoint.webURL(place: place, pass: pass, name: me?.name)
    }

    /// A web-only place in the browser, signed in.
    func openWeb(_ place: String? = nil, with openURL: OpenURLAction) {
        Task { if let url = await signedInWebURL(place: place) { openURL(url) } }
    }
}

/// "Jump to this message": open its channel, page back until it's loaded,
/// scroll to it and flash it. Search results, pins and reply quotes use it.
struct JumpTarget: Equatable {
    var channelId: String
    var messageId: String
    let token = UUID()
}

extension AppModel {
    func jump(to message: Message) {
        jump = JumpTarget(channelId: message.channelId, messageId: message.id)
        open(.channel(message.channelId))
    }
}

/// Where a search looks.
enum SearchScope: String, Hashable, CaseIterable {
    case channel, everywhere
    var title: String { self == .channel ? "This Channel" : "Everywhere" }
}

/// A search result's excerpt with the matched words marked (`highlight`
/// styles them).
@MainActor
func searchSnippet(_ message: Message, client: YapClient, query: String, highlight: (inout AttributedString, Range<AttributedString.Index>) -> Void) -> AttributedString {
    let content = MessageContent(message, client: client)
    var text = MessageFormatter.flatten(content.text)
    if text.isEmpty {
        text = content.image != nil || content.sealedImage != nil ? "Picture" : content.gif != nil ? "GIF" : "…"
    }
    let (excerpt, ranges) = SearchQuery(query).snippet(of: text)
    var out = AttributedString(excerpt)
    for range in ranges {
        let lower = excerpt.distance(from: excerpt.startIndex, to: range.lowerBound)
        let length = excerpt.distance(from: range.lowerBound, to: range.upperBound)
        let start = out.index(out.startIndex, offsetByCharacters: lower)
        let end = out.index(start, offsetByCharacters: length)
        highlight(&out, start..<end)
    }
    return out
}

/// "general" / "Personal space" for a result's channel.
@MainActor
func channelLabel(_ channelId: String, client: YapClient) -> String {
    client.channelsById[channelId]?.name ?? "archived"
}

extension View {
    /// VoiceOver: a combined voice row keeps the LIVE badge's action.
    @ViewBuilder
    func liveBadgeAction(_ session: Session) -> some View {
        if session.voice?.screen == true {
            modifier(LiveBadgeAction(session: session))
        } else {
            self
        }
    }
}

private struct LiveBadgeAction: ViewModifier {
    @Environment(AppModel.self) private var model
    var session: Session

    func body(content: Content) -> some View {
        let mine = session.sid == model.client.mySid
        content.accessibilityAction(named: Text(mine ? ScreenText.stopSharing : ScreenText.watch)) {
            model.watchScreen(session)
        }
    }
}

extension MessageContent {
    /// What the reply bar quotes (the web's reply bar): the first words in
    /// plain text, cut with an ellipsis, or what the message is when it has
    /// no words (a picture, a GIF, a note this device can't open).
    var snippet: String {
        let words = MessageFormatter.flatten(text)
        if !words.isEmpty { return words.count > 120 ? String(words.prefix(120)) + "…" : words }
        if sealedNotice != nil { return "🔒 Encrypted note" }
        if image != nil || sealedImage != nil { return "📷 A picture" }
        if gif != nil { return "GIF" }
        return ""
    }
}
