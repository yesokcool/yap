import Foundation

/// 16-bit PCM WAV, the soundboard's format: what we upload (mono, 24 kHz)
/// and what we play back (mono, 8–48 kHz from any client).
public enum WAV {
    /// Mono float samples (−1…1) → a PCM16 WAV file.
    public static func encode(_ samples: [Float], sampleRate: Int) -> Data {
        var data = Data(capacity: 44 + samples.count * 2)
        func u32(_ v: UInt32) { withUnsafeBytes(of: v.littleEndian) { data.append(contentsOf: $0) } }
        func u16(_ v: UInt16) { withUnsafeBytes(of: v.littleEndian) { data.append(contentsOf: $0) } }
        let bytes = UInt32(samples.count * 2)
        data.append(contentsOf: Array("RIFF".utf8)); u32(36 + bytes)
        data.append(contentsOf: Array("WAVE".utf8))
        data.append(contentsOf: Array("fmt ".utf8)); u32(16)
        u16(1) /* PCM */; u16(1) /* mono */; u32(UInt32(sampleRate)); u32(UInt32(sampleRate * 2)); u16(2); u16(16)
        data.append(contentsOf: Array("data".utf8)); u32(bytes)
        for s in samples {
            let clamped = max(-1, min(1, s))
            u16(UInt16(bitPattern: Int16((clamped * 32767).rounded())))
        }
        return data
    }

    public struct Decoded: Sendable, Equatable {
        public var samples: [Float]
        public var sampleRate: Int
        public var seconds: Double { Double(samples.count) / Double(max(1, sampleRate)) }
    }

    /// The rates a clip may have (the server takes 8–48 kHz).
    public static let sampleRates = 8_000...192_000

    /// A PCM16 WAV (mono, or the first channel of several) → float samples.
    /// Nil for anything else, a rate outside `sampleRates` included.
    public static func decode(_ data: Data) -> Decoded? {
        let bytes = [UInt8](data)
        guard bytes.count >= 12, String(decoding: bytes[0..<4], as: UTF8.self) == "RIFF",
              String(decoding: bytes[8..<12], as: UTF8.self) == "WAVE" else { return nil }
        func u16(_ i: Int) -> Int { Int(bytes[i]) | Int(bytes[i + 1]) << 8 }
        func u32(_ i: Int) -> Int { u16(i) | u16(i + 2) << 16 }
        var i = 12
        var channels = 1, rate = 0, bits = 0, format = 0
        var samples: [Float]?
        while i + 8 <= bytes.count {
            let id = String(decoding: bytes[i..<i + 4], as: UTF8.self)
            let size = u32(i + 4)
            let body = i + 8
            if id == "fmt ", body + 16 <= bytes.count {
                format = u16(body)
                channels = max(1, u16(body + 2))
                rate = u32(body + 4)
                bits = u16(body + 14)
            } else if id == "data" {
                guard format == 1, bits == 16, rate > 0 else { return nil }
                let end = min(bytes.count, body + size)
                let frame = 2 * channels
                var out: [Float] = []
                out.reserveCapacity((end - body) / frame)
                var j = body
                while j + 1 < end {
                    out.append(Float(Int16(bitPattern: UInt16(u16(j)))) / 32768)
                    j += frame
                }
                samples = out
                break
            }
            i = body + size + (size & 1)
        }
        // AVAudioFormat throws an exception (an abort) at a few MHz.
        guard let samples, Self.sampleRates.contains(rate) else { return nil }
        return Decoded(samples: samples, sampleRate: rate)
    }

    /// Linear-interpolation resampling (good enough for short clips; the
    /// app uses AVAudioConverter when it decodes a file).
    public static func resample(_ samples: [Float], from source: Double, to target: Double) -> [Float] {
        guard source > 0, target > 0, !samples.isEmpty, source != target else { return samples }
        let count = Int((Double(samples.count) * target / source).rounded())
        guard count > 0 else { return [] }
        return (0..<count).map { n in
            let x = Double(n) * source / target
            let i = Int(x)
            let f = Float(x - Double(i))
            let a = samples[min(i, samples.count - 1)]
            let b = samples[min(i + 1, samples.count - 1)]
            return a + (b - a) * f
        }
    }

    /// `seconds` from `start` (clamped), with 5 ms fades so the cut doesn't click.
    public static func trim(_ samples: [Float], sampleRate: Double, start: Double, seconds: Double) -> [Float] {
        let first = max(0, min(samples.count, Int(start * sampleRate)))
        let last = max(first, min(samples.count, first + Int(seconds * sampleRate)))
        var out = Array(samples[first..<last])
        let fade = min(out.count / 2, Int(0.005 * sampleRate))
        for k in 0..<fade {
            let g = Float(k) / Float(max(1, fade))
            out[k] *= g
            out[out.count - 1 - k] *= g
        }
        return out
    }

    /// Peak (0…1) and RMS of a clip: shows how loud an upload is.
    public static func levels(_ samples: [Float]) -> (peak: Float, rms: Float) {
        guard !samples.isEmpty else { return (0, 0) }
        var peak: Float = 0, sum: Float = 0
        for s in samples {
            peak = max(peak, abs(s))
            sum += s * s
        }
        return (peak, (sum / Float(samples.count)).squareRoot())
    }
}
