import Foundation

// Round 10's small rules around talking (PROTOCOL.md › Voice › Transmit
// gate, and › Connection hints › Shared media, for yourself): pure values the
// engine and the controller feed, so each is tested on a hand-cranked clock.

/// "You're muted": while you're muted, talking — the gate *would* open on
/// the level measured before the mute, or push to talk is pressed — gets a
/// soft blip and "You're muted" by the Mute button, at most once every 5 s.
public struct MutedNudge: Sendable, Equatable {
    public static let every: TimeInterval = 5
    /// How long "You're muted" stays by the button.
    public static let shownFor: TimeInterval = 2.5

    private var lastAt: TimeInterval = -.infinity

    public init() {}

    /// Talking while muted, right now: is a nudge due?
    public mutating func talking(at now: TimeInterval) -> Bool {
        guard now - lastAt >= Self.every else { return false }
        lastAt = now
        return true
    }
}

/// Push to talk's double-tap: pressed twice within 400 ms it latches open
/// ("Live — tap to stop" by the button); the next press ends the latch, and
/// push to talk closes when that press lets go (after its tail), as any
/// press does. That press doesn't start a new double-tap.
public struct PushToTalkLatch: Sendable, Equatable {
    public static let doubleTap: TimeInterval = 0.4

    /// The key or button is held right now.
    public private(set) var held = false
    /// Double-tapped: open until the next press.
    public private(set) var latched = false
    private var lastPress: TimeInterval = -.infinity
    /// The last press ended a latch, so it can't be the first of a double-tap.
    private var lastPressUnlatched = false

    public init() {}

    /// What the gate sees: held, or latched open.
    public var isDown: Bool { held || latched }

    public mutating func press(at now: TimeInterval) {
        guard !held else { return }
        held = true
        if latched {
            latched = false
            lastPressUnlatched = true
        } else {
            if now - lastPress <= Self.doubleTap, !lastPressUnlatched { latched = true }
            lastPressUnlatched = false
        }
        lastPress = now
    }

    public mutating func release() {
        held = false
    }

    /// A new mode, leaving the call: nothing held, nothing latched.
    public mutating func reset() {
        self = PushToTalkLatch()
    }
}

/// "Too loud (clipping)" on the mic meter: peaks at or over −1 dBFS again
/// and again (3 in 2 s; readings closer than 120 ms are one peak, which is
/// how long WebRTC holds its `audioLevel`), shown until 3 s after the last
/// time that held (the web's numbers).
public struct ClipWatch: Sendable, Equatable {
    public static let clipDb = -1.0
    public static let window: TimeInterval = 2
    public static let hitsNeeded = 3
    public static let hold: TimeInterval = 3
    public static let minGap: TimeInterval = 0.12

    public private(set) var tooLoud = false
    private var hits: [TimeInterval] = []
    private var until: TimeInterval = -.infinity

    public init() {}

    /// One reading: the loudest sample since the last one, in dBFS.
    @discardableResult
    public mutating func update(peakDb: Double, at now: TimeInterval) -> Bool {
        if peakDb >= Self.clipDb, now - (hits.last ?? -.infinity) >= Self.minGap { hits.append(now) }
        hits.removeAll { now - $0 > Self.window }
        if hits.count >= Self.hitsNeeded { until = now + Self.hold }
        tooLoud = now < until
        return tooLoud
    }

    public mutating func reset() {
        self = ClipWatch()
    }

    /// The loudest sample of a buffer, in dBFS (−100 for silence).
    public static func peakDb<C: Collection>(_ samples: C) -> Double where C.Element == Float {
        let peak = samples.reduce(Float(0)) { max($0, abs($1)) }
        return VoiceGate.dB(Double(peak))
    }
}

/// "Lower shared media while friends talk": while anyone else in the call
/// speaks, shared media (a screen share's sound) eases to 50 % of your
/// volume over ~150 ms, and back over ~600 ms once nobody has spoken for
/// 1 s. A factor on top of your volume: the saved volume never changes.
public struct MediaDip: Sendable, Equatable {
    public static let level = 0.5
    public static let down: TimeInterval = 0.15
    public static let up: TimeInterval = 0.6
    public static let quietFor: TimeInterval = 1

    /// 0.5…1, times your volume.
    public private(set) var factor = 1.0
    private var lastSpoke: TimeInterval = -.infinity
    private var lastAt: TimeInterval?

    public init() {}

    /// `on`: the setting; `othersSpeaking`: anyone else in the call right now.
    @discardableResult
    public mutating func update(on: Bool, othersSpeaking: Bool, at now: TimeInterval) -> Double {
        let seconds = lastAt.map { max(0, now - $0) } ?? 0
        lastAt = now
        // Switched off: back up the same way (the web's too).
        if on, othersSpeaking { lastSpoke = now }
        let target = on && now - lastSpoke < Self.quietFor ? Self.level : 1
        let span = 1 - Self.level
        if factor > target {
            factor = max(target, factor - span * seconds / Self.down)
        } else if factor < target {
            factor = min(target, factor + span * seconds / Self.up)
        }
        return factor
    }
}
