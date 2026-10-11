import Foundation

/// The transmit gate (PROTOCOL.md › Voice › Transmit gate; the web's is
/// `web/js/gate-worklet.js`): voice activity with hysteresis, a 450 ms hold
/// and an adaptive noise floor; push to talk with a 200 ms tail; always on.
///
/// The engine feeds it the mic's level about every 25 ms (all the
/// stasel/WebRTC SDK offers: there's no hook on the audio thread) and turns
/// the mic track on and off from its answer. libwebrtc ramps each switch
/// over 128 samples (~3 ms) itself, so there are no clicks, but the web's
/// 5/15 ms fades and its 40 ms pre-roll can't be done from here.
/// Mute and deafen stay outside: the engine switches the track off for those
/// whatever the gate says.
public struct VoiceGate: Sendable, Equatable {
    public enum Mode: Sendable, Equatable {
        case voiceActivity, pushToTalk, alwaysOn
    }

    public static let hold: TimeInterval = 0.45
    public static let hysteresisDb = 6.0
    public static let pushToTalkTail: TimeInterval = 0.2
    /// The adaptive part: open only this far above the room's noise…
    public static let floorMarginDb = 12.0
    /// …stay open until it's back within this much of it…
    public static let floorCloseMarginDb = 8.0
    /// …but never ask for more than this (a loud room still gets through).
    public static let floorCapDb = -30.0
    /// Noise floor = the quietest reading in each 500 ms, the quietest of the last 8.
    public static let floorBucket: TimeInterval = 0.5
    public static let floorBuckets = 8
    /// Nothing but digital silence for this long: the mic is dead or switched off.
    public static let silenceAfter: TimeInterval = 5

    public var mode: Mode
    /// Sensitivity, dBFS RMS (−50 by default, like every client).
    public var threshold: Double
    public private(set) var isOpen: Bool
    /// The last reading, dBFS.
    public private(set) var level: Double = -120
    /// The room's noise, dBFS (−120 until it's known).
    public private(set) var floor: Double = -120

    private var holdUntil: TimeInterval = -.infinity
    private var pushToTalkDown = false
    private var bucketMin: Double?
    private var bucketTime: TimeInterval = 0
    private var buckets: [Double] = []
    private var silentFor: TimeInterval = 0

    public init(mode: Mode = .voiceActivity, threshold: Double = VoiceSettings.defaultVadThreshold) {
        self.mode = mode
        self.threshold = threshold
        isOpen = mode == .alwaysOn
    }

    /// What voice activity opens at right now: your sensitivity, or higher in a noisy room.
    public var openAt: Double {
        max(threshold, min(floor + Self.floorMarginDb, Self.floorCapDb))
    }

    /// What keeps it open (hysteresis): 6 dB under that, but never down into the room's noise.
    public var closeAt: Double {
        max(threshold - Self.hysteresisDb, min(floor + Self.floorCloseMarginDb, Self.floorCapDb - Self.hysteresisDb))
    }

    /// The mic has sent nothing but digital silence for 5 s.
    public var isSilent: Bool { silentFor >= Self.silenceAfter }

    /// One reading: the mic's RMS level (dBFS) over the last `duration`
    /// seconds of audio, read at `now`. Returns whether the gate is open.
    @discardableResult
    public mutating func update(db: Double, duration: TimeInterval, at now: TimeInterval) -> Bool {
        level = db
        let digitalSilence = db <= Self.silenceDb
        silentFor = digitalSilence ? silentFor + duration : 0
        // Digital silence (a muted device) says nothing about the room.
        if !digitalSilence { bucketMin = min(bucketMin ?? db, db) }
        bucketTime += duration
        if bucketTime >= Self.floorBucket - 1e-6 {
            bucketTime = 0
            if let bucketMin { buckets.append(bucketMin) }
            if buckets.count > Self.floorBuckets { buckets.removeFirst(buckets.count - Self.floorBuckets) }
            bucketMin = nil
            floor = buckets.min() ?? -120
        }
        if mode == .voiceActivity, db > (isOpen ? closeAt : openAt) { holdUntil = now + Self.hold }
        return decide(at: now)
    }

    /// Push to talk pressed or released (the tail starts on release).
    @discardableResult
    public mutating func setPushToTalk(_ down: Bool, at now: TimeInterval) -> Bool {
        if down != pushToTalkDown {
            pushToTalkDown = down
            if !down { holdUntil = now + Self.pushToTalkTail }
        }
        return decide(at: now)
    }

    @discardableResult
    public mutating func setMode(_ mode: Mode, at now: TimeInterval) -> Bool {
        if mode != self.mode {
            self.mode = mode
            holdUntil = -.infinity
        }
        return decide(at: now)
    }

    /// Decide again without a reading (the hold or the push-to-talk tail ends on time).
    @discardableResult
    public mutating func decide(at now: TimeInterval) -> Bool {
        switch mode {
        case .alwaysOn: isOpen = true
        case .pushToTalk: isOpen = pushToTalkDown || now < holdUntil
        case .voiceActivity: isOpen = now < holdUntil
        }
        return isOpen
    }

    // MARK: Levels

    /// What a reading of exact zero is (digital silence).
    public static let silenceDb = -100.0

    /// WebRTC's mic counters (`totalAudioEnergy`, `totalSamplesDuration`)
    /// add up each 10 ms frame's *peak*², not its mean square, so their
    /// "RMS" reads hotter than the real thing: ~8.7 dB on speech, ~10 dB on
    /// noise, 3 dB on a pure tone. This brings it to the web's RMS scale (so
    /// −50 means the same thing everywhere).
    public static let peakToRMSDb = -9.0

    /// RMS over the span between two readings of WebRTC's mic counters
    /// (Σ level² × seconds, and seconds); nil when no audio came in between.
    public static func rms(fromEnergy e0: Double, duration d0: Double, toEnergy e1: Double, duration d1: Double) -> Double? {
        let seconds = d1 - d0
        guard seconds > 0 else { return nil }
        return (max(0, e1 - e0) / seconds).squareRoot()
    }

    /// A linear level in dBFS (−100 for silence).
    public static func dB(_ level: Double) -> Double {
        level > 0 ? 20 * log10(level) : silenceDb
    }

    /// A dBFS level as a linear one.
    public static func linear(_ db: Double) -> Double {
        db <= silenceDb ? 0 : pow(10, db / 20)
    }
}
