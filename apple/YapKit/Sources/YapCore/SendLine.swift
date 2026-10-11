import Foundation

/// My sends in flight, so an `error {re: "send"}` lands on the right one.
/// The server answers sends in order (each gets its `message` echo with my
/// nonce, or an error), so the error is about the oldest send that hasn't
/// been answered yet. Nothing on the wire changes.
///
/// `slow_down` ("Slow down a little.": more than 10 in a burst, refilled one
/// a second): the same message goes again with the same nonce 1.1 s later,
/// up to 5 times, and everything behind it goes one at a time, 1.1 s apart,
/// until nothing is waiting, so messages keep the order they were written
/// in. Any other refusal drops the stuck copy.
struct SendLine: Sendable, Equatable {
    static let resendAfter: Duration = .milliseconds(1100)
    /// Re-sends after a `slow_down` (6 tries in all), then it's refused.
    static let maxResends = 5

    /// On the wire, not answered yet, oldest first.
    private(set) var unanswered: [String] = []
    /// Waiting to go one at a time, in the order they were written.
    private(set) var waiting: [String] = []
    /// nonce → when it was written (1, 2, 3…): the order to keep.
    private(set) var written: [String: Int] = [:]
    private var lastWritten = 0
    /// `slow_down` re-sends so far, per nonce.
    private(set) var resends: [String: Int] = [:]
    /// Sends go one at a time until `waiting` is empty.
    private(set) var pacing = false

    enum Answer: Sendable, Equatable {
        /// `slow_down`: it waits for its turn (no notice).
        case again(String)
        /// Refused for good (another error, or too many `slow_down`s): drop it, say why.
        case refused(String)
    }

    /// A new message, in the order written.
    mutating func wrote(_ nonce: String) {
        lastWritten += 1
        written[nonce] = lastWritten
    }

    /// Where it stands among what I wrote (nil: not tracked).
    func position(of nonce: String) -> Int? { written[nonce] }

    /// It went out on the wire.
    mutating func sent(_ nonce: String) {
        unanswered.append(nonce)
    }

    /// Its echo came back.
    mutating func echoed(_ nonce: String) {
        unanswered.removeAll { $0 == nonce }
        resends.removeValue(forKey: nonce)
        written.removeValue(forKey: nonce)
    }

    /// An `error {re: "send"}`: about the oldest unanswered send.
    mutating func refused(code: String) -> Answer? {
        guard !unanswered.isEmpty else { return nil }
        let nonce = unanswered.removeFirst()
        let tries = resends[nonce, default: 0]
        guard code == "slow_down", tries < Self.maxResends, written[nonce] != nil else {
            forget(nonce)
            return .refused(nonce)
        }
        resends[nonce] = tries + 1
        hold(nonce)
        pacing = true
        return .again(nonce)
    }

    /// Instead of going now (we're pacing), wait in line in the order written.
    mutating func hold(_ nonce: String) {
        waiting.removeAll { $0 == nonce }
        let mine = written[nonce] ?? .max
        let index = waiting.firstIndex { (written[$0] ?? .max) > mine } ?? waiting.endIndex
        waiting.insert(nonce, at: index)
    }

    /// The next one to go (every `resendAfter`); nil = done pacing.
    mutating func next() -> String? {
        guard !waiting.isEmpty else {
            pacing = false
            return nil
        }
        return waiting.removeFirst()
    }

    /// Dropped from the outbox (refused): forget it everywhere.
    mutating func forget(_ nonce: String) {
        unanswered.removeAll { $0 == nonce }
        waiting.removeAll { $0 == nonce }
        resends.removeValue(forKey: nonce)
        written.removeValue(forKey: nonce)
    }

    /// A new socket: nothing on it is in flight (the outbox goes again on
    /// `ready`, in order). Re-send counts and the order carry over.
    mutating func reconnected() {
        unanswered = []
        waiting = []
        pacing = false
    }
}
