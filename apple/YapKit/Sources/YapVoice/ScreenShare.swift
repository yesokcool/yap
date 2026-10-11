import Foundation

// Screen sharing's rules that don't touch WebRTC (PROTOCOL.md › Screen
// sharing): the quality presets, what a viewer sees while it connects,
// plays, pauses and ends, and who's watching a share. The engine and the
// controller drive them; the tests crank them by hand.

/// The web's `SCREEN_QUALITY` (web/js/voice.js), same numbers. Bitrate is
/// per viewer: every viewer gets their own stream (a mesh).
public enum ScreenQuality: String, CaseIterable, Codable, Sendable, Identifiable {
    /// Games and video: frame rate first.
    case smooth
    /// Text and code: sharpness first.
    case sharp
    /// Up to 4K at 60 fps, for one or two viewers.
    case max

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .smooth: "Smooth"
        case .sharp: "Sharp"
        case .max: "Max"
        }
    }

    /// The largest picture sent (the share is scaled down to fit, never up).
    public var maxWidth: Int {
        switch self {
        case .smooth: 1920
        case .sharp: 2560
        case .max: 3840
        }
    }

    public var maxHeight: Int {
        switch self {
        case .smooth: 1080
        case .sharp: 1440
        case .max: 2160
        }
    }

    public var fps: Int {
        switch self {
        case .smooth: 60
        case .sharp: 30
        case .max: 60
        }
    }

    /// Per viewer, bps.
    public var bitrate: Int {
        switch self {
        case .smooth: 8_000_000
        case .sharp: 6_000_000
        case .max: 25_000_000
        }
    }

    /// What gives first on a busy network (the web's `degradation`).
    public enum Degradation: Sendable {
        case maintainFramerate, maintainResolution, balanced
    }

    public var degradation: Degradation {
        switch self {
        case .smooth: .maintainFramerate
        case .sharp: .maintainResolution
        case .max: .balanced
        }
    }

    /// A picture of `width`×`height` scaled down to fit this preset, keeping
    /// its shape (even sizes: the encoder works in 2×2 blocks).
    public func fit(width: Int, height: Int) -> (width: Int, height: Int) {
        guard width > 0, height > 0 else { return (maxWidth, maxHeight) }
        let scale = min(1, Double(maxWidth) / Double(width), Double(maxHeight) / Double(height))
        func even(_ value: Double) -> Int { Swift.max(2, Int((value / 2).rounded(.down)) * 2) }
        return (even(Double(width) * scale), even(Double(height) * scale))
    }
}

/// What someone watching a share sees, and when it changes
/// (`Connecting to Ana's screen…` → the picture → `Ana's screen is paused.`
/// after 3 s without a frame → `Ana stopped sharing.` → closes itself 2 s
/// later). A plain value: the controller feeds it frames, the sharer's
/// state and the time.
public struct ScreenWatch: Sendable, Equatable {
    public enum Phase: Sendable, Equatable {
        /// Until the first frame.
        case connecting
        case playing
        /// No frame for `pausedAfter` seconds (they're on a still screen, or the network stalled).
        case paused
        /// They stopped sharing (or left): the viewer closes itself `closesAfter` later.
        case stopped
        /// Time to close the viewer.
        case closed
    }

    public static let pausedAfter: TimeInterval = 3
    public static let closesAfter: TimeInterval = 2

    /// The sharer's session (moves when they resume under a new one).
    public var sid: String
    public var uid: String
    public private(set) var phase: Phase = .connecting
    /// When the last frame came (nil: none yet).
    public private(set) var lastFrameAt: TimeInterval?
    public private(set) var stoppedAt: TimeInterval?
    /// Decoded frames so far.
    public private(set) var frames = 0

    public init(sid: String, uid: String) {
        self.sid = sid
        self.uid = uid
    }

    /// A frame was shown.
    public mutating func frame(at now: TimeInterval) {
        frames(frames + 1, lastAt: now)
    }

    /// The renderer's counters, polled: frames shown so far and when the last one came.
    public mutating func frames(_ count: Int, lastAt: TimeInterval) {
        guard count > frames else { return }
        frames = count
        lastFrameAt = lastAt
        if phase == .connecting || phase == .paused { phase = .playing }
    }

    /// They stopped sharing, or left the call.
    public mutating func sharerStopped(at now: TimeInterval) {
        guard phase != .stopped, phase != .closed else { return }
        phase = .stopped
        stoppedAt = now
    }

    /// Moves on with time: paused after 3 s without a frame, closed 2 s after stopping.
    @discardableResult
    public mutating func tick(at now: TimeInterval) -> Phase {
        switch phase {
        case .playing:
            if let lastFrameAt, now - lastFrameAt >= Self.pausedAfter { phase = .paused }
        case .stopped:
            if let stoppedAt, now - stoppedAt >= Self.closesAfter { phase = .closed }
        case .connecting, .paused, .closed:
            break
        }
        return phase
    }

    /// The line over the picture (nil while it plays), in the web's words.
    public func status(name: String) -> String? {
        switch phase {
        case .connecting: "Connecting to \(name)'s screen…"
        case .playing: nil
        case .paused: "\(name)'s screen is paused."
        case .stopped, .closed: "\(name) stopped sharing."
        }
    }
}

/// Who's watching my share: the people whose `{screen: "want"}` reached me
/// and who haven't said "stop" or gone away. Local only (nothing on the wire).
public enum ScreenAudience {
    /// `Watching: Ana, Ben` / `No one's watching yet`. Each person once
    /// (two devices watching = one name), A–Z.
    public static func text(names: [String]) -> String {
        var seen = Set<String>()
        let unique = names.filter { seen.insert($0).inserted }.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
        return unique.isEmpty ? "No one's watching yet" : "Watching: " + unique.joined(separator: ", ")
    }
}

/// The common codec for screen video is H.264, constrained baseline,
/// packetization-mode=1 (PROTOCOL.md › Screen sharing): every client offers
/// and accepts it. We put it first, then any other H.264, then the rest in
/// their own order (VP8/VP9/AV1 stay as fallbacks).
public enum ScreenCodecs {
    public struct Codec: Sendable, Equatable {
        public var name: String
        public var parameters: [String: String]
        public init(name: String, parameters: [String: String] = [:]) {
            self.name = name
            self.parameters = parameters
        }
    }

    /// Constrained baseline (profile_idc 0x42, constraint_set1): 42e0xx / 42c0xx… (RFC 6184).
    public static func isConstrainedBaseline(_ profileLevelId: String?) -> Bool {
        guard let id = profileLevelId?.lowercased(), id.count == 6 else { return false }
        let profile = id.prefix(2)
        guard profile == "42", let constraints = UInt8(id.dropFirst(2).prefix(2), radix: 16) else { return false }
        return constraints & 0x40 != 0
    }

    /// 0: H.264 constrained baseline, packetization-mode=1; 1: other
    /// H.264 with packetization-mode=1; 2: other H.264; 3: everything else.
    public static func rank(_ codec: Codec) -> Int {
        guard codec.name.uppercased() == "H264" else { return 3 }
        let mode1 = codec.parameters["packetization-mode"] == "1"
        if mode1, isConstrainedBaseline(codec.parameters["profile-level-id"]) { return 0 }
        return mode1 ? 1 : 2
    }

    /// The order to hand `setCodecPreferences` (stable within a rank).
    public static func ordered<T>(_ codecs: [T], as codec: (T) -> Codec) -> [T] {
        codecs.enumerated()
            .sorted { (rank(codec($0.element)), $0.offset) < (rank(codec($1.element)), $1.offset) }
            .map(\.element)
    }
}
