import Foundation
import Testing
@testable import YapCore

/// The web's 2026-10-07 polish, on the protocol side (PROTOCOL.md › Since
/// you left, › Identity › Waiting room): mention counts from `unread.counts`,
/// Catch up's order and Mark all read, a reused personal link's `usedBy`,
/// newcomers starting caught up, and the session-only "joined" line.
@MainActor
@Suite("Web polish (catch up, waiting room, joined line)")
struct WebPolishTests {
    static let channels = [
        Channel(id: "general", name: "general", kind: .text, position: 0, lastMessageAt: 5_000),
        Channel(id: "games", name: "games", kind: .text, position: 1, lastMessageAt: 9_000),
        Channel(id: "quiet", name: "quiet", kind: .text, position: 2),
        Channel(id: "lounge", name: "Lounge", kind: .voice, position: 3),
    ]
    static let people = [User(id: "sam", name: "Sam", color: "#5b6cff"), User(id: "ada", name: "Ada", color: "#e5484d"),
                         User(id: "ben", name: "Ben", color: "#30a46c")]

    static func client(lastRead: [String: Int] = [:], pending: [User] = [], phase: YapClient.Phase = .online) -> (YapClient, MemoryCredentialStore) {
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t")
        store.lastRead = lastRead
        let client = YapClient(credentials: store, client: "test")
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "sam", channels: channels, users: people, sessions: [], pending: pending),
                        histories: [:], phase: phase)
        return (client, store)
    }

    // MARK: unread.counts › mentions

    @Test func countsCarryMentionsOrNothing() throws {
        let frame = #"{"t":"unread.counts","counts":{"general":{"n":4,"first":null,"mentions":2},"games":{"n":1,"first":null}}}"#
        guard case .unreadCounts(let counts) = try ServerMessage.decode(frame) else { Issue.record("unread.counts"); return }
        #expect(counts["general"]?.mentions == 2)
        #expect(counts["games"]?.mentions == nil, "an older server sends none: unknown, not 0")
        #expect(counts["games"]?.n == 1)
    }

    @Test func mentionsFromWhileAwaySeedTheBadges() {
        let (client, _) = Self.client(lastRead: ["general": 1, "games": 1])
        client.activeChannelId = "games"
        client.receiveForTesting(.unreadCounts([
            "general": UnreadCount(n: 5, mentions: 2),
            "games": UnreadCount(n: 3, mentions: 1), // open and on screen: no badge
            "quiet": UnreadCount(n: 2), // unknown
        ]))
        #expect(client.mentionCounts["general"] == 2)
        #expect(client.mentionCounts["games"] == nil, "the channel on screen gets no badge")
        #expect(client.mentionCounts["quiet"] == nil, "no mentions field: no badge")

        // Never lower than what's already counted live.
        client.receiveForTesting(.unreadCounts(["general": UnreadCount(n: 5, mentions: 1)]))
        #expect(client.mentionCounts["general"] == 2)
        client.receiveForTesting(.unreadCounts(["general": UnreadCount(n: 6, mentions: 3)]))
        #expect(client.mentionCounts["general"] == 3)
    }

    @Test func catchUpListsMentionsFirst() {
        let (client, _) = Self.client(lastRead: ["general": 1, "games": 1, "quiet": 1])
        client.receiveForTesting(.message(Message(id: "q", channelId: "quiet", uid: "ada", body: "hey", ts: 12_000), nonce: nil))
        #expect(client.catchUpChannels.map(\.id) == ["quiet", "games", "general"], "newest first while nothing mentions me")
        client.receiveForTesting(.unreadCounts([
            "general": UnreadCount(n: 5, first: .init(id: "g", ts: 2_000, uid: "ada", body: "x"), mentions: 2),
            "games": UnreadCount(n: 3, first: .init(id: "m", ts: 8_000, uid: "ada", body: "x"), mentions: 0),
            "quiet": UnreadCount(n: 1, first: .init(id: "q", ts: 12_000, uid: "ada", body: "hey")),
        ]))
        #expect(client.catchUpChannels.map(\.id) == ["general", "quiet", "games"], "the one that mentions me, then the newest")
        #expect(client.mentionsForMe(in: "general") == 2)
        client.receiveForTesting(.message(Message(id: "m2", channelId: "games", uid: "ada", body: "@sam look", ts: 13_000), nonce: nil))
        client.receiveForTesting(.message(Message(id: "m3", channelId: "games", uid: "ben", body: "@here look", ts: 14_000), nonce: nil))
        client.receiveForTesting(.message(Message(id: "m4", channelId: "games", uid: "ben", body: "@sam again", ts: 15_000), nonce: nil))
        #expect(client.mentionsForMe(in: "games") == 3, "live mentions count too")
        #expect(client.catchUpChannels.first?.id == "games", "most mentions first")
    }

    @Test func markAllReadClearsEverything() {
        let (client, store) = Self.client(lastRead: ["general": 1, "games": 1, "quiet": 1])
        client.receiveForTesting(.unreadCounts(["general": UnreadCount(n: 5, mentions: 2)]))
        #expect(client.unreadChannels.map(\.id).sorted() == ["games", "general"])
        client.markAllRead()
        #expect(client.unreadChannels.isEmpty)
        #expect(client.mentionCounts.isEmpty)
        #expect(store.lastRead["general"] == 5_000 && store.lastRead["games"] == 9_000, "saved, up to each channel's newest")
        #expect(store.lastRead["quiet"] == 1, "an empty channel keeps its mark")
        #expect(store.lastRead["lounge"] == nil, "voice channels have no read marks")
    }

    // MARK: Newcomers start caught up

    @Test func aDeviceThatNeverReadAnythingStartsCaughtUp() {
        let (client, store) = Self.client()
        #expect(client.unreadChannels.isEmpty, "not every channel unread since the beginning")
        #expect(store.lastRead == ["general": 5_000, "games": 9_000, "quiet": 1], "saved; voice left out; an empty channel gets 1")
        // Anything new still counts, even in the empty channel.
        client.receiveForTesting(.message(Message(id: "m1", channelId: "quiet", uid: "ada", body: "first!", ts: 10_000), nonce: nil))
        #expect(client.unreadChannels.map(\.id) == ["quiet"])
    }

    @Test func readMarksAlreadyKeptAreLeftAlone() {
        let (client, store) = Self.client(lastRead: ["general": 1])
        #expect(store.lastRead == ["general": 1])
        #expect(client.unreadChannels.map(\.id) == ["games", "general"], "a device that has read before isn't marked caught up")
    }

    // MARK: Waiting room › invite.usedBy

    @Test func aReusedPersonalLinkSaysWhoUsedItBefore() throws {
        let frame = ##"{"t":"join.request","user":{"id":"cleo","name":"Cleo","color":"#12a594","role":"pending","invitedBy":"Ada (for Dee)","invite":{"by":"Ada","label":"for Dee","usedBy":["Dee","Eve"]}}}"##
        guard case .joinRequest(let user) = try ServerMessage.decode(frame) else { Issue.record("join.request"); return }
        #expect(user.invite == InviteInfo(by: "Ada", label: "for Dee", usedBy: ["Dee", "Eve"]))
        #expect(user.invite?.warning == "This link was also used by Dee and Eve — check with Ada.")

        let first = ##"{"t":"join.request","user":{"id":"dee","name":"Dee","color":"#12a594","role":"pending","invite":{"by":"Ada","label":"for Dee","usedBy":[]}}}"##
        guard case .joinRequest(let fresh) = try ServerMessage.decode(first) else { Issue.record("join.request"); return }
        #expect(fresh.invite?.warning == nil, "a first use: nothing to say")
        let old = ##"{"t":"join.request","user":{"id":"x","name":"X","color":"#12a594","role":"pending","invitedBy":"Ada"}}"##
        guard case .joinRequest(let older) = try ServerMessage.decode(old) else { Issue.record("join.request"); return }
        #expect(older.invite == nil && older.invitedBy == "Ada", "older servers: just invitedBy")
    }

    @Test func namesReadAsASentence() {
        #expect(InviteInfo.names(["Ada"]) == "Ada")
        #expect(InviteInfo.names(["Ada", "Ben"]) == "Ada and Ben")
        #expect(InviteInfo.names(["Ada", "Ben", "Cleo"]) == "Ada, Ben and Cleo")
        #expect(InviteInfo.names(["Ada", "Ben", "Cleo", "Dee", "Eve"]) == "Ada, Ben and 3 others")
    }

    @Test func readyPendingCarriesTheInvite() {
        let waiting = User(id: "cleo", name: "Cleo", color: "#12a594", role: .pending, invitedBy: "Ada (for Cleo)",
                           invite: InviteInfo(by: "Ada", label: "for Cleo", usedBy: ["Dee"]))
        let (client, _) = Self.client(pending: [waiting])
        #expect(client.waitingRoom["cleo"]?.invite?.usedBy == ["Dee"])
    }

    // MARK: The joined line (session only, never a popup)

    @Test func letInLeavesAQuietLineInTheFirstChannel() throws {
        let cleo = User(id: "cleo", name: "Cleo", color: "#12a594", role: .pending, invitedBy: "Ada (for Cleo)",
                        invite: InviteInfo(by: "Ada", label: "for Cleo"))
        let (client, _) = Self.client(pending: [cleo])
        client.receiveForTesting(.joinResolved(uid: "cleo", approved: true, by: "ben"))
        #expect(client.waitingRoom.isEmpty)
        #expect(client.notice == nil, "never a popup")
        let note = try #require(client.localNotes(in: "general").first)
        #expect(note.key == "join:cleo")
        #expect(note.kind == .joined(name: "Cleo", color: "#12a594", invitedBy: "Ada", letInBy: "Ben"))
        #expect(note.plain == "Cleo joined — invited by Ada")
        #expect(client.localNotes(in: "games").isEmpty)

        // Older servers: invitedBy "Ada (for Cleo)" names Ada; nobody's invite: who let them in.
        let dee = User(id: "dee", name: "Dee", color: "#8d8d8d", role: .pending, invitedBy: "Ada (for Dee)")
        let eve = User(id: "eve", name: "Eve", color: "#8d8d8d", role: .pending)
        client.receiveForTesting(.joinRequest(dee))
        client.receiveForTesting(.joinRequest(eve))
        client.receiveForTesting(.joinResolved(uid: "dee", approved: true, by: "sam"))
        client.receiveForTesting(.joinResolved(uid: "eve", approved: true, by: "sam"))
        #expect(client.localNotes(in: "general").map(\.plain) == ["Cleo joined — invited by Ada", "Dee joined — invited by Ada",
                                                                   "Eve joined — let in by you"])
    }

    /// Only the owner and mods get `join.request`; everyone else meets the
    /// newcomer in `join.resolved.user` and gets the same line.
    @Test func aMemberWhoNeverSawTheRequestStillGetsTheLine() throws {
        let (client, _) = Self.client()
        let frame = ##"{"t":"join.resolved","uid":"cleo","approved":true,"by":"ada","user":{"id":"cleo","name":"Cleo","color":"#12a594","invitedBy":"Ben (for Cleo)"}}"##
        let message = try ServerMessage.decode(frame)
        guard case let .joinResolved(uid, approved, by, user) = message else { Issue.record("join.resolved"); return }
        #expect(uid == "cleo" && approved && by == "ada" && user?.name == "Cleo" && user?.invitedBy == "Ben (for Cleo)")
        client.receiveForTesting(message)
        #expect(client.notice == nil, "never a popup")
        #expect(client.localNotes(in: "general").map(\.plain) == ["Cleo joined — invited by Ben"])
        #expect(client.localNotes(in: "general").first?.kind == .joined(name: "Cleo", color: "#12a594", invitedBy: "Ben", letInBy: "Ada"))

        // Nobody's invite: who let them in (the admin API: "the owner").
        client.receiveForTesting(try ServerMessage.decode(##"{"t":"join.resolved","uid":"dee","approved":true,"by":"admin","user":{"id":"dee","name":"Dee","color":"#8d8d8d"}}"##))
        #expect(client.localNotes(in: "general").last?.plain == "Dee joined — let in by the owner")
        // Turned away: members were never told about them, so nothing at all.
        client.receiveForTesting(try ServerMessage.decode(##"{"t":"join.resolved","uid":"eve","approved":false,"by":"ada"}"##))
        #expect(client.notice == nil && client.localNotes.count == 2)
        // An older server: no `user`, and a member never knew them: nothing either.
        client.receiveForTesting(try ServerMessage.decode(##"{"t":"join.resolved","uid":"fay","approved":true,"by":"ada"}"##))
        #expect(client.localNotes.count == 2)
    }

    /// The owner and mods keep what the waiting room told them (the link and
    /// its label), and the newcomer isn't told about themselves: "You're in"
    /// says hi.
    @Test func theWaitingRoomsCopyWinsAndNeverAboutMe() throws {
        let cleo = User(id: "cleo", name: "Cleo", color: "#12a594", role: .pending, invitedBy: "Ada (for Cleo)",
                        invite: InviteInfo(by: "Ada", label: "for Cleo"))
        let (client, _) = Self.client(pending: [cleo])
        client.receiveForTesting(.joinResolved(uid: "cleo", approved: true, by: "ben",
                                               user: User(id: "cleo", name: "Cleo", color: "#12a594", invitedBy: "Someone (for Cleo)")))
        #expect(client.localNotes(in: "general").map(\.plain) == ["Cleo joined — invited by Ada"])
        client.receiveForTesting(.joinResolved(uid: "sam", approved: true, by: "ada", user: User(id: "sam", name: "Sam", color: "#5b6cff")))
        #expect(client.localNotes.count == 1, "not about me")
    }

    @Test func turnedAwayIsJustAWordForOthers() {
        let cleo = User(id: "cleo", name: "Cleo", color: "#12a594", role: .pending)
        let dee = User(id: "dee", name: "Dee", color: "#12a594", role: .pending)
        let (client, _) = Self.client(pending: [cleo, dee])
        client.receiveForTesting(.joinResolved(uid: "cleo", approved: false, by: "ada"))
        #expect(client.notice?.text == "Ada turned away Cleo.")
        #expect(client.localNotes.isEmpty)
        client.notice = nil
        client.receiveForTesting(.joinResolved(uid: "dee", approved: false, by: "sam"))
        #expect(client.notice == nil, "I did it: no need to tell me")
    }

    @Test func justLetInSaysHi() async {
        let store = MemoryCredentialStore(endpoint: ServerEndpoint(baseURL: URL(string: "https://chat.example.com")!, key: "k"), token: "t")
        let client = YapClient(credentials: store, client: "test")
        var chimed = 0
        client.onAdmitted = { chimed += 1 }
        client.loadDemo(ready: ReadyPayload(sid: "s0", uid: "sam", channels: Self.channels, users: Self.people, sessions: []),
                        histories: [:], phase: .waiting("Waiting for someone to let you in."))
        client.receiveForTesting(.ready(ReadyPayload(sid: "s1", uid: "sam", channels: Self.channels, users: Self.people, sessions: [])))
        #expect(client.localNotes(in: "general").map(\.kind) == [.welcome])
        #expect(client.localNotes.first?.plain == "You're in. Say hi 👋")
        #expect(chimed == 1)
        // A plain reconnect says nothing more.
        client.receiveForTesting(.ready(ReadyPayload(sid: "s2", uid: "sam", channels: Self.channels, users: Self.people, sessions: [])))
        #expect(client.localNotes.count == 1 && chimed == 1)
        #expect(client.unreadChannels.isEmpty, "a newcomer starts caught up")
    }

    @Test func notesSitAmongTheMessagesByTime() {
        let channel = Self.channels[0]
        let a = Message(id: "a", channelId: "general", uid: "ada", body: "one", ts: 1_000)
        let b = Message(id: "b", channelId: "general", uid: "ada", body: "two", ts: 3_000)
        let notes = [LocalNote(key: "join:x", channelId: "general", ts: 2_000, kind: .welcome),
                     LocalNote(key: "late", channelId: "general", ts: 9_000, kind: .welcome),
                     LocalNote(key: "elsewhere", channelId: "games", ts: 2_000, kind: .welcome)]
        let rows = MessageRows.build(channel: channel, messages: [a, b], pending: [], hasMore: false, loading: false,
                                     unreadAfter: nil, myUid: "sam", notes: notes)
        let shape = rows.compactMap { row -> String? in
            switch row {
            case let .message(m, grouped): "\(m.id)\(grouped ? "+" : "")"
            case .note(let note): "note:\(note.key)"
            default: nil
            }
        }
        #expect(shape == ["a", "note:join:x", "b", "note:late"], "b comes after the line, so it starts its own group")
    }

    // MARK: Mentions: custom emojis are covered like code

    @Test func customEmojisCountAsWords() {
        let wave = "<:wave:\(String(repeating: "a", count: 32))>"
        let people = Self.people.map { (id: $0.id, name: $0.name) }
        #expect(Mentions.find(in: "\(wave)@ben", among: people).uids.isEmpty)
        #expect(Mentions.find(in: "@ben\(wave)", among: people).uids.isEmpty)
        #expect(Mentions.find(in: "\(wave) @ben", among: people).uids == ["ben"])
        #expect(Mentions.find(in: "<:wave:nothex>@ben", among: people).uids == ["ben"], "not a token: just a > before the @")
    }
}
