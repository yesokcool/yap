import SwiftUI
import YapCore
import YapVoice

/// Picks the screen for where the account is: invite → sign on → waiting
/// room → the app. Notices and the temporary password (outside People) live here too.
struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        #if os(iOS)
        let _ = Look.ensureInstalled()
        #endif
        screens
        #if os(iOS)
        .background(StatusBarStyler(style: Look.statusBarStyle))
        #endif
        #if os(macOS)
        .modifier(SettingsWindowOpener())
        #endif
        .onChange(of: scenePhase, initial: true) { _, phase in model.scenePhaseChanged(phase) }
        .onChange(of: model.client.phase, initial: true) { _, phase in model.phaseChanged(phase) }
        // People (and the person's sheet in it) show it themselves.
        .pinResetNotice(active: !model.showingPeople)
    }

    /// Invite → sign on → waiting room → the app.
    @ViewBuilder private var screens: some View {
        Group {
            switch model.client.phase {
            case .needsInvite:
                InviteView()
            case .signedOut:
                AuthView()
            case .waiting(let message):
                WaitingRoomView(message: message)
            case .blocked(let title, let message):
                BlockedView(title: title, message: message)
            case .connecting, .online, .reconnecting:
                MainView()
            }
        }
        .overlay(alignment: .top) { NoticeBanner() }
        .environment(\.appColorScheme, colorScheme)
        // A link to a place or channel here opens in the app, wherever it's tapped.
        .modifier(OpensYapLinks())
    }
}

/// Transient one-liners from the server ("Ben let in Cleo.", pings, errors),
/// as a tooltip-yellow note.
struct NoticeBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let notice = model.client.notice {
            Tooltip98(text: notice.text)
                .padding(.top, 6)
                .padding(.horizontal, 24)
                .transition(.opacity)
                .id(notice.id)
                .task(id: notice.id) {
                    try? await Task.sleep(for: .seconds(4))
                    if model.client.notice?.id == notice.id {
                        withAnimation { model.client.notice = nil }
                    }
                }
                .onTapGesture { withAnimation { model.client.notice = nil } }
                .accessibilityAddTraits(.isStaticText)
        }
    }
}

/// The main window's status bar: connection, voice, Minecraft.
struct StatusBar98: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 2) {
            // No "online" light: the connection only shows while it's not there.
            if let connection {
                StatusPanel(text: connection.text, systemImage: connection.icon, color: connection.color)
            }
            StatusPanel(text: voice, systemImage: "speaker.wave.2.fill", color: model.voice.isConnected ? Theme.ok : Theme.grayText)
            if let minecraft {
                StatusPanel(text: minecraft.text, systemImage: "cube.fill", color: minecraft.color)
            }
        }
        .padding(.horizontal, 2)
        .padding(.vertical, 2)
        .background(Theme.face)
        .accessibilityElement(children: .combine)
    }

    private var connection: (text: String, icon: String, color: Color)? {
        switch model.client.phase {
        case .online: nil
        case .reconnecting: ("Reconnecting…", "circle.dotted", Theme.warn)
        default: ("Connecting…", "circle.dotted", Theme.warn)
        }
    }

    private var voice: String {
        let voice = model.voice
        guard let id = voice.wantedChannelId, let channel = model.client.channelsById[id] else { return "Not in voice" }
        let people = model.client.voiceRoster(in: id).count
        if voice.isResuming { return "Voice: reconnecting…" }
        return voice.isConnected ? "Voice: \(channel.name) (\(people))" : "Voice: connecting…"
    }

    private var minecraft: (text: String, color: Color)? {
        guard model.config?.minecraft != nil || model.minecraft != nil else { return nil }
        guard let status = model.minecraft else { return ("Minecraft: checking…", Theme.grayText) }
        guard status.online else { return ("Minecraft: offline", Theme.grayText) }
        let players = status.players.map { "\($0.online)/\($0.max)" } ?? "up"
        return ("Minecraft: \(players)", Theme.ok)
    }
}
