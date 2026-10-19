import Foundation

/// iPhone: a call while Yap isn't on screen. iOS keeps an app with the
/// `audio` background mode running only while its sound runs; the call's
/// anchor (`LevelMonitor`) keeps it running through the call, but a few
/// things are still outside Yap's hands: a phone call, an alarm or another
/// app can take the sound, the sound may not start again afterwards, and iOS
/// can still stop Yap. When that ends the call, the person should hear it from Yap
/// (a notification "You left Lounge" with the reason; one tap brings them
/// back) instead of finding out from their friends.
///
/// A plain value: the app feeds it what happens (scene, call, audio session,
/// socket, a tick every few seconds) and does what it says. Times are
/// seconds of wall-clock time (a frozen app's monotonic clock may not move).
public struct BackgroundCall: Sendable, Equatable {
    /// Why the call ended, in plain words.
    public enum Reason: String, Sendable, Equatable, CaseIterable {
        /// A phone call, an alarm or another app took the sound and kept it.
        case soundTaken
        /// The sound stopped and wouldn't start again (`VoiceController` kept trying).
        case soundFailed
        /// iOS stopped Yap in the background (we find out afterwards).
        case stopped
        /// No connection to the server for a minute.
        case offline
        /// The voice channel was archived.
        case channelGone

        /// The notification's body (and, after "You left Lounge:", the in-app note).
        public var text: String {
            switch self {
            case .soundTaken: "A phone call or another app took over the sound."
            case .soundFailed: "Yap lost the mic and speaker and couldn't get them back."
            case .stopped: "iOS stopped Yap in the background."
            case .offline: "Yap lost its connection for over a minute."
            case .channelGone: "The channel was archived."
            }
        }

        /// One tap rejoins (nothing to rejoin when the channel is gone).
        public var canRejoin: Bool { self != .channelGone }
    }

    /// What the app should do.
    public enum Effect: Sendable, Equatable {
        /// Keep a "You left <channel>" notification scheduled `frozenAfter`
        /// from now: if Yap is stopped it can't say so, so it says so ahead
        /// of time and keeps pushing it back while it's alive.
        case armNote
        case disarmNote
        /// Ask iOS for a little time (an interruption stopped the sound,
        /// which is what keeps Yap running), and give it back.
        case keepAwake
        case letSleep
        /// Leave the call and say why (`notify`: post the notification;
        /// false when the scheduled one already said it).
        case leave(Reason, notify: Bool)
    }

    /// How long an interruption may hold the sound, or the sound may stay
    /// stopped while `VoiceController` tries to start it again, before we
    /// leave (iOS gives a background task about 30 s).
    public static let soundGrace: TimeInterval = 20
    /// How long without the server before we leave (the call itself keeps
    /// going for 20 s; after that we'd only rejoin as a newcomer, much later).
    public static let offlineGrace: TimeInterval = 60
    /// Ticks come every `tickEvery`; this long without one means Yap was
    /// stopped (and the scheduled note has fired).
    public static let tickEvery: TimeInterval = 5
    public static let frozenAfter: TimeInterval = 40

    public private(set) var inCall = false
    public private(set) var active = true
    public private(set) var interruptedAt: TimeInterval?
    /// The sound couldn't be started again, since (nil once it runs again).
    public private(set) var soundLostAt: TimeInterval?
    public private(set) var offlineSince: TimeInterval?
    /// The last time we were alive in the background (nil: on screen, or not in a call).
    public private(set) var lastTick: TimeInterval?
    public private(set) var awake = false

    public init() {}

    /// Ticks are wanted (in a call, off screen).
    public var watching: Bool { inCall && !active }

    // MARK: Events

    public mutating func scene(active nowActive: Bool, at now: TimeInterval) -> [Effect] {
        guard nowActive != active else { return [] }
        active = nowActive
        guard inCall else { return [] }
        if nowActive {
            // Back on screen: were we stopped in the meantime?
            var effects: [Effect] = []
            if let lastTick, now - lastTick > Self.frozenAfter {
                effects.append(.leave(.stopped, notify: false))
            }
            effects += quiet()
            return effects
        }
        lastTick = now
        var effects: [Effect] = [.armNote]
        if soundStopped, !awake {
            awake = true
            effects.append(.keepAwake)
        }
        return effects
    }

    public mutating func call(_ nowInCall: Bool, at now: TimeInterval) -> [Effect] {
        guard nowInCall != inCall else { return [] }
        inCall = nowInCall
        if !nowInCall {
            interruptedAt = nil
            soundLostAt = nil
            offlineSince = nil
            return quiet()
        }
        guard !active else { return [] }
        lastTick = now
        return [.armNote]
    }

    /// The audio session was interrupted (a phone call, an alarm, Siri, another app).
    public mutating func interruptionBegan(at now: TimeInterval) -> [Effect] {
        interruptedAt = interruptedAt ?? now
        guard watching, !awake else { return [] }
        awake = true
        return [.keepAwake]
    }

    /// The interruption is over: WebRTC takes the sound back. Stay awake
    /// until the next tick, in case it can't.
    public mutating func interruptionEnded(at now: TimeInterval) -> [Effect] {
        interruptedAt = nil
        return []
    }

    /// The sound couldn't be started again (the session wouldn't activate,
    /// the audio unit wouldn't start). Often only for a moment (dictation or
    /// Siri still letting go of the mic): `VoiceController` tries again, and
    /// off screen we stay awake meanwhile and leave only if it's still
    /// stopped after `soundGrace`. On screen the "Your mic is sending
    /// silence" warning covers it.
    public mutating func soundFailed(at now: TimeInterval) -> [Effect] {
        soundLostAt = soundLostAt ?? now
        guard watching, !awake else { return [] }
        awake = true
        return [.keepAwake]
    }

    /// The sound runs again (the mic is measured again).
    public mutating func soundRecovered(at now: TimeInterval) -> [Effect] {
        soundLostAt = nil
        return []
    }

    public mutating func connection(online: Bool, at now: TimeInterval) -> [Effect] {
        if online { offlineSince = nil } else { offlineSince = offlineSince ?? now }
        return []
    }

    /// The call ended without us asking (the channel was archived).
    public mutating func ended(_ reason: Reason, at now: TimeInterval) -> [Effect] {
        guard watching else { return [] }
        return [.leave(reason, notify: true)]
    }

    /// Every `tickEvery` while watching.
    public mutating func tick(at now: TimeInterval) -> [Effect] {
        guard watching else { return [] }
        defer { lastTick = now }
        if let lastTick, now - lastTick > Self.frozenAfter {
            // We were stopped: the scheduled note already said so.
            return [.leave(.stopped, notify: false)]
        }
        if let interruptedAt, now - interruptedAt >= Self.soundGrace {
            return [.leave(.soundTaken, notify: true)]
        }
        if let soundLostAt, now - soundLostAt >= Self.soundGrace {
            return [.leave(.soundFailed, notify: true)]
        }
        if let offlineSince, now - offlineSince >= Self.offlineGrace {
            return [.leave(.offline, notify: true)]
        }
        var effects: [Effect] = [.armNote]
        if awake, !soundStopped {
            awake = false
            effects.append(.letSleep)
        }
        return effects
    }

    /// iOS is about to take back the time it gave (`keepAwake`).
    public mutating func timeRunningOut(at now: TimeInterval) -> [Effect] {
        guard awake else { return [] }
        if watching, interruptedAt != nil { return [.leave(.soundTaken, notify: true)] }
        if watching, soundLostAt != nil { return [.leave(.soundFailed, notify: true)] }
        awake = false
        return [.letSleep]
    }

    /// An interruption holds the sound, or it couldn't be started again.
    private var soundStopped: Bool { interruptedAt != nil || soundLostAt != nil }

    /// Back on screen or out of the call: nothing scheduled, no extra time.
    private mutating func quiet() -> [Effect] {
        lastTick = nil
        var effects: [Effect] = [.disarmNote]
        if awake {
            awake = false
            effects.append(.letSleep)
        }
        return effects
    }
}
