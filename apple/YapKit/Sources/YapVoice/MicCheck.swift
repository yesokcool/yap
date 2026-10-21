import AVFoundation
import Foundation
import Observation
import os

/// Settings › Voice › Hear yourself: 4 seconds of your mic the way a call
/// treats it, played back. The web records what its call path sends; the
/// stasel/WebRTC SDK has no way to tap that, so this rebuilds it outside a
/// call: Apple's voice processing (on iPhone the same unit WebRTC's calls
/// use: echo cancellation, noise suppression, gain), then the transmit gate
/// as the engine runs it (decided every 25 ms, a short ramp at each switch),
/// then Opus at the call's bitrate (Apple's encoder), then your speaker.
/// It can't share the mic with a call, so it's offered outside one.
@MainActor
@Observable
public final class MicCheck {
    public enum Phase: Equatable, Sendable {
        case idle, recording, playing, done
        case failed(String)
    }

    /// Where the 4 seconds come from: the mic, or (tests, the simulator) a
    /// synthetic chord with pauses, like `SyntheticAudioDevice`.
    public enum Source: Sendable {
        case microphone, synthetic
    }

    public static let seconds = 4.0
    nonisolated static let sampleRate = 48_000.0

    public private(set) var phase: Phase = .idle
    /// The level while recording (linear RMS, on the meter's scale).
    public private(set) var level = 0.0
    /// Peaks keep reaching −1 dBFS while recording ("Too loud (clipping)").
    public private(set) var tooLoud = false
    @ObservationIgnored private var clipWatch = ClipWatch()
    /// It went through Opus (false: no encoder here, so without the codec).
    public private(set) var throughCodec = true
    public var isRunning: Bool { phase == .recording || phase == .playing }

    /// Screenshots only (`YAP_DEMO_MICCHECK`): show this phase. Only the
    /// app's debug launch calls it; not behind `#if DEBUG` because SwiftPM
    /// builds this package without DEBUG under the app's Dev configuration.
    public func show(_ phase: Phase, level: Double = 0, throughCodec: Bool = true, tooLoud: Bool = false) {
        self.phase = phase
        self.level = level
        self.throughCodec = throughCodec
        self.tooLoud = tooLoud
    }

    /// One chunk's level and loudest sample while recording.
    private func heard(rms: Double, peakDb: Double) {
        level = rms
        let loud = clipWatch.update(peakDb: peakDb, at: ProcessInfo.processInfo.systemUptime)
        if loud != tooLoud { tooLoud = loud }
    }

    private let source: Source
    private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "voice")

    public init(source: Source = .microphone) {
        self.source = source
    }

    /// Record, process like a call at `bitrate` with this transmit mode, play back.
    public func run(bitrate: Int, mode: VoiceEngine.InputMode, threshold: Double, highQualityBluetooth: Bool = false) async {
        guard !isRunning else { return }
        phase = .recording
        level = 0
        clipWatch.reset()
        tooLoud = false
        let recorded: Recording
        do {
            switch source {
            case .synthetic:
                recorded = await recordSynthetic()
            case .microphone:
                guard await VoiceController.ensureMicPermission(ask: true) else { throw MicCheckError.noMic }
                recorded = try await recordMicrophone(highQualityBluetooth: highQualityBluetooth)
            }
        } catch {
            log.error("hear yourself: \(error.localizedDescription, privacy: .public)")
            restoreSession()
            phase = .failed("Couldn't open the microphone.")
            return
        }
        level = 0
        tooLoud = false
        let rendered = await Task.detached(priority: .userInitiated) {
            Self.render(recorded.samples, sampleRate: recorded.sampleRate, bitrate: bitrate, mode: mode, threshold: threshold)
        }.value
        throughCodec = rendered.throughCodec
        phase = .playing
        await play(rendered.samples)
        restoreSession()
        phase = .done
    }

    // MARK: Recording

    struct Recording: Sendable {
        var samples: [Float]
        var sampleRate: Double
    }

    enum MicCheckError: LocalizedError {
        case noMic, noInput
        var errorDescription: String? {
            switch self {
            case .noMic: "microphone permission denied"
            case .noInput: "no input device"
            }
        }
    }

    private func recordMicrophone(highQualityBluetooth: Bool) async throws -> Recording {
        #if os(iOS)
        let session = AVAudioSession.sharedInstance()
        if highQualityBluetooth {
            try session.setCategory(.playAndRecord, mode: .default, options: [.allowBluetoothA2DP, .defaultToSpeaker])
        } else {
            try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetoothHFP, .defaultToSpeaker])
        }
        try session.setActive(true)
        #endif
        let engine = AVAudioEngine()
        let input = engine.inputNode
        do {
            try input.setVoiceProcessingEnabled(true)
        } catch {
            log.error("hear yourself without voice processing: \(error.localizedDescription, privacy: .public)")
        }
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else { throw MicCheckError.noInput }
        // An output path (silent), which voice processing expects; the mic isn't wired to it.
        engine.mainMixerNode.outputVolume = 0
        let collected = OSAllocatedUnfairLock(initialState: [Float]())
        let wanted = Int(format.sampleRate * Self.seconds)
        input.installTap(onBus: 0, bufferSize: 4800, format: format) { @Sendable [weak self] buffer, _ in
            guard let channel = buffer.floatChannelData?[0] else { return }
            let chunk = Array(UnsafeBufferPointer(start: channel, count: Int(buffer.frameLength)))
            let level = Self.rms(chunk)
            let peak = ClipWatch.peakDb(chunk)
            collected.withLock { $0.append(contentsOf: chunk) }
            Task { @MainActor in self?.heard(rms: level, peakDb: peak) }
        }
        engine.prepare()
        try engine.start()
        defer {
            input.removeTap(onBus: 0)
            engine.stop()
        }
        let deadline = ContinuousClock.now + .seconds(Self.seconds + 1)
        while collected.withLock({ $0.count }) < wanted, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(50))
        }
        let samples = collected.withLock { Array($0.prefix(wanted)) }
        return Recording(samples: samples, sampleRate: format.sampleRate)
    }

    /// The test chord: 0.8 s on, 0.4 s quiet, in real time (the meter moves).
    private func recordSynthetic() async -> Recording {
        let samples = Self.syntheticSpeech(seconds: Self.seconds)
        let step = Int(Self.sampleRate / 10)
        var at = 0
        while at < samples.count {
            let chunk = Array(samples[at..<min(samples.count, at + step)])
            heard(rms: Self.rms(chunk), peakDb: ClipWatch.peakDb(chunk))
            at += step
            try? await Task.sleep(for: .milliseconds(100))
        }
        return Recording(samples: samples, sampleRate: Self.sampleRate)
    }

    nonisolated static func syntheticSpeech(seconds: Double) -> [Float] {
        let count = Int(sampleRate * seconds)
        var seed: UInt32 = 7
        return (0..<count).map { i in
            let t = Double(i) / sampleRate
            guard t.truncatingRemainder(dividingBy: 1.2) < 0.8 else { return 0 }
            let chord = [220.0, 277.18, 329.63].reduce(0) { $0 + sin(2 * .pi * $1 * t) * 0.15 }
            seed = seed &* 1_103_515_245 &+ 12345
            let noise = (Double(seed >> 16 & 0x7FFF) / 16384 - 1) * 0.05
            return Float(chord + noise)
        }
    }

    // MARK: What a call does to it

    struct Rendered: Sendable {
        var samples: [Float]
        var throughCodec: Bool
    }

    /// The gate (as the engine runs it), then Opus at `bitrate`. Outside a
    /// call push to talk acts like always on (there's no key to hold).
    nonisolated static func render(_ input: [Float], sampleRate: Double, bitrate: Int, mode: VoiceEngine.InputMode, threshold: Double) -> Rendered {
        let samples = sampleRate == Self.sampleRate ? input : resample(input, from: sampleRate) ?? input
        let gated = gate(samples, mode: mode == .pushToTalk ? .alwaysOn : mode.gateMode, threshold: threshold)
        if let coded = opusRoundTrip(gated, bitrate: bitrate) {
            return Rendered(samples: coded, throughCodec: true)
        }
        return Rendered(samples: gated, throughCodec: false)
    }

    /// The engine reads the level every 25 ms and switches the track for
    /// what comes next; libwebrtc ramps each switch over 128 samples.
    nonisolated static func gate(_ samples: [Float], mode: VoiceGate.Mode, threshold: Double) -> [Float] {
        let block = Int(sampleRate * 0.025)
        let ramp = 128
        var gate = VoiceGate(mode: mode, threshold: threshold)
        var out = samples
        var open = gate.isOpen
        var gain: Float = open ? 1 : 0
        var start = 0
        var t = 0.0
        while start < samples.count {
            let end = min(samples.count, start + block)
            let target: Float = open ? 1 : 0
            for i in start..<end {
                if gain != target { gain = target > gain ? min(1, gain + 1 / Float(ramp)) : max(0, gain - 1 / Float(ramp)) }
                out[i] = samples[i] * gain
            }
            let chunk = samples[start..<end]
            t += Double(end - start) / sampleRate
            open = gate.update(db: VoiceGate.dB(rms(chunk)), duration: Double(end - start) / sampleRate, at: t)
            start = end
        }
        return out
    }

    nonisolated static func rms<C: Collection>(_ samples: C) -> Double where C.Element == Float {
        guard !samples.isEmpty else { return 0 }
        let sum = samples.reduce(0.0) { $0 + Double($1) * Double($1) }
        return (sum / Double(samples.count)).squareRoot()
    }

    nonisolated static var pcmFormat: AVAudioFormat {
        AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: sampleRate, channels: 1, interleaved: false)!
    }

    nonisolated static func buffer(_ samples: [Float], format: AVAudioFormat) -> AVAudioPCMBuffer? {
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(max(1, samples.count))) else { return nil }
        buffer.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { source in
            buffer.floatChannelData?[0].update(from: source.baseAddress!, count: samples.count)
        }
        return buffer
    }

    nonisolated static func resample(_ samples: [Float], from rate: Double) -> [Float]? {
        guard rate > 0, let fromFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: 1, interleaved: false),
              let converter = AVAudioConverter(from: fromFormat, to: pcmFormat),
              let input = buffer(samples, format: fromFormat),
              let output = AVAudioPCMBuffer(pcmFormat: pcmFormat, frameCapacity: AVAudioFrameCount(Double(samples.count) * sampleRate / rate) + 4096) else { return nil }
        nonisolated(unsafe) var fed = false
        var error: NSError?
        converter.convert(to: output, error: &error) { _, status in
            if fed {
                status.pointee = .endOfStream
                return nil
            }
            fed = true
            status.pointee = .haveData
            return input
        }
        guard error == nil, let data = output.floatChannelData?[0] else { return nil }
        return Array(UnsafeBufferPointer(start: data, count: Int(output.frameLength)))
    }

    /// Through Apple's Opus encoder at `bitrate` and back; nil if there's none.
    nonisolated static func opusRoundTrip(_ samples: [Float], bitrate: Int) -> [Float]? {
        var description = AudioStreamBasicDescription(mSampleRate: sampleRate, mFormatID: kAudioFormatOpus, mFormatFlags: 0, mBytesPerPacket: 0,
                                                      mFramesPerPacket: 960, mBytesPerFrame: 0, mChannelsPerFrame: 1, mBitsPerChannel: 0, mReserved: 0)
        guard !samples.isEmpty, let opus = AVAudioFormat(streamDescription: &description),
              let encoder = AVAudioConverter(from: pcmFormat, to: opus),
              let decoder = AVAudioConverter(from: opus, to: pcmFormat),
              let input = buffer(samples, format: pcmFormat) else { return nil }
        // The nearest rate Apple's encoder offers (it lists 6k…512k).
        let rates = encoder.applicableEncodeBitRates?.map(\.intValue) ?? []
        encoder.bitRate = rates.min { abs($0 - bitrate) < abs($1 - bitrate) } ?? bitrate
        var packets: [AVAudioCompressedBuffer] = []
        nonisolated(unsafe) var fed = false
        while true {
            let out = AVAudioCompressedBuffer(format: opus, packetCapacity: 16, maximumPacketSize: max(1, encoder.maximumOutputPacketSize))
            var error: NSError?
            let status = encoder.convert(to: out, error: &error) { _, status in
                if fed {
                    status.pointee = .endOfStream
                    return nil
                }
                fed = true
                status.pointee = .haveData
                return input
            }
            if out.packetCount > 0 { packets.append(out) }
            if status == .error || error != nil { return nil }
            if status == .endOfStream || (status == .inputRanDry && out.packetCount == 0) { break }
        }
        var decoded: [Float] = []
        decoded.reserveCapacity(samples.count + 2048)
        nonisolated(unsafe) var next = 0
        let packetList = packets
        while true {
            guard let chunk = AVAudioPCMBuffer(pcmFormat: pcmFormat, frameCapacity: 9600) else { return nil }
            var error: NSError?
            let status = decoder.convert(to: chunk, error: &error) { _, status in
                guard next < packetList.count else {
                    status.pointee = .endOfStream
                    return nil
                }
                defer { next += 1 }
                status.pointee = .haveData
                return packetList[next]
            }
            if let data = chunk.floatChannelData?[0], chunk.frameLength > 0 {
                decoded.append(contentsOf: UnsafeBufferPointer(start: data, count: Int(chunk.frameLength)))
            }
            if status == .error || error != nil { return nil }
            if status == .endOfStream || chunk.frameLength == 0 { break }
        }
        return decoded.isEmpty ? nil : decoded
    }

    // MARK: Playing it back

    private func play(_ samples: [Float]) async {
        let engine = AVAudioEngine()
        let player = AVAudioPlayerNode()
        let format = Self.pcmFormat
        guard let buffer = Self.buffer(samples, format: format) else { return }
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: format)
        // The synthetic source (tests, the simulator) plays silently: same path, no noise.
        if source == .synthetic { player.volume = 0 }
        do {
            engine.prepare()
            try engine.start()
        } catch {
            log.error("hear yourself: no playback: \(error.localizedDescription, privacy: .public)")
            return
        }
        await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
            player.scheduleBuffer(buffer, at: nil, options: [], completionCallbackType: .dataPlayedBack) { _ in
                done.resume()
            }
            player.play()
        }
        player.stop()
        engine.stop()
    }

    private func restoreSession() {
        #if os(iOS)
        guard source == .microphone else { return }
        // Back to the app's everyday session (never stops someone's music).
        AudioSessionSetup.everyday()
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        #endif
    }
}
