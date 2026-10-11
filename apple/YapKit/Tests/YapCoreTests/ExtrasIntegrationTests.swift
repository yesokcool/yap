import Foundation
import Testing
@testable import YapCore

/// The v0.36 extras against a local dev server (start it with
/// `--var TEST_HANGS:1` so the hang alarm can fire within seconds). Part of
/// the integration suite, so they run one at a time with it: new members
/// being let in put notices on everyone's screen, which would race the
/// other tests' waits.
extension ServerIntegrationTests {
    /// Two people in general, histories loaded.
    private func extrasPair(_ a: String, _ b: String) async throws -> (YapClient, YapClient) {
        let ada = try await member("\(a) \(TestServer.run)").0
        let ben = try await member("\(b) \(TestServer.run)").0
        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("loaded") { ada.histories["general"]?.loading == false && ben.histories["general"]?.loading == false }
        return (ada, ben)
    }

    private func extrasNewest(_ client: YapClient, _ body: String) -> Message? {
        client.histories["general"]?.messages.last { $0.body == body }
    }

    @Test func pollsVoteChangeAndTakeBack() async throws {
        let (ada, ben) = try await extrasPair("Pol", "Lee")
        let question = "Pizza or tacos? \(TestServer.run)"
        #expect(ada.send(text: "/poll \(question) | pizza | tacos | both", in: "general"), "/poll turns into a poll")
        try await waitUntil("ben gets the poll") { extrasNewest(ben, question)?.poll?.options == ["pizza", "tacos", "both"] }
        let poll = try #require(extrasNewest(ben, question))
        ben.vote(poll, option: 1)
        try await waitUntil("ada sees ben's vote") { extrasNewest(ada, question)?.poll?.votes == [[], [ben.uid], []] }
        ben.vote(try #require(extrasNewest(ben, question)), option: 2)
        try await waitUntil("moved") { extrasNewest(ada, question)?.poll?.votes == [[], [], [ben.uid]] }
        ada.vote(try #require(extrasNewest(ada, question)), option: 2)
        try await waitUntil("two votes") { extrasNewest(ben, question)?.poll?.count(2) == 2 }
        ben.vote(try #require(extrasNewest(ben, question)), option: nil)
        try await waitUntil("taken back") { extrasNewest(ada, question)?.poll?.votes == [[], [], [ada.uid]] }

        // Refused on the device: one answer, or in a Personal space.
        #expect(!ada.send(text: "/poll Lonely? | yes", in: "general"))
        #expect(!ada.sendPoll(question: "Just me?", options: ["a", "b"], in: try #require(ada.personalChannel?.id)))

        // History carries the votes.
        let (late, _) = device()
        late.logIn(name: "Lee \(TestServer.run)", pin: TestServer.password)
        try await waitUntil("late device ready") { late.phase == .online }
        late.open(channel: "general")
        try await waitUntil("history has the tally") { late.histories["general"]?.messages.first { $0.id == poll.id }?.poll?.votes == [[], [], [ada.uid]] }
    }

    @Test func rollsAndFlipsAreRolledByTheServer() async throws {
        let (ada, ben) = try await extrasPair("Dic", "Ept")
        ada.send(text: "/roll 2d6+3", in: "general")
        try await waitUntil("ben gets a roll") { ben.histories["general"]?.messages.last?.roll != nil }
        let roll = try #require(ben.histories["general"]?.messages.last?.roll)
        #expect(roll.kind == "roll" && roll.rolls.count == 2 && roll.total == roll.rolls.reduce(0, +) + 3)
        #expect(ben.histories["general"]?.messages.last?.body == "🎲 2d6+3: \(roll.rolls[0]) + \(roll.rolls[1]) + 3 = \(roll.total)")
        try await Task.sleep(for: .milliseconds(1100))
        ada.send(text: "/flip", in: "general")
        try await waitUntil("a flip") { ben.histories["general"]?.messages.last?.roll?.isFlip == true }
        let flip = try #require(ben.histories["general"]?.messages.last)
        #expect(["🪙 Heads", "🪙 Tails"].contains(flip.body) && flip.roll?.headline == (flip.body.hasSuffix("Heads") ? "Heads" : "Tails"))
        try await Task.sleep(for: .milliseconds(1100))
        ada.send(text: "/roll banana", in: "general")
        try await waitUntil("bad spec refused") { ada.notice != nil }
    }

    @Test func awayMessagesAndTheAutoReply() async throws {
        let (ada, ben) = try await extrasPair("Awa", "Bey")
        ada.setAway("at work til 6", until: nil)
        try await waitUntil("ben sees ada away") { ben.user(ada.uid)?.awayNow()?.text == "at work til 6" }
        let ask = "@\(ada.me!.name) you around? \(TestServer.run)"
        ben.send(text: ask, in: "general")
        try await waitUntil("the auto-reply lands under ben's message") {
            guard let mine = extrasNewest(ben, ask) else { return false }
            return ben.awayNotes[mine.id]?.first?.text == "at work til 6"
        }
        #expect(ada.awayNotes.isEmpty, "only the one who mentioned hears it")
        ada.clearAway()
        try await waitUntil("cleared for ben") { ben.user(ada.uid)?.away == nil }

        // Chimes: a built-in, then back to the default.
        ada.setChime("tada")
        try await waitUntil("ben knows ada's chime") { ben.user(ada.uid)?.chime == "tada" }
        ada.setChime(nil)
        try await waitUntil("default again") { ben.user(ada.uid)?.chime == nil }
    }

    @Test func soundboardUploadPlayAndDelete() async throws {
        let (ada, ben) = try await extrasPair("Snd", "Tim")
        // 0.8 s of a loud 440 Hz tone, the way the app sends it.
        let tone = (0..<19_200).map { Float(sin(Double($0) * 2 * .pi * 440 / 24_000) * 0.95) }
        let sound = try await ada.uploadSound(WAV.encode(tone, sampleRate: 24_000), kind: "board", name: "Airhorn", emoji: "📯")
        #expect(sound.name == "Airhorn" && sound.emoji == "📯" && !sound.isChime)
        #expect(sound.gain < 0.5, "a loud clip is turned down: \(sound.gain)")
        #expect(abs(sound.ms - 800) < 5)
        try await waitUntil("ben's board has it") { ben.boardSounds.contains { $0.id == sound.id } }
        let (bytes, response) = try await URLSession.shared.data(from: TestServer.endpoint.mediaURL(sound.media))
        #expect((response as? HTTPURLResponse)?.value(forHTTPHeaderField: "Content-Type") == "audio/wav")
        #expect(WAV.decode(bytes)?.sampleRate == 24_000)

        // Plays reach people in the same voice channel.
        var heard: [String] = []
        ben.onSound = { sound, _, uid in heard.append("\(sound.name)/\(uid)") }
        ada.send(.voiceJoin(channelId: "lounge", muted: true, deafened: false))
        ben.send(.voiceJoin(channelId: "lounge", muted: true, deafened: false))
        try await waitUntil("both in lounge") { ada.voiceSessions(in: "lounge").count >= 2 }
        ada.playSound(sound)
        try await waitUntil("ben hears it") { heard == ["Airhorn/\(ada.uid)"] }

        // A chime: ≤ 2 s, becomes ada's chime.
        let short = Array(tone.prefix(24_000))
        let chime = try await ada.uploadSound(WAV.encode(short, sampleRate: 24_000), kind: "chime", name: "")
        #expect(chime.isChime)
        try await waitUntil("ben knows the clip chime") { ben.user(ada.uid)?.chime == "s:\(chime.id)" }
        #expect(ChimeChoice.from(ben.user(ada.uid)?.chime) == .clip(soundId: chime.id))
        #expect(ben.chimeSound(.clip(soundId: chime.id)) != nil, "sound.added reached ben too")
        ada.setChime(nil)

        ada.deleteSound(sound)
        try await waitUntil("gone from the board") { !ben.boardSounds.contains { $0.id == sound.id } }
        ada.send(.voiceLeave)
        ben.send(.voiceLeave)
    }

    @Test func doodleReactionsAndTheMediaShelf() async throws {
        let (ada, ben) = try await extrasPair("Doo", "Shf")
        let text = "draw me a reaction \(TestServer.run)"
        ada.send(text: text, in: "general")
        try await waitUntil("ben has it") { extrasNewest(ben, text) != nil }
        let message = try #require(extrasNewest(ben, text))
        try await ben.react(to: message, doodle: TestServer.uniquePNG(), mime: "image/png")
        try await waitUntil("ada sees a doodle") { extrasNewest(ada, text)?.doodles.first?.uids == [ben.uid] }
        let doodle = try #require(extrasNewest(ada, text)?.doodles.first)
        let id = try #require(Doodle.id(fromReaction: doodle.emoji))
        let (_, response) = try await URLSession.shared.data(from: TestServer.endpoint.mediaURL(id))
        #expect((response as? HTTPURLResponse)?.statusCode == 200)

        // The app's own wiggling GIF goes through, and replaces Ben's first doodle.
        let drawing = DoodleDrawing(strokes: [.init(color: 2, size: 4, points: [.init(x: 6, y: 6), .init(x: 40, y: 30)])], wiggle: true)
        try await ben.react(to: message, doodle: drawing.gif(), mime: "image/gif")
        try await waitUntil("the GIF doodle replaces the PNG") {
            guard let doodles = extrasNewest(ada, text)?.doodles else { return false }
            return doodles.count == 1 && doodles.first?.emoji != doodle.emoji
        }

        // The shelf: pictures and GIFs only, newest first.
        let up = try await ServerAPI(endpoint: TestServer.endpoint).uploadImage(TestServer.uniquePNG(), mime: "image/png",
                                                                               sid: ada.mySid!, uploadToken: ada.uploadToken ?? "")
        let caption = "shelf pic \(TestServer.run)"
        ada.send(.send(channelId: "general", body: caption, nonce: "shelf-\(TestServer.run)", gif: nil, image: ImageRef(id: up, width: 8, height: 8)))
        try await waitUntil("picture arrives") { extrasNewest(ben, caption) != nil }
        ben.loadShelf("general")
        try await waitUntil("shelf loaded") { ben.shelves["general"]?.loading == false }
        let shelf = try #require(ben.shelves["general"])
        #expect(shelf.items.first?.body == caption)
        #expect(shelf.items.allSatisfy { $0.image != nil || $0.gif != nil })
    }

    @Test func catchUpCountsAndSaveToPersonal() async throws {
        let (ada, ben) = try await extrasPair("Cat", "Sav")
        ada.close(channel: "general")
        try await Task.sleep(for: .milliseconds(300))
        let first = "while you were out \(TestServer.run)"
        ben.send(text: first, in: "general")
        try await waitUntil("ada got it") { ada.histories["general"]?.messages.contains { $0.body == first } == true }
        try await Task.sleep(for: .milliseconds(1100))
        ben.send(text: "and another \(TestServer.run)", in: "general")
        try await waitUntil("unread for ada") { ada.isUnread(ada.channelsById["general"]!) }
        ada.requestUnreadCounts()
        try await waitUntil("counts back") { ada.unreadCounts != nil }
        let count = try #require(ada.unreadCounts?["general"])
        #expect(count.n >= 2)
        #expect(count.first?.body == first, "the oldest unread one")
        #expect(count.first?.uid == ben.uid)

        // Save to Personal: a note replying to it, with where it lives.
        let original = try #require(ada.histories["general"]?.messages.last { $0.body == first })
        #expect(ada.saveToPersonal(original))
        let personal = try #require(ada.personalChannel?.id)
        ada.open(channel: personal)
        try await waitUntil("the bookmark lands") {
            ada.histories[personal]?.messages.contains { $0.reply?.id == original.id } == true
        }
        let saved = try #require(ada.histories[personal]?.messages.last { $0.reply?.id == original.id })
        #expect(saved.body.isEmpty, "a bookmark without a comment")
        #expect(saved.reply?.channelId == "general")
        #expect(saved.reply?.body == first)
    }

    @Test func quietHoursRoundTrip() async throws {
        let ada = try await member("Qui \(TestServer.run)").0
        let night = QuietHours(from: 23 * 60, to: 8 * 60, tz: "Europe/Rome", mentions: true)
        ada.setQuietHours(night)
        // A second device gets it in ready.
        let (other, _) = device()
        other.logIn(name: "Qui \(TestServer.run)", pin: TestServer.password)
        try await waitUntil("other device ready with quiet hours") { other.phase == .online && other.notifySettings?.quiet == night }
        let snooze = Date().addingTimeInterval(3600).yapMillis
        ada.setQuietHours(QuietHours(from: 23 * 60, to: 8 * 60, tz: "Europe/Rome", mentions: false, until: snooze))
        try await waitUntil("snooze reaches the other device") { other.notifySettings?.quiet?.until == snooze }
        #expect(other.quietHolds(mention: true), "snoozed, mentions held too")
        ada.setQuietHours(nil)
        try await waitUntil("off everywhere") { other.notifySettings?.quiet == nil && other.notifySettings != nil }
        #expect(!other.quietHolds(mention: false))
    }

    @Test func hangsPlanCancelAndStart() async throws {
        let (ada, ben) = try await extrasPair("Han", "Gam")
        let title = "Game night \(TestServer.run)"
        ada.planHang(at: Date().addingTimeInterval(3 * 3600), title: title, channelId: "lounge")
        try await waitUntil("ben sees the plan") { ben.hangs.contains { $0.title == title } }
        let hang = try #require(ben.hangs.first { $0.title == title })
        #expect(hang.channelId == "lounge" && hang.uid == ada.uid)
        #expect(hang.at % 1_800_000 == 0, "on a half-hour boundary")
        ada.cancelHang(hang)
        try await waitUntil("cancelled for ben") { !ben.hangs.contains { $0.id == hang.id } }

        // With TEST_HANGS the alarm may be seconds away: hang.now arrives.
        let soon = "Right now \(TestServer.run)"
        ada.send(.hangSet(at: Date().addingTimeInterval(2).yapMillis, title: soon, channelId: nil))
        let deadline = ContinuousClock.now + .seconds(3)
        while ContinuousClock.now < deadline, !ben.hangs.contains(where: { $0.title == soon }) {
            try await Task.sleep(for: .milliseconds(50))
        }
        guard ben.hangs.contains(where: { $0.title == soon }) else {
            print("    (start the dev server with --var TEST_HANGS:1 to test hang.now)")
            return
        }
        var started: Hang?
        ben.onHangNow = { started = $0 }
        try await waitUntil("hang.now", timeout: .seconds(12)) { started?.title == soon }
        #expect(ben.hangStarting?.title == soon)
    }

    @Test func watchTogetherQueue() async throws {
        let ada = try await member("Wat \(TestServer.run)").0
        ada.queueWatch(videoId: "dQw4w9WgXcQ")
        ada.send(.watchGet)
        try await waitUntil("now playing (or queued)") {
            ada.watch?.videoId == "dQw4w9WgXcQ" || ada.watch?.queue.contains { $0.id == "dQw4w9WgXcQ" } == true
        }
    }
}
