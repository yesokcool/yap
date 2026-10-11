import SwiftUI
import YapCore
import YapVoice

/// iPhone/iPad: two tabs (Chat, Buddies) with the call riding in the tab
/// bar's bottom accessory. Mac: one window of three panes (channels, chat,
/// buddy list) over a status bar.
struct MainView: View {
    var body: some View {
        #if os(macOS)
        MacRootView()
        #else
        MainTabView()
        #endif
    }
}

/// The main pane for a channel-list destination.
struct DestinationView: View {
    @Environment(AppModel.self) private var model
    var destination: Destination

    var body: some View {
        switch destination {
        case .channel(let id): ChannelView(model: model, channelId: id).id(id)
        case .web(let page): WebPageView(page: page)
        case .screen: ScreenViewer98()
        case .settings: SettingsView()
        case .settingsPage(let page):
            #if os(iOS)
            SettingsPageView98(page: page)
            #else
            SettingsView()
            #endif
        case .voiceAdvanced:
            #if os(iOS)
            VoiceAdvancedPage98()
            #else
            SettingsView()
            #endif
        }
    }
}

#if os(iOS)
/// iPhone: one navigation stack (channels → chat). People is a sheet from
/// the channel list's People button; the call rides at the top of the
/// channel list and of each chat.
struct MainTabView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        NavigationStack(path: $model.path) {
            ChannelListView()
                .navigationDestination(for: Destination.self) { DestinationView(destination: $0) }
        }
        .sheet(isPresented: $model.showingVoicePanel) {
            VoicePanel()
        }
        .sheet(isPresented: $model.showingPeople) {
            MembersView()
        }
        .sheet(item: $model.vaultSheet) { mode in
            VaultSheet98(mode: mode)
        }
        .sheet(isPresented: $model.changingPin) {
            ChangePinView()
        }
        .modifier(ExtrasSheets98())
        .onChange(of: model.voice.isConnected) { _, connected in
            if connected, DebugLaunch.current.wants("voice") { model.showingVoicePanel = true }
        }
    }
}

/// In a chat or a place during a call: the call bar under the title bar.
struct InCallBar98: ViewModifier {
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        content.safeAreaInset(edge: .top, spacing: 0) {
            if model.voice.isInVoice {
                VoiceBar()
                    .padding(.vertical, 5)
                    .bevel(.raised)
            }
        }
    }
}
#endif

#if os(macOS)
/// The Mac window: channels | chat, each a Win98 pane with a navy title
/// bar, over a status bar. People opens as its own window (sheet).
struct MacRootView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        @Bindable var model = model
        // Watching a screen full screen: the picture and nothing else.
        let screenOnly = model.screenFullScreen && model.selection == .screen
        VStack(spacing: 0) {
            HSplitView {
                if !screenOnly {
                    VStack(spacing: 3) {
                        // Your nameplate and the call at the top, like the web.
                        AccountNotices98()
                        HangNotice98()
                        UserPanel()
                        if model.voice.isInVoice { VoiceBar() }
                        SharingStrip98()
                        ChannelListView()
                    }
                    .padding(3)
                    .frame(minWidth: 210, idealWidth: 240, maxWidth: 340)
                }

                Group {
                    if let selection = model.selection {
                        DestinationView(destination: selection)
                            .id(selection)
                    } else {
                        Window98(title: "Yap") {
                            Text("Pick a channel on the left, or click a voice channel to hop in.")
                                .font(Theme.chrome)
                                .foregroundStyle(Theme.grayText)
                                .frame(maxWidth: .infinity, maxHeight: .infinity)
                                .well()
                        }
                    }
                }
                .padding(screenOnly ? 0 : 3)
                .frame(minWidth: 380, maxWidth: .infinity)
            }
            if !screenOnly { StatusBar98() }
        }
        .background(screenOnly ? Color.black : Theme.face)
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didExitFullScreenNotification)) { _ in
            // Esc or the green button: back to the normal window.
            if model.screenFullScreen { model.screenFullScreen = false }
        }
        .sheet(isPresented: $model.showingVoicePanel) {
            VoicePanel().frame(minWidth: 420, minHeight: 480)
        }
        .sheet(isPresented: $model.showingPeople) {
            MembersView()
        }
        .sheet(item: $model.vaultSheet) { mode in
            VaultSheet98(mode: mode)
        }
        .sheet(isPresented: $model.changingPin) {
            ChangePinView()
        }
        .modifier(ExtrasSheets98())
        .task {
            #if DEBUG
            if DebugLaunch.current.wantsSettings {
                try? await Task.sleep(for: .seconds(2))
                openSettings()
            }
            #endif
        }
        .onChange(of: model.client.textChannels.map(\.id), initial: true) { _, ids in
            if model.selection == nil, let first = ids.first(where: { $0 == "general" }) ?? ids.first {
                model.selection = .channel(first)
            }
        }
    }
}

#endif

/// "People" with a badge for anyone waiting: a Win98 button (title bar on
/// the Mac, the navigation bar on iPhone).
struct PeopleButton98: View {
    @Environment(AppModel.self) private var model
    var titleBar = false

    var body: some View {
        let waiting = model.client.waitingRoom.count
        Button { model.showingPeople = true } label: {
            if titleBar {
                // A title bar button is too small to hang a badge off: the count sits beside the icon.
                HStack(spacing: 2) {
                    Image(systemName: "person.2.fill")
                        .font(.system(size: 9, weight: .semibold))
                    if waiting > 0 {
                        Text("\(waiting)")
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(.white)
                            .padding(.horizontal, 2)
                            .background(Theme.danger)
                            .accessibilityHidden(true)
                    }
                }
            } else {
                Image(systemName: "person.2.fill")
                    .font(.system(size: 12, weight: .semibold))
                    .frame(minWidth: 18)
                    .overlay(alignment: .topTrailing) {
                        if waiting > 0 {
                            Text("\(waiting)")
                                .font(.system(size: 9, weight: .bold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 3)
                                .background(Theme.danger)
                                .offset(x: 9, y: -8)
                                .accessibilityHidden(true)
                        }
                    }
            }
        }
        .accessibilityLabel(waiting > 0 ? "People, \(waiting) waiting" : "People")
        .help("Everyone here, and the waiting room")
    }
}
