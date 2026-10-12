import Foundation

/// One entry of WebRTC's stats report as plain values (`RTCStatistics`'s
/// type and values; tests build them by hand).
struct StatsRow {
    var type: String
    var values: [String: NSObject]
}

/// What `getStats()` says about one peer (also what the interop test and
/// the call report read). Never an address: candidates give their *type*
/// only (host, srflx, prflx, relay).
public struct PeerStats: Sendable, Equatable {
    public var sid: String
    public var rttMs: Double?
    public var bytesSent: Double = 0
    public var bytesReceived: Double = 0
    public var packetsSent = 0
    public var packetsReceived = 0
    public var packetsLost = 0
    /// The voice's received samples, and how many were patched over (concealed).
    public var samplesReceived: Double = 0
    public var concealedSamples: Double = 0
    public var silentConcealedSamples: Double = 0
    public var inboundAudioLevel: Double = 0
    public var localAudioLevel: Double = 0
    public var inboundCodec: String?
    public var inboundFmtp: String?
    public var outboundCodec: String?
    public var outboundFmtp: String?
    /// A screen we watch: frames decoded, its size and codec.
    public var videoFramesDecoded = 0
    public var videoBytesReceived: Double = 0
    public var videoFrameWidth = 0
    public var videoFrameHeight = 0
    public var inboundVideoCodec: String?
    public var inboundVideoFmtp: String?
    /// Our screen to them.
    public var videoFramesEncoded = 0
    public var videoBytesSent: Double = 0
    public var outboundVideoCodec: String?
    public var outboundVideoFmtp: String?
    public var videoEncoder: String?
    /// A screen's sound (any audio after the voice).
    public var screenAudioBytesReceived: Double = 0

    // The voice alone (the first audio section), for the call report.
    public var voiceBytesReceived: Double = 0
    public var voicePacketsReceived = 0
    public var voicePacketsLost = 0
    /// Packets the jitter buffer threw away. With RED that's every copy of
    /// a frame whose own packet already came: the copies are arriving.
    public var voicePacketsDiscarded = 0
    /// libwebrtc's "secondary" packets (Opus FEC, and RED copies where it counts them) received / not needed.
    public var fecPacketsReceived = 0
    public var fecPacketsDiscarded = 0
    /// Σ seconds each played sample waited in the jitter buffer, and how many samples.
    public var jitterBufferDelay: Double = 0
    public var jitterBufferEmitted: Double = 0
    /// The candidate pair in use, by type only.
    public var localCandidateType: String?
    public var remoteCandidateType: String?
    /// RED first in the far side's description (so we send RED) / in ours (we ask for it).
    public var sendsRed = false
    public var asksRed = false
    /// The Opus fmtp in our own description: what we asked the far side to send.
    public var opusFmtpAsked: String?

    // What they report receiving from us (`remote-inbound-rtp` about our
    // voice, from their RTCP reports; PROTOCOL.md › Voice › Connection hints ›
    // Your own connection): the share lost in their latest report (0…1), how
    // many of ours they've lost so far, the round trip measured with them,
    // and how many such measurements (nil/0 until their first report).
    public var remoteFractionLost: Double?
    public var remotePacketsLost = 0
    public var remoteRttMs: Double?
    public var remoteReports = 0

    public init(sid: String) {
        self.sid = sid
    }

    /// Boils a stats report down. Reads named counters only — never a
    /// candidate's address, port, related address or server URL.
    static func read(_ rows: [String: StatsRow], sid: String, voiceMid: String?) -> PeerStats {
        var stats = PeerStats(sid: sid)
        var inboundCodecId: String?
        var outboundCodecId: String?
        var inboundVideoCodecId: String?
        var outboundVideoCodecId: String?
        var sawInbound = false
        var transportPair: String?
        var nominatedPair: String?
        func number(_ v: [String: NSObject], _ key: String) -> NSNumber? { v[key] as? NSNumber }
        for (id, s) in rows {
            let v = s.values
            switch s.type {
            case "transport":
                transportPair = v["selectedCandidatePairId"] as? String ?? transportPair
            case "candidate-pair":
                if number(v, "nominated")?.boolValue == true, v["state"] as? String == "succeeded" {
                    if let rtt = number(v, "currentRoundTripTime") { stats.rttMs = rtt.doubleValue * 1000 }
                    nominatedPair = id
                }
            case "inbound-rtp" where v["kind"] as? String == "video":
                stats.videoFramesDecoded += number(v, "framesDecoded")?.intValue ?? 0
                stats.videoBytesReceived += number(v, "bytesReceived")?.doubleValue ?? 0
                if let width = number(v, "frameWidth")?.intValue, width > 0 { stats.videoFrameWidth = width }
                if let height = number(v, "frameHeight")?.intValue, height > 0 { stats.videoFrameHeight = height }
                inboundVideoCodecId = v["codecId"] as? String ?? inboundVideoCodecId
            case "outbound-rtp" where v["kind"] as? String == "video":
                stats.videoFramesEncoded += number(v, "framesEncoded")?.intValue ?? 0
                stats.videoBytesSent += number(v, "bytesSent")?.doubleValue ?? 0
                outboundVideoCodecId = v["codecId"] as? String ?? outboundVideoCodecId
                if let encoder = v["encoderImplementation"] as? String { stats.videoEncoder = encoder }
            case "outbound-rtp" where v["kind"] as? String == "audio":
                stats.bytesSent += number(v, "bytesSent")?.doubleValue ?? 0
                stats.packetsSent += number(v, "packetsSent")?.intValue ?? 0
                outboundCodecId = v["codecId"] as? String
            case "inbound-rtp" where v["kind"] as? String == "audio":
                stats.bytesReceived += number(v, "bytesReceived")?.doubleValue ?? 0
                stats.packetsReceived += number(v, "packetsReceived")?.intValue ?? 0
                stats.packetsLost += number(v, "packetsLost")?.intValue ?? 0
                if let mid = v["mid"] as? String, let voiceMid, mid != voiceMid {
                    stats.screenAudioBytesReceived += number(v, "bytesReceived")?.doubleValue ?? 0
                }
                // The voice's stream (the first audio section).
                if !sawInbound || v["mid"] as? String == voiceMid {
                    sawInbound = true
                    stats.samplesReceived = number(v, "totalSamplesReceived")?.doubleValue ?? 0
                    stats.concealedSamples = number(v, "concealedSamples")?.doubleValue ?? 0
                    stats.silentConcealedSamples = number(v, "silentConcealedSamples")?.doubleValue ?? 0
                    stats.voiceBytesReceived = number(v, "bytesReceived")?.doubleValue ?? 0
                    stats.voicePacketsReceived = number(v, "packetsReceived")?.intValue ?? 0
                    stats.voicePacketsLost = number(v, "packetsLost")?.intValue ?? 0
                    stats.voicePacketsDiscarded = number(v, "packetsDiscarded")?.intValue ?? 0
                    stats.fecPacketsReceived = number(v, "fecPacketsReceived")?.intValue ?? 0
                    stats.fecPacketsDiscarded = number(v, "fecPacketsDiscarded")?.intValue ?? 0
                    stats.jitterBufferDelay = number(v, "jitterBufferDelay")?.doubleValue ?? 0
                    stats.jitterBufferEmitted = number(v, "jitterBufferEmittedCount")?.doubleValue ?? 0
                }
                stats.inboundAudioLevel = number(v, "audioLevel")?.doubleValue ?? stats.inboundAudioLevel
                inboundCodecId = v["codecId"] as? String
            case "media-source" where v["kind"] as? String == "audio":
                stats.localAudioLevel = number(v, "audioLevel")?.doubleValue ?? 0
            default:
                break
            }
        }
        // Their report about our voice: the remote-inbound-rtp whose outbound
        // stream is the voice's (the first audio section), else the only one.
        let remote = rows.values.filter { $0.type == "remote-inbound-rtp" && $0.values["kind"] as? String == "audio" }
        let voiceReport = remote.first { row in
            guard let voiceMid, let local = (row.values["localId"] as? String).flatMap({ rows[$0] }) else { return false }
            return local.values["mid"] as? String == voiceMid
        } ?? (remote.count == 1 ? remote.first : nil)
        if let v = voiceReport?.values {
            stats.remoteFractionLost = number(v, "fractionLost")?.doubleValue
            stats.remotePacketsLost = max(0, number(v, "packetsLost")?.intValue ?? 0)
            stats.remoteRttMs = number(v, "roundTripTime").map { $0.doubleValue * 1000 }
            stats.remoteReports = number(v, "roundTripTimeMeasurements")?.intValue ?? 0
        }
        if let pair = (transportPair ?? nominatedPair).flatMap({ rows[$0] }) {
            if let rtt = pair.values["currentRoundTripTime"] as? NSNumber { stats.rttMs = rtt.doubleValue * 1000 }
            stats.localCandidateType = (pair.values["localCandidateId"] as? String).flatMap { rows[$0]?.values["candidateType"] as? String }
            stats.remoteCandidateType = (pair.values["remoteCandidateId"] as? String).flatMap { rows[$0]?.values["candidateType"] as? String }
        }
        func codec(_ id: String?) -> (String?, String?) {
            guard let id, let row = rows[id] else { return (nil, nil) }
            return (row.values["mimeType"] as? String, row.values["sdpFmtpLine"] as? String)
        }
        (stats.inboundCodec, stats.inboundFmtp) = codec(inboundCodecId)
        (stats.outboundCodec, stats.outboundFmtp) = codec(outboundCodecId)
        (stats.inboundVideoCodec, stats.inboundVideoFmtp) = codec(inboundVideoCodecId)
        (stats.outboundVideoCodec, stats.outboundVideoFmtp) = codec(outboundVideoCodecId)
        return stats
    }
}
