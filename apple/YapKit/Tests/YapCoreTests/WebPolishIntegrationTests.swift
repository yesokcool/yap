import Foundation
import Testing
@testable import YapCore

/// The web's 2026-10-07 polish against a local dev server: mentions that
/// came while you were away, newcomers starting caught up, a reused personal
/// link's `usedBy`, and the "joined" line. Part of the serialized
/// integration suite (people being let in show up on everyone's screen).
extension ServerIntegrationTests {
    // (Names no other test uses: the server refuses a name that's taken.)
    @Test func mentionsWhileAwayGetTheirBadge() async throws {
        let adaName = "Oona \(TestServer.run)"
        let (ada, adaStore) = try await member(adaName)
        let (ben, _) = try await member("Pax \(TestServer.run)")
        #expect(ada.unreadChannels.isEmpty, "a newcomer starts caught up")
        #expect(!adaStore.lastRead.isEmpty, "with read marks saved")

        ada.open(channel: "general")
        ben.open(channel: "general")
        try await waitUntil("loaded") { ada.histories["general"]?.loading == false && ben.histories["general"]?.loading == false }
        let mine = "anyone around? \(TestServer.run)"
        #expect(ada.send(text: mine, in: "general"))
        try await waitUntil("ben has it") { ben.histories["general"]?.messages.contains { $0.body == mine } == true }
        ada.close(channel: "general")
        try await Task.sleep(for: .milliseconds(300))

        // While Ada looks elsewhere: an @mention, a reply to her, and a plain line.
        let original = try #require(ben.histories["general"]?.messages.last { $0.body == mine })
        ben.send(text: "@\(adaName.lowercased()) yes, me \(TestServer.run)", in: "general")
        ben.send(text: "replying \(TestServer.run)", in: "general", replyTo: original)
        let plain = "nothing for you \(TestServer.run)"
        ben.send(text: plain, in: "general")
        try await waitUntil("ada got all three") { ada.histories["general"]?.messages.contains { $0.body == plain } == true }

        // Another device of Ada's with the same read marks: on ready it asks
        // for counts by itself and the mentions from while it was away get the badge.
        let again = YapClient(credentials: adaStore, client: "test-ios")
        again.start()
        try await waitUntil("online") { again.phase == .online }
        try await waitUntil("counts back") { again.unreadCounts?["general"] != nil }
        let count = try #require(again.unreadCounts?["general"])
        #expect(count.n >= 3)
        #expect(count.mentions == 2, "the @mention and the reply; not the plain line")
        #expect(again.mentionCounts["general"] == 2)

        // Catch up › Mark all read.
        again.markAllRead()
        #expect(again.unreadChannels.isEmpty)
        #expect(again.mentionCounts.isEmpty)
    }

    @Test func aReusedLinkAndTheJoinedLine() async throws {
        let inviterName = "Rue \(TestServer.run)"
        // Both let people in, so both are mods (members never see the waiting room).
        let (ada, _) = try await mod(inviterName)
        let (ben, _) = try await mod("Sage \(TestServer.run)")
        // A member: never sees the request, still gets the line (join.resolved.user).
        let (nia, _) = try await member("Nia \(TestServer.run)")
        ada.createInvite(label: "for Tova")
        try await waitUntil("invite made") { ada.createdInvite != nil }
        let created = try #require(ada.createdInvite)
        let link = try #require(ServerEndpoint.parse(invite: created.link))

        func signUp(_ name: String) async throws -> YapClient {
            let newcomer = YapClient(credentials: MemoryCredentialStore(endpoint: link), client: "test-ios")
            newcomer.signUp(name: name, pin: TestServer.password, color: "#30a46c")
            try await waitUntil("\(name) waiting") { if case .waiting = newcomer.phase { true } else { false } }
            try await waitUntil("ben sees \(name)") { ben.waitingRoom.values.contains { $0.name == name } }
            return newcomer
        }

        // The first one with the link: nobody used it before. Ben lets them in.
        let cyName = "Tova \(TestServer.run)"
        let cy = try await signUp(cyName)
        let first = try #require(ben.waitingRoom.values.first { $0.name == cyName })
        #expect(first.invite?.by == inviterName)
        #expect(first.invite?.label == "for Tova")
        #expect(first.invite?.usedBy == [] && first.invite?.warning == nil)
        ben.approve(first.id)
        try await waitUntil("cy in") { cy.phase == .online }
        let general = try #require(ada.firstTextChannel?.id)
        try await waitUntil("ada's joined line") { !ada.localNotes(in: general).isEmpty }
        #expect(ada.localNotes(in: general).last?.plain == "\(cyName) joined — invited by \(inviterName)")
        #expect(ben.localNotes(in: general).last?.plain == "\(cyName) joined — invited by \(inviterName)")
        try await waitUntil("nia's joined line") { nia.localNotes(in: general).last?.plain == "\(cyName) joined — invited by \(inviterName)" }
        #expect(nia.waitingRoom.isEmpty, "a member was never told about the request")
        #expect(ada.notice?.text.contains("let in") != true, "never a popup")
        #expect(cy.localNotes(in: general).map(\.kind) == [.welcome], "You're in. Say hi")
        #expect(cy.unreadChannels.isEmpty, "the newcomer starts caught up")

        // The same link again: whoever lets them in hears who used it before.
        let deeName = "Vera \(TestServer.run)"
        let dee = try await signUp(deeName)
        let second = try #require(ada.waitingRoom.values.first { $0.name == deeName })
        #expect(second.invite?.usedBy == [cyName])
        #expect(second.invite?.warning == "This link was also used by \(cyName) — check with \(inviterName).")
        ada.notice = nil
        ben.notice = nil
        ada.deny(second.id)
        try await waitUntil("turned away") { if case .blocked = dee.phase { true } else { false } }
        try await waitUntil("ben hears it") { ben.notice != nil }
        #expect(ben.notice?.text == "\(inviterName) turned away \(deeName).")
        #expect(ada.notice == nil, "Ada did it herself")
        #expect(ada.localNotes(in: general).allSatisfy { !$0.plain.hasPrefix(deeName) }, "no joined line for someone turned away")
    }
}
