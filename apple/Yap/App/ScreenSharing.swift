import SwiftUI
import YapCore
import YapVoice
#if os(macOS)
import AppKit
#endif

/// Screen sharing in the app (PROTOCOL.md › Screen sharing): watching and
/// sharing on iPhone, iPad and Mac (the LIVE badge, or Watch screen on a
/// person; Share screen in the call). `VoiceController` does the work; this
/// opens and closes the viewer, starts a share and holds the words.
extension AppModel {
    /// The viewer is the main pane right now.
    var isShowingScreen: Bool {
        #if os(macOS)
        selection == .screen
        #else
        path.last == .screen
        #endif
    }

    /// Open someone's screen in the main area (joining their voice channel
    /// first if needed). Your own LIVE badge stops your share instead.
    func watchScreen(_ session: Session) {
        if session.sid == client.mySid {
            voice.stopSharing()
            return
        }
        guard session.voice?.screen == true else { return }
        showScreen()
        Task { await voice.watchScreen(session.sid) }
    }

    /// The viewer, big (again, after picture-in-picture).
    func showScreen() {
        #if os(macOS)
        if selection != .screen { beforeScreen = selection }
        selection = .screen
        #else
        if path.last != .screen { path.append(.screen) }
        #endif
    }

    /// "Stop watching", or the viewer closed itself 2 s after they stopped.
    func closeScreen() {
        voice.stopWatching()
        screenPiP.stop()
        if screenFullScreen { setScreenFullScreen(false) }
        #if os(macOS)
        if selection == .screen {
            selection = beforeScreen ?? client.textChannels.first.map { .channel($0.id) }
        }
        beforeScreen = nil
        #else
        path.removeAll { $0 == .screen }
        #endif
    }

    /// The viewer left the main pane: keep watching only while popped out.
    func screenViewerGone() {
        guard !isShowingScreen, !screenPiP.isActive, voice.watch != nil else { return }
        voice.stopWatching()
        if screenFullScreen { setScreenFullScreen(false) }
    }

    func setScreenFullScreen(_ on: Bool) {
        guard screenFullScreen != on else { return }
        screenFullScreen = on
        #if os(macOS)
        // The window itself goes full screen (and comes back).
        if let window = NSApp.keyWindow ?? NSApp.mainWindow, window.styleMask.contains(.fullScreen) != on {
            window.toggleFullScreen(nil)
        }
        #endif
    }

    // MARK: Sharing

    /// Share screen: the system picker, then the share goes to whoever asks.
    /// Mac: ScreenCaptureKit's picker. iPhone and iPad: iOS's "Screen
    /// Broadcast" sheet with Yap selected; the broadcast extension connects
    /// once it starts (`VoiceController.broadcastArrived`).
    func shareScreen() {
        #if os(macOS)
        Task {
            do {
                try await voice.startSharing(MacScreenFeeder())
            } catch is CancellationError {
                // Cancelled the picker: nothing to say.
            } catch {
                client.notice = YapClient.Notice(text: ScreenText.couldNotShare(error))
            }
        }
        #else
        if !BroadcastPicker.open() { client.notice = YapClient.Notice(text: ScreenText.pickerMissing) }
        #endif
    }

    /// "Watching: Ana, Ben" / "No one's watching yet".
    var screenWatchersText: String {
        ScreenAudience.text(names: voice.screenWatchers.map { client.user($0)?.name ?? "Someone" })
    }
}

/// The words (the web's: Share screen, Stop sharing, LIVE, Watch screen,
/// Stop watching, Pop out, Full screen, Smooth / Sharp / Max…).
enum ScreenText {
    static let share = "Share screen"
    static let stopSharing = "Stop sharing"
    static let watch = "Watch screen"
    static let stopWatching = "Stop watching"
    static let popOut = "Pop out"
    static let fullScreen = "Full screen"
    static let exitFullScreen = "Exit full screen"
    static let changeContent = "Share something else"
    static let popOutHelp = "Keep watching in a small window while you use the rest of Yap"
    static let soundHelp = "The sound of their share, only on this device (their voice has its own volume)"
    static let watchHelp = "Watch together: one YouTube player for everyone (opens in the browser)"

    static func title(_ name: String) -> String { "\(name)'s screen" }

    /// The LIVE badge's tooltip / VoiceOver label.
    static func live(name: String, mine: Bool) -> String {
        mine ? "You’re sharing your screen (click to stop)" : "Watch \(name)'s screen"
    }

    /// "Sharing · Smooth"
    static func sharing(_ quality: ScreenQuality) -> String { "Sharing · \(quality.title)" }

    static func couldNotShare(_ error: any Error) -> String {
        "Couldn't share your screen: \(error.localizedDescription)"
    }

    /// Under Share screen: who gets it (and, on iPhone and iPad, what's shared and how it stops).
    static var shareHint: String {
        #if os(iOS)
        "Shares your whole screen, picture only. Only people who click your LIVE badge get it. Stop here, in Control Center or with the red pill."
        #else
        "Only people who click your LIVE badge get the video; nothing is sent while nobody watches."
        #endif
    }

    static let pickerMissing = "Couldn't open screen sharing. Open the call and try again."
}
