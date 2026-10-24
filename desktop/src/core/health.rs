//! How a call is doing, and keeping it working: connection hints per person
//! (PROTOCOL.md › Voice › Connection hints), "you're sending silence", the mic
//! and speakers following devices and coming back after errors (V6), and
//! Settings › Voice › Hear yourself (V11).

use super::*;

/// Hints are judged this often…
const HINT_EVERY: Duration = Duration::from_secs(2);
/// …never in a connection's first 10 s (the jitter buffer is still settling)…
const HINT_SETTLE: Duration = Duration::from_secs(10);
/// …only while they're actually sending…
const HINT_MIN_PPS: f64 = 30.0;
/// …and say "weak" past these.
const WEAK_CONCEALED: f32 = 0.03;
const WEAK_RTT_MS: u32 = 300;
/// Devices are checked this often in a call.
const DEVICES_EVERY: Duration = Duration::from_secs(2);
/// Hear yourself records this long.
pub(super) const HEAR_SECS: u64 = 4;
/// "You're muted": at most this often…
pub const NUDGE_EVERY: Duration = Duration::from_secs(5);
/// …and only for talking this recent (ms; not something from before the call).
const NUDGE_FRESH_MS: u64 = 400;
/// The nudge's soft blip: two quick low notes (the web's `sounds.mutedTalk`).
pub const NUDGE_NOTES: [f32; 2] = [329.63, 329.63];

/// Should talking while muted be pointed out now? `talked_ms`: when the gate
/// last saw it (the gate's clock), `now_ms` the gate's clock now, `last`: the
/// last nudge.
pub fn nudge_due(talked_ms: u64, now_ms: u64, last: Option<Instant>, now: Instant) -> bool {
    talked_ms > 0 && now_ms.saturating_sub(talked_ms) <= NUDGE_FRESH_MS && last.is_none_or(|t| now.duration_since(t) >= NUDGE_EVERY)
}

/// One connection's numbers at the last hint check.
#[derive(Default)]
pub(super) struct HintTrack {
    at: Option<Instant>,
    /// Our packets sent to them by then.
    sent: u64,
    packets: u64,
    frames: u64,
    concealed: u64,
    /// Bad (+) or good (−) readings in a row.
    run: i32,
    weak: Option<(f32, Option<u32>)>,
}

/// One weak/good reading, three in a row to change: what to show now.
pub(super) fn judge(track: &mut HintTrack, share: f32, rtt: Option<u32>) -> Option<(f32, Option<u32>)> {
    let bad = share > WEAK_CONCEALED || rtt.is_some_and(|r| r > WEAK_RTT_MS);
    track.run = if bad { track.run.max(0) + 1 } else { track.run.min(0) - 1 };
    if track.run >= 3 || (track.weak.is_some() && bad) {
        track.weak = Some((share, rtt));
    } else if track.run <= -3 {
        track.weak = None;
    }
    track.weak
}

/// Your own connection (PROTOCOL.md › Connection hints): each 2 s, what the
/// people you're sending to report losing of your audio (the fraction lost
/// in their latest report about it). More than 3 % for most of them (or the
/// only one) three rounds in a row: it's your side, "Your connection is
/// dropping some of your audio"; gone after three good rounds, or when
/// there's nobody to judge by. The web's `OwnConnection`, the same rules.
#[derive(Default)]
pub(super) struct OwnTrack {
    /// Bad (+) or good (−) rounds in a row.
    run: i32,
    /// Shown, with the middle of what they report losing (0–1).
    pub weak: Option<f32>,
}

impl OwnTrack {
    /// One round: the fraction lost each person we're sending to reports. True when `weak` came or went.
    pub fn round(&mut self, fractions: &[f32]) -> bool {
        let was = self.weak.is_some();
        if fractions.is_empty() {
            self.run = 0;
            self.weak = None;
            return was;
        }
        let bad = fractions.iter().filter(|f| **f > WEAK_CONCEALED).count() * 2 > fractions.len();
        self.run = if bad { self.run.max(0) + 1 } else { self.run.min(0) - 1 };
        let mut sorted = fractions.to_vec();
        sorted.sort_by(f32::total_cmp);
        let middle = sorted[sorted.len() / 2];
        if self.run >= 3 || (self.weak.is_some() && bad) {
            self.weak = Some(middle);
        } else if self.run <= -3 {
            self.weak = None;
        }
        was != self.weak.is_some()
    }
}

/// The voice panel's words, and why (the web's).
pub const OWN_WEAK_TEXT: &str = "Your connection is dropping some of your audio";
pub fn own_weak_title(lost: f32) -> String {
    let pct = ((lost * 100.0).round() as u32).max(1);
    format!("The others say about {pct}% of what you send never reaches them, so it's your connection, not theirs: a busy Wi‑Fi or mobile network. It usually passes; a cable, or getting closer to the router, helps.")
}

/// Should the mic (or speakers) be reopened? `configured`: the device picked
/// in Settings ("" = the system default); `opened`: the one in use; `default`:
/// the system default now; `available`: what's plugged in.
pub fn device_moved(configured: &str, opened: Option<&str>, default: Option<&str>, available: &[String]) -> bool {
    let Some(opened) = opened else { return false };
    if configured.is_empty() {
        // Following the system default: it moved (headphones plugged in, AirPods connected).
        default.is_some_and(|d| d != opened)
    } else if opened != configured {
        // We fell back to the default because the picked one was gone: it's back.
        available.iter().any(|d| d == configured)
    } else {
        // The picked one went away.
        !available.iter().any(|d| d == opened)
    }
}

/// Hear yourself in progress.
pub(super) struct HearRun {
    pub until: Instant,
    /// We opened the mic for it (outside a call): close it again after.
    pub opened_mic: bool,
    /// Headless: write what came out of the codec here.
    pub save: Option<String>,
}

impl Core {
    /// Every tick in a call: hints (every 2 s), the silence warning, device checks.
    pub(super) fn tick_health(&mut self) {
        let in_voice = self.shared.model.lock().voice_channel.is_some();
        // "Your mic is sending silence": 5 s of nothing at all while unmuted.
        let silent = {
            let g = &self.shared.audio.gate;
            let m = self.shared.model.lock();
            in_voice && m.mic_ok && !m.config.muted && !m.config.deafened && g.silent.load(Ordering::Relaxed)
        };
        if silent != self.shared.model.lock().mic_silent {
            self.shared.model.lock().mic_silent = silent;
            self.call_note(if silent { "Your mic is sending silence (nothing at all comes from it)" } else { "Your mic is sending sound again" });
            self.emit(CoreEvent::Extra("MIC", if silent { "sending silence".into() } else { "sound again".into() }));
            self.repaint();
        }
        self.tick_nudge(in_voice);
        // Push-to-talk latched by a double-tap, or let go: on the timeline (the web's words).
        let latched = self.shared.audio.gate.ptt_latched.load(Ordering::Relaxed);
        if latched != self.ptt_latched {
            self.ptt_latched = latched;
            self.call_note(if latched { "Push-to-talk latched on (double-tap)" } else { "Push-to-talk latch off" });
        }
        if in_voice && self.last_hints.is_none_or(|t| t.elapsed() >= HINT_EVERY) {
            self.last_hints = Some(Instant::now());
            self.update_hints();
        }
        let live = self.shared.model.lock().voice_wanted.is_some() || self.hear.is_some();
        if live && self.last_devices.is_none_or(|t| t.elapsed() >= DEVICES_EVERY) {
            self.last_devices = Some(Instant::now());
            self.check_devices();
        }
        if self.hear.as_ref().is_some_and(|h| Instant::now() >= h.until) {
            self.finish_hearing();
        }
    }

    /// Talking while muted (PROTOCOL.md › Voice › Transmit gate): "You're
    /// muted" by the Mute button, at most every 5 s, with a soft blip the
    /// first time since we muted or joined; the first blip on this device,
    /// what it means. Not while another of our devices is live in the call.
    fn tick_nudge(&mut self, in_voice: bool) {
        let g = &self.shared.audio.gate;
        let talked = g.talked_muted_ms.load(Ordering::Relaxed);
        if talked == self.nudge_seen {
            return;
        }
        self.nudge_seen = talked;
        let (remind, muted, first, elsewhere) = {
            let m = self.shared.model.lock();
            // (Not while deafened, like the web: you meant to be out of it.)
            (m.config.remind_muted, m.config.muted && !m.config.deafened, !m.config.muted_explained, m.live_elsewhere())
        };
        let now = Instant::now();
        if !in_voice || !remind || !muted || elsewhere || !nudge_due(talked, g.now_ms(), self.last_nudge, now) {
            return;
        }
        self.last_nudge = Some(now);
        let blip = std::mem::take(&mut self.nudge_blip);
        let first = first && blip;
        if blip {
            self.shared.audio.mixer.play_notes(&NUDGE_NOTES, 0.11, 0.07, 0.035, false);
        }
        {
            let mut m = self.shared.model.lock();
            // (The first time's explanation stays up its whole 8 s.)
            let explaining = m.muted_nudge.is_some_and(|(at, explain)| explain && at.elapsed() < Duration::from_secs(8));
            if !explaining {
                m.muted_nudge = Some((now, first));
            }
            if first {
                m.config.muted_explained = true;
                m.config_dirty = true;
            }
        }
        self.emit(CoreEvent::Extra("NUDGE", if first { "you're muted (explained)".into() } else if blip { "you're muted".into() } else { "you're muted (quiet)".into() }));
        self.repaint();
    }

    /// Connection hints: "weak connection", "needs the voice relay" / "can't connect".
    fn update_hints(&mut self) {
        let relay = self.voice.has_relay();
        let peers = self.voice.peers();
        let debug = cfg!(debug_assertions).then(|| std::env::var("YAP_DEBUG_HINT").ok()).flatten();
        let mut hints: HashMap<String, Hint> = HashMap::new();
        let mut weak_changes = Vec::new();
        // What the people we're sending to report losing of our voice (your own connection).
        let mut yours: Vec<f32> = Vec::new();
        for p in &peers {
            let sid = p.sid();
            let track = self.hint_track.entry(p.id).or_default();
            let was_weak = track.weak;
            let stats = self.shared.audio.stream(&sid).map(|s| s.stats()).unwrap_or_default();
            let now = Instant::now();
            let connected = *p.state.lock() == "connected";
            let sent = p.packets_sent.load(Ordering::Relaxed);
            if let Some(at) = track.at {
                let secs = now.duration_since(at).as_secs_f64().max(0.001);
                let sending = sent.saturating_sub(track.sent) as f64 / secs >= HINT_MIN_PPS;
                if connected && sending && p.their_reports.load(Ordering::Relaxed) > 0 {
                    yours.push(p.their_fraction.load(Ordering::Relaxed) as f32 / 1000.0);
                }
                let pps = stats.packets.saturating_sub(track.packets) as f64 / secs;
                let settled = p.connected_at.lock().is_some_and(|c| c.elapsed() > HINT_SETTLE);
                if pps >= HINT_MIN_PPS && connected && settled {
                    let frames = stats.frames.saturating_sub(track.frames).max(1);
                    let share = stats.concealed.saturating_sub(track.concealed) as f32 / frames as f32;
                    let rtt = Some(p.rtt_ms.load(Ordering::Relaxed)).filter(|r| *r > 0);
                    judge(track, share, rtt);
                }
            }
            track.at = Some(now);
            track.sent = sent;
            track.packets = stats.packets;
            track.frames = stats.frames;
            track.concealed = stats.concealed;
            let weak = track.weak;
            if weak.is_some() != was_weak.is_some() {
                weak_changes.push((sid.clone(), weak));
            }
            let hint = if p.failed.load(Ordering::Relaxed) && !connected {
                Some(if relay { Hint::CantConnect } else { Hint::NeedsRelay })
            } else {
                track.weak.map(|(concealed, rtt)| Hint::Weak { concealed, rtt })
            };
            // Screenshot aid (debug builds): YAP_DEBUG_HINT=weak|relay|cant|reconnecting on everyone.
            let hint = match debug.as_deref() {
                Some("weak") => Some(Hint::Weak { concealed: 0.06, rtt: Some(340) }),
                Some("relay") => Some(Hint::NeedsRelay),
                Some("cant") => Some(Hint::CantConnect),
                Some("reconnecting") => Some(Hint::Reconnecting),
                _ => hint,
            };
            if let Some(h) = hint {
                hints.insert(sid, h);
            }
        }
        for (sid, weak) in weak_changes {
            self.call_weak(&sid, weak);
        }
        // Your own connection: on (or off) for the voice panel, the timeline and the logs.
        let own_changed = self.own.round(&yours);
        let own = if debug.as_deref() == Some("own") { Some(0.06) } else { self.own.weak };
        if self.shared.model.lock().own_weak.is_some() != own.is_some() || own_changed {
            self.shared.model.lock().own_weak = own;
            let line = match own {
                Some(lost) => format!("{OWN_WEAK_TEXT} (they lose about {} % of yours)", (lost * 100.0).round()),
                None => "Your connection is fine again".to_string(),
            };
            self.emit(CoreEvent::Extra("HINT", format!("self {}", if own.is_some() { "dropping" } else { "ok" })));
            self.call_note(line);
            self.repaint();
        } else if own.is_some() {
            self.shared.model.lock().own_weak = own; // (still on: the latest numbers)
        }
        let live: HashSet<u64> = peers.iter().map(|p| p.id).collect();
        self.hint_track.retain(|id, _| live.contains(id));
        let changed = {
            let mut m = self.shared.model.lock();
            if debug.as_deref() == Some("reconnecting") {
                m.resuming = true; // and our own call too (after a deploy everyone is)
            }
            for g in m.ghosts.keys() {
                hints.insert(g.clone(), Hint::Reconnecting);
            }
            let changed: Vec<(String, Option<Hint>)> = hints
                .iter()
                .filter(|(s, h)| m.hints.get(*s) != Some(*h) && !matches!((m.hints.get(*s), h), (Some(Hint::Weak { .. }), Hint::Weak { .. })))
                .map(|(s, h)| (s.clone(), Some(h.clone())))
                .chain(m.hints.keys().filter(|s| !hints.contains_key(*s)).map(|s| (s.clone(), None)))
                .collect();
            m.hints = hints;
            changed
        };
        for (sid, h) in &changed {
            self.emit(CoreEvent::Extra("HINT", format!("{sid} {}", h.as_ref().map(|h| h.text()).unwrap_or("ok"))));
        }
        if !changed.is_empty() {
            self.repaint();
        }
    }

    /// V6: never silently lose the mic or the speakers. A stream that errored
    /// or stalled is reopened; the system default moving (or a picked device
    /// coming back) reopens it on that device. Devices are only looked at
    /// with real audio (never in headless runs or with YAP_DEBUG_AUDIO).
    fn check_devices(&mut self) {
        let health = self.shared.audio.health();
        let input = health.input_error || health.input_stalled;
        let output = health.output_error || health.output_stalled;
        if input || output {
            log::warn!("audio: {health:?}; reopening");
            self.emit(CoreEvent::Extra("AUDIO", format!("reopening{}{}", if input { " mic" } else { "" }, if output { " speakers" } else { "" })));
            self.call_note(format!("The {} stopped or failed: reopening it", if input && output { "mic and speakers" } else if input { "mic" } else { "speakers" }));
            let _ = self.cmd_tx.send(Cmd::AudioRestart { input, output });
            return;
        }
        if self.shared.audio_override.lock().is_some() || self.devices_busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let (cfg_in, cfg_out) = {
            let m = self.shared.model.lock();
            (m.config.input_device.clone(), m.config.output_device.clone())
        };
        let (busy, tx) = (self.devices_busy.clone(), self.cmd_tx.clone());
        tokio::task::spawn_blocking(move || {
            let (ins, outs) = crate::audio::list_devices();
            let (din, dout) = crate::audio::default_devices();
            let input = device_moved(&cfg_in, health.opened_input.as_deref(), din.as_deref(), &ins);
            let output = device_moved(&cfg_out, health.opened_output.as_deref(), dout.as_deref(), &outs);
            if input || output {
                log::info!("audio devices changed (mic {input}, speakers {output}): following");
                let _ = tx.send(Cmd::AudioRestart { input, output });
            }
            busy.store(false, Ordering::SeqCst);
        });
    }

    /// Reopen the mic and/or speakers (the picked device, else the default).
    pub(super) async fn restart_audio(&mut self, input: bool, output: bool) {
        let audio = self.shared.audio.clone();
        if input && audio.input_running() {
            audio.stop_input();
        }
        if output && audio.output_running() {
            audio.stop_output();
        }
        self.prepare_audio().await;
        self.shared.apply_gate();
        self.send_voice_state();
        self.repaint();
    }

    /// Settings › Voice › Hear yourself: record 4 s of exactly what a call
    /// sends (the processing and the gate, so voice activity cuts in and out
    /// as it would), through Opus at the call's bitrate, then play it back.
    /// Outside a call push-to-talk acts like always-on meanwhile; in a call it
    /// doesn't (others would hear you), so it says to hold the key.
    pub(super) async fn hear_yourself(&mut self, save: Option<String>) {
        if self.hear.is_some() {
            return;
        }
        let opened_mic = !self.shared.audio.input_running();
        if opened_mic {
            self.prepare_input_only().await;
        }
        if !self.shared.audio.gate.mic_ok.load(Ordering::Relaxed) {
            self.shared.model.lock().hear = HearState::Failed;
            self.emit(CoreEvent::Extra("HEAR", "no mic".into()));
            return self.repaint();
        }
        let (alone, ptt) = {
            let m = self.shared.model.lock();
            (self.voice.peers().is_empty(), m.config.mode == InputMode::Ptt)
        };
        self.hear = Some(HearRun { until: Instant::now() + Duration::from_secs(HEAR_SECS), opened_mic, save });
        self.shared.model.lock().hear = HearState::Recording { hold_key: ptt && !alone };
        self.shared.apply_gate();
        self.shared.audio.record(true);
        self.emit(CoreEvent::Extra("HEAR", "recording".into()));
        self.repaint();
    }

    fn finish_hearing(&mut self) {
        let Some(run) = self.hear.take() else { return };
        let samples = self.shared.audio.record(false).unwrap_or_default();
        self.shared.model.lock().hear = HearState::Playing;
        self.shared.apply_gate();
        let (bitrate, device, in_call, mic_test) = {
            let m = self.shared.model.lock();
            (m.voice_bitrate(), m.config.output_device.clone(), m.voice_wanted.is_some(), m.mic_test)
        };
        if run.opened_mic && !in_call && !mic_test {
            self.shared.audio.stop_input();
        }
        let heard = through_opus(&samples, bitrate);
        let (rms, peak) = level(&heard);
        self.emit(CoreEvent::Extra("HEAR", format!("{:.1} s through Opus at {} kbps: {rms:.1} dBFS RMS, peak {peak:.1} dBFS", heard.len() as f32 / 48_000.0, bitrate / 1000)));
        if let Some(path) = &run.save
            && let Ok(mut w) = crate::audio::wav::WavWriter::create(path, crate::audio::RATE, 2)
        {
            let stereo: Vec<f32> = heard.iter().flat_map(|s| [*s, *s]).collect();
            w.write(&stereo);
            w.finish();
        }
        let secs = heard.len() as f32 / 48_000.0;
        self.shared.model.lock().hear = HearState::Playing;
        // Playback on your speakers — never a real device in headless runs or with YAP_DEBUG_AUDIO.
        if self.shared.audio_override.lock().is_none() {
            self.shared.audio.play_clip(Arc::new(heard), 1.0, crate::audio::clips::Bus::Fx, &device);
        }
        let tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f32(secs + 0.2)).await;
            let _ = tx.send(Cmd::HearDone);
        });
        self.repaint();
    }
}

/// Mono 48 kHz through an Opus encoder like a peer's (clean network) and a decoder.
fn through_opus(samples: &[f32], bitrate: u32) -> Vec<f32> {
    let (Some(mut enc), Ok(mut dec)) = (crate::voice::make_encoder(bitrate, true, 0), ropus::Decoder::new(48_000, ropus::Channels::Mono)) else {
        return samples.to_vec();
    };
    let mut out = Vec::with_capacity(samples.len());
    let (mut buf, mut pcm) = (vec![0u8; 4000], vec![0i16; 960]);
    for frame in samples.chunks(960) {
        let mut f = [0i16; 960];
        for (d, s) in f.iter_mut().zip(frame) {
            *d = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        }
        let Ok(n) = enc.encode(&f, &mut buf) else { continue };
        if let Ok(got) = dec.decode(&buf[..n], &mut pcm, ropus::DecodeMode::Normal) {
            out.extend(pcm[..got].iter().map(|&s| s as f32 / 32768.0));
        }
    }
    out
}

/// (RMS, peak) in dBFS.
fn level(x: &[f32]) -> (f32, f32) {
    let ms = x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32;
    let peak = x.iter().fold(0f32, |m, s| m.max(s.abs()));
    let db = |v: f32| if v > 0.0 { 20.0 * v.log10() } else { -120.0 };
    (db(ms.sqrt()), db(peak))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weak_needs_three_bad_readings_and_three_good_to_clear() {
        let mut t = HintTrack::default();
        assert_eq!(judge(&mut t, 0.05, None), None);
        assert_eq!(judge(&mut t, 0.05, None), None);
        assert!(judge(&mut t, 0.05, None).is_some(), "weak after three");
        assert!(judge(&mut t, 0.0, Some(20)).is_some());
        assert!(judge(&mut t, 0.0, Some(20)).is_some());
        assert_eq!(judge(&mut t, 0.0, Some(20)), None, "gone after three good");
        // A slow round trip alone counts too.
        for _ in 0..3 {
            judge(&mut t, 0.0, Some(450));
        }
        assert_eq!(t.weak, Some((0.0, Some(450))));
        // One good reading in a bad run starts over.
        let mut u = HintTrack::default();
        judge(&mut u, 0.1, None);
        judge(&mut u, 0.1, None);
        judge(&mut u, 0.0, None);
        assert_eq!(judge(&mut u, 0.1, None), None);
    }

    /// The web's OwnConnection cases: the only one, 2 of 3, not 1 of 2,
    /// exactly 3 % isn't more than 3 %, three rounds each way, and nobody to
    /// judge by clears it.
    #[test]
    fn your_own_connection_needs_most_people_three_rounds_running() {
        let mut t = OwnTrack::default();
        assert!(!t.round(&[0.08]) && !t.round(&[0.08]));
        assert!(t.round(&[0.08]), "the only one, three rounds: shown");
        assert_eq!(t.weak, Some(0.08));
        assert!(!t.round(&[0.0]) && !t.round(&[0.0]));
        assert!(t.round(&[0.0]) && t.weak.is_none(), "three good rounds: gone");
        let mut t = OwnTrack::default();
        for _ in 0..3 {
            t.round(&[0.1, 0.05, 0.0]);
        }
        assert_eq!(t.weak, Some(0.05), "2 of 3 lose more than 3 % (the middle one)");
        let mut t = OwnTrack::default();
        for _ in 0..5 {
            t.round(&[0.2, 0.0]);
        }
        assert_eq!(t.weak, None, "1 of 2 isn't most: theirs, not yours");
        let mut t = OwnTrack::default();
        for _ in 0..5 {
            t.round(&[0.03]);
        }
        assert_eq!(t.weak, None, "exactly 3 % isn't more than 3 %");
        let mut t = OwnTrack::default();
        for _ in 0..3 {
            t.round(&[0.1]);
        }
        assert!(t.round(&[]) && t.weak.is_none(), "alone (or no reports): nothing to say");
        // One good round in a bad run starts over.
        let mut t = OwnTrack::default();
        t.round(&[0.1]);
        t.round(&[0.1]);
        t.round(&[0.0]);
        t.round(&[0.1]);
        assert_eq!(t.weak, None);
        assert!(own_weak_title(0.004).contains("about 1%"), "never \"0%\"");
    }

    #[test]
    fn the_muted_nudge_waits_five_seconds_and_only_for_fresh_talk() {
        let t0 = Instant::now();
        assert!(nudge_due(1000, 1100, None, t0), "talking now, never nudged");
        assert!(!nudge_due(0, 1100, None, t0), "never talked");
        assert!(!nudge_due(1000, 2000, None, t0), "talked a second ago (before the call): not now");
        assert!(!nudge_due(9000, 9050, Some(t0), t0 + Duration::from_millis(4900)), "4.9 s after the last one");
        assert!(nudge_due(9000, 9050, Some(t0), t0 + Duration::from_secs(5)), "5 s after");
    }

    #[test]
    fn follows_the_default_and_picked_devices() {
        let devs = vec!["MacBook Mic".to_string(), "AirPods".to_string()];
        // Following the default: it moved to the AirPods.
        assert!(device_moved("", Some("MacBook Mic"), Some("AirPods"), &devs));
        assert!(!device_moved("", Some("AirPods"), Some("AirPods"), &devs));
        // Picked AirPods, they were gone (we fell back), now they're back.
        assert!(device_moved("AirPods", Some("MacBook Mic"), Some("MacBook Mic"), &devs));
        assert!(!device_moved("AirPods", Some("MacBook Mic"), Some("MacBook Mic"), &devs[..1]));
        // The picked one went away while in use.
        assert!(device_moved("AirPods", Some("AirPods"), Some("MacBook Mic"), &devs[..1]));
        // Nothing open (a WAV, silence): nothing to follow.
        assert!(!device_moved("", None, Some("AirPods"), &devs));
    }

    #[test]
    fn hear_yourself_goes_through_opus() {
        let x: Vec<f32> = (0..48_000).map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / 48_000.0).sin() * 0.3).collect();
        let y = through_opus(&x, 128_000);
        assert_eq!(y.len(), x.len());
        let (rms, _) = level(&y[4800..]);
        assert!((rms - level(&x).0).abs() < 1.5, "level through the codec {rms:.1} dBFS");
    }
}
