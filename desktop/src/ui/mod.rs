//! The window: egui on OpenGL, dressed as Windows 98 / AIM. Like the web
//! client: two windows on a teal desktop — channels on the left, the chat on
//! the right (everyone's in the People sheet) — and a status bar at the bottom.
//!
//! The UI never blocks: it reads `Model` under the lock, collects `Act`ions,
//! and applies them after the lock is released (commands go to the core).

mod chat;
mod content;
mod debug;
mod dialogs;
mod doodle;
mod draw;
mod extras;
mod format;
mod gifs;
mod icons;
mod images;
mod markup;
mod places;
mod screen;
mod sections;
mod side;
mod sidebar;
mod size;
mod sounds;
mod space;
mod statusbar;
mod theme;

use crate::config::Config;
use crate::core::{self, Cmd, Model, Shared, Status, UpdateState};
use crate::protocol::User;
use eframe::egui::{self, Color32, Pos2, Rect, Vec2, vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
pub use format::*;
pub use places::*;
pub use sections::avatar;
pub use size::{AUTO_SIZE, auto_size};
use theme::{Pal, RoundRect};

pub fn run(cfg: Config, path: PathBuf) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("yap-net")
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let ctx_cell: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
    let repaint = {
        let c = ctx_cell.clone();
        Arc::new(move || {
            if let Some(ctx) = c.get() {
                ctx.request_repaint();
            }
        }) as Arc<dyn Fn() + Send + Sync>
    };
    let shared = Shared::new(cfg, path, repaint);
    // Screenshot aid (debug builds): YAP_DEBUG_VAULT=<passcode> keeps the Personal
    // space key in memory only (no keychain) and unlocks with it once connected.
    let debug_vault = if cfg!(debug_assertions) { std::env::var("YAP_DEBUG_VAULT").ok() } else { None };
    if debug_vault.is_some() {
        crate::vault::disable_keychain();
    }
    // Screenshot aid (debug builds): YAP_DEBUG_AUDIO=silence never opens a real mic or
    // speakers; YAP_DEBUG_AUDIO=wav:<file> has that WAV (looped) as the mic instead.
    if cfg!(debug_assertions) {
        match std::env::var("YAP_DEBUG_AUDIO").as_deref() {
            Ok("silence") => *shared.audio_override.lock() = Some(("-".into(), None)),
            Ok(v) if v.starts_with("wav:") => *shared.audio_override.lock() = Some((v["wav:".len()..].into(), None)),
            _ => {}
        }
    }
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let cmd = {
        let _guard = rt.enter();
        core::start_with_done(shared.clone(), None, Some(done_tx))
    };
    // `yap --remote mute` and friends (macOS, Linux): a private socket, as long as the window is open.
    #[cfg(unix)]
    let _remote = match crate::remote::listen(&crate::remote::socket_path(&shared.config_path), core::remote_handler(shared.clone(), cmd.clone())) {
        Ok(l) => Some(l),
        Err(e) => {
            log::info!("remote: off ({e})");
            None
        }
    };
    // Screenshot aid (debug builds): a YAP_DEBUG_SHOT window never takes the focus.
    let shot = cfg!(debug_assertions) && std::env::var_os("YAP_DEBUG_SHOT").is_some();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Yap")
            .with_app_id("yap")
            .with_inner_size([1120.0, 740.0])
            .with_min_inner_size([MIN_W, MIN_H])
            .with_icon(Arc::new(app_icon()))
            .with_active(!shot),
        renderer: eframe::Renderer::Glow,
        event_loop_builder: quiet_shots(shot),
        ..Default::default()
    };
    eframe::run_native(
        "Yap",
        options,
        Box::new(move |cc| {
            let _ = ctx_cell.set(cc.egui_ctx.clone());
            theme::install(&cc.egui_ctx);
            let mut app = App::new(&cc.egui_ctx, shared, cmd, rt, done_rx);
            app.debug_vault = debug_vault;
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| e.to_string())
}

/// The window's smallest size, in UI points (it grows with the zoom).
const MIN_W: f32 = 680.0;
const MIN_H: f32 = 440.0;
/// Search's shortcut (Ctrl/⌘+K is the quick switcher now).
/// The voice panel's warning when the mic sends nothing but digital silence (the web's words).
pub const SILENT_TEXT: &str = "Your mic is sending silence";
/// Push-to-talk latched open by a double-tap (the spec's words).
pub const LIVE_TEXT: &str = "Live — tap to stop";
pub const SILENT_TIP: &str = "Nothing at all is coming from your microphone. Check its mute switch, or pick another one in Settings › Voice.";
pub const SEARCH_KEYS: &str = if cfg!(target_os = "macos") { "⌘F" } else { "Ctrl+Shift+F" };

/// Things the UI wants done once the model lock is released (a handful a
/// frame, so the big `Cmd` variant doesn't matter).
#[allow(clippy::large_enum_variant)]
pub enum Act {
    Cmd(Cmd),
    Config(Box<dyn FnOnce(&mut Config)>),
    ApplyGate,
    Copy(String, &'static str),
    OpenUrl(String),
    Toast(String),
    /// The message list consumed the model's scroll hint.
    ClearScroll,
    /// The message list took the core's jump target.
    ClearJump,
    /// Space played the first n queued mini-game updates.
    ClearSpaceGames(usize),
    /// The first n refused messages' texts went back into their message boxes.
    ClearRestored(usize),
    /// Put a (pasted) picture in the tray above the composer.
    StagePicture(Vec<u8>),
    /// Open the file picker.
    PickFile(PickFor),
    /// macOS 14+: open Apple's screen picker; what's picked is shared at this quality, with
    /// or without sound (`switching`: Share something else instead).
    PickScreen { quality: crate::screen::Quality, sound: bool, switching: bool },
    /// Open the Personal space passcode dialog (from Settings, which closes).
    Vault(dialogs::VaultMode),
    /// Hide the "New sign-in" bar.
    DismissSigninAlert,
    /// Open Settings on a tab.
    OpenSettings(usize),
    /// Later on the "Choose a password" prompt: not again on this device, for
    /// that reason.
    PasswordLater(crate::core::WeakReason),
    /// Settings › Account, asking "Sign out everywhere else?" (the sign-in alert).
    AskSignOutOthers,
    ClearPinReset,
    /// The core's `open_url` was opened.
    ClearOpenUrl,
    /// Stop the sounds playing now (the soundboard was muted).
    StopSounds,
    /// Screenshot aid (debug builds): put the model in a state the server
    /// would (a sign-in alert, the "Choose a password" prompt, a hang starting).
    DebugModel(Box<dyn FnOnce(&mut Model)>),
}

pub struct App {
    shared: Arc<Shared>,
    cmd: tokio::sync::mpsc::UnboundedSender<Cmd>,
    rt: Option<tokio::runtime::Runtime>,
    done_rx: std::sync::mpsc::Receiver<()>,
    images: images::Images,
    pub composer: String,
    drafts: HashMap<String, String>,
    draft_channel: String,
    focus_composer: bool,
    scroll: chat::ScrollState,
    modal: Option<dialogs::Modal>,
    /// A dialog to open once the current one closes.
    next_modal: Option<dialogs::Modal>,
    welcome: dialogs::Welcome,
    settings: Option<dialogs::SettingsState>,
    emoji_picker: Option<String>,
    volume_popup: Option<(String, Pos2)>,
    /// The popup is asking "really reset their password / ban them?".
    volume_ask: Option<(String, dialogs::Ask)>,
    body_cache: HashMap<String, Arc<Vec<markup::Block>>>,
    /// Lowercase display names, longest first (for @mention highlights).
    names: Vec<String>,
    names_len: usize,
    gif_picker: gifs::GifPicker,
    /// Screenshot aid: YAP_DEBUG_OPEN=gifs|react opens that picker once config/history arrive.
    debug_open: Option<String>,
    /// Reaction picker: (message id, where to open it, search).
    react_picker: Option<(String, Pos2, String)>,
    title: String,
    was_focused: bool,
    shortcodes: Arc<OnceLock<HashMap<String, String>>>,
    shortcodes_requested: bool,
    h12: bool,
    /// The window with the navy title bar.
    active: Pane,
    pane_rects: Vec<(Pane, Rect)>,
    relaunching: bool,
    /// Editing one of our messages in the composer.
    pub editing: Option<chat::Editing>,
    /// A file picker in flight: what it's for, and where the answer lands.
    file_pick: Option<(PickFor, PickSlot)>,
    /// Apple's screen picker in flight (macOS 14+).
    screen_pick: Option<ScreenPick>,
    /// When text was last pasted (a Cmd+V release right after isn't a picture paste).
    last_text_paste: f64,
    /// A GIF or picture waiting above the composer (preview before sending).
    pub staged: Option<Staged>,
    /// Replying to a message (the bar over the composer).
    pub replying: Option<content::Replying>,
    /// Jump to this message (a reply's quote was clicked) / which one flashes, since when.
    pub jump: Option<String>,
    pub flash: Option<(String, f64)>,
    /// The message just put in the composer to edit: brought into view once.
    pub edit_scroll: Option<String>,
    /// Long messages opened with "Show more".
    pub expanded: HashSet<String>,
    /// Messages whose spoilers were clicked open (this run only, on purpose).
    pub spoilers: HashSet<String>,
    /// Open popovers: Catch up / the soundboard (where they hang).
    pub catch_up: Option<Pos2>,
    pub soundboard: Option<Pos2>,
    /// When they opened (the click that opened one mustn't close it).
    pub catch_up_opened: f64,
    pub soundboard_opened: f64,
    /// The voice panel's ⋯ (the channel's sound quality), where it hangs.
    pub voice_more: Option<Pos2>,
    pub voice_more_opened: f64,
    /// The "hang is starting" notice was closed (this hang).
    pub hang_dismissed: Option<String>,
    /// The panel on the right: Search or Pinned.
    pub side: Option<side::Side>,
    /// Where that panel floats over the chat this frame (a narrow window), if it does.
    pub side_float: Option<Rect>,
    /// Draw and Space, inside the app.
    pub draw: draw::DrawUi,
    pub space: space::SpaceUi,
    /// The shared view shown last frame (to notice leaving one).
    last_view: Option<core::View>,
    /// The window was sized for the UI size once the monitor was known.
    sized: bool,
    /// The zoom the window's minimum size was last set for.
    min_size_zoom: f32,
    /// The zoom last frame, and for how many frames it has held.
    zoom_seen: f32,
    zoom_still: u32,
    /// Auto size for this monitor, once known.
    auto: Option<f32>,
    /// Screenshot aid: unlock the Personal space with this once connected ("-" = stay locked).
    debug_vault: Option<String>,
    /// Encrypted notes, decrypted (by ciphertext), for this key.
    pub notes: content::NoteCache,
    pub notes_key: Option<crate::vault::Key>,
    /// The screen we're watching: its picture, full screen, popped out.
    pub screen: screen::ScreenUi,
    /// Ctrl/⌘ + scroll over a voice row, adding up to a 5 % step.
    pub wheel_volume: sections::WheelVolume,
}

/// Preview before sending: what's waiting in the tray above the composer.
pub enum Staged {
    Gif { gif: crate::protocol::Gif, query: String },
    Picture {
        /// What you picked (kept to redo it when "Full res" flips).
        bytes: Arc<Vec<u8>>,
        /// "Full res": the original if it fits, instead of a smaller copy.
        full: bool,
        /// Compressing in the background while you type.
        slot: crate::picture::PrepSlot,
        thumb: Arc<parking_lot::Mutex<Option<image::RgbaImage>>>,
        tex: Option<egui::TextureHandle>,
    },
}

/// Where a file picker's answer lands: None = still open; Some(None) = cancelled.
type PickSlot = Arc<parking_lot::Mutex<Option<Option<(String, Vec<u8>)>>>>;

/// Where Apple's screen picker's answer lands: None = still open (see `capture::OnPicked`).
type ScreenPickSlot = Arc<parking_lot::Mutex<Option<Result<Option<crate::screen::capture::Source>, String>>>>;

/// Apple's screen picker is open: where its answer lands, and what to do with it.
struct ScreenPick {
    slot: ScreenPickSlot,
    quality: crate::screen::Quality,
    sound: bool,
    switching: bool,
}

/// What a picked file is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickFor {
    Picture,
    Emoji,
    /// A soundboard clip, or (`chime`) your join chime.
    Sound { chime: bool },
}

/// The windows on the desktop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Channels,
    Chat,
    Side,
}

impl App {
    fn new(ctx: &egui::Context, shared: Arc<Shared>, cmd: tokio::sync::mpsc::UnboundedSender<Cmd>, rt: tokio::runtime::Runtime, done_rx: std::sync::mpsc::Receiver<()>) -> Self {
        let (theme_pref, size) = {
            let m = shared.model.lock();
            (m.config.theme, m.config.size)
        };
        apply_theme(ctx, theme_pref);
        // Settings › Look › Size; the shortcuts are ours (they persist), not egui's.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        ctx.set_zoom_factor(size.unwrap_or(AUTO_SIZE));
        let welcome = dialogs::Welcome::from_config(&shared.model.lock().config);
        let (modal, settings) = debug::start_dialog(ctx, &shared);
        let mut app = Self {
            shared,
            cmd,
            rt: Some(rt),
            done_rx,
            images: images::Images::new(ctx),
            composer: String::new(),
            drafts: HashMap::new(),
            draft_channel: String::new(),
            focus_composer: true,
            scroll: chat::ScrollState::default(),
            modal,
            next_modal: None,
            welcome,
            settings,
            emoji_picker: None,
            volume_popup: None,
            volume_ask: None,
            body_cache: HashMap::new(),
            names: Vec::new(),
            names_len: usize::MAX,
            react_picker: None,
            gif_picker: gifs::GifPicker::default(),
            debug_open: std::env::var("YAP_DEBUG_OPEN").ok().filter(|v| debug::DEBUG_LATER.contains(&v.as_str())),
            title: String::new(),
            was_focused: true,
            shortcodes: Arc::new(OnceLock::new()),
            shortcodes_requested: false,
            h12: uses_12h(),
            active: Pane::Chat,
            pane_rects: Vec::new(),
            relaunching: false,
            editing: None,
            file_pick: None,
            screen_pick: None,
            last_text_paste: -10.0,
            staged: None,
            replying: None,
            jump: None,
            flash: None,
            edit_scroll: None,
            expanded: HashSet::new(),
            spoilers: HashSet::new(),
            catch_up: None,
            soundboard: None,
            catch_up_opened: 0.0,
            soundboard_opened: 0.0,
            voice_more: None,
            voice_more_opened: 0.0,
            hang_dismissed: None,
            notes: HashMap::new(),
            notes_key: None,
            debug_vault: None,
            side: None,
            side_float: None,
            draw: draw::DrawUi::default(),
            space: space::SpaceUi::default(),
            last_view: None,
            sized: false,
            min_size_zoom: 0.0,
            zoom_seen: 0.0,
            zoom_still: 0,
            auto: None,
            screen: screen::ScreenUi::default(),
            wheel_volume: Default::default(),
        };
        debug::stage(&mut app);
        app
    }

    /// The tray's thumbnail: (texture, pixel size), once it's loaded.
    pub fn staged_thumb(&mut self, ctx: &egui::Context) -> Option<(egui::TextureId, Vec2)> {
        match self.staged.as_mut()? {
            Staged::Gif { gif, .. } => {
                let url = gif.still.as_ref().or(gif.webp.as_ref()).or(gif.gif.as_ref())?.clone();
                self.images.get(&url, false)
            }
            Staged::Picture { thumb, tex, .. } => {
                if tex.is_none()
                    && let Some(img) = thumb.lock().take()
                {
                    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                    *tex = Some(ctx.load_texture("staged-picture", ci, egui::TextureOptions::LINEAR));
                }
                tex.as_ref().map(|t| (t.id(), t.size_vec2()))
            }
        }
    }

    /// Open the system file picker (async; the answer is picked up next frames).
    pub fn pick_file(&mut self, purpose: PickFor) {
        if self.file_pick.is_some() {
            return;
        }
        let Some(rt) = self.rt.as_ref() else { return };
        let slot = Arc::new(parking_lot::Mutex::new(None));
        let out = slot.clone();
        let title = match purpose {
            PickFor::Picture => "Send a picture",
            PickFor::Emoji => "Add a custom emoji",
            PickFor::Sound { chime: false } => "Add a sound",
            PickFor::Sound { chime: true } => "Your join chime",
        };
        let (kind, exts): (&str, &[&str]) = match purpose {
            PickFor::Sound { .. } => ("Sounds", &["wav", "mp3", "m4a", "aac", "ogg", "oga", "opus", "flac", "mp4", "caf", "aif", "aiff"]),
            _ => ("Pictures", &["png", "jpg", "jpeg", "gif", "webp"]),
        };
        rt.spawn(async move {
            let picked = rfd::AsyncFileDialog::new().set_title(title).add_filter(kind, exts).pick_file().await;
            let result = match picked {
                Some(f) => Some((f.file_name(), f.read().await)),
                None => None,
            };
            *out.lock() = Some(result);
        });
        self.file_pick = Some((purpose, slot));
    }

    fn poll_file_pick(&mut self, ctx: &egui::Context) {
        let Some((purpose, slot)) = &self.file_pick else { return };
        let Some(result) = slot.lock().take() else {
            ctx.request_repaint_after(Duration::from_millis(150));
            return;
        };
        let purpose = *purpose;
        self.file_pick = None;
        let Some((name, bytes)) = result else { return };
        match purpose {
            PickFor::Picture => self.stage_picture(bytes),
            PickFor::Emoji => {
                let stem = std::path::Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "emoji".into());
                self.emoji_picker = None;
                self.modal = Some(dialogs::Modal::EmojiUpload(dialogs::EmojiUpload::new(ctx, bytes, &stem)));
            }
            PickFor::Sound { chime } => {
                self.modal = Some(dialogs::Modal::SoundUpload(sounds::Upload::new(if chime { "chime" } else { "board" }, &name, bytes)));
            }
        }
    }

    /// macOS 14+: Apple's screen picker. What's picked starts the share (or, `switching`,
    /// replaces what it shows); cancelling it does nothing.
    pub fn pick_screen(&mut self, quality: crate::screen::Quality, sound: bool, switching: bool) {
        let slot = Arc::new(parking_lot::Mutex::new(None));
        let out = slot.clone();
        let repaint = self.shared.repaint.clone();
        self.screen_pick = Some(ScreenPick { slot, quality, sound, switching });
        crate::screen::capture::pick(Box::new(move |result| {
            *out.lock() = Some(result);
            repaint();
        }));
    }

    fn poll_screen_pick(&mut self) {
        let Some(result) = self.screen_pick.as_ref().and_then(|p| p.slot.lock().take()) else { return };
        let Some(p) = self.screen_pick.take() else { return };
        match result {
            Ok(Some(source)) if p.switching => self.send(Cmd::ShareSource(source)),
            Ok(Some(source)) => self.send(Cmd::ShareScreen { source, quality: p.quality, sound: p.sound }),
            Ok(None) => {}
            Err(e) => {
                // Apple's picker couldn't open: Yap's own list instead (macOS may ask about
                // Screen Recording then, as it did before).
                log::warn!("screen: {e}; offering Yap's own list instead");
                let config = self.shared.model.lock().config.clone();
                self.modal = Some(dialogs::Modal::Share(dialogs::ShareDialog::with_list(&config, p.switching)));
            }
        }
    }

    /// Put a picture in the tray (it shrinks in the background); Send posts it
    /// with whatever's typed as the caption.
    pub fn stage_picture(&mut self, bytes: Vec<u8>) {
        if self.editing.is_some() {
            chat::cancel_edit(self);
        }
        let thumb = Arc::new(parking_lot::Mutex::new(None));
        let repaint = self.shared.repaint.clone();
        let bytes = Arc::new(bytes);
        let slot = crate::picture::prepare_in_background(bytes.clone(), false, Some(thumb.clone()), move || repaint());
        self.staged = Some(Staged::Picture { bytes, full: false, slot, thumb, tex: None });
        self.emoji_picker = None;
        self.gif_picker.open = false;
        self.focus_composer = true;
    }

    /// The tray's "Full res" flipped: prepare the picture again that way.
    pub fn set_full_res(&mut self, on: bool) {
        let repaint = self.shared.repaint.clone();
        if let Some(Staged::Picture { bytes, full, slot, .. }) = self.staged.as_mut()
            && *full != on
        {
            *full = on;
            *slot = crate::picture::prepare_in_background(bytes.clone(), on, None, move || repaint());
        }
    }

    pub fn send(&self, c: Cmd) {
        let _ = self.cmd.send(c);
    }

    pub fn body(&mut self, text: &str) -> Arc<Vec<markup::Block>> {
        if self.body_cache.len() > 4000 {
            self.body_cache.clear();
        }
        let names = &self.names;
        self.body_cache.entry(text.to_string()).or_insert_with(|| Arc::new(markup::parse(text, names))).clone()
    }

    /// Re-derive the @mention name list when the member list changes.
    fn refresh_names(&mut self, m: &Model) {
        if m.users.len() == self.names_len {
            return;
        }
        self.names_len = m.users.len();
        let mut names: Vec<String> = m.users.values().map(|u| u.name.to_lowercase()).collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        self.names = names;
        self.body_cache.clear();
    }

    fn apply(&mut self, ctx: &egui::Context, acts: Vec<Act>) {
        for a in acts {
            match a {
                Act::Cmd(c) => self.send(c),
                Act::Config(f) => {
                    let mut m = self.shared.model.lock();
                    f(&mut m.config);
                    m.config_dirty = true;
                }
                Act::ApplyGate => self.shared.apply_gate(),
                Act::Copy(text, note) => {
                    ctx.copy_text(text);
                    self.shared.model.lock().toast(note);
                }
                Act::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
                Act::Toast(t) => self.shared.model.lock().toast(t),
                Act::ClearScroll => self.shared.model.lock().scroll = None,
                Act::ClearJump => self.shared.model.lock().jump_to = None,
                Act::ClearSpaceGames(n) => {
                    let mut m = self.shared.model.lock();
                    let n = n.min(m.space.games.len());
                    m.space.games.drain(..n);
                }
                Act::ClearRestored(n) => {
                    let mut m = self.shared.model.lock();
                    let n = n.min(m.composer_restore.len());
                    m.composer_restore.drain(..n);
                }
                Act::StagePicture(bytes) => self.stage_picture(bytes),
                Act::PickFile(purpose) => self.pick_file(purpose),
                Act::PickScreen { quality, sound, switching } => self.pick_screen(quality, sound, switching),
                Act::Vault(mode) => self.modal = Some(dialogs::Modal::Vault(dialogs::VaultDialog::new(mode))),
                Act::DismissSigninAlert => self.shared.model.lock().signin_alert = None,
                Act::OpenSettings(tab) => {
                    let st = dialogs::SettingsState::new(&self.shared.model.lock().config).with_tab(tab);
                    self.settings = Some(st);
                    self.modal = Some(dialogs::Modal::Settings);
                }
                Act::AskSignOutOthers => {
                    self.settings = Some(dialogs::SettingsState::new(&self.shared.model.lock().config).asking_signout_others());
                    self.modal = Some(dialogs::Modal::Settings);
                }
                Act::PasswordLater(reason) => {
                    let mut m = self.shared.model.lock();
                    if !m.config.password_later.iter().any(|r| r == reason.as_str()) {
                        m.config.password_later.push(reason.as_str().into());
                        m.config_dirty = true;
                    }
                }
                Act::ClearPinReset => self.shared.model.lock().pin_reset = None,
                Act::ClearOpenUrl => self.shared.model.lock().open_url = None,
                Act::StopSounds => self.shared.audio.stop_clips(),
                Act::DebugModel(f) => f(&mut self.shared.model.lock()),
            }
        }
    }

    /// Standard emoji shortcodes (`:smile:`), from the server's emoji-data.json.
    fn load_shortcodes(&mut self, server: &str) {
        if self.shortcodes_requested || server.is_empty() {
            return;
        }
        self.shortcodes_requested = true;
        let cell = self.shortcodes.clone();
        let url = format!("{server}/emoji-data.json");
        let _ = std::thread::Builder::new().name("yap-emoji-data".into()).spawn(move || {
            let mut map = HashMap::new();
            if let Ok(v) = crate::http::get_json(&url)
                && let Some(list) = v.get("emojis").and_then(|e| e.as_array())
            {
                for e in list {
                    let (Some(ch), Some(codes)) = (e.get(0).and_then(|x| x.as_str()), e.get(2).and_then(|x| x.as_str())) else { continue };
                    for code in codes.split_whitespace() {
                        map.entry(code.to_string()).or_insert_with(|| ch.to_string());
                    }
                }
            }
            let _ = cell.set(map);
        });
    }

    fn handle_global_input(&mut self, ctx: &egui::Context) {
        self.apply_size(ctx);
        // In-window keys (fallback where global polling can't see keys) and key binding.
        let ptt = self.shared.ptt.clone();
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key { key: k, pressed, repeat: false, modifiers, .. } = ev
                    && let Some(name) = crate::ptt::from_egui(*k)
                {
                    if ptt.binding().is_some() && *pressed && *k != egui::Key::Escape {
                        if let Some(combo) = crate::ptt::combo_from_egui(*k, *modifiers) {
                            ptt.offer_bound(&combo);
                        }
                    } else {
                        ptt.window_key(name, *pressed, *modifiers);
                    }
                }
            }
            if ptt.binding().is_some() && i.key_pressed(egui::Key::Escape) {
                ptt.cancel_binding();
            }
        });
        if let Some((action, combo)) = ptt.take_bound() {
            let mut m = self.shared.model.lock();
            match crate::ptt::bind(&mut m.config, action, &combo) {
                Ok(()) => m.config_dirty = true,
                Err(why) => m.toast(why),
            }
            drop(m);
            self.shared.apply_gate();
        }
        let focused = ctx.input(|i| i.focused);
        if focused != self.was_focused {
            self.was_focused = focused;
            if !focused {
                ptt.window_blurred();
            }
            if focused {
                let _ = self.cmd.send(Cmd::Wake);
            }
            let mut m = self.shared.model.lock();
            m.window_focused = focused;
            if focused {
                let ch = m.config.channel.clone();
                m.mark_read(&ch, None);
            }
        }
        // Drop a picture onto the window to send it (like the web client).
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(f) = dropped.into_iter().next() {
            let online = self.shared.model.lock().status == Status::Online;
            match f.bytes() {
                Ok(b) if online && self.modal.is_none() => self.stage_picture(b),
                Ok(_) => self.shared.model.lock().toast("Connect first, then drop the picture again."),
                Err(_) => self.shared.model.lock().toast("Couldn't read that file."),
            }
        }
        self.poll_file_pick(ctx);
        self.poll_screen_pick();
        // Mute and deafen from the keyboard in the window, like the web: Ctrl/⌘+Shift+M and
        // Ctrl/⌘+Shift+D (unless a global key already is that combination: it toggles itself).
        let (mute_free, deafen_free) = {
            let m = self.shared.model.lock();
            (crate::ptt::shortcut_free(&m.config, crate::ptt::MUTE_SHORTCUT), crate::ptt::shortcut_free(&m.config, crate::ptt::DEAFEN_SHORTCUT))
        };
        let both = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        if mute_free && ctx.input_mut(|i| i.consume_key(both, egui::Key::M)) {
            let _ = self.cmd.send(Cmd::ToggleMute);
        }
        if deafen_free && ctx.input_mut(|i| i.consume_key(both, egui::Key::D)) {
            let _ = self.cmd.send(Cmd::ToggleDeafen);
        }
        // Ctrl/⌘+K: the quick switcher. Search: Ctrl/⌘+Shift+F (and ⌘F on a Mac).
        let online = self.shared.model.lock().status == Status::Online;
        if online && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K)) {
            match self.modal {
                Some(dialogs::Modal::Switcher(_)) => self.modal = None,
                None => self.modal = Some(dialogs::Modal::Switcher(extras::Switcher::new())),
                _ => {}
            }
        }
        let search = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::F) || cfg!(target_os = "macos") && i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
        if online && self.modal.is_none() && search {
            match &mut self.side {
                Some(side::Side::Search { focus, .. }) => *focus = true,
                _ => self.side = Some(side::Side::search()),
            }
        }
        // Esc closes the panel when nothing else wants it.
        if self.side.is_some() && self.modal.is_none() && self.editing.is_none() && self.staged.is_none() && self.replying.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.memory(|m| m.focused().is_some_and(|f| f == egui::Id::new("composer"))) {
            self.side = None;
            let _ = self.cmd.send(Cmd::Search { q: String::new(), channel: None });
        }
    }

    fn update_title(&mut self, ctx: &egui::Context, m: &Model) {
        let unread = m.channels.iter().filter(|c| m.is_unread(c)).count();
        let title = if unread > 0 { format!("({unread}) Yap") } else { "Yap".into() };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        debug::tab(&ctx);
        debug::shot(&ctx);
        self.images.pump();
        self.handle_global_input(&ctx);
        let p = theme::pal(&ctx);
        let shared = self.shared.clone();
        let mut acts: Vec<Act> = Vec::new();
        theme::desktop(ui.painter(), ui.max_rect(), &p);
        // Clicking a window brings its title bar to life (like Windows).
        if ctx.input(|i| i.pointer.any_pressed())
            && let Some(pos) = ctx.input(|i| i.pointer.interact_pos())
            && let Some((pane, _)) = self.pane_rects.iter().find(|(_, r)| r.contains(pos))
        {
            self.active = *pane;
        }
        {
            let m = shared.model.lock();
            if m.view != self.last_view {
                if self.last_view == Some(core::View::Space) {
                    self.space.reset();
                }
                self.last_view = m.view;
            }
            self.refresh_names(&m);
            // The core has a web page to open (a signed-in handoff link).
            if let Some(url) = &m.open_url {
                acts.push(Act::OpenUrl(url.clone()));
                acts.push(Act::ClearOpenUrl);
            }
            if m.status == Status::Online {
                let server = m.config.server.clone();
                self.load_shortcodes(&server);
            }
            self.update_title(&ctx, &m);
            // An update was installed: quit, and main() starts the new version.
            if matches!(m.update, UpdateState::Installed(_)) && !self.relaunching {
                self.relaunching = true;
                crate::update::mark_relaunch();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            self.debug_late(&ctx, &m, &mut acts);
            self.screen.pump(&ctx, &shared, &m);
            // Full screen: the screen we're watching is the whole window.
            let full = m.status == Status::Online && m.screen.watching.is_some() && self.screen.full;
            if !full {
                statusbar::status_bar(self, ui, &m, &p, &mut acts);
            }
            match m.status {
                _ if full => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ui, |ui| screen::fullscreen(self, ui, &m));
                }
                _ if self.welcome.forced => dialogs::welcome(self, ui, &m, &p, &mut acts),
                Status::Setup | Status::BadKey | Status::SignIn => dialogs::welcome(self, ui, &m, &p, &mut acts),
                Status::Pending | Status::Removed => dialogs::waiting(self, ui, &m, &p, &mut acts),
                _ => {
                    // Every window's title bar looks active (no focus swap: it flashed on each click).
                    let g = theme::GAP;
                    let half = g / 2;
                    let mut rects = Vec::new();
                    // Three windows on the desktop, like the web client.
                    let r = egui::Panel::left("sidebar")
                        .exact_size(236.0 + (g + half) as f32)
                        .resizable(false)
                        .show_separator_line(false)
                        .frame(theme::window_frame(&p, egui::Margin { left: g, right: half, top: g, bottom: g }))
                        .show(ui, |ui| sidebar::sidebar(self, ui, &m, &p, &mut acts));
                    rects.push((Pane::Channels, r.response.rect));
                    // No buddy list: everyone is in the People sheet (no online/offline, on purpose).
                    // Search / Pinned / the shelf open as a third window on the
                    // right, or (no room for it) a window floating over the chat.
                    let side_open = self.side.is_some() && ui.available_width() > 560.0;
                    if side_open {
                        let r = egui::Panel::right("side")
                            .exact_size(side::WIDTH + (g + half) as f32)
                            .resizable(false)
                            .show_separator_line(false)
                            .frame(theme::window_frame(&p, egui::Margin { left: half, right: g, top: g, bottom: g }))
                            .show(ui, |ui| side::panel(self, ui, &m, &p, &mut acts));
                        rects.push((Pane::Side, r.response.rect));
                    }
                    let avail = ui.available_rect_before_wrap();
                    self.side_float = (self.side.is_some() && !side_open).then(|| {
                        let (g, w) = (g as f32, side::WIDTH + 2.0 * theme::PAD as f32);
                        let w = w.min(avail.width() - 2.0 * g);
                        // Under the chat's title bar, so its buttons still switch or close it.
                        let top = avail.top() + g + theme::PAD as f32 + theme::TITLE_H + 3.0;
                        Rect::from_min_max(egui::pos2(avail.right() - g - w, top), egui::pos2(avail.right() - g, avail.bottom() - g))
                    });
                    let right = if side_open { half } else { g };
                    let r = egui::CentralPanel::default()
                        .frame(theme::window_frame(&p, egui::Margin { left: half, right, top: g, bottom: g }))
                        .show(ui, |ui| chat::chat(self, ui, &m, &p, &mut acts));
                    if let Some(rect) = self.side_float {
                        side::overlay(self, &ctx, &m, &p, &mut acts, rect);
                        rects.push((Pane::Side, rect));
                    }
                    rects.push((Pane::Chat, r.response.rect));
                    self.pane_rects = rects;
                    screen::popout(self, &ctx, &m, &p, &mut acts);
                    sidebar::volume_popup(self, &ctx, &m, &p, &mut acts);
                    extras::catch_up_popover(self, &ctx, &m, &p, &mut acts);
                    extras::soundboard_popover(self, &ctx, &m, &p, &mut acts);
                    voice_more_popover(self, &ctx, &m, &p, &mut acts);
                    chat::react_picker(self, &ctx, &m, &p, &mut acts);
                    dialogs::modals(self, &ctx, &m, &p, &mut acts);
                }
            }
            if self.modal.is_some() && matches!(m.status, Status::Setup | Status::BadKey | Status::SignIn | Status::Pending | Status::Removed) {
                // The update dialog can open from the status bar before you're in.
                dialogs::modals(self, &ctx, &m, &p, &mut acts);
            }
            sections::toasts(&ctx, &m);
            // Keep the voice panel's numbers and the mic meter fresh while they matter.
            if m.voice_channel.is_some() || matches!(m.update, UpdateState::Downloading { .. }) {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
            if m.toasts.front().is_some() {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
        self.apply(&ctx, acts);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.cmd.send(Cmd::Quit);
        let _ = self.done_rx.recv_timeout(Duration::from_millis(1500));
        if let Some(rt) = self.rt.take() {
            rt.shutdown_timeout(Duration::from_millis(300));
        }
    }
}

pub fn apply_theme(ctx: &egui::Context, pref: crate::config::ThemePref) {
    ctx.set_theme(match pref {
        crate::config::ThemePref::System => egui::ThemePreference::System,
        crate::config::ThemePref::Light => egui::ThemePreference::Light,
        crate::config::ThemePref::Dark => egui::ThemePreference::Dark,
    });
}

/// Who to draw: name (for the initial), colour, picture.
#[derive(Clone, Copy)]
pub struct Face<'a> {
    pub name: &'a str,
    pub color: &'a str,
    pub avatar: Option<&'a str>,
}

pub fn face(user: Option<&User>) -> Face<'_> {
    match user {
        Some(u) => Face { name: &u.name, color: &u.color, avatar: u.avatar.as_deref() },
        None => Face { name: "?", color: "#8d8d8d", avatar: None },
    }
}

/// A full-width clickable row (sidebar channel, member). Returns (rect, response).
pub fn row(ui: &mut egui::Ui, height: f32, id: impl std::hash::Hash + std::fmt::Debug) -> (Rect, egui::Response) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, height), egui::Sense::hover());
    let resp = ui.interact(rect, ui.id().with(id), egui::Sense::click());
    (rect, resp)
}

pub fn primary_button(ui: &mut egui::Ui, text: &str, p: &Pal, enabled: bool) -> egui::Response {
    theme::button(ui, text, theme::Btn::Default, enabled, p)
}

/// Screenshot aid (debug builds): on a Mac, a window opened for
/// `YAP_DEBUG_SHOT` doesn't activate the app (no Dock icon, never frontmost),
/// so keys typed elsewhere meanwhile stay there.
#[cfg(target_os = "macos")]
fn quiet_shots(shot: bool) -> Option<eframe::EventLoopBuilderHook> {
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    shot.then(|| {
        Box::new(|b: &mut eframe::EventLoopBuilder<eframe::UserEvent>| {
            b.with_activation_policy(ActivationPolicy::Accessory).with_activate_ignoring_other_apps(false);
        }) as eframe::EventLoopBuilderHook
    })
}

#[cfg(not(target_os = "macos"))]
fn quiet_shots(_shot: bool) -> Option<eframe::EventLoopBuilderHook> {
    None
}

/// The window icon: a white speech bubble with a black outline on a
/// title-bar-blue square (drawn, no assets).
fn app_icon() -> egui::IconData {
    let n = 64usize;
    let mut rgba = vec![0u8; n * n * 4];
    let (a, b) = ([0x00 as f32, 0x00 as f32, 0x80 as f32], [0x10 as f32, 0x84 as f32, 0xd0 as f32]);
    let bubble = |fx: f32, fy: f32, grow: f32| -> bool {
        let (ex, ey) = ((fx - 32.0) / (21.0 + grow), (fy - 28.0) / (16.0 + grow));
        let tail = fx > 16.0 - grow && fx < 30.0 + grow && fy > 36.0 && fy < 53.0 + grow && (fy - 36.0) < (30.0 + grow - fx) * 1.2;
        ex * ex + ey * ey <= 1.0 || tail
    };
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let i = (y * n + x) * 4;
            let px = if bubble(fx, fy, 0.0) {
                [255, 255, 255, 255]
            } else if bubble(fx, fy, 3.0) {
                [0, 0, 0, 255]
            } else {
                let t = fx / n as f32;
                let c = |k: usize| (a[k] + (b[k] - a[k]) * t) as u8;
                [c(0), c(1), c(2), 255]
            };
            rgba[i..i + 4].copy_from_slice(&px);
        }
    }
    egui::IconData { rgba, width: n as u32, height: n as u32 }
}
