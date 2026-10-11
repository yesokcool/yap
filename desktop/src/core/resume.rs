//! Keeping calls across reconnects (PROTOCOL.md › Voice › Keeping calls
//! across reconnects; the web's half is in web/js/app.js): when our socket
//! drops mid-call we keep every peer connection and queue their signals,
//! then rejoin with `prev` + `proof` and move each connection to the new
//! session ids. When someone else's socket drops we keep their connection
//! for a while ("reconnecting…") and hold back chimes.

use super::*;

/// Give up keeping our call after this long without the socket…
pub(super) const RESUME_FOR: Duration = Duration::from_secs(20);
/// …and keep someone else's this long while theirs is down.
const GHOST_FOR: Duration = Duration::from_secs(30);
/// A kept connection that's been down this long is let go.
const GHOST_DOWN_FOR: Duration = Duration::from_secs(5);
/// A lost connection's leave chime waits this long (no chimes if they're straight back).
const LEAVE_CHIME_AFTER: Duration = Duration::from_secs(5);
/// Kept connections not `connected` this long after a handover are rebuilt.
const HANDOVER_CHECK: Duration = Duration::from_secs(4);

/// Our call while the socket is down.
pub(super) struct Resume {
    /// Our session before the drop, and its resume key (the proof).
    pub sid: String,
    pub key: String,
    pub at: Instant,
    pub channel: String,
}

impl Core {
    /// Send signals the voice engine hands back (queued ones, after a resume).
    pub(super) fn send_signals(&self, signals: Vec<(String, SignalData)>) {
        for (to, data) in signals {
            self.send(out::signal(&to, &data));
        }
    }

    /// Our socket just dropped. In a call, on a server that hands out resume
    /// keys: keep it (audio is peer to peer and doesn't need the socket).
    /// Returns whether the call is kept.
    pub(super) fn keep_call(&mut self) -> bool {
        if self.resume.is_some() {
            return true;
        }
        let (channel, sid) = {
            let m = self.shared.model.lock();
            (m.voice_channel.clone(), m.sid.clone())
        };
        let (Some(channel), Some(sid), Some(key)) = (channel, sid, self.resume_key.take()) else { return false };
        log::info!("socket dropped mid-call: keeping the call (up to {} s)", RESUME_FOR.as_secs());
        self.call_note("Your connection to Yap dropped; keeping the call (audio goes straight between you)");
        self.old_sids.insert(sid.clone());
        self.resume = Some(Resume { sid, key, at: Instant::now(), channel });
        self.voice.suspend();
        self.shared.model.lock().resuming = true;
        true
    }

    pub(super) fn end_resume(&mut self) {
        self.resume = None;
        self.shared.model.lock().resuming = false;
    }

    /// `ready` after a drop we kept the call through: whoever's connection we
    /// kept but isn't in this list yet may be reconnecting too (a deploy drops
    /// everyone): keep them as ghosts. Anyone already back under a new sid:
    /// move over. Then let go of ghosts whose apps never resume.
    pub(super) fn resume_on_ready(&mut self, before: &HashMap<String, Session>) {
        if self.resume.is_none() {
            return;
        }
        let channel = self.resume.as_ref().map(|r| r.channel.clone()).unwrap_or_default();
        for sid in self.voice.sids() {
            let missing = !self.shared.model.lock().sessions.contains_key(&sid);
            if missing {
                let s = before.get(&sid).cloned().unwrap_or_else(|| Session {
                    sid: sid.clone(),
                    uid: String::new(),
                    client: String::new(),
                    voice: None,
                    view: None,
                    watching: false,
                });
                let mut s = s;
                if s.voice.is_none() {
                    s.voice = Some(VoiceState { channel_id: channel.clone(), muted: false, deafened: false, screen: false, resume: true, prev: None });
                }
                self.shared.model.lock().sessions.insert(sid.clone(), s);
                self.hold_ghost(&sid);
            }
        }
        let sessions: Vec<String> = self.shared.model.lock().sessions.keys().cloned().collect();
        for sid in sessions {
            self.adopt_resumed(&sid);
        }
        let ghosts: Vec<String> = self.shared.model.lock().ghosts.keys().cloned().collect();
        for sid in ghosts {
            let (resumable, back, uid, obs) = {
                let m = self.shared.model.lock();
                let ghost = m.sessions.get(&sid);
                let uid = ghost.map(|g| g.uid.clone()).unwrap_or_default();
                let ch = ghost.and_then(|g| g.voice.as_ref()).map(|v| v.channel_id.clone());
                // (Their OBS link in the channel isn't them back.)
                let back = m.sessions.values().any(|s| s.uid == uid && !s.is_obs() && !m.ghosts.contains_key(&s.sid) && s.voice.as_ref().map(|v| &v.channel_id) == ch.as_ref());
                (ghost.and_then(|g| g.voice.as_ref()).is_some_and(|v| v.resume), back, uid, ghost.is_some_and(Session::is_obs))
            };
            if !resumable || back {
                self.drop_ghost(&sid, true);
                if !back && !obs {
                    self.defer_leave_chime(&uid);
                }
            }
        }
    }

    /// A session that continues an earlier one's call (`voice.prev`, checked
    /// by the server): move our connection — and everything we keep by sid —
    /// over to it. Returns whether there was a connection (or a ghost) to move.
    pub(super) fn adopt_resumed(&mut self, sid: &str) -> bool {
        let (prev, uid, old_uid) = {
            let m = self.shared.model.lock();
            let Some(s) = m.sessions.get(sid) else { return false };
            let Some(prev) = s.voice.as_ref().and_then(|v| v.prev.clone()) else { return false };
            (prev.clone(), s.uid.clone(), m.sessions.get(&prev).map(|o| o.uid.clone()))
        };
        if prev == sid || Some(&prev) == self.shared.model.lock().sid.as_ref() || self.old_sids.contains(&prev) {
            return false;
        }
        // The server checked it's the same account; check again, cheaply.
        if old_uid.is_some_and(|u| !u.is_empty() && u != uid) {
            return false;
        }
        let had = self.shared.model.lock().ghosts.contains_key(&prev) || self.voice.has_peer(&prev);
        if !had {
            return false;
        }
        log::info!("{prev} resumed the call as {sid}: moving the connection over");
        let resumable = self.shared.model.lock().sessions.get(sid).and_then(|s| s.voice.as_ref()).is_some_and(|v| v.resume);
        {
            let mut m = self.shared.model.lock();
            m.ghosts.remove(&prev);
            m.sessions.remove(&prev);
            m.speaking.remove(&prev);
            if let Some(st) = m.peer_states.remove(&prev) {
                m.peer_states.insert(sid.to_string(), st);
            }
            if let Some(h) = m.hints.remove(&prev)
                && h != Hint::Reconnecting
            {
                m.hints.insert(sid.to_string(), h);
            }
        }
        if let Some(signals) = self.voice.rekey(&prev, sid) {
            self.send_signals(signals);
        }
        self.screen_rekey(&prev, sid);
        self.voice.set_resumable(sid, resumable);
        *self.shared.voice_peers.lock() = self.voice.peers();
        self.emit(CoreEvent::Extra("RESUME", format!("{prev} → {sid}")));
        true
    }

    /// Keep someone's connection while their socket is gone (they may resume):
    /// up to 30 s, or until the connection fails or has been down for 5 s.
    pub(super) fn hold_ghost(&mut self, sid: &str) {
        let state = self.voice.peer(sid).map(|p| p.state.lock().clone());
        {
            let mut m = self.shared.model.lock();
            if let Some(s) = m.sessions.get_mut(sid) {
                s.view = None;
                s.watching = false;
            }
            m.ghosts.insert(sid.to_string(), Ghost { since: Instant::now(), down_since: None });
            m.hints.insert(sid.to_string(), Hint::Reconnecting);
        }
        self.emit(CoreEvent::Extra("GHOST", format!("{sid} kept (reconnecting…)")));
        self.ghost_health(sid, state.as_deref());
    }

    /// A kept connection's state changed: failed/closed → let go; down → 5 s to come back.
    pub(super) fn ghost_health(&mut self, sid: &str, state: Option<&str>) {
        let mut m = self.shared.model.lock();
        let Some(g) = m.ghosts.get_mut(sid) else { return };
        match state {
            None | Some("failed") | Some("closed") => {
                drop(m);
                self.drop_ghost(sid, false);
            }
            Some("connected") => g.down_since = None,
            _ => {
                g.down_since.get_or_insert(Instant::now());
            }
        }
    }

    /// They didn't come back: hang up on the kept connection (and chime, unless
    /// `quiet` or it's an OBS link, which listens without showing up).
    pub(super) fn drop_ghost(&mut self, sid: &str, quiet: bool) {
        if self.shared.model.lock().ghosts.contains_key(sid) {
            self.call_person(sid, "left", if quiet { "their old connection, closed" } else { "they didn't come back" });
        }
        let (obs, uid) = {
            let mut m = self.shared.model.lock();
            if m.ghosts.remove(sid).is_none() {
                return;
            }
            m.hints.remove(sid);
            m.speaking.remove(sid);
            m.peer_states.remove(sid);
            m.sessions.remove(sid).map(|s| (s.is_obs(), s.uid)).unwrap_or_default()
        };
        self.voice.drop_peer(sid);
        *self.shared.voice_peers.lock() = self.voice.peers();
        self.emit(CoreEvent::Extra("GHOST", format!("{sid} let go")));
        let still_here = {
            let m = self.shared.model.lock();
            m.voice_channel.as_ref().is_some_and(|ch| m.sessions.values().any(|s| s.uid == uid && !s.is_obs() && s.voice.as_ref().is_some_and(|v| &v.channel_id == ch)))
        };
        if !quiet && !obs && !still_here {
            self.leave_chime();
        }
        self.repaint();
    }

    /// A connection lost without a goodbye: chime in 5 s, unless they're back by then (V7).
    pub(super) fn defer_leave_chime(&mut self, uid: &str) {
        if uid.is_empty() {
            return self.leave_chime();
        }
        self.leave_chimes.insert(uid.to_string(), Instant::now());
    }

    /// Every tick: give up on our call, let ghosts go, play held-back leave
    /// chimes, rebuild kept connections that didn't come back after a handover.
    pub(super) fn tick_resume(&mut self) {
        if self.resume.as_ref().is_some_and(|r| r.at.elapsed() > RESUME_FOR) {
            log::info!("not back in voice within {} s: hanging up", RESUME_FOR.as_secs());
            self.call_note(format!("Gave up keeping the call: no connection to Yap for {} s", RESUME_FOR.as_secs()));
            self.end_resume();
            let ghosts: Vec<String> = self.shared.model.lock().ghosts.keys().cloned().collect();
            for sid in ghosts {
                self.drop_ghost(&sid, true);
            }
            self.voice.hangup_all();
            *self.shared.voice_peers.lock() = Vec::new();
            {
                let mut m = self.shared.model.lock();
                m.peer_states.clear();
                m.speaking.clear();
                m.hints.clear();
            }
            self.repaint();
        }
        let expired: Vec<String> = {
            let m = self.shared.model.lock();
            m.ghosts.iter().filter(|(_, g)| g.since.elapsed() > GHOST_FOR || g.down_since.is_some_and(|d| d.elapsed() > GHOST_DOWN_FOR)).map(|(s, _)| s.clone()).collect()
        };
        for sid in expired {
            self.drop_ghost(&sid, false);
        }
        let due: Vec<String> = self.leave_chimes.iter().filter(|(_, at)| at.elapsed() > LEAVE_CHIME_AFTER).map(|(u, _)| u.clone()).collect();
        for uid in due {
            self.leave_chimes.remove(&uid);
            if self.shared.model.lock().voice_channel.is_some() {
                self.leave_chime();
            }
        }
        if self.handover_check.as_ref().is_some_and(|(at, _)| at.elapsed() > HANDOVER_CHECK)
            && let Some((_, sids)) = self.handover_check.take()
        {
            for sid in sids {
                let Some(p) = self.voice.peer(&sid) else { continue };
                if *p.state.lock() != "connected" && p.resumable.load(Ordering::Relaxed) {
                    log::info!("peer {sid}: not back 4 s after the handover; starting over");
                    self.call_peer(&sid, None, Some((report::Count::Rebuilds, "connection rebuilt (not back 4 s after our reconnect)".into())));
                    let v = self.shared.model.lock().session_volume(&sid);
                    self.voice.restart(&sid, v);
                }
            }
            *self.shared.voice_peers.lock() = self.voice.peers();
        }
    }
}
