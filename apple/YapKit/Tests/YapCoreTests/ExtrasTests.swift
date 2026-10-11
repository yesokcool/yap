import AVFoundation
import Foundation
import ImageIO
import Testing
@testable import YapCore

/// The v0.36 extras (PROTOCOL.md › Extras): frames both ways and the shared
/// rules (spoilers, YouTube links, /poll, quiet hours, chimes, WAV, gain).
@Suite("Extras (v0.36)")
struct ExtrasTests {
    private func json(_ message: ClientMessage) throws -> [String: Any] {
        try JSONSerialization.jsonObject(with: Data(try message.frame().utf8)) as! [String: Any]
    }

    // MARK: Client → server

    @Test func encodesTheNewRequests() throws {
        let poll = try json(.send(channelId: "general", body: "Pizza?", nonce: "n", gif: nil, poll: ["yes", "no"]))
        #expect(poll["t"] as? String == "send")
        #expect((poll["poll"] as? [String: Any])?["options"] as? [String] == ["yes", "no"])
        #expect(try json(.send(channelId: "g", body: "x", nonce: nil, gif: nil))["poll"] == nil)

        let vote = try json(.pollVote(id: "m1", option: 2))
        #expect(vote["t"] as? String == "poll.vote")
        #expect(vote["option"] as? Int == 2)
        let unvote = try json(.pollVote(id: "m1", option: nil))
        #expect(unvote.keys.contains("option") && unvote["option"] is NSNull, "null takes it back")

        #expect(try json(.soundPlay(id: "s1")) as NSDictionary == ["t": "sound.play", "id": "s1"])
        #expect(try json(.soundDelete(id: "s1")) as NSDictionary == ["t": "sound.delete", "id": "s1"])
        #expect(try json(.shelf(channelId: "general", before: 5)) as NSDictionary == ["t": "shelf", "channelId": "general", "before": 5])
        #expect(try json(.shelf(channelId: "general", before: nil))["before"] == nil)
        #expect(try json(.unreadCounts(since: ["general": 7])) as NSDictionary == ["t": "unread.counts", "since": ["general": 7]])
        #expect(try json(.hangSet(at: 1_800_000, title: "Game night", channelId: "lounge")) as NSDictionary
                == ["t": "hang.set", "at": 1_800_000, "title": "Game night", "channelId": "lounge"])
        #expect(try json(.hangSet(at: 1_800_000, title: "Hang", channelId: nil))["channelId"] == nil)
        #expect(try json(.hangCancel(id: "h1")) as NSDictionary == ["t": "hang.cancel", "id": "h1"])
        #expect(try json(.watchQueue(add: "dQw4w9WgXcQ")) as NSDictionary == ["t": "watch.queue", "add": "dQw4w9WgXcQ"])
        #expect(try json(.notifySettings(mode: "pings")) as NSDictionary == ["t": "notify.settings", "mode": "pings"])

        let away = try json(.away(text: "at work til 6", until: nil))
        #expect(away["t"] as? String == "profile")
        let body = away["away"] as? [String: Any]
        #expect(body?["text"] as? String == "at work til 6")
        #expect(body?["until"] is NSNull, "until I'm back = null")
        #expect(try json(.away(text: "nap", until: 99))["away"] as? NSDictionary == ["text": "nap", "until": 99])
        let off = try json(.awayOff)
        #expect(off["t"] as? String == "profile" && off["away"] is NSNull)
        #expect(try json(.chime("tada")) as NSDictionary == ["t": "profile", "chime": "tada"])
        #expect(try json(.chime(nil))["chime"] is NSNull)

        let quiet = try json(.notifyQuiet(QuietHours(from: 1380, to: 480, tz: "Europe/Rome", mentions: true, until: nil)))
        #expect(quiet["t"] as? String == "notify.quiet")
        let q = quiet["quiet"] as? [String: Any]
        #expect(q?["from"] as? Int == 1380 && q?["to"] as? Int == 480 && q?["tz"] as? String == "Europe/Rome" && q?["mentions"] as? Bool == true)
        #expect(try json(.notifyQuiet(nil))["quiet"] is NSNull)
    }

    // MARK: Server → client

    @Test func decodesTheNewFrames() throws {
        guard case let .poll(id, channelId, votes) = try ServerMessage.decode(#"{"t":"poll","id":"m","channelId":"general","votes":[[],["u2"],[]]}"#) else {
            Issue.record("poll"); return
        }
        #expect(id == "m" && channelId == "general" && votes == [[], ["u2"], []])

        guard case let .sound(id: soundId, sid: sid, uid: uid) = try ServerMessage.decode(#"{"t":"sound","id":"s1","sid":"x","uid":"u"}"#) else {
            Issue.record("sound"); return
        }
        #expect(soundId == "s1" && sid == "x" && uid == "u")

        let added = #"{"t":"sound.added","sound":{"id":"s1","kind":"board","name":"Airhorn","emoji":"📯","media":"m1","gain":0.38,"ms":800,"uid":"u","createdAt":5}}"#
        guard case .soundAdded(let sound) = try ServerMessage.decode(added) else { Issue.record("sound.added"); return }
        #expect(sound == Sound(id: "s1", kind: "board", name: "Airhorn", emoji: "📯", media: "m1", gain: 0.38, ms: 800, uid: "u", createdAt: 5))
        guard case .soundDeleted(let gone) = try ServerMessage.decode(#"{"t":"sound.deleted","id":"s1"}"#) else { Issue.record("sound.deleted"); return }
        #expect(gone == "s1")

        guard case .away(let note) = try ServerMessage.decode(#"{"t":"away","uid":"u","text":"at work til 6","until":null}"#) else {
            Issue.record("away"); return
        }
        #expect(note == AwayNote(uid: "u", text: "at work til 6", until: nil))

        let shelf = #"{"t":"shelf","channelId":"general","items":[{"id":"m","channelId":"general","uid":"u","body":"","ts":3,"image":{"id":"i","width":4,"height":3}},{"bad":true}],"hasMore":true}"#
        guard case let .shelf(shelfChannel, items, hasMore) = try ServerMessage.decode(shelf) else { Issue.record("shelf"); return }
        #expect(shelfChannel == "general" && items.map(\.id) == ["m"] && hasMore, "a malformed item is dropped, not the page")

        let counts = #"{"t":"unread.counts","counts":{"general":{"n":12,"first":{"id":"m","ts":9,"uid":"u","body":"hey"}},"random":{"n":0,"first":null}}}"#
        guard case .unreadCounts(let map) = try ServerMessage.decode(counts) else { Issue.record("unread.counts"); return }
        #expect(map["general"] == UnreadCount(n: 12, first: .init(id: "m", ts: 9, uid: "u", body: "hey")))
        #expect(map["random"] == UnreadCount(n: 0, first: nil))
        #expect(UnreadCount(n: 999).label == "999+")

        guard case .hang(let hang) = try ServerMessage.decode(#"{"t":"hang","hang":{"id":"h","at":1800000,"title":"Game night","channelId":"lounge","uid":"u","createdAt":1}}"#) else {
            Issue.record("hang"); return
        }
        #expect(hang == Hang(id: "h", at: 1_800_000, title: "Game night", channelId: "lounge", uid: "u", createdAt: 1))
        guard case .hangCancelled("h") = try ServerMessage.decode(#"{"t":"hang.cancelled","id":"h"}"#) else { Issue.record("hang.cancelled"); return }
        guard case .hangNow("h") = try ServerMessage.decode(#"{"t":"hang.now","id":"h"}"#) else { Issue.record("hang.now"); return }

        guard case .notify(let notify) = try ServerMessage.decode(#"{"t":"notify","mode":"pings","topic":"abc","quiet":{"from":1380,"to":480,"tz":"Europe/Rome","mentions":false,"until":null}}"#) else {
            Issue.record("notify"); return
        }
        #expect(notify.mode == "pings" && notify.topic == "abc")
        #expect(notify.quiet == QuietHours(from: 1380, to: 480, tz: "Europe/Rome", mentions: false, until: nil))
        guard case .notify(let off) = try ServerMessage.decode(#"{"t":"notify","mode":"off","topic":null,"quiet":null}"#) else { Issue.record("notify off"); return }
        #expect(off.quiet == nil)
    }

    @Test func readyCarriesSoundsHangsAndQuiet() throws {
        let frame = #"""
        {"t":"ready","v":1,"sid":"s","uid":"u","needsPin":false,"channels":[],"users":[
          {"id":"u","name":"Ada","color":"#fff","lastSeen":0,"role":"member","avatar":null,"away":{"text":"brb","until":null,"since":4},"chime":"tada"}],
         "sessions":[],"notify":{"mode":"off","topic":"t","quiet":{"from":0,"to":420,"tz":"UTC","mentions":true,"until":null}},
         "sounds":[{"id":"s1","kind":"chime","name":"Ada","emoji":null,"media":"m","gain":1,"ms":900,"uid":"u","createdAt":1},{"nope":1}],
         "hangs":[{"id":"h","at":1800000,"title":"Movie","channelId":null,"uid":"u","createdAt":0}]}
        """#
        guard case .ready(let ready) = try ServerMessage.decode(frame) else { Issue.record("ready"); return }
        #expect(ready.sounds.map(\.id) == ["s1"])
        #expect(ready.sounds.first?.isChime == true)
        #expect(ready.hangs.map(\.title) == ["Movie"])
        #expect(ready.notify?.quiet?.to == 420)
        #expect(ready.users.first?.away == Away(text: "brb", until: nil, since: 4))
        #expect(ready.users.first?.chime == "tada")
    }

    @Test func messagesCarryPollsRollsAndReplyChannels() throws {
        let frame = #"""
        {"t":"message","message":{"id":"m","channelId":"p_u","uid":"u","body":"🎲 2d6+3: 4 + 2 + 3 = 9","ts":1,
          "poll":{"options":["a","b"],"votes":[["u"]]},
          "roll":{"kind":"roll","spec":"2d6+3","rolls":[4,2],"total":9},
          "reply":{"id":"o","uid":"x","channelId":"general","body":"hi"},
          "reactions":[{"emoji":"<d:abcdef0123456789abcdef0123456789>","uids":["x"]},{"emoji":"👍","uids":["u"]}]}}
        """#
        guard case .message(let m, _) = try ServerMessage.decode(frame) else { Issue.record("message"); return }
        #expect(m.poll?.votes == [["u"], []], "votes padded to the options")
        #expect(m.poll?.choice(of: "u") == 0)
        #expect(m.roll == Roll(kind: "roll", spec: "2d6+3", rolls: [4, 2], total: 9))
        #expect(m.roll?.headline == "9")
        #expect(m.roll?.detail == "2d6+3 · 4 + 2")
        #expect(m.reply?.channelId == "general")
        #expect(m.doodles.map(\.emoji) == ["<d:abcdef0123456789abcdef0123456789>"])
        #expect(m.emojiReactions.map(\.emoji) == ["👍"])
        #expect(Doodle.id(fromReaction: "<d:abc>") == "abc")
        #expect(Doodle.id(fromReaction: "<:name:abc>") == nil)
        #expect(Doodle.mine(on: m, uid: "x")?.uids == ["x"])
        let flip = Roll(kind: "flip", spec: "coin", rolls: [1], total: 1)
        #expect(flip.headline == "Tails" && flip.detail.isEmpty)
        #expect(Roll(kind: "roll", spec: "1-100", rolls: [42], total: 42).detail == "1–100")
    }

    // MARK: Polls

    @Test func pollRules() {
        #expect(SlashCommand.parse("/poll Pizza or tacos? | pizza | tacos") == .poll(question: "Pizza or tacos?", options: ["pizza", "tacos"]))
        #expect(SlashCommand.parse("/POLL Q|a|b|") == .poll(question: "Q", options: ["a", "b"]))
        #expect(SlashCommand.parse("let's /poll later") == nil)
        #expect(SlashCommand.parse("/pollen") == nil)
        #expect(SlashCommand.isDice("/roll 2d6") && SlashCommand.isDice("/flip") && SlashCommand.isDice("/roll"))
        #expect(!SlashCommand.isDice("/rolling"))
        #expect(Poll.problem(question: "Q", options: ["a"]) != nil)
        #expect(Poll.problem(question: "Q", options: ["a", "A"]) != nil, "no duplicates")
        #expect(Poll.problem(question: " ", options: ["a", "b"]) != nil)
        #expect(Poll.problem(question: "Q", options: Array(repeating: "x", count: 11).enumerated().map { "\($0.offset)" }) != nil)
        #expect(Poll.problem(question: "Q", options: ["a", "b", ""]) == nil, "blank answers are ignored")
        var poll = Poll(options: ["a", "b", "c"])
        poll = poll.voting("me", for: 1)
        #expect(poll.votes == [[], ["me"], []])
        poll = poll.voting("me", for: 2).voting("you", for: 2)
        #expect(poll.votes == [[], [], ["me", "you"]] && poll.total == 2 && poll.share(2) == 1)
        poll = poll.voting("me", for: nil)
        #expect(poll.votes == [[], [], ["you"]])
        #expect(poll.choice(of: "me") == nil)
    }

    // MARK: Spoilers

    @Test func spoilersParseHideAndReveal() {
        let body = "the ending: ||he was a ghost|| and ||the dog lives||"
        #expect(Spoilers.contains(body))
        #expect(Spoilers.hidden(body) == "the ending: ▒▒▒ and ▒▒▒")
        #expect(!Spoilers.contains("unclosed || stays"))
        #expect(Spoilers.hidden("`||code||` and ||yes||") == "`||code||` and ▒▒▒", "not inside code")
        #expect(Spoilers.hidden("||two\nlines||") == "||two\nlines||", "no newlines")

        let hidden = MessageFormatter.parse("look ||https://example.com @Ada|| ok", names: ["Ada"], hideSpoilers: true)
        guard case .paragraph(let inlines) = hidden.first else { Issue.record("paragraph"); return }
        #expect(inlines.contains { if case let .text(t, style) = $0 { t.contains("https://example.com") && style.contains(.spoiler) } else { false } })
        #expect(!inlines.contains { if case .link = $0 { true } else { false } }, "a hidden link can't be tapped")
        #expect(!inlines.contains { if case .mention = $0 { true } else { false } })

        let shown = MessageFormatter.parse("look ||https://example.com @Ada|| ok", names: ["Ada"])
        guard case .paragraph(let open) = shown.first else { Issue.record("paragraph"); return }
        #expect(open.contains { if case let .link(_, text, style) = $0 { text == "https://example.com" && style.contains(.spoiler) } else { false } },
                "revealed: the link works and stops at the closing ||")
        #expect(open.contains { if case .mention("Ada") = $0 { true } else { false } })

        #expect(MessageFormatter.flatten("thinking **Overcooked** at ~~8~~ 9, ||Rosebud||\nsee `code`") == "thinking Overcooked at 8 9, ▒▒▒ see code")
        // Previews everywhere (reply quotes, notifications) are plain text too.
        let m = Message(id: "m", channelId: "c", uid: "u", body: "who's around? **Overcooked** at _9_ ||secret||\nline two", ts: 1)
        #expect(MessageFormatter.preview(m) == "who's around? Overcooked at 9 ▒▒▒ line two")
        #expect(Reply(quoting: m).preview == "who's around? Overcooked at 9 ▒▒▒…")
        let message = Message(id: "m", channelId: "c", uid: "u", body: "it was ||Rosebud||", ts: 0)
        #expect(MessageFormatter.preview(message) == "it was ▒▒▒")
        #expect(Reply(id: "m", uid: "u", body: "it was ||Rosebud||").preview == "it was ▒▒▒")
    }

    // MARK: YouTube

    @Test func youTubeLinks() {
        #expect(YouTube.firstVideoID(in: "watch https://www.youtube.com/watch?v=dQw4w9WgXcQ now") == "dQw4w9WgXcQ")
        #expect(YouTube.firstVideoID(in: "https://youtu.be/dQw4w9WgXcQ?t=3") == "dQw4w9WgXcQ")
        #expect(YouTube.firstVideoID(in: "https://youtube.com/shorts/abcdefghijk") == "abcdefghijk")
        #expect(YouTube.firstVideoID(in: "https://music.youtube.com/watch?v=abcdefghijk&list=x") == "abcdefghijk")
        #expect(YouTube.firstVideoID(in: "https://m.youtube.com/watch?feature=share&v=abcdefghijk") == "abcdefghijk")
        #expect(YouTube.firstVideoID(in: "two: https://youtu.be/aaaaaaaaaaa https://youtu.be/bbbbbbbbbbb") == "aaaaaaaaaaa", "first one")
        #expect(YouTube.firstVideoID(in: "https://youtu.be/short") == nil)
        #expect(YouTube.firstVideoID(in: "https://youtu.be/abcdefghijkl") == nil, "12 characters isn't an id")
        #expect(YouTube.firstVideoID(in: "(see https://youtu.be/dQw4w9WgXcQ).") == "dQw4w9WgXcQ", "trailing punctuation")
        #expect(YouTube.firstVideoID(in: "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=3") == "dQw4w9WgXcQ")
        #expect(YouTube.firstVideoID(in: "https://youtube.com/live/abcdefghijk") == "abcdefghijk")
        #expect(YouTube.videoID("dQw4w9WgXcQ") == "dQw4w9WgXcQ", "a bare id")
        #expect(YouTube.videoID("youtu.be/dQw4w9WgXcQ") == "dQw4w9WgXcQ", "no scheme")
        #expect(YouTube.firstVideoID(in: "https://notyoutube.com/watch?v=dQw4w9WgXcQ") == nil)
    }

    // MARK: Quiet hours, hangs, away

    @Test func quietHoursFollowTheServersRule() {
        let rome = TimeZone(identifier: "Europe/Rome")!
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = rome
        let midnight = calendar.date(from: DateComponents(year: 2026, month: 10, day: 5, hour: 0, minute: 30))!
        let noon = calendar.date(from: DateComponents(year: 2026, month: 10, day: 5, hour: 12))!
        let night = QuietHours(from: 23 * 60, to: 8 * 60, tz: "Europe/Rome", mentions: true)
        #expect(night.inWindow(at: midnight) && !night.inWindow(at: noon), "wraps past midnight")
        #expect(night.holds(at: midnight, mention: false))
        #expect(!night.holds(at: midnight, mention: true), "mentions let through")
        #expect(!night.holds(at: noon, mention: false))
        let strict = QuietHours(from: 23 * 60, to: 8 * 60, tz: "Europe/Rome", mentions: false)
        #expect(strict.holds(at: midnight, mention: true))
        #expect(!QuietHours(from: 600, to: 600, tz: "UTC").inWindow(at: noon), "from = to: no daily window")
        let snoozed = QuietHours(from: 600, to: 600, tz: "UTC", mentions: false, until: noon.yapMillis + 60_000)
        #expect(snoozed.holds(at: noon, mention: true) && !snoozed.holds(at: noon.addingTimeInterval(120), mention: false))
        let morning = night.tomorrowMorning(after: midnight)
        #expect(calendar.component(.hour, from: morning) == 8 && morning > midnight && morning.timeIntervalSince(midnight) < 86_400)
        #expect(QuietHours.label(23 * 60).isEmpty == false)
    }

    @Test func hangSlotsAndAway() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let first = Hang.firstSlot(after: now)
        #expect(first.timeIntervalSince1970.truncatingRemainder(dividingBy: 1800) == 0)
        #expect(first.timeIntervalSince(now) >= Hang.soonest && first.timeIntervalSince(now) < Hang.soonest + 1800)
        #expect(Hang.snap(Date(timeIntervalSince1970: 1800 * 10 + 600)).timeIntervalSince1970 == 1800 * 10)
        let hang = Hang(id: "h", at: now.yapMillis, title: "x")
        #expect(hang.isOn(at: now.addingTimeInterval(60)) && !hang.isOn(at: now.addingTimeInterval(3 * 3600)) && !hang.isOn(at: now.addingTimeInterval(-60)))

        let away = Away(text: "at work til 6", until: now.yapMillis + 1000)
        #expect(away.isOn(at: now) && !away.isOn(at: now.addingTimeInterval(2)))
        #expect(Away(text: "brb").isOn(at: now))
        #expect(away.line(name: "Ada") == "Ada is away: at work til 6")
        let user = User(id: "u", name: "Ada", color: "#fff", away: away)
        #expect(user.awayNow(now) != nil && user.awayNow(now.addingTimeInterval(5)) == nil)
    }

    // MARK: Chimes, soundboard, WAV

    @Test func chimesAreTheSpecsTable() {
        #expect(ChimeTone.all.map(\.id) == ["default", "ding", "chord", "tada", "bubble", "knock", "arcade", "none"])
        let tada = ChimeTone.named("tada")!
        #expect(tada.notes == [392, 523.25, 659.25, 783.99, 1046.5] && tada.step == 0.07 && tada.length == 0.25 && tada.volume == 0.06 && tada.wave == .triangle)
        #expect(ChimeTone.named(nil)?.id == "default")
        #expect(ChimeTone.named("none")!.render().isEmpty)
        let samples = ChimeTone.named("default")!.render(sampleRate: 48_000)
        // Two notes 75 ms apart, each 110 ms + 20 ms of tail.
        #expect(abs(Double(samples.count) - (0.075 + 0.11 + 0.02) * 48_000) <= 1)
        let peak = samples.map { abs($0) }.max() ?? 0
        #expect(peak > 0.04 && peak <= 0.1, "about the table's volume (0.05), overlap included: \(peak)")
        #expect(abs(samples[0]) < 0.001, "starts from silence (12 ms attack)")
        #expect(abs(samples.last!) < 0.001, "decays to silence")
        #expect(ChimeChoice.from(nil) == .builtIn(ChimeTone.all[0]))
        #expect(ChimeChoice.from("s:abc") == .clip(soundId: "abc"))
        #expect(ChimeChoice.from("kazoo") == .builtIn(ChimeTone.all[0]), "unknown → default")
    }

    @Test func soundboardVolumeNeverExceedsTheMeasuredGain() {
        #expect(Soundboard.defaultVolume == 0.5)
        #expect(Soundboard.playbackGain(clipGain: 0.38, volume: 0.5) == 0.19)
        #expect(Soundboard.playbackGain(clipGain: 0.38, volume: 1) == 0.38)
        #expect(Soundboard.playbackGain(clipGain: 0.38, volume: 3) == 0.38, "the volume can't go past 100 %")
        #expect(Soundboard.playbackGain(clipGain: 2, volume: 1) == 1, "never louder than uploaded")
        #expect(Soundboard.shouldPlay(muted: false, deafened: false, personVolume: nil))
        #expect(!Soundboard.shouldPlay(muted: true, deafened: false, personVolume: 1), "mute switch")
        #expect(!Soundboard.shouldPlay(muted: false, deafened: true, personVolume: 1), "silent while deafened")
        #expect(!Soundboard.shouldPlay(muted: false, deafened: false, personVolume: 0), "people I've muted")
    }

    @Test func wavRoundTripTrimAndResample() throws {
        let tone = (0..<24_000).map { Float(sin(Double($0) * 2 * .pi * 440 / 24_000) * 0.5) }
        let wav = WAV.encode(tone, sampleRate: 24_000)
        #expect(wav.count == 44 + 48_000)
        #expect(String(decoding: wav.prefix(4), as: UTF8.self) == "RIFF")
        let decoded = try #require(WAV.decode(wav))
        #expect(decoded.sampleRate == 24_000 && decoded.samples.count == 24_000)
        #expect(abs(decoded.samples[100] - tone[100]) < 0.0001)
        #expect(decoded.seconds == 1)
        #expect(WAV.decode(Data("nope".utf8)) == nil)

        let trimmed = WAV.trim(tone, sampleRate: 24_000, start: 0.25, seconds: 0.5)
        #expect(trimmed.count == 12_000)
        #expect(trimmed.first == 0 && trimmed.last == 0, "faded in and out")
        #expect(WAV.trim(tone, sampleRate: 24_000, start: 0.9, seconds: 5).count == 2_400, "clamped to the end")

        let up = WAV.resample(tone, from: 24_000, to: 48_000)
        #expect(up.count == 48_000)
        let down = WAV.resample(tone, from: 24_000, to: 8_000)
        #expect(down.count == 8_000)
        let (peak, rms) = WAV.levels(tone)
        #expect(abs(peak - 0.5) < 0.01 && abs(rms - 0.3536) < 0.01)
    }

    // MARK: Doodles and clips

    @Test func doodlesExportAsSmallGIFs() throws {
        var drawing = DoodleDrawing()
        #expect(drawing.isEmpty)
        drawing.strokes = [
            .init(color: 1, size: 2, points: [.init(x: 4, y: 4), .init(x: 40, y: 40)]),
            .init(color: 2, size: 4, points: [.init(x: 40, y: 6), .init(x: 8, y: 30), .init(x: 30, y: 44)]),
            .init(color: 0, size: 4, points: [.init(x: 22, y: 22)]),
        ]
        #expect(!drawing.isEmpty)
        let still = drawing.pixels()
        #expect(still.count == 48 * 48)
        #expect(still[4 * 48 + 4] == 1 && still[40 * 48 + 40] == 1, "the line's ends are inked")
        #expect(still[22 * 48 + 22] == 0, "the eraser dab is paper")
        #expect(drawing.pixels(frame: 0) == drawing.pixels(frame: 0), "deterministic")

        let gif = drawing.gif()
        let source = try #require(CGImageSourceCreateWithData(gif as CFData, nil))
        #expect(CGImageSourceGetCount(source) == 1)
        let image = try #require(CGImageSourceCreateImageAtIndex(source, 0, nil))
        #expect(image.width == 96 && image.height == 96, "2×")

        drawing.wiggle = true
        #expect(drawing.pixels(frame: 1) != drawing.pixels(frame: 0) || drawing.pixels(frame: 2) != drawing.pixels(frame: 0), "it wiggles")
        let animated = drawing.gif()
        let frames = try #require(CGImageSourceCreateWithData(animated as CFData, nil))
        #expect(CGImageSourceGetCount(frames) == 3)
        #expect(animated.count < Doodle.maxBytes, "\(animated.count) bytes")

        // Pixels survive the LZW round trip exactly (a noisy frame exercises code growth and resets).
        let side = 96
        var noisy = [UInt8](repeating: 0, count: side * side)
        var seed: UInt32 = 7
        for i in noisy.indices {
            seed = seed &* 1_103_515_245 &+ 12_345
            noisy[i] = UInt8((seed >> 16) % 12)
        }
        let data = GIFEncoder.encode(width: side, height: side, frames: [noisy], palette: DoodleDrawing.palette)
        let noisySource = try #require(CGImageSourceCreateWithData(data as CFData, nil))
        let decoded = try #require(CGImageSourceCreateImageAtIndex(noisySource, 0, nil))
        let ctx = CGContext(data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side * 4,
                            space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        ctx.draw(decoded, in: CGRect(x: 0, y: 0, width: side, height: side))
        let bytes = ctx.data!.assumingMemoryBound(to: UInt8.self)
        var mismatches = 0
        for i in noisy.indices {
            let want = DoodleDrawing.palette[Int(noisy[i])]
            let px = i * 4
            if abs(Int(bytes[px]) - Int(want.r)) > 2 || abs(Int(bytes[px + 1]) - Int(want.g)) > 2 || abs(Int(bytes[px + 2]) - Int(want.b)) > 2 {
                mismatches += 1
            }
        }
        #expect(mismatches == 0)
    }

    @Test func clipsDecodeTrimAndEncode() throws {
        // A stereo 44.1 kHz AAC file, like a voice memo.
        let url = FileManager.default.temporaryDirectory.appending(path: "yap-clip-\(UUID().uuidString).m4a")
        defer { try? FileManager.default.removeItem(at: url) }
        let settings: [String: Any] = [AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 44_100, AVNumberOfChannelsKey: 2]
        do {
            let file = try AVAudioFile(forWriting: url, settings: settings)
            let format = file.processingFormat
            let frames = AVAudioFrameCount(44_100 * 3)
            let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames))
            buffer.frameLength = frames
            for c in 0..<Int(format.channelCount) {
                for i in 0..<Int(frames) { buffer.floatChannelData![c][i] = Float(sin(Double(i) * 2 * .pi * 440 / 44_100) * 0.6) }
            }
            try file.write(from: buffer)
        }
        let clip = try SoundClip.load(url)
        #expect(abs(clip.seconds - 3) < 0.15, "\(clip.seconds) s at 24 kHz")
        #expect(WAV.levels(clip.samples).peak > 0.4)
        let (wav, ms) = try clip.wav(start: 0.5, seconds: Soundboard.maxChimeSeconds)
        #expect(ms == 2000)
        let decoded = try #require(WAV.decode(wav))
        #expect(decoded.sampleRate == 24_000 && decoded.samples.count == 48_000)
        #expect(wav.count <= Soundboard.maxBytes)
        #expect(clip.outline(20).count == 20)
        let padded = SoundClip(samples: [Float](repeating: 0, count: 12_000) + [Float](repeating: 0.5, count: 2_400), name: "x")
        #expect(abs(padded.firstSound - 0.49) < 0.001, "leading silence skipped, 10 ms kept")
        #expect(throws: SoundClip.Failure.unreadable) { try SoundClip.load(URL(fileURLWithPath: "/nonexistent.m4a")) }
        #expect(throws: SoundClip.Failure.silent) { try SoundClip(samples: [Float](repeating: 0, count: 24_000), name: "x").wav(start: 0, seconds: 1) }
    }

    // MARK: The client applies them

    @MainActor
    @Test func clientAppliesExtrasFrames() throws {
        let client = YapClient(credentials: MemoryCredentialStore(), client: "test")
        let ready = ReadyPayload(sid: "me-sid", uid: "me", channels: [
            Channel(id: "general", name: "general", kind: .text, position: 0, lastMessageAt: 10),
            Channel(id: "p_me", name: "Personal space", kind: .personal, position: 0),
        ], users: [User(id: "me", name: "Me", color: "#fff"), User(id: "ada", name: "Ada", color: "#000")], sessions: [])
        let pollMessage = Message(id: "p1", channelId: "general", uid: "ada", body: "Pizza?", ts: 5, poll: Poll(options: ["yes", "no"]))
        let mine = Message(id: "m1", channelId: "general", uid: "me", body: "@Ada you around?", ts: 9)
        client.loadDemo(ready: ready, histories: ["general": [pollMessage, mine]])

        client.receiveForTesting(.poll(id: "p1", channelId: "general", votes: [["ada"], []]))
        #expect(client.histories["general"]?.messages.first?.poll?.votes == [["ada"], []])

        client.receiveForTesting(.away(AwayNote(uid: "ada", text: "at work til 6")))
        #expect(client.awayNotes["m1"] == [AwayNote(uid: "ada", text: "at work til 6")], "under my newest message")

        let sound = Sound(id: "s1", name: "Airhorn", media: "m", gain: 0.4)
        var heard: [String] = []
        client.onSound = { sound, _, uid in heard.append("\(sound.id)/\(uid)") }
        client.receiveForTesting(.soundAdded(sound))
        client.receiveForTesting(.sound(id: "s1", sid: "x", uid: "ada"))
        #expect(heard == ["s1/ada"])
        #expect(client.boardSounds.map(\.id) == ["s1"])
        client.receiveForTesting(.soundDeleted(id: "s1"))
        #expect(client.boardSounds.isEmpty)

        let later = Hang(id: "h2", at: Date().yapMillis + 7_200_000, title: "Later")
        let soon = Hang(id: "h1", at: Date().yapMillis + 3_600_000, title: "Soon")
        client.receiveForTesting(.hang(later))
        client.receiveForTesting(.hang(soon))
        #expect(client.upcomingHangs().map(\.id) == ["h1", "h2"])
        var started: Hang?
        client.onHangNow = { started = $0 }
        client.receiveForTesting(.hangNow(id: "h1"))
        #expect(started?.id == "h1" && client.hangStarting?.id == "h1")
        client.receiveForTesting(.hangCancelled(id: "h1"))
        #expect(client.hangStarting == nil && client.hangs.map(\.id) == ["h2"])

        client.receiveForTesting(.notify(NotifySettings(mode: "pings", topic: "t", quiet: QuietHours(from: 0, to: 0, tz: "UTC", mentions: false,
                                                                                                         until: Date().yapMillis + 60_000))))
        #expect(client.quietHolds(mention: true), "snoozed")

        client.receiveForTesting(.shelf(channelId: "general", items: [mine], hasMore: false))
        #expect(client.shelves["general"]?.items.map(\.id) == ["m1"] && client.shelves["general"]?.loading == false)

        client.receiveForTesting(.unreadCounts(["general": UnreadCount(n: 3)]))
        #expect(client.unreadCounts?["general"]?.n == 3)
    }
}
