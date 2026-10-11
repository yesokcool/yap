//! Voice: joining and leaving, peers' state, audio devices, and the sounds
//! (chimes, join chimes, the soundboard). Keeping a call across a reconnect
//! is in `resume.rs`; connection hints, device recovery and Hear yourself in
//! `health.rs`.

use super::*;

/// A notification chime.
#[derive(Clone, Copy)]
pub(super) enum Chime {
    Mention,
    Ping,
}

impl Core {
    pub(super) fn chime(&self, kind: Chime) {
        let (sounds, device) = {
            let m = self.shared.model.lock();
            (m.config.sounds, m.config.output_device.clone())
        };
        if !sounds || self.shared.audio_override.lock().is_some() {
            return; // headless tests stay quiet
        }
        let tone = match kind {
            Chime::Mention => crate::audio::Tone { notes: &[987.77, 1318.5], step: 0.08, length: 0.16, volume: 0.09, wave: crate::audio::Wave::Triangle },
            Chime::Ping => crate::audio::Tone { notes: &[783.99, 1046.5, 1318.5, 1567.98], step: 0.09, length: 0.22, volume: 0.11, wave: crate::audio::Wave::Square },
        };
        self.shared.audio.chime(&tone, &device);
    }

    /// Someone in our call left (or was let go): the leave blip. Logged for
    /// the headless tests either way, played only with Sounds on.
    pub(super) fn leave_chime(&self) {
        self.emit(CoreEvent::Extra("CHIME", "leave".into()));
        if self.shared.model.lock().config.sounds {
            self.shared.audio.mixer.play_notes(&[880.0, 659.25], 0.075, 0.11, 0.05, false);
        }
    }

    /// A clip's samples (mono 48 kHz): cached, or fetched in the background
    /// and then handed to `then`.
    pub(super) fn with_pcm(&self, sound: &Sound, then: impl FnOnce(Arc<Vec<f32>>) + Send + 'static) {
        if let Some(pcm) = self.pcm.lock().get(&sound.id).cloned() {
            return then(pcm);
        }
        let (server, cache, id, media) = (self.shared.model.lock().config.server.clone(), self.pcm.clone(), sound.id.clone(), sound.media.clone());
        tokio::task::spawn_blocking(move || {
            let fetched = crate::http::get_bytes(&format!("{server}/media/{media}")).and_then(|(bytes, _)| crate::audio::wav::decode_mono(&bytes));
            match fetched {
                Ok(w) => {
                    let pcm = Arc::new(crate::audio::wav::resample_mono(&w.samples, w.rate, crate::audio::RATE));
                    cache.lock().insert(id, pcm.clone());
                    then(pcm);
                }
                Err(e) => log::warn!("sound {id}: {e}"),
            }
        });
    }

    /// Someone in our call played a soundboard clip: through the volume cap and
    /// limiter, unless deafened, the soundboard is muted, or we muted them.
    /// Returns whether it plays (headless: whether it would).
    pub(super) fn play_board_sound(&self, id: &str, uid: &str) -> bool {
        let (sound, vol, device, ok) = {
            let m = self.shared.model.lock();
            let ok = !m.config.deafened && !m.config.sound_mute && m.config.volume_for(uid) > 0.0 && m.voice_channel.is_some();
            (m.sound(id).cloned(), m.config.sound_volume.clamp(0.0, 1.0), m.config.output_device.clone(), ok)
        };
        let Some(sound) = sound.filter(|_| ok) else { return false };
        if self.shared.audio_override.lock().is_some() {
            return true; // headless: say it would play, stay quiet
        }
        let (audio, gain) = (self.shared.audio.clone(), sound.gain.min(1.0) * vol);
        self.with_pcm(&sound, move |pcm| audio.play_clip(pcm, gain, crate::audio::clips::Bus::Board, &device));
        true
    }

    /// Someone joined our voice channel: their chime (built-in or uploaded),
    /// through the same volume cap and limiter; nothing when deafened or with
    /// Sounds off.
    pub(super) fn play_join_chime(&self, uid: &str) {
        self.emit(CoreEvent::Extra("CHIME", format!("join {uid}")));
        let (chime, vol, device, ok) = {
            let m = self.shared.model.lock();
            let chime = m.users.get(uid).and_then(|u| u.chime.clone()).unwrap_or_else(|| "default".into());
            (chime, m.config.sound_volume.clamp(0.0, 1.0), m.config.output_device.clone(), m.config.sounds && !m.config.deafened)
        };
        if !ok || self.shared.audio_override.lock().is_some() {
            return;
        }
        let audio = self.shared.audio.clone();
        match chime.strip_prefix("s:") {
            Some(id) => {
                let sound = self.shared.model.lock().sound(id).cloned();
                match sound {
                    Some(sound) => {
                        let gain = sound.gain.min(1.0) * vol;
                        self.with_pcm(&sound, move |pcm| audio.play_clip(pcm, gain, crate::audio::clips::Bus::Fx, &device));
                    }
                    None => self.play_builtin_chime("default", vol, &device),
                }
            }
            None => self.play_builtin_chime(&chime, vol, &device),
        }
    }

    /// Built-ins play at their own (quiet) volume, like the web; the soundboard
    /// volume is for uploaded clips.
    pub(super) fn play_builtin_chime(&self, id: &str, _vol: f32, device: &str) {
        if let Some(samples) = chime_samples(id).or_else(|| (id != "none").then(|| chime_samples("default")).flatten()) {
            self.shared.audio.play_clip(Arc::new(samples), 1.0, crate::audio::clips::Bus::Fx, device);
        }
    }

    /// Who keeps calls across reconnects, as the connections should know it.
    fn sync_resumable(&self) {
        let m = self.shared.model.lock();
        for sid in self.voice.sids() {
            let on = m.sessions.get(&sid).and_then(|s| s.voice.as_ref()).is_some_and(|v| v.resume);
            self.voice.set_resumable(&sid, on);
        }
    }

    /// A `voice` message: someone (or we) joined, left or changed state.
    pub(super) fn on_voice(&mut self, v: VoiceMsg) {
        let VoiceMsg { sid, channel_id, muted, deafened, screen, resume, prev: prev_sid, lost } = v;
        // Our own earlier session going (the server drops it after we resumed): not someone leaving.
        if self.old_sids.contains(&sid) {
            let mut m = self.shared.model.lock();
            if channel_id.is_none() {
                m.sessions.remove(&sid);
            }
            return;
        }
        let mut m = self.shared.model.lock();
        let prev = m.sessions.get(&sid).and_then(|s| s.voice.as_ref().map(|v| v.channel_id.clone()));
        let was_resumable = m.sessions.get(&sid).and_then(|s| s.voice.as_ref()).is_some_and(|v| v.resume);
        let was_sharing = m.sessions.get(&sid).and_then(|s| s.voice.as_ref()).is_some_and(|v| v.screen);
        if let Some(s) = m.sessions.get_mut(&sid) {
            s.voice = channel_id.clone().map(|c| crate::protocol::VoiceState { channel_id: c, muted, deafened, screen, resume, prev: prev_sid.clone() });
        }
        let me = m.sid.as_deref() == Some(sid.as_str());
        drop(m);
        // Someone continues an earlier session's call (maybe one we're keeping): move it over.
        let resumed = channel_id.is_some() && !me && self.adopt_resumed(&sid);
        let sharing = screen && channel_id.is_some();
        // (A sharer whose socket dropped may be back in a moment, screen and all.)
        if sharing != was_sharing && !(lost && was_resumable) {
            self.emit(CoreEvent::Screen { sid: sid.clone(), on: sharing });
            if !sharing {
                self.sharer_stopped(&sid);
            }
        }
        let mut m = self.shared.model.lock();
        let sounds = m.config.sounds;
        if me {
            if let Some(ch) = channel_id.clone().filter(|c| Some(c) != m.voice_channel.as_ref()) {
                m.voice_channel = Some(ch.clone());
                m.voice_wanted = Some(ch.clone());
                let others: Vec<String> = m
                    .sessions
                    .values()
                    .filter(|s| s.sid != sid && !m.ghosts.contains_key(&s.sid) && s.voice.as_ref().is_some_and(|v| v.channel_id == ch))
                    .map(|s| s.sid.clone())
                    .collect();
                let vols: HashMap<String, f32> = m.sessions.values().map(|s| (s.sid.clone(), m.session_volume(&s.sid))).collect();
                self.voice.settings.bitrate.store(m.voice_bitrate(), Ordering::Relaxed);
                drop(m);
                let handed_over = self.resume.as_ref().is_some_and(|r| prev_sid.as_deref() == Some(r.sid.as_str()));
                self.sync_resumable();
                if handed_over {
                    // The same call, under our new session.
                    log::info!("voice: our call was handed over to this session");
                    let signals = self.voice.handover(&others, |s| vols.get(s).copied().unwrap_or(1.0));
                    self.send_signals(signals);
                    self.handover_check = Some((Instant::now(), others.clone()));
                    self.emit(CoreEvent::Extra("RESUME", format!("handed over ({} kept)", self.voice.sids().len())));
                } else {
                    // A fresh join (or the server didn't take our resume): start over.
                    let ghosts: Vec<String> = self.shared.model.lock().ghosts.keys().cloned().collect();
                    for g in ghosts {
                        self.drop_ghost(&g, true);
                    }
                    self.voice.hangup_all();
                    self.voice.enter(&others, |s| vols.get(s).copied().unwrap_or(1.0));
                }
                self.sync_resumable();
                *self.shared.voice_peers.lock() = self.voice.peers();
                self.call_joined(&ch, &others, handed_over);
                if !self.rejoining {
                    self.emit(CoreEvent::Extra("CHIME", "self-join".into()));
                    if sounds {
                        self.shared.audio.mixer.play_notes(&[523.25, 659.25, 783.99], 0.075, 0.11, 0.07, false);
                    }
                }
                self.rejoining = false;
                self.end_resume();
                self.emit(CoreEvent::VoiceJoined { channel: ch });
                self.shared.apply_gate();
                // Still sharing after a fresh join (the call couldn't be kept): say so again.
                if !handed_over && self.shared.screen.share.sharing() {
                    self.send_voice_state();
                }
            } else if channel_id.is_none() && m.voice_channel.is_some() {
                m.voice_channel = None;
                // Not a switch we asked for (e.g. the channel was archived): we're out.
                let out_entirely = m.voice_wanted == prev;
                if out_entirely {
                    m.voice_wanted = None;
                    m.voice_stats = None;
                }
                m.speaking.clear();
                m.peer_states.clear();
                m.hints.clear();
                drop(m);
                self.voice.hangup_all();
                if out_entirely {
                    self.call_ended("Yap took you out of the channel (it may have been archived)");
                    self.share_stop();
                    self.stop_watching();
                    self.stop_audio_soon(sounds);
                    self.emit(CoreEvent::VoiceLeft);
                }
                self.shared.apply_gate();
            }
        } else if let Some(mine) = m.voice_channel.clone() {
            if channel_id.as_ref() == Some(&mine) && prev.as_ref() != Some(&mine) {
                let uid = m.sessions.get(&sid).map(|s| s.uid.clone()).unwrap_or_default();
                drop(m);
                self.voice.set_resumable(&sid, resume);
                // The same call back under a new session: no chimes (V7: nor when anyone's straight back).
                let mut quiet = resumed;
                if !resumed {
                    // A plain join by someone whose old connection we were keeping:
                    // they couldn't resume it (or their app can't), so it's dead weight now.
                    let theirs: Vec<String> = {
                        let m = self.shared.model.lock();
                        m.ghosts.keys().filter(|g| m.sessions.get(*g).is_some_and(|s| s.uid == uid)).cloned().collect()
                    };
                    for g in theirs {
                        self.drop_ghost(&g, true);
                        quiet = true;
                    }
                }
                if self.leave_chimes.remove(&uid).is_some() {
                    // Back within 5 s of losing their connection: neither chime.
                    quiet = true;
                }
                self.call_person(&sid, if resumed { "back" } else { "joined" }, "");
                if !quiet {
                    // Their own join chime (they offer to us once their join lands).
                    self.play_join_chime(&uid);
                }
            } else if prev.as_ref() == Some(&mine) && channel_id.as_ref() != Some(&mine) {
                let keep = lost && was_resumable && self.voice.has_peer(&sid);
                if keep {
                    // Their socket dropped but the call may well continue: keep it, quietly.
                    if let Some(s) = m.sessions.get_mut(&sid) {
                        s.voice = Some(crate::protocol::VoiceState { channel_id: mine.clone(), muted, deafened, screen: was_sharing, resume: true, prev: None });
                    }
                    drop(m);
                    self.call_person(&sid, "away", "");
                    self.hold_ghost(&sid);
                } else {
                    drop(m);
                    self.call_person(&sid, "left", if lost { "lost their connection" } else { "" });
                    let mut m = self.shared.model.lock();
                    let uid = m.sessions.get(&sid).map(|s| s.uid.clone()).unwrap_or_default();
                    m.peer_states.remove(&sid);
                    m.speaking.remove(&sid);
                    m.hints.remove(&sid);
                    drop(m);
                    self.voice.drop_peer(&sid);
                    self.sharer_stopped(&sid);
                    if lost {
                        self.defer_leave_chime(&uid);
                    } else {
                        self.leave_chime();
                    }
                }
            }
        }
        *self.shared.voice_peers.lock() = self.voice.peers();
        self.repaint();
    }

    pub(super) fn on_voice_event(&mut self, ev: VoiceEvent) {
        match ev {
            VoiceEvent::PeerState { sid, state } => {
                {
                    let mut m = self.shared.model.lock();
                    if state == "closed" {
                        m.peer_states.remove(&sid);
                    } else if self.voice.has_peer(&sid) {
                        m.peer_states.insert(sid.clone(), state.clone());
                    }
                }
                self.ghost_health(&sid, (state != "closed").then_some(state.as_str()));
                self.call_peer(&sid, Some(&state), None);
                if state == "connected" {
                    self.screen_peer_connected(&sid);
                }
                *self.shared.voice_peers.lock() = self.voice.peers();
                self.emit(CoreEvent::PeerState { sid, state });
                self.repaint();
            }
            VoiceEvent::Rebuild { sid } => {
                self.call_peer(&sid, None, Some((report::Count::Rebuilds, "connection rebuilt (we started over after 3 ICE restarts)".into())));
                let v = self.shared.model.lock().session_volume(&sid);
                self.voice.restart(&sid, v);
                *self.shared.voice_peers.lock() = self.voice.peers();
                self.emit(CoreEvent::Extra("ICE", format!("{sid} rebuilt after 3 restarts")));
            }
            VoiceEvent::IceRestart { sid, attempt } => self.call_peer(&sid, None, Some((report::Count::Restarts, format!("ICE restart (try {attempt})")))),
            VoiceEvent::FreshStart { sid } => self.call_peer(&sid, None, Some((report::Count::Rebuilds, "connection rebuilt (they started over)".into()))),
        }
    }

    pub(super) fn stop_audio_soon(&self, sounds: bool) {
        if sounds && self.shared.audio.output_running() {
            self.shared.audio.mixer.play_notes(&[783.99, 659.25, 523.25], 0.075, 0.11, 0.07, false);
        }
        let tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(450)).await;
            let _ = tx.send(Cmd::StopAudioIfIdle);
        });
    }

    /// Where the mic and speakers come from: the devices, or (headless runs and
    /// `YAP_DEBUG_AUDIO=silence`) a WAV / silence and nowhere — never a real device then.
    fn sources(&self) -> (InputSource, OutputSink) {
        let (input_dev, output_dev) = {
            let m = self.shared.model.lock();
            (m.config.input_device.clone(), m.config.output_device.clone())
        };
        let overrides = self.shared.audio_override.lock().clone();
        let speakers = *self.shared.speakers_override.lock();
        let source = match &overrides {
            Some((input, _)) if input == "-" => InputSource::Silence,
            Some((input, _)) => InputSource::Wav(input.clone()),
            None => InputSource::Device(input_dev),
        };
        let sink = match overrides {
            Some(_) if speakers => OutputSink::Device(output_dev),
            Some((_, record)) => OutputSink::Null(record),
            None => OutputSink::Device(output_dev),
        };
        (source, sink)
    }

    /// Start whichever of mic/speakers isn't running.
    pub(super) async fn prepare_audio(&self) {
        let audio = self.shared.audio.clone();
        let need_in = !audio.input_running();
        let need_out = !audio.output_running();
        if !need_in && !need_out {
            return;
        }
        let (source, sink) = self.sources();
        let result = tokio::task::spawn_blocking(move || {
            let mic_err = need_in.then(|| audio.start_input(source).err()).flatten();
            let out_err = need_out.then(|| audio.start_output(sink).err()).flatten();
            (need_in, mic_err, out_err)
        })
        .await;
        let mut m = self.shared.model.lock();
        if let Ok((need_in, mic_err, out_err)) = result {
            if need_in {
                m.mic_ok = mic_err.is_none();
                if let Some(e) = &mic_err {
                    m.toast(format!("{e} You can listen but not talk."));
                }
                m.mic_error = mic_err;
            }
            if let Some(e) = out_err {
                m.toast(e);
            }
        }
    }

    pub(super) async fn join_voice(&mut self, channel_id: String) {
        {
            let mut m = self.shared.model.lock();
            if m.voice_wanted.as_deref() == Some(channel_id.as_str()) {
                return;
            }
            if m.status != Status::Online {
                m.toast("Not connected yet.");
                drop(m);
                return self.repaint();
            }
            m.voice_wanted = Some(channel_id.clone());
        }
        self.repaint();
        let (server, key) = {
            let m = self.shared.model.lock();
            (m.config.server.clone(), m.config.key.clone())
        };
        let (s2, k2) = (server.clone(), key.clone());
        let ice = tokio::task::spawn_blocking(move || crate::http::ice(&s2, &k2)).await.unwrap_or_default();
        let relay = crate::http::has_relay(&ice);
        let relay_only = {
            let mut m = self.shared.model.lock();
            m.relay = Some(relay);
            m.config.relay_only && relay
        };
        self.voice.set_ice(ice, relay_only);
        // ICE restarts fetch it again (relay credentials can expire).
        self.voice.set_ice_fetch(Arc::new(move || crate::http::ice(&server, &key)));
        self.prepare_audio().await;
        self.shared.apply_gate();
        let (muted, deafened, still) = {
            let m = self.shared.model.lock();
            (m.config.muted || !m.mic_ok, m.config.deafened, m.voice_wanted.as_deref() == Some(channel_id.as_str()))
        };
        if !still {
            return; // changed their mind meanwhile
        }
        // `resume`: we keep calls across reconnects. `prev`/`proof`: this join continues our call from before the reconnect.
        let prev = self.resume.as_ref().filter(|r| r.channel == channel_id).map(|r| (r.sid.clone(), r.key.clone()));
        let screen = self.shared.screen.share.sharing();
        self.send(out::voice_join(&channel_id, muted, deafened, prev.as_ref().map(|(s, k)| (s.as_str(), k.as_str())), screen));
        self.repaint();
    }

    pub(super) fn leave_voice(&mut self) {
        let sounds = {
            let mut m = self.shared.model.lock();
            if m.voice_wanted.is_none() {
                return;
            }
            m.voice_wanted = None;
            m.voice_channel = None;
            m.speaking.clear();
            m.peer_states.clear();
            m.hints.clear();
            m.voice_stats = None;
            m.config.sounds
        };
        self.rejoining = false;
        self.end_resume();
        self.call_ended("you left");
        let ghosts: Vec<String> = self.shared.model.lock().ghosts.keys().cloned().collect();
        for g in ghosts {
            self.drop_ghost(&g, true);
        }
        self.leave_chimes.clear();
        self.send(out::voice_leave());
        self.voice.hangup_all();
        *self.shared.voice_peers.lock() = Vec::new();
        self.stop_audio_soon(sounds);
        self.shared.apply_gate();
        self.emit(CoreEvent::VoiceLeft);
        self.repaint();
    }

    pub(super) fn send_voice_state(&self) {
        let (in_voice, muted, deafened) = {
            let m = self.shared.model.lock();
            (m.voice_channel.is_some(), m.config.muted || !m.mic_ok, m.config.deafened)
        };
        if in_voice {
            self.send(out::voice_state(muted, deafened, self.shared.screen.share.sharing()));
        }
    }

    pub(super) async fn prepare_input_only(&self) {
        // Headless runs and YAP_DEBUG_AUDIO=silence never open a real mic, not even for a test.
        let (source, _) = self.sources();
        let audio = self.shared.audio.clone();
        let r = tokio::task::spawn_blocking(move || audio.start_input(source)).await;
        let mut m = self.shared.model.lock();
        match r {
            Ok(Ok(())) => {
                m.mic_ok = true;
                m.mic_error = None;
            }
            Ok(Err(e)) => {
                m.mic_ok = false;
                m.toast(format!("Could not open the microphone: {e}"));
                m.mic_error = Some(e);
            }
            Err(_) => {}
        }
    }
}

/// A `voice` broadcast, unpacked.
pub(super) struct VoiceMsg {
    pub sid: String,
    pub channel_id: Option<String>,
    pub muted: bool,
    pub deafened: bool,
    pub screen: bool,
    pub resume: bool,
    pub prev: Option<String>,
    pub lost: bool,
}
