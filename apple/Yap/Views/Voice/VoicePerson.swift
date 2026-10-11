import SwiftUI
import YapCore
import YapVoice
#if os(macOS)
import AppKit
#endif

/// Someone under a voice channel (or in the call sheet): click (Mac), tap
/// (iPhone, iPad), right-click or long-press → a small popover with their
/// volume (0–200 %, the web's), Watch screen while they share, and the
/// owner/mod actions the web shows. Your own row has none of it (an OBS
/// link has no row at all).
struct VoicePersonMenu: ViewModifier {
    @Environment(AppModel.self) private var model
    var session: Session
    @State private var open = false
    @State private var asking: Ask?

    enum Ask: Identifiable {
        case ban, resetPassword
        var id: Self { self }
    }

    /// Who gets the popover: someone else, not an OBS listener.
    static func offered(_ session: Session, mySid: String?, myUid: String?) -> Bool {
        !session.isListener && session.sid != mySid && session.uid != myUid
    }

    func body(content: Content) -> some View {
        let client = model.client
        if !Self.offered(session, mySid: client.mySid, myUid: client.uid) {
            content
        } else {
            let user = client.user(session.uid)
            let name = user?.name ?? "them"
            content
                .contentShape(.rect)
                .onTapGesture { open = true }
                #if os(iOS)
                .onLongPressGesture { open = true }
                #else
                .modifier(RightClickOpens(id: session.sid) { open = true })
                #endif
                .popover(isPresented: $open, arrowEdge: .trailing) {
                    VoicePersonPopover98(session: session) { action in
                        open = false
                        switch action {
                        case .watch: model.watchScreen(session)
                        case .ask(let what):
                            // Asked from the row once the popover is gone (a box can't sit on a popover).
                            Task {
                                try? await Task.sleep(for: .milliseconds(350))
                                asking = what
                            }
                        }
                    }
                    .environment(model)
                    .presentationCompactAdaptation(.popover)
                }
                .question(isPresented: Binding(get: { asking == .ban }, set: { if !$0 { asking = nil } }),
                          .ban(name) { [client, uid = session.uid] in client.ban(uid) })
                .question(isPresented: Binding(get: { asking == .resetPassword }, set: { if !$0 { asking = nil } }),
                          .resetPassword(name) { [client, uid = session.uid] in client.resetPassword(for: uid) })
                .accessibilityAction(named: Text("Volume and more")) { open = true }
                .accessibilityHint("Their volume, and more")
                .onAppear {
                    #if DEBUG
                    if let demo = DebugLaunch.current.person, demo == user?.name {
                        Task {
                            try? await Task.sleep(for: .seconds(1.5))
                            open = true
                        }
                    }
                    #endif
                }
        }
    }
}

extension View {
    /// Volume, Watch screen and mod actions for someone in voice.
    func voicePersonMenu(_ session: Session) -> some View {
        modifier(VoicePersonMenu(session: session))
    }
}

/// The popover itself: name (and role), their away note, Volume, Watch
/// screen, Make mod / Remove mod, Reset password…, Ban — the web's words.
struct VoicePersonPopover98: View {
    enum Action {
        case watch
        case ask(VoicePersonMenu.Ask)
    }

    @Environment(AppModel.self) private var model
    var session: Session
    var act: (Action) -> Void

    var body: some View {
        let client = model.client
        let user = client.user(session.uid)
        let live = client.sessions[session.sid] ?? session
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                Avatar(user: user, size: 20)
                Text(user?.name ?? "Someone").font(Theme.chatBold).foregroundStyle(Theme.text).lineLimit(1)
                if let role = user?.role { RoleBadge(role: role) }
            }
            if let away = user?.awayNow() {
                Label(away.text, systemImage: "cup.and.saucer.fill")
                    .font(Theme.small)
                    .italic()
                    .foregroundStyle(Theme.grayText)
                    .lineLimit(2)
            }
            VolumeRow98(uid: session.uid)
            if live.voice?.screen == true {
                Button(ScreenText.watch, systemImage: "display") { act(.watch) }
                    .buttonStyle(.win98)
                    .labelStyle(.titleAndIcon)
            }
            if let user {
                let actions = Self.modActions(me: client.myRole, them: user.role)
                if !actions.isEmpty {
                    // Wraps when "Remove mod", "Reset password…" and "Ban" don't fit on one line.
                    FlowLayout(spacing: 6) {
                        if let promote = actions.first(where: { $0 == "Make mod" || $0 == "Remove mod" }) {
                            Button(promote) { client.setRole(user.id, mod: user.role != .mod) }
                                .buttonStyle(.win98Compact)
                        }
                        if actions.contains("Ban") {
                            Button(PasswordText.reset) { act(.ask(.resetPassword)) }
                                .buttonStyle(.win98Compact)
                            Button("Ban") { act(.ask(.ban)) }
                                .buttonStyle(.win98Compact)
                        }
                    }
                }
            }
        }
        .padding(10)
        .frame(width: 270, alignment: .leading)
        .background(Theme.face)
        .appScheme()
    }
}

extension VoicePersonPopover98 {
    /// What owner and mods can do to them, in the web's words and order:
    /// the owner makes or removes mods; owner and mods reset passwords and
    /// ban, never the owner.
    static func modActions(me: Role, them: Role) -> [String] {
        var actions: [String] = []
        if me == .owner, them == .member || them == .mod { actions.append(them == .mod ? "Remove mod" : "Make mod") }
        if me.canModerate, them != .owner { actions += [PasswordText.reset, "Ban"] }
        return actions
    }
}

/// "Volume 100%" and a 0–200 % slider in steps of 5 (the web's); past 100 %
/// boosts quiet people (with a limiter). Only on this device.
struct VolumeRow98: View {
    @Environment(AppModel.self) private var model
    var uid: String

    var body: some View {
        let volume = Binding(
            get: { model.voice.settings.volumes[uid] ?? 1 },
            set: { model.voice.setVolume(($0 * 20).rounded() / 20, for: uid) }
        )
        let percent = Int((volume.wrappedValue * 100).rounded())
        VStack(alignment: .leading, spacing: 2) {
            Text("Volume \(percent)%")
                .font(Theme.chrome.monospacedDigit())
                .foregroundStyle(Theme.text)
            Slider(value: volume, in: 0...2, step: 0.05)
                .accessibilityLabel("Volume")
                .accessibilityValue("\(percent)%")
            Text("Past 100% boosts quiet people")
                .font(Theme.small)
                .foregroundStyle(Theme.grayText)
        }
        .help("Past 100% boosts quiet people")
    }
}

#if os(macOS)
/// Right-click (or Control-click) on a row opens its popover too. SwiftUI
/// has no right-click gesture, so one local event monitor sends the click to
/// whichever row the pointer is over (rows say so on hover).
private struct RightClickOpens: ViewModifier {
    var id: String
    var action: () -> Void
    @State private var clicks = RightClicks.shared

    func body(content: Content) -> some View {
        content
            .onHover { inside in
                if inside {
                    clicks.hovered = id
                } else if clicks.hovered == id {
                    clicks.hovered = nil
                }
            }
            .onChange(of: clicks.count) { _, _ in
                if clicks.requested == id { action() }
            }
            .onAppear { clicks.install() }
    }
}

@MainActor
@Observable
final class RightClicks {
    static let shared = RightClicks()
    @ObservationIgnored var hovered: String?
    private(set) var requested: String?
    private(set) var count = 0
    @ObservationIgnored private var monitor: Any?

    func install() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.rightMouseDown, .leftMouseDown]) { event in
            let secondary = event.type == .rightMouseDown || event.modifierFlags.contains(.control)
            guard secondary else { return event }
            let taken = MainActor.assumeIsolated { () -> Bool in
                let clicks = RightClicks.shared
                guard let id = clicks.hovered else { return false }
                clicks.requested = id
                clicks.count += 1
                return true
            }
            return taken ? nil : event
        }
    }
}
#endif
