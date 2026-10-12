#if os(iOS)
import AudioToolbox
import Foundation
import os
@preconcurrency import WebRTC

/// A fake microphone + speaker for WebRTC: records a music-like chord with a
/// little noise (so Opus actually spends its bits) and "plays" what arrives
/// by pulling it and measuring its level. Lets tests and the simulator run a
/// real call without touching the Mac's mic. Not used by the shipping app.
///
/// WebRTC calls this from its own threads; the clock is a dedicated thread
/// (WebRTC wants recorded audio delivered from one thread). Like stopping a
/// real audio unit, stopping playout or recording (or terminating) waits
/// for a tick that's inside WebRTC to come back, and only one clock thread
/// ever runs, so WebRTC is never called after it has stopped or torn down.
public final class SyntheticAudioDevice: NSObject, RTCAudioDevice, @unchecked Sendable {
    private struct State {
        var delegate: (any RTCAudioDeviceDelegate)?
        var initialized = false
        var playoutInitialized = false
        var playing = false
        var recordingInitialized = false
        var recording = false
        var running = false
        /// Which clock thread is the live one (a stale one stops at its next check).
        var clock = 0
        var phase = 0.0
        var seed: UInt32 = 1
        var sampleIndex = 0
        var playedFrames = 0
        var recordedFrames = 0
        var recentPlayoutRMS = 0.0
        /// Times the device went fully quiet (neither playing nor recording)
        /// after it had been running: a real iPhone's audio unit stops then.
        var ioStops = 0
    }

    private let lock = OSAllocatedUnfairLock(initialState: State())
    /// Held for a whole tick, WebRTC calls included. Stopping takes it once,
    /// so it returns only after an in-flight tick is out of WebRTC.
    /// Recursive in case WebRTC stops the device from inside a callback.
    private let tickLock = NSRecursiveLock()
    private let sampleRate = 48_000.0
    private let framesPerTick = 480 // 10 ms

    /// Silence instead of the chord (to check the far side's level drops).
    public let silent = OSAllocatedUnfairLock(initialState: false)
    /// The chord four times too loud, clipped at full scale (the meter's
    /// "Too loud (clipping)").
    public let loud = OSAllocatedUnfairLock(initialState: false)

    public override init() {}

    // MARK: What the test reads

    public var playedFrames: Int { lock.withLock { $0.playedFrames } }
    public var recordedFrames: Int { lock.withLock { $0.recordedFrames } }
    /// RMS (0…1) of the received audio over the last ~100 ms.
    public var recentPlayoutRMS: Double { lock.withLock { $0.recentPlayoutRMS } }
    /// How many times playout and recording both stopped (on an iPhone that
    /// shuts the voice-processing unit down and lets go of the audio session).
    public var ioStops: Int { lock.withLock { $0.ioStops } }
    /// Playing or recording right now.
    public var isRunningIO: Bool { lock.withLock { $0.playing || $0.recording } }

    // MARK: RTCAudioDevice

    public var deviceInputSampleRate: Double { sampleRate }
    public var inputIOBufferDuration: TimeInterval { 0.01 }
    public var inputNumberOfChannels: Int { 1 }
    public var inputLatency: TimeInterval { 0 }
    public var deviceOutputSampleRate: Double { sampleRate }
    public var outputIOBufferDuration: TimeInterval { 0.01 }
    public var outputNumberOfChannels: Int { 1 }
    public var outputLatency: TimeInterval { 0 }

    public var isInitialized: Bool { lock.withLock { $0.initialized } }
    public var isPlayoutInitialized: Bool { lock.withLock { $0.playoutInitialized } }
    public var isPlaying: Bool { lock.withLock { $0.playing } }
    public var isRecordingInitialized: Bool { lock.withLock { $0.recordingInitialized } }
    public var isRecording: Bool { lock.withLock { $0.recording } }

    public func initialize(with delegate: any RTCAudioDeviceDelegate) -> Bool {
        lock.withLock {
            $0.delegate = delegate
            $0.initialized = true
        }
        return true
    }

    public func terminateDevice() -> Bool {
        lock.withLock {
            let clock = $0.clock
            let stops = $0.ioStops + ($0.playing || $0.recording ? 1 : 0)
            $0 = State()
            $0.clock = clock + 1 // any clock thread still going stops
            $0.ioStops = stops
        }
        waitForTick()
        return true
    }

    public func initializePlayout() -> Bool {
        lock.withLock { $0.playoutInitialized = true }
        return true
    }

    public func startPlayout() -> Bool {
        lock.withLock { $0.playing = true }
        startClock()
        return true
    }

    public func stopPlayout() -> Bool {
        lock.withLock {
            if $0.playing, !$0.recording { $0.ioStops += 1 }
            $0.playing = false
        }
        waitForTick()
        return true
    }

    public func initializeRecording() -> Bool {
        lock.withLock { $0.recordingInitialized = true }
        return true
    }

    public func startRecording() -> Bool {
        lock.withLock { $0.recording = true }
        startClock()
        return true
    }

    public func stopRecording() -> Bool {
        lock.withLock {
            if $0.recording, !$0.playing { $0.ioStops += 1 }
            $0.recording = false
        }
        waitForTick()
        return true
    }

    /// Returns once a tick that's under way has finished (the next one sees the new state).
    private func waitForTick() {
        tickLock.lock()
        tickLock.unlock()
    }

    // MARK: Clock

    private func startClock() {
        let clock = lock.withLock { state -> Int? in
            if state.running { return nil }
            state.running = true
            state.clock += 1
            return state.clock
        }
        guard let clock else { return }
        let thread = Thread { [weak self] in
            var next = Date()
            while let self, self.tick(clock: clock) {
                next = next.addingTimeInterval(0.01)
                let wait = next.timeIntervalSinceNow
                if wait > 0 { Thread.sleep(forTimeInterval: wait) } else if wait < -0.2 { next = Date() }
            }
        }
        thread.name = "yap.synthetic-audio"
        thread.qualityOfService = .userInteractive
        thread.start()
    }

    /// What one tick does, read under the lock.
    private struct Tick {
        var delegate: (any RTCAudioDeviceDelegate)?
        var recording: Bool
        var playing: Bool
    }

    /// One 10 ms tick; false once this clock should stop (stopped, torn
    /// down, or replaced by a newer clock thread).
    private func tick(clock: Int) -> Bool {
        tickLock.lock()
        defer { tickLock.unlock() }
        let frames = framesPerTick
        let now: Tick? = lock.withLock { state -> Tick? in
            guard state.running, state.clock == clock else { return nil }
            // Nothing left to do: stop in the same breath, so a start right
            // after this sees no clock running and begins a new one.
            guard state.playing || state.recording else {
                state.running = false
                return nil
            }
            return Tick(delegate: state.delegate, recording: state.recording, playing: state.playing)
        }
        guard let now else { return false }
        guard let delegate = now.delegate else { return true }
        let recording = now.recording, playing = now.playing
        var flags = AudioUnitRenderActionFlags()
        var timestamp = AudioTimeStamp()

        if recording {
            let silent = self.silent.withLock { $0 }
            let gain = self.loud.withLock { $0 } ? 4.0 : 1.0
            var (start, seed) = lock.withLock { ($0.sampleIndex, $0.seed) }
            var samples = [Int16](repeating: 0, count: frames)
            for i in 0..<frames {
                let t = Double(start + i) / sampleRate
                // Two alternating chords + noise, like the web e2e test's WAV.
                let chord: [Double] = Int(t) % 2 == 1 ? [196, 246.94, 293.66] : [220, 277.18, 329.63]
                var v = chord.reduce(0) { $0 + sin(2 * .pi * $1 * t) * 0.18 }
                seed = seed &* 1_103_515_245 &+ 12345
                v += (Double(seed >> 16 & 0x7FFF) / 16384 - 1) * 0.12
                v *= 0.6 + 0.4 * sin(2 * .pi * 0.5 * t)
                samples[i] = silent ? 0 : Int16(max(-1, min(1, v * gain)) * (gain > 1 ? 32_767 : 30_000))
            }
            start += frames
            let newSeed = seed, newStart = start
            lock.withLock {
                $0.sampleIndex = newStart
                $0.seed = newSeed
                $0.recordedFrames += frames
            }
            timestamp.mSampleTime = Double(lock.withLock { $0.sampleIndex })
            timestamp.mFlags = .sampleTimeValid
            samples.withUnsafeMutableBytes { raw in
                var list = AudioBufferList(mNumberBuffers: 1, mBuffers: AudioBuffer(mNumberChannels: 1, mDataByteSize: UInt32(raw.count), mData: raw.baseAddress))
                _ = delegate.deliverRecordedData(&flags, &timestamp, 1, UInt32(frames), &list, nil, nil)
            }
        }

        if playing {
            var out = [Int16](repeating: 0, count: frames)
            out.withUnsafeMutableBytes { raw in
                var list = AudioBufferList(mNumberBuffers: 1, mBuffers: AudioBuffer(mNumberChannels: 1, mDataByteSize: UInt32(raw.count), mData: raw.baseAddress))
                _ = delegate.getPlayoutData(&flags, &timestamp, 0, UInt32(frames), &list)
            }
            let energy = out.reduce(0.0) { $0 + Double($1) * Double($1) } / Double(frames)
            let rms = sqrt(energy) / 32768
            lock.withLock {
                $0.playedFrames += frames
                $0.recentPlayoutRMS = $0.recentPlayoutRMS * 0.9 + rms * 0.1
            }
        }
        return true
    }
}
#endif
