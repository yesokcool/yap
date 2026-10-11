#if os(iOS)
import Foundation
import Testing
@testable import YapCore
@testable import YapVoice
@preconcurrency import WebRTC

/// RED over a real WebRTC connection inside one process (PROTOCOL.md ›
/// Voice › Redundancy): two engines with synthetic mics, wired signal to
/// signal. With RED first in both descriptions both sides send it, and the
/// Opus target stays at the quality setting: the copy rides on top, so the
/// wire carries about twice the bits. With it off, plain Opus as before.
/// libwebrtc's stats name the codec inside (`audio/opus`) either way; RED
/// shows as twice the bytes per packet, and on the receiving side as one
/// discarded packet per packet (each copy of a frame that already came).
@MainActor
@Suite("RED between two engines", .serialized)
final class RedLoopbackTests {
    struct Measured {
        var outCodec: String?
        var inCodec: String?
        var sendKbps: Double
        var receiveKbps: Double
        var bytesPerPacket: Double
        /// Copies the far side's jitter buffer threw away, per packet it received.
        var discardedPerPacket: Double
        var sendsRed: Bool
        var asksRed: Bool
        var offer: String
        var answer: String
    }

    private func waitUntil(_ what: String, timeout: Duration = .seconds(15), _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + timeout
        while !condition() {
            if ContinuousClock.now > deadline {
                Issue.record("timed out waiting for: \(what)")
                throw CancellationError()
            }
            try await Task.sleep(for: .milliseconds(50))
        }
    }

    /// A call between two engines at `bitrate`, measured over 4 s after it settles.
    private func call(redundancy: Bool, bitrate: Int = 128_000, dump: Bool = false) async throws -> Measured {
        let a = VoiceEngine(audioDevice: SyntheticAudioDevice())
        let b = VoiceEngine(audioDevice: SyntheticAudioDevice())
        defer {
            a.stop()
            b.stop()
        }
        for (engine, sid) in [(a, "s-a"), (b, "s-b")] {
            engine.mySid = sid
            engine.mode = .openMic
            engine.bitrate = bitrate
            engine.redundancy = redundancy
        }
        a.onSignal = { [b] _, data in DispatchQueue.main.async { MainActor.assumeIsolated { b.handleSignal(data, from: "s-a") } } }
        b.onSignal = { [a] _, data in DispatchQueue.main.async { MainActor.assumeIsolated { a.handleSignal(data, from: "s-b") } } }
        a.prepare(iceServers: [])
        b.prepare(iceServers: [])
        a.enter(["s-b"])
        try await waitUntil("connected") { a.connectionState(of: "s-b") == .connected && b.connectionState(of: "s-a") == .connected }
        try await Task.sleep(for: .seconds(3))
        let s1 = try #require(await a.peerStats().first)
        try await Task.sleep(for: .seconds(4))
        let s2 = try #require(await a.peerStats().first)
        if dump {
            let pc = try #require(a.peerConnection("s-b"))
            let report = await pc.statistics()
            for s in report.statistics.values where ["outbound-rtp", "inbound-rtp", "codec", "media-source"].contains(s.type) {
                if s.type != "codec", s.values["kind"] as? String != "audio" { continue }
                let pairs = s.values.keys.sorted().map { "\($0)=\(s.values[$0].map { "\($0)" } ?? "")" }.joined(separator: " ")
                print("RED stats \(s.type): \(pairs)")
            }
        }
        let packets = Double(s2.packetsSent - s1.packetsSent)
        let received = Double(s2.voicePacketsReceived - s1.voicePacketsReceived)
        return Measured(outCodec: s2.outboundCodec, inCodec: s2.inboundCodec,
                        sendKbps: (s2.bytesSent - s1.bytesSent) * 8 / 4000,
                        receiveKbps: (s2.bytesReceived - s1.bytesReceived) * 8 / 4000,
                        bytesPerPacket: packets > 0 ? (s2.bytesSent - s1.bytesSent) / packets : 0,
                        discardedPerPacket: received > 0 ? Double(s2.voicePacketsDiscarded - s1.voicePacketsDiscarded) / received : 0,
                        sendsRed: s2.sendsRed, asksRed: s2.asksRed,
                        offer: a.peerLocalDescription("s-b") ?? "", answer: b.peerLocalDescription("s-a") ?? "")
    }

    @Test func redBothWaysOpusTargetKept() async throws {
        let caps = VoiceEngine(audioDevice: SyntheticAudioDevice()).factory.rtpSenderCapabilities(forKind: kRTCMediaStreamTrackKindAudio).codecs.map(\.name)
        print("RED this WebRTC's audio codecs: \(caps)")
        #expect(caps.contains { $0.lowercased() == "red" }, "libwebrtc offers RED without a field trial")

        let red = try await call(redundancy: true, dump: true)
        let plain = try await call(redundancy: false)
        print("RED on:  sent \(red.outCodec ?? "?") \(Int(red.sendKbps)) kbps (\(Int(red.bytesPerPacket)) B/packet), received \(red.inCodec ?? "?") \(Int(red.receiveKbps)) kbps, copies discarded per packet \(String(format: "%.2f", red.discardedPerPacket)); offer \(SDPTuner.audioCodecOrder(in: red.offer).first?.prefix(3) ?? []), answer \(SDPTuner.audioCodecOrder(in: red.answer).first?.prefix(3) ?? [])")
        print("RED off: sent \(plain.outCodec ?? "?") \(Int(plain.sendKbps)) kbps (\(Int(plain.bytesPerPacket)) B/packet), received \(plain.inCodec ?? "?") \(Int(plain.receiveKbps)) kbps, copies discarded per packet \(String(format: "%.2f", plain.discardedPerPacket))")
        #expect(SDPTuner.audioCodecOrder(in: red.offer).first?.prefix(2) == ["red", "opus"])
        #expect(SDPTuner.audioCodecOrder(in: red.answer).first?.prefix(2) == ["red", "opus"])
        #expect(red.sendsRed && red.asksRed && !plain.sendsRed && !plain.asksRed)
        #expect(red.discardedPerPacket > 0.9, "a copy of every frame arrives (and goes unused without loss): \(red.discardedPerPacket)")
        #expect(plain.discardedPerPacket < 0.1, "plain Opus: no copies")
        #expect(red.outCodec == "audio/opus" && plain.outCodec == "audio/opus", "the stats name the codec inside")
        // The copy rides on top of the Opus target (maxBitrate doesn't count it).
        #expect(red.bytesPerPacket > plain.bytesPerPacket * 1.7, "about twice the bytes per packet: \(red.bytesPerPacket) vs \(plain.bytesPerPacket)")
        #expect(plain.sendKbps > 128 * 0.85, "plain Opus at the setting: \(plain.sendKbps)")
        #expect(red.sendKbps > 2 * 128 * 0.85, "the copy on top: \(red.sendKbps)")
    }

    /// At 256 kbps a frame and its copy would be ~1285 bytes, at 510 kbps a
    /// frame alone is over 1023 (RED's block length): libwebrtc leaves the
    /// copy out rather than send a payload over 1200 bytes.
    @Test(arguments: [96_000, 256_000, 510_000])
    func copyOnlyWhereItFits(bitrate: Int) async throws {
        let red = try await call(redundancy: true, bitrate: bitrate)
        let plain = try await call(redundancy: false, bitrate: bitrate)
        print("RED \(bitrate / 1000)k on:  sent \(Int(red.sendKbps)) kbps (\(Int(red.bytesPerPacket)) B/packet), copies discarded per packet \(String(format: "%.2f", red.discardedPerPacket))")
        print("RED \(bitrate / 1000)k off: sent \(Int(plain.sendKbps)) kbps (\(Int(plain.bytesPerPacket)) B/packet)")
        // A copy only when frame + copy fit in 1200 bytes (a big frame still goes, alone).
        #expect(red.bytesPerPacket < max(1210, plain.bytesPerPacket + 8), "never past RED's limits: \(red.bytesPerPacket)")
        if bitrate <= 128_000 { #expect(red.discardedPerPacket > 0.9, "the copy fits") }
    }
}
#endif
