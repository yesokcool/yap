import Foundation
import Testing
@testable import YapCore

/// A message the server turns away (PROTOCOL.md › Messages): `slow_down`
/// goes again by itself, in order; anything else drops the stuck copy.
@Suite("Sends the server turns away")
struct SendLineTests {
    /// a, b, c… written and sent.
    private func line(_ nonces: [String]) -> SendLine {
        var line = SendLine()
        for nonce in nonces {
            line.wrote(nonce)
            line.sent(nonce)
        }
        return line
    }

    @Test func anErrorIsAboutTheOldestUnansweredSend() {
        var line = line(["a", "b", "c"])
        line.echoed("a")
        #expect(line.refused(code: "too_long") == .refused("b"))
        #expect(line.unanswered == ["c"])
        #expect(!line.pacing, "only slow_down paces")
        #expect(line.refused(code: "no_channel") == .refused("c"))
        #expect(line.refused(code: "no_channel") == nil, "nothing in flight")
    }

    @Test func slowDownSendsItAgainWithTheSameNonceInItsTurn() {
        var line = line(["a", "b"])
        #expect(line.refused(code: "slow_down") == .again("a"))
        #expect(line.pacing)
        #expect(line.waiting == ["a"])
        #expect(line.next() == "a", "the same nonce goes again")
        line.sent("a")
        #expect(line.next() == nil, "nothing else waits: pacing ends")
        #expect(!line.pacing)
        line.echoed("b")
        line.echoed("a")
        #expect(line.unanswered.isEmpty)
    }

    @Test func fiveResendsThenItsRefused() {
        var line = line(["a"])
        for attempt in 1...SendLine.maxResends {
            #expect(line.refused(code: "slow_down") == .again("a"), "re-send \(attempt)")
            #expect(line.next() == "a")
            line.sent("a")
        }
        // Six tries in all: the sixth slow_down is the end of it.
        #expect(line.refused(code: "slow_down") == .refused("a"))
        #expect(line.waiting.isEmpty && line.unanswered.isEmpty && line.resends.isEmpty)
    }

    @Test func everythingBehindGoesOneAtATimeInTheOrderWritten() {
        // Twelve at once: ten get through, the last two are slowed down.
        let twelve = (1...12).map { "m\($0)" }
        var line = line(twelve)
        for nonce in twelve.prefix(10) { line.echoed(nonce) }
        #expect(line.refused(code: "slow_down") == .again("m11"))
        // Written while pacing: waits behind them, not ahead.
        line.wrote("m13")
        line.hold("m13")
        #expect(line.refused(code: "slow_down") == .again("m12"))
        #expect(line.waiting == ["m11", "m12", "m13"], "the order they were written in")
        #expect(line.next() == "m11")
        line.sent("m11")
        // m11 is slowed down again: it goes back in front of the others.
        #expect(line.refused(code: "slow_down") == .again("m11"))
        #expect(line.waiting == ["m11", "m12", "m13"])
        #expect(line.resends["m11"] == 2)
        for expected in ["m11", "m12", "m13"] {
            #expect(line.next() == expected)
            line.sent(expected)
        }
        #expect(line.next() == nil)
        #expect(!line.pacing)
    }

    @Test func aNewSocketStartsTheLineOver() {
        var line = line(["a", "b"])
        _ = line.refused(code: "slow_down")
        line.reconnected()
        #expect(line.unanswered.isEmpty && line.waiting.isEmpty && !line.pacing)
        #expect(line.resends["a"] == 1, "re-sends still count")
        #expect(line.position(of: "a")! < line.position(of: "b")!, "the order is kept for the resend")
    }

    @MainActor
    @Test func afterAReconnectTheyGoAgainInTheOrderWritten() {
        var line = SendLine()
        var outbox: [String: Message] = [:]
        let nonces = (1...12).map { "n\($0)-\(UUID().uuidString)" }
        for nonce in nonces {
            // Written in the same millisecond: their times don't tell them apart.
            outbox[nonce] = Message(id: nonce, channelId: "general", uid: "me", body: nonce, ts: 1_000, isPending: true)
            line.wrote(nonce)
        }
        #expect(YapClient.resendOrder(outbox, line: line) == nonces)
    }

    @Test func aDroppedMessageIsForgottenEverywhere() {
        var line = line(["a", "b"])
        _ = line.refused(code: "slow_down")
        line.forget("a")
        #expect(line.waiting.isEmpty && line.unanswered == ["b"] && line.position(of: "a") == nil)
    }
}

@MainActor
@Suite("Sending a lot at once (dev server)", .serialized, .enabled(if: TestServer.configured))
struct SendLineIntegrationTests {
    @Test func twelveAtOnceAllGoOutInTheOrderWritten() async throws {
        let (client, _) = try await member("Burst \(TestServer.run)")
        // The Personal space: the same send limit, and nobody else's tests see it.
        let channel = try #require(client.personalChannel?.id)
        client.open(channel: channel)
        try await waitUntil("history") { client.histories[channel]?.loading == false }
        let tag = "burst-\(TestServer.run)-\(UUID().uuidString.prefix(4))"
        let texts = (1...12).map { "\(tag) #\($0)" }
        for text in texts { #expect(client.send(text: text, in: channel)) }
        // The server takes ten; the rest are slowed down and go one at a time.
        try await waitUntil("slowed down", timeout: .seconds(5)) { client.sendLine.pacing }
        let later = ["\(tag) #13", "\(tag) #14"]
        for text in later { #expect(client.send(text: text, in: channel)) }
        #expect(client.outbox.count >= 2, "still shown as sending meanwhile")
        try await waitUntil("all of them went out", timeout: .seconds(25)) { client.outbox.isEmpty }
        let mine = client.histories[channel]!.messages.map(\.body).filter { $0.hasPrefix(tag) }
        #expect(mine == texts + later, "every one, in the order written")
        #expect(client.notice?.text != "Slow down a little.", "no notice while it's going again")
    }

    @Test func aRefusedMessageGoesAndItsTextComesBack() async throws {
        let (client, _) = try await member("Refused \(TestServer.run)")
        // A channel the server doesn't have: `no_channel`, re: send.
        client.activeChannelId = "nowhere"
        #expect(client.send(text: "hello :wave: nowhere", in: "nowhere"))
        #expect(client.outbox.count == 1)
        try await waitUntil("turned away") { client.outbox.isEmpty }
        #expect(client.notice != nil, "the reason shows")
        #expect(client.returnedDraft?.channelId == "nowhere")
        #expect(client.returnedDraft?.text.hasPrefix("hello ") == true)
        #expect(client.returnedDraft?.text.hasSuffix(" nowhere") == true)

        // Not on that channel any more: nothing comes back to the box.
        client.returnedDraft = nil
        client.activeChannelId = "general"
        #expect(client.send(text: "elsewhere", in: "nowhere"))
        try await waitUntil("turned away again") { client.outbox.isEmpty }
        #expect(client.returnedDraft == nil)
        #expect(client.sendLine.unanswered.isEmpty)
    }
}
