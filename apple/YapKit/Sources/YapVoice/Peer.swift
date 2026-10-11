import Foundation
import os
import YapCore
@preconcurrency import WebRTC

/// One direct connection to another session in the voice channel, with a
/// single bidirectional audio transceiver (PROTOCOL.md › Voice), plus a
/// screen share's video (and its sound, a later audio section) only while
/// someone asked for it (PROTOCOL.md › Screen sharing).
///
/// Glare is resolved with *perfect negotiation*: the session whose sid is
/// lexicographically larger is polite (rolls back on a colliding offer).
/// Signals are applied strictly in order: a candidate can't beat its offer.
///
/// A call outlives the socket (PROTOCOL.md › Voice › Keeping calls across
/// reconnects): while our socket is down (or theirs, `held`), outgoing
/// signals wait in an outbox; afterwards the connection may move to the far
/// side's new session id (`sid` changes) and `resync()` sends what waited.
@MainActor
final class Peer {
    /// The far side's session (moves when either side resumes under a new one).
    var sid: String
    /// Their account, from when the connection was made (a resume must match it).
    let uid: String?
    let pc: RTCPeerConnection
    private let observer: PeerObserver
    private unowned let engine: VoiceEngine
    private var makingOffer = false
    private var ignoreOffer = false
    private var queue: Task<Void, Never>?
    private(set) var closed = false

    var state: VoiceEngine.PeerState = .new
    var speakingUntil = Date.distantPast
    private var lastLevelTimestamp: CFTimeInterval = 0
    private var lastLevelSeenAt = Date.distantPast
    /// Their latest RTP audio level (linear RMS), for the boost limiter.
    private(set) var remoteLevel = 0.0

    /// Their socket is down too (we're keeping the call for them): signals wait.
    var held = false
    /// Signals waiting for a socket (ours or theirs) to come back.
    private var outbox: [SignalData] = []

    /// ICE restarts when the path dies.
    private var path = PathWatch()
    private var recoverTask: Task<Void, Never>?
    /// ICE failed since it was last connected ("needs the voice relay").
    var failed: Bool { path.failed }
    var restarts: Int { path.restarts }

    /// "weak connection" (judged in `judgeQuality`).
    private(set) var quality = LinkQuality()
    private var lastQuality: (at: Date, stats: PeerStats)?
    private var connectedAt: Date?

    /// Keeps a boost past 100 % from clipping.
    private var limiter = BoostLimiter()
    private var appliedVolume: Double = -1
    private var limiterAt = Date()

    /// They asked for our screen (`{screen: "want"}`, until "stop"), and when.
    var wantsScreen = false {
        didSet { if wantsScreen, !oldValue { wantedAt = Date() } }
    }
    private(set) var wantedAt = Date.distantPast
    /// The video transceiver our screen goes out on (kept for the next time they ask).
    private var screenTransceiver: RTCRtpTransceiver?
    /// Their screen's video, while it comes (nil when they aren't sending us one).
    private(set) var remoteScreen: RTCVideoTrack?
    /// Their screen comes with sound (a later audio section that receives).
    private(set) var remoteScreenSound = false

    private static let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)
    private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "voice")

    /// The session with the larger sid is polite (recomputed: either side's sid can change on a resume).
    var polite: Bool { Self.isPolite(mine: engine.mySid, theirs: sid) }

    nonisolated static func isPolite(mine: String?, theirs: String) -> Bool {
        (mine ?? "") > theirs
    }

    init?(engine: VoiceEngine, sid: String, initiator: Bool) {
        self.engine = engine
        self.sid = sid
        uid = engine.uidOf?(sid)
        observer = PeerObserver()
        guard let pc = engine.factory.peerConnection(with: engine.configuration(), constraints: Self.constraints, delegate: observer) else { return nil }
        self.pc = pc
        observer.peer = self

        if initiator {
            // Adding the transceiver fires "should negotiate" → we send the offer.
            let transceiverInit = RTCRtpTransceiverInit()
            transceiverInit.direction = .sendRecv
            transceiverInit.streamIds = ["yap"]
            if let track = engine.audioTrack {
                pc.addTransceiver(with: track, init: transceiverInit)
            } else {
                pc.addTransceiver(of: .audio, init: transceiverInit)
            }
        }
    }

    // MARK: Signals

    /// To the far side, or into the outbox while a socket is down.
    func send(_ data: SignalData) {
        guard !closed else { return }
        if engine.online, !held {
            engine.sendSignal(to: sid, data)
        } else {
            outbox.append(data)
        }
    }

    /// After a reconnect or a resume: send what waited, and if an offer of
    /// ours is still unanswered, send it again (the answer may have gone to
    /// a session that's gone).
    func resync() {
        guard !closed, engine.online, !held else { return }
        let queued = outbox
        outbox = []
        for data in queued { engine.sendSignal(to: sid, data) }
        if pc.signalingState == .haveLocalOffer, let local = pc.localDescription {
            let resent = queued.contains { if case .sdp(let d) = $0 { d.sdp == local.sdp } else { false } }
            if !resent { engine.sendSignal(to: sid, .sdp(SessionDescriptionPayload(type: "offer", sdp: local.sdp))) }
        }
    }

    /// How many signals are waiting (tests).
    var waiting: Int { outbox.count }

    // MARK: Negotiation

    func renegotiate() async {
        guard !closed else { return }
        makingOffer = true
        defer { makingOffer = false }
        do {
            let offer = try await pc.offer(for: Self.constraints)
            guard pc.signalingState == .stable, !closed else { return }
            let tuned = RTCSessionDescription(type: .offer, sdp: SDPTuner.tuneOpus(offer.sdp, bitrate: engine.bitrate, redundancy: engine.redundancy))
            try await pc.setLocalDescription(tuned)
            send(.sdp(SessionDescriptionPayload(type: "offer", sdp: pc.localDescription?.sdp ?? tuned.sdp)))
        } catch {
            log.error("offer to \(self.sid, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// Signals are applied strictly in order.
    func handle(_ data: SignalData) {
        let previous = queue
        queue = Task { [weak self] in
            await previous?.value
            await self?.process(data)
        }
    }

    private func process(_ data: SignalData) async {
        guard !closed else { return }
        switch data {
        case .sdp(let description):
            do {
                let isOffer = description.type == "offer"
                let collision = isOffer && (makingOffer || pc.signalingState != .stable)
                ignoreOffer = !polite && collision
                if ignoreOffer { return }
                if collision, pc.signalingState == .haveLocalOffer {
                    // Polite side backs off its own offer.
                    try await pc.setLocalDescription(RTCSessionDescription(type: .rollback, sdp: ""))
                }
                try await pc.setRemoteDescription(RTCSessionDescription(type: isOffer ? .offer : .answer, sdp: description.sdp))
                if isOffer {
                    // A screen they're sending: ask for H.264 first in our answer
                    // (hardware decoding; every Yap client offers it).
                    for transceiver in pc.transceivers where transceiver.mediaType == .video { preferH264(transceiver) }
                    // Answer with our mic on the transceiver they offered.
                    if let transceiver = pc.transceivers.first(where: { $0.mediaType == .audio }) {
                        if transceiver.sender.track == nil, let track = engine.audioTrack { transceiver.sender.track = track }
                        if transceiver.sender.streamIds.isEmpty { transceiver.sender.streamIds = ["yap"] }
                        transceiver.setDirection(.sendRecv, error: nil)
                    }
                    let answer = try await pc.answer(for: Self.constraints)
                    guard !closed else { return }
                    let tuned = RTCSessionDescription(type: .answer, sdp: SDPTuner.tuneOpus(answer.sdp, bitrate: engine.bitrate, redundancy: engine.redundancy))
                    try await pc.setLocalDescription(tuned)
                    send(.sdp(SessionDescriptionPayload(type: "answer", sdp: pc.localDescription?.sdp ?? tuned.sdp)))
                }
                applySenderParameters()
                updateRemoteMedia()
                applyOutput()
            } catch {
                log.error("sdp from \(self.sid, privacy: .public) failed: \(error.localizedDescription, privacy: .public)")
            }
        case .screen:
            return // the engine handles these
        case .candidate(let c):
            guard !c.candidate.isEmpty else { return }
            let candidate = RTCIceCandidate(sdp: c.candidate, sdpMLineIndex: Int32(c.sdpMLineIndex ?? 0), sdpMid: c.sdpMid)
            do {
                try await pc.add(candidate)
            } catch {
                if !ignoreOffer { log.debug("candidate failed: \(error.localizedDescription, privacy: .public)") }
            }
        }
    }

    // MARK: Observer callbacks (already on the main actor)

    func shouldNegotiate() {
        Task { await renegotiate() }
    }

    func generated(_ candidate: IceCandidatePayload) {
        send(.candidate(candidate))
    }

    func connectionStateChanged(_ newState: RTCPeerConnectionState) {
        guard !closed else { return }
        state = VoiceEngine.PeerState(newState)
        if newState == .connected {
            applySenderParameters()
            applyOutput()
            connectedAt = Date()
            lastQuality = nil
            // (Re)connected to someone whose screen we're watching: ask again.
            if engine.watchingScreens.contains(sid) { send(.screen(true)) }
        } else {
            connectedAt = nil
        }
        watchPath()
        engine.peerStateChanged(self)
    }

    func receiverAdded() {
        updateRemoteMedia()
        applyOutput()
    }

    // MARK: When the path dies (PROTOCOL.md › Voice › ICE restarts)

    private func watchPath() {
        recoverTask?.cancel()
        recoverTask = nil
        if let wait = path.stateChanged(state, polite: polite) { scheduleRecovery(after: wait) }
    }

    private func scheduleRecovery(after seconds: TimeInterval) {
        recoverTask?.cancel()
        recoverTask = Task { [weak self] in
            if seconds > 0 { try? await Task.sleep(for: .seconds(seconds)) }
            guard !Task.isCancelled else { return }
            await self?.recover()
        }
    }

    private func recover() async {
        guard !closed, state == .disconnected || state == .failed else { return }
        switch path.attempt(polite: polite, canRebuild: engine.resumable?(sid) ?? false) {
        case .rebuild:
            log.info("rebuilding the connection to \(self.sid, privacy: .public) after \(PathWatch.triesBeforeRebuild) ICE restarts")
            engine.restart(sid)
        case .restartIce:
            // Relay credentials can expire: fresh ICE servers first.
            let servers = await engine.freshIceServers()
            guard !closed, state == .disconnected || state == .failed else { return }
            let config = pc.configuration
            config.iceServers = servers
            if !pc.setConfiguration(config) { log.error("couldn't update the ICE servers for \(self.sid, privacy: .public)") }
            log.info("ICE restart \(self.path.restarts) to \(self.sid, privacy: .public) (\(self.polite ? "polite" : "impolite", privacy: .public))")
            pc.restartIce() // → should negotiate → an offer with new ICE credentials
            engine.peerStateChanged(self)
            engine.peerEvent(self, .iceRestart(path.restarts))
            scheduleRecovery(after: PathWatch.retryEvery)
        }
    }

    // MARK: Screen sharing

    /// Send our screen to this one peer (it asked). The transceiver stays
    /// for the next time; sendrecv, so their own share can ride on it too.
    func addScreen(_ track: RTCVideoTrack) {
        guard !closed else { return }
        let transceiver = screenTransceiver ?? pc.transceivers.first {
            // One of theirs we don't send on yet (they shared to us before): use it, like addTrack.
            $0.mediaType == .video && !$0.isStopped && $0.sender.track == nil
        }
        if let transceiver {
            guard transceiver.sender.track == nil else { return }
            screenTransceiver = transceiver
            preferH264(transceiver)
            transceiver.sender.track = track
            if transceiver.sender.streamIds.isEmpty { transceiver.sender.streamIds = [Self.screenStream] }
            transceiver.setDirection(.sendRecv, error: nil) // → should negotiate → an offer
        } else {
            let transceiverInit = RTCRtpTransceiverInit()
            transceiverInit.direction = .sendRecv
            transceiverInit.streamIds = [Self.screenStream]
            guard let created = pc.addTransceiver(with: track, init: transceiverInit) else { return }
            preferH264(created)
            screenTransceiver = created
        }
        applySenderParameters()
    }

    /// Stop sending our screen to this peer: nothing is encoded for it any more.
    func removeScreen() {
        guard let transceiver = screenTransceiver, transceiver.sender.track != nil else { return }
        transceiver.sender.track = nil
        transceiver.setDirection(.recvOnly, error: nil)
    }

    /// We're sending our screen on this connection.
    var sendsScreen: Bool { screenTransceiver?.sender.track != nil }

    /// The web reads a screen from any video (or later audio) track with a
    /// stream id; ours is this one.
    static let screenStream = "yap-screen"

    /// H.264 constrained baseline first, then the rest (PROTOCOL.md › Screen sharing).
    private func preferH264(_ transceiver: RTCRtpTransceiver) {
        let capabilities = engine.factory.rtpReceiverCapabilities(forKind: kRTCMediaStreamTrackKindVideo).codecs
        guard !capabilities.isEmpty else { return }
        let ordered = ScreenCodecs.ordered(capabilities) { ScreenCodecs.Codec(name: $0.name, parameters: $0.parameters) }
        do {
            try transceiver.setCodecPreferences(ordered, error: ())
        } catch {
            log.debug("codec preferences not applied: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// What they're sending us besides their voice: a screen's video, and its sound.
    func updateRemoteMedia() {
        guard !closed else { return }
        func receiving(_ transceiver: RTCRtpTransceiver) -> Bool {
            var direction = RTCRtpTransceiverDirection.inactive
            guard !transceiver.isStopped, transceiver.currentDirection(&direction) else { return false }
            return direction == .recvOnly || direction == .sendRecv
        }
        let video = pc.transceivers.first { $0.mediaType == .video && receiving($0) }?.receiver.track as? RTCVideoTrack
        let audio = pc.transceivers.filter { $0.mediaType == .audio }
        let sound = audio.dropFirst().contains(where: receiving)
        guard video !== remoteScreen || sound != remoteScreenSound else { return }
        remoteScreen = video
        remoteScreenSound = sound
        engine.remoteMediaChanged(self)
    }

    // MARK: Audio

    /// The voice's transceiver: always the first audio one (a screen's sound comes later).
    private var voiceTransceiver: RTCRtpTransceiver? {
        pc.transceivers.first { $0.mediaType == .audio }
    }

    /// Cap our senders: the voice at the call's bitrate with high priority;
    /// our screen at its quality's bitrate and frame rate.
    func applySenderParameters() {
        if let sender = voiceTransceiver?.sender {
            let parameters = sender.parameters
            if let encoding = parameters.encodings.first {
                encoding.maxBitrateBps = NSNumber(value: engine.bitrate)
                encoding.networkPriority = .high
                encoding.bitratePriority = 4.0
                sender.parameters = parameters
            }
        }
        if let sender = screenTransceiver?.sender, sender.track != nil, let quality = engine.screenQuality {
            let parameters = sender.parameters
            if let encoding = parameters.encodings.first {
                encoding.maxBitrateBps = NSNumber(value: quality.bitrate)
                encoding.maxFramerate = NSNumber(value: quality.fps)
                encoding.scaleResolutionDownBy = 1
                let preference: RTCDegradationPreference = switch quality.degradation {
                case .maintainFramerate: .maintainFramerate
                case .maintainResolution: .maintainResolution
                case .balanced: .balanced
                }
                parameters.degradationPreference = NSNumber(value: preference.rawValue)
                sender.parameters = parameters
            }
        }
    }

    /// Deafen and per-person volume act on the remote track; a boost past
    /// 100 % goes through the limiter. A screen's sound has its own volume (0–100 %).
    func applyOutput() {
        let now = Date()
        let gain = limiter.update(wanted: engine.volume(forSid: sid), level: remoteLevel, seconds: now.timeIntervalSince(limiterAt))
        limiterAt = now
        let voice = voiceTransceiver
        for transceiver in pc.transceivers where transceiver.mediaType == .audio {
            guard let track = transceiver.receiver.track as? RTCAudioTrack else { continue }
            if track.isEnabled == engine.deafened { track.isEnabled = !engine.deafened }
            if transceiver === voice {
                let volume = max(0, min(10, gain))
                if abs(volume - appliedVolume) > 0.005 {
                    track.source.volume = volume
                    appliedVolume = volume
                }
            } else {
                // WebRTC's remote source volume: 0…10, 1 = as sent.
                let volume = max(0, min(1, engine.screenVolume(forSid: sid)))
                if abs(track.source.volume - volume) > 0.005 { track.source.volume = volume }
            }
        }
    }

    /// Our mic's sender on this connection.
    var audioSender: RTCRtpSender? {
        voiceTransceiver?.sender
    }

    /// Latest RTP audio level (0…1) from the remote side, or 0 if packets stopped.
    func remoteAudioLevel(now: Date) -> Double {
        guard let receiver = pc.transceivers.first(where: { $0.mediaType == .audio })?.receiver else { return 0 }
        let sources = receiver.sources
        guard let source = sources.first(where: { $0.sourceType == .SSRC }) ?? sources.first else { return 0 }
        if source.timestampUs != lastLevelTimestamp {
            lastLevelTimestamp = source.timestampUs
            lastLevelSeenAt = now
        }
        remoteLevel = now.timeIntervalSince(lastLevelSeenAt) > 0.4 ? 0 : source.audioLevel?.doubleValue ?? 0
        return remoteLevel
    }

    /// How many RTP sources the receiver hears (the debug log).
    var sourceCount: Int { pc.transceivers.first(where: { $0.mediaType == .audio })?.receiver.sources.count ?? -1 }

    /// "weak connection": judged from two stats readings ~2 s apart. Returns
    /// whether it changed.
    func judgeQuality(_ stats: PeerStats, at now: Date) -> Bool {
        guard state == .connected, let connectedAt else {
            lastQuality = nil
            let was = quality.weak != nil
            quality.reset()
            return was
        }
        defer { lastQuality = (now, stats) }
        guard let last = lastQuality else { return false }
        return quality.update(LinkQuality.Reading(
            seconds: now.timeIntervalSince(last.at),
            packets: stats.packetsReceived - last.stats.packetsReceived,
            samples: stats.samplesReceived - last.stats.samplesReceived,
            concealed: (stats.concealedSamples - stats.silentConcealedSamples) - (last.stats.concealedSamples - last.stats.silentConcealedSamples),
            rttMs: stats.rttMs,
            connectedFor: now.timeIntervalSince(connectedAt)))
    }

    /// This peer's `getStats()`, boiled down (no addresses: `PeerStats.read`).
    func stats() async -> PeerStats {
        let report = await pc.statistics()
        let voiceMid = pc.transceivers.first { $0.mediaType == .audio }?.mid
        var stats = PeerStats.read(report.statistics.mapValues { StatsRow(type: $0.type, values: $0.values) }, sid: sid, voiceMid: voiceMid)
        // libwebrtc sends with the first codec of the far side's description,
        // and the far side (if it's libwebrtc too) with the first of ours.
        stats.sendsRed = SDPTuner.audioCodecOrder(in: pc.remoteDescription?.sdp ?? "").first?.first?.lowercased() == "red"
        stats.asksRed = SDPTuner.audioCodecOrder(in: pc.localDescription?.sdp ?? "").first?.first?.lowercased() == "red"
        stats.opusFmtpAsked = pc.localDescription.map { SDPTuner.opusFmtpLine(in: $0.sdp) } ?? nil
        return stats
    }

    func close() {
        closed = true
        remoteScreen = nil
        recoverTask?.cancel()
        recoverTask = nil
        queue?.cancel()
        observer.peer = nil
        pc.close()
    }
}

/// WebRTC calls its delegate on its own signaling thread. Under Swift 6 a
/// main-actor delegate would trap at runtime, so this one is nonisolated and
/// hops to the main queue in order (DispatchQueue keeps FIFO; separate
/// `Task`s wouldn't), turning WebRTC objects into plain values first.
final class PeerObserver: NSObject, RTCPeerConnectionDelegate, @unchecked Sendable {
    /// Read and written on the main thread only.
    nonisolated(unsafe) weak var peer: Peer?

    private func hop(_ body: @escaping @Sendable @MainActor (Peer) -> Void) {
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let peer = self?.peer, !peer.closed else { return }
                body(peer)
            }
        }
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {}

    func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {
        hop { $0.shouldNegotiate() }
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {
        let payload = IceCandidatePayload(candidate: candidate.sdp, sdpMid: candidate.sdpMid, sdpMLineIndex: Int(candidate.sdpMLineIndex))
        hop { $0.generated(payload) }
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCPeerConnectionState) {
        let state = newState
        hop { $0.connectionStateChanged(state) }
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didAdd rtpReceiver: RTCRtpReceiver, streams mediaStreams: [RTCMediaStream]) {
        hop { $0.receiverAdded() }
    }
}
