import Foundation
import Testing
@testable import YapCore

/// The socket's heartbeat: every ping has its own deadline. An extra ping
/// (Yap coming back on screen, a network change) used to be judged by the
/// regular ping's clock, so one sent just before that check dropped a
/// healthy socket.
@MainActor
@Suite("Heartbeat")
struct HeartbeatTests {
    @Test func aPingIsAnsweredByAnyLaterPong() {
        let sent = ContinuousClock.now
        #expect(!Heartbeat.answered(sentAt: sent, pongAt: nil))
        #expect(!Heartbeat.answered(sentAt: sent, pongAt: sent - .milliseconds(1)), "a pong to an earlier ping says nothing about this one")
        #expect(Heartbeat.answered(sentAt: sent, pongAt: sent))
        #expect(Heartbeat.answered(sentAt: sent, pongAt: sent + .milliseconds(30)))
    }

    @Test func aNewRouteIsAChangeABlipIsNot() {
        let wifi = NetworkPathSignature(satisfied: true, interfaces: "en0,pdp_ip0")
        #expect(wifi != NetworkPathSignature(satisfied: true, interfaces: "pdp_ip0"))
        #expect(wifi == NetworkPathSignature(satisfied: true, interfaces: "en0,pdp_ip0"))
        #expect(wifi != NetworkPathSignature(satisfied: false, interfaces: "en0,pdp_ip0"))
    }

    /// Against the dev server: extra pings all the time, short deadlines,
    /// and the socket must stay up (the old clock dropped it within seconds).
    @Test(.enabled(if: TestServer.configured)) func extraPingsNeverDropAHealthySocket() async throws {
        let endpoint = TestServer.endpoint
        let connection = YapConnection(url: endpoint.webSocketURL, protocols: endpoint.webSocketProtocols)
        connection.pingInterval = .milliseconds(300)
        connection.pongTimeout = .milliseconds(200)
        var closes: [YapConnection.CloseReason] = []
        connection.onClose = { closes.append($0) }
        connection.connect()
        defer { connection.stop() }
        try await waitUntil("open") { connection.isOpen }
        for _ in 0..<300 {
            connection.reconnectNow()
            try await Task.sleep(for: .milliseconds(10))
        }
        try await Task.sleep(for: .milliseconds(400))
        #expect(closes.isEmpty, "dropped: \(closes)")
        #expect(connection.isOpen)
    }
}

/// Frames reach the server in the order they were sent: the protocol leans
/// on it (sends are answered in order, signals applied in order).
@MainActor
@Suite("Frames go out in order (dev server)", .serialized, .enabled(if: TestServer.configured))
struct FrameOrderTests {
    @Test func aBurstOfRequestsIsAnsweredInTheOrderSent() async throws {
        let (client, _) = try await member("Order \(TestServer.run)")
        // The Personal space: nobody else's tests see these.
        let channel = try #require(client.personalChannel?.id)
        for n in 1...10 { client.send(text: "order \(n)", in: channel) }
        try await waitUntil("ten messages to page through") { client.outbox.isEmpty }
        let connection = try #require(client.connection)
        var counts: [Int] = []
        let original = connection.onMessage
        connection.onMessage = { message in
            if case let .history(channelId, messages, _) = message, channelId == channel { counts.append(messages.count) }
            original?(message)
        }
        // History pages of 1, 2, … 10 messages, five times over, all at once:
        // the answers' sizes say in which order the server got them.
        let rounds = 5
        for _ in 0..<rounds {
            for limit in 1...10 { client.send(.history(channelId: channel, before: nil, beforeId: nil, limit: limit)) }
        }
        try await waitUntil("every answer") { counts.count == rounds * 10 }
        #expect(counts == Array(repeating: Array(1...10), count: rounds).flatMap { $0 })
    }
}
