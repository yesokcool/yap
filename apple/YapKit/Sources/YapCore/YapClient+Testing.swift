import Foundation

/// Tests only (`@testable import YapCore`).
extension YapClient {
    /// Load a canned state without a server.
    func loadDemo(ready: ReadyPayload, histories: [String: [Message]], phase: Phase = .online) {
        onReady(ready)
        for (channel, messages) in histories {
            self.histories[channel] = ChannelHistory(messages: messages, hasMore: false, loading: false)
        }
        self.phase = phase
    }

    /// Feed a server frame as if it arrived.
    func receiveForTesting(_ message: ServerMessage) {
        handle(message)
    }

    /// Lose the socket as if the network had; it reconnects by itself.
    func dropConnectionForTesting() {
        connection?.dropForTesting()
    }
}
