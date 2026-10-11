import Testing
@testable import YapVoice

@Suite("Opus SDP tuning")
struct SDPTunerTests {
    let chromeOffer = [
        "v=0",
        "o=- 1 2 IN IP4 127.0.0.1",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126",
        "a=rtpmap:111 opus/48000/2",
        "a=rtcp-fb:111 transport-cc",
        "a=fmtp:111 minptime=10;useinbandfec=1",
        "a=rtpmap:63 red/48000/2",
        "a=fmtp:63 111/111",
        "",
    ].joined(separator: "\r\n")

    @Test func mergesIntoExistingFmtp() {
        let tuned = SDPTuner.tuneOpus(chromeOffer, bitrate: 128_000)
        #expect(tuned.contains("a=fmtp:111 minptime=10;useinbandfec=1;usedtx=0;stereo=1;sprop-stereo=0;maxaveragebitrate=128000;maxplaybackrate=48000;sprop-maxcapturerate=48000\r\n"))
        #expect(tuned.contains("a=fmtp:63 111/111"), "other codecs untouched")
        #expect(tuned.hasSuffix("\r\n"))
        let params = SDPTuner.opusFmtp(in: tuned)
        #expect(params["maxaveragebitrate"] == "128000")
        #expect(params["usedtx"] == "0")
    }

    @Test func addsFmtpWhenMissingAndOverridesValues() {
        let noFmtp = chromeOffer.replacingOccurrences(of: "a=fmtp:111 minptime=10;useinbandfec=1\r\n", with: "")
        let tuned = SDPTuner.tuneOpus(noFmtp, bitrate: 510_000, sendStereo: true)
        #expect(tuned.contains("a=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10;useinbandfec=1;usedtx=0;stereo=1;sprop-stereo=1;maxaveragebitrate=510000"))
        let overridden = SDPTuner.tuneOpus(chromeOffer.replacingOccurrences(of: "useinbandfec=1", with: "useinbandfec=0;usedtx=1"), bitrate: 64_000)
        let params = SDPTuner.opusFmtp(in: overridden)
        #expect(params["useinbandfec"] == "1")
        #expect(params["usedtx"] == "0")
        #expect(params["maxaveragebitrate"] == "64000")
    }

    @Test func leavesSdpWithoutOpusAlone() {
        let sdp = "v=0\r\nm=audio 9 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n"
        #expect(SDPTuner.tuneOpus(sdp, bitrate: 128_000) == sdp)
    }

    /// A screen share adds audio sections after the voice (PROTOCOL.md ›
    /// Opus tuning): each one is tuned, as music.
    @Test func everyAudioSectionIsTuned() {
        let sdp = [
            "v=0",
            "a=group:BUNDLE 0 1 2",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111",
            "a=mid:0",
            "a=sendrecv",
            "a=rtpmap:111 opus/48000/2",
            "a=fmtp:111 minptime=10;useinbandfec=1",
            "m=video 9 UDP/TLS/RTP/SAVPF 96",
            "a=mid:1",
            "a=rtpmap:96 VP8/90000",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111",
            "a=mid:2",
            "a=recvonly",
            "a=rtpmap:111 opus/48000/2",
            "a=fmtp:111 minptime=10;useinbandfec=1",
            "m=audio 9 UDP/TLS/RTP/SAVPF 109",
            "a=mid:3",
            "a=sendonly",
            "a=rtpmap:109 opus/48000/2",
            "",
        ].joined(separator: "\r\n")
        let sections = SDPTuner.tuneOpus(sdp, bitrate: 96_000).components(separatedBy: "\r\nm=")
        #expect(sections.count == 5)
        #expect(sections[1].contains("a=fmtp:111 minptime=10;useinbandfec=1;usedtx=0;stereo=1;sprop-stereo=0;maxaveragebitrate=96000;"), "the voice: my quality, mono")
        #expect(!sections[2].contains("fmtp"), "video untouched")
        #expect(sections[3].contains("sprop-stereo=0;maxaveragebitrate=160000;"), "someone's screen audio I receive: at least 160 kbps")
        #expect(sections[4].contains("a=rtpmap:109 opus/48000/2\r\na=fmtp:109 minptime=10;useinbandfec=1;usedtx=0;stereo=1;sprop-stereo=1;maxaveragebitrate=160000;"), "screen audio I send: stereo")
        #expect(SDPTuner.tuneOpus(sdp, bitrate: 256_000).contains("maxaveragebitrate=256000;maxplaybackrate=48000;sprop-maxcapturerate=48000\r\nm=video"))
    }

    @Test func bitrateChoices() {
        #expect(VoiceSettings.bitrateChoices == [64_000, 96_000, 128_000, 256_000, 510_000])
        #expect(VoiceSettings().bitrate == 128_000)
    }
}

@Suite("No transport-cc on audio")
struct TransportCCTests {
    let offer = [
        "v=0",
        "o=- 1 2 IN IP4 127.0.0.1",
        "a=group:BUNDLE 0 1",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111 63",
        "a=mid:0",
        "a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level",
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
        "a=rtpmap:111 opus/48000/2",
        "a=rtcp-fb:111 transport-cc",
        "a=fmtp:111 minptime=10;useinbandfec=1",
        "a=rtpmap:63 red/48000/2",
        "m=video 9 UDP/TLS/RTP/SAVPF 96",
        "a=mid:1",
        "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
        "a=rtpmap:96 VP8/90000",
        "a=rtcp-fb:96 transport-cc",
        "a=rtcp-fb:96 nack",
        "",
    ].joined(separator: "\r\n")

    @Test func strippedFromAudioKeptOnVideo() {
        let tuned = SDPTuner.tuneOpus(offer, bitrate: 128_000)
        let sections = tuned.components(separatedBy: "\r\nm=")
        let audio = sections[1], video = sections[2]
        #expect(audio.hasPrefix("audio"))
        #expect(!audio.contains("transport-cc"))
        #expect(!audio.contains("transport-wide-cc"))
        #expect(audio.contains("ssrc-audio-level"), "other extensions stay")
        #expect(video.contains("a=rtcp-fb:96 transport-cc") && video.contains("transport-wide-cc") && video.contains("a=rtcp-fb:96 nack"))
        #expect(SDPTuner.opusFmtp(in: tuned)["maxaveragebitrate"] == "128000")
        #expect(tuned.hasSuffix("\r\n"))
    }

    @Test func strippedEvenWithoutOpus() {
        let sdp = "v=0\r\nm=audio 9 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=rtcp-fb:0 transport-cc\r\n"
        #expect(SDPTuner.tuneOpus(sdp, bitrate: 128_000) == "v=0\r\nm=audio 9 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n")
    }
}
