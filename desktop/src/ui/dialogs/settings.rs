//! Settings: a property sheet over the tabs Look, Profile, Voice,
//! Notifications, Account, Owner & mods, Invite and About.

use super::*;
use crate::ptt::Action;
use std::collections::HashSet;

/// Audio devices (inputs, outputs), listed in the background.
pub(super) type DeviceLists = Arc<Mutex<Option<(Vec<String>, Vec<String>)>>>;
pub struct SettingsState {
    tab: usize,
    /// Settings › Invite: who the next link is for.
    invite_label: String,
    asked_invites: bool,
    asked_modlog: bool,
    name: String,
    /// Account › Change password / Set a password.
    password: PasswordForm,
    asked_devices: bool,
    devices: DeviceLists,
    /// Profile › Away message: the draft and its "until" choice.
    away_text: String,
    away_until: usize,
    /// Voice › "Test mic" is on: the mic is open (only while the Voice tab shows).
    pub mic_test: bool,
    /// Account › "Sign out of this computer" is asking "really?".
    confirm_signout: bool,
    /// Account › "Sign out everywhere else" is asking "really?" (the web asks too).
    confirm_others: bool,
    /// Invite › "Turn off" is asking about this link.
    confirm_revoke: Option<String>,
    /// Invite › Reset main link (owner): the warning, then the new link.
    reset_main: Option<ResetMain>,
    /// Voice: asked the server whether it has a relay.
    asked_relay: bool,
    /// Screenshot aid: YAP_DEBUG_HEAR started Hear yourself once.
    hear_asked: bool,
    /// Voice › Advanced is open (folded again each time Settings opens).
    advanced: bool,
    /// Long lists showing everything ("Show all N"), until Settings opens again.
    show_all: HashSet<&'static str>,
}

/// Settings › Invite › Reset main link, a popup over Settings (owner only;
/// PROTOCOL.md › HTTP › Resetting the main link).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResetMain {
    /// "Reset main link?" with the warning, [Reset link] [Cancel]. Screenshot
    /// aid: `press` presses Reset link once the server is there.
    Asking { press: bool },
    /// Sent; waiting for `main_key_seq` to move past this.
    Waiting(u64),
    /// The new link, with Copy.
    Done,
}

impl ResetMain {
    /// The server answered: the new key (Done), or its words (asking again, `main_reset_error` shows).
    fn poll(self, m: &Model) -> Self {
        match self {
            ResetMain::Waiting(sent) if m.main_key_seq != sent => {
                if m.main_reset_error.is_some() { ResetMain::Asking { press: false } } else { ResetMain::Done }
            }
            other => other,
        }
    }
}

impl SettingsState {
    pub fn new(c: &Config) -> Self {
        let devices = Arc::new(Mutex::new(None));
        let d2 = devices.clone();
        let _ = std::thread::Builder::new().name("yap-devices".into()).spawn(move || {
            *d2.lock() = Some(crate::audio::list_devices());
        });
        // Screenshot aid (debug builds): YAP_DEBUG_SETTINGS_ADVANCED=1 opens Voice › Advanced.
        let advanced = cfg!(debug_assertions) && std::env::var_os("YAP_DEBUG_SETTINGS_ADVANCED").is_some();
        Self {
            tab: 0,
            invite_label: String::new(),
            asked_invites: false,
            asked_modlog: false,
            name: c.name.clone(),
            password: PasswordForm::default(),
            asked_devices: false,
            devices,
            away_text: String::new(),
            away_until: 0,
            mic_test: false,
            confirm_signout: false,
            confirm_others: false,
            confirm_revoke: None,
            // Screenshot aid (debug builds): YAP_DEBUG_RESET_MAIN=ask opens the warning; `press` resets (a test server only!).
            reset_main: match std::env::var("YAP_DEBUG_RESET_MAIN").ok().filter(|_| cfg!(debug_assertions)).as_deref() {
                Some("ask") => Some(ResetMain::Asking { press: false }),
                Some("press") => Some(ResetMain::Asking { press: true }),
                _ => None,
            },
            asked_relay: false,
            hear_asked: false,
            advanced,
            show_all: HashSet::new(),
        }
    }

    pub fn with_tab(mut self, tab: usize) -> Self {
        self.tab = tab.min(TAB_ABOUT);
        self
    }

    /// Account, asking "Sign out everywhere else?" (the sign-in alert's button).
    pub fn asking_signout_others(mut self) -> Self {
        self.tab = TAB_ACCOUNT;
        self.confirm_others = true;
        self
    }
}

/// Settings lists show this many rows until "Show all N" (the web's SHORT_LIST).
const SHORT_LIST: usize = 5;

/// The first few of a long list (all of it when it's short or "Show all" was
/// clicked), and whether to offer "Show all N".
fn short_list<T>(st: &SettingsState, name: &str, rows: &[T]) -> (usize, bool) {
    let all = st.show_all.contains(name) || rows.len() <= SHORT_LIST + 1;
    if all { (rows.len(), false) } else { (SHORT_LIST, true) }
}

/// "Show all N" under a short list: a link.
fn show_all_link(ui: &mut egui::Ui, st: &mut SettingsState, name: &'static str, n: usize, p: &Pal) {
    let r = ui.add(egui::Label::new(egui::RichText::new(format!("Show all {n}")).font(theme::font(theme::ui())).color(p.link).underline()).sense(egui::Sense::click()));
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if r.clicked() {
        st.show_all.insert(name);
    }
}

/// Settings tabs, in the web's order (About, the app's own, last).
pub(super) const TABS: [&str; 8] = ["Look", "Profile", "Voice", "Notifications", "Account", "Owner & mods", "Invite", "About"];
pub const TAB_LOOK: usize = 0;
pub const TAB_PROFILE: usize = 1;
pub const TAB_VOICE: usize = 2;
pub const TAB_NOTIFICATIONS: usize = 3;
pub const TAB_ACCOUNT: usize = 4;
pub const TAB_MODS: usize = 5;
pub const TAB_INVITE: usize = 6;
/// The last tab.
pub const TAB_ABOUT: usize = TABS.len() - 1;
pub(super) fn settings(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, keep: &mut bool) {
    let Some(mut st) = app.settings.take() else {
        *keep = false;
        return;
    };
    let mut k = true;
    let width = 500.0;
    // The web's words: "Yap Properties" with OK.
    let (title, done) = ("Yap Properties", "OK");
    window(ui, p, title, Some("settings"), width, Some(&mut k), |ui| {
        let mut tab = st.tab;
        // Every tab is the same height, so the window doesn't jump or re-centre
        // when you switch; taller ones scroll inside. It always fits: title,
        // tabs and the button take ~200 points (any UI size).
        let page_h = (ui.ctx().content_rect().height() - 230.0).clamp(120.0, 460.0);
        theme::tabs(ui, &mut tab, &TABS, p, |ui, tab| {
            ui.set_min_height(page_h);
            let area = egui::ScrollArea::vertical().id_salt(("settings-tab", tab)).max_height(page_h).auto_shrink([false, true]);
            // Screenshot aid (debug builds): YAP_DEBUG_SETTINGS_SCROLL=<points> scrolls the tab down that far (or `end`, below).
            let area = match std::env::var("YAP_DEBUG_SETTINGS_SCROLL").ok().and_then(|v| v.parse().ok()).filter(|_| cfg!(debug_assertions)) {
                Some(y) => area.vertical_scroll_offset(y),
                None => area,
            };
            // Keep the scroll bar showing, so a tab with more below (Voice's
            // soundboard) says so.
            ui.spacing_mut().scroll.dormant_handle_opacity = ui.spacing().scroll.dormant_handle_opacity.max(0.5);
            ui.spacing_mut().scroll.dormant_background_opacity = ui.spacing().scroll.dormant_background_opacity.max(0.25);
            theme::scroll(ui, area, p, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                match tab {
                    TAB_LOOK => look_tab(ui, m, p, acts),
                    TAB_PROFILE => profile_tab(ui, m, p, acts, &mut st, app.h12),
                    TAB_VOICE => voice_tab(app, ui, m, p, acts, &mut st),
                    TAB_NOTIFICATIONS => notifications_tab(ui, m, p, acts, app.h12),
                    TAB_ACCOUNT => account_tab(app, ui, m, p, acts, &mut st, keep),
                    TAB_MODS => mods_tab(ui, m, p, acts, &mut st),
                    TAB_INVITE => invite_tab(ui, m, p, acts, &mut st),
                    _ => about_tab(app, ui, m, p, acts),
                }
                // Screenshot aid (debug builds): YAP_DEBUG_SETTINGS_SCROLL=end shows the tab's end.
                if cfg!(debug_assertions) && std::env::var("YAP_DEBUG_SETTINGS_SCROLL").as_deref() == Ok("end") {
                    ui.scroll_to_cursor(Some(egui::Align::Max));
                }
            });
        });
        st.tab = tab;
        // The mic test only runs while the Voice tab shows.
        if st.mic_test && tab != TAB_VOICE {
            st.mic_test = false;
            acts.push(Act::Cmd(Cmd::MicTest(false)));
        }
        actions(ui, |ui| {
            if primary_button(ui, done, p, true).clicked() {
                *keep = false;
            }
        });
    });
    *keep &= k;
    if app.settings.is_none() {
        app.settings = Some(st);
    }
}

pub(super) fn profile_tab(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, st: &mut SettingsState, h12: bool) {
    theme::group(ui, "You", p, |ui| {
        field_label(ui, "Display name:", p);
        let r = text_field(ui, &mut st.name, 32, "", p);
        if r.lost_focus() && !st.name.trim().is_empty() && st.name.trim() != m.config.name {
            let n = st.name.trim().to_string();
            acts.push(Act::Config(Box::new(move |c| c.name = n)));
            acts.push(Act::Cmd(Cmd::Profile { name: true, color: false }));
        }
        field_label(ui, "Color:", p);
        if let Some(c) = swatches(ui, &m.config.color, p) {
            acts.push(Act::Config(Box::new(move |cfg| cfg.color = c)));
            acts.push(Act::Cmd(Cmd::Profile { name: false, color: true }));
        }
    });
    super::super::extras::away_group(ui, m, p, acts, &mut st.away_text, &mut st.away_until, h12);
}

/// Settings › Voice in the web's groups: Mic & output, How you talk,
/// Sounds, then Advanced (folded each time Settings opens). Only what this
/// app has: no music mode or OBS here yet.
pub(super) fn voice_tab(app: &App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, st: &mut SettingsState) {
    let gate = app.shared.audio.gate.clone();
    let devices = st.devices.lock().clone();
    // The mic is only open in a call or while "Test mic" is on (like the web).
    let in_call = m.voice_wanted.is_some();
    let live = in_call || st.mic_test;
    theme::group(ui, "Mic & output", p, |ui| {
        field_label(ui, "Microphone:", p);
        device_combo(ui, "mic", &m.config.input_device, devices.as_ref().map(|d| &d.0), p, |name| {
            acts.push(Act::Config(Box::new(move |c| c.input_device = name)));
            acts.push(Act::Cmd(Cmd::AudioSettings { restart_input: true, restart_output: false, renegotiate: false }));
        });
        field_label(ui, "Output:", p);
        device_combo(ui, "out", &m.config.output_device, devices.as_ref().map(|d| &d.1), p, |name| {
            acts.push(Act::Config(Box::new(move |c| c.output_device = name)));
            acts.push(Act::Cmd(Cmd::AudioSettings { restart_input: false, restart_output: true, renegotiate: false }));
        });
        ui.add_space(4.0);
        let mut lower = m.config.lower_media;
        if theme::tip(theme::checkbox(ui, &mut lower, "Lower shared media while friends talk", p), "Never changes the volume you set for it").changed() {
            acts.push(Act::Config(Box::new(move |c| c.lower_media = lower)));
            acts.push(Act::ApplyGate);
        }
        note(ui, "A screen share's sound goes to half while someone in your call speaks, then back. (Watch together plays in your browser, which has the same switch.)", p);
        hear_yourself(ui, m, p, acts, st);
        super::super::call_report_field(ui, &app.shared, p, acts);
    });
    theme::group(ui, "How you talk", p, |ui| {
        field_label(ui, "Input mode:", p);
        let mut mode = m.config.mode;
        if theme::radio_row(ui, &mut mode, &[(InputMode::Vad, "Voice activity"), (InputMode::Ptt, "Push to talk"), (InputMode::Open, "Always on")], p) {
            acts.push(Act::Config(Box::new(move |c| c.mode = mode)));
            acts.push(Act::ApplyGate);
        }
        match mode {
            InputMode::Vad => {
                // Sensitivity: the level meter with the line, the slider, Test mic.
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(theme::text("Sensitivity:", p.text));
                    note(ui, "speak; the bar should cross the line", p);
                });
                let mic_ok = gate.mic_ok.load(std::sync::atomic::Ordering::Relaxed);
                let db = if live && mic_ok { gate.level_db.get() } else { -100.0 };
                let pct = |v: f32| ((v + 80.0) / 80.0).clamp(0.0, 1.0);
                let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), egui::Sense::hover());
                let hot = db > m.config.vad_threshold;
                // The mic's peaks keep hitting the top: red, and say so (the web's words).
                let clipping = live && mic_ok && gate.clipping.load(std::sync::atomic::Ordering::Relaxed);
                theme::meter(ui, r, pct(db), if clipping { p.danger } else if hot { p.ok } else { p.meter }, p);
                theme::meter_mark(ui, r, pct(m.config.vad_threshold), p);
                if clipping {
                    theme::tip(
                        ui.label(theme::text("Too loud (clipping)", p.danger)),
                        "The loudest moments hit the top and distort for everyone. Turn your mic's input level down (in your system's sound settings), or sit a little further from it.",
                    );
                }
                if live {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
                }
                let mut t = m.config.vad_threshold;
                ui.horizontal(|ui| {
                    if theme::trackbar(ui, &mut t, -80.0..=-10.0, 1.0, 220.0, p).changed() {
                        acts.push(Act::Config(Box::new(move |c| c.vad_threshold = t)));
                        acts.push(Act::ApplyGate);
                    }
                    ui.label(theme::text(format!("{t:.0} dB"), p.text));
                });
                // Screenshot aid (debug builds): YAP_DEBUG_MIC_TEST=1 starts Test mic once (with YAP_DEBUG_AUDIO, never a real mic).
                static DEBUG_TEST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if cfg!(debug_assertions)
                    && !in_call
                    && std::env::var_os("YAP_DEBUG_MIC_TEST").is_some()
                    && std::env::var_os("YAP_DEBUG_AUDIO").is_some()
                    && !DEBUG_TEST.swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    st.mic_test = true;
                    acts.push(Act::Cmd(Cmd::MicTest(true)));
                }
                ui.horizontal(|ui| {
                    if in_call {
                        note(ui, "Your mic is live: you're in a call.", p);
                    } else {
                        let label = if st.mic_test { "Stop test" } else { "Test mic" };
                        if theme::tip(theme::button(ui, label, Btn::Normal, true, p), "Open the mic here to see the level (it closes when you leave this tab)").clicked() {
                            st.mic_test = !st.mic_test;
                            acts.push(Act::Cmd(Cmd::MicTest(st.mic_test)));
                        }
                    }
                });
                if live && !mic_ok {
                    note(ui, m.mic_error.clone().unwrap_or_else(|| "Opening the microphone…".into()), p);
                }
            }
            InputMode::Ptt => {
                ui.add_space(4.0);
                key_row(app, ui, m, p, acts, Action::Talk);
                let mut click = m.config.ptt_click;
                if theme::tip(theme::checkbox(ui, &mut click, "Push-to-talk click", p), "A quiet click when it opens and when it closes; only you hear it").changed() {
                    acts.push(Act::Config(Box::new(move |c| c.ptt_click = click)));
                    acts.push(Act::ApplyGate);
                }
                note(ui, "Double-tap the key to stay live; tap it again to stop.", p);
            }
            InputMode::Open => {}
        }
        // The other global keys, next to push-to-talk's (all optional).
        ui.add_space(4.0);
        for a in [Action::Mute, Action::Deafen, Action::Cough] {
            key_row(app, ui, m, p, acts, a);
        }
        note(ui, KEYS_NOTE, p);
        let status = app.shared.ptt.describe();
        if !status.is_empty() {
            note(ui, status, p);
        }
        ui.add_space(4.0);
        let mut remind = m.config.remind_muted;
        if theme::tip(theme::checkbox(ui, &mut remind, "Remind me when I talk while muted", p), "A soft blip and \"You're muted\" by the Mute button, at most every 5 seconds").changed() {
            acts.push(Act::Config(Box::new(move |c| c.remind_muted = remind)));
        }
        ui.add_space(4.0);
        let mut ns = m.config.noise_suppression;
        if theme::tip(theme::checkbox(ui, &mut ns, "Noise suppression", p), "RNNoise: an AI filter for keyboards, fans and dogs").changed() {
            acts.push(Act::Config(Box::new(move |c| c.noise_suppression = ns)));
            acts.push(Act::ApplyGate);
        }
        // Applies at once, in a call too (the mic's thread reads it every 10 ms).
        let mut aec = m.config.echo_cancellation;
        if theme::tip(theme::checkbox(ui, &mut aec, "Echo cancellation", p), "Takes what your speakers play back out of your mic").changed() {
            acts.push(Act::Config(Box::new(move |c| c.echo_cancellation = aec)));
            acts.push(Act::ApplyGate);
        }
        let mut agc = m.config.auto_gain;
        if theme::checkbox(ui, &mut agc, "Automatic gain", p).changed() {
            acts.push(Act::Config(Box::new(move |c| c.auto_gain = agc)));
            acts.push(Act::ApplyGate);
        }
        note(ui, "Echo cancellation keeps your speakers out of your mic, so friends don't hear themselves. Automatic gain evens out how loud you come across while you talk; quiet stays quiet.", p);
    });
    super::super::extras::sound_group(ui, m, p, acts);
    let mut open = st.advanced;
    theme::advanced(ui, &mut open, p, |ui| {
        field_label(ui, "Quality:", p);
        let mut br = m.config.bitrate;
        // The web's words.
        let label = |b: u32| match b {
            64_000 => "64 kbps — Discord's default".to_string(),
            96_000 => "96 kbps — Discord's max (unboosted)".to_string(),
            128_000 => "128 kbps — crisp (default)".to_string(),
            256_000 => "256 kbps — overkill".to_string(),
            510_000 => "510 kbps — Opus maximum".to_string(),
            _ => format!("{} kbps", b / 1000),
        };
        theme::combo(ui, "bitrate", &label(br), ui.available_width().min(300.0), p, |ui| {
            for b in BITRATES {
                if theme::menu_item(ui, &label(b), b == br, p).clicked() {
                    br = b;
                }
            }
        });
        if br != m.config.bitrate {
            acts.push(Act::Config(Box::new(move |c| c.bitrate = br)));
            acts.push(Act::Cmd(Cmd::AudioSettings { restart_input: false, restart_output: false, renegotiate: true }));
        }
        // 256 and 510: what they cost (PROTOCOL.md › Voice, the spec's words).
        if br > 239_000 {
            ui.label(egui::RichText::new(HIGH_BITRATE_WARNING).font(theme::font(theme::small())).color(p.warn));
        }
        note(ui, QUALITY_NOTE, p);
        relay_toggle(ui, m, p, acts, st);
    });
    st.advanced = open;
}

/// Next to 256 and 510 kbps (PROTOCOL.md › Voice: RED's copy no longer fits a packet above ~239 kbps).
pub const HIGH_BITRATE_WARNING: &str = "No dropout protection at this quality (each bit of voice no longer fits twice in a packet), and several times the upload for each friend in the call.";
/// Under Quality. (This app sends a friend who picked less at most what they
/// asked for; browsers don't, so their voice comes at their own setting.)
const QUALITY_NOTE: &str = "Voice goes straight between you and your friends, so better quality costs the server nothing. Your voice goes out at your setting (or a friend's, if they picked less), and each friend's at theirs.";

/// Under the keys (the modifiers as this computer names them).
const KEYS_NOTE: &str = if cfg!(target_os = "macos") {
    "Global keys work while you're in a call, even when Yap isn't the front window. Hold Ctrl, Option, Shift or ⌘ while you press one to make a combination. Push-to-mute keeps you muted while it's held. In Yap's window, ⌘+Shift+M mutes and ⌘+Shift+D deafens."
} else {
    "Global keys work while you're in a call, even when Yap isn't the front window. Hold Ctrl, Alt or Shift while you press one to make a combination. Push-to-mute keeps you muted while it's held. In Yap's window, Ctrl+Shift+M mutes and Ctrl+Shift+D deafens."
};

/// One global key in Settings › Voice: its name, the key (click, then press
/// one), Clear (not for push-to-talk, which always has one), and what it's
/// doing now.
fn key_row(app: &App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, a: Action) {
    let ptt = app.shared.ptt.clone();
    let gate = &app.shared.audio.gate;
    let current = crate::ptt::config_key(&m.config, a).to_string();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(118.0, 23.0), egui::Sense::hover());
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, a.label(), theme::font(theme::ui()), p.text);
        let label = if ptt.binding() == Some(a) { "Press a key or mouse button…".to_string() } else { crate::ptt::key_label(&current) };
        if theme::tip(theme::button_sized(ui, &label, Btn::Normal, true, vec2(160.0, 23.0), p), "Click, then press the key (or a combination, or mouse button 3–5). Esc cancels.").clicked() {
            ptt.start_binding(a);
        }
        if a != Action::Talk && !current.is_empty() && theme::button_sized(ui, "Clear", Btn::Normal, true, vec2(60.0, 23.0), p).clicked() {
            acts.push(Act::Config(Box::new(move |c| {
                let _ = crate::ptt::bind(c, a, "");
            })));
            acts.push(Act::ApplyGate);
        }
        let on = |f: &std::sync::atomic::AtomicBool| f.load(std::sync::atomic::Ordering::Relaxed);
        match a {
            Action::Talk if on(&gate.ptt_latched) => {
                ui.label(theme::text("live", p.ok));
            }
            Action::Talk if on(&gate.ptt_down) => {
                ui.label(theme::text("talking", p.ok));
            }
            Action::Cough if on(&gate.cough) => {
                ui.label(theme::text("muted", p.danger));
            }
            _ => {}
        }
    });
}

/// Mic check: "Hear yourself" — 4 s of exactly what a call sends (noise
/// removal, gain, voice activity, Opus at the call's quality), played back.
/// The web's words.
fn hear_yourself(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, st: &mut SettingsState) {
    ui.add_space(6.0);
    field_label(ui, "Mic check:", p);
    note(ui, "Hear yourself the way others do: noise removal, voice activity and the call's quality included.", p);
    let busy = matches!(m.hear, HearState::Recording { .. } | HearState::Playing);
    // Screenshot aid (debug builds): YAP_DEBUG_HEAR=1 starts it once (only with YAP_DEBUG_AUDIO=silence: no real mic).
    let debug = cfg!(debug_assertions) && std::env::var("YAP_DEBUG_HEAR").is_ok() && std::env::var("YAP_DEBUG_AUDIO").as_deref() == Ok("silence");
    if debug && !st.hear_asked && m.status == Status::Online {
        st.hear_asked = true;
        acts.push(Act::Cmd(Cmd::HearYourself(None)));
    }
    ui.horizontal(|ui| {
        if theme::tip(theme::button(ui, "Hear yourself", Btn::Normal, !busy, p), "Record 4 seconds and hear exactly how you sound to others").clicked() && !busy {
            acts.push(Act::Cmd(Cmd::HearYourself(None)));
        }
        let status = match &m.hear {
            HearState::Idle => String::new(),
            HearState::Recording { hold_key: true } => format!("Recording 4 seconds — hold {} and say something…", crate::ptt::key_label(&m.config.ptt_key)),
            HearState::Recording { hold_key: false } => "Recording 4 seconds — say something…".to_string(),
            HearState::Playing => "Playing it back…".to_string(),
            HearState::Done => "That’s how you sound to others.".to_string(),
            HearState::Failed => "Couldn’t open the microphone.".to_string(),
        };
        if !status.is_empty() {
            note(ui, status, p);
        }
    });
    if busy {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
}

/// "Hide my IP address in calls" (the web's relayOnly): voice only through
/// the server's TURN relay. Off and greyed out until the server has one.
pub(super) fn relay_toggle(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, st: &mut SettingsState) {
    if !st.asked_relay && m.status == Status::Online {
        st.asked_relay = true;
        acts.push(Act::Cmd(Cmd::CheckRelay));
    }
    ui.add_space(4.0);
    let available = m.relay == Some(true);
    let mut on = m.config.relay_only && available;
    let r = ui.add_enabled_ui(available, |ui| theme::checkbox(ui, &mut on, "Hide my IP address in calls", p)).inner;
    if r.changed() {
        acts.push(Act::Config(Box::new(move |c| c.relay_only = on)));
    }
    note(
        ui,
        match m.relay {
            Some(true) if m.voice_wanted.is_some() => "Sends your voice through the server's voice relay instead of straight to friends (a little more delay). A change applies from the next call you join.",
            Some(true) => "Sends your voice through the server's voice relay instead of straight to friends (a little more delay).",
            Some(false) => "Needs the voice relay, which isn't switched on for this server yet.",
            None => "Checking whether this server has a voice relay…",
        },
        p,
    );
}

/// Settings › Notifications in the web's groups: On this device, then Quiet
/// hours. (Phone pushes through ntfy are set up on the web or the phone app.)
pub(super) fn notifications_tab(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, h12: bool) {
    theme::group(ui, "On this device", p, |ui| {
        field_label(ui, "Notifications:", p);
        #[derive(Clone, Copy, PartialEq)]
        enum Mode {
            All,
            Mentions,
            Off,
        }
        let mode = if !m.config.notifications { Mode::Off } else if m.config.notify_all { Mode::All } else { Mode::Mentions };
        let mut pick = mode;
        ui.vertical(|ui| {
            for (v, label) in [(Mode::All, "Every message"), (Mode::Mentions, "Only @mentions and pings"), (Mode::Off, "Off")] {
                if theme::radio(ui, pick == v, label, p).clicked() {
                    pick = v;
                }
            }
        });
        if pick != mode {
            acts.push(Act::Config(Box::new(move |c| {
                c.notifications = pick != Mode::Off;
                c.notify_all = pick == Mode::All;
            })));
        }
        note(ui, "While Yap is open in the background (not the front window).", p);
    });
    super::super::extras::quiet_group(ui, m, p, acts, h12);
}

/// Ago, in words ("5 min ago", "yesterday", "Oct 3").
pub(super) fn ago(ts: i64) -> String {
    let secs = (crate::core::now_ms() - ts).max(0) / 1000;
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        86_400..=172_799 => "yesterday".into(),
        _ => super::super::local_time(ts).format("%b %-d").to_string(),
    }
}

pub(super) fn invite_tab(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, st: &mut SettingsState) {
    if !st.asked_invites {
        st.asked_invites = true;
        acts.push(Act::Cmd(Cmd::ListInvites));
    }
    theme::group(ui, "Make a link", p, |ui| {
        note(ui, "A personal invite for one friend: when they knock you'll see who invited them, and you can turn the link off later. People already in keep working.", p);
        field_label(ui, "Who's it for? (optional)", p);
        ui.horizontal(|ui| {
            let w = ui.available_width() - 110.0;
            let r = theme::field(ui, egui::TextEdit::singleline(&mut st.invite_label).char_limit(40).hint_text("Ben").desired_width(w), p);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if theme::button_sized(ui, "Make a link", Btn::Default, m.status == Status::Online, vec2(96.0, 23.0), p).clicked() || enter {
                acts.push(Act::Cmd(Cmd::CreateInvite(std::mem::take(&mut st.invite_label))));
            }
        });
        if let Some((label, link)) = &m.new_invite {
            ui.add_space(4.0);
            ui.label(theme::text(if label.is_empty() { "Your new link:".to_string() } else { format!("Your new link for {label}:") }, p.text));
            ui.horizontal(|ui| {
                let mut shown = link.clone();
                let w = ui.available_width() - 82.0;
                theme::field(ui, egui::TextEdit::singleline(&mut shown).desired_width(w).font(theme::mono(12.0)), p);
                if theme::button(ui, "Copy", Btn::Normal, true, p).clicked() {
                    acts.push(Act::Copy(link.clone(), "Invite link copied."));
                }
            });
            note(ui, "It's shown only once — copy it now.", p);
        }
    });
    let (shown, more) = short_list(st, "invites", &m.invites);
    theme::group(ui, "Invite links", p, |ui| {
        if m.invites.is_empty() {
            note(ui, "No invite links yet.", p);
        }
        for inv in m.invites.iter().take(shown) {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let label = if inv.label.is_empty() { "(no label)".to_string() } else { inv.label.clone() };
                    let by = if inv.by == m.config.uid { String::new() } else { format!(" · by {}", name_in(m, &inv.by)) };
                    ui.label(egui::RichText::new(format!("{label}{by}")).font(theme::bold_font(theme::ui())).color(if inv.revoked { p.gray } else { p.text }));
                    let uses = if inv.uses == 1 { "1 person joined".to_string() } else { format!("{} people joined", inv.uses) };
                    note(ui, format!("{uses} · made {}{}", ago(inv.created_at), if inv.revoked { " · turned off" } else { "" }), p);
                });
                if !inv.revoked && st.confirm_revoke.as_deref() != Some(inv.id.as_str()) {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::button(ui, "Turn off…", Btn::Normal, true, p).clicked() {
                            st.confirm_revoke = Some(inv.id.clone());
                        }
                    });
                }
            });
            // Asked first, like the web.
            if !inv.revoked && st.confirm_revoke.as_deref() == Some(inv.id.as_str()) {
                let what = if inv.label.is_empty() { "this link".to_string() } else { format!("“{}”", inv.label) };
                ui.label(theme::text(format!("Turn off {what}? Nobody new can join with this link. People already in stay in."), p.text));
                ui.horizontal_wrapped(|ui| {
                    if theme::button(ui, "Turn off", Btn::Danger, true, p).clicked() {
                        acts.push(Act::Cmd(Cmd::RevokeInvite(inv.id.clone())));
                        st.confirm_revoke = None;
                    }
                    if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                        st.confirm_revoke = None;
                    }
                });
            }
            ui.add_space(2.0);
        }
        if more {
            show_all_link(ui, st, "invites", m.invites.len(), p);
        }
    });
    theme::group(ui, "Main link", p, |ui| {
        // The web's caution, before the button.
        note(ui, "The main link never expires and doesn't say who invited whom. Only share it if you mean to.", p);
        ui.horizontal_wrapped(|ui| {
            if theme::button(ui, "Copy main invite link", Btn::Normal, true, p).clicked() {
                acts.push(Act::Copy(crate::config::invite_link(&m.config), "Invite link copied."));
            }
            // The owner can make a new one (the old one stops letting new people in).
            if m.am_owner() && theme::button_sized(ui, "Reset main link…", Btn::Normal, m.status == Status::Online, vec2(120.0, 23.0), p).clicked() {
                st.reset_main = Some(ResetMain::Asking { press: false });
            }
        });
    });
    reset_main_popup(ui, m, p, acts, st);
}

/// Reset main link? — a popup over Settings, in the spec's words: the
/// warning with [Reset link] [Cancel] (Windows' order, like every dialog
/// here); then the new link with Copy.
fn reset_main_popup(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, st: &mut SettingsState) {
    let Some(state) = st.reset_main.map(|r| r.poll(m)) else { return };
    // Only the owner, while connected (a screenshot aid could open it for anyone).
    let state = match state {
        ResetMain::Asking { press: true } if m.status == Status::Online && m.am_owner() => {
            acts.push(Act::Cmd(Cmd::ResetMainLink));
            ResetMain::Waiting(m.main_key_seq)
        }
        other => other,
    };
    let (mut keep, mut close_box) = (true, true);
    let mut next = state;
    let resp = egui::Modal::new(egui::Id::new("reset-main")).frame(dialog_frame(p)).backdrop_color(Color32::TRANSPARENT).show(ui.ctx(), |ui| {
        let title = if state == ResetMain::Done { "New main link" } else { "Reset main link" };
        window(ui, p, title, Some("link"), 360.0, Some(&mut close_box), |ui| match state {
            ResetMain::Asking { .. } | ResetMain::Waiting(_) => {
                let waiting = matches!(state, ResetMain::Waiting(_));
                ui.label(egui::RichText::new("Reset main link?").font(theme::bold_font(theme::ui())).color(p.text));
                ui.label(theme::text(
                    "The old link stops working for anyone who hasn't joined yet. Everyone already signed in keeps working, and their apps get the new link by themselves. Personal invite links keep working.",
                    p.text,
                ));
                if let Some(e) = m.main_reset_error.as_ref().filter(|_| !waiting) {
                    ui.label(theme::text(e, p.danger));
                }
                actions(ui, |ui| {
                    if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                        keep = false;
                    }
                    let ok = !waiting && m.status == Status::Online;
                    if theme::button_sized(ui, if waiting { "Resetting…" } else { "Reset link" }, Btn::Danger, ok, vec2(90.0, 23.0), p).clicked() && ok {
                        acts.push(Act::Cmd(Cmd::ResetMainLink));
                        next = ResetMain::Waiting(m.main_key_seq);
                    }
                });
            }
            ResetMain::Done => {
                // The web's words.
                let link = crate::config::invite_link(&m.config);
                ui.label(theme::text("The new main link:", p.text));
                ui.horizontal(|ui| {
                    let mut shown = link.clone();
                    let w = ui.available_width() - 82.0;
                    theme::field(ui, egui::TextEdit::singleline(&mut shown).desired_width(w).font(theme::mono(12.0)), p);
                    if theme::button(ui, "Copy", Btn::Normal, true, p).clicked() {
                        acts.push(Act::Copy(link.clone(), "New main link copied."));
                    }
                });
                note(ui, "The old one stopped working for anyone who hasn't joined yet.", p);
                actions(ui, |ui| {
                    if primary_button(ui, "OK", p, true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        keep = false;
                    }
                });
            }
        });
    });
    if resp.should_close() || !close_box {
        keep = false;
    }
    st.reset_main = keep.then_some(next);
}

/// Who did something, at the start of an Activity line: "The owner (admin
/// key)" for the admin key, else their name.
pub(super) fn actor_name(m: &Model, uid: Option<&str>) -> String {
    match uid {
        Some("admin") => "The owner (admin key)".into(),
        Some(uid) => m.users.get(uid).map(|u| u.name.clone()).unwrap_or_else(|| "Someone".into()),
        None => "Someone".into(),
    }
}

/// The same, in the middle of a sentence.
fn name_in(m: &Model, uid: &str) -> String {
    if uid == "admin" { "the owner (admin key)".into() } else { actor_name(m, Some(uid)) }
}

/// Settings › Owner & mods › Activity, in plain words: "<actor> let Ada in".
/// Every action the server logs (hub.ts › log), the web's LOG_WORDS.
pub(super) fn log_words(m: &Model, e: &crate::protocol::ModLogEntry) -> String {
    let who = e.target.as_deref().map(|t| name_in(m, t)).unwrap_or_else(|| "someone".into());
    let detail = e.detail.clone();
    let d = detail.clone().unwrap_or_default();
    let with_detail = |words: &str| if d.is_empty() { words.to_string() } else { format!("{words} ({d})") };
    match e.action.as_str() {
        "member.approve" => format!("let {who} in"),
        "member.deny" => format!("turned {who} away"),
        "member.ban" => format!("banned {who}"),
        "member.role" => match d.as_str() {
            "banned" => format!("banned {who}"),
            "pending" => format!("sent {who} back to the waiting room"),
            // From the app only the owner sets roles, and "member" means taking a mod's role away.
            "member" if e.actor.as_deref() != Some("admin") => format!("made {who} a member again (no longer a mod)"),
            "mod" => format!("made {who} a mod"),
            "member" | "" => format!("made {who} a member"),
            "owner" => format!("made {who} the owner"),
            other => format!("made {who} {other}"),
        },
        "member.resetPin" => format!("reset {who}'s password"),
        "member.merge" => format!("merged a duplicate account into {who}'s"),
        "member.unmerge" => "split a merged account back out".into(),
        "owner.claim" => "became the owner".into(),
        "message.delete" => format!("deleted a message by {who}"),
        "channel.create" => format!("made a {}", detail.as_deref().unwrap_or("channel")),
        "channel.archive" => format!("archived the channel {d}").trim().to_string(),
        "channel.unarchive" => format!("brought back the {}", detail.as_deref().unwrap_or("channel")),
        "channel.bitrate" => match d.split_once(" → ").filter(|(c, _)| c.starts_with('"') && c.ends_with('"')) {
            Some((channel, "personal settings")) => format!("let everyone in {channel} pick their own voice quality again"),
            Some((channel, to)) => format!("set the voice quality in {channel} to {to} for everyone"),
            None => "changed a voice channel's quality".into(),
        },
        "emoji.delete" => format!("deleted the emoji {d}").trim().to_string(),
        "invite.create" => with_detail("made an invite link"),
        "invite.revoke" => with_detail("turned off an invite link"),
        // The owner made a new main link (PROTOCOL.md › HTTP › Resetting the main
        // link); the admin API can also put the server's own YAP_KEY back.
        "invite.resetMain" if d == "YAP_KEY" => "set the main link back to the server’s first one".into(),
        "invite.resetMain" => "reset the main link".into(),
        "link.add" => match d.split_once(" → ") {
            Some((name, host)) => format!("added {name} ({host}) to Places"),
            None => format!("added {} to Places", detail.as_deref().unwrap_or("a link")),
        },
        "link.remove" => format!("removed {} from Places", detail.as_deref().unwrap_or("a link")),
        "obs.create" => "made an OBS link".into(),
        "obs.revoke" => match d.parse::<u32>() {
            Ok(n) if n > 1 => format!("turned off their {n} OBS links"),
            _ => "turned off their OBS link".into(),
        },
        "sound.delete" => format!("deleted a soundboard clip by {who}"),
        // An action a newer server logs that this app doesn't know yet.
        other => with_detail(&format!("did “{}”", other.replace(['.', '_'], " "))),
    }
}

/// Settings › Owner & mods: what they can do, and the moderation log.
pub(super) fn mods_tab(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, st: &mut SettingsState) {
    if !st.asked_modlog {
        st.asked_modlog = true;
        acts.push(Act::Cmd(Cmd::ModLog));
    }
    ui.label(theme::text("New people wait in a waiting room until the owner or a mod lets them in. The owner and mods can also ban people, delete any message and reset passwords.", p.gray));
    ui.add_space(4.0);
    // The five newest, then "Show all N" (no box scrolling inside the scrolling tab).
    let (shown, more) = short_list(st, "modlog", &m.modlog);
    theme::group(ui, "Activity", p, |ui| {
        note(ui, "Who let people in, banned, made invites… (never message text)", p);
        ui.add_space(2.0);
        if m.modlog.is_empty() {
            note(ui, "Nothing yet.", p);
        }
        for e in m.modlog.iter().take(shown) {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(egui::RichText::new(actor_name(m, e.actor.as_deref())).font(theme::bold_font(theme::ui())).color(p.text));
                ui.label(theme::text(log_words(m, e), p.text));
                ui.label(egui::RichText::new(ago(e.ts)).font(theme::font(theme::small())).color(p.gray));
            });
        }
        ui.horizontal(|ui| {
            if more {
                show_all_link(ui, st, "modlog", m.modlog.len(), p);
                ui.add_space(8.0);
            }
            if theme::button(ui, "Refresh", Btn::Normal, true, p).clicked() {
                acts.push(Act::Cmd(Cmd::ModLog));
            }
        });
    });
}

pub(super) fn look_tab(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    theme::group(ui, "Theme", p, |ui| {
        let mut theme_pref = m.config.theme;
        ui.vertical(|ui| {
            for (v, label) in [(ThemePref::Dark, "Charcoal (dark)"), (ThemePref::Light, "Windows Standard (light)"), (ThemePref::System, "Match system")] {
                if theme::radio(ui, theme_pref == v, label, p).clicked() {
                    theme_pref = v;
                }
            }
        });
        if theme_pref != m.config.theme {
            super::super::apply_theme(ui.ctx(), theme_pref);
            acts.push(Act::Config(Box::new(move |c| c.theme = theme_pref)));
        }
    });
    theme::group(ui, "Size", p, |ui| {
        let auto = super::super::auto_size(ui.ctx());
        let mut is_auto = m.config.size.is_none();
        if theme::checkbox(ui, &mut is_auto, &format!("Auto ({}% on this screen)", (auto * 100.0).round()), p).changed() {
            let size = if is_auto { None } else { Some(ui.ctx().zoom_factor()) };
            acts.push(Act::Config(Box::new(move |c| c.size = size)));
        }
        // The new size applies when you let go (the slider would move under the pointer).
        let id = egui::Id::new("size-drag");
        let mut pct = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or((m.config.size.unwrap_or(auto) * 100.0).round());
        ui.horizontal(|ui| {
            let r = theme::trackbar(ui, &mut pct, crate::config::SIZE_MIN * 100.0..=crate::config::SIZE_MAX * 100.0, 5.0, 220.0, p);
            ui.label(theme::text(format!("{}%", pct as i32), p.text));
            if r.dragged() {
                ui.ctx().data_mut(|d| d.insert_temp(id, pct));
            }
            if r.drag_stopped() || r.clicked() {
                ui.ctx().data_mut(|d| d.remove::<f32>(id));
                let size = Some(pct / 100.0);
                acts.push(Act::Config(Box::new(move |c| c.size = size)));
            }
        });
        let k = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };
        ui.label(egui::RichText::new(format!("{k} + and {k} − change it anywhere; {k} 0 goes back to Auto.")).font(theme::font(theme::small())).color(p.gray));
    });
}

pub(super) fn account_tab(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, st: &mut SettingsState, keep: &mut bool) {
    theme::group(ui, "Account", p, |ui| {
        ui.label(theme::text(format!("Signed in as {} on {}", m.config.name, m.config.server), p.text));
        note(ui, "On another phone or computer, choose “I have an account” and sign in with your name and password.", p);
        ui.add_space(4.0);
        if st.confirm_signout {
            // Asked once, like the web (it's easy to undo, but not by accident).
            ui.label(theme::text("Sign out of Yap on this computer? You can sign back in with your name and password.", p.text));
            ui.horizontal_wrapped(|ui| {
                if theme::button(ui, "Sign out", Btn::Danger, true, p).clicked() {
                    acts.push(Act::Cmd(Cmd::SignOut));
                    app.welcome = Welcome::from_config(&m.config);
                    app.welcome.new_account = false;
                    st.confirm_signout = false;
                    *keep = false;
                }
                if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                    st.confirm_signout = false;
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                if theme::button(ui, "Sign out of this computer", Btn::Danger, true, p).clicked() {
                    st.confirm_signout = true;
                }
                if theme::button(ui, "Use a different server…", Btn::Normal, true, p).clicked() {
                    app.welcome = Welcome::other_server(&m.config);
                    *keep = false;
                }
            });
        }
    });
    theme::group(ui, "Personal space", p, |ui| {
        let text = match (&m.vault, &m.vault_key) {
            (Some(_), Some(_)) => "End-to-end encrypted with a passcode only you know. This computer remembers the key until you lock it.",
            (Some(_), None) => "Encrypted, and locked on this computer: your passcode opens it.",
            _ => "Not encrypted yet: Yap's server (and its backups) can read it. Encrypt it with a passcode only you know.",
        };
        ui.label(theme::text(text, p.text));
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            let mut open = |mode: VaultMode| {
                acts.push(Act::Vault(mode));
                *keep = false;
            };
            if m.vault.is_none() {
                if theme::button_sized(ui, "Encrypt it…", Btn::Normal, true, vec2(96.0, 23.0), p).clicked() {
                    open(VaultMode::Setup);
                }
            } else {
                if m.vault_key.is_none() && theme::button(ui, "Unlock…", Btn::Normal, true, p).clicked() {
                    open(VaultMode::Unlock);
                }
                if theme::button_sized(ui, "Change passcode…", Btn::Normal, true, vec2(120.0, 23.0), p).clicked() {
                    open(VaultMode::Change);
                }
            }
            if m.vault_key.is_some() && theme::button_sized(ui, "Lock on this computer", Btn::Normal, true, vec2(140.0, 23.0), p).clicked() {
                acts.push(Act::Cmd(Cmd::Vault(crate::core::VaultOp::Lock)));
            }
        });
    });
    // "Set a password" while there's none or it's still an old PIN.
    let title = if m.needs_pin || m.secret_kind == "pin" { "Set a password" } else { "Change password" };
    st.password.poll(m);
    theme::group(ui, title, p, |ui| {
        let enter = st.password.fields(ui, m, p, false);
        ui.add_space(2.0);
        let ok = st.password.ready(m);
        let label = if st.password.waiting.is_some() { "Saving…" } else { "Save password" };
        if (theme::button_sized(ui, label, Btn::Normal, ok, vec2(100.0, 23.0), p).clicked() || enter) && ok {
            st.password.submit(m, acts);
        }
    });
    devices_group(ui, m, p, acts, st);
}

/// Settings › Account: where you're signed in; sign one out, or all the others.
pub(super) fn devices_group(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, st: &mut SettingsState) {
    if !st.asked_devices && m.status == Status::Online {
        st.asked_devices = true;
        acts.push(Act::Cmd(Cmd::Devices));
    }
    theme::group(ui, "Signed in on", p, |ui| {
        let Some(devices) = &m.devices else {
            ui.label(theme::text("Loading…", p.gray));
            return;
        };
        for d in devices {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
                let icon = if d.scope.as_deref() == Some("listen") { "tv" } else { "chat" };
                super::super::icons::paint(ui.painter(), r, icon, p.gray);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let mut name = egui::text::LayoutJob::default();
                    name.append(&d.label, 0.0, egui::TextFormat { font_id: theme::bold_font(theme::ui()), color: p.text, ..Default::default() });
                    if d.current {
                        name.append("this one", 6.0, egui::TextFormat { font_id: theme::font(theme::small()), color: p.ok, ..Default::default() });
                    }
                    ui.label(name);
                    ui.label(egui::RichText::new(format!("signed in {} · last used {}", ago(d.created_at), ago(d.last_seen))).font(theme::font(theme::small())).color(p.gray));
                });
                if !d.current {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::button(ui, "Sign out", Btn::Normal, true, p).clicked() {
                            acts.push(Act::Cmd(Cmd::RevokeDevice(d.id.clone())));
                        }
                    });
                }
            });
        }
        let others = devices.iter().filter(|d| !d.current).count();
        ui.add_space(2.0);
        if st.confirm_others && others > 0 {
            // Asked first, in the web's words.
            ui.label(theme::text("Sign out everywhere else? Every other device (and your OBS links) is signed out. You stay signed in here.", p.text));
            ui.horizontal_wrapped(|ui| {
                if theme::button(ui, "Sign out", Btn::Danger, true, p).clicked() {
                    acts.push(Act::Cmd(Cmd::SignOutOthers));
                    acts.push(Act::DismissSigninAlert);
                    st.confirm_others = false;
                }
                if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                    st.confirm_others = false;
                }
            });
        } else if theme::button_sized(ui, "Sign out everywhere else…", Btn::Normal, others > 0, vec2(160.0, 23.0), p).clicked() {
            st.confirm_others = true;
        }
        note(ui, "Don't recognize one? Sign it out, then change your password.", p);
    });
}

pub(super) fn about_tab(app: &App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    theme::group(ui, "Yap desktop", p, |ui| {
        ui.label(egui::RichText::new(format!("Version {} ({})", crate::update::current_version(), crate::update::platform())).font(theme::bold_font(theme::ui())).color(p.text));
        note(ui, format!("Settings live in {}", app.shared.config_path.display()), p);
        ui.horizontal(|ui| {
            let v = ui.add(egui::Label::new(egui::RichText::new("What's new (changelog)").font(theme::font(theme::ui())).color(p.link).underline()).sense(egui::Sense::click()));
            if v.clicked() {
                acts.push(Act::OpenUrl(format!("{}/changelog", m.config.server)));
            }
            ui.label(theme::text("·", p.gray));
            let v = ui.add(egui::Label::new(egui::RichText::new("Privacy").font(theme::font(theme::ui())).color(p.link).underline()).sense(egui::Sense::click()));
            if theme::tip(v, "What Yap keeps, and what it doesn't").clicked() {
                acts.push(Act::OpenUrl(format!("{}/privacy", m.config.server)));
            }
        });
    });
    theme::group(ui, "Updates", p, |ui| {
        if !crate::update::enabled() {
            ui.label(theme::text(crate::update::OFF_TEXT, p.text));
            ui.add_space(2.0);
            if !m.config.server.is_empty() && theme::button_sized(ui, "Open the download page", Btn::Normal, true, vec2(160.0, 23.0), p).clicked() {
                acts.push(Act::OpenUrl(format!("{}/download", m.config.server)));
            }
            return;
        }
        let (text, color) = match &m.update {
            UpdateState::Idle => ("Yap checks for updates when it starts and every 6 hours.".to_string(), p.text),
            UpdateState::Checking => ("Checking…".to_string(), p.text),
            UpdateState::UpToDate => ("You're up to date.".to_string(), p.text),
            UpdateState::Available(a) => (format!("Yap {} is available.", a.version), p.text),
            UpdateState::Downloading { version, done, total } => (format!("Downloading {version}… {}%", if *total > 0 { done * 100 / total } else { 0 }), p.text),
            UpdateState::Installed(v) => (format!("{v} installed; restarting…"), p.text),
            UpdateState::Failed(e) => (e.clone(), p.danger),
        };
        ui.label(theme::text(text, color));
        note(ui, "Updates are signed; Yap only installs one when you click, and only after checking the signature.", p);
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let busy = matches!(m.update, UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Installed(_));
            if theme::button_sized(ui, "Check for updates", Btn::Normal, !busy, vec2(120.0, 23.0), p).clicked() {
                acts.push(Act::Cmd(Cmd::CheckUpdate { manual: true }));
            }
            if let UpdateState::Available(a) = &m.update
                && primary_button(ui, &format!("Install {}…", a.version), p, true).clicked()
            {
                acts.push(Act::Cmd(Cmd::InstallUpdate));
            }
        });
    });
}

pub(super) fn device_combo(ui: &mut egui::Ui, id: &str, current: &str, list: Option<&Vec<String>>, p: &Pal, mut on_pick: impl FnMut(String)) {
    let shown = if current.is_empty() { "System default".to_string() } else { current.to_string() };
    theme::combo(ui, id, &shown, ui.available_width(), p, |ui| {
        if theme::menu_item(ui, "System default", current.is_empty(), p).clicked() && !current.is_empty() {
            on_pick(String::new());
        }
        match list {
            None => {
                ui.label(theme::text("Looking for devices…", p.gray));
            }
            Some(list) => {
                for d in list {
                    if theme::menu_item(ui, d, d == current, p).clicked() && d != current {
                        on_pick(d.clone());
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_log_words_match_the_web() {
        let mut m = Model::new(Config::default());
        m.users.insert(
            "ada".into(),
            serde_json::from_value(serde_json::json!({ "id": "ada", "name": "Ada", "color": "#000000", "role": "member" })).unwrap(),
        );
        let e = |action: &str, target: Option<&str>, detail: Option<&str>| crate::protocol::ModLogEntry {
            ts: 0,
            actor: Some("ada".into()),
            action: action.into(),
            target: target.map(Into::into),
            detail: detail.map(Into::into),
        };
        // The web's words (LOG_WORDS in web/js/app.js).
        assert_eq!(log_words(&m, &e("channel.unarchive", None, Some("text channel games"))), "brought back the text channel games");
        assert_eq!(log_words(&m, &e("sound.delete", Some("ada"), None)), "deleted a soundboard clip by Ada");
        assert_eq!(log_words(&m, &e("channel.archive", None, Some("games"))), "archived the channel games");
        assert_eq!(log_words(&m, &e("member.role", Some("ada"), Some("member"))), "made Ada a member again (no longer a mod)");
        assert_eq!(log_words(&m, &e("member.role", Some("ada"), Some("mod"))), "made Ada a mod");
        assert_eq!(log_words(&m, &e("member.approve", Some("admin"), None)), "let the owner (admin key) in");
        assert_eq!(log_words(&m, &e("channel.bitrate", None, Some("\"Lounge\" → 256 kbps"))), "set the voice quality in \"Lounge\" to 256 kbps for everyone");
        assert_eq!(log_words(&m, &e("channel.bitrate", None, Some("\"Lounge\" → personal settings"))), "let everyone in \"Lounge\" pick their own voice quality again");
        assert_eq!(log_words(&m, &e("link.add", None, Some("Wiki → example.com"))), "added Wiki (example.com) to Places");
        assert_eq!(log_words(&m, &e("obs.revoke", None, Some("3"))), "turned off their 3 OBS links");
        assert_eq!(log_words(&m, &e("invite.resetMain", None, None)), "reset the main link");
        assert_eq!(log_words(&m, &e("invite.resetMain", None, Some("YAP_KEY"))), "set the main link back to the server’s first one");
        assert_eq!(log_words(&m, &e("member.kick", Some("ada"), Some("x"))), "did “member kick” (x)");
        assert_eq!(actor_name(&m, Some("admin")), "The owner (admin key)");
        let mut admin = e("member.role", Some("ada"), Some("member"));
        admin.actor = Some("admin".into());
        assert_eq!(log_words(&m, &admin), "made Ada a member");
    }
}

#[cfg(test)]
mod reset_main_tests {
    use super::*;

    /// The popup waits for the server: the new key (Done, then the link with
    /// Copy) or its refusal (back to the warning, with the server's words).
    #[test]
    fn the_popup_follows_the_server() {
        let mut m = Model::new(Config::default());
        let sent = ResetMain::Waiting(m.main_key_seq);
        assert_eq!(sent.poll(&m), sent, "nothing yet");
        m.main_key_seq += 1;
        assert_eq!(sent.poll(&m), ResetMain::Done, "the new main key came");
        m.main_reset_error = Some("Only the owner can do that.".into());
        m.main_key_seq += 1;
        let again = ResetMain::Waiting(m.main_key_seq - 1);
        assert_eq!(again.poll(&m), ResetMain::Asking { press: false }, "turned away: the warning again, with the words");
        assert_eq!(ResetMain::Done.poll(&m), ResetMain::Done);
    }
}
