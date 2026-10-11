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
                client.notice = YapClient.Notice(text: client.onPin
                    ? "Signed out everywhere else. If it wasn’t you, set a password now."
                    : "Signed out everywhere else. If it wasn’t you, change your password now.")
            }))
    }
}

/// The temporary password after an owner or mod resets someone's, in a
/// Win98 box with the password to copy. Whichever screen is on top shows
/// it: the person's sheet where Reset password… was pressed, else People,
/// else the app's root (`active` says which).
struct PinResetNotice: ViewModifier {
    @Environment(AppModel.self) private var model
    var active: Bool
    /// Kept while the box closes, so its words don't blank out.
    @State private var shown: YapClient.PinReset?
    /// A beat after the password comes: the "Reset password?" box it
    /// answers is usually still closing, and a second box can't open over that.
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
                title: PasswordText.temporaryTitle,
                text: shown.map { PasswordText.resetResult(name: $0.name, password: $0.pin) },
                cancel: nil, value: shown?.pin, copyLabel: PasswordText.copy))
    }
}

extension View {
    /// The temporary password after a reset, when this screen is the one on top.
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

    /// The waiting room: only the owner and mods let people in (they get a
    /// push); an app needn't be kept open.
    static func waiting(_ name: String?) -> String {
        "Hang tight\(name.map { ", \($0)" } ?? ""). \(waitingLine) This opens by itself as soon as they do."
    }

    /// The newcomer's line (the server's `pending` message says the same).
    static let waitingLine = "Waiting for the owner or a mod to let you in."

    static func hint(signup: Bool) -> String {
        signup
            ? "Pick a name friends will see and a password. It’s how you sign in on your other devices."
            : "Sign in with your name and password — same you, same messages, on any device."
    }

    /// Under a wrong password or a lockout: who can help. With the server's
    /// line above it, this reads as the web's: "Wrong name or password. An
    /// old PIN works here too. Forgot it? The owner or a mod can reset it."
    /// Someone still on an old PIN only sees a Password field, so a wrong
    /// try says the PIN works there.
    static func help(for error: YapClient.AuthError?) -> String? {
        switch error?.code {
        case "bad_login": "An old PIN works here too. Forgot it? The owner or a mod can reset it."
        case "locked": "Forgot your password? The owner or a mod can reset it."
        default: nil
        }
    }
}

/// Password wording, the same in every client (PROTOCOL.md › Identity).
/// No "PIN" anywhere, except "Current password or PIN" (and the prompt's
/// "Your PIN still works…") for someone still on one.
enum PasswordText {
    static let field = "Password"
    /// Under a new password's field.
    static let rule = "8 characters or more."
    static let show = "Show password"

    /// Settings › Account.
    static func change(onPin: Bool) -> String { onPin ? "Set a password" : "Change password" }
    static func current(onPin: Bool) -> String { onPin ? "Current password or PIN" : "Current password" }
    static let new = "New password"
    static func saved(onPin: Bool) -> String {
        onPin ? "Password set. Sign in anywhere with your name and this password."
              : "Password changed. Sign in anywhere with your name and this password."
    }

    /// The once-per-device prompt (`ready.weakPin`).
    static let promptTitle = "Choose a password"
    static func prompt(_ nudge: YapClient.PasswordNudge) -> String {
        switch nudge {
        case .pin: "Yap uses passwords now. Your PIN still works until you choose one."
        case .temporary: "You signed in with a temporary password. Choose your own."
        case .guessable: "Your password is easy to guess. Choose a stronger one."
        }
    }
    static let choose = "Choose a password"
    static let later = "Later"

    /// Owner and mods.
    static let reset = "Reset password…"
    static let temporaryTitle = "Temporary password"
    static let copy = "Copy password"
    static func resetResult(name: String, password: String) -> String {
        "\(name)’s temporary password: \(password). Send it to \(name); they’ll choose their own after signing in."
    }
}

/// Account wording (the web's).
enum AccountText {
    static let howToSignIn = "On another phone or computer, choose “I have an account” and sign in with your name and password."

    static let signOutTitle = "Sign out on this device?"

    static let signOutMessage = "You can sign back in with your name and password."
}
