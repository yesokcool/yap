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
use health::{HearRun, HintTrack};
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
        g.echo_cancellation.store(m.config.echo_cancellation, Ordering::Relaxed);
        // Hear yourself with nobody listening: push-to-talk acts like always-on meanwhile.
        g.test.store(matches!(m.hear, HearState::Recording { hold_key: false }), Ordering::Relaxed);
        self.ptt.set_key(&m.config.ptt_key);
        self.ptt.set_active(m.config.mode == InputMode::Ptt && (m.voice_wanted.is_some() || m.mic_test));
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
        last_hints: None,
        last_devices: None,
        devices_busy: Arc::new(AtomicBool::new(false)),
        hear: None,
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
    last_hints: Option<Instant>,
    last_devices: Option<Instant>,
    devices_busy: Arc<AtomicBool>,
    /// Settings › Voice › Hear yourself, while it records.
    hear: Option<HearRun>,
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
        // while, thinking we're only reconnecting), save settings.
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
                let failures = {
                    let mut m = self.shared.model.lock();
                    m.failures += 1;
                    m.failures
                };
                if unauthorized {
                    self.set_status(Status::BadKey, "The invite key changed. Paste the new invite link.");
                } else {
                    self.set_status(Status::Offline, &reason);
                    // Repeated failures without getting in: maybe the key changed.
                    if failures == 3 && !was_online {
                        let (server, key) = {
                            let m = self.shared.model.lock();
                            (m.config.server.clone(), m.config.key.clone())
                        };
                        if let Ok(Ok(check)) = tokio::task::spawn_blocking(move || crate::http::check(&server, &key)).await
                            && !check.ok
                        {
                            self.set_status(Status::BadKey, "The invite key changed. Paste the new invite link.");
                        }
                    }
                }
            }
            NetEvent::Msg(msg) => self.on_msg(*msg).await,
        }
    }

    /// Store a device token the server just issued; the PIN is no longer needed.
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
