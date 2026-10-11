import Foundation
import Testing
@testable import Yap
@testable import YapCore

/// The popover on someone under a voice channel: who gets it, and which of
/// the web's owner/mod actions it offers.
@Suite("Someone in voice")
struct VoicePersonTests {
    @Test func everyoneButMeAndOBS() {
        let ada = Session(sid: "s1", uid: "ada", client: "web", voice: VoiceState(channelId: "lounge", muted: false, deafened: false))
        #expect(VoicePersonMenu.offered(ada, mySid: "s0", myUid: "me"))
        #expect(!VoicePersonMenu.offered(ada, mySid: "s1", myUid: "me"), "my own row: no volume")
        let myPhone = Session(sid: "s2", uid: "me", client: "ios", voice: VoiceState(channelId: "lounge", muted: false, deafened: false))
        #expect(!VoicePersonMenu.offered(myPhone, mySid: "s0", myUid: "me"), "my other device either")
        let obs = Session(sid: "s3", uid: "ada", client: "obs", voice: VoiceState(channelId: "lounge", muted: false, deafened: false))
        #expect(!VoicePersonMenu.offered(obs, mySid: "s0", myUid: "me"), "an OBS listener has no voice to turn up")
    }

    @Test func theWebsModActions() {
        #expect(VoicePersonPopover98.modActions(me: .member, them: .member).isEmpty)
        #expect(VoicePersonPopover98.modActions(me: .mod, them: .member) == ["Reset PIN", "Ban"])
        #expect(VoicePersonPopover98.modActions(me: .mod, them: .owner).isEmpty, "nobody bans the owner")
        #expect(VoicePersonPopover98.modActions(me: .owner, them: .member) == ["Make mod", "Reset PIN", "Ban"])
        #expect(VoicePersonPopover98.modActions(me: .owner, them: .mod) == ["Remove mod", "Reset PIN", "Ban"])
    }
}
