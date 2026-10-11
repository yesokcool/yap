import Foundation

/// The web client's `tuneOpus()` (web/js/voice.js), line for line.
///
/// Rewrites the Opus `a=fmtp` line of every audio section of a local
/// description before it's set.
/// These parameters describe what *we* want to receive, so each side tunes
/// its own description:
///  - maxaveragebitrate: our quality setting (a call runs at the lower of the
///    two people's settings; the sender also caps itself via maxBitrate)
///  - useinbandfec=1: forward error correction, survives packet loss
///  - usedtx=0: keep sending during silence (no clipped first syllables)
///  - stereo=1: we can play stereo; sprop-stereo: whether *we* send stereo
///  - minptime=10, maxplaybackrate=48000: low latency, full band
///
/// It also drops `transport-cc` from audio sections (video keeps it): with
/// transport-wide feedback on audio, WebRTC's bandwidth estimator probes
/// with padding packets — ~18 kbps per direction per peer on a clean line,
/// hundreds of kbps on a lossy one — and voice doesn't need the estimate.
public enum SDPTuner {
    public static func opusParameters(bitrate: Int, sendStereo: Bool) -> [(String, String)] {
        [
            ("minptime", "10"),
            ("useinbandfec", "1"),
            ("usedtx", "0"),
            ("stereo", "1"),
            ("sprop-stereo", sendStereo ? "1" : "0"),
            ("maxaveragebitrate", String(bitrate)),
            ("maxplaybackrate", "48000"),
            ("sprop-maxcapturerate", "48000"),
        ]
    }

    /// Screen-share audio (a later audio section) is music: at least this rich.
    public static let screenAudioBitrate = 160_000

    /// Tunes every audio section: the first is the voice (`sendStereo` =
    /// music mode); any later one is a screen share's audio
    /// (PROTOCOL.md › Screen sharing): at least 160 kbps, and stereo where
    /// we're the one sending it.
    public static func tuneOpus(_ sdp: String, bitrate: Int, sendStereo: Bool = false) -> String {
        let newline = sdp.contains("\r\n") ? "\r\n" : "\n"
        let lines = withoutAudioTransportCC(sdp.components(separatedBy: newline))
        var out: [String] = []
        var section: [String] = []
        var audioSections = 0
        func flush() {
            guard let first = section.first else { return }
            if first.hasPrefix("m=audio") {
                let voice = audioSections == 0
                audioSections += 1
                let sending = section.contains("a=sendrecv") || section.contains("a=sendonly")
                section = tuneAudioSection(section, want: opusParameters(bitrate: voice ? bitrate : max(bitrate, screenAudioBitrate),
                                                                         sendStereo: voice ? sendStereo : sending))
            }
            out += section
            section = []
        }
        for line in lines {
            if line.hasPrefix("m=") {
                flush()
                section = [line]
            } else if section.isEmpty {
                out.append(line)
            } else {
                section.append(line)
            }
        }
        flush()
        return out.joined(separator: newline)
    }

    /// One audio section's Opus `a=fmtp`: existing params kept (in order),
    /// ours overridden, the rest appended; added after the rtpmap if missing.
    static func tuneAudioSection(_ lines: [String], want: [(String, String)]) -> [String] {
        guard let pt = lines.lazy.compactMap(opusPayloadType).first else { return lines }
        var lines = lines
        let fmtpPrefix = "a=fmtp:\(pt) "
        if let i = lines.firstIndex(where: { $0.hasPrefix(fmtpPrefix) }) {
            var params: [(String, String)] = []
            for pair in lines[i].dropFirst(fmtpPrefix.count).split(separator: ";") {
                let kv = pair.trimmingCharacters(in: .whitespaces).split(separator: "=", maxSplits: 1).map(String.init)
                guard let key = kv.first, !key.isEmpty else { continue }
                params.append((key, kv.count > 1 ? kv[1] : ""))
            }
            for (key, value) in want {
                if let j = params.firstIndex(where: { $0.0 == key }) { params[j].1 = value } else { params.append((key, value)) }
            }
            lines[i] = fmtpPrefix + params.map { "\($0.0)=\($0.1)" }.joined(separator: ";")
        } else if let r = lines.firstIndex(where: { $0.lowercased().hasPrefix("a=rtpmap:\(pt) opus/48000") }) {
            lines.insert(fmtpPrefix + want.map { "\($0.0)=\($0.1)" }.joined(separator: ";"), at: r + 1)
        }
        return lines
    }

    /// The DTLS fingerprint in an SDP (`a=fingerprint:sha-256 AB:CD:…`): a
    /// different one in a new offer means the far side started a brand-new
    /// connection (PROTOCOL.md › Keeping calls across reconnects › fresh start).
    public static func fingerprint(in sdp: String) -> String? {
        for line in sdp.split(whereSeparator: \.isNewline) where line.hasPrefix("a=fingerprint:") {
            let parts = line.split(separator: " ")
            if parts.count >= 2 { return String(parts[1]) }
        }
        return nil
    }

    static let transportCCExtension = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01"

    /// Audio sections without `a=rtcp-fb:<pt> transport-cc` and the
    /// transport-wide-cc header extension; other sections untouched.
    static func withoutAudioTransportCC(_ lines: [String]) -> [String] {
        var inAudio = false
        return lines.filter { line in
            if line.hasPrefix("m=") {
                inAudio = line.hasPrefix("m=audio")
                return true
            }
            guard inAudio else { return true }
            if line.hasPrefix("a=rtcp-fb:"), line.hasSuffix(" transport-cc") { return false }
            if line.hasPrefix("a=extmap:"), line.hasSuffix(" " + transportCCExtension) { return false }
            return true
        }
    }

    /// `a=rtpmap:111 opus/48000/2` → "111"
    static func opusPayloadType(_ line: String) -> String? {
        let lower = line.lowercased()
        guard lower.hasPrefix("a=rtpmap:"), lower.contains(" opus/48000") else { return nil }
        let rest = line.dropFirst("a=rtpmap:".count)
        guard let space = rest.firstIndex(of: " ") else { return nil }
        let pt = String(rest[..<space])
        return pt.allSatisfy(\.isNumber) && !pt.isEmpty ? pt : nil
    }

    /// The Opus fmtp parameters in an SDP (for tests and diagnostics).
    public static func opusFmtp(in sdp: String) -> [String: String] {
        let newline = sdp.contains("\r\n") ? "\r\n" : "\n"
        let lines = sdp.components(separatedBy: newline)
        guard let pt = lines.lazy.compactMap(opusPayloadType).first,
              let line = lines.first(where: { $0.hasPrefix("a=fmtp:\(pt) ") }) else { return [:] }
        var out: [String: String] = [:]
        for pair in line.dropFirst("a=fmtp:\(pt) ".count).split(separator: ";") {
            let kv = pair.split(separator: "=", maxSplits: 1).map(String.init)
            if let key = kv.first { out[key] = kv.count > 1 ? kv[1] : "" }
        }
        return out
    }
}
