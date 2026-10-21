import Foundation
import Observation
import UserNotifications
import YapCore
import YapVoice

/// The call Yap was last in, kept on the device while you're in it, so a
/// relaunch can offer it back: "You were in Lounge." [Rejoin] [×]. iOS
/// stopping Yap, a crash, quitting mid-call. Never a join by itself.
struct LastCall: Codable, Equatable {
    var channelId: String
    /// When we were last seen in it (refreshed while the call runs).
    var at: Date

    /// How long after the call it's still worth offering.
    static let offerFor: TimeInterval = 10 * 60
    /// How often the time is refreshed during a call.
    static let refreshEvery: TimeInterval = 30

    /// A launch: the call to offer, if it was recent enough.
    static func offer(_ last: LastCall?, now: Date) -> LastCall? {
        guard let last, now.timeIntervalSince(last.at) <= offerFor, now >= last.at.addingTimeInterval(-60) else { return nil }
        return last
    }

    /// The bar shows: we're not in a call and the channel is still a voice channel.
    func shows(inVoice: Bool, channel: Channel?, now: Date) -> Bool {
        !inVoice && channel?.kind == .voice && now.timeIntervalSince(at) <= Self.offerFor
    }

    /// When the bar goes by itself.
    var expires: Date { at.addingTimeInterval(Self.offerFor) }
}

/// Where `LastCall` lives: UserDefaults (it has to outlive the app).
struct LastCallStore {
    var defaults: UserDefaults = .standard
    static let key = "yap.lastCall"

    func load() -> LastCall? {
        guard let data = defaults.data(forKey: Self.key) else { return nil }
        return try? JSONDecoder().decode(LastCall.self, from: data)
    }

    func save(_ call: LastCall) {
        defaults.set(try? JSONEncoder().encode(call), forKey: Self.key)
    }

    func clear() {
        defaults.removeObject(forKey: Self.key)
    }
}

/// The offer's state and the call's record (one per app).
@MainActor
@Observable
final class RejoinOffer {
    /// What the bar offers (nil: no bar).
    private(set) var call: LastCall?
    @ObservationIgnored let store: LastCallStore
    @ObservationIgnored private var refresher: Task<Void, Never>?
    @ObservationIgnored private var expiry: Task<Void, Never>?
    /// Quitting: the record stays for the next launch.
    @ObservationIgnored var terminating = false
    /// The call we're in now (`VoiceController.wantedChannelId`).
    @ObservationIgnored var currentCall: @MainActor () -> String? = { nil }

    init(store: LastCallStore = LastCallStore(), now: Date = Date()) {
        self.store = store
        // A launch takes the record: it's offered once, never on the launch after.
        call = LastCall.offer(store.load(), now: now)
        store.clear()
        armExpiry()
    }

    /// Joined or left a call while Yap runs.
    func callChanged() {
        refresher?.cancel()
        refresher = nil
        guard let channelId = currentCall() else {
            // Left on purpose (Leave, the Live Activity's ×), or the server
            // took us out: nothing to offer next time. Quitting keeps it.
            if !terminating { store.clear() }
            return
        }
        // In a call: no bar, and keep the record fresh while it lasts (the
        // channel too: moving to another one keeps the call going).
        dismiss()
        store.save(LastCall(channelId: channelId, at: Date()))
        refresher = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(LastCall.refreshEvery))
                guard let self, !Task.isCancelled, let id = self.currentCall() else { return }
                self.store.save(LastCall(channelId: id, at: Date()))
            }
        }
    }

    /// × (or the call came back some other way).
    func dismiss() {
        call = nil
        expiry?.cancel()
        expiry = nil
    }

    /// Signed out: nothing of that account's calls stays.
    func forget() {
        dismiss()
        refresher?.cancel()
        refresher = nil
        store.clear()
    }

    #if DEBUG
    /// `YAP_DEMO_REJOIN`: the bar as after a relaunch mid-call.
    func offerForDemo(_ demo: LastCall) {
        call = demo
        armExpiry()
    }
    #endif

    private func armExpiry() {
        guard let call else { return }
        expiry = Task { [weak self] in
            try? await Task.sleep(for: .seconds(max(0, call.expires.timeIntervalSinceNow)))
            guard !Task.isCancelled else { return }
            self?.dismiss()
        }
    }
}

extension AppModel {
    /// The offer, unless iOS's "You left …" note already said it. A note that
    /// came while Yap was stopped (delivered, tap to rejoin) covers it: no
    /// bar. One still waiting to come (Yap was stopped less than 40 s ago)
    /// isn't needed now Yap is back on screen: the bar says it instead.
    func settleRejoinOffer() {
        #if os(iOS)
        guard rejoinOffer.call != nil else { return }
        let center = UNUserNotificationCenter.current()
        center.getDeliveredNotifications { delivered in
            let covered = delivered.contains { [Notifier.callNoteId, Notifier.callEndedId].contains($0.request.identifier) }
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    if covered {
                        self.rejoinOffer.dismiss()
                    } else {
                        self.notifier.disarmCallNote()
                    }
                }
            }
        }
        #endif
    }

    /// Rejoin: the same as tapping the channel.
    func rejoinOffered() {
        guard let call = rejoinOffer.call else { return }
        rejoinOffer.dismiss()
        Task { await voice.join(call.channelId) }
    }
}
