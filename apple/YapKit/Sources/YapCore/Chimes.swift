import Foundation

// The soundboard and join chimes (PROTOCOL.md › Extras).

/// An uploaded clip: a soundboard sound or someone's join chime.
public struct Sound: Codable, Sendable, Hashable, Identifiable {
    public var id: String
    /// "board" or "chime".
    public var kind: String
    public var name: String
    public var emoji: String?
    /// Bytes at `/media/<media>` (audio/wav).
    public var media: String
    /// Measured loudness correction, 0…1: never louder than uploaded.
    public var gain: Double
    public var ms: Int
    public var uid: String
    public var createdAt: Int

    public init(id: String, kind: String = "board", name: String, emoji: String? = nil, media: String, gain: Double = 1,
                ms: Int = 0, uid: String = "", createdAt: Int = 0) {
        self.id = id
        self.kind = kind
        self.name = name
        self.emoji = emoji
        self.media = media
        self.gain = gain
        self.ms = ms
        self.uid = uid
        self.createdAt = createdAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = (try? c.decodeIfPresent(String.self, forKey: .kind)) ?? "board"
        name = (try? c.decodeIfPresent(String.self, forKey: .name)) ?? "Sound"
        emoji = try? c.decodeIfPresent(String.self, forKey: .emoji)
        media = (try? c.decodeIfPresent(String.self, forKey: .media)) ?? id
        gain = min(1, max(0, (try? c.decodeIfPresent(Double.self, forKey: .gain)) ?? 1))
        ms = (try? c.decodeIfPresent(Int.self, forKey: .ms)) ?? 0
        uid = (try? c.decodeIfPresent(String.self, forKey: .uid)) ?? ""
        createdAt = (try? c.decodeIfPresent(Int.self, forKey: .createdAt)) ?? 0
    }

    public var isChime: Bool { kind == "chime" }
}

/// The soundboard's volume rule (PROTOCOL.md › Soundboard), the same in every client.
public enum Soundboard {
    /// Settings › Voice default.
    public static let defaultVolume = 0.5
    /// At most this many clips at once (a third stops the oldest).
    public static let maxConcurrent = 2
    /// The limiter's threshold, dB.
    public static let limiterThreshold = -12.0
    /// Board clips ≤ 5 s, chimes ≤ 2 s, ≤ 480 KB, mono PCM16 at 24 kHz from us.
    public static let maxBoardSeconds = 5.0
    public static let maxChimeSeconds = 2.0
    public static let maxBytes = 480_000
    public static let sampleRate = 24_000.0
    public static let maxBoardSounds = 60
    public static let maxNameLength = 24

    /// Linear gain to play a clip at: its measured gain × my volume (0…1),
    /// so it's never above what the server measured.
    public static func playbackGain(clipGain: Double, volume: Double) -> Double {
        min(1, max(0, clipGain)) * min(1, max(0, volume))
    }

    /// Should a play from `uid` sound here at all?
    public static func shouldPlay(muted: Bool, deafened: Bool, personVolume: Double?) -> Bool {
        !muted && !deafened && (personVolume ?? 1) > 0
    }
}

/// The built-in join chimes, synthesized (no audio files), identical in every
/// client: notes (Hz) `step` s apart, each `length` s with a 12 ms attack and
/// an exponential decay.
public struct ChimeTone: Sendable, Equatable, Identifiable {
    public enum Wave: String, Sendable { case sine, triangle, square }

    public var id: String
    public var title: String
    public var notes: [Double]
    public var step: Double
    public var length: Double
    public var volume: Double
    public var wave: Wave

    public static let all: [ChimeTone] = [
        ChimeTone(id: "default", title: "Classic blip", notes: [659.25, 880], step: 0.075, length: 0.11, volume: 0.05, wave: .sine),
        ChimeTone(id: "ding", title: "Ding", notes: [1318.51], step: 0, length: 0.5, volume: 0.08, wave: .sine),
        ChimeTone(id: "chord", title: "Chord", notes: [523.25, 659.25, 783.99], step: 0, length: 0.6, volume: 0.05, wave: .triangle),
        ChimeTone(id: "tada", title: "Ta-da", notes: [392, 523.25, 659.25, 783.99, 1046.5], step: 0.07, length: 0.25, volume: 0.06, wave: .triangle),
        ChimeTone(id: "bubble", title: "Bubble", notes: [440, 660, 990], step: 0.04, length: 0.08, volume: 0.06, wave: .sine),
        ChimeTone(id: "knock", title: "Knock knock", notes: [160, 160], step: 0.16, length: 0.09, volume: 0.14, wave: .sine),
        ChimeTone(id: "arcade", title: "Arcade", notes: [523.25, 783.99, 1046.5, 1567.98], step: 0.05, length: 0.09, volume: 0.04, wave: .square),
        ChimeTone(id: "none", title: "Silent", notes: [], step: 0, length: 0, volume: 0, wave: .sine),
    ]

    public static func named(_ id: String?) -> ChimeTone? {
        let id = id ?? "default"
        return all.first { $0.id == id }
    }

    /// Yap's own little blips (the web's `sounds.js`), on the same synth.
    public static let join = ChimeTone(id: "_join", title: "", notes: [523.25, 659.25, 783.99], step: 0.075, length: 0.11, volume: 0.07, wave: .sine)
    public static let leave = ChimeTone(id: "_leave", title: "", notes: [783.99, 659.25, 523.25], step: 0.075, length: 0.11, volume: 0.07, wave: .sine)
    public static let peerLeave = ChimeTone(id: "_peerLeave", title: "", notes: [880, 659.25], step: 0.075, length: 0.11, volume: 0.05, wave: .sine)
    public static let mute = ChimeTone(id: "_mute", title: "", notes: [392], step: 0.075, length: 0.08, volume: 0.05, wave: .triangle)
    public static let unmute = ChimeTone(id: "_unmute", title: "", notes: [587.33], step: 0.075, length: 0.08, volume: 0.05, wave: .triangle)
    public static let ping = ChimeTone(id: "_ping", title: "", notes: [783.99, 1046.5, 1318.5, 1567.98], step: 0.09, length: 0.22, volume: 0.13, wave: .square)
    public static let mention = ChimeTone(id: "_mention", title: "", notes: [987.77, 1318.5], step: 0.08, length: 0.16, volume: 0.09, wave: .triangle)

    /// Mono samples at `sampleRate`, like the web's WebAudio graph (each
    /// note: 0 → volume over 12 ms, exponential decay to 0.0001 at `length`,
    /// 20 ms of tail).
    public func render(sampleRate: Double = 48_000) -> [Float] {
        guard !notes.isEmpty, length > 0 else { return [] }
        let duration = Double(notes.count - 1) * step + length + 0.02
        var out = [Float](repeating: 0, count: Int((duration * sampleRate).rounded(.up)))
        let attack = 0.012
        for (i, freq) in notes.enumerated() {
            let start = Double(i) * step
            let first = Int(start * sampleRate)
            let count = Int((length + 0.02) * sampleRate)
            for n in 0..<count where first + n < out.count {
                let t = Double(n) / sampleRate
                let envelope: Double
                if t < attack {
                    envelope = volume * t / attack
                } else if t < length {
                    // Exponential ramp from `volume` (at the attack's end) to 0.0001 at `length`.
                    let k = (t - attack) / (length - attack)
                    envelope = volume * pow(0.0001 / volume, k)
                } else {
                    envelope = 0
                }
                let phase = (freq * t).truncatingRemainder(dividingBy: 1)
                let sample: Double
                switch wave {
                case .sine: sample = sin(2 * .pi * phase)
                case .triangle: sample = 1 - 4 * abs(phase - 0.5)
                case .square: sample = phase < 0.5 ? 1 : -1
                }
                out[first + n] += Float(sample * envelope)
            }
        }
        return out
    }
}

/// A person's chime setting: a built-in, or an uploaded clip.
public enum ChimeChoice: Equatable, Sendable {
    case builtIn(ChimeTone)
    case clip(soundId: String)

    /// `User.chime` → what to play (unknown ids fall back to `default`).
    public static func from(_ value: String?) -> ChimeChoice {
        if let value, value.hasPrefix("s:"), value.count > 2 { return .clip(soundId: String(value.dropFirst(2))) }
        return .builtIn(ChimeTone.named(value) ?? ChimeTone.all[0])
    }
}
