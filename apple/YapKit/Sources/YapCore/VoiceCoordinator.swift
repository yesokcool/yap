import Foundation

/// A session's voice channel changed (a `voice` event), our own echo included.
public struct VoiceChange: Sendable, Equatable {
    public var sid: String
    /// Where it is now (nil = out of voice).
    public var channelId: String?
    /// Its voice state before this event (nil = it wasn't in voice).
    public var before: VoiceState?
    /// Its socket closed without a `voice.leave`: the call may come back under
    /// a new session (PROTOCOL.md › Voice › Keeping calls across reconnects).
    public var lost: Bool
    /// The earlier session whose call this one continues.
    public var prev: String?

    /// The channel it was in before this event.
    public var previous: String? { before?.channelId }

    public init(sid: String, channelId: String?, before: VoiceState? = nil, lost: Bool = false, prev: String? = nil) {
        self.sid = sid
        self.channelId = channelId
        self.before = before
        self.lost = lost
        self.prev = prev
    }
}

/// Voice lives in YapVoice (WebRTC); the client hands it the socket events
/// it cares about. Everything else about a voice call is the coordinator's job.
@MainActor
public protocol VoiceCoordinator: AnyObject {
    /// A `ready` arrived (first connect or a reconnect): rejoin if we were in voice.
    func clientDidBecomeReady()
    /// The socket dropped. Called before `resumeKey` is forgotten, so a call
    /// can be kept and continued under the next session.
    func clientConnectionDropped()
    /// Signed out or blocked: leave voice for good.
    func clientDidSignOut()
    /// A session's voice channel changed (`voice` event), including our own echo.
    func clientVoiceChanged(_ change: VoiceChange)
    func clientSessionLeft(sid: String)
    func clientReceivedSignal(_ data: SignalData, from sid: String)
    /// A channel changed (e.g. its shared voice quality).
    func clientChannelChanged(_ channel: Channel)
}

public extension VoiceCoordinator {
    func clientChannelChanged(_ channel: Channel) {}
}
