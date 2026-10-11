import Foundation
import Testing
@testable import YapVoice

@Suite("ICE restarts when the path dies")
struct PathWatchTests {
    @Test func impoliteTriesAfter3_5sPoliteAfter10s() {
        var impolite = PathWatch()
        let r1 = impolite.stateChanged(.disconnected, polite: false)
        #expect(r1 == 3.5)
        var polite = PathWatch()
        let r2 = polite.stateChanged(.disconnected, polite: true)
        #expect(r2 == 10)
        let r3 = impolite.stateChanged(.connecting, polite: false)
        #expect(r3 == nil, "nothing to do while it's trying")
        let r4 = impolite.stateChanged(.connected, polite: false)
        #expect(r4 == nil)
    }

    @Test func failedIsAtOnceForTheImpoliteSideAndMarksIt() {
        var watch = PathWatch()
        #expect(!watch.failed)
        let r5 = watch.stateChanged(.failed, polite: false)
        #expect(r5 == 0)
        #expect(watch.failed, "\"needs the voice relay\" / \"can't connect\" until it's up again")
        var polite = PathWatch()
        let r6 = polite.stateChanged(.failed, polite: true)
        #expect(r6 == 10)
        _ = watch.stateChanged(.connected, polite: false)
        #expect(!watch.failed)
    }

    @Test func rebuildsAfterTheThirdTryOnlyIfImpoliteAndTheyCanFollow() {
        var watch = PathWatch()
        let r7 = watch.attempt(polite: false, canRebuild: true)
        #expect(r7 == .restartIce)
        let r8 = watch.attempt(polite: false, canRebuild: true)
        #expect(r8 == .restartIce)
        let r9 = watch.attempt(polite: false, canRebuild: true)
        #expect(r9 == .restartIce)
        let r10 = watch.attempt(polite: false, canRebuild: true)
        #expect(r10 == .rebuild, "the fourth try starts over")
        #expect(watch.restarts == 4)
        _ = watch.stateChanged(.connected, polite: false)
        #expect(watch.restarts == 0, "counting starts again once it's up")

        var older = PathWatch()
        for _ in 0..<6 { #expect(older.attempt(polite: false, canRebuild: false) == .restartIce, "an older app can't follow a rebuild") }
        var polite = PathWatch()
        for _ in 0..<6 { #expect(polite.attempt(polite: true, canRebuild: true) == .restartIce, "only the impolite side rebuilds") }
        #expect(PathWatch.retryEvery == 10)
    }
}

@Suite("Connection hints: weak connection")
struct LinkQualityTests {
    static func reading(concealed: Double = 0, rtt: Double? = 40, packets: Int = 100, connectedFor: TimeInterval = 30) -> LinkQuality.Reading {
        // 2 s of 48 kHz audio at 50 packets/s.
        LinkQuality.Reading(seconds: 2, packets: packets, samples: 96_000, concealed: concealed * 96_000, rttMs: rtt, connectedFor: connectedFor)
    }

    @Test func threeBadReadingsInARowThenThreeGood() {
        var q = LinkQuality()
        let r11 = q.update(Self.reading(concealed: 0.05))
        #expect(!r11)
        let r12 = q.update(Self.reading(concealed: 0.05))
        #expect(!r12)
        #expect(q.weak == nil, "two isn't enough")
        let r13 = q.update(Self.reading(concealed: 0.05))
        #expect(r13)
        #expect(q.weak?.concealed == 0.05)
        let r14 = q.update(Self.reading())
        #expect(!r14)
        let r15 = q.update(Self.reading())
        #expect(!r15)
        #expect(q.weak != nil)
        let r16 = q.update(Self.reading())
        #expect(r16)
        #expect(q.weak == nil, "gone after three good readings")
    }

    @Test func thresholdsAre3PercentAnd300ms() {
        var under = LinkQuality()
        for _ in 0..<5 { under.update(Self.reading(concealed: 0.029, rtt: 299)) }
        #expect(under.weak == nil)
        var slow = LinkQuality()
        for _ in 0..<3 { slow.update(Self.reading(rtt: 301)) }
        #expect(slow.weak?.rttMs == 301)
        var lossy = LinkQuality()
        for _ in 0..<3 { lossy.update(Self.reading(concealed: 0.031)) }
        #expect(lossy.weak != nil)
    }

    @Test func notWhileSettlingNorWhenTheyAreNotSending() {
        var settling = LinkQuality()
        for _ in 0..<5 { settling.update(Self.reading(concealed: 0.2, connectedFor: 9)) }
        #expect(settling.weak == nil, "the first 10 s don't count")
        var quiet = LinkQuality()
        for _ in 0..<5 { quiet.update(Self.reading(concealed: 0.2, packets: 59)) }
        #expect(quiet.weak == nil, "under 30 packets/s: they're not really sending")
        var mixed = LinkQuality()
        mixed.update(Self.reading(concealed: 0.2))
        mixed.update(Self.reading(concealed: 0.2))
        mixed.update(Self.reading(concealed: 0.2, packets: 10)) // doesn't count either way
        #expect(mixed.weak == nil)
        mixed.update(Self.reading(concealed: 0.2))
        #expect(mixed.weak != nil, "three bad readings that count")
    }
}

@Suite("Limiter on volume over 100 %")
struct BoostLimiterTests {
    @Test func noBoostNoLimit() {
        var limiter = BoostLimiter()
        let r17 = limiter.update(wanted: 1, level: 0.9, seconds: 0.025)
        #expect(r17 == 1)
        let r18 = limiter.update(wanted: 0.5, level: 0.9, seconds: 0.025)
        #expect(r18 == 0.5)
        let r19 = limiter.update(wanted: 0, level: 0.9, seconds: 0.025)
        #expect(r19 == 0)
    }

    @Test func aQuietTalkerGetsTheWholeBoost() {
        var limiter = BoostLimiter()
        // −40 dBFS RMS: peaks ~ −25; 200 % (+6 dB) is nowhere near clipping.
        let r20 = limiter.update(wanted: 2, level: VoiceGate.linear(-40), seconds: 0.025)
        #expect(r20 == 2)
    }

    @Test func aLoudBitPullsTheBoostDownAtOnceAndItComesBackSlowly() {
        var limiter = BoostLimiter()
        _ = limiter.update(wanted: 2, level: VoiceGate.linear(-40), seconds: 0.025)
        // A shout at −12 dBFS RMS: peaks ~ +3 dBFS already, so no boost at all (never under 100 %).
        let r21 = limiter.update(wanted: 2, level: VoiceGate.linear(-12), seconds: 0.025)
        #expect(r21 == 1)
        // −20 dBFS RMS: room for +4 dB (≈ 1.58×), but it climbs back at 6 dB/s.
        let next = limiter.update(wanted: 2, level: VoiceGate.linear(-20), seconds: 0.025)
        #expect(next > 1 && next < 1.02)
        var g = next
        for _ in 0..<80 { g = limiter.update(wanted: 2, level: VoiceGate.linear(-20), seconds: 0.025) }
        #expect(abs(g - VoiceGate.linear(4)) < 0.001, "settles at the most that keeps peaks under −1 dBFS")
        // Silence (no packets): back towards the full 200 %.
        for _ in 0..<200 { g = limiter.update(wanted: 2, level: 0, seconds: 0.025) }
        #expect(g == 2)
    }
}

@Suite("WebRTC field trials")
struct FieldTrialTests {
    /// iOS turns on NWPathMonitor and iOS-Holding through the old global
    /// setter, looked up at run time: it must still be in this build.
    @Test func theSetterIsThere() {
        #expect(FieldTrials.setter != nil)
    }
}
