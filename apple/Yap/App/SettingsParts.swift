import SwiftUI
import YapCore
#if os(iOS)
import UIKit
#else
import AppKit
#endif

/// Settings lists (Owner & mods › Activity, Invite › Your links) show five
/// rows until "Show all N", like the web's `shortList`: a list with just one
/// more than that shows whole. Each opening of Settings starts short again.
enum ShortList {
    static let first = 5

    static func rows<T>(_ rows: [T], all: Bool) -> [T] {
        all || rows.count <= first + 1 ? rows : Array(rows.prefix(first))
    }

    /// Whether "Show all N" is offered.
    static func isCut(_ count: Int, all: Bool) -> Bool {
        !all && count > first + 1
    }
}

/// Settings › Voice and › Notifications, grouped the web's way (Mic &
/// output · How you talk · Sounds · Advanced; On this device · On your
/// phone (ntfy) · Quiet hours). Only what the app has: no device pickers (iPhone uses the call's output picker, the Mac
/// its system devices), no music mode, RNNoise or OBS links.
enum SettingsText {
    static let micOutput = "Mic & output"
    static let howYouTalk = "How you talk"
    static let sounds = "Sounds"
    static let advanced = "Advanced"
    static let onThisDevice = "On this device"
    static let onYourPhone = "On your phone (ntfy)"
    static let quietHours = "Quiet hours"

    static let joinSounds = "Join & mute sounds"
    static let chime = "Your join chime"
    static let ownClip = "Use my own clip…"
    /// Under the away message (the web's words, and when it ends here). It only
    /// shows in People and to people who mention you — never beside your name.
    static let awayNote98 = "Like AIM: people who mention you get it as a quiet reply, and it shows in People. It ends when you send a message, or at the time you pick."
    static let ownClipNote = "Your own clip: up to 2 seconds; loud ones are turned down."
    static let boardVolume = "Soundboard volume"
    static let boardMute = "Mute the soundboard"
    static let clips = "Soundboard clips"
    static let clipsNote = "Short clips anyone in a voice channel can play for everyone in it: two at most at once, never while you're deafened or from someone you've turned all the way down."

    static let sensitivity = "Sensitivity"
    static let pushToTalkNote = "Push to talk adds a hold-to-talk button to the call. Double-tap it to stay live; tap it again to stop."
    static let quality = "Quality"
    static let qualityNote = "Voice goes straight between you and your friends, so better quality costs the server nothing. Your voice goes out at your setting, and each friend's at theirs."

    #if os(iOS)
    static let deviceNote = "While Yap is open, or in a call in the background, @mentions, pings and hangs show as notifications here, with a chime."
    static let deviceButton = "Notification Settings…"
    #else
    static let deviceNote = "While Yap is open, @mentions, pings and hangs show as notifications on this Mac, with a chime."
    static let deviceButton = "Notifications in System Settings…"
    #endif
    static let pushes = "Phone pushes"
    static let pushesNote = "Via the free ntfy app: install it, subscribe to your private topic, and pings and @mentions buzz that phone even when Yap is closed. Keep the topic to yourself."
    static let pushModes: [(value: String, title: String)] = [("off", "Off"), ("pings", "Pings and @mentions"), ("all", "Everything while I'm away")]
    private static let quietNoteFormat = "No pushes or notification sounds at night (your time zone, %@). Sign-in alerts always come through."
    static let quietOn = "Every night"
    static let quietMentions = "Still let @mentions and replies through"

    static func quietNote(_ zone: String = TimeZone.current.identifier) -> String {
        String(format: quietNoteFormat, zone)
    }

    /// This app's page in the system's notification settings.
    static var deviceSettingsURL: URL? {
        #if os(iOS)
        URL(string: UIApplication.openNotificationSettingsURLString)
        #else
        URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension")
        #endif
    }
}

extension AppModel {
    /// Settings was closed (Mac) or opened afresh: its long lists start short
    /// and Voice › Advanced folds up again.
    func settingsClosed() {
        voiceAdvancedOpen = false
        settingsSession += 1
    }
}

#if os(macOS)
/// Calls `onClose` when the window holding this view closes (the Mac's
/// Settings window, which SwiftUI keeps around between openings).
struct WindowCloseWatcher: NSViewRepresentable {
    var onClose: () -> Void

    func makeNSView(context: Context) -> WatchView {
        let view = WatchView()
        view.onClose = onClose
        return view
    }

    func updateNSView(_ view: WatchView, context: Context) {
        view.onClose = onClose
    }

    final class WatchView: NSView {
        var onClose: (() -> Void)?
        nonisolated(unsafe) private var observer: NSObjectProtocol?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let observer { NotificationCenter.default.removeObserver(observer) }
            observer = nil
            guard let window else { return }
            observer = NotificationCenter.default.addObserver(forName: NSWindow.willCloseNotification, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.onClose?() }
            }
        }

        deinit {
            if let observer { NotificationCenter.default.removeObserver(observer) }
        }
    }
}
#endif
