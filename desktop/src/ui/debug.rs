//! Screenshot aids (`YAP_DEBUG_*`, most of them debug builds only): dialogs
//! and bars opened at start, a picture in the tray, the window saving its
//! own pixels. README › Run lists them.

use super::*;

/// Screenshot aid: the `YAP_DEBUG_OPEN` targets that wait until the model has
/// what they show (connected, history, a voice channel…).
pub(super) const DEBUG_LATER: [&str; 29] = [
    "gifs", "react", "voice", "reply", "draw", "space", "search", "pins", "plan", "catchup", "shelf", "soundboard", "people-away", "delete", "archive", "viewer", "volume", "signin-alert", "weak-pin",
    "weak-password", "temp-password", "password-reset", "hang-now", "voice-more", "edit", "watch", "share", "share-dialog", "burst",
];

/// A small made-up picture (the emoji dialog's screenshot aid).
pub(super) fn debug_picture() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(64, 64, |x, y| {
        let (dx, dy) = (x as f32 - 31.5, y as f32 - 31.5);
        if dx * dx + dy * dy < 900.0 { image::Rgba([0xf5, 0xa5, 0x24, 255]) } else { image::Rgba([0, 0, 0, 0]) }
    });
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut out, image::ImageFormat::Png);
    out.into_inner()
}

/// Screenshot aid (debug builds): `YAP_DEBUG_SHOT=<file.png>` saves the
/// window's own pixels (no screen-capture permission, works with the screen
/// locked) `YAP_DEBUG_SHOT_AFTER` seconds in (default 6), then quits.
pub(super) fn shot(ctx: &egui::Context) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Ok(path) = std::env::var("YAP_DEBUG_SHOT") else { return };
    let after: f64 = std::env::var("YAP_DEBUG_SHOT_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(6.0);
    let id = egui::Id::new("debug-shot-asked");
    let asked = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let shot = ctx.input(|i| {
        i.raw.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    if let Some(img) = shot {
        let [w, h] = img.size;
        let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
        if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) {
            let _ = buf.save(&path);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    } else if !asked && ctx.input(|i| i.time) > after {
        ctx.data_mut(|d| d.insert_temp(id, true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
    }
    ctx.request_repaint_after(Duration::from_millis(200));
}

/// Screenshot aid (debug builds): `YAP_DEBUG_TAB=<n>` moves the keyboard
/// focus on n times, as Tab would, 3 s in (to see the focus marks).
pub(super) fn tab(ctx: &egui::Context) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Some(n) = std::env::var("YAP_DEBUG_TAB").ok().and_then(|v| v.parse::<u32>().ok()) else { return };
    let id = egui::Id::new("debug-tabs");
    let done = ctx.data(|d| d.get_temp::<u32>(id)).unwrap_or(0);
    if done < n && ctx.input(|i| i.time) > 3.0 {
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::Next));
        ctx.data_mut(|d| d.insert_temp(id, done + 1));
        ctx.request_repaint();
    }
}

/// Screenshot aid: `YAP_DEBUG_OPEN=settings[:tab]|channel|update|people|…`
/// opens that dialog at start.
pub(super) fn start_dialog(ctx: &egui::Context, shared: &Shared) -> (Option<dialogs::Modal>, Option<dialogs::SettingsState>) {
    let debug = std::env::var("YAP_DEBUG_OPEN").unwrap_or_default();
    match debug.as_str() {
        d if d.starts_with("settings") => {
            // "settings" or "settings:<tab>".
            let tab = d.strip_prefix("settings:").and_then(|t| t.parse().ok()).unwrap_or(0);
            (Some(dialogs::Modal::Settings), Some(dialogs::SettingsState::new(&shared.model.lock().config).with_tab(tab)))
        }
        "channel" => (Some(dialogs::Modal::NewChannel { name: String::new(), voice: false }), None),
        "update" => (Some(dialogs::Modal::Update), None),
        "people" => (Some(dialogs::Modal::People { open: None, ask: None }), None),
        "unlock" => (Some(dialogs::Modal::Vault(dialogs::VaultDialog::new(dialogs::VaultMode::Unlock))), None),
        "switcher" => (Some(dialogs::Modal::Switcher(extras::Switcher::new())), None),
        "encrypt" => (Some(dialogs::Modal::Vault(dialogs::VaultDialog::new(dialogs::VaultMode::Setup))), None),
        "ping" => (Some(dialogs::Modal::Ping { note: String::new() }), None),
        "link" => (Some(dialogs::Modal::AddLink { name: String::new(), url: String::new() }), None),
        "password" | "setpin" => (Some(dialogs::Modal::Password(Default::default())), None),
        // Debug builds: "Add custom emoji" with YAP_DEBUG_FILE=<picture> (else a made-up one).
        "emoji" if cfg!(debug_assertions) => {
            let file = std::env::var("YAP_DEBUG_FILE").unwrap_or_default();
            let bytes = std::fs::read(&file).unwrap_or_else(|_| debug_picture());
            let name = std::path::Path::new(&file).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "party_parrot".into());
            (Some(dialogs::Modal::EmojiUpload(dialogs::EmojiUpload::new(ctx, bytes, &name))), None)
        }
        // Debug builds: the doodle box, and the sound dialog with YAP_DEBUG_SOUND=<file> (":chime" for a chime).
        "doodle" if cfg!(debug_assertions) => (Some(dialogs::Modal::Doodle(doodle::DoodleBox::sample())), None),
        d if cfg!(debug_assertions) && (d == "sound" || d == "sound:chime") => {
            let file = std::env::var("YAP_DEBUG_SOUND").unwrap_or_default();
            let bytes = std::fs::read(&file).unwrap_or_default();
            let name = std::path::Path::new(&file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            (Some(dialogs::Modal::SoundUpload(sounds::Upload::new(if d.ends_with(":chime") { "chime" } else { "board" }, &name, bytes))), None)
        }
        _ => (None, None),
    }
}

/// Screenshot aid (debug builds): `YAP_DEBUG_STAGE=<picture file>` puts it in the tray.
pub(super) fn stage(app: &mut App) {
    if cfg!(debug_assertions)
        && let Some(bytes) = std::env::var_os("YAP_DEBUG_STAGE").and_then(|f| std::fs::read(f).ok())
    {
        app.stage_picture(bytes);
        app.composer = "the view from the hike 🏔️".into();
        app.draft_channel = app.shared.model.lock().config.channel.clone();
    }
}

impl App {
    /// Screenshot aid: unlock the Personal space with `YAP_DEBUG_VAULT`, and
    /// the `YAP_DEBUG_OPEN` targets that wait until the model has what they show.
    pub(super) fn debug_late(&mut self, ctx: &egui::Context, m: &Model, acts: &mut Vec<Act>) {
        if m.status == Status::Online
            && m.vault.is_some()
            && let Some(pass) = self.debug_vault.take()
            && pass != "-"
        {
            acts.push(Act::Cmd(Cmd::Vault(core::VaultOp::Unlock { pass })));
        }
        match self.debug_open.as_deref() {
            // Debug builds: twelve notes at once (the server takes ten, then asks us to slow
            // down) and a reply to a message that doesn't exist (refused): what sending does then.
            Some("burst") if cfg!(debug_assertions) && m.status == Status::Online && m.history.contains_key(&m.config.channel) => {
                self.debug_open = None;
                for i in 1..=12 {
                    acts.push(Act::Cmd(Cmd::Send { body: format!("Note {i} of 12."), reply_to: None }));
                }
                acts.push(Act::Cmd(Cmd::Send { body: "A reply to a message that's gone.".into(), reply_to: Some("gone".into()) }));
            }
            Some("gifs") if m.server_config.is_some() => {
                self.debug_open = None;
                gifs::toggle(self, ctx, m);
            }
            // Join the first voice channel — only with YAP_DEBUG_AUDIO=silence (no real mic).
            Some("voice") if m.status == Status::Online && self.shared.audio_override.lock().is_some() => {
                if let Some(c) = m.channels.iter().find(|c| c.is_voice()) {
                    acts.push(Act::Cmd(Cmd::JoinVoice(c.id.clone())));
                    self.debug_open = None;
                }
            }
            Some(v @ ("draw" | "space")) if m.status == Status::Online => {
                acts.push(Act::Cmd(Cmd::SetView(Some(if v == "draw" { core::View::Draw } else { core::View::Space }))));
                self.debug_open = None;
            }
            Some("search") if m.status == Status::Online => {
                // YAP_DEBUG_QUERY=<words> fills it (Everywhere); YAP_DEBUG_HERE=1 searches this channel only.
                let everywhere = std::env::var_os("YAP_DEBUG_HERE").is_none();
                self.side = Some(side::Side::Search { q: std::env::var("YAP_DEBUG_QUERY").unwrap_or_default(), everywhere, edited: 0.0, sent: None, focus: false });
                self.debug_open = None;
            }
            Some("pins") if m.status == Status::Online => {
                side::toggle(self, side::Side::Pins, m, acts);
                self.debug_open = None;
            }
            Some("shelf") if m.status == Status::Online => {
                side::toggle(self, side::Side::Shelf, m, acts);
                self.debug_open = None;
            }
            Some("plan") if m.status == Status::Online => {
                self.modal = Some(dialogs::Modal::PlanHang(extras::PlanHang::new(m)));
                self.debug_open = None;
            }
            Some("catchup") if m.status == Status::Online && !m.channels.is_empty() => {
                self.catch_up = Some(egui::pos2(260.0, 150.0));
                self.catch_up_opened = ctx.input(|i| i.time);
                acts.push(Act::Cmd(Cmd::UnreadCounts));
                self.debug_open = None;
            }
            // In a voice channel (YAP_DEBUG_AUDIO=silence), with the soundboard open.
            Some("soundboard") if m.status == Status::Online && self.shared.audio_override.lock().is_some() => {
                if m.voice_channel.is_some() {
                    self.soundboard = Some(egui::pos2(20.0, 240.0));
                    self.soundboard_opened = ctx.input(|i| i.time);
                    self.debug_open = None;
                } else if m.voice_wanted.is_none()
                    && let Some(c) = m.channels.iter().find(|c| c.is_voice())
                {
                    acts.push(Act::Cmd(Cmd::JoinVoice(c.id.clone())));
                }
            }
            // Screen sharing (with YAP_DEBUG_AUDIO=silence): `watch` joins the
            // first sharer's voice channel and watches them; `share` joins the
            // first voice channel and shares the test pattern (needs
            // YAP_DEBUG_SCREEN=pattern: never the real screen); `share-dialog`
            // opens Share Screen once in voice.
            Some("watch") if m.status == Status::Online && self.shared.audio_override.lock().is_some() => {
                let me = m.sid.clone();
                if let Some(s) = m.sessions.values().find(|s| Some(&s.sid) != me.as_ref() && s.voice.as_ref().is_some_and(|v| v.screen)) {
                    acts.push(Act::Cmd(Cmd::WatchScreen(s.sid.clone())));
                    self.debug_open = None;
                }
            }
            Some(what @ ("share" | "share-dialog")) if m.status == Status::Online && self.shared.audio_override.lock().is_some() => {
                if m.voice_channel.is_some() {
                    if what == "share" {
                        if let Some(source) = crate::screen::capture::sources().ok().and_then(|l| l.into_iter().find(|s| s.id == crate::screen::capture::pattern::ID)) {
                            acts.push(Act::Cmd(Cmd::ShareScreen { source, quality: m.config.share_quality, sound: m.config.share_sound }));
                        }
                    } else {
                        screen::open_share_dialog(self, m, false);
                    }
                    self.debug_open = None;
                } else if m.voice_wanted.is_none()
                    && let Some(c) = m.channels.iter().find(|c| c.is_voice())
                {
                    acts.push(Act::Cmd(Cmd::JoinVoice(c.id.clone())));
                }
            }
            // In a voice channel (YAP_DEBUG_AUDIO=silence), with the voice panel's ⋯ open.
            Some("voice-more") if m.status == Status::Online && self.shared.audio_override.lock().is_some() => {
                if m.voice_channel.is_some() {
                    let at = egui::pos2(20.0, 260.0);
                    self.voice_more = Some(at);
                    self.voice_more_opened = ctx.input(|i| i.time);
                    self.debug_open = None;
                } else if m.voice_wanted.is_none()
                    && let Some(c) = m.channels.iter().find(|c| c.is_voice())
                {
                    acts.push(Act::Cmd(Cmd::JoinVoice(c.id.clone())));
                }
            }
            // Edit our own last message here (the composer bar, the message outlined).
            Some("edit") if self.draft_channel == m.config.channel => {
                if let Some(last) = m.last_own_message(&m.config.channel).cloned() {
                    chat::start_edit(self, m, &last);
                    self.debug_open = None;
                }
            }
            // Reply to the last message from someone else here.
            Some("reply") if self.draft_channel == m.config.channel => {
                if let Some(last) = m.history.get(&m.config.channel).and_then(|h| h.messages.iter().rev().find(|x| x.uid != m.config.uid)) {
                    content::start_reply(self, m, last);
                    self.debug_open = None;
                }
            }
            Some("react") => {
                if let Some(last) = m.history.get(&m.config.channel).and_then(|h| h.messages.last()) {
                    self.react_picker = Some((last.id.clone(), egui::pos2(520.0, 300.0), String::new()));
                    self.debug_open = None;
                }
            }
            // People, with someone who's away opened (their volume and status).
            Some("people-away") if m.status == Status::Online && !m.users.is_empty() => {
                let away = m.users.values().filter(|u| u.is_member() && u.id != m.config.uid).find(|u| u.away.as_ref().is_some_and(|a| a.on(core::now_ms())));
                self.modal = Some(dialogs::Modal::People { open: away.map(|u| u.id.clone()), ask: None });
                self.debug_open = None;
            }
            // "Delete this message?" for the newest message here.
            Some("delete") => {
                if let Some(last) = m.history.get(&m.config.channel).and_then(|h| h.messages.last()) {
                    self.modal = Some(dialogs::Modal::ConfirmDelete { id: last.id.clone() });
                    self.debug_open = None;
                }
            }
            Some("archive") if m.status == Status::Online => {
                if let Some(c) = m.channels.iter().find(|c| c.is_text()) {
                    self.modal = Some(dialogs::Modal::ConfirmArchive { id: c.id.clone(), name: c.name.clone() });
                    self.debug_open = None;
                }
            }
            // The newest picture here, full size in the app.
            Some("viewer") => {
                if let Some((id, w, h)) = m.history.get(&m.config.channel).and_then(|h| h.messages.iter().rev().find_map(|x| x.image.as_ref().map(|i| (i.id.clone(), i.width, i.height)))) {
                    let url = format!("{}/media/{id}", m.config.server);
                    self.modal = Some(dialogs::Modal::Viewer(dialogs::Viewer { url, key: None, title: "Picture".into(), size: (w, h), pixel: false, remove: None }));
                    self.debug_open = None;
                }
            }
            // Someone's volume popup (the first other member, A–Z).
            Some("volume") if m.status == Status::Online && !m.users.is_empty() => {
                let mut others: Vec<&User> = m.users.values().filter(|u| u.is_member() && u.id != m.config.uid).collect();
                // Someone sharing their screen first (the popup then offers Watch screen).
                let sharing = |u: &User| m.sessions.values().any(|s| s.uid == u.id && s.voice.as_ref().is_some_and(|v| v.screen));
                others.sort_by_key(|u| (!sharing(u), u.name.to_lowercase()));
                if let Some(u) = others.first() {
                    self.volume_popup = Some((u.id.clone(), egui::pos2(60.0, 300.0)));
                }
                self.debug_open = None;
            }
            // Debug builds: bars as if the server had sent them.
            Some("signin-alert") if cfg!(debug_assertions) && m.status == Status::Online => {
                acts.push(Act::DebugModel(Box::new(|m| {
                    m.signin_alert = Some(crate::protocol::Device { id: "debug".into(), client: "web".into(), label: "Firefox on Windows".into(), created_at: core::now_ms(), last_seen: core::now_ms(), scope: None, current: false });
                })));
                self.debug_open = None;
            }
            // The "Choose a password" prompt, for an old PIN / a guessable password /
            // a reset's temporary password.
            Some(what @ ("weak-pin" | "weak-password" | "temp-password")) if cfg!(debug_assertions) && m.status == Status::Online => {
                let what = what.to_string();
                acts.push(Act::DebugModel(Box::new(move |m| {
                    m.weak_pin = true;
                    m.needs_pin = false;
                    m.secret_kind = if what == "weak-pin" { "pin" } else { "password" }.into();
                    m.weak_reason = Some(match what.as_str() {
                        "weak-pin" => crate::core::WeakReason::Pin,
                        "temp-password" => crate::core::WeakReason::Temporary,
                        _ => crate::core::WeakReason::Guessable,
                    });
                })));
                self.debug_open = None;
            }
            // An owner/mod's result after Reset password….
            Some("password-reset") if cfg!(debug_assertions) && m.status == Status::Online => {
                acts.push(Act::DebugModel(Box::new(|m| m.pin_reset = Some(("Cleo".into(), "maple-river-stone-42".into())))));
                self.debug_open = None;
            }
            // The "hang is starting" bar, for the next hang.
            Some("hang-now") if cfg!(debug_assertions) && m.status == Status::Online && !m.hangs.is_empty() => {
                let id = m.hangs[0].id.clone();
                acts.push(Act::DebugModel(Box::new(move |m| m.hang_now = Some(id))));
                self.debug_open = None;
            }
            _ => {}
        }
    }
}
