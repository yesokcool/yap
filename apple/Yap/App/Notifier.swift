import AudioToolbox
import Foundation
import UserNotifications
import YapVoice
#if os(macOS)
import AppKit
#endif

/// Local notifications + a chime for @mentions and 📣 pings. Only shown
/// while Yap is in the background (it stays running during a voice call);
/// when Yap is suspended the server's ntfy pushes cover it. Calm: nothing
/// for ordinary messages. Also "You left Lounge" when a call ends while
/// Yap is off screen (`CallWatch`), whose tap brings you back into it.
@MainActor
final class Notifier {
    private let askedKey = "yap.askedNotifications"
    private let taps = NotificationTaps()

    /// A "You left …" notification was tapped: rejoin that voice channel.
    var onRejoin: (@MainActor (String) -> Void)? {
        get { taps.onRejoin }
        set { taps.onRejoin = newValue }
    }

    /// A mention's notification was tapped: open that channel at that message.
    var onOpen: (@MainActor (_ channelId: String, _ messageId: String) -> Void)? {
        get { taps.onOpen }
        set { taps.onOpen = newValue }
    }

    init() {
        // Before launch finishes, so a tap that launched Yap still arrives.
        UNUserNotificationCenter.current().delegate = taps
    }

    /// Ask once, the first time we're in. Never nags again.
    func requestAuthorizationOnce() {
        guard !UserDefaults.standard.bool(forKey: askedKey), !DebugLaunch.current.isScripted else { return }
        UserDefaults.standard.set(true, forKey: askedKey)
        Task {
            _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])
        }
    }

    /// `opens`: the message a tap opens (a mention's).
    func post(title: String, subtitle: String?, body: String, thread: String, opens message: (channelId: String, id: String)? = nil) {
        let content = UNMutableNotificationContent()
        content.title = title
        if let subtitle { content.subtitle = subtitle }
        content.body = String(body.prefix(200))
        content.sound = .default
        content.threadIdentifier = thread
        if let message { content.userInfo = [Self.channelKey: message.channelId, Self.messageKey: message.id] }
        let request = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        Task { try? await UNUserNotificationCenter.current().add(request) }
    }

    /// A short system chime (the "tri-tone"-ish one), respecting the silent switch.
    func chime() {
        #if os(iOS)
        AudioServicesPlaySystemSound(1007)
        #else
        NSSound(named: "Glass")?.play()
        #endif
    }

    // MARK: Calls that end off screen

    /// The note that says a call ended because iOS stopped Yap: scheduled
    /// ahead and pushed back while Yap runs (a stopped app can't say anything).
    nonisolated static let callNoteId = "yap-call-note"
    nonisolated static let callEndedId = "yap-call-ended"
    nonisolated static let rejoinKey = "yapRejoin"
    /// A mention's channel and message (a tap opens them).
    nonisolated static let channelKey = "yapChannel"
    nonisolated static let messageKey = "yapMessage"

    /// "You left Lounge" + why (+ "Tap to rejoin."); silent in quiet hours.
    nonisolated static func callEndedContent(channel: String, channelId: String, reason: BackgroundCall.Reason, quiet: Bool) -> UNMutableNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = "You left \(channel)"
        content.body = reason.canRejoin ? "\(reason.text) Tap to rejoin." : reason.text
        content.sound = quiet ? nil : .default
        content.threadIdentifier = "yap-call"
        if reason.canRejoin { content.userInfo = [rejoinKey: channelId] }
        return content
    }

    func armCallNote(_ content: UNMutableNotificationContent, after seconds: TimeInterval) {
        let trigger = UNTimeIntervalNotificationTrigger(timeInterval: seconds, repeats: false)
        // Same id: each one replaces the last (the time moves on).
        let request = UNNotificationRequest(identifier: Self.callNoteId, content: content, trigger: trigger)
        UNUserNotificationCenter.current().add(request) { _ in }
    }

    func disarmCallNote() {
        UNUserNotificationCenter.current().removePendingNotificationRequests(withIdentifiers: [Self.callNoteId])
    }

    func postCallEnded(_ content: UNMutableNotificationContent) {
        disarmCallNote()
        let request = UNNotificationRequest(identifier: Self.callEndedId, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request) { _ in }
    }

    /// Back in a call: "You left …" notes are old news.
    func clearCallNotes() {
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [Self.callNoteId, Self.callEndedId])
    }
}

/// Taps on Yap's notifications. UserNotifications calls it off the main
/// actor; a rejoin hops back.
///
/// Completion handlers, not the `async` versions: Swift calls an `async`
/// one's handler wherever the method finished (a background thread here),
/// and UIKit aborts when a tap's handler runs off the main thread (build 10
/// crashed on taps, 2026-10-17).
nonisolated final class NotificationTaps: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    /// Read and written on the main thread only.
    nonisolated(unsafe) var onRejoin: (@MainActor (String) -> Void)?
    nonisolated(unsafe) var onOpen: (@MainActor (String, String) -> Void)?

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler completionHandler: @escaping () -> Void) {
        let info = response.actionIdentifier == UNNotificationDefaultActionIdentifier ? response.notification.request.content.userInfo : [:]
        let rejoin = info[Notifier.rejoinKey] as? String
        let channel = info[Notifier.channelKey] as? String
        let message = info[Notifier.messageKey] as? String
        // Called on the main thread only.
        nonisolated(unsafe) let done = completionHandler
        DispatchQueue.main.async { [weak self] in
            if let rejoin { self?.onRejoin?(rejoin) }
            if let channel, let message { self?.onOpen?(channel, message) }
            done()
        }
    }

    /// While Yap is on screen its own notices say it: no banners (as before).
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([])
    }
}
