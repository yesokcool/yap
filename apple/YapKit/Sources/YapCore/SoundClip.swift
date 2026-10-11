import AVFoundation
import Foundation

/// A clip someone picked for the soundboard or their chime: whatever the
/// file was (m4a voice memo, mp3, wav, aiff, caf…), decoded with
/// AVFoundation, mixed to mono and resampled to 24 kHz. Trim it, then
/// `wav(start:seconds:)` is what goes to `POST /api/sound`.
public struct SoundClip: Sendable {
    public static let sampleRate = Soundboard.sampleRate

    /// Mono float samples at 24 kHz.
    public var samples: [Float]
    public var name: String

    public init(samples: [Float], name: String) {
        self.samples = samples
        self.name = name
    }

    public var seconds: Double { Double(samples.count) / Self.sampleRate }

    /// Seconds to the first sample above the silence floor (0.01), less
    /// 10 ms — where the web starts a clip.
    public var firstSound: Double {
        guard let i = samples.firstIndex(where: { abs($0) >= 0.01 }) else { return 0 }
        return max(0, Double(i) / Self.sampleRate - 0.01)
    }

    public enum Failure: Error, LocalizedError, Equatable {
        case unreadable
        case silent
        case tooBig

        public var errorDescription: String? {
            switch self {
            case .unreadable: "That file isn't a sound Yap can read. Try an m4a, mp3 or wav."
            case .silent: "That clip is silent."
            case .tooBig: "That clip is too big even after trimming."
            }
        }
    }

    /// Decode a file. Reads at most `maxSeconds` (long files only need their start).
    public static func load(_ url: URL, maxSeconds: Double = 120) throws -> SoundClip {
        let file: AVAudioFile
        do { file = try AVAudioFile(forReading: url) } catch { throw Failure.unreadable }
        let source = file.processingFormat
        let frames = AVAudioFrameCount(min(Double(file.length), maxSeconds * source.sampleRate))
        guard frames > 0, let buffer = AVAudioPCMBuffer(pcmFormat: source, frameCapacity: frames) else { throw Failure.unreadable }
        do { try file.read(into: buffer, frameCount: frames) } catch { throw Failure.unreadable }
        let mono = try mixToMono(buffer)
        let samples = try resample(mono, from: source.sampleRate)
        guard WAV.levels(samples).peak > 0.001 else { throw Failure.silent }
        return SoundClip(samples: samples, name: url.deletingPathExtension().lastPathComponent)
    }

    /// The clip from `start`, at most `seconds` long, as a PCM16 WAV.
    public func wav(start: Double, seconds: Double) throws -> (data: Data, ms: Int) {
        let cut = WAV.trim(samples, sampleRate: Self.sampleRate, start: start, seconds: seconds)
        guard !cut.isEmpty, WAV.levels(cut).peak > 0.001 else { throw Failure.silent }
        let data = WAV.encode(cut, sampleRate: Int(Self.sampleRate))
        guard data.count <= Soundboard.maxBytes else { throw Failure.tooBig }
        return (data, Int((Double(cut.count) / Self.sampleRate * 1000).rounded()))
    }

    /// A rough loudness outline for a trim strip (`count` bars, 0…1).
    public func outline(_ count: Int) -> [Float] {
        guard count > 0, !samples.isEmpty else { return [] }
        let per = max(1, samples.count / count)
        let peak = max(0.001, WAV.levels(samples).peak)
        return (0..<count).map { i in
            let slice = samples[min(samples.count, i * per)..<min(samples.count, (i + 1) * per)]
            return (slice.map { abs($0) }.max() ?? 0) / peak
        }
    }

    private static func mixToMono(_ buffer: AVAudioPCMBuffer) throws -> [Float] {
        let frames = Int(buffer.frameLength)
        let channels = Int(buffer.format.channelCount)
        if let data = buffer.floatChannelData {
            var out = [Float](repeating: 0, count: frames)
            for c in 0..<channels {
                let channel = data[c]
                for i in 0..<frames { out[i] += channel[i] }
            }
            if channels > 1 { for i in 0..<frames { out[i] /= Float(channels) } }
            return out
        }
        if let data = buffer.int16ChannelData {
            var out = [Float](repeating: 0, count: frames)
            for c in 0..<channels {
                for i in 0..<frames { out[i] += Float(data[c][i]) / 32768 }
            }
            if channels > 1 { for i in 0..<frames { out[i] /= Float(channels) } }
            return out
        }
        throw Failure.unreadable
    }

    /// AVAudioConverter for quality (linear interpolation as the fallback).
    private static func resample(_ mono: [Float], from rate: Double) throws -> [Float] {
        guard rate != sampleRate, !mono.isEmpty else { return mono }
        guard let inFormat = AVAudioFormat(standardFormatWithSampleRate: rate, channels: 1),
              let outFormat = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1),
              let converter = AVAudioConverter(from: inFormat, to: outFormat),
              let input = AVAudioPCMBuffer(pcmFormat: inFormat, frameCapacity: AVAudioFrameCount(mono.count)) else {
            return WAV.resample(mono, from: rate, to: sampleRate)
        }
        input.frameLength = AVAudioFrameCount(mono.count)
        mono.withUnsafeBufferPointer { input.floatChannelData![0].update(from: $0.baseAddress!, count: mono.count) }
        let capacity = AVAudioFrameCount(Double(mono.count) * sampleRate / rate) + 1024
        guard let output = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: capacity) else {
            return WAV.resample(mono, from: rate, to: sampleRate)
        }
        nonisolated(unsafe) var fed = false
        nonisolated(unsafe) let source = input
        var error: NSError?
        let status = converter.convert(to: output, error: &error) { _, outStatus in
            if fed {
                outStatus.pointee = .endOfStream
                return nil
            }
            fed = true
            outStatus.pointee = .haveData
            return source
        }
        guard status != .error, error == nil, let data = output.floatChannelData else {
            return WAV.resample(mono, from: rate, to: sampleRate)
        }
        return Array(UnsafeBufferPointer(start: data[0], count: Int(output.frameLength)))
    }
}
