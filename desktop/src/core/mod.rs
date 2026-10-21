//! The client core: owns the socket, the protocol state and the voice engine.
//! A port of the state logic in web/js/app.js. The GUI and the headless mode
//! both drive it with `Cmd`s and read `Model` (behind a mutex).
//!
//! Here: the shared state, starting the core, its loop and connection.
//! The model is in `model`, commands and events in `cmd`, plain text
//! helpers in `text`; server messages are handled in `msg`, commands in
//! `cmds`, and `vault`, `search` and `voice` hold those parts of the core.

mod cmd;
mod cmds;
mod health;
mod model;
mod msg;
mod outbox;
pub mod report;
mod resume;
mod screen;
mod search;
mod text;
mod vault;
mod voice;

pub use cmd::*;
pub use model::*;
pub use text::*;
use health::{HearRun, HintTrack, OwnTrack};
pub use health::{OWN_WEAK_TEXT, own_weak_title};
use resume::Resume;
use voice::{Chime, VoiceMsg};

use crate::audio::{AudioEngine, InputSource, OutputSink};
use crate::config::{Config, InputMode};
use crate::net::{self, NetEvent, NetHandle};
use crate::protocol::{Channel, Emoji, Hang, ImageRef, Invite, Link, Message, ModLogEntry, QuietHours, ServerMsg, Session, SignalData, Sound, UnreadCount, User, VoiceState, out};
use crate::voice::{VoiceEngine, VoiceEvent};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub const TYPING_MS: u64 = 6000;
/// When the app started (debug aids count from it).
static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
const TYPING_SEND_EVERY: Duration = Duration::from_millis(3500);
const TOAST_MS: u64 = 4500;

pub struct Shared {
    pub model: Mutex<Model>,
    pub audio: Arc<AudioEngine>,
    pub ptt: Arc<crate::ptt::Ptt>,
    pub repaint: Arc<dyn Fn() + Send + Sync>,
    pub config_path: PathBuf,
    /// Headless: input/output overrides.
    pub audio_override: Mutex<Option<(String, Option<String>)>>,
    /// Headless device test: real speakers even with a WAV mic.
    pub speakers_override: Mutex<bool>,
    pub voice_peers: Mutex<Vec<Arc<crate::voice::PeerInfo>>>,
    /// Screen sharing: our share's encoder and the picture we're watching.
    pub screen: Arc<crate::screen::Screen>,
    /// The call report: the call we're in (or the last one), and the last three kept.
    pub calls: Mutex<report::Calls>,
}

impl Shared {
    pub fn new(config: Config, config_path: PathBuf, repaint: Arc<dyn Fn() + Send + Sync>) -> Arc<Self> {
        let token = crate::config::Tokens::load(&config_path).servers.get(&config.server).cloned();
        let audio = AudioEngine::new(repaint.clone());
        let ptt = crate::ptt::Ptt::new(audio.gate.clone(), repaint.clone());
        let screen = crate::screen::Screen::new();
        screen.watch.set_repaint(repaint.clone());
        let shared = Arc::new(Self {
            model: Mutex::new({
                let mut m = Model::new(config);
                m.token = token;
                m
            }),
            audio,
            ptt,
            repaint,
            audio_override: Mutex::new(None),
            speakers_override: Mutex::new(false),
            voice_peers: Mutex::new(Vec::new()),
            screen,
            calls: Mutex::new(report::Calls::load(Some(report::reports_path(&config_path)))),
            config_path,
        });
        shared.apply_gate();
        shared
    }

    pub fn apply_gate(&self) {
        let m = self.model.lock();
        let g = &self.audio.gate;
        g.muted.store(m.config.muted, Ordering::Relaxed);
        g.deafened.store(m.config.deafened, Ordering::Relaxed);
        g.set_mode(m.config.mode);
        g.vad_threshold.set(m.config.vad_threshold);
        g.noise_suppression.store(m.config.noise_suppression, Ordering::Relaxed);
        g.auto_gain.store(m.config.auto_gain, Ordering::Relaxed);
        g.mic_gain.set(m.config.mic_volume);
        g.echo_cancellation.store(m.config.echo_cancellation, Ordering::Relaxed);
        g.ptt_click.store(m.config.ptt_click, Ordering::Relaxed);
        self.audio.mixer.lower_media.store(m.config.lower_media, Ordering::Relaxed);
        // Hear yourself with nobody listening: push-to-talk acts like always-on meanwhile.
        g.test.store(matches!(m.hear, HearState::Recording { hold_key: false }), Ordering::Relaxed);
        let c = &m.config;
        self.ptt.set_keys(&c.ptt_key, &c.mute_key, &c.deafen_key, &c.push_to_mute_key);
        let in_call = m.voice_wanted.is_some();
        self.ptt.set_active(c.mode == InputMode::Ptt && (in_call || m.mic_test), in_call);
    }

    /// Remember (or forget) this server's device token.
    pub fn store_token(&self, token: Option<&str>) {
        let server = {
            let mut m = self.model.lock();
            m.token = token.map(str::to_string);
            m.config.server.clone()
        };
        let mut t = crate::config::Tokens::load(&self.config_path);
        match token {
            Some(tok) => {
                t.servers.insert(server, tok.to_string());
            }
            None => {
                t.servers.remove(&server);
            }
        }
        t.save(&self.config_path);
    }

    pub fn save_config(&self) {
        let cfg = {
            let mut m = self.model.lock();
            m.config_dirty = false;
            m.config.clone()
        };
        cfg.save(&self.config_path);
    }
}

/// What `yap --remote` gets (remote.rs): the command goes to the core, and
/// the answer says how things will be (mute and deafen, in a call or not).
#[cfg_attr(not(unix), allow(dead_code))]
pub fn remote_handler(shared: Arc<Shared>, cmd: mpsc::UnboundedSender<Cmd>) -> crate::remote::Handler {
    Arc::new(move |r| {
        let (muted, deafened, in_call) = {
            let m = shared.model.lock();
            (m.config.muted, m.config.deafened, m.voice_wanted.is_some())
        };
        let (muted, deafened) = crate::remote::next(r, muted, deafened);
        if r != crate::remote::Remote::Status {
            let _ = cmd.send(Cmd::Remote(r));
        }
        crate::remote::reply(muted, deafened, in_call)
    })
}

pub fn start(shared: Arc<Shared>, observer: Option<mpsc::UnboundedSender<CoreEvent>>) -> mpsc::UnboundedSender<Cmd> {
    start_with_done(shared, observer, None)
}

/// Like `start`; `done` is signalled once the core has shut down after `Cmd::Quit`.
pub fn start_with_done(
    shared: Arc<Shared>,
    observer: Option<mpsc::UnboundedSender<CoreEvent>>,
    done: Option<std::sync::mpsc::Sender<()>>,
) -> mpsc::UnboundedSender<Cmd> {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (signal_tx, signal_rx) = mpsc::unbounded_channel();
    let _ = *START;
    let (voice_tx, voice_rx) = mpsc::unbounded_channel();
    let bitrate = shared.model.lock().config.bitrate;
    let voice = VoiceEngine::new(shared.audio.clone(), shared.screen.clone(), bitrate, signal_tx, voice_tx);
    let (share_tx, share_rx) = mpsc::unbounded_channel();
    shared.screen.share.set_events(share_tx);
    // The mute and deafen keys (Settings › Voice) toggle like the buttons.
    let keys_tx = cmd_tx.clone();
    shared.ptt.set_on_toggle(Arc::new(move |a| {
        let _ = keys_tx.send(match a {
            crate::ptt::Action::Mute => Cmd::ToggleMute,
            crate::ptt::Action::Deafen => Cmd::ToggleDeafen,
            _ => return,
        });
    }));
    let core = Core {
        shared,
        observer,
        cmd_tx: cmd_tx.clone(),
        net: None,
        net_rx: None,
        online: false,
        voice,
        signal_rx,
        voice_rx,
        share_rx,
        last_typing_sent: None,
        last_stats: None,
        last_save: Instant::now(),
        last_mc_poll: None,
        last_update_check: None,
        seek: None,
        pcm: Arc::new(Mutex::new(HashMap::new())),
        handoff: None,
        resume: None,
        resume_key: None,
        old_sids: HashSet::new(),
        leave_chimes: HashMap::new(),
        handover_check: None,
        rejoining: false,
        hint_track: HashMap::new(),
        own: OwnTrack::default(),
        last_hints: None,
        last_devices: None,
        devices_busy: Arc::new(AtomicBool::new(false)),
        hear: None,
        nudge_seen: 0,
        last_nudge: None,
        ptt_latched: false,
        last_report: None,
        debug_fail: std::env::var("YAP_DEBUG_AUDIO_FAIL").ok().filter(|_| cfg!(debug_assertions)),
        seal_wait: None,
        outbox: outbox::Outbox::default(),
    };
    tokio::spawn(async move {
        core.run(cmd_rx).await;
        if let Some(d) = done {
            let _ = d.send(());
        }
    });
    cmd_tx
}

struct Core {
    shared: Arc<Shared>,
    observer: Option<mpsc::UnboundedSender<CoreEvent>>,
    cmd_tx: mpsc::UnboundedSender<Cmd>,
    net: Option<NetHandle>,
    net_rx: Option<mpsc::UnboundedReceiver<NetEvent>>,
    online: bool,
    voice: VoiceEngine,
    signal_rx: mpsc::UnboundedReceiver<(u64, SignalData)>,
    voice_rx: mpsc::UnboundedReceiver<VoiceEvent>,
    /// Our screen share stopping by itself (a window closed…).
    share_rx: mpsc::UnboundedReceiver<crate::screen::ShareEvent>,
    last_typing_sent: Option<Instant>,
    last_stats: Option<(Instant, u64, u64)>,
    last_save: Instant,
    last_mc_poll: Option<Instant>,
    last_update_check: Option<Instant>,
    /// Looking for a message: (channel, id, older pages loaded so far).
    seek: Option<(String, String, u8)>,
    /// Soundboard clips and chimes, decoded (mono 48 kHz), by sound id.
    pcm: Arc<Mutex<HashMap<String, Arc<Vec<f32>>>>>,
    /// Waiting for a `device.handoff` pass to open the web at this place.
    handoff: Option<(Option<String>, Instant)>,
    /// Our call while the socket is down (PROTOCOL.md › Voice › Keeping calls across reconnects).
    resume: Option<Resume>,
    /// This session's resume key (`ready.resumeKey`).
    resume_key: Option<String>,
    /// Our own earlier session ids: after a reconnect the server may still list
    /// a half-dead old one for a while. That's us, not someone to call or
    /// show (and the server dropping it isn't someone leaving).
    old_sids: HashSet<String>,
    /// uid → a leave chime held back 5 s (a lost connection; none if they're straight back).
    leave_chimes: HashMap<String, Instant>,
    /// Kept connections to check 4 s after our call was handed over.
    handover_check: Option<(Instant, Vec<String>)>,
    /// Coming back into voice after a drop: no join blip for ourselves.
    rejoining: bool,
    /// Connection hints' numbers, per connection.
    hint_track: HashMap<u64, HintTrack>,
    /// Your own connection (what the others report losing of your voice).
    own: OwnTrack,
    last_hints: Option<Instant>,
    last_devices: Option<Instant>,
    devices_busy: Arc<AtomicBool>,
    /// Settings › Voice › Hear yourself, while it records.
    hear: Option<HearRun>,
    /// Talking while muted: the gate's last report we looked at, and the last nudge.
    nudge_seen: u64,
    last_nudge: Option<Instant>,
    /// Push-to-talk latched, as last noted in the call report.
    ptt_latched: bool,
    /// The call report's last reading of the connections.
    last_report: Option<Instant>,
    /// Debug builds: `YAP_DEBUG_AUDIO_FAIL=mic|speakers[@secs]` fakes a stream error once.
    debug_fail: Option<String>,
    /// Earlier notes sealed and handed back, until the server confirms them.
    seal_wait: Option<vault::SealWait>,
    /// Our sends the server hasn't taken yet (refusals, `slow_down` retries).
    outbox: outbox::Outbox,
}

async fn recv_opt<T>(rx: &mut Option<mpsc::UnboundedReceiver<T>>) -> Option<T> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// What the sign-in screen says when the invite key is wrong.
const BAD_KEY: &str = "The invite key changed. Paste the new invite link.";

/// What a closed socket says about our invite key.
#[derive(Debug, PartialEq, Eq)]
enum KeyVerdict {
    /// It's wrong: ask for the new link (the app keeps trying meanwhile).
    Wrong,
    /// Maybe: ask `/api/check`.
    Ask,
    /// Nothing to do with the key: just offline for now.
    Fine,
}

/// `unauthorized`: a 401 on the upgrade (a server from before resettable
/// links) or a 4003 close (a newer one: the key was wrong and the socket
/// didn't sign in with a device token). Three failures in a row without ever
/// getting in may also be a changed key, but only without a device token:
/// since the main link can be reset, a newer server lets a device token in
/// whatever key came with it, so `/api/check` saying "wrong key" doesn't stop
/// a signed-in device (it saves the new key from `ready`). PROTOCOL.md › HTTP ›
/// Resetting the main link.
fn key_verdict(unauthorized: bool, failures: u32, was_online: bool, has_token: bool) -> KeyVerdict {
    if unauthorized {
        KeyVerdict::Wrong
    } else if failures == 3 && !was_online && !has_token {
        KeyVerdict::Ask
    } else {
        KeyVerdict::Fine
    }
}

/// A key in the logs: never the key itself, the start of its SHA-256.
pub fn key_fingerprint(key: &str) -> String {
    use sha2::Digest;
    let hash = sha2::Sha256::digest(key.as_bytes());
    format!("sha256:{}", hash.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>())
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Core {
    fn emit(&self, ev: CoreEvent) {
        if let Some(o) = &self.observer {
            let _ = o.send(ev);
        }
    }

    fn repaint(&self) {
        (self.shared.repaint)();
    }

    fn send(&self, v: serde_json::Value) -> bool {
        if !self.online {
            return false;
        }
        match &self.net {
            Some(n) => n.out.send(v.to_string()).is_ok(),
            None => false,
        }
    }

    /// A `send` frame, through the outbox (`outbox.rs`): now, or when its turn
    /// comes after a `slow_down`. `restore`: the text to give back if it's refused;
    /// `again_on_reconnect`: it has a grey copy that goes again after a reconnect.
    fn send_message(&mut self, nonce: String, frame: serde_json::Value, restore: Option<outbox::Restore>, again_on_reconnect: bool) {
        if self.outbox.add(nonce.clone(), frame.clone(), restore, again_on_reconnect) && !self.send(frame) {
            self.outbox.unsent(&nonce);
        }
    }

    /// Going one at a time after a `slow_down`: the next one, when it's time.
    fn tick_outbox(&mut self) {
        if let Some((nonce, frame)) = self.outbox.due(Instant::now()) {
            log::info!("sending {nonce} (one at a time: the server asked us to slow down)");
            if !self.send(frame) {
                self.outbox.unsent(&nonce);
            }
        }
    }

    /// `error {re: "send"}`: about our oldest unanswered send. True when there's
    /// nothing to show (it goes again in a moment).
    fn send_refused(&mut self, code: &str) -> bool {
        match self.outbox.refused(code, Instant::now()) {
            Some(outbox::Refused::Retrying { nonce, retries }) => {
                log::info!("send {nonce}: slow down; again in a moment ({retries} of {})", outbox::MAX_RETRIES);
                self.emit(CoreEvent::Extra("SEND", format!("retry {nonce} {retries}")));
                true
            }
            Some(outbox::Refused::Failed { nonce, restore }) => {
                let back = restore.filter(|r| !r.text.trim().is_empty());
                {
                    let mut m = self.shared.model.lock();
                    m.pending.retain(|(k, _)| *k != nonce);
                    if let Some(r) = &back {
                        m.composer_restore.push((r.channel.clone(), r.text.clone()));
                    }
                }
                log::info!("send {nonce} refused ({code}){}", if back.is_some() { "; its text goes back in the message box" } else { "" });
                self.emit(CoreEvent::Extra("SEND", format!("refused {nonce} {code}{}", back.map(|r| format!(" text back: {:?}", r.text)).unwrap_or_default())));
                self.repaint();
                false
            }
            None => false,
        }
    }

    async fn run(mut self, mut cmd_rx: mpsc::UnboundedReceiver<Cmd>) {
        self.connect();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                ev = recv_opt(&mut self.net_rx) => match ev {
                    Some(ev) => self.on_net(ev).await,
                    None => self.net_rx = None,
                },
                cmd = cmd_rx.recv() => match cmd {
                    Some(Cmd::Quit) | None => break,
                    Some(cmd) => self.on_cmd(cmd).await,
                },
                Some((id, data)) = self.signal_rx.recv() => {
                    // Straight out, or queued while the socket is down (a kept call).
                    if let Some((to, data)) = self.voice.route(id, data) {
                        self.send(out::signal(&to, &data));
                    }
                }
                Some(ev) = self.voice_rx.recv() => self.on_voice_event(ev),
                Some(ev) = self.share_rx.recv() => self.on_share_event(ev),
                _ = tick.tick() => self.on_tick(),
            }
        }
        // Shutting down: leave voice politely (or others keep our call for a
        // while, thinking we're only reconnecting), save settings (with the
        // call as of now, for "You were in …" at the next launch).
        self.shared.model.lock().note_call(now_ms());
        self.call_ended("you quit Yap");
        self.say_goodbye_in_voice().await;
        self.shared.screen.share.stop();
        self.voice.hangup_all();
        self.shared.audio.stop();
        if let Some(n) = &self.net {
            n.stop.notify_waiters();
        }
        self.shared.save_config();
    }

    /// Leaving for real (quitting, another server, signing in again): say so,
    /// so nobody keeps our call waiting for us to come back.
    async fn say_goodbye_in_voice(&mut self) {
        self.call_ended("you left");
        if self.shared.model.lock().voice_wanted.is_some() && self.send(out::voice_leave()) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.end_resume();
        self.leave_chimes.clear();
        let mut m = self.shared.model.lock();
        m.ghosts.clear();
        m.hints.clear();
        m.own_weak = None;
    }

    fn connect(&mut self) {
        if let Some(n) = self.net.take() {
            n.stop.notify_waiters();
        }
        self.net_rx = None;
        self.online = false;
        let (server, key, can_hello) = {
            let m = self.shared.model.lock();
            (m.config.server.clone(), m.config.key.clone(), m.token.is_some() || m.auth.is_some())
        };
        if server.is_empty() {
            self.set_status(Status::Setup, "");
            return;
        }
        if !can_hello {
            self.set_status(Status::SignIn, "");
            return;
        }
        let (tx, rx) = mpsc::unbounded_channel();
        self.net = Some(net::spawn(server, key, tx));
        self.net_rx = Some(rx);
        self.set_status(Status::Connecting, "");
    }

    fn set_status(&self, status: Status, detail: &str) {
        {
            let mut m = self.shared.model.lock();
            m.status = status;
            m.status_detail = detail.to_string();
        }
        self.emit(CoreEvent::Status(status, detail.to_string()));
        self.repaint();
    }

    async fn on_net(&mut self, ev: NetEvent) {
        match ev {
            NetEvent::Open => {
                self.online = true;
                let hello = {
                    let m = self.shared.model.lock();
                    match (&m.token, &m.auth) {
                        (Some(t), _) => Some(out::hello_token(t)),
                        (None, Some(Auth::Login { name, pin })) => Some(out::hello_login(name, pin)),
                        (None, Some(Auth::Signup { name, pin, color })) => Some(out::hello_signup(name, pin, color)),
                        // (The pre-PIN uid-only hello is gone: the server answers pin_required.)
                        (None, None) => None,
                    }
                };
                match hello {
                    Some(h) => {
                        self.send(h);
                    }
                    None => self.set_status(Status::SignIn, ""),
                }
            }
            NetEvent::Closed { unauthorized, reason } => {
                let was_online = self.online;
                self.online = false;
                self.on_disconnect();
                let (failures, has_token) = {
                    let mut m = self.shared.model.lock();
                    m.failures += 1;
                    (m.failures, m.token.is_some())
                };
                match key_verdict(unauthorized, failures, was_online, has_token) {
                    KeyVerdict::Wrong => self.set_status(Status::BadKey, BAD_KEY),
                    KeyVerdict::Ask => {
                        self.set_status(Status::Offline, &reason);
                        let (server, key) = {
                            let m = self.shared.model.lock();
                            (m.config.server.clone(), m.config.key.clone())
                        };
                        if let Ok(Ok(check)) = tokio::task::spawn_blocking(move || crate::http::check(&server, &key)).await
                            && !check.ok
                        {
                            self.set_status(Status::BadKey, BAD_KEY);
                        }
                    }
                    KeyVerdict::Fine => self.set_status(Status::Offline, &reason),
                }
            }
            NetEvent::Msg(msg) => self.on_msg(*msg).await,
        }
    }

    /// The server's main invite key (`key` after the owner reset the main
    /// link, or `ready.key`): our stored link from now on, saved straight
    /// away, and the key every reconnect uses. True when it changed.
    fn save_main_key(&mut self, key: String) -> bool {
        if key.is_empty() {
            return false;
        }
        {
            let mut m = self.shared.model.lock();
            if m.config.key == key {
                return false;
            }
            m.config.key = key.clone();
            m.main_key_seq += 1;
            m.main_reset_error = None;
        }
        self.shared.save_config();
        if let Some(n) = &self.net {
            *n.key.lock() = key.clone();
        }
        log::info!("saved the server's main invite key");
        self.emit(CoreEvent::Extra("KEY", format!("saved {}", key_fingerprint(&key))));
        self.repaint();
        true
    }

    /// Store a device token the server just issued; the password is no longer needed.
    fn got_token(&mut self, token: Option<String>) {
        if let Some(t) = token.filter(|t| !t.is_empty()) {
            self.shared.store_token(Some(&t));
            self.shared.model.lock().auth = None;
            self.emit(CoreEvent::Token(t));
        }
    }

    fn check_update(&mut self, manual: bool) {
        self.last_update_check = Some(Instant::now());
        let server = self.shared.model.lock().config.server.clone();
        // No update key built in: this build never asks (Settings › About says so).
        if server.is_empty() || !crate::update::enabled() {
            return;
        }
        if manual {
            self.shared.model.lock().update = UpdateState::Checking;
            self.repaint();
        }
        let tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || crate::update::check(&server)).await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = tx.send(Cmd::UpdateChecked { result, manual });
        });
    }

    fn fetch_server_config(&self) {
        let (server, key) = {
            let m = self.shared.model.lock();
            (m.config.server.clone(), m.config.key.clone())
        };
        let tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            if let Ok(Ok(c)) = tokio::task::spawn_blocking(move || crate::http::server_config(&server, &key)).await {
                let _ = tx.send(Cmd::ServerConfig(c));
            }
        });
    }

    fn notifications_on(&self) -> bool {
        self.shared.model.lock().config.notifications
    }

    fn on_disconnect(&mut self) {
        // In a call? Keep it: audio is peer to peer and doesn't need the socket.
        // (Servers that predate this send no resume key: hang up as before.)
        let kept = self.keep_call();
        if !kept && self.shared.model.lock().voice_channel.is_some() {
            self.call_note("Your connection to Yap dropped (this server can't keep calls across that): hanging up until it's back");
        }
        {
            let mut m = self.shared.model.lock();
            self.rejoining |= m.voice_wanted.is_some();
            // (The server may still list this session after we're back: see `old_sids`.)
            if let Some(old) = m.sid.take() {
                self.old_sids.insert(old);
            }
            m.upload_token = None;
            m.voice_channel = None;
            m.speaking.clear();
            if !kept {
                m.peer_states.clear();
                m.hints.clear();
                m.own_weak = None;
            }
        }
        self.resume_key = None;
        if !kept {
            self.voice.hangup_all();
            *self.shared.voice_peers.lock() = Vec::new();
        }
        self.repaint();
    }

    fn on_tick(&mut self) {
        self.tick_screen();
        self.tick_outbox();
        self.tick_resume();
        self.check_sealed();
        self.tick_health();
        self.tick_report();
        // Debug builds: YAP_DEBUG_AUDIO_FAIL=mic|speakers[@secs] fakes a stream error once (the recovery path, no real devices).
        if let Some(what) = self.debug_fail.clone() {
            let (dev, at) = what.split_once('@').map(|(d, s)| (d.to_string(), s.parse().unwrap_or(5.0))).unwrap_or((what.clone(), 5.0));
            let mic = dev != "speakers";
            let running = if mic { self.shared.audio.input_running() } else { self.shared.audio.output_running() };
            if running && START.elapsed().as_secs_f64() >= at {
                self.debug_fail = None;
                log::info!("debug: faking a {dev} stream error");
                self.shared.audio.simulate_failure(mic);
            }
        }
        let mut changed = false;
        // Space: a voice that arrived after its pan was set (a new connection) comes from the right place too.
        {
            let m = self.shared.model.lock();
            for (sid, pan) in &m.pan {
                self.shared.audio.set_pan(sid, *pan);
            }
        }
        // Speaking rings: remote from decoded audio, local from the gate.
        let mut speaking: HashSet<String> = self.shared.audio.streams().iter().filter(|s| s.speaking()).map(|s| s.sid()).collect();
        let mut m = self.shared.model.lock();
        if let Some(sid) = &m.sid
            && self.shared.audio.gate.speaking.load(Ordering::Relaxed)
            && m.voice_channel.is_some()
        {
            speaking.insert(sid.clone());
        }
        if speaking != m.speaking {
            m.speaking = speaking;
            changed = true;
        }
        // The call we're in, for "You were in …" after a restart.
        m.note_call(now_ms());
        // Typing indicators expire.
        let now = Instant::now();
        for t in m.typing.values_mut() {
            let before = t.len();
            t.retain(|_, until| *until > now);
            changed |= t.len() != before;
        }
        let before = m.toasts.len();
        m.toasts.retain(|(_, at)| at.elapsed().as_millis() < TOAST_MS as u128);
        changed |= m.toasts.len() != before;
        // Voice stats every 2 s.
        if m.voice_channel.is_some() {
            if self.last_stats.is_none_or(|(at, _, _)| at.elapsed() >= Duration::from_secs(2)) {
                let peers = self.voice.peers();
                let sent: u64 = peers.iter().map(|p| p.bytes_sent.load(Ordering::Relaxed)).sum();
                let recv: u64 = self.shared.audio.streams().iter().map(|s| s.stats().bytes).sum();
                if let Some((at, s0, r0)) = self.last_stats {
                    let secs = at.elapsed().as_secs_f64().max(0.001);
                    let n = peers.len().max(1) as f64;
                    m.voice_stats = Some(VoiceStats {
                        send_kbps: ((sent.saturating_sub(s0)) as f64 * 8.0 / 1000.0 / secs / n) as u32,
                        recv_kbps: ((recv.saturating_sub(r0)) as f64 * 8.0 / 1000.0 / secs) as u32,
                        bitrate: peers.iter().map(|p| p.send_bitrate.load(Ordering::Relaxed)).min().unwrap_or(m.config.bitrate),
                    });
                    changed = true;
                }
                self.last_stats = Some((Instant::now(), sent, recv));
            }
        } else {
            self.last_stats = None;
        }
        let dirty = m.config_dirty;
        let mc_due = m.status == Status::Online
            && m.window_focused
            && m.server_config.as_ref().is_some_and(|c| c.minecraft.is_some())
            && self.last_mc_poll.is_none_or(|t| t.elapsed() >= Duration::from_secs(60));
        let (server, key) = (m.config.server.clone(), m.config.key.clone());
        drop(m);
        // No handoff pass came back: open the web anyway (it asks to sign in).
        if self.handoff.as_ref().is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(4))
            && let Some((place, _)) = self.handoff.take()
        {
            let url = self.web_url(place.as_deref(), None);
            self.shared.model.lock().open_url = Some(url);
            self.repaint();
        }
        // Updates: at launch (once online) and every 6 hours, if this build updates itself.
        let update_due = self.online && crate::update::enabled() && self.last_update_check.is_none_or(|t| t.elapsed() >= Duration::from_secs(6 * 3600));
        if update_due && self.shared.audio_override.lock().is_none() {
            self.check_update(false);
        }
        if mc_due {
            self.last_mc_poll = Some(Instant::now());
            let tx = self.cmd_tx.clone();
            tokio::spawn(async move {
                if let Ok(Ok(st)) = tokio::task::spawn_blocking(move || crate::http::minecraft(&server, &key)).await {
                    let _ = tx.send(Cmd::Minecraft(st));
                }
            });
        }
        if dirty && self.last_save.elapsed() > Duration::from_secs(1) {
            self.last_save = Instant::now();
            self.shared.save_config();
        }
        if changed {
            self.repaint();
        }
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;

    /// A wrong key stops at the sign-in screen only when it's certain (a 401
    /// or a 4003), or after three failures without a device token; a device
    /// that's signed in keeps knocking with its token whatever /api/check says.
    #[test]
    fn a_device_token_gets_past_a_stale_key() {
        assert_eq!(key_verdict(true, 1, false, true), KeyVerdict::Wrong, "a 4003 (or an older server's 401) is certain, token or not");
        assert_eq!(key_verdict(true, 1, true, false), KeyVerdict::Wrong);
        assert_eq!(key_verdict(false, 3, false, false), KeyVerdict::Ask, "three failures, no token: maybe the key changed");
        assert_eq!(key_verdict(false, 3, false, true), KeyVerdict::Fine, "a device token connects even when /api/check says the key is wrong");
        assert_eq!(key_verdict(false, 3, true, false), KeyVerdict::Fine, "it got in before: not the key");
        assert_eq!(key_verdict(false, 1, false, false), KeyVerdict::Fine);
        assert_eq!(key_verdict(false, 4, false, false), KeyVerdict::Fine, "asked once");
    }

    #[test]
    fn keys_never_reach_the_logs() {
        let f = key_fingerprint("testkey");
        assert!(f.starts_with("sha256:") && f.len() == 15 && !f.contains("testkey"), "{f}");
        assert_ne!(key_fingerprint("testkey"), key_fingerprint("testkey2"));
    }
}
