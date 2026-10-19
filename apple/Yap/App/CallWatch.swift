import Foundation
import UserNotifications
import YapCore
import YapVoice
#if os(iOS)
import UIKit
#endif

/// iPhone: a call while Yap is off screen (`BackgroundCall` has the rules).
/// The call's anchor keeps Yap running in the background; this covers what
/// Yap can't prevent: an interruption that keeps the sound (a phone call, an
/// alarm), sound that won't start again, a long loss of connection, iOS
/// stopping Yap anyway. Then we leave the call for real (nobody keeps
/// talking to a frozen phone, and Yap never pops back into a call hours
/// later) and say so: "You left Lounge", the reason, one tap to rejoin.
/// The Mac is never stopped for going quiet: nothing to do there.
@MainActor
final class CallWatch {
    private var rules = BackgroundCall()
    private unowned let model: AppModel
    private var tickTask: Task<Void, Never>?
    /// The call the notes name (kept a moment after leaving, for the note).
    private var channelId: String?
    /// "You left Lounge: …", shown in Yap the next time it's on screen.
    private(set) var pendingNotice: String?
    #if os(iOS)
    private var backgroundTask: UIBackgroundTaskIdentifier = .invalid
    #endif

    init(model: AppModel) {
        self.model = model
    }

    /// Wall-clock seconds (a stopped app's monotonic clock may not have moved).
    private var now: TimeInterval { Date().timeIntervalSince1970 }

    private static var enabled: Bool {
        #if os(iOS)
        true
        #else
        false
        #endif
    }

    // MARK: What happens

    func scene(active: Bool) {
        guard Self.enabled else { return }
        apply(rules.scene(active: active, at: now))
        if active, let pendingNotice {
            self.pendingNotice = nil
            model.client.notice = YapClient.Notice(text: pendingNotice)
        }
    }

    func callChanged() {
        guard Self.enabled else { return }
        let voice = model.voice
        if let id = voice.wantedChannelId { channelId = id }
        if voice.isInVoice { model.notifier.clearCallNotes() }
        apply(rules.call(voice.isInVoice, at: now))
    }

    func audio(_ event: AudioSessionEvent) {
        guard Self.enabled else { return }
        switch event {
        case .interruptionBegan: apply(rules.interruptionBegan(at: now))
        case .interruptionEnded: apply(rules.interruptionEnded(at: now))
        case .failed: apply(rules.soundFailed(at: now))
        case .recovered: apply(rules.soundRecovered(at: now))
        }
    }

    func connection(online: Bool) {
        guard Self.enabled else { return }
        apply(rules.connection(online: online, at: now))
    }

    /// The server ended the call (the channel was archived).
    func endedByServer() {
        guard Self.enabled else { return }
        apply(rules.ended(.channelGone, at: now))
    }

    /// Quitting (swiped away): no note about a call we're leaving on purpose.
    func willTerminate() {
        model.notifier.disarmCallNote()
    }

    #if DEBUG
    /// `YAP_DEMO_CALL_ENDED`: what you'd see coming back after `reason` ended the call off screen.
    func demoEnd(_ reason: BackgroundCall.Reason) {
        if let id = model.voice.wantedChannelId { channelId = id }
        leave(reason, notify: false)
        scene(active: true)
    }
    #endif

    // MARK: Doing it

    private func apply(_ effects: [BackgroundCall.Effect]) {
        for effect in effects {
            switch effect {
            case .armNote:
                if let content = noteContent(.stopped) {
                    model.notifier.armCallNote(content, after: BackgroundCall.frozenAfter)
                }
            case .disarmNote:
                model.notifier.disarmCallNote()
            case .keepAwake:
                keepAwake()
            case .letSleep:
                letSleep()
            case let .leave(reason, notify):
                leave(reason, notify: notify)
            }
        }
        updateTicking()
    }

    private func noteContent(_ reason: BackgroundCall.Reason) -> UNMutableNotificationContent? {
        guard let channelId else { return nil }
        let name = model.client.channelsById[channelId]?.name ?? "the call"
        return Notifier.callEndedContent(channel: name, channelId: channelId, reason: reason,
                                         quiet: model.client.quietHolds(mention: false))
    }

    private func leave(_ reason: BackgroundCall.Reason, notify: Bool) {
        let id = channelId
        let name = id.flatMap { model.client.channelsById[$0]?.name } ?? "the call"
        if notify, let content = noteContent(reason) { model.notifier.postCallEnded(content) }
        pendingNotice = "You left \(name): \(reason.text)"
        model.voice.noteEnding(reason.text)
        if model.voice.isInVoice { model.voice.leave() }
        // (leave() comes back through callChanged: nothing scheduled any more.)
        apply(rules.call(false, at: now))
    }

    private func keepAwake() {
        #if os(iOS)
        guard backgroundTask == .invalid else { return }
        backgroundTask = UIApplication.shared.beginBackgroundTask(withName: "Yap call") { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.apply(self.rules.timeRunningOut(at: self.now))
                self.letSleep()
            }
        }
        #endif
    }

    private func letSleep() {
        #if os(iOS)
        guard backgroundTask != .invalid else { return }
        UIApplication.shared.endBackgroundTask(backgroundTask)
        backgroundTask = .invalid
        #endif
    }

    private func updateTicking() {
        if rules.watching {
            guard tickTask == nil else { return }
            tickTask = Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(BackgroundCall.tickEvery))
                    guard let self, !Task.isCancelled else { return }
                    // (SwiftUI may not pass on changes while off screen: look.)
                    self.apply(self.rules.connection(online: self.model.client.phase == .online, at: self.now))
                    self.apply(self.rules.tick(at: self.now))
                }
            }
        } else {
            tickTask?.cancel()
            tickTask = nil
        }
    }
}
