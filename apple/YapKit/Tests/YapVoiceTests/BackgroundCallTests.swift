import Foundation
import Testing
@testable import YapVoice

/// iPhone calls with the screen off: what Yap does when something outside
/// its hands ends the call (BackgroundCall's rules, on a hand-cranked clock).
@Suite("A call while Yap is off screen")
struct BackgroundCallTests {
    typealias Rules = BackgroundCall

    /// In a call, then the screen goes off at t = 100.
    private func offScreen() -> Rules {
        var rules = Rules()
        _ = rules.call(true, at: 0)
        #expect(rules.scene(active: false, at: 100) == [.armNote])
        return rules
    }

    /// Ticks every 5 s from `from` to `to`, everything they said.
    private func ticks(_ rules: inout Rules, from: TimeInterval, to: TimeInterval) -> [Rules.Effect] {
        var effects: [Rules.Effect] = []
        var t = from
        while t <= to {
            effects += rules.tick(at: t)
            t += Rules.tickEvery
        }
        return effects
    }

    @Test func onScreenNothingIsScheduled() {
        var rules = Rules()
        #expect(rules.call(true, at: 0).isEmpty)
        #expect(rules.tick(at: 5).isEmpty)
        #expect(rules.interruptionBegan(at: 6).isEmpty, "on screen Yap keeps running: no extra time needed")
        #expect(rules.soundFailed(at: 7).isEmpty, "on screen the silence warning covers it")
        #expect(!rules.watching)
    }

    @Test func aQuietNightKeepsTheCall() {
        var rules = offScreen()
        // Muted, deafened, alone: none of it reaches these rules; only time passes.
        let effects = ticks(&rules, from: 105, to: 105 + 8 * 3600)
        #expect(!effects.contains { if case .leave = $0 { true } else { false } })
        #expect(effects.allSatisfy { $0 == .armNote }, "the note keeps being pushed back")
        #expect(rules.scene(active: true, at: 105 + 8 * 3600 + 1) == [.disarmNote])
    }

    @Test func aShortInterruptionIsSurvived() {
        var rules = offScreen()
        #expect(rules.interruptionBegan(at: 110) == [.keepAwake], "the sound stopped: ask iOS for time")
        #expect(rules.tick(at: 115) == [.armNote])
        #expect(rules.interruptionEnded(at: 118).isEmpty)
        // The next tick lets go of the extra time (the call's sound runs again).
        #expect(rules.tick(at: 120) == [.armNote, .letSleep])
        #expect(ticks(&rules, from: 125, to: 400).allSatisfy { $0 == .armNote })
    }

    @Test func aPhoneCallTakesTheSoundAndWeLeaveSaying() {
        var rules = offScreen()
        _ = rules.interruptionBegan(at: 110)
        let effects = ticks(&rules, from: 115, to: 130)
        #expect(effects.last == .leave(.soundTaken, notify: true), "\(effects)")
        #expect(effects.filter { if case .leave = $0 { true } else { false } }.count == 1)
        // Leaving takes us out of the call: nothing scheduled, no extra time.
        #expect(rules.call(false, at: 130) == [.disarmNote, .letSleep])
        #expect(rules.tick(at: 135).isEmpty)
    }

    @Test func iOSTakingBackTheTimeLeavesToo() {
        var rules = offScreen()
        _ = rules.interruptionBegan(at: 110)
        #expect(rules.timeRunningOut(at: 125) == [.leave(.soundTaken, notify: true)])
    }

    @Test func anInterruptionBeforeTheScreenWentOffStillCounts() {
        var rules = Rules()
        _ = rules.call(true, at: 0)
        #expect(rules.interruptionBegan(at: 10).isEmpty)
        #expect(rules.scene(active: false, at: 12) == [.armNote, .keepAwake])
        #expect(rules.tick(at: 30) == [.leave(.soundTaken, notify: true)])
    }

    @Test func soundThatComesBackKeepsTheCall() {
        var rules = offScreen()
        // Dictation in another app: an interruption, then the sound won't start at first.
        #expect(rules.interruptionBegan(at: 110) == [.keepAwake])
        #expect(rules.interruptionEnded(at: 112).isEmpty)
        #expect(rules.soundFailed(at: 113).isEmpty, "not at once: VoiceController tries again (and we're already awake)")
        #expect(rules.tick(at: 115) == [.armNote], "still awake while the sound is stopped")
        #expect(rules.soundFailed(at: 116).isEmpty, "a try that failed again changes nothing")
        #expect(rules.soundRecovered(at: 118).isEmpty)
        #expect(rules.tick(at: 120) == [.armNote, .letSleep])
        #expect(ticks(&rules, from: 125, to: 400).allSatisfy { $0 == .armNote })
    }

    @Test func soundThatWontComeBackEndsTheCallAfterTheGrace() {
        var rules = offScreen()
        #expect(rules.soundFailed(at: 113) == [.keepAwake], "the sound is what keeps Yap running: ask for time")
        let effects = ticks(&rules, from: 115, to: 135)
        #expect(effects.last == .leave(.soundFailed, notify: true), "\(effects)")
        #expect(effects.filter { if case .leave = $0 { true } else { false } }.count == 1)
        #expect(rules.call(false, at: 135) == [.disarmNote, .letSleep])
    }

    @Test func soundLostOnScreenIsGivenTimeOffScreen() {
        var rules = Rules()
        _ = rules.call(true, at: 0)
        #expect(rules.soundFailed(at: 10).isEmpty)
        #expect(rules.scene(active: false, at: 12) == [.armNote, .keepAwake])
        #expect(rules.timeRunningOut(at: 40) == [.leave(.soundFailed, notify: true)])
    }

    @Test func soundBackOnScreenIsForgotten() {
        var rules = Rules()
        _ = rules.call(true, at: 0)
        _ = rules.soundFailed(at: 10)
        _ = rules.soundRecovered(at: 11)
        #expect(rules.scene(active: false, at: 100) == [.armNote])
        #expect(ticks(&rules, from: 105, to: 200).allSatisfy { $0 == .armNote })
    }

    @Test func stoppedByIOSTheNoteSaysItAndWakingUpMakesItTrue() {
        var rules = offScreen()
        _ = ticks(&rules, from: 105, to: 120)
        // Frozen at 120 (iOS stopped Yap). The note armed then fires at 160.
        // Woken in the background later: we left (the note already said so).
        #expect(rules.tick(at: 900) == [.leave(.stopped, notify: false)])
    }

    @Test func stoppedThenOpenedLeavesWithoutASecondNotification() {
        var rules = offScreen()
        _ = rules.tick(at: 105)
        let effects = rules.scene(active: true, at: 3600)
        #expect(effects == [.leave(.stopped, notify: false), .disarmNote])
    }

    @Test func aShortGapBetweenTicksIsNotAStop() {
        var rules = offScreen()
        _ = rules.tick(at: 105)
        #expect(rules.tick(at: 105 + Rules.frozenAfter - 1) == [.armNote])
        #expect(rules.scene(active: true, at: 105 + Rules.frozenAfter + 2) == [.disarmNote])
    }

    @Test func aMinuteWithoutTheServerEndsIt() {
        var rules = offScreen()
        #expect(rules.connection(online: false, at: 110).isEmpty)
        // A blip that comes back is nothing.
        _ = rules.connection(online: true, at: 125)
        #expect(ticks(&rules, from: 105, to: 200).allSatisfy { $0 == .armNote })
        _ = rules.connection(online: false, at: 200)
        _ = rules.connection(online: false, at: 230) // still down: the first time counts
        let effects = ticks(&rules, from: 205, to: 265)
        #expect(effects.last == .leave(.offline, notify: true), "\(effects)")
    }

    @Test func theChannelGoingAwaySaysSoOffScreenOnly() {
        var onScreen = Rules()
        _ = onScreen.call(true, at: 0)
        #expect(onScreen.ended(.channelGone, at: 5).isEmpty, "on screen the app shows it")
        var rules = offScreen()
        #expect(rules.ended(.channelGone, at: 110) == [.leave(.channelGone, notify: true)])
    }

    @Test func joiningOffScreenStartsWatching() {
        var rules = Rules()
        _ = rules.scene(active: false, at: 0)
        #expect(rules.call(true, at: 3) == [.armNote], "a notification tap rejoins before the app is on screen")
        #expect(rules.watching)
    }

    @Test func wordsForEveryReason() {
        for reason in Rules.Reason.allCases {
            #expect(reason.text.hasSuffix("."))
            #expect(!reason.text.isEmpty)
        }
        #expect(Rules.Reason.soundTaken.text == "A phone call or another app took over the sound.")
        #expect(Rules.Reason.soundFailed.text == "Yap lost the mic and speaker and couldn't get them back.")
        #expect(!Rules.Reason.channelGone.canRejoin)
        #expect(Rules.Reason.stopped.canRejoin)
    }
}
