import Foundation
import Testing
import YapCore
@testable import YapVoice

/// PROTOCOL.md › Voice › Transmit gate, as the engine drives it: a reading
/// of the mic's RMS level every ~25 ms.
@Suite("Transmit gate")
struct VoiceGateTests {
    static let step = 0.025

    /// Feed `seconds` of readings at `db` from `t`; returns the open/closed answers.
    @discardableResult
    static func feed(_ gate: inout VoiceGate, db: Double, from t: inout TimeInterval, for seconds: Double) -> [Bool] {
        var out: [Bool] = []
        let end = t + seconds - 1e-9
        while t < end {
            t += step
            out.append(gate.update(db: db, duration: step, at: t))
        }
        return out
    }

    @Test func opensAboveTheSensitivityAndHoldsFor450ms() {
        var gate = VoiceGate()
        var t = 0.0
        #expect(!Self.feed(&gate, db: -60, from: &t, for: 1).contains(true), "a quiet room doesn't open it")
        #expect(Self.feed(&gate, db: -30, from: &t, for: 0.025) == [true], "opens on the first loud reading")
        Self.feed(&gate, db: -70, from: &t, for: 0.425)
        #expect(gate.isOpen, "held 425 ms later")
        Self.feed(&gate, db: -70, from: &t, for: 0.05)
        #expect(!gate.isOpen, "closed once the 450 ms hold ran out")
    }

    @Test func hysteresisKeepsItOpenWithin6dB() {
        // Open on −45, then a softer stretch: −54 is under the sensitivity
        // but within 6 dB of it, so it keeps renewing the hold; −57 doesn't.
        var soft = VoiceGate(threshold: -50)
        var t = 0.0
        Self.feed(&soft, db: -45, from: &t, for: 0.05)
        #expect(soft.isOpen)
        Self.feed(&soft, db: -54, from: &t, for: 0.45)
        #expect(soft.isOpen, "still open: the hold ran from the last −54")
        var softer = VoiceGate(threshold: -50)
        t = 0
        Self.feed(&softer, db: -45, from: &t, for: 0.05)
        Self.feed(&softer, db: -57, from: &t, for: 0.45)
        #expect(!softer.isOpen, "closed: the hold ran from the last −45")
        // And −54 doesn't open a closed gate.
        var closed = VoiceGate(threshold: -50)
        t = 0
        #expect(!Self.feed(&closed, db: -54, from: &t, for: 1).contains(true))
    }

    @Test func adaptiveNoiseFloorRaisesTheBarInANoisyRoom() {
        var gate = VoiceGate(threshold: -50)
        var t = 0.0
        // A room at −48 dBFS: louder than the sensitivity. It opens at first…
        let first = Self.feed(&gate, db: -48, from: &t, for: 0.1)
        #expect(first.contains(true))
        // …but within a few seconds the floor is known and the room alone closes it.
        Self.feed(&gate, db: -48, from: &t, for: 4.5)
        #expect(abs(gate.floor + 48) < 0.01)
        #expect(gate.openAt == -36 && gate.closeAt == -40, "floor + 12 to open, + 8 to stay open")
        #expect(!gate.isOpen)
        // Speech well over the room opens it; the room closes it again.
        #expect(Self.feed(&gate, db: -30, from: &t, for: 0.05).contains(true))
        Self.feed(&gate, db: -48, from: &t, for: 0.5)
        #expect(!gate.isOpen)
    }

    @Test func theFloorIsCappedSoALoudRoomStillGetsThrough() {
        var gate = VoiceGate(threshold: -50)
        var t = 0.0
        Self.feed(&gate, db: -25, from: &t, for: 4.5)
        #expect(gate.openAt == -30, "never asks for more than −30 dBFS")
        #expect(gate.closeAt == -36)
        // The quietest reading of each 500 ms counts: a dip lowers the floor.
        Self.feed(&gate, db: -60, from: &t, for: 0.025)
        Self.feed(&gate, db: -25, from: &t, for: 0.5)
        #expect(gate.floor == -60)
        #expect(gate.openAt == -48 && gate.closeAt == -52)
    }

    @Test func digitalSilenceIsIgnoredByTheFloorAndReported() {
        var gate = VoiceGate(threshold: -50)
        var t = 0.0
        Self.feed(&gate, db: -60, from: &t, for: 1)
        Self.feed(&gate, db: VoiceGate.silenceDb, from: &t, for: 4.9)
        #expect(gate.floor == -60, "silence says nothing about the room")
        #expect(!gate.isSilent)
        Self.feed(&gate, db: VoiceGate.silenceDb, from: &t, for: 0.15)
        #expect(gate.isSilent, "5 s of nothing at all")
        Self.feed(&gate, db: -70, from: &t, for: 0.025)
        #expect(!gate.isSilent, "any sound clears it")
    }

    @Test func pushToTalkHasA200msTail() {
        var gate = VoiceGate(mode: .pushToTalk)
        var t = 0.0
        #expect(!Self.feed(&gate, db: -20, from: &t, for: 0.5).contains(true), "talking alone doesn't open push to talk")
        let r1 = gate.setPushToTalk(true, at: t)
        #expect(r1)
        Self.feed(&gate, db: -80, from: &t, for: 1)
        #expect(gate.isOpen, "open while held, loud or not")
        gate.setPushToTalk(false, at: t)
        let r2 = gate.decide(at: t + 0.19)
        #expect(r2, "still open 190 ms after release")
        let r3 = gate.decide(at: t + 0.21)
        #expect(!r3, "closed after the 200 ms tail")
    }

    @Test func alwaysOnIsAlwaysOpenAndModesSwitchCleanly() {
        var gate = VoiceGate(mode: .alwaysOn)
        var t = 0.0
        #expect(Self.feed(&gate, db: VoiceGate.silenceDb, from: &t, for: 1).allSatisfy { $0 })
        let r4 = gate.setMode(.voiceActivity, at: t)
        #expect(!r4, "voice activity starts closed")
        let r5 = gate.setMode(.alwaysOn, at: t)
        #expect(r5)
    }

    /// Syllables (150 ms on, 120 ms off) over a quiet room, read every 25 ms
    /// from WebRTC's counters: opens within one reading and stays open
    /// through the gaps.
    @Test func opensQuicklyOnSyllables() {
        let speech = -26.0, room = -60.0
        var gate = VoiceGate()
        var t = 0.0
        var openedAt: Double?
        while t < 2 {
            t += Self.step
            let s = t - 0.5
            let db = s >= 0 && s.truncatingRemainder(dividingBy: 0.27) < 0.15 ? speech : room
            let open = gate.update(db: db, duration: Self.step, at: t)
            if t < 0.5 { #expect(!open) }
            if open, openedAt == nil { openedAt = t }
            if t > 0.55, t < 1.9 { #expect(open, "open through syllables at \(t)") }
        }
        let lag = (openedAt ?? 9) - 0.5
        #expect(lag >= 0 && lag <= 0.026, "opened \(Int(lag * 1000)) ms after the first syllable")
    }

    @Test func rmsAndDecibels() {
        // A full-scale sine: mean square 0.5 → RMS 0.707 → −3 dBFS.
        let rms = try! #require(VoiceGate.rms(fromEnergy: 1, duration: 2, toEnergy: 1 + 0.5 * 0.025, duration: 2.025))
        #expect(abs(rms - 0.5.squareRoot()) < 1e-9)
        #expect(abs(VoiceGate.dB(rms) + 3.01) < 0.01)
        #expect(VoiceGate.rms(fromEnergy: 1, duration: 2, toEnergy: 1, duration: 2) == nil, "no new audio")
        #expect(VoiceGate.dB(0) == VoiceGate.silenceDb)
        #expect(VoiceGate.linear(VoiceGate.silenceDb) == 0)
        #expect(abs(VoiceGate.linear(-20) - 0.1) < 1e-12)
    }

    @Test func settingsMoveToTheRMSScale() throws {
        // Saved long ago: the old default (a peak level) becomes −50; a picked value stays.
        let oldDefault = try JSONDecoder().decode(VoiceSettings.self, from: Data(#"{"mode":"voiceActivity","bitrate":128000,"vadThreshold":-45}"#.utf8))
        #expect(oldDefault.vadThreshold == -50)
        let picked = try JSONDecoder().decode(VoiceSettings.self, from: Data(#"{"vadThreshold":-38}"#.utf8))
        #expect(picked.vadThreshold == -38)
        // Picked on the peak-based counters (scale 2): 9 dB lower now, same voice opens it.
        let peakScale = try JSONDecoder().decode(VoiceSettings.self, from: Data(#"{"vadThreshold":-40,"vadScale":2}"#.utf8))
        #expect(peakScale.vadThreshold == -49)
        let peakDefault = try JSONDecoder().decode(VoiceSettings.self, from: Data(#"{"vadThreshold":-50,"vadScale":2}"#.utf8))
        #expect(peakDefault.vadThreshold == -50, "the default stays the default")
        let edge = try JSONDecoder().decode(VoiceSettings.self, from: Data(#"{"vadThreshold":-75,"vadScale":2}"#.utf8))
        #expect(edge.vadThreshold == -80, "kept in the slider's range")
        // Once on the RMS scale, −45 is a choice and stays.
        var chosen = VoiceSettings()
        chosen.vadThreshold = -45
        let again = try JSONDecoder().decode(VoiceSettings.self, from: try JSONEncoder().encode(chosen))
        #expect(again.vadThreshold == -45)
        #expect(VoiceSettings().vadThreshold == -50 && !VoiceSettings().relayOnly && !VoiceSettings().highQualityBluetooth)
        let on = try JSONDecoder().decode(VoiceSettings.self, from: Data(#"{"relayOnly":true,"highQualityBluetooth":true}"#.utf8))
        #expect(on.relayOnly && on.highQualityBluetooth)
    }

    @Test func relayDetection() {
        #expect(!IceServerConfig.hasRelay([IceServerConfig(urls: ["stun:stun.cloudflare.com:3478"])]))
        #expect(IceServerConfig.hasRelay([IceServerConfig(urls: ["stun:a"]), IceServerConfig(urls: ["turn:relay.example.com:3478?transport=udp"], username: "u", credential: "c")]))
        #expect(IceServerConfig.hasRelay([IceServerConfig(urls: ["TURNS:relay.example.com:5349"])]))
    }
}
