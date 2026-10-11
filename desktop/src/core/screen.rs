//! Screen sharing in the core (PROTOCOL.md › Screen sharing): sharing ours
//! (`voice.state.screen`, sending it to whoever asks), watching someone's
//! (`signal {screen: "want" | "stop"}`), and the states the UI shows.

use super::*;
use crate::screen::Quality;
use crate::screen::capture::Source;

/// "Ana stopped sharing." stays this long, then the view closes itself.
const ENDED_FOR: Duration = Duration::from_secs(2);

impl Core {
    fn sharing(&self) -> bool {
        self.shared.screen.share.sharing()
    }

    /// Share Screen: start sending `source` to whoever asks.
    pub(super) fn share_start(&mut self, source: Source, quality: Quality, sound: bool) {
        if self.shared.model.lock().voice_channel.is_none() {
            // (Something picked in macOS's picker meanwhile: let it go.)
            crate::screen::capture::release(&source);
            self.shared.model.lock().toast("Join a voice channel to share your screen.");
            return self.repaint();
        }
        let sound = sound && crate::screen::capture::sound_supported();
        log::info!("screen: sharing {:?} ({}, sound {})", source.name, quality.label(), if sound { "on" } else { "off" });
        self.shared.screen.share.start(source.clone(), quality, sound);
        {
            let mut m = self.shared.model.lock();
            m.screen.sharing = Some(MyShare { source, quality, sound });
            m.config.share_quality = quality;
            m.config.share_sound = sound;
            m.config_dirty = true;
        }
        self.voice.screen_changed();
        self.send_voice_state();
        self.emit(CoreEvent::Extra("SCREEN", format!("sharing on ({})", quality.label())));
        self.repaint();
    }

    pub(super) fn share_stop(&mut self) {
        if !self.sharing() && self.shared.model.lock().screen.sharing.is_none() {
            return;
        }
        self.shared.screen.share.stop();
        {
            let mut m = self.shared.model.lock();
            m.screen.sharing = None;
            m.screen.watchers.clear();
        }
        // Who asked was for this share; the next one starts with nobody.
        self.voice.forget_screen_requests();
        self.voice.screen_changed();
        self.send_voice_state();
        self.emit(CoreEvent::Extra("SCREEN", "sharing off".into()));
        self.repaint();
    }

    pub(super) fn share_quality(&mut self, quality: Quality) {
        self.shared.screen.share.set_quality(quality);
        let mut m = self.shared.model.lock();
        if let Some(s) = m.screen.sharing.as_mut() {
            s.quality = quality;
        }
        m.config.share_quality = quality;
        m.config_dirty = true;
        drop(m);
        self.emit(CoreEvent::Extra("SCREEN", format!("quality {}", quality.label())));
        self.repaint();
    }

    pub(super) fn share_sound(&mut self, on: bool) {
        let on = on && crate::screen::capture::sound_supported();
        self.shared.screen.share.set_sound(on);
        {
            let mut m = self.shared.model.lock();
            if let Some(s) = m.screen.sharing.as_mut() {
                s.sound = on;
            }
            m.config.share_sound = on;
            m.config_dirty = true;
        }
        // The sound is its own track: viewers' connections add or drop it.
        self.voice.screen_changed();
        self.repaint();
    }

    /// Share something else (another window or screen) without stopping.
    pub(super) fn share_source(&mut self, source: Source) {
        if !self.sharing() {
            crate::screen::capture::release(&source);
            return;
        }
        self.shared.screen.share.set_source(source.clone());
        if let Some(s) = self.shared.model.lock().screen.sharing.as_mut() {
            s.source = source;
        }
        self.repaint();
    }

    /// Our share stopped by itself: say why, and stop for real.
    pub(super) fn on_share_event(&mut self, ev: crate::screen::ShareEvent) {
        match ev {
            crate::screen::ShareEvent::Ended(why) => {
                if self.shared.model.lock().screen.sharing.is_none() {
                    return;
                }
                self.share_stop();
                // ("Screen sharing was stopped from the macOS menu bar." says it all already.)
                let text = if why.starts_with("Screen sharing") { why } else { format!("Screen sharing stopped: {why}") };
                self.shared.model.lock().toast(text);
                self.repaint();
            }
        }
    }

    /// Watch Screen: join their voice channel if need be, and ask for their screen.
    pub(super) async fn watch_screen(&mut self, sid: String) {
        let (channel, uid, mine) = {
            let m = self.shared.model.lock();
            let Some(s) = m.sessions.get(&sid) else { return };
            let Some(v) = s.voice.as_ref().filter(|v| v.screen) else { return };
            (v.channel_id.clone(), s.uid.clone(), m.voice_wanted.clone())
        };
        if mine.as_deref() != Some(channel.as_str()) {
            self.join_voice(channel).await;
        }
        let previous = self.shared.model.lock().screen.watching.as_ref().map(|w| w.sid.clone()).filter(|w| *w != sid);
        if let Some(prev) = previous {
            self.send(out::signal_screen(&prev, false));
            self.shared.audio.remove_stream(&crate::voice::screen_key(&prev));
        }
        let volume = self.shared.model.lock().screen_volume(&uid);
        self.voice.set_screen_volume(&sid, volume);
        self.shared.screen.watch.set(Some(sid.clone()));
        self.shared.model.lock().screen.watching = Some(Watching { sid: sid.clone(), uid, ended: None });
        // Connected already? Ask now. Otherwise as soon as the connection is up.
        if self.voice.peer(&sid).is_some_and(|p| *p.state.lock() == "connected") {
            self.send(out::signal_screen(&sid, true));
        }
        self.emit(CoreEvent::Extra("SCREEN", format!("watching {sid}")));
        self.repaint();
    }

    pub(super) fn stop_watching(&mut self) {
        let Some(w) = self.shared.model.lock().screen.watching.take() else { return };
        self.send(out::signal_screen(&w.sid, false));
        self.shared.screen.watch.set(None);
        self.shared.audio.remove_stream(&crate::voice::screen_key(&w.sid));
        self.emit(CoreEvent::Extra("SCREEN", format!("stopped watching {}", w.sid)));
        self.repaint();
    }

    /// The share's sound volume for someone changed (config already updated).
    pub(super) fn screen_volume(&mut self, uid: &str) {
        let (sid, v) = {
            let m = self.shared.model.lock();
            (m.screen.watching.as_ref().filter(|w| w.uid == uid).map(|w| w.sid.clone()), m.screen_volume(uid))
        };
        if let Some(sid) = sid {
            self.voice.set_screen_volume(&sid, v);
        }
    }

    /// `signal {screen: "want" | "stop"}` from someone in our call.
    pub(super) fn on_screen_signal(&mut self, from: &str, want: bool) {
        let ok = {
            let m = self.shared.model.lock();
            m.voice_channel.is_some() && m.sessions.get(from).and_then(|s| s.voice.as_ref()).is_some_and(|v| Some(&v.channel_id) == m.voice_channel.as_ref())
        };
        if !ok {
            return;
        }
        self.voice.set_wants_screen(from, want);
        self.emit(CoreEvent::Extra("SCREEN", format!("{from} {}", if want { "wants our screen" } else { "is done with our screen" })));
        self.sync_watchers();
    }

    /// Someone's `voice` says they stopped sharing (or left the call).
    pub(super) fn sharer_stopped(&mut self, sid: &str) {
        let mut m = self.shared.model.lock();
        if let Some(w) = m.screen.watching.as_mut().filter(|w| w.sid == sid && w.ended.is_none()) {
            w.ended = Some(Instant::now());
            drop(m);
            self.emit(CoreEvent::Extra("SCREEN", format!("{sid} stopped sharing")));
            self.repaint();
        }
    }

    /// A connection came up: if it's the screen we're watching, ask (again).
    pub(super) fn screen_peer_connected(&mut self, sid: &str) {
        let watching = self.shared.model.lock().screen.watching.as_ref().is_some_and(|w| w.sid == sid && w.ended.is_none());
        if watching {
            self.send(out::signal_screen(sid, true));
        }
    }

    /// A session moved to a new id (resumed call): follow it.
    pub(super) fn screen_rekey(&mut self, from: &str, to: &str) {
        if let Some(w) = self.shared.model.lock().screen.watching.as_mut().filter(|w| w.sid == from) {
            w.sid = to.to_string();
        }
        self.sync_watchers();
    }

    /// "Watching: Ana, Ben": who asked, from the connections (gone ones drop out).
    fn sync_watchers(&mut self) {
        let sids = self.voice.watchers();
        let mut m = self.shared.model.lock();
        let mut list: Vec<String> = sids.into_iter().filter(|s| m.sessions.contains_key(s)).collect();
        list.sort_by_key(|s| m.sessions.get(s).map(|x| m.user_name(&x.uid).to_lowercase()).unwrap_or_default());
        if list != m.screen.watchers {
            m.screen.watchers = list.clone();
            drop(m);
            self.emit(CoreEvent::Extra("SCREEN", format!("watchers [{}]", list.join(", "))));
            self.repaint();
        }
    }

    /// Every 100 ms: watchers follow the connections; an ended share closes itself.
    pub(super) fn tick_screen(&mut self) {
        if self.sharing() {
            self.sync_watchers();
        } else if !self.shared.model.lock().screen.watchers.is_empty() {
            self.shared.model.lock().screen.watchers.clear();
        }
        let ended = self.shared.model.lock().screen.watching.as_ref().and_then(|w| w.ended).is_some_and(|t| t.elapsed() >= ENDED_FOR);
        if ended {
            self.stop_watching();
        }
        // Repaint while watching, so "paused" shows up without anything else happening.
        if self.shared.model.lock().screen.watching.is_some() {
            self.repaint_soon();
        }
    }

    fn repaint_soon(&self) {
        // (Pictures repaint by themselves; this only matters for the state words.)
        let due = self.shared.screen.watch.last_at().is_none_or(|t| t.elapsed() > Duration::from_millis(2900));
        if due {
            self.repaint();
        }
    }
}
