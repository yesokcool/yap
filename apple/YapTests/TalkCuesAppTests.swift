import Foundation
import Testing
@testable import Yap
@testable import YapCore
@testable import YapVoice

/// Round 10 in the app (PROTOCOL.md › Voice › Transmit gate, Connection
/// hints, Your volume, on the row): the spec's words, the row's volume
/// badge and Ctrl/⌘ + scroll's steps, and what the call bar's status says.
@MainActor
@Suite("Talk cues in the app")
struct TalkCuesAppTests {
    @Test func theSpecsWords() {
        #expect(TalkWords.youreMuted == "You're muted")
        #expect(TalkWords.explainNudge == "That sound means you're muted. Turn it off in Settings › Voice.")
        #expect(TalkWords.reminder == "Remind me when I talk while muted")
        #expect(TalkWords.pushToTalkClick == "Push-to-talk click")
        #expect(TalkWords.live == "Live — tap to stop")
        #expect(TalkWords.ownConnection == "Your connection is dropping some of your audio")
        #expect(TalkWords.ownConnectionDetail(lost: 0.062).contains("about 6%"))
        #expect(TalkWords.dip == "Lower shared media while friends talk")
        #expect(TalkWords.tooLoud == "Too loud (clipping)")
        #expect(SettingsText.pushToTalkNote.contains("Double-tap it to stay live"))
    }

    @Test func qualityWordsAreHonest() {
        #expect(!SettingsText.qualityNote.contains("lower of the two"), "each sender uses its own setting")
        #expect(SettingsText.qualityNote.contains("your setting"))
        #expect(VoiceSettings.qualityWarning(256_000)?.hasPrefix("No dropout protection at this quality") == true)
        #expect(VoiceSettings.qualityWarning(128_000) == nil)
        #expect(CallQuality.menuWarning == "No dropout protection; several times the upload")
    }

    @Test func theRowSaysTheVolumeOnlyWhenChanged() {
        #expect(PersonVolume.label(1) == nil, "100 %: nothing")
        #expect(PersonVolume.label(0.6) == "60%")
        #expect(PersonVolume.label(1.5) == "150%")
        #expect(PersonVolume.label(0) == nil && PersonVolume.isMuted(0), "0 %: the muted-speaker glyph")
        #expect(!PersonVolume.isMuted(0.05))
        #expect(PersonVolume.spoken(0) == "muted for you")
        #expect(PersonVolume.spoken(0.6) == "60% for you")
        #expect(PersonVolume.spoken(1) == nil)
    }

    @Test func scrollStepsAreFivePercentWithinRange() {
        #expect(PersonVolume.stepped(1, by: 1) == 1.05)
        #expect(PersonVolume.stepped(1, by: -3) == 0.85)
        #expect(PersonVolume.stepped(0.05, by: -4) == 0, "never below 0 %")
        #expect(PersonVolume.stepped(1.95, by: 5) == 2, "never above 200 %")
        #expect(PersonVolume.stepped(0.62, by: 1) == 0.65, "snaps to the slider's 5 % steps")
    }

    @Test func callBarStatusSaysMutedOrLive() {
        let client = YapClient(credentials: MemoryCredentialStore(), client: "yap-test")
        let defaults = UserDefaults(suiteName: "yap-talk-cues-\(UUID().uuidString)")!
        let voice = VoiceController(client: client, defaults: defaults, requestsMicPermission: false)
        #expect(CallCue.line(voice) == nil)
        voice.settings.mode = .pushToTalk
        for down in [true, false, true, false] { voice.setPushToTalk(down) }
        #expect(voice.engine.pushToTalkLatched)
        #expect(CallCue.line(voice) == "Live — tap to stop")
        voice.showMutedNudgeForDemo()
        #expect(CallCue.line(voice) == "You're muted", "the nudge wins while it shows")
        voice.settings.mode = .voiceActivity
        #expect(!voice.engine.pushToTalkLatched, "a new mode lets go of the latch")
    }
}
