import Foundation
import os
@preconcurrency import WebRTC

/// Two peer connections joined to each other inside the app (nothing
/// leaves the device: no ICE servers, candidates are handed across in
/// memory), in one of two jobs:
///
/// - **The mic level while you're alone** (`track`: the mic). WebRTC only
///   captures the mic while something is sending it, so with nobody to call
///   the meter and voice activity had nothing to read. This sends the mic
///   from one side to the other; the receiving side plays nothing. The
///   sender's stats then carry the level, measured the same way as in a
///   call. The engine closes it the moment a real call starts.
/// - **The call's anchor on iPhone** (`track` nil): nothing is sent, but the
///   receiving side holds an audio stream for the whole call. WebRTC stops
///   the voice-processing unit and deactivates the audio session the moment
///   its last audio stream goes, which happens for a split second every time
///   the people in the call change (alone → someone joins, the last one
///   leaves, a connection rebuilt). In the background iOS then pauses Yap
///   and won't let it start the mic again, so the call dropped. With the
///   anchor there's always a stream, so the sound never stops mid-call.
@MainActor
final class LevelMonitor {
    let sender: RTCPeerConnection
    let receiver: RTCPeerConnection
    private let senderObserver = LoopObserver()
    private let receiverObserver = LoopObserver()
    private(set) var closed = false
    private let log = Logger(subsystem: Bundle.main.bundleIdentifier ?? "yap", category: "voice")

    private static let constraints = RTCMediaConstraints(mandatoryConstraints: nil, optionalConstraints: nil)

    /// `track` nil: the anchor (a receive-only audio stream, nothing encoded).
    init?(factory: RTCPeerConnectionFactory, track: RTCAudioTrack?) {
        let config = RTCConfiguration()
        config.iceServers = []
        config.sdpSemantics = .unifiedPlan
        config.bundlePolicy = .maxBundle
        config.rtcpMuxPolicy = .require
        guard let sender = factory.peerConnection(with: config, constraints: Self.constraints, delegate: senderObserver),
              let receiver = factory.peerConnection(with: config, constraints: Self.constraints, delegate: receiverObserver) else { return nil }
        self.sender = sender
        self.receiver = receiver
        senderObserver.onCandidate = { [weak self] c in self?.add(c, to: self?.receiver) }
        receiverObserver.onCandidate = { [weak self] c in self?.add(c, to: self?.sender) }
        receiverObserver.onTrack = { [weak self] in self?.silenceReceiver() }

        let send = RTCRtpTransceiverInit()
        send.direction = .sendOnly
        send.streamIds = ["yap-monitor"]
        if let track {
            sender.addTransceiver(with: track, init: send)
        } else {
            // No track: the offer still announces a stream, so the other
            // side opens an audio receive stream (that's the anchor).
            sender.addTransceiver(of: .audio, init: send)
        }
    }

    /// The sender's audio sender (whose stats carry the level).
    var audioSender: RTCRtpSender? {
        sender.transceivers.first { $0.mediaType == .audio }?.sender
    }

    var isConnected: Bool { sender.connectionState == .connected }

    /// A real call can start (and close this) during any of the awaits, so
    /// each step checks first; a step refused because it closed meanwhile
    /// is that, not a failure.
    func start() async {
        do {
            let offer = try await sender.offer(for: Self.constraints)
            guard !closed else { return }
            try await sender.setLocalDescription(offer)
            guard !closed else { return }
            try await receiver.setRemoteDescription(offer)
            guard !closed else { return }
            let answer = try await receiver.answer(for: Self.constraints)
            guard !closed else { return }
            try await receiver.setLocalDescription(answer)
            guard !closed else { return }
            try await sender.setRemoteDescription(answer)
            guard !closed else { return }
            silenceReceiver()
        } catch {
            guard !closed else { return }
            log.error("level monitor didn't start: \(error.localizedDescription, privacy: .public)")
        }
    }

    private func add(_ candidate: RTCIceCandidate, to pc: RTCPeerConnection?) {
        guard !closed, let pc else { return }
        Task { try? await pc.add(candidate) }
    }

    /// Nothing from the loop reaches the speaker.
    private func silenceReceiver() {
        for transceiver in receiver.transceivers where transceiver.mediaType == .audio {
            guard let track = transceiver.receiver.track as? RTCAudioTrack else { continue }
            track.isEnabled = false
            track.source.volume = 0
        }
    }

    func close() {
        guard !closed else { return }
        closed = true
        senderObserver.onCandidate = nil
        receiverObserver.onCandidate = nil
        receiverObserver.onTrack = nil
        sender.close()
        receiver.close()
    }
}

/// The loop's delegate: WebRTC calls it on its signaling thread, so it hops
/// to the main queue (like `PeerObserver`).
final class LoopObserver: NSObject, RTCPeerConnectionDelegate, @unchecked Sendable {
    /// Read and written on the main thread only.
    nonisolated(unsafe) var onCandidate: (@MainActor (RTCIceCandidate) -> Void)?
    nonisolated(unsafe) var onTrack: (@MainActor () -> Void)?

    func peerConnection(_ peerConnection: RTCPeerConnection, didGenerate candidate: RTCIceCandidate) {
        let copy = RTCIceCandidate(sdp: candidate.sdp, sdpMLineIndex: candidate.sdpMLineIndex, sdpMid: candidate.sdpMid)
        nonisolated(unsafe) let sendable = copy
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.onCandidate?(sendable) }
        }
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didAdd rtpReceiver: RTCRtpReceiver, streams mediaStreams: [RTCMediaStream]) {
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated { self?.onTrack?() }
        }
    }

    func peerConnection(_ peerConnection: RTCPeerConnection, didChange stateChanged: RTCSignalingState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didAdd stream: RTCMediaStream) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove stream: RTCMediaStream) {}
    func peerConnectionShouldNegotiate(_ peerConnection: RTCPeerConnection) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceConnectionState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didChange newState: RTCIceGatheringState) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didRemove candidates: [RTCIceCandidate]) {}
    func peerConnection(_ peerConnection: RTCPeerConnection, didOpen dataChannel: RTCDataChannel) {}
}
