import Foundation
import Testing
@testable import YapVoice

/// PROTOCOL.md › Voice › Call report: the notes, the text, the files.
@MainActor
@Suite("Call report")
struct CallReportTests {
    /// A clock the test turns by hand.
    final class Clock {
        var now = Date(timeIntervalSince1970: 1_791_700_000)
        func advance(_ seconds: TimeInterval) { now += seconds }
    }

    static let addresses = ["192.168.1.23", "10.0.0.5", "203.0.113.9", "2001:db8::5", "fe80::1", "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
                            "turn:relay.example.com", "stun:stun.example.com", "4bd1e2a3-0c6f-4c4e-9d57-2b3f1a0e9c11.local"]

    /// A stats report the way libwebrtc writes it, addresses and all.
    static func rows(received: Int, lost: Int, discarded: Int, bytesIn: Double, bytesOut: Double, concealed: Double = 0,
                     samples: Double, rtt: Double = 0.038) -> [String: StatsRow] {
        func n(_ v: Double) -> NSNumber { NSNumber(value: v) }
        return [
            "T01": StatsRow(type: "transport", values: ["selectedCandidatePairId": "CP1" as NSString]),
            "CP1": StatsRow(type: "candidate-pair", values: [
                "localCandidateId": "IL1" as NSString, "remoteCandidateId": "IR1" as NSString,
                "nominated": NSNumber(value: true), "state": "succeeded" as NSString, "currentRoundTripTime": n(rtt),
            ]),
            "IL1": StatsRow(type: "local-candidate", values: [
                "candidateType": "host" as NSString, "address": "192.168.1.23" as NSString, "ip": "192.168.1.23" as NSString,
                "port": n(50_123), "protocol": "udp" as NSString, "foundation": "1" as NSString,
            ]),
            "IR1": StatsRow(type: "remote-candidate", values: [
                "candidateType": "srflx" as NSString, "address": "203.0.113.9" as NSString, "relatedAddress": "10.0.0.5" as NSString,
                "url": "turn:relay.example.com:3478?transport=udp" as NSString, "port": n(61_000),
            ]),
            "IR2": StatsRow(type: "remote-candidate", values: [
                "candidateType": "host" as NSString, "address": "2001:db8::5" as NSString,
            ]),
            "IIT": StatsRow(type: "inbound-rtp", values: [
                "kind": "audio" as NSString, "mid": "0" as NSString, "packetsReceived": n(Double(received)), "packetsLost": n(Double(lost)),
                "packetsDiscarded": n(Double(discarded)), "bytesReceived": n(bytesIn), "totalSamplesReceived": n(samples),
                "concealedSamples": n(concealed), "silentConcealedSamples": n(0), "jitterBufferDelay": n(samples * 0.04),
                "jitterBufferEmittedCount": n(samples), "codecId": "CI" as NSString,
            ]),
            "OUT": StatsRow(type: "outbound-rtp", values: [
                "kind": "audio" as NSString, "bytesSent": n(bytesOut), "packetsSent": n(Double(received)), "codecId": "CO" as NSString,
            ]),
            "CI": StatsRow(type: "codec", values: ["mimeType": "audio/opus" as NSString]),
            "CO": StatsRow(type: "codec", values: ["mimeType": "audio/opus" as NSString]),
        ]
    }

    static func report(at date: Date, settings: [String] = ["Quality: 128 kbps"]) -> CallReport {
        CallReport(started: date, app: "Yap for iPhone 1.0 (7)", device: "iPhone17,1, iOS 26.2 (23C55)", channel: "Lounge",
                   settings: settings, audio: "in iPhone Microphone (MicrophoneBuiltIn), out Speaker (Speaker)")
    }

    @Test func noAddressesEverLeave() throws {
        let clock = Clock()
        let notes = CallNotes(store: nil)
        notes.now = { clock.now }
        // Free text can carry addresses too: a name, a device, the audio route.
        var fresh = CallReport(started: clock.now, app: "Yap for iPhone 1.0 (7)", device: "iPhone at 10.0.0.5",
                               channel: "Lounge", settings: ["Quality: 128 kbps", "Proxy: fe80::1%en0"], audio: "in USB mic (2001:db8::5)")
        fresh.add("set up", at: 0)
        notes.begin(fresh)
        var stats = PeerStats.read(Self.rows(received: 0, lost: 0, discarded: 0, bytesIn: 0, bytesOut: 0, samples: 0), sid: "s-ana", voiceMid: "0")
        stats.sendsRed = true
        stats.opusFmtpAsked = "minptime=10;useinbandfec=1;maxaveragebitrate=128000"
        #expect(stats.localCandidateType == "host" && stats.remoteCandidateType == "srflx", "types, not addresses")
        notes.stats([CallNotes.Reading(uid: "u-ana", name: "Ana 192.168.1.23", connected: true, stats: stats)])
        for second in stride(from: 2, through: 20, by: 2) {
            clock.advance(2)
            var later = PeerStats.read(Self.rows(received: second * 50, lost: second / 2, discarded: second * 48, bytesIn: Double(second) * 32_000,
                                                 bytesOut: Double(second) * 32_000, concealed: Double(second) * 10, samples: Double(second) * 48_000),
                                       sid: "s-ana", voiceMid: "0")
            later.sendsRed = true
            notes.stats([CallNotes.Reading(uid: "u-ana", name: "Ana 192.168.1.23", connected: true, stats: later)])
        }
        notes.note("ICE failed at turn:relay.example.com:3478 via 4bd1e2a3-0c6f-4c4e-9d57-2b3f1a0e9c11.local and 2001:0db8:85a3:0000:0000:8a2e:0370:7334")
        notes.end("You left.")
        let text = try #require(notes.text(live: false))
        print(text)
        for address in Self.addresses {
            #expect(!text.contains(address), "\(address) leaked")
        }
        #expect(text.range(of: #"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}"#, options: .regularExpression) == nil)
        #expect(text.range(of: #"(?i)[0-9a-f]{0,4}::[0-9a-f]"#, options: .regularExpression) == nil)
        #expect(text.contains("Connection: direct (host ↔ srflx)"))
        #expect(text.contains("Codec: red+opus out, red+opus in"))
        #expect(text.contains("Ana [address]"), "the rest of the name stays")
        #expect(text.contains("+00:10 Ana [address]: rtt 38 ms, lost 1.0 %, patched 0.0 %, 256 kbps in, 256 out"))
        #expect(text.hasPrefix("Yap call report\nApp: Yap for iPhone 1.0 (7)\n"))
        #expect(text.contains("Ended: You left."))
    }

    @Test func timelineKeepsTheLast2000() throws {
        let clock = Clock()
        let notes = CallNotes(store: nil)
        notes.now = { clock.now }
        notes.begin(Self.report(at: clock.now))
        notes.person("u-ana", "Ana")
        for i in 1...2_100 {
            clock.advance(1)
            notes.note("line \(i)")
        }
        let report = try #require(notes.report)
        #expect(report.timeline.count == CallReport.maxLines)
        #expect(report.timeline.first?.text == "line 101", "the oldest go first")
        #expect(report.droppedLines == 100)
        let text = report.text(live: true)
        #expect(text.contains("(100 earlier lines left out: only the last 2000 are kept)"))
        #expect(text.contains("People\nAna\n  Connection: never connected"), "the summary never goes")
        #expect(text.contains("+35:00 line 2100"))
        #expect(text.contains("Ended: not yet (copied during the call)"))
    }

    @Test func stampsPercentsAndScrubbing() {
        #expect(CallReport.stamp(0) == "+00:00")
        #expect(CallReport.stamp(75.9) == "+01:15")
        #expect(CallReport.stamp(3725) == "+1:02:05")
        #expect(CallReport.percent(4, of: 1000) == "0.4 %")
        #expect(CallReport.percent(0, of: 10) == "0 %")
        #expect(CallReport.percent(25, of: 100) == "25 %")
        let harmless = "+12:34 rtt 38 ms, v0.40.1, iOS 26.2.1 (23C55), Sat 11 Oct 2026, 09:20:31, +1:02:05, minptime=10;useinbandfec=1"
        #expect(CallReport.scrub(harmless) == harmless, "times, versions and fmtp aren't addresses")
        #expect(CallReport.scrub("::ffff:192.0.2.1 and ::1") == "[address] and [address]")
        #expect(CallReport.scrub("stuns:stun.example.com:5349, then") == "[server], then")
    }

    /// A line per person every 10 s for the first minute, then every minute.
    @Test func summaryEveryTenSecondsThenEveryMinute() throws {
        let clock = Clock()
        let notes = CallNotes(store: nil)
        notes.now = { clock.now }
        notes.begin(Self.report(at: clock.now))
        for second in stride(from: 0, through: 180, by: 2) {
            let stats = PeerStats.read(Self.rows(received: second * 50, lost: 0, discarded: 0, bytesIn: Double(second) * 16_000,
                                                 bytesOut: Double(second) * 16_000, samples: Double(second) * 48_000), sid: "s-ben", voiceMid: "0")
            notes.stats([CallNotes.Reading(uid: "u-ben", name: "Ben", connected: true, stats: stats)])
            clock.advance(2)
        }
        let stamps = try #require(notes.report).timeline.filter { $0.text.hasPrefix("Ben: rtt") }.map { CallReport.stamp($0.at) }
        #expect(stamps == ["+00:10", "+00:20", "+00:30", "+00:40", "+00:50", "+01:00", "+02:00", "+03:00"])
        let text = try #require(notes.text(live: true))
        #expect(text.contains("Codec: opus out, opus in"))
        #expect(text.contains("Sound: 128 kbps in, 128 out (median)"))
        #expect(text.contains("RED rebuilt: no copies arrived"))
    }

    @Test func storeKeepsTheLastThree() throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: "yap-call-reports-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = CallReportStore(directory: directory)
        for i in 0..<5 {
            var report = Self.report(at: Date(timeIntervalSince1970: 1_791_700_000 + Double(i) * 3600))
            report.add("call \(i)", at: 1)
            try store.save(report)
        }
        #expect(store.files().count == 3)
        #expect(store.all().map { $0.timeline.first?.text } == ["call 4", "call 3", "call 2"])

        // An unfinished one (iOS stopped Yap mid-call): kept, and it says so.
        let notes = CallNotes(store: store)
        #expect(notes.hasReport, "Settings can copy the one from before")
        let text = try #require(notes.text(live: false))
        #expect(text.contains("call 4"))
        #expect(text.contains("Ended: unknown. Yap stopped taking notes at +00:00"))
        #expect(!CallNotes(store: CallReportStore(directory: directory.appending(path: "empty"))).hasReport, "No call yet.")
    }
}
