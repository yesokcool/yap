//! Pieces the windows share, written once: the sidebar's list rows and its
//! Upcoming, voice-channel and Places sections, the chat's information
//! bars, avatars, toasts, the typing line's words, day dividers and the
//! start of a channel.

use super::{Act, App, Face, dialogs::Modal, face, icons, theme, theme::Pal};
use crate::core::{Cmd, Hint, Model, Status};
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, pos2, vec2};

// ── List rows ──────────────────────────────────────────────────────────

/// How a list row looks this time.
#[derive(Default, Clone, Copy)]
pub struct Look {
    pub active: bool,
    pub bold: bool,
    pub tint: Option<Color32>,
}

/// A row's height (rows under a row too).
const ROW_H: f32 = 20.0;

/// A group heading with an optional + button; true when it's clicked.
pub fn list_heading(ui: &mut egui::Ui, p: &Pal, title: &str, add: Option<&str>) -> bool {
    let mut clicked = false;
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    ui.painter().text(pos2(rect.left() + 3.0, rect.center().y), Align2::LEFT_CENTER, title, theme::bold_font(theme::ui()), p.text);
    if let Some(tip) = add {
        let b = Rect::from_min_size(pos2(rect.right() - 22.0, rect.top()), vec2(20.0, 18.0));
        clicked = ui.place(b, |ui: &mut egui::Ui| icons::tool(ui, "plus", vec2(20.0, 18.0), p.gray, tip, false)).clicked();
    }
    clicked
}

/// A row: icon + name, navy when active, underlined under the pointer.
pub fn list_row(ui: &mut egui::Ui, p: &Pal, kind: &str, key: &str, icon: &str, name: &str, look: Look) -> (Rect, egui::Response) {
    let (rect, resp) = super::row(ui, ROW_H, (kind, key));
    let painter = ui.painter();
    if look.active {
        painter.rect_filled(rect, 0.0, p.select);
    }
    let fg = if look.active { p.on_select } else { look.tint.unwrap_or(p.text) };
    let icon_color = if look.active { p.on_select } else { look.tint.unwrap_or(p.gray) };
    let x = rect.left() + 14.0;
    icons::paint(painter, Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(14.0, 14.0)), icon, icon_color);
    let font = if look.bold { theme::bold_font(theme::ui()) } else { theme::font(theme::ui()) };
    // A long name ends in "…" short of a badge or the archive button.
    let g = fit(painter, name, font, rect.right() - 24.0 - (x + 19.0), fg);
    let at = pos2(x + 19.0, rect.center().y - g.size().y / 2.0);
    let w = g.size().x;
    painter.with_clip_rect(rect.intersect(painter.clip_rect())).galley(at, g, fg);
    if resp.hovered() {
        painter.hline(at.x..=at.x + w, (rect.center().y + 7.0).round(), egui::Stroke::new(1.0, fg));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    (rect, resp)
}

/// "Nothing planned" and the like: a quiet line where a list would be.
fn list_empty(ui: &mut egui::Ui, p: &Pal, kind: &str, text: &str) -> egui::Response {
    let (rect, resp) = super::row(ui, ROW_H, (kind, ""));
    ui.painter().text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, text, theme::font(theme::small()), p.gray);
    resp
}

/// A count badge: a red box with a black ring.
pub fn badge(painter: &egui::Painter, right_center: Pos2, n: usize) {
    let text = n.to_string();
    let w = (8.0 + 6.0 * text.len() as f32).max(16.0);
    let r = theme::RoundRect::round(Rect::from_min_size(pos2(right_center.x - w, right_center.y - 7.0), vec2(w, 14.0)));
    painter.rect_filled(r, 0.0, theme::BADGE);
    theme::outline(painter, r.expand(1.0), Color32::BLACK);
    painter.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(10.0), Color32::WHITE);
}

// ── Sidebar sections ───────────────────────────────────────────────────

/// Upcoming hangs (and one going on now), with + to plan one; right-click
/// cancels yours, a click joins its voice channel.
pub fn hangs(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    if m.status != Status::Online {
        return;
    }
    if list_heading(ui, p, "Upcoming", Some("Plan a hang")) {
        app.modal = Some(Modal::PlanHang(super::extras::PlanHang::new(m)));
    }
    let list = super::extras::upcoming_hangs(m);
    if list.is_empty() {
        let resp = list_empty(ui, p, "no-hangs", "Nothing planned");
        if theme::tip(resp, "Plan a hang: the people who are free then get one reminder").clicked() {
            app.modal = Some(Modal::PlanHang(super::extras::PlanHang::new(m)));
        }
        return;
    }
    for h in list {
        let when = super::extras::hang_when(h.at, app.h12);
        let now = when == "now";
        let (r, resp) = list_row(ui, p, "hang", &h.id, "calendar", &h.title, Look { bold: now, tint: now.then_some(p.ok), ..Default::default() });
        // When, on the right (over the end of a long title).
        let color = if now { p.ok } else { p.gray };
        let g = ui.painter().layout_no_wrap(when, theme::font(theme::small()), color);
        let gw = g.size().x;
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.right() - gw - 12.0, r.top()), r.max), 0.0, p.window);
        ui.painter().galley(pos2(r.right() - 6.0 - gw, r.center().y - g.size().y / 2.0), g, color);
        if super::extras::can_cancel_hang(m, h) {
            let id = h.id.clone();
            theme::context_menu(&resp, p, |ui| {
                if theme::menu_item(ui, "Cancel this hang", false, p).clicked() {
                    acts.push(Act::Cmd(Cmd::CancelHang(id.clone())));
                    ui.close();
                }
            });
        }
        if theme::tip(resp, super::extras::hang_tip(m, h, app.h12)).clicked()
            && let Some(c) = h.channel_id.clone()
            && m.voice_channel.as_deref() != Some(c.as_str())
        {
            acts.push(Act::Cmd(Cmd::JoinVoice(c)));
        }
    }
}

/// A row under a row (someone in voice, someone on a Place): its avatar's
/// centre, from the row's left…
const SUB_X: f32 = 38.0;
/// …the avatar's size…
const SUB_AVATAR: f32 = 14.0;
/// …and from the avatar's centre to the name.
const SUB_GAP: f32 = 12.0;

/// A connection hint after someone's name ("weak connection", "reconnecting…"),
/// cut short with an ellipsis if the row is narrow. The web's colours: a
/// warning, grey italics while they're reconnecting, red when they can't connect.
fn hint_label(ui: &egui::Ui, p: &Pal, at: Pos2, max_w: f32, hint: &Hint) {
    if max_w < 24.0 {
        return;
    }
    let (color, italics) = match hint {
        Hint::Reconnecting => (p.gray, true),
        Hint::NeedsRelay | Hint::CantConnect => (p.danger, false),
        Hint::Weak { .. } => (p.warn, false),
    };
    let font_id = theme::font(theme::small() - 1.0);
    let mut job = egui::text::LayoutJob::single_section(hint.text().to_string(), egui::TextFormat { font_id, color, italics, ..Default::default() });
    job.wrap = egui::text::TextWrapping { max_width: max_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(at.x, at.y - g.size().y / 2.0), g, color);
}

/// Your volume for someone, on their row when you've changed it (PROTOCOL.md ›
/// Your volume, on the row): "60 %" / "150 %", or a muted speaker at 0.
#[derive(Debug, PartialEq)]
pub enum VolumeTag {
    Level(String),
    Off,
}

pub fn volume_tag(volume: f32) -> Option<VolumeTag> {
    match (volume * 100.0).round() as i32 {
        100 => None,
        0 => Some(VolumeTag::Off),
        v => Some(VolumeTag::Level(format!("{v}\u{a0}%"))),
    }
}

/// Ctrl/⌘ + scroll over someone's row: their volume in 5 % steps (0–200 %).
/// A mouse notch is one step; a trackpad's small deltas add up to one (50
/// points), like the web's. Up is louder.
#[derive(Default)]
pub struct WheelVolume {
    uid: String,
    sum: f32,
}

impl WheelVolume {
    /// Scroll over `uid`'s row (points, up positive): their new volume, when a step is taken.
    pub fn scroll(&mut self, uid: &str, delta: f32, current: f32) -> Option<f32> {
        if self.uid != uid {
            *self = WheelVolume { uid: uid.to_string(), sum: 0.0 };
        }
        self.sum += delta;
        if self.sum.abs() < 50.0 {
            return None;
        }
        let step = if self.sum > 0.0 { 5 } else { -5 };
        self.sum = 0.0;
        let now = (current * 100.0).round() as i32;
        let next = (((now + step) as f32 / 5.0).round() as i32 * 5).clamp(0, 200);
        (next != now).then_some(next as f32 / 100.0)
    }
}

/// How much Ctrl/⌘ + scroll moved this frame, in points (a mouse notch ≈ 33, like a browser's line).
fn command_scroll(ui: &egui::Ui) -> f32 {
    ui.input(|i| {
        i.raw
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, modifiers, .. } if modifiers.command || modifiers.ctrl => Some(match unit {
                    egui::MouseWheelUnit::Point => delta.y,
                    egui::MouseWheelUnit::Line => delta.y * 33.0,
                    egui::MouseWheelUnit::Page => delta.y * 400.0,
                }),
                _ => None,
            })
            .sum()
    })
}

/// What the row's tooltip says about the wheel.
const SCROLL_HOW: &str = if cfg!(target_os = "macos") { "⌘ + scroll" } else { "Ctrl + scroll" };

/// Who's in a voice channel, under it: speaking rings, "connecting", the
/// LIVE badge, connection hints, mute / deafen marks; click someone for
/// their volume. (Not OBS links: they listen without showing up, unless
/// their account has left: `obs_listening`.)
pub fn voice_members(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, channel_id: &str, joined: bool, acts: &mut Vec<Act>) {
    for s in m.voice_members(channel_id) {
        let me = Some(&s.sid) == m.sid.as_ref();
        let user = m.users.get(&s.uid);
        let name = user.map(|u| u.name.as_str()).unwrap_or("Unknown");
        let (rect, resp) = super::row(ui, ROW_H, ("vm", &s.sid));
        let c = pos2(rect.left() + SUB_X, rect.center().y);
        let ghost = m.ghosts.contains_key(&s.sid);
        let connecting = joined && !me && !ghost && m.peer_states.get(&s.sid).map(String::as_str) != Some("connected");
        let hint = if joined && !me { m.hints.get(&s.sid) } else { None };
        let speaking = m.speaking.contains(&s.sid);
        // Your volume for them (not on your own row), and room for it after the name.
        let volume = m.config.volume_for(&s.uid);
        let tag = if me { None } else { volume_tag(volume) };
        let tag_font = theme::font(theme::small() - 1.0);
        let tag_g = match &tag {
            Some(VolumeTag::Level(t)) => Some(ui.painter().layout_no_wrap(t.clone(), tag_font.clone(), p.gray)),
            _ => None,
        };
        let tag_w = match &tag {
            Some(VolumeTag::Level(_)) => tag_g.as_ref().map_or(0.0, |g| g.size().x) + 5.0,
            Some(VolumeTag::Off) => 16.0,
            None => 0.0,
        };
        // A long name ends in "…" before the mute / deafen mark.
        let flagged = s.voice.as_ref().is_some_and(|v| v.muted || v.deafened);
        let name_w = rect.right() - if flagged { 26.0 } else { 6.0 } - (c.x + SUB_GAP) - tag_w;
        // Someone you've turned all the way down, talking: their ring, dimmed.
        let ring = match (speaking, tag == Some(VolumeTag::Off)) {
            (true, true) => Ring::Dim,
            (true, false) => Ring::Speaking,
            _ => Ring::None,
        };
        avatar_ring(ui, &app.images, &m.config.server, c, SUB_AVATAR, face(user), ring, p);
        let color = if connecting { p.gray } else { p.text };
        let g = fit(ui.painter(), name, theme::font(theme::ui()), name_w, color);
        let at = pos2(c.x + SUB_GAP, rect.center().y - g.size().y / 2.0);
        let w = g.size().x;
        ui.painter().galley(at, g, color);
        if resp.hovered() && !me {
            ui.painter().hline(at.x..=at.x + w, (rect.center().y + 7.0).round(), egui::Stroke::new(1.0, color));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let (mut name_right, red) = (at.x + w, p.danger);
        match (&tag, tag_g) {
            (Some(VolumeTag::Level(_)), Some(g)) => {
                ui.painter().galley(pos2(name_right + 5.0, rect.center().y - g.size().y / 2.0), g.clone(), p.gray);
                name_right += 5.0 + g.size().x;
            }
            (Some(VolumeTag::Off), _) => {
                icons::paint(ui.painter(), Rect::from_center_size(pos2(name_right + 10.0, c.y), vec2(12.0, 12.0)), "volume-x", p.gray);
                name_right += 16.0;
            }
            _ => {}
        }
        // Sharing their screen: LIVE opens it here (ours: stop sharing, like the web).
        let mut after = name_right;
        if s.voice.as_ref().is_some_and(|v| v.screen) {
            let badge = super::live_badge(ui, pos2(name_right + 6.0, c.y), &s.sid, red, name, me);
            after = badge.rect.right();
            if badge.clicked() {
                acts.push(Act::Cmd(if me { Cmd::StopSharing } else { Cmd::WatchScreen(s.sid.clone()) }));
            }
        }
        if let Some(h) = hint {
            let flags = if s.voice.as_ref().is_some_and(|v| v.muted || v.deafened) { 26.0 } else { 6.0 };
            hint_label(ui, p, pos2(after + 6.0, c.y), rect.right() - flags - after - 6.0, h);
        }
        if let Some(v) = &s.voice {
            let flag = if v.deafened { Some("headphones-off") } else if v.muted { Some("mic-off") } else { None };
            if let Some(f) = flag {
                let (inset, size) = (12.0, 13.0);
                icons::paint(ui.painter(), Rect::from_center_size(rect.right_center() - vec2(inset, 0.0), vec2(size, size)), f, red);
            }
        }
        let tip = match hint {
            Some(h) => h.title(name),
            None if connecting => "Connecting…".to_string(),
            None if me => "You".to_string(),
            None => match &tag {
                Some(VolumeTag::Off) => format!("{name}: you turned them all the way down (only for you) — click for volume, or {SCROLL_HOW}"),
                Some(VolumeTag::Level(t)) => format!("{name} at {t} for you (only you hear it this way) — click for volume, or {SCROLL_HOW}"),
                None => format!("{name} — click for volume, or {SCROLL_HOW}"),
            },
        };
        // Ctrl/⌘ + scroll over them: their volume, 5 % a step.
        if resp.hovered() && !me {
            let delta = command_scroll(ui);
            if delta != 0.0
                && let Some(v) = app.wheel_volume.scroll(&s.uid, delta, volume)
            {
                let uid = s.uid.clone();
                acts.push(Act::Config(Box::new(move |c| {
                    c.volumes.insert(uid, v);
                })));
                acts.push(Act::Cmd(Cmd::SetVolume { uid: s.uid.clone() }));
            }
        }
        let resp = theme::tip(resp, tip);
        // Click or right-click someone: their volume (+ Watch screen, mod actions). Not yourself.
        if (resp.clicked() || resp.secondary_clicked()) && !me {
            app.volume_popup = Some((s.uid.clone(), resp.rect.left_bottom() + vec2(30.0, 4.0)));
            app.volume_ask = None;
        }
    }
    obs_listening(app, ui, m, p, channel_id);
}

/// Someone who left the call while their OBS link still listens (PROTOCOL.md ›
/// Voice › OBS links in a call): their name greyed out under the channel,
/// "Rosa's OBS is still listening". An in-between state, not a call member:
/// a faded picture, no ring, no marks, no volume on click.
fn obs_listening(app: &App, ui: &mut egui::Ui, m: &Model, p: &Pal, channel_id: &str) {
    for uid in m.obs_listening(channel_id) {
        let user = m.users.get(uid);
        let name = user.map(|u| u.name.as_str()).unwrap_or("Someone");
        let (rect, resp) = super::row(ui, ROW_H, ("obs", uid));
        let c = pos2(rect.left() + SUB_X, rect.center().y);
        super::avatar(ui, &app.images, &m.config.server, c, SUB_AVATAR, face(user), false, p);
        // Faded into the list's own background.
        let [r, g, b, _] = p.window.to_array();
        ui.painter().rect_filled(Rect::from_center_size(c, vec2(SUB_AVATAR + 2.0, SUB_AVATAR + 2.0)), 0.0, Color32::from_rgba_unmultiplied(r, g, b, 150));
        // "Rosa's OBS is still listening", grey italics (like "reconnecting…"), cut short with "…" if narrow.
        let at_x = c.x + SUB_GAP;
        let text = format!("{name}{OBS_LISTENING}");
        let mut job = egui::text::LayoutJob::single_section(text, egui::TextFormat { font_id: theme::font(theme::ui()), color: p.gray, italics: true, ..Default::default() });
        job.wrap = egui::text::TextWrapping { max_width: (rect.right() - 6.0 - at_x).max(20.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(pos2(at_x, rect.center().y - galley.size().y / 2.0), galley, p.gray);
        theme::tip(resp, obs_listening_tip(name));
    }
}

/// After the name, on an OBS link's greyed row (the spec's words: "<name>'s OBS is still listening").
pub const OBS_LISTENING: &str = "'s OBS is still listening";

/// The row's tooltip (the web's words).
pub fn obs_listening_tip(name: &str) -> String {
    format!("{name} left the call, but their OBS link is still listening. It stops within 5 minutes unless they come back.")
}

/// Places: the server's links (its board, if it has one; anyone can add an
/// https link with + or remove one with a right-click), Draw and Space (in
/// the app), the web-only views (in the browser, signed in), and who's on
/// each. (The Minecraft server is in the status bar.)
pub fn places(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    if m.links.is_none() && m.server_config.is_none() {
        return;
    }
    if list_heading(ui, p, "Places", Some("Add a link")) {
        app.modal = Some(Modal::AddLink { name: String::new(), url: String::new() });
    }
    let (name_font, dim) = (theme::font(theme::ui()), p.gray);
    for place in super::places(m) {
        let here = place.view.is_some() && place.view == m.view;
        let (_, resp) = list_row(ui, p, "place", &place.name, place.icon, &place.name, Look { active: here, ..Default::default() });
        if let Some(id) = &place.id {
            theme::context_menu(&resp, p, |ui| {
                if theme::menu_item(ui, &format!("Remove {}", place.name), false, p).clicked() {
                    acts.push(Act::Cmd(Cmd::RemoveLink(id.clone())));
                    ui.close();
                }
            });
        }
        if theme::tip(resp, &place.tip).clicked() {
            // (An in-app view takes the main window: the screen we watch closes, like the web.)
            if place.view.is_some() {
                super::screen::leaving(app, m, acts);
            }
            super::open_place(&place, acts);
        }
        // What's on (Watch together), and who's there (being on a shared view
        // or in the Watch audience makes you visible).
        if let Some(title) = super::watch_now(m, &place) {
            let (rect, resp) = super::row(ui, ROW_H, ("watch-now", &place.name));
            let (x, w) = (32.0, rect.width() - 36.0);
            super::now_playing(ui, pos2(rect.left() + x, rect.center().y), &title, name_font.clone(), dim, w);
            if theme::tip(resp, format!("Playing: {title}\nClick to watch in your browser")).clicked() {
                super::open_place(&place, acts);
            }
        }
        for (u, _) in super::place_people(m, &place) {
            let (rect, _) = super::row(ui, ROW_H, ("viewer", &format!("{}/{}", place.name, u.id)));
            let c = pos2(rect.left() + SUB_X, rect.center().y);
            super::avatar(ui, &app.images, &m.config.server, c, SUB_AVATAR, face(Some(u)), false, p);
            // Dimmed, so they don't read like more places (like the web's); a long name ends in "…".
            let color = p.gray;
            let g = fit(ui.painter(), &u.name, name_font.clone(), rect.right() - 6.0 - (c.x + SUB_GAP), color);
            ui.painter().galley(pos2(c.x + SUB_GAP, rect.center().y - g.size().y / 2.0), g, color);
        }
    }
}

// ── Information bars ───────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Bar {
    Hang,
    Offline,
    SigninAlert,
    /// The "Choose a password" prompt (`ready.weakPin`, until Later).
    ChoosePassword,
    /// An account with no password at all (`ready.needsPin`).
    NoPassword,
    Joins,
}

/// Is there a bar to show over the chat (connection lost, set a password,
/// people waiting, a new sign-in, choose a password, a hang starting)?
pub fn has_bars(app: &App, m: &Model) -> bool {
    !bars_now(app, m).is_empty()
}

fn offline(m: &Model) -> bool {
    !m.channels.is_empty() && (m.status == Status::Offline || m.status == Status::Connecting && m.sid.is_none())
}

fn bars_now(app: &App, m: &Model) -> Vec<Bar> {
    let on = |b: &Bar| match b {
        Bar::Hang => super::extras::hang_notice(app, m).is_some(),
        Bar::Offline => offline(m),
        Bar::SigninAlert => m.signin_alert.is_some(),
        Bar::ChoosePassword => m.password_prompt().is_some(),
        Bar::NoPassword => m.needs_pin,
        // Only the owner and mods let people in (an older server told everyone).
        Bar::Joins => !m.join_requests.is_empty() && m.am_moderator(),
    };
    let order = [Bar::Hang, Bar::SigninAlert, Bar::ChoosePassword, Bar::Offline, Bar::NoPassword, Bar::Joins];
    order.into_iter().filter(on).collect()
}

/// A bar button: label, kind, size (None = the standard 75×23).
type BarButton<'a> = (&'a str, theme::Btn, Option<egui::Vec2>);

/// How the bars look: yellow information bars (black text, light buttons
/// with fixed sizes).
struct BarLook {
    light: Pal,
}

impl BarLook {
    fn bar(&self, ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
        ui.add_space(3.0);
        theme::info_bar(ui, add);
    }

    fn icon(&self, ui: &mut egui::Ui, name: &str) {
        let (r, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
        icons::paint(ui.painter(), r, name, theme::INFO_TEXT);
    }

    /// How wide [`Self::button`] makes this one.
    fn button_w(&self, ui: &egui::Ui, label: &str, kind: theme::Btn, size: Option<egui::Vec2>) -> f32 {
        match size {
            Some(min) => {
                let f = if kind == theme::Btn::Default { theme::bold_font(theme::UI) } else { theme::font(theme::UI) };
                (ui.painter().layout_no_wrap(label.to_string(), f, Color32::WHITE).size().x + 20.0).max(min.x).round()
            }
            None => theme::button_width(ui, label, kind),
        }
    }

    /// A bar's words as one wrapped text: strong parts bold, plain ones regular.
    fn words(&self, ui: &mut egui::Ui, parts: &[(&str, bool)], width: f32) {
        let mut job = egui::text::LayoutJob::default();
        for (i, (text, strong)) in parts.iter().enumerate() {
            let f = if *strong { theme::bold_font(theme::ui()) } else { theme::font(theme::ui()) };
            job.append(text, if i > 0 { 4.0 } else { 0.0 }, egui::TextFormat { font_id: f, color: theme::INFO_TEXT, ..Default::default() });
        }
        job.wrap.max_width = width.max(60.0);
        theme::job_label(ui, job);
    }

    /// A bar's row: an icon, the words wrapped short of the buttons, the
    /// buttons on the right (`buttons[0]` rightmost); in a narrow window the
    /// buttons go on a row of their own under the words, so nothing overlaps.
    /// Returns which button was clicked.
    fn row(&self, ui: &mut egui::Ui, icon: Option<&str>, parts: &[(&str, bool)], buttons: &[BarButton<'_>]) -> Option<usize> {
        let gap = ui.spacing().item_spacing.x;
        let icon_w = icon.map_or(0.0, |_| 13.0 + gap);
        let used: f32 = buttons.iter().map(|(l, k, s)| self.button_w(ui, l, *k, *s) + gap).sum();
        let room = ui.available_width() - icon_w - used - gap;
        let stacked = room < 200.0;
        let mut clicked = None;
        let mut put_buttons = |ui: &mut egui::Ui| {
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 23.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (i, (label, kind, size)) in buttons.iter().enumerate() {
                    if self.button(ui, label, *kind, *size).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        };
        ui.horizontal(|ui| {
            if let Some(name) = icon {
                self.icon(ui, name);
            }
            let width = if stacked { ui.available_width() } else { room };
            self.words(ui, parts, width);
            if !stacked {
                put_buttons(ui);
            }
        });
        if stacked {
            put_buttons(ui);
        }
        clicked
    }

    /// A bar button: a fixed size on the light palette (None = the standard 75×23).
    fn button(&self, ui: &mut egui::Ui, label: &str, kind: theme::Btn, size: Option<egui::Vec2>) -> egui::Response {
        match size {
            Some(min) => theme::button_sized(ui, label, kind, true, min, &self.light),
            None => theme::button(ui, label, kind, true, &self.light),
        }
    }
}

/// The information bars over the chat.
pub fn bars(app: &mut App, ui: &mut egui::Ui, m: &Model, acts: &mut Vec<Act>) {
    let look = BarLook { light: theme::light() };
    let right = egui::Layout::right_to_left(egui::Align::Center);
    for bar in bars_now(app, m) {
        match bar {
            Bar::Hang => {
                let Some((id, text, join)) = super::extras::hang_notice(app, m) else { continue };
                look.bar(ui, |ui| {
                    let mut buttons = vec![("Dismiss", theme::Btn::Normal, Some(vec2(64.0, 21.0)))];
                    if join.is_some() {
                        buttons.push(("Join voice", theme::Btn::Default, Some(vec2(80.0, 21.0))));
                    }
                    match look.row(ui, Some("calendar"), &[(&text, true)], &buttons) {
                        Some(0) => app.hang_dismissed = Some(id.clone()),
                        Some(_) => {
                            if let Some(c) = &join {
                                acts.push(Act::Cmd(Cmd::JoinVoice(c.clone())));
                            }
                            app.hang_dismissed = Some(id.clone());
                        }
                        None => {}
                    }
                });
            }
            Bar::Offline => {
                look.bar(ui, |ui| {
                    let text = "Connection lost — reconnecting…";
                    let rt = egui::RichText::new(text).font(theme::bold_font(theme::ui())).color(Color32::from_rgb(0xc0, 0, 0));
                    ui.vertical_centered(|ui| ui.label(rt));
                });
            }
            Bar::SigninAlert => {
                let Some(d) = &m.signin_alert else { continue };
                look.bar(ui, |ui| {
                    let words = format!("New sign-in: {}, just now.", d.label);
                    // "Sign out everywhere else…" asks first, with the devices in view (like the web).
                    match look.row(ui, Some("lock"), &[(&words, true), ("Not you?", false)], &[("It was me", theme::Btn::Normal, Some(vec2(70.0, 21.0))), ("Sign out everywhere else…", theme::Btn::Default, Some(vec2(160.0, 21.0)))]) {
                        Some(0) => acts.push(Act::DismissSigninAlert),
                        Some(_) => acts.push(Act::AskSignOutOthers),
                        None => {}
                    }
                });
            }
            Bar::ChoosePassword => {
                let Some(reason) = m.password_prompt() else { continue };
                look.bar(ui, |ui| {
                    // Later puts this reason off on this device for good (Settings ›
                    // Account keeps "Set a password"); Choose a password opens the
                    // dialog, and the bar goes once the password is saved (like the web).
                    match look.row(ui, Some("lock"), &[("Choose a password", true), (password_prompt_words(reason), false)], &[("Later", theme::Btn::Normal, Some(vec2(60.0, 21.0))), ("Choose a password", theme::Btn::Default, Some(vec2(120.0, 21.0)))]) {
                        Some(0) => acts.push(Act::PasswordLater(reason)),
                        Some(_) => app.modal = Some(Modal::Password(Default::default())),
                        None => {}
                    }
                });
            }
            Bar::NoPassword => {
                look.bar(ui, |ui| {
                    ui.horizontal(|ui| {
                        let room = ui.available_width() - look.button_w(ui, "Set a password…", theme::Btn::Normal, None) - 2.0 * ui.spacing().item_spacing.x;
                        look.words(ui, &[("Set a password so you can sign in on your other devices as yourself.", false)], room);
                        // The button right after the words.
                        if look.button(ui, "Set a password…", theme::Btn::Normal, None).clicked() {
                            app.modal = Some(Modal::Password(Default::default()));
                        }
                    });
                });
            }
            Bar::Joins => {
                let (fg, dim) = (theme::INFO_TEXT, theme::INFO_TEXT);
                // One or two get their own bar; more collapse into one (Review
                // opens People) so the chat stays visible — the web's rule.
                if m.join_requests.len() > 2 {
                    look.bar(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(join_words(None, m.join_requests.len(), None, fg, dim));
                            ui.with_layout(right, |ui| {
                                if look.button(ui, "Review", theme::Btn::Default, None).clicked() {
                                    app.modal = Some(Modal::People { open: None, ask: None });
                                }
                            });
                        });
                    });
                    continue;
                }
                for u in &m.join_requests {
                    look.bar(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            super::avatar(ui, &app.images, &m.config.server, r.center(), 16.0, face(Some(u)), false, &look.light);
                            // The words wrap short of the buttons (a narrow window).
                            let room = (ui.available_width() - 190.0).max(120.0);
                            let mut words = join_words(Some(&u.name), 1, u.invited_by.as_deref(), fg, dim);
                            words.wrap.max_width = room;
                            match u.invite.as_ref().and_then(|i| i.warning()) {
                                // A personal link that was already used: say so, so whoever lets them in can check first.
                                Some(w) => {
                                    ui.vertical(|ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        theme::job_label(ui, words);
                                        invite_warning(ui, &w, true, room);
                                    });
                                }
                                None => {
                                    theme::job_label(ui, words);
                                }
                            }
                            ui.with_layout(right, |ui| {
                                if look.button(ui, "Turn away", theme::Btn::Normal, None).clicked() {
                                    acts.push(Act::Cmd(Cmd::Approve { uid: u.id.clone(), approved: false }));
                                }
                                if look.button(ui, "Let in", theme::Btn::Default, None).clicked() {
                                    acts.push(Act::Cmd(Cmd::Approve { uid: u.id.clone(), approved: true }));
                                }
                            });
                        });
                    });
                }
            }
        }
    }
}

/// The "Choose a password" prompt's words, by why it asks (the web's).
fn password_prompt_words(reason: crate::core::WeakReason) -> &'static str {
    match reason {
        crate::core::WeakReason::Pin => "Yap uses passwords now. Your PIN still works until you choose one.",
        crate::core::WeakReason::Temporary => "You signed in with a temporary password. Choose your own.",
        crate::core::WeakReason::Guessable => "Your password is easy to guess. Choose a stronger one.",
    }
}

/// `text` on one line, ending in "…" if it's wider than `max_w`.
pub fn fit(painter: &egui::Painter, text: &str, font: egui::FontId, max_w: f32, color: Color32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(20.0));
    painter.layout_job(job)
}

/// The words [`fit`] keeps (with the "…"), for a layout job of several parts.
pub fn fit_text(painter: &egui::Painter, text: &str, font: egui::FontId, max_w: f32) -> String {
    let g = fit(painter, text, font, max_w, Color32::PLACEHOLDER);
    g.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| g.chr)).collect()
}

/// "This link was also used by Noor — check with Priya." under a join
/// request, with a warning sign: dark red on the yellow bar (`on_info`),
/// else the palette's red. Returns its height.
pub fn invite_warning(ui: &mut egui::Ui, text: &str, on_info: bool, max_w: f32) -> f32 {
    let size = theme::small();
    let color = if on_info { Color32::from_rgb(0x80, 0, 0) } else { theme::pal(ui.ctx()).danger };
    let f = theme::font(size);
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat { font_id: f, color, ..Default::default() });
    let icon = size + 4.0;
    job.wrap.max_width = (max_w - icon).max(80.0);
    let g = ui.painter().layout_job(job);
    let (r, _) = ui.allocate_exact_size(vec2(icon + g.size().x, g.size().y), Sense::hover());
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + size / 2.0, r.top() + g.rows.first().map_or(size, |row| row.rect().height()) / 2.0), vec2(size, size)), "alert", color);
    ui.painter().galley(pos2(r.left() + icon, r.top()), g, color);
    r.height()
}

/// A join request's words, like the web: "**Ada** wants to join · invited by
/// Ben", or (`name` None) "**3 people** want to join".
pub fn join_words(name: Option<&str>, n: usize, invited_by: Option<&str>, fg: Color32, dim: Color32) -> egui::text::LayoutJob {
    let bold = egui::TextFormat { font_id: theme::bold_font(theme::ui()), color: fg, ..Default::default() };
    let plain = egui::TextFormat { font_id: theme::font(theme::ui()), color: fg, ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    match name {
        Some(name) => {
            job.append(name, 0.0, bold);
            job.append(" wants to join", 0.0, plain);
        }
        None => {
            job.append(&format!("{n} people"), 0.0, bold);
            job.append(" want to join", 0.0, plain);
        }
    }
    if let Some(by) = invited_by {
        job.append(&format!("· invited by {by}"), 6.0, egui::TextFormat { font_id: theme::font(theme::small()), color: dim, ..Default::default() });
    }
    job
}

// ── Small shared pieces ────────────────────────────────────────────────

/// Who's typing, in words ("Ada is typing…"). Empty when nobody is.
pub fn typing_words(typers: &[String]) -> String {
    match typers.len() {
        0 => String::new(),
        1 => format!("{} is typing…", typers[0]),
        2 => format!("{} and {} are typing…", typers[0], typers[1]),
        _ => "Several people are typing…".into(),
    }
}

/// The ring around a buddy icon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ring {
    None,
    /// Talking: the green LED ring.
    Speaking,
    /// Talking, but you've turned them all the way down: the same green, dimmed (45 %, like the web).
    Dim,
}

/// Someone's picture (or colour + initial): a square buddy icon in a 1-pt
/// frame, a green LED ring while talking.
#[allow(clippy::too_many_arguments)]
pub fn avatar(ui: &egui::Ui, images: &super::images::Images, server: &str, center: Pos2, size: f32, f: Face<'_>, speaking: bool, p: &Pal) {
    avatar_ring(ui, images, server, center, size, f, if speaking { Ring::Speaking } else { Ring::None }, p);
}

/// [`avatar`] with any ring.
#[allow(clippy::too_many_arguments)]
pub fn avatar_ring(ui: &egui::Ui, images: &super::images::Images, server: &str, center: Pos2, size: f32, f: Face<'_>, ring: Ring, p: &Pal) {
    let painter = ui.painter();
    let pic = f.avatar.filter(|id| !id.is_empty()).and_then(|id| images.get_square(&format!("{server}/media/{id}")));
    let initial: String = f.name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    let rect = theme::RoundRect::round(Rect::from_center_size(center, vec2(size, size)));
    match ring {
        Ring::Speaking | Ring::Dim => {
            let green = if ring == Ring::Dim { Color32::from_rgba_unmultiplied(0, 0xe0, 0, 115) } else { Color32::from_rgb(0, 0xe0, 0) };
            painter.rect_filled(rect.expand(3.0), 0.0, green);
            painter.rect_filled(rect.expand(1.0), 0.0, p.dk);
        }
        Ring::None => theme::outline(painter, rect.expand(1.0), p.shadow),
    }
    painter.rect_filled(rect, 0.0, theme::hex_color(f.color));
    match pic {
        Some(tex) => {
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(tex, rect, uv, Color32::WHITE);
        }
        None => {
            let font = theme::bold_font((size * 0.45).max(9.0).round());
            if size >= 24.0 {
                painter.text(center + vec2(1.0, 1.0), Align2::CENTER_CENTER, &initial, font.clone(), Color32::from_black_alpha(115));
            }
            painter.text(center, Align2::CENTER_CENTER, initial, font, Color32::WHITE);
        }
    }
}

/// Toasts: tooltip balloons (pale yellow, black ring, a small hard shadow)
/// over the bottom.
pub fn toasts(ctx: &egui::Context, m: &Model) {
    if m.toasts.is_empty() {
        return;
    }
    let (anchor, offset) = (Align2::CENTER_BOTTOM, vec2(0.0, -84.0));
    egui::Area::new(egui::Id::new("toasts")).order(egui::Order::Tooltip).anchor(anchor, offset).interactable(false).show(ctx, |ui| {
        for (text, _) in &m.toasts {
            toast(ui, text);
        }
    });
}

fn toast(ui: &mut egui::Ui, text: &str) {
    balloon_frame(ui, 460.0, |ui| {
        ui.label(egui::RichText::new(text).font(theme::font(theme::ui())).color(theme::INFO_TEXT));
    });
    ui.add_space(6.0);
}

/// A tooltip balloon's look (pale yellow, black ring, a small hard shadow) around `add`.
pub fn balloon_frame(ui: &mut egui::Ui, max_w: f32, add: impl FnOnce(&mut egui::Ui)) {
    let bg = ui.painter().add(egui::Shape::Noop);
    let out = egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_max_width(max_w);
        add(ui);
    });
    let r = theme::RoundRect::round(out.response.rect);
    ui.painter().set(
        bg,
        egui::Shape::Vec(vec![egui::Shape::rect_filled(r.translate(vec2(2.0, 2.0)), 0.0, Color32::from_black_alpha(90)), egui::Shape::rect_filled(r, 0.0, theme::INFO)]),
    );
    theme::outline(ui.painter(), r, Color32::BLACK);
}

/// "You're muted" by the Mute button for a moment after you talked while
/// muted (PROTOCOL.md › Voice › Transmit gate); the first time on this
/// device it also says what the blip meant.
pub fn muted_balloon(ctx: &egui::Context, m: &Model, under: Rect) {
    let Some((at, explain)) = m.muted_nudge else { return };
    let left = if explain { 8.0 } else { 3.0 } - at.elapsed().as_secs_f32();
    if left <= 0.0 || !(m.config.muted || m.config.deafened) {
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_secs_f32(left.min(0.5)));
    egui::Area::new(egui::Id::new("muted-nudge")).order(egui::Order::Tooltip).fixed_pos(under.left_bottom() + vec2(0.0, 4.0)).interactable(false).show(ctx, |ui| {
        balloon_frame(ui, if explain { 260.0 } else { 200.0 }, |ui| {
            ui.label(egui::RichText::new(MUTED_TEXT).font(theme::bold_font(theme::ui())).color(theme::INFO_TEXT));
            if explain {
                ui.label(egui::RichText::new(MUTED_EXPLAIN).font(theme::font(theme::ui())).color(theme::INFO_TEXT));
            }
        });
    });
}

/// The spec's words (PROTOCOL.md › Voice › Transmit gate › Talking while muted).
pub const MUTED_TEXT: &str = "You're muted";
pub const MUTED_EXPLAIN: &str = "That sound means you're muted. Turn it off in Settings › Voice.";

/// A quiet line of this session's own in the chat (PROTOCOL.md › Since you
/// left): "Theo joined — invited by Priya", "You're in. Say hi 👋": small
/// and grey with an icon.
pub fn local_note(ui: &mut egui::Ui, n: &crate::core::LocalNote, p: &Pal) {
    let (size, fg) = (theme::small(), p.gray);
    let name_color = match n.color.as_deref() {
        Some(c) => theme::hex_color(c),
        None => p.text,
    };
    let (bold, plain) = (theme::bold_font(size), theme::font(size));
    let mut job = egui::text::LayoutJob::default();
    job.append(&n.bold, 0.0, egui::TextFormat { font_id: bold, color: name_color, ..Default::default() });
    job.append(&n.text, 0.0, egui::TextFormat { font_id: plain, color: fg, ..Default::default() });
    let icon = size + 4.0;
    let side = 8.0;
    job.wrap.max_width = (ui.available_width() - side * 2.0 - icon).max(60.0);
    let g = ui.painter().layout_job(job);
    let (top, bottom) = (6.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), g.size().y + top + bottom), Sense::hover());
    let x = rect.left() + side;
    let y = rect.top() + top;
    icons::paint(ui.painter(), Rect::from_center_size(pos2(x + size / 2.0, y + g.rows.first().map_or(size, |r| r.rect().height()) / 2.0), vec2(size, size)), n.icon, fg);
    ui.painter().galley(pos2(x + icon, y), g, fg);
}

/// The line between two days: etched rules with the date between.
pub fn day_divider(ui: &mut egui::Ui, ts: i64, p: &Pal) {
    ui.add_space(10.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let g = ui.painter().layout_no_wrap(super::fmt_day(ts), theme::font(theme::small()), p.gray);
    let tw = g.size().x;
    let y = rect.center().y - 1.0;
    let (l, r) = (rect.left() + 8.0, rect.right() - 8.0);
    let mid = rect.center().x;
    theme::etched_hline(ui.painter(), egui::Rangef::new(l, (mid - tw / 2.0 - 8.0).max(l)), y, p);
    theme::etched_hline(ui.painter(), egui::Rangef::new((mid + tw / 2.0 + 8.0).min(r), r), y, p);
    ui.painter().galley(pos2(mid - tw / 2.0, rect.center().y - g.size().y / 2.0), g, p.gray);
    ui.add_space(4.0);
}

/// The very start of a channel: "Welcome to <name>" (the Personal space:
/// its intro and card).
pub fn channel_start(app: &mut App, ui: &mut egui::Ui, m: &Model, channel: &crate::protocol::Channel, p: &Pal, acts: &mut Vec<Act>) {
    let name = channel.name.as_str();
    let personal = channel.is_personal();
    ui.add_space(16.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.vertical(|ui| {
            if personal {
                ui.set_max_width(ui.available_width() - 20.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(egui::RichText::new(name).font(theme::bold_font(theme::chat())).color(p.text));
                ui.label(theme::text(super::chat::PERSONAL_INTRO, p.text));
                super::chat::personal_card(app, ui, m, acts);
            } else {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new(format!("Welcome to {name}")).font(theme::bold_font(theme::chat())).color(p.text));
                ui.label(theme::text("This is the very beginning of the channel.", p.gray));
            }
        });
    });
    ui.add_space(8.0);
}

#[cfg(test)]
mod row_tests {
    use super::*;

    #[test]
    fn your_volume_on_the_row() {
        assert_eq!(volume_tag(1.0), None, "unchanged: nothing");
        assert_eq!(volume_tag(0.6), Some(VolumeTag::Level("60\u{a0}%".into())));
        assert_eq!(volume_tag(1.5), Some(VolumeTag::Level("150\u{a0}%".into())));
        assert_eq!(volume_tag(0.0), Some(VolumeTag::Off), "all the way down: a muted speaker");
        assert_eq!(volume_tag(0.999), None);
    }

    /// The web's wheel: a notch (33 points) or two is a step only once they add up
    /// to 50; up is louder; 5 % steps on the 5 % grid, 0–200 %.
    #[test]
    fn command_scroll_changes_their_volume_in_steps() {
        let mut w = WheelVolume::default();
        assert_eq!(w.scroll("ana", 33.0, 1.0), None, "one line isn't enough yet");
        assert_eq!(w.scroll("ana", 33.0, 1.0), Some(1.05), "two add up: one step louder");
        assert_eq!(w.scroll("ana", -100.0, 1.05), Some(1.0), "down");
        assert_eq!(w.scroll("ben", 30.0, 0.5), None, "someone else starts over");
        assert_eq!(w.scroll("ben", 30.0, 0.5), Some(0.55));
        assert_eq!(w.scroll("ben", 60.0, 0.62), Some(0.65), "back onto the 5 % grid");
        assert_eq!(w.scroll("cy", 60.0, 2.0), None, "200 % is the top");
        assert_eq!(w.scroll("cy", -60.0, 0.0), None, "0 is the bottom");
        assert_eq!(w.scroll("cy", -60.0, 0.05), Some(0.0));
    }
}
