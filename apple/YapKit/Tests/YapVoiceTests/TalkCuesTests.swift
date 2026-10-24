import Foundation
import Testing
@testable import YapCore
@testable import YapVoice

/// Round 10 (PROTOCOL.md › Voice › Transmit gate, Connection hints, Call
/// report): the muted nudge's 5 s, push to talk's double-tap, the meter's
/// "Too loud", the shared-media dip, your own connection, and the words.
@Suite("Talk cues")
struct TalkCuesTests {
    // MARK: "You're muted"

    @Test func nudgeAtMostEveryFiveSecondsBlipOncePerMute() {
        var nudge = MutedNudge()
        let r1 = nudge.talking(at: 100)
        #expect(r1 == .blip, "the first since muting blips")
        let r2 = nudge.talking(at: 100.025)
        #expect(r2 == nil)
        let r3 = nudge.talking(at: 104.9)
        #expect(r3 == nil)
        let r4 = nudge.talking(at: 105.0)
        #expect(r4 == .quiet, "5 s later, again: just the words")
        let r5 = nudge.talking(at: 109)
        #expect(r5 == nil)
        let r6 = nudge.talking(at: 200)
        #expect(r6 == .quiet, "a new bout of talking, still the same mute")
        nudge.muted()
        let r7 = nudge.talking(at: 300)
        #expect(r7 == .blip, "muted again: the blip again")
    }

    @Test func noNudgeWhileAnotherOfMyDevicesIsLive() {
        func live(_ voice: VoiceState?, uid: String = "u1", sid: String = "s2", client: String = "web") -> Bool {
            let mine = Session(sid: "s1", uid: "u1", voice: VoiceState(channelId: "c1", muted: true, deafened: false))
            let other = Session(sid: sid, uid: uid, client: client, voice: voice)
            return MutedNudge.liveElsewhere([mine, other], uid: "u1", mySid: "s1", channelId: "c1")
        }
        #expect(live(VoiceState(channelId: "c1", muted: false, deafened: false)), "my PC, unmuted in this call")
        #expect(!live(VoiceState(channelId: "c1", muted: true, deafened: false)), "muted there too")
        #expect(!live(VoiceState(channelId: "c1", muted: false, deafened: true)), "deafened there")
        #expect(!live(VoiceState(channelId: "c2", muted: false, deafened: false)), "another call")
        #expect(!live(nil), "not in voice")
        #expect(!live(VoiceState(channelId: "c1", muted: false, deafened: false), uid: "u2"), "someone else")
        #expect(!live(VoiceState(channelId: "c1", muted: false, deafened: false), sid: "s1"), "this device")
        #expect(!live(VoiceState(channelId: "c1", muted: false, deafened: false), client: "obs"), "an OBS link")
    }

    // MARK: Push to talk: hold, double-tap, tap to stop

    @Test func holdIsDownWhileHeld() {
        var latch = PushToTalkLatch()
        latch.press(at: 1)
        #expect(latch.isDown && !latch.latched)
        latch.release()
        #expect(!latch.isDown)
        latch.press(at: 2)
        #expect(latch.isDown && !latch.latched, "a second press 1 s later is just a press")
        latch.release()
        #expect(!latch.isDown)
    }

    @Test func doubleTapStaysLiveUntilTheNextPress() {
        var latch = PushToTalkLatch()
        latch.press(at: 10)
        latch.release()
        latch.press(at: 10.35)
        #expect(latch.latched, "twice within 400 ms")
        latch.release()
        #expect(latch.isDown, "stays live after letting go")
        // Tap to stop: the press ends the latch; it closes when it lets go.
        latch.press(at: 20)
        #expect(!latch.latched && latch.isDown)
        latch.release()
        #expect(!latch.isDown)
        // That stopping press isn't the first of a new double-tap.
        latch.press(at: 20.2)
        #expect(!latch.latched)
        latch.release()
    }

    @Test func slowDoubleTapIsTwoPresses() {
        var latch = PushToTalkLatch()
        latch.press(at: 5)
        latch.release()
        latch.press(at: 5.41)
        #expect(!latch.latched)
        latch.release()
        #expect(!latch.isDown)
    }

    @Test func pressesWhileHeldAreIgnoredAndResetClears() {
        var latch = PushToTalkLatch()
        latch.press(at: 1)
        latch.press(at: 1.1) // key repeat
        #expect(!latch.latched)
        latch.release()
        latch.press(at: 1.3)
        #expect(latch.latched)
        latch.reset()
        #expect(!latch.isDown && !latch.latched)
    }

    // MARK: Too loud (clipping)

    @Test func clippingNeedsRepeatedPeaks() {
        var clip = ClipWatch()
        let r7 = clip.update(peakDb: -0.5, at: 0)
        #expect(!r7, "one peak isn't a warning")
        let r8 = clip.update(peakDb: -0.5, at: 0.05)
        #expect(!r8, "the same ~110 ms reading again")
        let r9 = clip.update(peakDb: -3, at: 0.5)
        #expect(!r9, "below −1 dBFS")
        let r10 = clip.update(peakDb: -1, at: 0.8)
        #expect(!r10)
        let r11 = clip.update(peakDb: 0, at: 1.2)
        #expect(r11, "a third peak within 2 s")
        let r12 = clip.update(peakDb: -20, at: 4.1)
        #expect(r12, "shown 3 s after the last time it held")
        let r13 = clip.update(peakDb: -20, at: 4.3)
        #expect(!r13)
    }

    @Test func spreadOutPeaksAreFine() {
        var clip = ClipWatch()
        for t in stride(from: 0.0, to: 20, by: 1.1) {
            let r14 = clip.update(peakDb: -0.2, at: t)
            #expect(!r14, "one peak a second is someone laughing, not clipping")
        }
    }

    @Test func peakOfABuffer() {
        #expect(ClipWatch.peakDb([0, 0.5, -0.999, 0.2] as [Float]) > -0.01)
        #expect(abs(ClipWatch.peakDb([0.1, -0.05] as [Float]) - -20) < 0.01)
        #expect(ClipWatch.peakDb([Float]()) == VoiceGate.silenceDb)
    }

    // MARK: Lower shared media while friends talk

    @Test func dipEasesDownAndBack() {
        var dip = MediaDip()
        var t = 0.0
        func step(_ speaking: Bool, for seconds: Double, on: Bool = true) -> Double {
            var factor = dip.factor
            let end = t + seconds
            while t < end - 1e-9 {
                t += 0.025
                factor = dip.update(on: on, othersSpeaking: speaking, at: t)
            }
            return factor
        }
        let d1 = step(false, for: 1)
        #expect(d1 == 1, "nobody talking: your volume")
        let early = step(true, for: 0.075)
        #expect(early < 1 && early > 0.5, "easing down: \(early)")
        #expect(abs(step(true, for: 0.1) - 0.5) < 1e-9, "half within ~150 ms")
        let d2 = step(true, for: 3)
        #expect(d2 == 0.5, "stays while they talk")
        let d3 = step(false, for: 0.95)
        #expect(d3 == 0.5, "nobody for under 1 s: still down")
        let rising = step(false, for: 0.35)
        #expect(rising > 0.5 && rising < 1, "easing back: \(rising)")
        let d4 = step(false, for: 0.5)
        #expect(d4 == 1, "back within ~600 ms")
        _ = step(true, for: 0.5)
        let d5 = step(true, for: 0.3, on: false)
        #expect(d5 > 0.5 && d5 < 1, "switched off: easing back up, \(d5)")
        let d6 = step(true, for: 0.4, on: false)
        #expect(d6 == 1, "and back")
    }

    // MARK: Your own connection

    @Test func ownConnectionThreeBadReadingsOnThreeGoodOff() {
        var own = OwnLinkQuality()
        let r15 = own.update(losses: [0.06])
        #expect(!r15)
        let r16 = own.update(losses: [0.05])
        #expect(!r16)
        let r17 = own.update(losses: [0.07])
        #expect(r17 && own.weak, "three bad in a row")
        #expect(abs(own.lost - 0.07) < 1e-9)
        let r18 = own.update(losses: [0.0])
        #expect(!r18 && own.weak)
        let r19 = own.update(losses: [0.01])
        #expect(!r19 && own.weak)
        let r20 = own.update(losses: [0.0])
        #expect(r20 && !own.weak, "three good in a row")
    }

    @Test func ownConnectionNeedsMostPeople() {
        var own = OwnLinkQuality()
        for _ in 0..<5 { own.update(losses: [0.08, 0.0]) }
        #expect(!own.weak, "one of two losing ours says more about them")
        for _ in 0..<3 { own.update(losses: [0.08, 0.05, 0.0]) }
        #expect(own.weak, "two of three")
        #expect(abs(own.lost - 0.05) < 1e-9, "the median person")
        own.reset()
        #expect(!own.weak)
    }

    @Test func ownConnectionNobodyReportingStartsOver() {
        var own = OwnLinkQuality()
        for _ in 0..<3 { own.update(losses: [0.1]) }
        #expect(own.weak)
        let r21 = own.update(losses: [])
        #expect(r21 && !own.weak, "nobody to judge by (alone, no reports yet): no hint")
        own.update(losses: [0.1])
        own.update(losses: [0.1])
        #expect(!own.weak, "the count starts over")
        let r22 = own.update(losses: [0.1])
        #expect(r22 && own.weak)
        let r23 = own.update(losses: [0.03])
        #expect(!r23 && own.weak, "exactly 3 % isn't more than 3 %")
    }

    // MARK: remote-inbound-rtp in the stats

    static func ownRows(fractionLost: Double, lost: Int, rtt: Double, measurements: Int) -> [String: StatsRow] {
        func n(_ v: Double) -> NSNumber { NSNumber(value: v) }
        return [
            "OUT0": StatsRow(type: "outbound-rtp", values: ["kind": "audio" as NSString, "mid": "0" as NSString, "packetsSent": n(1000), "bytesSent": n(32_000)]),
            "OUTV": StatsRow(type: "outbound-rtp", values: ["kind": "video" as NSString, "mid": "1" as NSString]),
            "RI0": StatsRow(type: "remote-inbound-rtp", values: [
                "kind": "audio" as NSString, "localId": "OUT0" as NSString, "fractionLost": n(fractionLost), "packetsLost": n(Double(lost)),
                "roundTripTime": n(rtt), "roundTripTimeMeasurements": n(Double(measurements)),
            ]),
            "RIV": StatsRow(type: "remote-inbound-rtp", values: [
                "kind": "video" as NSString, "localId": "OUTV" as NSString, "fractionLost": n(0.5), "packetsLost": n(900),
            ]),
        ]
    }

    @Test func readsWhatTheyReportReceivingFromUs() {
        let stats = PeerStats.read(Self.ownRows(fractionLost: 0.046875, lost: 12, rtt: 0.041, measurements: 3), sid: "s1", voiceMid: "0")
        #expect(stats.remoteFractionLost == 0.046875)
        #expect(stats.remotePacketsLost == 12)
        #expect(abs((stats.remoteRttMs ?? 0) - 41) < 1e-9)
        #expect(stats.remoteReports == 3)
        #expect(stats.packetsSent == 1000)
        let none = PeerStats.read([:], sid: "s1", voiceMid: "0")
        #expect(none.remoteFractionLost == nil && none.remoteRttMs == nil && none.remoteReports == 0, "before their first report")
    }

    @Test func picksTheVoicesReportAmongTwo() {
        func n(_ v: Double) -> NSNumber { NSNumber(value: v) }
        var rows = Self.ownRows(fractionLost: 0.01, lost: 1, rtt: 0.02, measurements: 1)
        rows["OUT2"] = StatsRow(type: "outbound-rtp", values: ["kind": "audio" as NSString, "mid": "2" as NSString])
        rows["RI2"] = StatsRow(type: "remote-inbound-rtp", values: ["kind": "audio" as NSString, "localId": "OUT2" as NSString, "fractionLost": n(0.9)])
        #expect(PeerStats.read(rows, sid: "s1", voiceMid: "0").remoteFractionLost == 0.01)
    }

    // MARK: The call report's "they lost X % of yours"

    @MainActor
    @Test func callReportSaysWhatTheyLostOfYours() throws {
        let clock = CallReportTests.Clock()
        let notes = CallNotes(store: nil)
        notes.now = { clock.now }
        notes.begin(CallReportTests.report(at: clock.now))
        notes.person("u-ana", "Ana")
        func reading(sent: Int, theyLost: Int, rtt: Double?, reports: Int) -> CallNotes.Reading {
            var s = PeerStats.read(CallReportTests.rows(received: sent, lost: 0, discarded: 0, bytesIn: 0, bytesOut: Double(sent) * 320, samples: Double(sent) * 960),
                                   sid: "s-ana", voiceMid: "0")
            s.packetsSent = sent
            s.remotePacketsLost = theyLost
            s.remoteRttMs = rtt
            s.remoteReports = reports
            return CallNotes.Reading(uid: "u-ana", name: "Ana", connected: true, stats: s)
        }
        notes.stats([reading(sent: 100, theyLost: 0, rtt: nil, reports: 0)])
        clock.advance(2)
        notes.stats([reading(sent: 200, theyLost: 2, rtt: 40, reports: 1)])
        clock.advance(2)
        notes.stats([reading(sent: 500, theyLost: 10, rtt: 42, reports: 2)])
        notes.end("You left.")
        let text = try #require(notes.text(live: false))
        #expect(text.contains("From you: they lost 2.0 % of yours (their round trip 41 ms median)"), "\(text)")
    }

    @MainActor
    @Test func olderReportsWithoutTheNewFieldsStillOpen() throws {
        var report = CallReportTests.report(at: Date(timeIntervalSince1970: 1_791_700_000))
        _ = report.person("u-ana", name: "Ana")
        var json = try #require(try JSONSerialization.jsonObject(with: JSONEncoder().encode(report)) as? [String: Any])
        var people = try #require(json["people"] as? [[String: Any]])
        for key in ["sentToThem", "theyLost", "theirRttMs"] { people[0].removeValue(forKey: key) }
        json["people"] = people
        let decoded = try JSONDecoder().decode(CallReport.self, from: JSONSerialization.data(withJSONObject: json))
        #expect(decoded.people.first?.sentToThem == nil)
        #expect(!decoded.text().contains("of yours"), "nothing to say about it")
    }

    // MARK: Settings and words

    @Test func newSettingsDefaultsAndOlderSaves() throws {
        let fresh = VoiceSettings()
        #expect(fresh.mutedReminder && fresh.pushToTalkClick && !fresh.dipSharedMedia)
        let old = try JSONDecoder().decode(VoiceSettings.self, from: Data(#"{"mode":"pushToTalk","bitrate":96000,"muted":true,"vadScale":3}"#.utf8))
        #expect(old.mutedReminder, "on unless turned off")
        #expect(old.pushToTalkClick)
        #expect(!old.dipSharedMedia, "off by default")
        var changed = old
        changed.mutedReminder = false
        changed.pushToTalkClick = false
        changed.dipSharedMedia = true
        let back = try JSONDecoder().decode(VoiceSettings.self, from: JSONEncoder().encode(changed))
        #expect(!back.mutedReminder && !back.pushToTalkClick && back.dipSharedMedia)
    }

    @Test func qualityWarningOnlyAbove128() {
        for bitrate in [64_000, 96_000, 128_000] { #expect(VoiceSettings.qualityWarning(bitrate) == nil) }
        for bitrate in [256_000, 510_000] {
            #expect(VoiceSettings.qualityWarning(bitrate) == "No dropout protection at this quality (each bit of voice no longer fits twice in a packet), and several times the upload for each friend in the call.")
        }
    }

    @Test func cueSoundsAreShortAndQuiet() {
        for tone in [ChimeTone.mutedNudge, .pushToTalkOn, .pushToTalkOff] {
            let samples = tone.render()
            let peak = samples.reduce(Float(0)) { max($0, abs($1)) }
            #expect(!samples.isEmpty)
            #expect(Double(samples.count) / 48_000 < 0.35, "\(tone.id) is short")
            #expect(Double(peak) <= tone.volume * 1.01 && tone.volume <= 0.05, "\(tone.id) is soft")
        }
        #expect(ChimeTone.pushToTalkOn.notes != ChimeTone.pushToTalkOff.notes, "opening and closing sound different")
    }
}
