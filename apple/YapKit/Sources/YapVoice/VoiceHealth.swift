import Foundation

/// When a connection whose path died tries again (PROTOCOL.md › Voice › ICE
/// restarts): the impolite side restarts ICE after 3.5 s of `disconnected`
/// (at once on `failed`), the polite side after 10 s in case the other app
/// only restarts on `failed`; then every 10 s while it's down. After the
/// third try, the impolite side rebuilds the connection from scratch if the
/// other side can follow (it keeps calls across reconnects).
public struct PathWatch: Sendable, Equatable {
    public enum Step: Sendable, Equatable {
        /// Fetch `/api/ice` again, `setConfiguration`, `restartIce()`.
        case restartIce
        /// Throw the connection away and offer a new one.
        case rebuild
    }

    public static let impoliteWait: TimeInterval = 3.5
    public static let politeWait: TimeInterval = 10
    public static let retryEvery: TimeInterval = 10
    public static let triesBeforeRebuild = 3

    /// ICE restarts since the connection was last up.
    public private(set) var restarts = 0
    /// ICE failed since it was last connected (no route: maybe it needs the relay).
    public private(set) var failed = false

    public init() {}

    /// The connection's state changed: how long until the first try (nil = nothing to do).
    public mutating func stateChanged(_ state: VoiceEngine.PeerState, polite: Bool) -> TimeInterval? {
        switch state {
        case .connected:
            restarts = 0
            failed = false
            return nil
        case .failed:
            failed = true
            return polite ? Self.politeWait : 0
        case .disconnected:
            return polite ? Self.politeWait : Self.impoliteWait
        default:
            return nil
        }
    }

    /// A try is due (the connection is still down): what to do now. The
    /// next try comes `retryEvery` later, unless this one rebuilt.
    public mutating func attempt(polite: Bool, canRebuild: Bool) -> Step {
        restarts += 1
        if restarts > Self.triesBeforeRebuild, !polite, canRebuild { return .rebuild }
        return .restartIce
    }
}

/// "weak connection" (PROTOCOL.md › Voice › Connection hints): more than 3 %
/// of their audio concealed (patched over after lost or late packets) or a
/// round trip over 300 ms, judged every 2 s while they're actually sending
/// (≥ 30 packets/s), never in a connection's first 10 s; shown after three
/// bad readings in a row, gone after three good ones.
public struct LinkQuality: Sendable, Equatable {
    public struct Reading: Sendable, Equatable {
        /// Seconds since the last reading.
        public var seconds: Double
        public var packets: Int
        public var samples: Double
        /// Concealed minus silent-concealed samples.
        public var concealed: Double
        public var rttMs: Double?
        public var connectedFor: TimeInterval

        public init(seconds: Double, packets: Int, samples: Double, concealed: Double, rttMs: Double?, connectedFor: TimeInterval) {
            self.seconds = seconds
            self.packets = packets
            self.samples = samples
            self.concealed = concealed
            self.rttMs = rttMs
            self.connectedFor = connectedFor
        }
    }

    public struct Weak: Sendable, Equatable {
        /// Share of their audio concealed, 0…1.
        public var concealed: Double
        public var rttMs: Double?
    }

    public static let concealedLimit = 0.03
    public static let rttLimitMs = 300.0
    public static let minPacketsPerSecond = 30.0
    public static let settling: TimeInterval = 10
    public static let run = 3

    public private(set) var weak: Weak?
    /// Consecutive bad (+) or good (−) readings.
    private var streak = 0

    public init() {}

    /// One reading (deltas since the last). Returns whether `weak` changed.
    @discardableResult
    public mutating func update(_ r: Reading) -> Bool {
        guard r.seconds > 0, Double(r.packets) / r.seconds >= Self.minPacketsPerSecond, r.connectedFor > Self.settling else { return false }
        let share = r.samples > 0 ? max(0, r.concealed) / r.samples : 0
        let bad = share > Self.concealedLimit || (r.rttMs ?? 0) > Self.rttLimitMs
        streak = bad ? max(1, streak + 1) : min(-1, streak - 1)
        let was = weak != nil
        if streak >= Self.run {
            weak = Weak(concealed: share, rttMs: r.rttMs)
        } else if streak <= -Self.run {
            weak = nil
        } else if weak != nil, bad {
            weak = Weak(concealed: share, rttMs: r.rttMs)
        }
        return was != (weak != nil)
    }

    /// The connection went down or came back: start judging afresh.
    public mutating func reset() {
        weak = nil
        streak = 0
    }
}

/// Someone turned up past 100 %: keep the boost from clipping. The SDK has
/// no per-track processing, and WebRTC applies the gain with plain
/// saturation, so this sets the gain itself from how loud they are: their
/// own RMS level (the RTP audio level, every ~25 ms), allowing for speech
/// peaks ~15 dB above it, keeps the boosted peaks under −1 dBFS. It drops at
/// once and comes back slowly; it never goes under 100 % (that's not a
/// boost).
public struct BoostLimiter: Sendable, Equatable {
    /// Speech peaks this far above its RMS level…
    public static let crestDb = 15.0
    /// …land at most here after the boost.
    public static let ceilingDb = -1.0
    /// How fast the gain comes back after a loud bit, dB per second.
    public static let releaseDbPerSecond = 6.0

    public private(set) var gain: Double = 1
    private var lastWanted: Double?

    public init() {}

    /// `wanted`: the person's volume (1 = 100 %); `level`: their RMS level
    /// (linear, 0…1) right now; `seconds` since the last call.
    public mutating func update(wanted: Double, level: Double, seconds: Double) -> Double {
        defer { lastWanted = wanted }
        guard wanted > 1 else {
            gain = max(0, wanted)
            return gain
        }
        let room = Self.ceilingDb - Self.crestDb - VoiceGate.dB(level)
        let allowed = min(wanted, max(1, VoiceGate.linear(room)))
        if allowed < gain || wanted != lastWanted {
            // Louder: down at once. A new volume: straight there.
            gain = allowed
        } else {
            let up = pow(10, Self.releaseDbPerSecond * max(0, seconds) / 20)
            gain = min(allowed, max(gain, 1) * up)
        }
        return gain
    }
}
