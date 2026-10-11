import Foundation
import Testing
@testable import YapVoice

/// Hear yourself: what a call does to 4 s of mic, without a mic.
@Suite("Hear yourself")
struct MicCheckTests {
    static let rate = MicCheck.sampleRate

    static func tone(_ seconds: Double, amplitude: Float = 0.2) -> [Float] {
        (0..<Int(rate * seconds)).map { amplitude * Float(sin(2 * .pi * 440 * Double($0) / rate)) }
    }

    static func hiss(_ seconds: Double, amplitude: Float = 0.0003) -> [Float] {
        var seed: UInt32 = 3
        return (0..<Int(rate * seconds)).map { _ in
            seed = seed &* 1_103_515_245 &+ 12345
            return amplitude * (Float(seed >> 16 & 0x7FFF) / 16384 - 1)
        }
    }

    @Test func voiceActivityCutsTheLongPauseAndOpensAgain() {
        // 0.5 s of talk, 1 s of a quiet room (−76 dBFS), 0.5 s of talk.
        let input = Self.tone(0.5) + Self.hiss(1) + Self.tone(0.5)
        let out = MicCheck.gate(input, mode: .voiceActivity, threshold: -50)
        #expect(out.count == input.count)
        let at = { (s: Double) in Int(s * Self.rate) }
        #expect(MicCheck.rms(out[at(0.1)..<at(0.5)]) > 0.1, "talk goes through")
        #expect(MicCheck.rms(out[at(0.5)..<at(0.95)]) > 0, "held 450 ms after the talk")
        #expect(out[at(1.05)..<at(1.5)].allSatisfy { $0 == 0 }, "then nothing: the gate is closed")
        #expect(MicCheck.rms(out[at(1.55)..<at(2)]) > 0.1, "open again within a reading of the talk coming back")
        let always = MicCheck.gate(input, mode: .alwaysOn, threshold: -50)
        #expect(always == input, "always on changes nothing")
    }

    @Test func opusAtTheCallBitrateKeepsTheLevel() throws {
        let input = Self.tone(1)
        let coded = try #require(MicCheck.opusRoundTrip(input, bitrate: 128_000), "Apple's Opus encoder")
        #expect(abs(coded.count - input.count) < 4096)
        let before = MicCheck.rms(input), after = MicCheck.rms(coded)
        #expect(abs(20 * log10(after / before)) < 1, "same loudness after the codec: \(before) → \(after)")
    }

    @Test func renderResamplesAndSaysWhetherTheCodecRan() {
        let input44 = (0..<44_100).map { Float(0.2 * sin(2 * .pi * 440 * Double($0) / 44_100)) }
        let rendered = MicCheck.render(input44, sampleRate: 44_100, bitrate: 64_000, mode: .pushToTalk, threshold: -50)
        #expect(rendered.throughCodec)
        #expect(abs(rendered.samples.count - 48_000) < 4096, "at 48 kHz")
        #expect(MicCheck.rms(rendered.samples) > 0.1, "push to talk acts like always on here")
    }

    @MainActor
    @Test func theSyntheticSourceIsTalkWithPauses() async {
        let check = MicCheck(source: .synthetic)
        let samples = MicCheck.syntheticSpeech(seconds: 1.2)
        #expect(MicCheck.rms(samples[0..<Int(Self.rate * 0.8)]) > 0.05)
        #expect(samples[Int(Self.rate * 0.85)...].allSatisfy { $0 == 0 })
        #expect(check.phase == .idle && !check.isRunning)
    }
}
