import Foundation
import Testing
@testable import Yap
import YapCore

/// Round 12: per-channel drafts, the "↓ New messages" pill, the mention
/// badge on the icon, and "You were in Lounge." after a relaunch.
@MainActor
@Suite("Round 12")
struct Round12Tests {
    // MARK: Drafts

    @Test func eachChannelKeepsItsOwnDraft() {
        var drafts = ChannelDrafts()
        drafts.keep("half a thought", for: "general")
        drafts.keep("other words", for: "games")
        #expect(drafts.text(for: "general") == "half a thought")
        #expect(drafts.text(for: "games") == "other words")
        #expect(drafts.text(for: "music") == "", "a channel you never typed in starts empty")
        // Sent (the box is empty when you leave): nothing waits.
        drafts.keep("", for: "general")
        drafts.keep("  \n ", for: "games")
        #expect(drafts.isEmpty)
    }

    @Test func signingOutForgetsEveryDraft() {
        var drafts = ChannelDrafts()
        drafts.keep("secret plans", for: "general")
        drafts.removeAll()
        #expect(drafts.text(for: "general") == "")
    }

    @Test func aChannelScreenGetsItsDraftBackAndAnEditGivesBackTheOneBehindIt() {
        let model = AppModel()
        let first = ChannelController(model: model, channelId: "general")
        first.draft = "see you at 8"
        first.disappear()
        #expect(ChannelController(model: model, channelId: "general").draft == "see you at 8")
        #expect(ChannelController(model: model, channelId: "games").draft == "", "never leaks into another channel")
        // Editing a message when you leave: the draft behind the edit is what waits.
        let editing = ChannelController(model: model, channelId: "general")
        editing.editing = MessageEdit(message: Self.message("m1", ts: 1, uid: model.client.uid), savedDraft: "see you at 9")
        editing.draft = "the message's own words"
        editing.disappear()
        #expect(ChannelController(model: model, channelId: "general").draft == "see you at 9")
    }

    // MARK: "↓ New messages"

    static func message(_ id: String, ts: Int, uid: String = "ada") -> Message {
        Message(id: id, channelId: "general", uid: uid, body: "hi", ts: ts)
    }

    @Test func showsOnlyForSomeoneElsesMessageWhileScrolledUp() {
        var pill = NewMessagesPill()
        pill.newest(Self.message("m1", ts: 100), myUid: "me")
        #expect(!pill.shows, "the first page isn't news")
        pill.newest(Self.message("m2", ts: 200), myUid: "me")
        #expect(!pill.shows, "at the bottom you see it come in")
        pill.scrolled(atBottom: false)
        pill.newest(Self.message("m3", ts: 300, uid: "me"), myUid: "me")
        #expect(!pill.shows, "my own message isn't news to me")
        pill.newest(Self.message("m4", ts: 400), myUid: "me")
        #expect(pill.shows)
        pill.scrolled(atBottom: true)
        #expect(!pill.shows, "gone once you're at the bottom")
    }

    @Test func olderPagesAndTapsDontShowIt() {
        var pill = NewMessagesPill()
        pill.newest(Self.message("m5", ts: 500), myUid: "me")
        pill.scrolled(atBottom: false)
        // Older messages loaded above, or the same newest one again: not new.
        pill.newest(Self.message("m5", ts: 500), myUid: "me")
        pill.newest(Self.message("m1", ts: 100), myUid: "me")
        #expect(!pill.shows)
        pill.newest(Self.message("m6", ts: 600), myUid: "me")
        #expect(pill.shows)
        pill.tapped()
        #expect(!pill.shows)
    }

    // MARK: The icon's badge

    @Test func theBadgeAddsUpTheSidebarsMentions() {
        let channels = [Channel(id: "general", name: "general", kind: .text, position: 0),
                        Channel(id: "games", name: "games", kind: .text, position: 1)]
        #expect(MentionBadge.total([:], channels: channels) == 0)
        #expect(MentionBadge.total(["general": 2, "games": 1], channels: channels) == 3)
        #expect(MentionBadge.total(["general": 2, "archived": 5], channels: channels) == 2, "a channel no longer in the list counts nowhere")
        #expect(MentionBadge.label(0) == nil)
        #expect(MentionBadge.label(7) == "7")
        #expect(MentionBadge.label(140) == "99+")
    }

    // MARK: "You were in Lounge."

    @Test func aRecentCallIsOfferedOnce() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let lounge = LastCall(channelId: "lounge", at: now.addingTimeInterval(-120))
        #expect(LastCall.offer(lounge, now: now) == lounge)
        #expect(LastCall.offer(LastCall(channelId: "lounge", at: now.addingTimeInterval(-601)), now: now) == nil, "over 10 minutes ago")
        #expect(LastCall.offer(nil, now: now) == nil)
        #expect(LastCall.offer(LastCall(channelId: "lounge", at: now.addingTimeInterval(3600)), now: now) == nil, "a clock that went back")
    }

    @Test func theBarShowsOnlyOutOfACallForAVoiceChannel() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let call = LastCall(channelId: "lounge", at: now.addingTimeInterval(-60))
        let lounge = Channel(id: "lounge", name: "Lounge", kind: .voice, position: 0)
        #expect(call.shows(inVoice: false, channel: lounge, now: now))
        #expect(!call.shows(inVoice: true, channel: lounge, now: now), "joining any voice channel hides it")
        #expect(!call.shows(inVoice: false, channel: nil, now: now), "the channel is gone")
        #expect(!call.shows(inVoice: false, channel: lounge, now: now.addingTimeInterval(600)), "10 minutes after the call")
        #expect(RejoinWords.line("Lounge") == "You were in Lounge.")
    }

    static func store() -> LastCallStore {
        let suite = "yap.tests.lastCall.\(UUID().uuidString)"
        return LastCallStore(defaults: UserDefaults(suiteName: suite)!)
    }

    @Test func theRecordFollowsTheCall() {
        let store = Self.store()
        let offer = RejoinOffer(store: store)
        var current: String? = "lounge"
        offer.currentCall = { current }
        offer.callChanged()
        #expect(store.load()?.channelId == "lounge", "in a call: kept on the device")
        current = nil
        offer.callChanged()
        #expect(store.load() == nil, "left on purpose: nothing to offer next time")
    }

    @Test func quittingMidCallKeepsTheRecordForTheNextLaunch() {
        let store = Self.store()
        let offer = RejoinOffer(store: store)
        var current: String? = "lounge"
        offer.currentCall = { current }
        offer.callChanged()
        offer.terminating = true
        current = nil
        offer.callChanged()
        #expect(store.load()?.channelId == "lounge")
        // The next launch offers it, and takes it: the launch after doesn't.
        let next = RejoinOffer(store: store)
        #expect(next.call?.channelId == "lounge")
        #expect(store.load() == nil)
        #expect(RejoinOffer(store: store).call == nil)
    }

    @Test func dismissingOrSigningOutEndsTheOffer() {
        let store = Self.store()
        store.save(LastCall(channelId: "lounge", at: Date()))
        let offer = RejoinOffer(store: store)
        #expect(offer.call != nil)
        offer.dismiss()
        #expect(offer.call == nil)
        store.save(LastCall(channelId: "lounge", at: Date()))
        let again = RejoinOffer(store: store)
        again.forget()
        #expect(again.call == nil && store.load() == nil)
    }

    @Test func joiningACallTakesTheBarAway() {
        let store = Self.store()
        store.save(LastCall(channelId: "lounge", at: Date()))
        let offer = RejoinOffer(store: store)
        offer.currentCall = { "games" }
        offer.callChanged()
        #expect(offer.call == nil)
        #expect(store.load()?.channelId == "games")
    }
}
