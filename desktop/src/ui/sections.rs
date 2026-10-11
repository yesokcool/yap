//! Pieces the windows share, written once: the sidebar's list rows and its
//! Upcoming, voice-channel and Places sections, the chat's information
//! bars, avatars, toasts, the typing line's words, day dividers and the
//! start of a channel. `RowLook` is how a list is drawn.

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

/// How the sidebar's lists are drawn (the palette of the moment).
#[derive(Clone, Copy)]
pub struct RowLook {
    pub p: Pal,
}

/// A row's height.
const ROW_98: f32 = 20.0;

impl RowLook {
    pub fn of(ui: &egui::Ui) -> Self {
        Self { p: theme::pal(ui.ctx()) }
    }

    fn id<'a>(&self, kind: &'a str, key: &'a str) -> (&'static str, &'a str, &'a str) {
        ("98", kind, key)
    }

    /// A group heading with an optional + button; true when it's clicked.
    pub fn heading(&self, ui: &mut egui::Ui, title: &str, add: Option<&str>) -> bool {
        let p = &self.p;
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
    pub fn row(&self, ui: &mut egui::Ui, kind: &str, key: &str, icon: &str, name: &str, look: Look) -> (Rect, egui::Response) {
        let id = self.id(kind, key);
        let p = &self.p;
        let (rect, resp) = super::row(ui, ROW_98, id);
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
    fn empty(&self, ui: &mut egui::Ui, kind: &str, text: &str) -> egui::Response {
        let id = self.id(kind, "");
        let (rect, resp) = super::row(ui, ROW_98, id);
        ui.painter().text(pos2(rect.left() + 14.0, rect.center().y), Align2::LEFT_CENTER, text, theme::font(theme::small()), self.p.gray);
        resp
    }

    fn ok(&self) -> Color32 {
        self.p.ok
    }
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
pub fn hangs(app: &mut App, ui: &mut egui::Ui, m: &Model, look: &RowLook, acts: &mut Vec<Act>) {
    if m.status != Status::Online {
        return;
    }
    if look.heading(ui, "Upcoming", Some("Plan a hang")) {
        app.modal = Some(Modal::PlanHang(super::extras::PlanHang::new(m)));
    }
    let list = super::extras::upcoming_hangs(m);
    if list.is_empty() {
        let resp = look.empty(ui, "no-hangs", "Nothing planned");
        if theme::tip(resp, "Plan a hang: the people who are free then get one reminder").clicked() {
            app.modal = Some(Modal::PlanHang(super::extras::PlanHang::new(m)));
        }
        return;
    }
    let p = theme::pal(ui.ctx());
    for h in list {
        let when = super::extras::hang_when(h.at, app.h12);
        let now = when == "now";
        let (r, resp) = look.row(ui, "hang", &h.id, "calendar", &h.title, Look { bold: now, tint: now.then_some(look.ok()), ..Default::default() });
        // When, on the right (over the end of a long title).
        let color = if now { p.ok } else { p.gray };
        let g = ui.painter().layout_no_wrap(when, theme::font(theme::small()), color);
        let gw = g.size().x;
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.right() - gw - 12.0, r.top()), r.max), 0.0, p.window);
        ui.painter().galley(pos2(r.right() - 6.0 - gw, r.center().y - g.size().y / 2.0), g, color);
        if super::extras::can_cancel_hang(m, h) {
            let id = h.id.clone();
            theme::context_menu(&resp, &p, |ui| {
                if theme::menu_item(ui, "Cancel this hang", false, &p).clicked() {
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

/// Where a row under a row (someone in voice, someone on a Place) puts things.
struct SubRow {
    h: f32,
    /// The avatar's (or icon's) centre, from the row's left.
    x: f32,
    avatar: f32,
    /// From the avatar's centre to the name.
    gap: f32,
    icon: f32,
}

fn sub_row() -> SubRow {
    SubRow { h: ROW_98, x: 38.0, avatar: 14.0, gap: 12.0, icon: 14.0 }
}

/// A connection hint after someone's name ("weak connection", "reconnecting…"),
/// cut short with an ellipsis if the row is narrow. The web's colours: a
/// warning, grey italics while they're reconnecting, red when they can't connect.
fn hint_label(ui: &egui::Ui, look: &RowLook, at: Pos2, max_w: f32, hint: &Hint) {
    if max_w < 24.0 {
        return;
    }
    let p = &look.p;
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

/// Who's in a voice channel, under it: speaking rings, "connecting", the
/// LIVE badge, connection hints, mute / deafen marks; click someone for
/// their volume.
pub fn voice_members(app: &mut App, ui: &mut egui::Ui, m: &Model, look: &RowLook, channel_id: &str, joined: bool, acts: &mut Vec<Act>) {
    let mut members: Vec<_> = m.sessions.values().filter(|s| s.voice.as_ref().is_some_and(|v| v.channel_id == channel_id)).collect();
    members.sort_by_key(|s| m.user_name(&s.uid).to_lowercase());
    let sr = sub_row();
    let p = &look.p;
    for s in members {
        let me = Some(&s.sid) == m.sid.as_ref();
        let user = m.users.get(&s.uid);
        let name = user.map(|u| u.name.as_str()).unwrap_or("Unknown");
        let (rect, resp) = super::row(ui, sr.h, look.id("vm", &s.sid));
        let c = pos2(rect.left() + sr.x, rect.center().y);
        if s.is_obs() {
            // A listen-only OBS Browser Source: shown, dimmed, no menu.
            let dim = p.gray;
            icons::paint(ui.painter(), Rect::from_center_size(c, vec2(sr.icon, sr.icon)), "tv", dim);
            ui.painter().text(pos2(c.x + sr.gap, c.y), Align2::LEFT_CENTER, format!("{name}'s OBS"), theme::font(theme::ui()), dim);
            theme::tip(resp, "Listening for a stream (OBS)");
            continue;
        }
        let ghost = m.ghosts.contains_key(&s.sid);
        let connecting = joined && !me && !ghost && m.peer_states.get(&s.sid).map(String::as_str) != Some("connected");
        let hint = if joined && !me { m.hints.get(&s.sid) } else { None };
        let speaking = m.speaking.contains(&s.sid);
        // A long name ends in "…" before the mute / deafen mark.
        let flagged = s.voice.as_ref().is_some_and(|v| v.muted || v.deafened);
        let name_w = rect.right() - if flagged { 26.0 } else { 6.0 } - (c.x + sr.gap);
        super::avatar(ui, &app.images, &m.config.server, c, sr.avatar, face(user), speaking, p);
        let color = if connecting { p.gray } else { p.text };
        let g = fit(ui.painter(), name, theme::font(theme::ui()), name_w, color);
        let at = pos2(c.x + sr.gap, rect.center().y - g.size().y / 2.0);
        let w = g.size().x;
        ui.painter().galley(at, g, color);
        if resp.hovered() && !me {
            ui.painter().hline(at.x..=at.x + w, (rect.center().y + 7.0).round(), egui::Stroke::new(1.0, color));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let (name_right, red) = (at.x + w, p.danger);
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
            hint_label(ui, look, pos2(after + 6.0, c.y), rect.right() - flags - after - 6.0, h);
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
            None => format!("{name} — click for volume"),
        };
        let resp = theme::tip(resp, tip);
        // Click or right-click someone: their volume (+ Watch screen, mod actions). Not yourself.
        if (resp.clicked() || resp.secondary_clicked()) && !me {
            app.volume_popup = Some((s.uid.clone(), resp.rect.left_bottom() + vec2(30.0, 4.0)));
            app.volume_ask = None;
        }
    }
}

/// Places: the server's links (its board, if it has one; anyone can add an
/// https link with + or remove one with a right-click), Draw and Space (in
/// the app), the web-only views (in the browser, signed in), and who's on
/// each. (The Minecraft server is in the status bar.)
pub fn places(app: &mut App, ui: &mut egui::Ui, m: &Model, look: &RowLook, acts: &mut Vec<Act>) {
    if m.links.is_none() && m.server_config.is_none() {
        return;
    }
    if look.heading(ui, "Places", Some("Add a link")) {
        app.modal = Some(Modal::AddLink { name: String::new(), url: String::new() });
    }
    let p = theme::pal(ui.ctx());
    let sr = sub_row();
    let (sub_h, name_font, dim) = (ROW_98, theme::font(theme::ui()), p.gray);
    for place in super::places(m) {
        let here = place.view.is_some() && place.view == m.view;
        let (_, resp) = look.row(ui, "place", &place.name, place.icon, &place.name, Look { active: here, ..Default::default() });
        if let Some(id) = &place.id {
            theme::context_menu(&resp, &p, |ui| {
                if theme::menu_item(ui, &format!("Remove {}", place.name), false, &p).clicked() {
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
            let (rect, resp) = super::row(ui, sub_h, look.id("watch-now", &place.name));
            let (x, w) = (32.0, rect.width() - 36.0);
            super::now_playing(ui, pos2(rect.left() + x, rect.center().y), &title, name_font.clone(), dim, w);
            if theme::tip(resp, format!("Playing: {title}\nClick to watch in your browser")).clicked() {
                super::open_place(&place, acts);
            }
        }
        for (u, _) in super::place_people(m, &place) {
            let (rect, _) = super::row(ui, sub_h, look.id("viewer", &format!("{}/{}", place.name, u.id)));
            let c = pos2(rect.left() + sr.x, rect.center().y);
            super::avatar(ui, &app.images, &m.config.server, c, 14.0, face(Some(u)), false, &p);
            // Dimmed, so they don't read like more places (like the web's); a long name ends in "…".
            let color = p.gray;
            let g = fit(ui.painter(), &u.name, name_font.clone(), rect.right() - 6.0 - (c.x + sr.gap), color);
            ui.painter().galley(pos2(c.x + sr.gap, rect.center().y - g.size().y / 2.0), g, color);
        }
    }
}

// ── Information bars ───────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Bar {
    Hang,
    Offline,
    SigninAlert,
    WeakPin,
    NeedsPin,
    Joins,
}

/// Is there a bar to show over the chat (connection lost, set a PIN, people
/// waiting, a new sign-in, a weak PIN, a hang starting)?
pub fn has_bars(app: &App, m: &Model) -> bool {
    !bars_now(app, m).is_empty()
}

fn offline(m: &Model) -> bool {
    !m.channels.is_empty() && (m.status == Status::Offline || m.status == Status::Connecting && m.sid.is_none())
}

fn bars_now(app: &App, m: &Model) -> Vec<Bar> {
    let weak = m.weak_pin && !app.weak_pin_dismissed && !m.needs_pin;
    let on = |b: &Bar| match b {
        Bar::Hang => super::extras::hang_notice(app, m).is_some(),
        Bar::Offline => offline(m),
        Bar::SigninAlert => m.signin_alert.is_some(),
        Bar::WeakPin => weak,
        Bar::NeedsPin => m.needs_pin,
        Bar::Joins => !m.join_requests.is_empty(),
    };
    let order = [Bar::Hang, Bar::SigninAlert, Bar::WeakPin, Bar::Offline, Bar::NeedsPin, Bar::Joins];
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
            Bar::WeakPin => {
                look.bar(ui, |ui| {
                    match look.row(ui, None, &[("Your PIN is easy to guess — change it, so a friend can't sign in as you.", false)], &[("Later", theme::Btn::Normal, Some(vec2(60.0, 21.0))), ("Change…", theme::Btn::Default, Some(vec2(70.0, 21.0)))]) {
                        Some(0) => app.weak_pin_dismissed = true,
                        Some(_) => acts.push(Act::OpenSettings(super::dialogs::TAB_ACCOUNT)),
                        None => {}
                    }
                });
            }
            Bar::NeedsPin => {
                look.bar(ui, |ui| {
                    ui.horizontal(|ui| {
                        let room = ui.available_width() - look.button_w(ui, "Set PIN…", theme::Btn::Normal, None) - 2.0 * ui.spacing().item_spacing.x;
                        look.words(ui, &[("Set a PIN or a password so you can sign in on your other devices as yourself.", false)], room);
                        // The button right after the words.
                        if look.button(ui, "Set PIN…", theme::Btn::Normal, None).clicked() {
                            app.modal = Some(Modal::SetPin { pin: String::new(), confirm: String::new(), password: false });
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

/// Someone's picture (or colour + initial): a square buddy icon in a 1-pt
/// frame, a green LED ring while talking.
#[allow(clippy::too_many_arguments)]
pub fn avatar(ui: &egui::Ui, images: &super::images::Images, server: &str, center: Pos2, size: f32, f: Face<'_>, speaking: bool, p: &Pal) {
    let painter = ui.painter();
    let pic = f.avatar.filter(|id| !id.is_empty()).and_then(|id| images.get_square(&format!("{server}/media/{id}")));
    let initial: String = f.name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    let rect = theme::RoundRect::round(Rect::from_center_size(center, vec2(size, size)));
    if speaking {
        painter.rect_filled(rect.expand(3.0), 0.0, Color32::from_rgb(0, 0xe0, 0));
        painter.rect_filled(rect.expand(1.0), 0.0, p.dk);
    } else {
        theme::outline(painter, rect.expand(1.0), p.shadow);
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
    let bg = ui.painter().add(egui::Shape::Noop);
    let out = egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_max_width(460.0);
        ui.label(egui::RichText::new(text).font(theme::font(theme::ui())).color(theme::INFO_TEXT));
    });
    let r = theme::RoundRect::round(out.response.rect);
    ui.painter().set(
        bg,
        egui::Shape::Vec(vec![egui::Shape::rect_filled(r.translate(vec2(2.0, 2.0)), 0.0, Color32::from_black_alpha(90)), egui::Shape::rect_filled(r, 0.0, theme::INFO)]),
    );
    theme::outline(ui.painter(), r, Color32::BLACK);
    ui.add_space(6.0);
}

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
