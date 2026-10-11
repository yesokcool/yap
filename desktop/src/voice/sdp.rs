//! Opus SDP tuning, a port of `tuneOpus()` in web/js/voice.js
//! (PROTOCOL.md › Voice › Opus tuning).

/// Screen-share audio (a tab, a game) is music: stereo and at least this rich.
pub const SCREEN_AUDIO_MIN_BITRATE: u32 = 160_000;

const TWCC_URI: &str = "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";

/// Payload type of `opus/48000` in this SDP (its first audio section), if any.
pub fn opus_pt(sdp: &str) -> Option<String> {
    sdp.lines().find_map(|l| opus_pt_line(l.trim_end_matches('\r')))
}

fn opus_pt_line(l: &str) -> Option<String> {
    let rest = l.strip_prefix("a=rtpmap:")?;
    let (pt, codec) = rest.split_once(' ')?;
    codec.to_ascii_lowercase().starts_with("opus/48000").then(|| pt.to_string())
}

/// The voice's Opus `a=fmtp` parameter string (e.g. `minptime=10;useinbandfec=1`).
pub fn opus_fmtp(sdp: &str) -> Option<String> {
    let pt = opus_pt(sdp)?;
    let prefix = format!("a=fmtp:{pt} ");
    sdp.lines().find_map(|l| l.trim_end_matches('\r').strip_prefix(&prefix).map(str::to_string))
}

pub fn fmtp_param(fmtp: &str, key: &str) -> Option<String> {
    fmtp.split(';').find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k.trim() == key).then(|| v.trim().to_string())
    })
}

/// The DTLS fingerprint (hash value) in this SDP: a different one for the
/// same session means the far side started a new connection (PROTOCOL.md ›
/// Keeping calls across reconnects › fresh start).
pub fn fingerprint(sdp: &str) -> Option<String> {
    sdp.lines().find_map(|l| l.trim_end_matches('\r').strip_prefix("a=fingerprint:")?.split_whitespace().nth(1).map(str::to_string))
}

/// The fmtp parameters we want to receive on the voice (what `tune_opus` writes there).
pub fn tuned_fmtp(bitrate: u32, sprop_stereo: bool) -> String {
    format!(
        "minptime=10;useinbandfec=1;usedtx=0;stereo=1;sprop-stereo={};maxaveragebitrate={bitrate};maxplaybackrate=48000;sprop-maxcapturerate=48000",
        if sprop_stereo { 1 } else { 0 }
    )
}

fn is_twcc(line: &str) -> bool {
    if let Some(rest) = line.strip_prefix("a=rtcp-fb:") {
        let mut it = rest.split_whitespace();
        return it.next().is_some_and(|pt| pt.bytes().all(|b| b.is_ascii_digit())) && it.next() == Some("transport-cc") && it.next().is_none();
    }
    if let Some(rest) = line.strip_prefix("a=extmap:") {
        let mut it = rest.split_whitespace();
        return it.next().is_some() && it.next() == Some(TWCC_URI);
    }
    false
}

/// Tune every audio section (offer or answer). The Opus parameters describe
/// what *we* want to receive:
///  - the first audio section is the voice: `maxaveragebitrate` = our
///    setting, `sprop-stereo` = whether we send stereo (`music`);
///  - any later one is a screen share's audio: at least 160 kbps, and
///    `sprop-stereo=1` where we're the one sending it (sendrecv/sendonly);
///  - `useinbandfec=1`, `usedtx=0`, `stereo=1`, `minptime=10`, full band.
///
/// Audio sections also lose `transport-cc` (the rtcp-fb line and the header
/// extension): with it, libwebrtc's bandwidth probing pads audio-only calls
/// and gains nothing, since Opus runs at a fixed target. Video keeps it.
pub fn tune_opus(sdp: &str, bitrate: u32, music: bool) -> String {
    let eol = if sdp.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = sdp.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    // Split into the session part and media sections.
    let mut sections: Vec<Vec<&str>> = vec![Vec::new()];
    for l in lines {
        if l.starts_with("m=") {
            sections.push(Vec::new());
        }
        sections.last_mut().unwrap().push(l);
    }
    let mut audio = 0;
    let mut out: Vec<String> = Vec::new();
    for section in sections {
        if !section.first().is_some_and(|l| l.starts_with("m=audio")) {
            out.extend(section.iter().map(|l| l.to_string()));
            continue;
        }
        let voice = audio == 0;
        audio += 1;
        let sending = section.iter().any(|l| *l == "a=sendrecv" || *l == "a=sendonly");
        let stereo = if voice { music } else { sending };
        let rate = if voice { bitrate } else { bitrate.max(SCREEN_AUDIO_MIN_BITRATE) };
        out.extend(tune_audio(&section, rate, stereo));
    }
    out.join(eol)
}

fn tune_audio(section: &[&str], bitrate: u32, sprop_stereo: bool) -> Vec<String> {
    let lines: Vec<&str> = section.iter().copied().filter(|l| !is_twcc(l)).collect();
    let Some(pt) = lines.iter().find_map(|l| opus_pt_line(l)) else { return lines.iter().map(|l| l.to_string()).collect() };
    let want: [(&str, String); 8] = [
        ("minptime", "10".into()),
        ("useinbandfec", "1".into()),
        ("usedtx", "0".into()),
        ("stereo", "1".into()),
        ("sprop-stereo", if sprop_stereo { "1" } else { "0" }.into()),
        ("maxaveragebitrate", bitrate.to_string()),
        ("maxplaybackrate", "48000".into()),
        ("sprop-maxcapturerate", "48000".into()),
    ];
    let merge = |existing: &str| -> String {
        let mut params: Vec<(String, String)> = existing
            .split(';')
            .filter_map(|kv| {
                let kv = kv.trim();
                if kv.is_empty() {
                    return None;
                }
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                Some((k.trim().to_string(), v.trim().to_string()))
            })
            .collect();
        for (k, v) in &want {
            match params.iter_mut().find(|(pk, _)| pk == k) {
                Some(slot) => slot.1 = v.clone(),
                None => params.push((k.to_string(), v.clone())),
            }
        }
        params.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(";")
    };
    let fmtp_prefix = format!("a=fmtp:{pt} ");
    let rtpmap_prefix = format!("a=rtpmap:{pt} ");
    let has_fmtp = lines.iter().any(|l| l.starts_with(&fmtp_prefix));
    let mut out = Vec::with_capacity(lines.len() + 1);
    for line in lines {
        if let Some(existing) = line.strip_prefix(&fmtp_prefix) {
            out.push(format!("{fmtp_prefix}{}", merge(existing)));
        } else {
            out.push(line.to_string());
            if !has_fmtp && line.starts_with(&rtpmap_prefix) {
                out.push(format!("{fmtp_prefix}{}", merge("")));
            }
        }
    }
    out
}

/// Does this (negotiated) description already send our track: an
/// `a=msid:<stream> <track_id>` line in a section that sends?
pub fn sends_track(sdp: &str, track_id: &str) -> bool {
    sdp.split("\nm=").any(|section| {
        let lines: Vec<&str> = section.lines().map(str::trim_end).collect();
        lines.iter().any(|l| l.starts_with("a=msid:") && l.split_whitespace().nth(1) == Some(track_id)) && lines.iter().any(|l| *l == "a=sendrecv" || *l == "a=sendonly")
    })
}

/// How many of our tracks this (negotiated) description sends: sections with
/// an `a=msid` that send.
pub fn sending_tracks(sdp: &str) -> usize {
    sdp.split("\nm=")
        .skip(1)
        .filter(|section| {
            let lines: Vec<&str> = section.lines().map(str::trim_end).collect();
            lines.iter().any(|l| l.starts_with("a=msid:")) && lines.iter().any(|l| *l == "a=sendrecv" || *l == "a=sendonly")
        })
        .count()
}

#[cfg(test)]
mod tests {
    //! The web's SDP tests (test/voice.test.mjs) on the same offer.
    use super::*;

    /// A late negotiationneeded has nothing left to offer once our track is in the description.
    #[test]
    fn sends_track_reads_msid_and_direction() {
        let base = "v=0\r\no=- 1 2 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=mid:0\r\na=rtpmap:111 opus/48000/2\r\n";
        let ours = format!("{base}a=msid:- yap-audio-abc123\r\na=sendrecv\r\n");
        assert!(super::sends_track(&ours, "yap-audio-abc123"));
        assert!(!super::sends_track(&ours, "yap-audio-other"), "someone else's track");
        assert!(!super::sends_track(&format!("{base}a=msid:- yap-audio-abc123\r\na=inactive\r\n"), "yap-audio-abc123"), "not sending");
        assert!(!super::sends_track(&format!("{base}a=recvonly\r\n"), "yap-audio-abc123"), "no track of ours");
    }

    const SDP: &str = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111 0\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10;useinbandfec=1\r\na=rtpmap:0 PCMU/8000\r\n";

    /// What Chrome offers for a voice call that later gained a screen share (voice, tab audio, video).
    fn offer() -> String {
        [
            "v=0", "o=- 1 2 IN IP4 127.0.0.1", "s=-", "t=0 0", "a=group:BUNDLE 0 1 2", "a=msid-semantic: WMS",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9 0 8 13 110 126",
            "c=IN IP4 0.0.0.0", "a=mid:0", "a=sendrecv",
            "a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level",
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
            "a=rtpmap:111 opus/48000/2", "a=rtcp-fb:111 transport-cc", "a=fmtp:111 minptime=10;useinbandfec=1",
            "a=rtpmap:63 red/48000/2", "a=fmtp:63 111/111",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111 63",
            "c=IN IP4 0.0.0.0", "a=mid:1", "a=sendrecv",
            "a=extmap:1 urn:ietf:params:rtp-hdrext:ssrc-audio-level",
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
            "a=rtpmap:111 opus/48000/2", "a=rtcp-fb:111 transport-cc", "a=fmtp:111 minptime=10;useinbandfec=1",
            "m=video 9 UDP/TLS/RTP/SAVPF 96",
            "c=IN IP4 0.0.0.0", "a=mid:2", "a=sendrecv",
            "a=extmap:3 http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01",
            "a=rtpmap:96 VP8/90000", "a=rtcp-fb:96 transport-cc", "a=rtcp-fb:96 nack",
            "",
        ]
        .join("\r\n")
    }

    fn sections(sdp: &str) -> Vec<String> {
        sdp.split("\r\nm=").skip(1).map(str::to_string).collect()
    }

    fn fmtp(section: &str) -> std::collections::HashMap<String, String> {
        let line = section.lines().find_map(|l| l.trim_end_matches('\r').strip_prefix("a=fmtp:111 ")).unwrap();
        line.split(';').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn tunes_existing_line() {
        let out = tune_opus(SDP, 128_000, false);
        assert!(out.contains(
            "a=fmtp:111 minptime=10;useinbandfec=1;usedtx=0;stereo=1;sprop-stereo=0;maxaveragebitrate=128000;maxplaybackrate=48000;sprop-maxcapturerate=48000\r\n"
        ));
        assert!(out.ends_with("\r\n"));
        assert_eq!(fmtp_param(&opus_fmtp(&out).unwrap(), "maxaveragebitrate").as_deref(), Some("128000"));
    }

    #[test]
    fn every_audio_section_is_tuned() {
        let s = sections(&tune_opus(&offer(), 128_000, false));
        for sec in &s[..2] {
            let p = fmtp(sec);
            assert_eq!(p["stereo"], "1");
            assert_eq!(p["useinbandfec"], "1");
            assert_eq!(p["usedtx"], "0");
            assert_eq!(p["maxplaybackrate"], "48000");
        }
        assert_eq!(fmtp(&s[0])["maxaveragebitrate"], "128000");
        assert_eq!(fmtp(&s[1])["maxaveragebitrate"], "160000", "screen audio is at least 160 kbps");
    }

    #[test]
    fn sprop_stereo_follows_music_and_who_sends_screen_audio() {
        let s = sections(&tune_opus(&offer(), 128_000, false));
        assert_eq!(fmtp(&s[0])["sprop-stereo"], "0");
        assert_eq!(fmtp(&s[1])["sprop-stereo"], "1", "sendrecv screen audio = ours");
        let recv = offer().replace("a=mid:1\r\na=sendrecv", "a=mid:1\r\na=recvonly");
        let s = sections(&tune_opus(&recv, 128_000, false));
        assert_eq!(fmtp(&s[1])["sprop-stereo"], "0", "a screen we only watch isn't ours to describe");
        let s = sections(&tune_opus(&offer(), 128_000, true));
        assert_eq!(fmtp(&s[0])["sprop-stereo"], "1");
    }

    #[test]
    fn transport_cc_leaves_audio_only() {
        let s = sections(&tune_opus(&offer(), 128_000, false));
        for sec in &s[..2] {
            assert!(!sec.contains("transport-cc"), "{sec}");
            assert!(!sec.contains("transport-wide-cc"), "{sec}");
            assert!(sec.contains("ssrc-audio-level"), "speaking indicators keep their header extension");
        }
        assert!(s[2].contains("a=rtcp-fb:96 transport-cc"));
        assert!(s[2].contains("transport-wide-cc"));
    }

    #[test]
    fn idempotent_keeps_other_lines_and_line_endings() {
        let once = tune_opus(&offer(), 128_000, false);
        assert_eq!(tune_opus(&once, 128_000, false), once);
        assert!(once.ends_with("\r\n"));
        assert!(once.contains("a=fmtp:63 111/111"));
        assert!(once.contains("a=group:BUNDLE 0 1 2\r\n"));
        let lf = tune_opus(&offer().replace("\r\n", "\n"), 128_000, false);
        assert!(!lf.contains('\r'), "LF-only SDP stays LF-only");
    }

    #[test]
    fn adds_missing_line_after_its_rtpmap() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 109\r\na=rtpmap:109 OPUS/48000/2\r\n";
        let out = tune_opus(sdp, 64_000, true);
        assert!(out.contains("a=rtpmap:109 OPUS/48000/2\r\na=fmtp:109 minptime=10;"));
        assert!(out.contains("sprop-stereo=1"));
    }

    #[test]
    fn no_opus_untouched() {
        let sdp = "v=0\r\nm=audio 9 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        assert_eq!(tune_opus(sdp, 64_000, false), sdp);
    }

    #[test]
    fn reads_the_fingerprint() {
        let sdp = "v=0\r\na=fingerprint:sha-256 AB:CD:EF\r\nm=audio 9 x 111\r\n";
        assert_eq!(fingerprint(sdp).as_deref(), Some("AB:CD:EF"));
        assert_eq!(fingerprint("v=0\r\n"), None);
    }
}
