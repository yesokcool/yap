//! `yap --headless …`: no window, no audio devices. Connects, optionally says
//! something and joins voice with a WAV file as the mic, then logs events and
//! once a second the received audio (bytes, packets, decoded level).
//! Ends with one `SUMMARY {json}` line for scripts.

use crate::config::{Config, InputMode, parse_invite};
use crate::core::{self, Cmd, CoreEvent, Status};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

#[derive(Default, Debug)]
pub struct Args {
    pub server: String,
    pub key: Option<String>,
    pub name: String,
    pub color: Option<String>,
    pub join_voice: Option<String>,
    pub input_wav: Option<String>,
    pub record_wav: Option<String>,
    pub say: Vec<String>,
    /// Send this as a reply to a message that doesn't exist (the server refuses it).
    pub reply_to_missing: Option<String>,
    pub channel: Option<String>,
    pub duration: Option<f64>,
    pub bitrate: Option<u32>,
    pub mode: Option<InputMode>,
    pub denoise: bool,
    /// Echo cancellation on the WAV mic, against what we hear (off by default here).
    pub echo_cancel: bool,
    pub expect: Option<String>,
    pub mute_after: Option<f64>,
    /// Drop the socket at these times (s) — the call should survive each.
    pub drop_socket_at: Vec<f64>,
    /// Once in voice: Hear yourself, saving the codec's output here.
    pub hear_yourself: Option<String>,
    /// Write the call report here at the end (the call is over by then: "you quit Yap").
    pub call_report: Option<String>,
    pub approve_all: bool,
    /// Sign in with name + password (`--password`, or `--pin` as before; an
    /// old 6-digit PIN signs in too). With `signup`: create the account.
    pub pin: Option<String>,
    /// Once in: change the password to this (the sign-in one as the current).
    pub set_password: Option<String>,
    pub signup: bool,
    pub token: Option<String>,
    /// Play through the default speakers instead of discarding (device test).
    pub speakers: bool,
    pub react: Option<String>,
    pub react_to: Option<String>,
    pub ping: Option<String>,
    pub send_gif: Option<String>,
    /// Edit our first echoed message to this text.
    pub edit_to: Option<String>,
    /// Send this picture file once connected.
    pub send_image: Option<String>,
    /// Add a sidebar Place: "Name=https://…".
    pub add_link: Option<String>,
    /// Make a personal invite link with this label (prints `INVITE <link>`).
    pub make_invite: Option<String>,
    /// Owner only: reset the main invite link once in (everyone connected logs `KEY saved …`).
    pub reset_main_link: bool,
    /// Set the voice channel's quality for everyone (bits/s, or 0 = personal).
    pub channel_bitrate: Option<u32>,
    /// Reply (with `reply_body`) to the first message from someone else containing this.
    pub reply_to: Option<String>,
    pub reply_body: Option<String>,
    /// Personal space passcode: set it up (sealing earlier notes) or unlock;
    /// `--say`/`--send-image` wait until it's open. Encrypted notes are logged as `NOTE`.
    pub vault: Option<String>,
    /// Delete our last message this many seconds in (an encrypted picture: with its blob).
    pub delete_after: Option<f64>,
    /// Search everywhere (`search_here`: just the channel) once connected; logs SEARCH.
    pub search: Option<String>,
    pub search_here: bool,
    /// Then jump to the oldest result (loading older pages); logs JUMPED.
    pub jump_oldest: bool,
    /// Pin the first message containing this (live or already there); logs PINNED / PINS.
    pub pin_msg: Option<String>,
    /// Open the drawing board, draw a line (on the top layer), a box and a
    /// word, save them as a GIF to the gallery; logs DRAW.
    pub draw: bool,
    /// Walk into Space, move around, wave, kick the ball, throw a balloon,
    /// catch fire and run a lap; logs SPACE (incl. `gallery <n>`).
    pub space: bool,
    // ── Extras (v0.36) ──
    /// A poll: "Question|A|B".
    pub poll: Option<String>,
    /// Vote for this option in the first poll someone else posts.
    pub vote: Option<usize>,
    pub away: Option<String>,
    /// Plan a hang `hang_in` seconds from now (default an hour).
    pub hang: Option<String>,
    pub hang_in: Option<f64>,
    /// Upload this WAV as a soundboard clip called `sound_name`.
    pub sound_file: Option<String>,
    pub sound_name: Option<String>,
    /// Once in voice, play the clip with this name.
    pub play_sound: Option<String>,
    pub chime: Option<String>,
    pub shelf: bool,
    pub unread: bool,
    /// `--last-read CH=MS`: start as if these channels were read up to then.
    pub last_read: Vec<(String, i64)>,
    pub save_msg: Option<String>,
    pub doodle_on: Option<String>,
    /// Snooze notifications for an hour.
    pub quiet: bool,
    /// Once in voice, share the synthetic test pattern (never the real
    /// screen) at this quality (smooth / sharp / max).
    pub share_screen: Option<String>,
    /// ...with its test tone as the share's sound.
    pub share_sound: bool,
    /// Watch the first person sharing in our voice channel.
    pub watch_screen: bool,
}

/// An encrypted note, opened with this device's key: `NOTE <id> text=… [gif] [image <blob id>]`.
/// Returns whether it holds a picture.
fn print_note(shared: &Arc<core::Shared>, id: &str, body: &str, start: Instant) -> bool {
    if !crate::vault::is_note(body) {
        return false;
    }
    let key = shared.model.lock().vault_key;
    match key.map(|k| crate::vault::decrypt_note(&k, body)) {
        Some(Ok(n)) => {
            let has_image = n.image.is_some();
            println!(
            "{} NOTE {id} text={:?}{}{}",
            ts(start),
            n.text,
            if n.gif.is_some() { " [gif]" } else { "" },
            n.image.map(|i| format!(" [image {} {}x{} {}]", i.id, i.width, i.height, i.mime)).unwrap_or_default()
            );
            has_image
        }
        Some(Err(e)) => {
            println!("{} NOTE {id} can't open: {e}", ts(start));
            false
        }
        None => {
            println!("{} NOTE {id} locked", ts(start));
            false
        }
    }
}

fn ts(start: Instant) -> String {
    format!("[+{:6.2}s]", start.elapsed().as_secs_f64())
}

pub fn run(args: Args) -> i32 {
    // Test bots never touch the OS keychain (the Personal space key stays in memory).
    crate::vault::disable_keychain();
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio");
    rt.block_on(run_async(args))
}

async fn run_async(args: Args) -> i32 {
    let start = Instant::now();
    let (server, invite_key) = parse_invite(&args.server);
    let mut cfg = Config {
        server,
        key: args.key.clone().or(invite_key).unwrap_or_default(),
        name: if args.name.is_empty() { "Bot".into() } else { args.name.clone() },
        noise_suppression: args.denoise,
        echo_cancellation: args.echo_cancel,
        sounds: false,
        mode: args.mode.unwrap_or(InputMode::Open),
        ..Config::default()
    };
    if let Some(c) = &args.color {
        cfg.color = c.clone();
    }
    // A bot has never read anything (it would start caught up): pretend it has, up to then.
    cfg.last_read.extend(args.last_read.iter().cloned());
    if let Some(b) = args.bitrate {
        cfg.bitrate = b;
    }
    cfg.fill_defaults();
    if let Some(b) = args.bitrate {
        cfg.bitrate = b; // allow any value in tests
    }
    if cfg.server.is_empty() {
        eprintln!("--server is required");
        return 2;
    }
    println!("{} yap headless: {} as {} (uid {}), bitrate {}", ts(start), cfg.server, cfg.name, cfg.uid, cfg.bitrate);

    let config_path = std::env::temp_dir().join(format!("yap-headless-{}.json", cfg.uid));
    let color = cfg.color.clone();
    let name = cfg.name.clone();
    let shared = core::Shared::new(cfg, config_path, Arc::new(|| {}));
    if args.pin.is_none() && args.token.is_none() {
        eprintln!("--password or --token is needed (accounts sign in with a name and password)");
        return 2;
    }
    {
        let mut m = shared.model.lock();
        m.token = args.token.clone();
        m.auth = args.pin.as_ref().map(|pin| if args.signup { core::Auth::Signup { name, pin: pin.clone(), color } } else { core::Auth::Login { name, pin: pin.clone() } });
    }
    *shared.audio_override.lock() = Some((args.input_wav.clone().unwrap_or_else(|| "-".into()), args.record_wav.clone()));
    if args.speakers {
        *shared.speakers_override.lock() = true;
    }
    // Screen sharing in tests: only ever the synthetic pattern.
    if args.share_screen.is_some() {
        crate::screen::capture::offer_pattern(true);
    }
    let (obs_tx, mut obs_rx) = tokio::sync::mpsc::unbounded_channel();
    let cmd = core::start(shared.clone(), Some(obs_tx));
    let watch_first = |shared: &Arc<core::Shared>, cmd: &tokio::sync::mpsc::UnboundedSender<Cmd>, start: Instant| {
        let m = shared.model.lock();
        if m.screen.watching.is_some() {
            return;
        }
        let Some(mine) = m.voice_channel.clone() else { return };
        let me = m.sid.clone();
        if let Some(s) = m.sessions.values().find(|s| Some(&s.sid) != me.as_ref() && s.voice.as_ref().is_some_and(|v| v.screen && v.channel_id == mine)) {
            println!("{} SCREEN watching {}", ts(start), s.sid);
            let _ = cmd.send(Cmd::WatchScreen(s.sid.clone()));
        }
    };

    let deadline = args.duration.map(|d| start + Duration::from_secs_f64(d));
    let mut report = tokio::time::interval(Duration::from_secs(1));
    let mut did_ready = false;
    let mut said = false;
    let mut edited_once = false;
    let mut replied = false;
    let mut deleted_once = false;
    let (mut jumped, mut drew, mut walked, mut searched, mut pinned) = (false, false, false, false, false);
    let (mut voted, mut saved, mut doodled, mut played) = (false, false, false, false);
    let mut last_picture: Option<String> = None;
    let mut muted_once = false;
    let mut drops = args.drop_socket_at.clone();
    let mut hear = args.hear_yourself.clone();
    let mut expect_seen = false;
    let mut messages_seen = 0usize;
    let mut tried_signup = false;
    let (mut mentions_seen, mut reactions_seen, mut pings_seen) = (0usize, 0usize, 0usize);
    let mut voice_channel: Option<String> = None;
    let mut max_level = 0f32;
    let mut first_connected: Option<Instant> = None;
    let mut window: Vec<(Instant, u64)> = Vec::new();
    let mut summary_peers = serde_json::Value::Null;
    // The share's sound we've heard (bytes on someone's screen-sound track).
    let mut screen_sound_bytes = 0u64;
    // Sends the server asked us to slow down for / refused, and texts given back.
    let (mut send_retries, mut sends_refused) = (0usize, 0usize);
    let mut restored: Vec<String> = Vec::new();
    // Whose OBS link listens in a voice channel without them (the greyed rows), as last logged.
    let mut obs_rows = String::new();

    loop {
        tokio::select! {
            ev = obs_rx.recv() => {
                let Some(ev) = ev else { break };
                match &ev {
                    CoreEvent::Ready { sid } => {
                        println!("{} ready, sid {sid}", ts(start));
                        if !did_ready {
                            did_ready = true;
                            {
                                // What we signed in with, and whether the "Choose a password" prompt would show.
                                let m = shared.model.lock();
                                let prompt = m.password_prompt().map(|r| format!("{r:?}")).unwrap_or_else(|| "none".into());
                                println!("{} ACCOUNT uid={} secret={} weak={} prompt={prompt} role={}", ts(start), m.config.uid, m.secret_kind, m.weak_pin, m.users.get(&m.config.uid).map(|u| u.role.as_str()).unwrap_or("?"));
                            }
                            if let Some(new) = &args.set_password {
                                let current = args.pin.clone();
                                let _ = cmd.send(Cmd::SetPassword { password: new.clone(), current });
                                println!("{} changing the password", ts(start));
                            }
                            let (text_id, voice_id) = {
                                let m = shared.model.lock();
                                let find = |want: &str, kind: &str| m.channels.iter()
                                    .find(|c| (c.kind == kind || kind == "text" && c.is_personal()) && (c.id == want || c.name.eq_ignore_ascii_case(want)))
                                    .map(|c| c.id.clone());
                                let text = args.channel.as_deref().map(|c| find(c, "text")).unwrap_or_else(|| m.first_text_channel().map(|c| c.id.clone()));
                                let voice = args.join_voice.as_deref().and_then(|c| find(c, "voice"));
                                (text, voice)
                            };
                            if let Some(t) = &text_id {
                                let _ = cmd.send(Cmd::OpenChannel(t.clone()));
                            }
                            if let Some(q) = args.send_gif.clone() {
                                send_gif(&shared, &cmd, q, start);
                            }
                            if let Some(pass) = &args.vault {
                                let has = shared.model.lock().vault.is_some();
                                println!("{} vault: {}", ts(start), if has { "unlocking" } else { "setting up" });
                                let op = if has { core::VaultOp::Unlock { pass: pass.clone() } } else { core::VaultOp::Setup { pass: pass.clone() } };
                                let _ = cmd.send(Cmd::Vault(op));
                            }
                            if let Some(path) = args.send_image.as_ref().filter(|_| args.vault.is_none()) {
                                match std::fs::read(path) {
                                    Ok(bytes) => {
                                        let _ = cmd.send(Cmd::SendPicture { bytes, caption: String::new() });
                                        println!("{} sending picture {path}", ts(start));
                                    }
                                    Err(e) => println!("{} can't read {path}: {e}", ts(start)),
                                }
                            }
                            if let Some((name, url)) = args.add_link.as_deref().and_then(|l| l.split_once('=')) {
                                let _ = cmd.send(Cmd::AddLink { name: name.to_string(), url: url.to_string() });
                                println!("{} adding link {name}", ts(start));
                            }
                            if let Some(label) = &args.make_invite {
                                let _ = cmd.send(Cmd::CreateInvite(label.clone()));
                            }
                            if args.reset_main_link {
                                let _ = cmd.send(Cmd::ResetMainLink);
                                println!("{} resetting the main link", ts(start));
                            }
                            if let Some(p) = &args.ping {
                                let _ = cmd.send(Cmd::Ping(p.clone()));
                                println!("{} pinged everyone: {p}", ts(start));
                            }
                            if !args.say.is_empty() && !said && args.vault.is_none() {
                                said = true;
                                for s in &args.say {
                                    let _ = cmd.send(Cmd::Send { body: s.clone(), reply_to: None });
                                    println!("{} said {s:?} in #{}", ts(start), text_id.clone().unwrap_or_default());
                                }
                            }
                            if let Some(text) = &args.reply_to_missing {
                                let _ = cmd.send(Cmd::Send { body: text.clone(), reply_to: Some(format!("gone-{}", crate::config::random_hex(8))) });
                                println!("{} said {text:?} as a reply to a message that doesn't exist", ts(start));
                            }
                            extras_at_ready(&args, &shared, &cmd, start);
                            if let Some(v) = voice_id {
                                println!("{} joining voice {v}", ts(start));
                                let _ = cmd.send(Cmd::JoinVoice(v));
                            } else if let Some(want) = &args.join_voice {
                                println!("{} no voice channel {want:?}", ts(start));
                            }
                        }
                    }
                    CoreEvent::Message { id, channel, from, body, gif, image, reply } => {
                        messages_seen += 1;
                        let re = reply.as_deref().map(|r| format!(" [reply to {r}]")).unwrap_or_default();
                        println!("{} message #{channel} <{from}> {body}{}{}{re}", ts(start), if *gif { " [gif]" } else { "" }, if *image { " [image]" } else { "" });
                        let note_image = print_note(&shared, id, body, start);
                        let (mine, my_uid) = {
                            let m = shared.model.lock();
                            (*from == m.config.name, m.config.uid.clone())
                        };
                        if mine && (*image || note_image) {
                            last_picture = Some(id.clone());
                        }
                        if let Some(want) = &args.reply_to
                            && !replied
                            && body.contains(want.as_str())
                            && shared.model.lock().history.get(channel).and_then(|h| h.messages.iter().find(|x| &x.id == id)).is_some_and(|x| x.uid != my_uid)
                        {
                            replied = true;
                            let text = args.reply_body.clone().unwrap_or_else(|| "got it".into());
                            let _ = cmd.send(Cmd::Send { body: text.clone(), reply_to: Some(id.clone()) });
                            println!("{} REPLIED to {id}: {text}", ts(start));
                        }
                        if mine && !edited_once && let Some(to) = &args.edit_to {
                            edited_once = true;
                            let _ = cmd.send(Cmd::Edit { id: id.clone(), body: to.clone() });
                            println!("{} editing {id} to {to:?}", ts(start));
                        }
                        if let (Some(want), Some(emoji)) = (&args.react_to, &args.react)
                            && body.contains(want.as_str())
                        {
                            let _ = cmd.send(Cmd::React { id: id.clone(), emoji: emoji.clone(), on: true });
                            println!("{} reacted {emoji} to {id}", ts(start));
                        }
                        if let Some(n) = args.vote.filter(|_| !voted && !mine) {
                            let is_poll = shared.model.lock().history.get(channel).and_then(|h| h.messages.iter().find(|x| &x.id == id)).is_some_and(|x| x.poll.is_some());
                            if is_poll {
                                voted = true;
                                let _ = cmd.send(Cmd::PollVote { id: id.clone(), option: Some(n) });
                                println!("{} voted {n} in {id}", ts(start));
                            }
                        }
                        if let Some(want) = args.save_msg.as_ref().filter(|w| !saved && body.contains(w.as_str()) && !mine) {
                            saved = true;
                            let _ = cmd.send(Cmd::SaveToPersonal { id: id.clone(), comment: String::new() });
                            println!("{} saving {id} ({want}) to the Personal space", ts(start));
                        }
                        if args.doodle_on.as_ref().is_some_and(|w| !doodled && body.contains(w.as_str()) && !mine) {
                            doodled = true;
                            let _ = cmd.send(Cmd::Doodle { message: id.clone(), bytes: test_doodle() });
                            println!("{} doodling on {id}", ts(start));
                        }
                        if args.expect.as_deref().is_some_and(|e| body.contains(e)) {
                            expect_seen = true;
                            println!("{} EXPECT_SEEN {:?}", ts(start), args.expect.as_deref().unwrap_or(""));
                        }
                    }
                    CoreEvent::VoiceJoined { channel } => {
                        voice_channel = Some(channel.clone());
                        println!("{} voice joined {channel}", ts(start));
                        if let Some(q) = &args.share_screen {
                            let quality = crate::screen::Quality::parse(q).unwrap_or_default();
                            println!("{} SCREEN sharing the test pattern ({}, sound {})", ts(start), quality.label(), if args.share_sound { "on" } else { "off" });
                            let _ = cmd.send(Cmd::ShareScreen { source: crate::screen::capture::pattern::source(), quality, sound: args.share_sound });
                        }
                        if args.watch_screen {
                            watch_first(&shared, &cmd, start);
                        }
                        if let Some(path) = hear.take() {
                            let _ = cmd.send(Cmd::HearYourself(Some(path)));
                        }
                        played |= play_named(&args, &shared, &cmd, start);
                        if let Some(b) = args.channel_bitrate {
                            let _ = cmd.send(Cmd::SetChannelBitrate { channel_id: channel.clone(), bitrate: (b > 0).then_some(b) });
                            println!("{} channel quality -> {b}", ts(start));
                        }
                    }
                    CoreEvent::PeerState { sid, state } => {
                        println!("{} peer {sid} {state}", ts(start));
                        if state == "connected" && first_connected.is_none() {
                            first_connected = Some(Instant::now());
                        }
                    }
                    CoreEvent::Status(s, d) => {
                        if *s != Status::Online {
                            println!("{} status {s:?} {d}", ts(start));
                        }
                        if *s == Status::Pending {
                            println!("{} PENDING uid {}", ts(start), shared.model.lock().config.uid);
                        }
                    }
                    // `--password` without `--signup`: sign in, or create the account if there's none
                    // (a wrong name or password both say `bad_login`; signing up then finds out which).
                    CoreEvent::Error { code, message } => {
                        println!("{} {ev:?}", ts(start));
                        if code == "bad_login"
                            && !args.signup
                            && !tried_signup
                            && shared.model.lock().status == Status::SignIn
                            && let Some(pin) = &args.pin
                        {
                            tried_signup = true;
                            println!("{} no such account ({message}); signing up", ts(start));
                            let (name, color) = {
                                let m = shared.model.lock();
                                (m.config.name.clone(), m.config.color.clone())
                            };
                            let _ = cmd.send(Cmd::SignIn(core::Auth::Signup { name, pin: pin.clone(), color }));
                        }
                    }
                    CoreEvent::Token(t) => println!("{} TOKEN {t}", ts(start)),
                    CoreEvent::Edited { id, body } => {
                        println!("{} EDITED {id} {body}", ts(start));
                        let _ = print_note(&shared, id, body, start);
                    }
                    CoreEvent::Vault(what) => {
                        println!("{} VAULT {what}", ts(start));
                        if (what == "set" || what == "unlocked") && !said {
                            said = true;
                            let personal = shared.model.lock().channels.iter().find(|c| c.is_personal()).map(|c| c.id.clone());
                            if let Some(p) = personal {
                                let _ = cmd.send(Cmd::OpenChannel(p));
                            }
                            for s in &args.say {
                                let _ = cmd.send(Cmd::Send { body: s.clone(), reply_to: None });
                                println!("{} said {s:?} (encrypted)", ts(start));
                            }
                            if let Some(path) = &args.send_image {
                                match std::fs::read(path) {
                                    Ok(bytes) => {
                                        let _ = cmd.send(Cmd::SendPicture { bytes, caption: String::new() });
                                        println!("{} sending picture {path} (encrypted)", ts(start));
                                    }
                                    Err(e) => println!("{} can't read {path}: {e}", ts(start)),
                                }
                            }
                        }
                    }
                    CoreEvent::Sealed(n) => println!("{} SEALED {n} earlier notes", ts(start)),
                    CoreEvent::SearchResults(ids) => {
                        let lines: Vec<String> = {
                            let m = shared.model.lock();
                            m.search.as_ref().map(|st| st.results().iter().map(|x| format!("{}:{}", x.id, x.body.chars().take(40).collect::<String>())).collect()).unwrap_or_default()
                        };
                        println!("{} SEARCH {} results: {}", ts(start), ids.len(), lines.join(" | "));
                        if args.jump_oldest && !jumped {
                            jumped = true;
                            let target = {
                                let m = shared.model.lock();
                                m.search.as_ref().and_then(|st| st.results().last().map(|x| (x.channel_id.clone(), x.id.clone())))
                            };
                            if let Some((channel, id)) = target {
                                println!("{} jumping to {id}", ts(start));
                                let _ = cmd.send(Cmd::JumpTo { channel, id });
                            }
                        }
                    }
                    CoreEvent::Jumped { id, pages } => println!("{} JUMPED {id} after {pages} older pages", ts(start)),
                    CoreEvent::Pinned { id, on } => println!("{} PINNED {id} {}", ts(start), if *on { "on" } else { "off" }),
                    CoreEvent::Pins { channel, count } => println!("{} PINS #{channel} {count}", ts(start)),
                    CoreEvent::Watch(what) => println!("{} WATCH {what}", ts(start)),
                    CoreEvent::Screen { sid, on } => {
                        println!("{} SCREEN {sid} {}", ts(start), if *on { "on" } else { "off" });
                        if *on && args.watch_screen {
                            watch_first(&shared, &cmd, start);
                        }
                    }
                    CoreEvent::Draw(what) => {
                        println!("{} DRAW {what}", ts(start));
                        if what == "state" && args.draw && !drew {
                            drew = true;
                            let sk = crate::config::random_hex(10);
                            // The line goes on the top layer (layer 3 in the app; `l` 2 on the wire).
                            let line = crate::protocol::Stroke { id: sk.clone(), c: "#0080ff".into(), w: 32.0, p: vec![1000.0, 1000.0, 4000.0, 2500.0], k: Some("poly".into()), l: 2, ..Default::default() };
                            let _ = cmd.send(Cmd::DrawSeg { sk, c: line.c.clone(), w: 32.0, e: false, p: line.p.clone(), l: line.l });
                            let _ = cmd.send(Cmd::DrawStroke(line.clone()));
                            let b = crate::protocol::Stroke { id: crate::config::random_hex(10), c: "#ff0000".into(), w: 14.0, p: vec![5000.0, 1000.0, 8000.0, 1000.0, 8000.0, 3000.0, 5000.0, 3000.0, 5000.0, 1000.0], k: Some("poly".into()), ..Default::default() };
                            let _ = cmd.send(Cmd::DrawStroke(b.clone()));
                            let t = crate::protocol::Stroke { id: crate::config::random_hex(10), c: "#008000".into(), w: 32.0, p: vec![1200.0, 4500.0], k: Some("text".into()), text: Some("hi from native".into()), ..Default::default() };
                            let _ = cmd.send(Cmd::DrawStroke(t.clone()));
                            println!("{} drew a line, a box and a word (the line on the top layer, {})", ts(start), line.id);
                            // "GIF": the three wiggle frames, into the gallery (logged as DRAW saved <media>).
                            let _ = cmd.send(Cmd::SaveDrawing { strokes: vec![line, b, t], gif: true, wiggle: true, download: false, share: false, then: None });
                        }
                    }
                    CoreEvent::Space(what) => {
                        println!("{} SPACE {what}", ts(start));
                        if what.starts_with("state") && args.space && !walked {
                            walked = true;
                            // A spot on the plaza that differs per name (bots don't stand on each other).
                            let spread = shared.model.lock().config.name.bytes().map(|b| b as u32).sum::<u32>() % 7;
                            let me = crate::protocol::SpacePos { x: 1000.0 + spread as f32 * 70.0, y: 760.0 + (spread % 3) as f32 * 60.0, ..Default::default() };
                            let _ = cmd.send(Cmd::SpaceMove { x: me.x, y: me.y, d: 0.0 });
                            let _ = cmd.send(Cmd::SpaceMove { x: me.x + 60.0, y: me.y, d: 0.0 });
                            let _ = cmd.send(Cmd::SpaceEmote("👋".into()));
                            println!("{} walked to {:.0},{:.0} and waved", ts(start), me.x + 60.0, me.y);
                            // Ask for the games' state, and kick the ball (a high k wins it).
                            let _ = cmd.send(Cmd::SpaceGame { g: "sync".into(), d: serde_json::json!({}) });
                            let k = crate::config::random_hex(3).chars().fold(1_000_000u64, |a, ch| a + ch.to_digit(16).unwrap_or(0) as u64);
                            let _ = cmd.send(Cmd::SpaceGame { g: "ball".into(), d: serde_json::json!({ "x": 1490, "y": 1272, "vx": 400, "vy": 0, "k": k, "s": [0, 0] }) });
                            println!("{} kicked the ball (k {k})", ts(start));
                            // A water balloon east, on fire for 3 s, and a lap of the track.
                            let _ = cmd.send(Cmd::SpaceGame { g: "throw".into(), d: serde_json::json!({ "x": (me.x + 96.0) as i64, "y": me.y as i64, "a": 0, "id": 7 }) });
                            let _ = cmd.send(Cmd::SpaceGame { g: "fx".into(), d: serde_json::json!({ "fire": 3000, "wet": 0 }) });
                            let _ = cmd.send(Cmd::SpaceGame { g: "lap".into(), d: serde_json::json!({ "ms": 41234, "best": 41234, "n": 1 }) });
                            println!("{} threw a balloon (id 7), caught fire and ran a lap (41.2 s)", ts(start));
                        }
                    }
                    CoreEvent::Links(names) => println!("{} LINKS {}", ts(start), names.join(", ")),
                    CoreEvent::Invite { label, link } => println!("{} INVITE {label} {link}", ts(start)),
                    CoreEvent::Mention { from, body } => {
                        mentions_seen += 1;
                        println!("{} MENTION from {from}: {body}", ts(start));
                    }
                    CoreEvent::Reaction { message_id, emoji, from, on } => {
                        reactions_seen += 1;
                        println!("{} REACTION {emoji} by {from} {} on {message_id}", ts(start), if *on { "added" } else { "removed" });
                    }
                    CoreEvent::Ping { from, text } => {
                        pings_seen += 1;
                        println!("{} PING from {from}: {text}", ts(start));
                    }
                    CoreEvent::JoinRequest { uid, name, invited_by, used_by } => {
                        let via = invited_by.as_deref().map(|b| format!(" invited by {b}")).unwrap_or_default();
                        let also = if used_by.is_empty() { String::new() } else { format!(" · link also used by {}", used_by.join(", ")) };
                        println!("{} join request from {name} ({uid}){via}{also}", ts(start));
                        if args.approve_all {
                            let _ = cmd.send(Cmd::Approve { uid: uid.clone(), approved: true });
                            println!("{} let {name} in", ts(start));
                        }
                    }
                    CoreEvent::Extra(tag, text) => {
                        println!("{} {tag} {text}", ts(start));
                        if *tag == "SEND" {
                            if text.starts_with("retry") {
                                send_retries += 1;
                            } else if text.starts_with("refused") {
                                sends_refused += 1;
                            }
                        }
                        // Our clip just landed: play it if we're already in voice.
                        if *tag == "SOUND" && text.starts_with("added") && voice_channel.is_some() && !played {
                            played = play_named(&args, &shared, &cmd, start);
                        }
                    }
                    other => println!("{} {other:?}", ts(start)),
                }
            }
            _ = report.tick() => {
                // --reply-to also finds the message in what's already there.
                if let Some(want) = args.reply_to.as_ref().filter(|_| !replied && did_ready) {
                    let found = {
                        let m = shared.model.lock();
                        m.history.get(&m.config.channel).and_then(|h| h.messages.iter().rev().find(|x| x.uid != m.config.uid && x.body.contains(want.as_str())).map(|x| x.id.clone()))
                    };
                    if let Some(id) = found {
                        replied = true;
                        let text = args.reply_body.clone().unwrap_or_else(|| "got it".into());
                        let _ = cmd.send(Cmd::Send { body: text.clone(), reply_to: Some(id.clone()) });
                        println!("{} REPLIED to {id}: {text}", ts(start));
                    }
                }
                if did_ready && !searched && let Some(q) = &args.search {
                    searched = true;
                    let channel = args.search_here.then(|| shared.model.lock().config.channel.clone());
                    println!("{} searching {q:?}{}", ts(start), if channel.is_some() { " here" } else { " everywhere" });
                    let _ = cmd.send(Cmd::Search { q: q.clone(), channel });
                }
                if did_ready && (args.draw || args.space) && shared.model.lock().view.is_none() && !(drew || walked) {
                    let v = if args.draw { core::View::Draw } else { core::View::Space };
                    let _ = cmd.send(Cmd::SetView(Some(v)));
                }
                if let Some(want) = args.pin_msg.as_ref().filter(|_| !pinned && did_ready) {
                    let found = {
                        let m = shared.model.lock();
                        m.history.get(&m.config.channel).and_then(|h| h.messages.iter().rev().find(|x| x.body.contains(want.as_str())).map(|x| x.id.clone()))
                    };
                    if let Some(id) = found {
                        pinned = true;
                        println!("{} pinning {id}", ts(start));
                        let _ = cmd.send(Cmd::Pin { id, on: true });
                    }
                }
                if let Some(at) = args.delete_after
                    && !deleted_once && start.elapsed().as_secs_f64() > at
                {
                    deleted_once = true;
                    let last = last_picture.clone().or_else(|| {
                        let m = shared.model.lock();
                        m.last_own_message(&m.config.channel).map(|x| x.id.clone())
                    });
                    match last {
                        Some(id) => {
                            println!("{} deleting {id}", ts(start));
                            let _ = cmd.send(Cmd::Delete(id));
                        }
                        None => println!("{} nothing of ours to delete", ts(start)),
                    }
                }
                if drops.first().is_some_and(|at| start.elapsed().as_secs_f64() > *at) {
                    drops.remove(0);
                    println!("{} DROP socket", ts(start));
                    let _ = cmd.send(Cmd::DropSocket);
                }
                if let Some(at) = args.mute_after
                    && !muted_once && start.elapsed().as_secs_f64() > at
                {
                    muted_once = true;
                    println!("{} toggling mute", ts(start));
                    let _ = cmd.send(Cmd::ToggleMute);
                }
                // The greyed "<name>'s OBS is still listening" rows, when they change.
                let rows = obs_listening_rows(&shared);
                if rows != obs_rows {
                    println!("{} OBSLISTEN {}", ts(start), if rows.is_empty() { "none" } else { rows.as_str() });
                    obs_rows = rows;
                }
                // A refused message's text, which the window would put back in the message box.
                for (channel, text) in std::mem::take(&mut shared.model.lock().composer_restore) {
                    println!("{} RESTORED #{channel} {text:?}", ts(start));
                    restored.push(text);
                }
                // Screen sharing, once a second: what we watch, what we send.
                let (frames, size) = shared.screen.watch.stats();
                if shared.model.lock().screen.watching.is_some() {
                    let sound: u64 = shared.audio.streams().iter().filter(|s| s.sid().starts_with("screen:")).map(|s| s.stats().bytes).sum();
                    screen_sound_bytes = screen_sound_bytes.max(sound);
                    println!("{} SCREEN frames {frames} {}x{} sound {sound}B", ts(start), size.0, size.1);
                }
                if shared.screen.share.sharing() {
                    let (n, sz) = shared.screen.share.stats();
                    let names: Vec<String> = { let m = shared.model.lock(); m.screen.watchers.iter().map(|s| m.sessions.get(s).map(|x| m.user_name(&x.uid)).unwrap_or_default()).collect() };
                    println!("{} SCREEN encoded {n} {}x{} viewers {} watching [{}]", ts(start), sz.0, sz.1, shared.screen.share.viewers(), names.join(", "));
                }
                if voice_channel.is_some() {
                    let peers = shared.voice_peers.lock().clone();
                    let streams = shared.audio.streams();
                    let gate = &shared.audio.gate;
                    let mut line = format!(
                        "{} mic {:6.1} dBFS tx={} out={}k frames |",
                        ts(start),
                        gate.level_db.get(),
                        gate.transmitting.load(Ordering::Relaxed) as u8,
                        shared.audio.mixer.rendered.load(Ordering::Relaxed) / 1000
                    );
                    let mut recv_total = 0u64;
                    let mut peer_json = Vec::new();
                    for p in &peers {
                        let st = p.state.lock().clone();
                        let stream = streams.iter().find(|s| s.sid() == p.sid());
                        let (bytes, packets, level) = stream.map(|s| { let x = s.stats(); (x.bytes, x.packets, s.level.get()) }).unwrap_or((0, 0, 0.0));
                        let jb = stream.map(|s| s.stats()).unwrap_or_default();
                        recv_total += bytes;
                        if st == "connected" { max_level = max_level.max(level); }
                        let db = if level > 0.0 { 20.0 * level.log10() } else { -100.0 };
                        line.push_str(&format!(
                            " peer {} {st} sent {}B/{}pk @{}k{} loss{}% | recv {}B/{}pk{} level {:5.1} dBFS plc={} fec={} red={} late={} under={} pad={} ins={} drop={} jb={:.0}/{:.0}ms rtt={}{}",
                            &p.sid()[..p.sid().len().min(8)], p.bytes_sent.load(Ordering::Relaxed), p.packets_sent.load(Ordering::Relaxed),
                            p.send_bitrate.load(Ordering::Relaxed) / 1000, if p.send_red.load(Ordering::Relaxed) { " red+opus" } else { "" }, p.loss_hint.load(Ordering::Relaxed),
                            bytes, packets, if jb.red_packets > 0 { " red+opus" } else { "" }, db, jb.concealed, jb.fec, jb.red, jb.late, jb.underruns, jb.padding,
                            jb.inserted, jb.dropped, jb.delay_ms, jb.target_ms, p.rtt_ms.load(Ordering::Relaxed),
                            if stream.is_some_and(|s| s.speaking()) { " SPEAKING" } else { "" },
                        ));
                        let stats = stream.map(|s| s.stats());
                        peer_json.push(serde_json::json!({
                            "sid": p.sid(), "state": st, "polite": p.polite(),
                            "bytesSent": p.bytes_sent.load(Ordering::Relaxed), "packetsSent": p.packets_sent.load(Ordering::Relaxed),
                            "sendBitrate": p.send_bitrate.load(Ordering::Relaxed),
                            "bytesReceived": bytes, "packetsReceived": packets, "level": level,
                            "opusBytesSent": p.opus_bytes_sent.load(Ordering::Relaxed), "sendRed": p.send_red.load(Ordering::Relaxed),
                            "redPackets": stats.map(|s| s.red_packets), "red": stats.map(|s| s.red), "expected": stats.map(|s| s.expected),
                            "concealed": stats.map(|s| s.concealed), "fec": stats.map(|s| s.fec), "underruns": stats.map(|s| s.underruns), "late": stats.map(|s| s.late), "padding": stats.map(|s| s.padding),
                            "frames": stats.map(|s| s.frames), "inserted": stats.map(|s| s.inserted), "dropped": stats.map(|s| s.dropped), "targetMs": stats.map(|s| s.target_ms), "delayMs": stats.map(|s| s.delay_ms),
                            "lossHint": p.loss_hint.load(Ordering::Relaxed), "rttMs": p.rtt_ms.load(Ordering::Relaxed), "localTransportCc": p.local_twcc.load(Ordering::Relaxed),
                            "localFmtp": p.local_fmtp.lock().clone(), "remoteFmtp": p.remote_fmtp.lock().clone(),
                        }));
                    }
                    if peers.is_empty() { line.push_str(" no peers"); }
                    // Echo cancellation (--echo-cancel): AEC3's delay estimate and how much echo it takes out.
                    let echo = *gate.echo.lock();
                    if echo.on {
                        let ms = echo.delay_ms.map_or("?".into(), |d| d.to_string());
                        let erle = echo.erle_db.map_or("?".into(), |e| format!("{e:.0}"));
                        line.push_str(&format!(" | aec={ms}ms erle={erle}dB realigned={} owed={}", echo.realigned, echo.underruns));
                    }
                    println!("{line}");
                    // Only measure while connected, so the summary describes the live call.
                    if peers.iter().any(|p| *p.state.lock() == "connected") {
                        window.push((Instant::now(), recv_total));
                        summary_peers = serde_json::Value::Array(peer_json);
                    }
                }
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    break;
                }
            }
        }
    }

    // Received kbps over the last ~5 s (skip ramp-up).
    let recv_kbps = if window.len() >= 3 {
        let end = window[window.len() - 1];
        let begin = window[window.len().saturating_sub(6)];
        let secs = end.0.duration_since(begin.0).as_secs_f64().max(0.001);
        (end.1.saturating_sub(begin.1) as f64 * 8.0 / 1000.0 / secs).round()
    } else {
        0.0
    };
    let status = format!("{:?}", shared.model.lock().status);
    let summary = serde_json::json!({
        "status": status,
        "messagesSeen": messages_seen,
        "mentionsSeen": mentions_seen,
        "reactionsSeen": reactions_seen,
        "pingsSeen": pings_seen,
        "expectSeen": expect_seen,
        "voiceChannel": voice_channel,
        "maxRecvLevel": max_level,
        "recvKbps": recv_kbps,
        "peers": summary_peers,
        "screenFrames": shared.screen.watch.stats().0,
        "screenSoundBytes": screen_sound_bytes,
        "screenSize": [shared.screen.watch.stats().1.0, shared.screen.watch.stats().1.1],
        "screenEncoded": shared.screen.share.stats().0,
        "screenEncodedSize": [shared.screen.share.stats().1.0, shared.screen.share.stats().1.1],
        "sendRetries": send_retries,
        "sendsRefused": sends_refused,
        "pendingLeft": shared.model.lock().pending.len(),
        "restored": restored,
    });
    println!("SUMMARY {summary}");
    let _ = cmd.send(Cmd::Quit);
    tokio::time::sleep(Duration::from_millis(300)).await;
    if let Some(path) = &args.call_report {
        match shared.calls.lock().text() {
            Some(text) => match std::fs::write(path, &text) {
                Ok(()) => println!("CALLREPORT {} lines in {path}", text.lines().count()),
                Err(e) => println!("CALLREPORT couldn't write {path}: {e}"),
            },
            None => println!("CALLREPORT none (no call)"),
        }
    }
    0
}

/// The greyed rows under each voice channel (`Model::obs_listening`):
/// "Lounge: Rosa, Ada; Games: Ben", or empty.
fn obs_listening_rows(shared: &Arc<core::Shared>) -> String {
    let m = shared.model.lock();
    let mut rows = Vec::new();
    for c in m.channels.iter().filter(|c| c.is_voice()) {
        let names: Vec<String> = m.obs_listening(&c.id).iter().map(|uid| m.user_name(uid)).collect();
        if !names.is_empty() {
            rows.push(format!("{}: {}", c.name, names.join(", ")));
        }
    }
    rows.join("; ")
}

/// `--send-gif`: once /api/config arrives, search KLIPY and send the first hit.
fn send_gif(shared: &Arc<core::Shared>, cmd: &tokio::sync::mpsc::UnboundedSender<Cmd>, query: String, start: Instant) {
    let (shared, cmd) = (shared.clone(), cmd.clone());
    tokio::spawn(async move {
        for _ in 0..50 {
            let cfg = shared.model.lock().server_config.clone();
            if let Some(cfg) = cfg {
                let Some(key) = cfg.gifs.map(|g| g.app_key) else {
                    println!("{} GIF: no KLIPY key on this server", ts(start));
                    return;
                };
                let uid = shared.model.lock().config.uid.clone();
                let q = query.clone();
                let found = tokio::task::spawn_blocking(move || {
                    let r = crate::klipy::fetch(&key, &uid, crate::klipy::Mode::Search, &q, 1);
                    if let Ok((gifs, _)) = &r
                        && let Some(g) = gifs.first()
                    {
                        crate::klipy::report_share(&key, &uid, &g.slug, &q);
                    }
                    r
                })
                .await;
                match found {
                    Ok(Ok((gifs, _))) if !gifs.is_empty() => {
                        println!("{} GIF sent: {} ({})", ts(start), gifs[0].title, gifs[0].slug);
                        let _ = cmd.send(Cmd::SendGif { gif: gifs[0].clone(), caption: String::new(), reply_to: None });
                    }
                    other => println!("{} GIF: nothing to send ({other:?})", ts(start)),
                }
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
}

/// The extras to do once we're in (`--away`, `--chime`, `--quiet`, `--poll`,
/// `--hang`, `--sound-file`, `--shelf`, `--unread`).
fn extras_at_ready(args: &Args, shared: &Arc<core::Shared>, cmd: &tokio::sync::mpsc::UnboundedSender<Cmd>, start: Instant) {
    if let Some(text) = &args.away {
        let _ = cmd.send(Cmd::SetAway(Some((text.clone(), None))));
        println!("{} away: {text}", ts(start));
    }
    if let Some(c) = &args.chime {
        let _ = cmd.send(Cmd::SetChime(Some(c.clone())));
        println!("{} chime: {c}", ts(start));
    }
    if args.quiet {
        let tz = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
        let q = crate::protocol::QuietHours { from: 0, to: 0, tz, mentions: false, until: Some(core::now_ms() + 3_600_000) };
        let _ = cmd.send(Cmd::SetQuiet(Some(q)));
        println!("{} snoozed for an hour", ts(start));
    }
    if let Some(spec) = &args.poll {
        let text = format!("/poll {}", spec.replacen('|', " | ", 1));
        match core::parse_poll(&text) {
            Some(Ok((question, options))) => {
                println!("{} polling {question:?} {options:?}", ts(start));
                let _ = cmd.send(Cmd::SendPoll { question, options });
            }
            other => println!("{} bad poll {spec:?}: {other:?}", ts(start)),
        }
    }
    if let Some(title) = &args.hang {
        let at = core::now_ms() + (args.hang_in.unwrap_or(3600.0) * 1000.0) as i64;
        let channel = shared.model.lock().channels.iter().find(|c| c.is_voice()).map(|c| c.id.clone());
        let _ = cmd.send(Cmd::SetHang { at, title: title.clone(), channel });
        println!("{} planning {title:?} at {at}", ts(start));
    }
    if let Some(path) = &args.sound_file {
        match std::fs::read(path) {
            Ok(wav) => {
                let name = args.sound_name.clone().unwrap_or_else(|| "Bot sound".into());
                println!("{} uploading sound {name:?}", ts(start));
                let _ = cmd.send(Cmd::UploadSound { kind: "board".into(), name, emoji: Some("🤖".into()), wav });
            }
            Err(e) => println!("{} can't read {path}: {e}", ts(start)),
        }
    }
    if args.shelf {
        let ch = shared.model.lock().config.channel.clone();
        let _ = cmd.send(Cmd::Shelf { channel: ch, more: false });
    }
    if args.unread {
        let _ = cmd.send(Cmd::UnreadCounts);
    }
}

/// `--play-sound NAME`: play it once we're in voice and it exists. Returns whether it went out.
fn play_named(args: &Args, shared: &Arc<core::Shared>, cmd: &tokio::sync::mpsc::UnboundedSender<Cmd>, start: Instant) -> bool {
    let Some(want) = &args.play_sound else { return false };
    let id = shared.model.lock().sounds.iter().find(|s| s.kind == "board" && s.name.eq_ignore_ascii_case(want)).map(|s| s.id.clone());
    match id {
        Some(id) => {
            let _ = cmd.send(Cmd::PlaySound(id.clone()));
            println!("{} playing sound {want} ({id})", ts(start));
            true
        }
        None => false,
    }
}

/// A tiny doodle for `--doodle-on`: a 48 × 48 smiley as a PNG.
fn test_doodle() -> Vec<u8> {
    let mut img = image::RgbaImage::from_pixel(48, 48, image::Rgba([255, 255, 255, 255]));
    for (x, y, pix) in img.enumerate_pixels_mut() {
        let (dx, dy) = (x as f32 - 23.5, y as f32 - 23.5);
        let r = (dx * dx + dy * dy).sqrt();
        let eye = ((dx + 8.0).abs() < 3.0 || (dx - 8.0).abs() < 3.0) && (dy + 6.0).abs() < 3.0;
        let mouth = (r - 12.0).abs() < 1.5 && dy > 3.0;
        if (r - 20.0).abs() < 1.5 || eye || mouth {
            *pix = image::Rgba([0, 0, 0, 255]);
        }
    }
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut out, image::ImageFormat::Png);
    out.into_inner()
}
