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
///
/// And it puts redundancy (RED, RFC 2198: `red/48000/2`, fmtp
/// `<opus>/<opus>`) first in the voice section, Opus right after, so each
/// voice packet also carries the frame before it (PROTOCOL.md › Voice ›
/// Redundancy). libwebrtc sends with the first codec of the *remote*
/// description, so this asks the far side for RED; it packs and unpacks the
/// copies itself. A screen share's audio stays plain Opus (Opus before RED).
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
    /// music mode; `redundancy`: RED first, or no RED at all when off); any
    /// later one is a screen share's audio (PROTOCOL.md › Screen sharing):
    /// at least 160 kbps, stereo where we're the one sending it, plain Opus.
    public static func tuneOpus(_ sdp: String, bitrate: Int, sendStereo: Bool = false, redundancy: Bool = true) -> String {
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
                section = voice ? (redundancy ? redFirst(section) : withoutRed(section)) : opusBeforeRed(section)
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

    // MARK: Redundancy (RED)

    /// The voice: `red/48000/2` first in the m= line, Opus right after, the
    /// rest as they were; the RED fmtp says one earlier Opus frame
    /// (`<opus>/<opus>`). A section without RED (the far side's offer had
    /// none) stays as it is: RED is never required.
    static func redFirst(_ lines: [String]) -> [String] {
        guard let opus = lines.lazy.compactMap(opusPayloadType).first,
              let red = lines.lazy.compactMap(redPayloadType).first,
              let m = lines.firstIndex(where: { $0.hasPrefix("m=audio") }) else { return lines }
        var lines = lines
        lines[m] = reordered(lines[m]) { formats in [red, opus] + formats.filter { $0 != red && $0 != opus } }
        let fmtp = "a=fmtp:\(red) \(opus)/\(opus)"
        if let i = lines.firstIndex(where: { $0.hasPrefix("a=fmtp:\(red) ") }) {
            lines[i] = fmtp
        } else if let r = lines.firstIndex(where: { redPayloadType($0) == red }) {
            lines.insert(fmtp, at: r + 1)
        }
        return lines
    }

    /// The voice without RED at all (redundancy off: measurements, tests).
    static func withoutRed(_ lines: [String]) -> [String] {
        guard let red = lines.lazy.compactMap(redPayloadType).first,
              let m = lines.firstIndex(where: { $0.hasPrefix("m=audio") }) else { return lines }
        var lines = lines
        lines[m] = reordered(lines[m]) { $0.filter { $0 != red } }
        return lines.filter { line in
            !["a=rtpmap:", "a=fmtp:", "a=rtcp-fb:"].contains { line.hasPrefix("\($0)\(red) ") }
        }
    }

    /// A screen share's audio stays plain Opus: if RED comes before Opus in
    /// the m= line, Opus moves in front of it.
    static func opusBeforeRed(_ lines: [String]) -> [String] {
        guard let opus = lines.lazy.compactMap(opusPayloadType).first,
              let red = lines.lazy.compactMap(redPayloadType).first,
              let m = lines.firstIndex(where: { $0.hasPrefix("m=audio") }) else { return lines }
        var lines = lines
        lines[m] = reordered(lines[m]) { formats in
            guard let r = formats.firstIndex(of: red), let o = formats.firstIndex(of: opus), r < o else { return formats }
            var formats = formats
            formats.remove(at: o)
            formats.insert(opus, at: r)
            return formats
        }
        return lines
    }

    /// `m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9` with its payload types rearranged.
    private static func reordered(_ mLine: String, _ arrange: ([String]) -> [String]) -> String {
        let parts = mLine.split(separator: " ", omittingEmptySubsequences: false).map(String.init)
        guard parts.count > 3 else { return mLine }
        return (parts.prefix(3) + arrange(Array(parts.dropFirst(3)))).joined(separator: " ")
    }

    /// `a=rtpmap:63 red/48000/2` → "63"
    static func redPayloadType(_ line: String) -> String? {
        let lower = line.lowercased()
        guard lower.hasPrefix("a=rtpmap:"), lower.contains(" red/48000") else { return nil }
        let rest = line.dropFirst("a=rtpmap:".count)
        guard let space = rest.firstIndex(of: " ") else { return nil }
        let pt = String(rest[..<space])
        return pt.allSatisfy(\.isNumber) && !pt.isEmpty ? pt : nil
    }

    /// The codecs of each audio section, in m= line order, by name
    /// (`[["red", "opus", "G722", …], ["opus", …]]`; tests and diagnostics).
    public static func audioCodecOrder(in sdp: String) -> [[String]] {
        let lines = sdp.components(separatedBy: sdp.contains("\r\n") ? "\r\n" : "\n")
        var out: [[String]] = []
        var formats: [String]?
        var names: [String: String] = [:]
        func flush() {
            if let formats { out.append(formats.map { names[$0] ?? $0 }) }
            formats = nil
            names = [:]
        }
        for line in lines {
            if line.hasPrefix("m=") {
                flush()
                if line.hasPrefix("m=audio") { formats = Array(line.split(separator: " ").dropFirst(3).map(String.init)) }
            } else if formats != nil, line.hasPrefix("a=rtpmap:") {
                let rest = line.dropFirst("a=rtpmap:".count).split(separator: " ", maxSplits: 1)
                if rest.count == 2 { names[String(rest[0])] = String(rest[1].split(separator: "/").first ?? "") }
            }
        }
        flush()
        return out
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

    /// The voice's Opus fmtp, as written (`minptime=10;useinbandfec=1;…`; the call report).
    public static func opusFmtpLine(in sdp: String) -> String? {
        let lines = sdp.components(separatedBy: sdp.contains("\r\n") ? "\r\n" : "\n")
        guard let pt = lines.lazy.compactMap(opusPayloadType).first,
              let line = lines.first(where: { $0.hasPrefix("a=fmtp:\(pt) ") }) else { return nil }
        return String(line.dropFirst("a=fmtp:\(pt) ".count))
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
