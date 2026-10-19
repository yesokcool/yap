#if os(iOS)
import ActivityKit
import Foundation
import SwiftUI
import YapCore
import YapVoice

/// iPhone: the voice call on the Lock Screen and in the Dynamic Island, a
/// Live Activity that YapLive draws (`CallActivityViews`). It starts when
/// you join, follows the call about once a second (who's in it, who's
/// talking, mute and deafen) and ends when you leave. Its buttons come back
/// here through `CallButtonIntent`: Mute, Deafen, and the title bar's ×,
/// which asks "Leave Lounge?" first (a Live Activity can't be held down).
@MainActor
final class LiveCall {
    private unowned let model: AppModel
    /// The running activity's id. (`Activity` isn't Sendable: it's looked up
    /// by id wherever it's updated or ended.)
    private var activityId: String?
    private var sent: CallState?
    private var sentAt = Date.distantPast
    private var tickTask: Task<Void, Never>?
    private var askingToLeave = false
    private var askTask: Task<Void, Never>?

    /// Names on the buddy line; the rest are "+2".
    static let shown = 6
    /// How long "Leave Lounge?" waits for an answer.
    static let askFor: Duration = .seconds(5)
    /// Without an update for this long, iOS greys it out ("Yap isn't
    /// answering"): Yap was stopped and the call with it.
    static let staleAfter: TimeInterval = 120
    /// Nothing changed: the same state again this often, to move that date.
    static let refreshEvery: TimeInterval = 60

    init(model: AppModel) {
        self.model = model
        CallButtons.press = { [weak self] button in self?.press(button) }
        // A Yap that's just started isn't in a call: whatever an earlier one left up is over.
        Self.endAll()
    }

    /// Joined or left (and back on screen: iOS starts these only for an app on screen).
    func callChanged() {
        if model.voice.isInVoice { start() } else { end() }
    }

    /// Quitting: take it down (if iOS stops Yap first, it goes stale instead).
    func willTerminate() {
        end()
    }

    // MARK: Doing it

    private func start() {
        guard activityId == nil, ActivityAuthorizationInfo().areActivitiesEnabled, let state = currentState() else { return }
        let content = ActivityContent(state: state, staleDate: Date().addingTimeInterval(Self.staleAfter))
        guard let started = try? Activity.request(attributes: CallActivityAttributes(joinedAt: Date()), content: content) else { return }
        activityId = started.id
        sent = state
        sentAt = Date()
        tickTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                guard let self, !Task.isCancelled else { return }
                self.update()
            }
        }
    }

    private func update() {
        guard let activityId, let state = currentState() else { return }
        let now = Date()
        guard state != sent || now.timeIntervalSince(sentAt) >= Self.refreshEvery else { return }
        sent = state
        sentAt = now
        let content = ActivityContent(state: state, staleDate: now.addingTimeInterval(Self.staleAfter))
        Task { await Self.send(content, to: activityId) }
    }

    private func end() {
        tickTask?.cancel()
        tickTask = nil
        stopAsking()
        guard let activityId else { return }
        self.activityId = nil
        sent = nil
        Task { await Self.end(activityId) }
    }

    private static func endAll() {
        Task { await end(nil) }
    }

    private nonisolated static func send(_ content: ActivityContent<CallState>, to id: String) async {
        await Activity<CallActivityAttributes>.activities.first { $0.id == id }?.update(content)
    }

    /// One activity, or (nil) every one.
    private nonisolated static func end(_ id: String?) async {
        for activity in Activity<CallActivityAttributes>.activities where id == nil || activity.id == id {
            await activity.end(nil, dismissalPolicy: .immediate)
        }
    }

    private func press(_ button: CallButton) {
        // A window left from a call that's over (Yap was stopped): close it.
        guard model.voice.isInVoice else { return Self.endAll() }
        switch button {
        case .mute: model.voice.toggleMute()
        case .deafen: model.voice.toggleDeafen()
        case .askToLeave:
            askingToLeave = true
            askTask?.cancel()
            askTask = Task { [weak self] in
                try? await Task.sleep(for: Self.askFor)
                guard !Task.isCancelled else { return }
                self?.stopAsking()
                self?.update()
            }
        case .stay: stopAsking()
        case .leave:
            stopAsking()
            model.voice.leave()
            return
        }
        update()
    }

    private func stopAsking() {
        askTask?.cancel()
        askTask = nil
        askingToLeave = false
    }

    #if DEBUG
    /// `YAP_DEMO_LIVE_CALL`: a pretend call (Ada is you, Ben is talking),
    /// then 8 s later the same again with an alert, so iOS shows it by itself
    /// (the island opens, or a banner). Its buttons do nothing: no call.
    func demo(_ variant: String) {
        let people = [("Ada", true, false), ("Ben", false, true), ("Cleo", false, false), ("Dot", false, false)]
            .map { CallActivityAttributes.Person(name: $0.0, me: $0.1, talking: $0.2) }
        let appearance = UserDefaults.standard.string(forKey: Appearance.storageKey).flatMap(Appearance.init) ?? .charcoal
        let state = CallState(channel: "Lounge", people: people, more: variant.contains("more") ? 2 : 0,
                              muted: variant.contains("muted"), deafened: variant.contains("deafened"), status: nil,
                              askingToLeave: variant.contains("leaving"), dark: appearance.colorScheme.map { $0 == .dark })
        let staleDate = variant.contains("stale") ? Date().addingTimeInterval(4) : nil
        Task {
            // On screen first: iOS starts these only for an app in front.
            try? await Task.sleep(for: .seconds(1.5))
            let content = ActivityContent(state: state, staleDate: staleDate)
            guard let started = try? Activity.request(attributes: CallActivityAttributes(joinedAt: Date().addingTimeInterval(-754)),
                                                      content: content) else { return }
            let id = started.id
            try? await Task.sleep(for: .seconds(8))
            await Self.alert(content, to: id)
        }
    }

    private nonisolated static func alert(_ content: ActivityContent<CallState>, to id: String) async {
        let alert = AlertConfiguration(title: "Lounge", body: "Ben is talking", sound: .default)
        await Activity<CallActivityAttributes>.activities.first { $0.id == id }?.update(content, alertConfiguration: alert)
    }
    #endif

    private func currentState() -> CallState? {
        let voice = model.voice
        let client = model.client
        guard let channelId = voice.wantedChannelId else { return nil }
        let people = client.voiceRoster(in: channelId).map { session in
            CallActivityAttributes.Person(name: String((client.user(session.uid)?.name ?? "Someone").prefix(24)),
                                          me: session.sid == voice.ownSid,
                                          talking: voice.isSpeaking(session))
        }
        let appearance = UserDefaults.standard.string(forKey: Appearance.storageKey).flatMap(Appearance.init) ?? .charcoal
        return CallState(channel: client.channelsById[channelId]?.name ?? "Voice",
                         people: Array(people.prefix(Self.shown)),
                         more: max(0, people.count - Self.shown),
                         muted: voice.settings.muted || voice.micDenied,
                         deafened: voice.settings.deafened,
                         status: voice.isResuming ? "Reconnecting…" : voice.isConnected ? nil : "Connecting…",
                         askingToLeave: askingToLeave,
                         dark: appearance.colorScheme.map { $0 == .dark })
    }
}
#endif
