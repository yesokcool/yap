import SwiftUI
import YapCore
#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// Puts text on the clipboard (the Copy buttons).
func copyToPasteboard(_ text: String) {
    #if os(iOS)
    UIPasteboard.general.string = text
    #else
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(text, forType: .string)
    #endif
}

/// "Ping everyone": always asked first, with an optional note, wherever the
/// button is (the web asks too), in a Win98 message box; each place
/// with a Ping button owns its own `isPresented`.
struct PingPrompt: ViewModifier {
    @Environment(AppModel.self) private var model
    @Binding var isPresented: Bool

    func body(content: Content) -> some View {
        let client = model.client
        // The web's words (its sentence is this app's, since v0.41); the server
        // keeps 140 characters of the note.
        content.question(isPresented: $isPresented, Question(
            title: "Ping everyone?", text: "Everyone with Yap open hears a chime; people with pushes on get a notification.",
            action: "Ping", field: .init(label: "Add a note (optional)", placeholder: "hop on!", maxLength: 140),
            perform: { client.pingEveryone($0) }))
    }
}

/// "Sign out everywhere else" always asks first (the web's wording), then
/// says what to do if the sign-in wasn't you.
struct SignOutOthersPrompt: ViewModifier {
    @Environment(AppModel.self) private var model
    @Binding var isPresented: Bool

    func body(content: Content) -> some View {
        let client = model.client
        content.question(isPresented: $isPresented, Question(
            title: "Sign out everywhere else?", text: "Every other device (and your OBS links) is signed out. You stay signed in here.",
            action: "Sign out", danger: true, perform: { _ in
                client.signOutOthers()
                let secret = client.secretKind == "password" ? "password" : "PIN"
                client.notice = YapClient.Notice(text: "Signed out everywhere else. If it wasn’t you, change your \(secret) now.")
            }))
    }
}

/// The temporary PIN after an owner or mod resets someone's (the web's
/// "Temporary PIN"), in a Win98 box with the PIN to copy. Whichever
/// screen is on top shows it: the person's sheet where Reset PIN was
/// pressed, else People, else the app's root (`active` says which).
struct PinResetNotice: ViewModifier {
    @Environment(AppModel.self) private var model
    var active: Bool
    /// Kept while the box closes, so its words don't blank out.
    @State private var shown: YapClient.PinReset?
    /// A beat after the PIN comes: the "Reset PIN?" box it answers is
    /// usually still closing, and a second box can't open over that.
    @State private var ready = false

    func body(content: Content) -> some View {
        let client = model.client
        content
            .task(id: client.pinReset?.id) {
                ready = false
                guard let reset = client.pinReset else { return }
                shown = reset
                try? await Task.sleep(for: .milliseconds(450))
                ready = true
            }
            .question(isPresented: Binding(get: { active && ready && client.pinReset != nil }, set: { if !$0 { client.pinReset = nil } }), Question(
                title: "Temporary PIN",
                text: shown.map { "Tell \($0.name) to sign in with the PIN \($0.pin) (“\(AuthText.returning)”), then change it in Settings." },
                cancel: nil, value: shown?.pin, copyLabel: "Copy PIN"))
    }
}

extension View {
    /// The temporary PIN after a reset, when this screen is the one on top.
    func pinResetNotice(active: Bool = true) -> some View {
        modifier(PinResetNotice(active: active))
    }

    func pingPrompt(isPresented: Binding<Bool>) -> some View {
        modifier(PingPrompt(isPresented: isPresented))
    }

    func signOutOthersPrompt(isPresented: Binding<Bool>) -> some View {
        modifier(SignOutOthersPrompt(isPresented: isPresented))
    }
}

extension AppModel {
    /// Settings on a given page, from anywhere: the Settings window on the
    /// Mac, the Settings screen on iPhone (closing the People sheet first).
    func showSettings(_ page: SettingsPage) {
        settingsPage = page
        showingPeople = false
        #if os(macOS)
        settingsRequested = true
        #else
        // Settings' list, then the page (back goes to the list, then to where you were).
        var next = path
        if let list = next.firstIndex(of: .settings) { next.removeSubrange((list + 1)...) } else { next.append(.settings) }
        next.append(.settingsPage(page))
        path = next
        #endif
    }
}

#if os(macOS)
/// Opens the Settings window when `showSettings` asks (only views can).
struct SettingsWindowOpener: ViewModifier {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings

    func body(content: Content) -> some View {
        content.onChange(of: model.settingsRequested) { _, requested in
            guard requested else { return }
            model.settingsRequested = false
            openSettings()
        }
    }
}
#endif

/// Sign-on wording, the web's (`syncWelcomeMode`): "I'm new" first.
enum AuthText {
    static let new = "I'm new"
    static let returning = "I have an account"

    /// The waiting room (the web's words; an app needn't be kept open).
    static func waiting(_ name: String?) -> String {
        "Hang tight\(name.map { ", \($0)" } ?? ""). Everyone here just got a ping — this opens by itself as soon as someone lets you in."
    }

    /// Follows "Use a password" (the web's `syncWelcomeMode`).
    static func hint(signup: Bool, password: Bool = false) -> String {
        if signup {
            return password
                ? "Pick a name friends will see and a password of 10 or more characters. It’s how you sign in on your other devices."
                : "Pick a name friends will see and a 6-digit PIN. The PIN is how you sign in on your other devices."
        }
        return "Sign in with your name and \(password ? "password" : "PIN") — same you, same messages, on any device."
    }

    /// Under a wrong PIN or a lockout (the web's second line): who can help.
    static func help(for error: YapClient.AuthError?) -> String? {
        guard let code = error?.code, code == "bad_login" || code == "locked" else { return nil }
        return "Forgot your PIN? The owner or a mod can reset it."
    }
}

/// Account wording (the web's).
enum AccountText {
    @MainActor
    static func howToSignIn(_ client: YapClient) -> String {
        "On another phone or computer, choose “I have an account” and sign in with your name and \(client.secretKind == "password" ? "password" : "PIN")."
    }

    static let signOutTitle = "Sign out on this device?"

    @MainActor
    static func signOutMessage(_ client: YapClient) -> String {
        "You can sign back in with your name and \(client.secretKind == "password" ? "password" : "PIN")."
    }
}
