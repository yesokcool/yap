import SwiftUI
import YapCore
#if os(iOS)
import AVFoundation
#endif

@main
struct YapApp: App {
    @State private var model: AppModel
    @AppStorage(Appearance.storageKey) private var appearance: Appearance = .charcoal

    init() {
        #if os(iOS)
        // GIF videos are muted and must never stop someone's music; voice
        // switches the session to playAndRecord itself when a call starts.
        try? AVAudioSession.sharedInstance().setCategory(.ambient, options: [.mixWithOthers])
        #endif
        // One look now: a skin stored by an earlier version (Apple, Arch) is forgotten.
        Look.settle()
        _model = State(initialValue: AppModel())
        #if os(iOS)
        Look.ensureInstalled()
        #endif
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(model)
                .preferredColorScheme(appearance.colorScheme)
                .tint(Theme.accent)
                .task { model.start() }
        }
        .commands {
            // ⌘K everywhere (Mac, and iPad with a keyboard).
            CommandMenu("Go") {
                Button("Jump to…") { model.showingSwitcher = true }
                    .keyboardShortcut("k")
                // The web's search shortcut (⌘F also searches the open channel).
                Button("Search messages…") { model.searchMessages() }
                    .keyboardShortcut("f", modifiers: [.command, .shift])
                Button("Catch up") {
                    model.showingCatchUp = true
                }
                .keyboardShortcut("u", modifiers: [.command, .shift])
                Button("Plan a hang…") { model.planningHang = true }
            }
        }
        #if os(macOS)
        .defaultSize(width: 1100, height: 720)
        .commands {
            CommandGroup(replacing: .newItem) {}
        }
        #endif

        #if os(macOS)
        Settings {
            SettingsView()
                .frame(minWidth: 520, minHeight: 520)
                // Each opening starts fresh: long lists short, Voice › Advanced folded.
                .id(model.settingsSession)
                .background(WindowCloseWatcher { model.settingsClosed() })
                .environment(model)
                .preferredColorScheme(appearance.colorScheme)
                .tint(Theme.accent)
        }
        #endif
    }
}
