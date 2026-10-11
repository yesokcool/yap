import Foundation
import Testing
@testable import Yap
@testable import YapVoice

/// Screen sharing's words are the web's (the brief's list), and the viewer's
/// lines and the sharer's "Watching: …" read the same everywhere.
@Suite("Screen sharing words")
struct ScreenTextTests {
    @Test func theWebsWords() {
        #expect(ScreenText.share == "Share screen")
        #expect(ScreenText.stopSharing == "Stop sharing")
        #expect(ScreenText.watch == "Watch screen")
        #expect(ScreenText.stopWatching == "Stop watching")
        #expect(ScreenText.popOut == "Pop out")
        #expect(ScreenText.fullScreen == "Full screen")
        #expect(ScreenQuality.allCases.map(\.title) == ["Smooth", "Sharp", "Max"])
        #expect(ScreenText.title("Ana") == "Ana's screen")
        #expect(ScreenText.live(name: "Ana", mine: false) == "Watch Ana's screen")
        #expect(ScreenText.live(name: "Ana", mine: true) == "You’re sharing your screen (click to stop)")
        #expect(ScreenText.sharing(.sharp) == "Sharing · Sharp")
        #if os(iOS)
        #expect(ScreenText.shareHint.contains("Control Center"), "how a broadcast stops")
        #else
        #expect(ScreenText.shareHint.contains("LIVE badge"))
        #endif
    }

    @Test func theViewersLines() {
        var watch = ScreenWatch(sid: "s1", uid: "ana")
        #expect(watch.status(name: "Ana") == "Connecting to Ana's screen…")
        watch.frame(at: 1)
        #expect(watch.status(name: "Ana") == nil)
        watch.tick(at: 4)
        #expect(watch.status(name: "Ana") == "Ana's screen is paused.")
        watch.sharerStopped(at: 5)
        #expect(watch.status(name: "Ana") == "Ana stopped sharing.")
    }

    @Test func whosWatching() {
        #expect(ScreenAudience.text(names: []) == "No one's watching yet")
        #expect(ScreenAudience.text(names: ["Ben", "Ana"]) == "Watching: Ana, Ben")
    }

    /// Since round 4 iPhone and iPad share too (the broadcast extension).
    @Test func everyAppShares() {
        #expect(VoiceController.canShareScreen)
        #if os(iOS)
        #expect(ScreenText.pickerMissing.hasPrefix("Couldn't open screen sharing"))
        #endif
    }
}
