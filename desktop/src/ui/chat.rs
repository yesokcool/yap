//! Middle pane: header, message list (grouping, day dividers, paging,
//! unread-friendly scrolling), composer, typing line, emoji picker.

use super::{Act, App, avatar, content, dialogs::Modal, face, fmt_stamp, fmt_time, icons, markup, theme, theme::Bevel, theme::Pal, theme::RoundRect};
use crate::core::{Cmd, Model, ScrollHint, Status};
use crate::protocol::{Gif, Message};
use eframe::egui::{self, Align2, Color32, Rect, Vec2, vec2};

const GROUP_MS: i64 = 7 * 60_000;

/// The Personal space's start-of-channel text (web: personalStartHtml).
pub const PERSONAL_INTRO: &str = "Only you can see this. Send yourself links, pictures and notes to grab on your other devices.";
pub const PERSONAL_ENCRYPTED: &str = "End-to-end encrypted with your passcode: this device and your others that know it can read these notes; Yap's server and its backups only ever see scrambled text. Forget the passcode and they're gone for good. (Someone holding your unlocked device can still read it.)";
pub const PERSONAL_LOCKED: &str = "Locked on this device.";
pub const PERSONAL_PLAIN: &str = "Not encrypted yet: Yap's server (and its backups) can read it like any other message.";

/// The composer's placeholder for a channel.
pub fn composer_hint(m: &Model, channel: Option<&crate::protocol::Channel>) -> String {
    match channel {
        Some(c) if c.is_personal() && content::locked_here(m) => "Unlock your Personal space to write".into(),
        Some(c) if c.is_personal() => "Note to self: a link, a picture, anything".into(),
        Some(c) => format!("Message {}", c.name),
        None => "Message".into(),
    }
}
const GUTTER: f32 = 28.0;

/// The longest message the server takes (characters).
pub const MAX_BODY: usize = 40_000;

/// Under the Personal space's intro: encrypted, locked, or not encrypted yet
/// (with the button that changes that).
pub fn personal_card(app: &mut App, ui: &mut egui::Ui, m: &Model, acts: &mut Vec<Act>) {
    let _ = acts;
    let (text, button, mode) = match (&m.vault, &m.vault_key) {
        (Some(_), Some(_)) => (PERSONAL_ENCRYPTED, None, None),
        (Some(_), None) => (PERSONAL_LOCKED, Some("Unlock…"), Some(super::dialogs::VaultMode::Unlock)),
        _ => (PERSONAL_PLAIN, Some("Encrypt it with a passcode…"), Some(super::dialogs::VaultMode::Setup)),
    };
    let mut clicked = false;
    let body = |ui: &mut egui::Ui, fg: Color32, bp: &Pal| {
        let mut clicked = false;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 16.0), egui::Sense::hover());
            icons::paint(ui.painter(), r, "lock", fg);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let rt = egui::RichText::new(text).font(theme::font(theme::ui())).color(fg);
                ui.label(if m.vault.is_some() { rt.strong() } else { rt });
                if let Some(b) = button {
                    clicked = theme::button_sized(ui, b, theme::Btn::Default, true, vec2(90.0, 23.0), bp).clicked();
                }
            });
        });
        clicked
    };
    let light = theme::light();
    theme::info_bar(ui, |ui| {
        clicked = body(ui, theme::INFO_TEXT, &light);
    });
    if clicked && let Some(mode) = mode {
        app.modal = Some(Modal::Vault(super::dialogs::VaultDialog::new(mode)));
    }
}

/// We never got in this run (no channels yet, offline or still connecting):
/// the chat pane says so in the middle, with the reason and what to do,
/// instead of an empty chat with a live message box.
pub fn never_connected(m: &Model) -> bool {
    m.channels.is_empty() && matches!(m.status, Status::Offline | Status::Connecting)
}

/// The server's host, for people ("chat.example.com").
pub fn host(server: &str) -> &str {
    server.split("://").nth(1).unwrap_or(server).trim_end_matches('/')
}

/// "Can't reach <host>" (or "Connecting to <host>…"), the reason, Try again
/// and "Use a different invite link…". Fills `ui`.
/// Why the socket dropped, in plain words (the raw error stays in a tooltip):
/// "IO error: Connection refused (os error 61)" → "Nothing answered at that address."
pub fn offline_words(detail: &str) -> &'static str {
    let d = detail.to_lowercase();
    if d.contains("refused") {
        "Nothing answered at that address."
    } else if d.contains("lookup") || d.contains("resolve") || d.contains("nodename") || d.contains("dns") || d.contains("no such host") {
        "That address wasn't found. Is this computer online?"
    } else if d.contains("timed out") || d.contains("timeout") {
        "The server didn't answer in time."
    } else if d.contains("unreachable") || d.contains("no route") {
        "This computer can't reach the internet right now."
    } else if d.contains("tls") || d.contains("certificate") {
        "Couldn't make a secure connection to the server."
    } else if d.contains("http error") || d.contains("status") {
        "The server answered, but not like a Yap server."
    } else {
        "The connection dropped."
    }
}

pub fn not_connected(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let offline = m.status == Status::Offline;
    let host = host(&m.config.server).to_string();
    let (fg, dim) = (p.text, p.gray);
    let width = (ui.available_width() - 40.0).clamp(160.0, 380.0).min(ui.available_width());
    ui.add_space((ui.available_height() * 0.28).max(16.0));
    ui.vertical_centered(|ui| {
        ui.set_max_width(width);
        ui.spacing_mut().item_spacing.y = 6.0;
        let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), egui::Sense::hover());
        icons::paint(ui.painter(), r, if offline { "signal" } else { "chat" }, dim);
        let title = if offline { format!("Can't reach {host}") } else { format!("Connecting to {host}…") };
        ui.add(egui::Label::new(egui::RichText::new(title).font(theme::bold_font(theme::ui() + 2.0)).color(fg)).wrap());
        if offline {
            if !m.status_detail.is_empty() {
                // Plain words; the error itself is under the pointer.
                let r = ui.add(egui::Label::new(egui::RichText::new(offline_words(&m.status_detail)).font(theme::font(theme::small())).color(dim)).wrap().sense(egui::Sense::hover()));
                theme::tip(r, &m.status_detail);
            }
            ui.add(egui::Label::new(egui::RichText::new("Yap keeps trying in the background. Check your connection, or ask whoever invited you if the address changed.").font(theme::font(theme::ui())).color(fg)).wrap());
            ui.add_space(4.0);
            // The two buttons side by side, centred (stacked when there's no room).
            let (retry, other) = ("Try again", "Use a different invite link…");
            let gap = 6.0;
            let w = theme::button_width(ui, retry, theme::Btn::Default) + gap + theme::button_width(ui, other, theme::Btn::Normal);
            let mut buttons = |ui: &mut egui::Ui| {
                if theme::button(ui, retry, theme::Btn::Default, true, p).clicked() {
                    acts.push(Act::Cmd(Cmd::Wake));
                }
                if theme::button(ui, other, theme::Btn::Normal, true, p).clicked() {
                    app.welcome = super::dialogs::Welcome::other_server(&m.config);
                }
            };
            if w <= ui.available_width() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    ui.add_space(((ui.available_width() - w) / 2.0).floor());
                    buttons(ui);
                });
            } else {
                buttons(ui);
            }
        }
    });
}

/// The quieter note after a title-bar title ("end-to-end encrypted"), from
/// `x`, cut short at `right` (left out when there's no room).
fn title_note(ui: &egui::Ui, x: f32, y: f32, text: &str, right: f32, p: &Pal) {
    let x = x + 8.0;
    if right - x < 30.0 {
        return;
    }
    let g = theme::elide(ui.painter(), text, theme::font(theme::small()), p.title_text, right - x);
    ui.painter().galley(egui::pos2(x, y - g.size().y / 2.0), g, p.title_text.gamma_multiply(0.85));
}

#[derive(Default)]
pub struct ScrollState {
    channel: String,
    content_h: f32,
    offset: f32,
    /// After older messages were prepended: (old content height, old offset).
    anchor: Option<(f32, f32)>,
    /// Next pass: set the offset to this.
    set_offset: Option<f32>,
    /// Keep re-asserting "at the bottom" for a few frames (content may still be growing).
    want_bottom: u8,
    /// The newest message last frame (id, ts): something newer from someone
    /// else while you're scrolled up is "↓ New messages".
    newest: Option<(String, i64)>,
    /// "↓ New messages" shows (until you're at the bottom, or another channel).
    new_below: bool,
    /// Screenshot aid: one frame without sticking to the bottom, so `set_offset` can move off it.
    unstick: bool,
}

impl ScrollState {
    /// After this frame's layout: is there something new below what you're
    /// reading? `newest`: the newest message (id, ts, ours); `at_bottom`: the
    /// list is scrolled all the way down (or about to be).
    fn track_new(&mut self, newest: Option<(&str, i64, bool)>, at_bottom: bool) -> bool {
        if at_bottom {
            self.new_below = false;
        } else if let (Some((_, ts, mine)), Some((_, seen))) = (newest, &self.newest)
            && ts > *seen
            && !mine
        {
            self.new_below = true;
        }
        self.newest = newest.map(|(id, ts, _)| (id.to_string(), ts));
        self.new_below
    }

    /// Screenshot aid: scrolled to the top (with "↓ New messages" showing if
    /// `pill`), once the list has settled at the bottom. False until then.
    pub fn debug_scroll_up(&mut self, pill: bool) -> bool {
        if self.want_bottom > 0 || self.content_h <= 0.0 {
            return false;
        }
        self.set_offset = Some(0.0);
        self.unstick = true;
        self.new_below |= pill;
        true
    }
}

/// What you had typed in each channel, and the picture or GIF waiting in its
/// tray: switching channels keeps them with their channel and brings them
/// back when you return (in memory, this run only). `T` is the tray's.
pub struct Drafts<T> {
    /// The channel the composer is showing.
    pub channel: String,
    pub text: std::collections::HashMap<String, String>,
    pub staged: std::collections::HashMap<String, T>,
}

impl<T> Default for Drafts<T> {
    fn default() -> Self {
        Self { channel: String::new(), text: Default::default(), staged: Default::default() }
    }
}

impl<T> Drafts<T> {
    /// The composer moves to channel `to`: what's in it (and in the tray) stays
    /// with the channel it was written in, and `to`'s comes back. False if
    /// it's already there. The first time (no channel yet) keeps what's there.
    pub fn switch(&mut self, to: &str, composer: &mut String, staged: &mut Option<T>) -> bool {
        if self.channel == to {
            return false;
        }
        if !self.channel.is_empty() {
            let old = std::mem::take(composer);
            if !old.is_empty() {
                self.text.insert(self.channel.clone(), old);
            }
            if let Some(s) = staged.take() {
                self.staged.insert(self.channel.clone(), s);
            }
            *composer = self.text.remove(to).unwrap_or_default();
            *staged = self.staged.remove(to);
        }
        self.channel = to.to_string();
        true
    }
}

pub fn chat(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let channel = m.channel(&m.config.channel).cloned();
    // Drafts are per channel, the tray's picture or GIF too (an edit or a
    // reply in progress is dropped: they never follow you to another channel).
    if app.drafts.channel != m.config.channel {
        cancel_edit(app);
        app.replying = None;
        app.drafts.switch(&m.config.channel, &mut app.composer, &mut app.staged);
        app.focus_composer = true;
    }
    restore_refused(app, m, acts);
    theme::window_bevel(ui, p);
    // Someone's screen, while we watch it (unless it's popped out).
    if super::screen::in_pane(app, m) {
        super::screen::pane(app, ui, m, p, acts);
        return;
    }
    // A shared view (Draw, Space) instead of the chat; voice keeps going.
    if let Some(v) = m.view {
        let (title, icon) = super::view_title(v);
        egui::Panel::top("chat-title").exact_size(theme::TITLE_H).resizable(false).show_separator_line(false).frame(egui::Frame::new()).show(ui, |ui| {
            let rect = ui.max_rect();
            let b = Rect::from_min_size(egui::pos2(rect.right() - 24.0, rect.top() + 2.0), vec2(22.0, 18.0));
            let pb = b.translate(vec2(-24.0, 0.0));
            // The title and its note get what the buttons leave.
            let text_right = pb.left() - 6.0;
            let end = theme::paint_title_upto(ui.painter(), rect, p, Some(icon), &format!("{title} - Yap"), text_right);
            title_note(ui, end, rect.center().y, "voice keeps going", text_right, p);
            if theme::caption_button(ui, b, "x", "Back to the chat", false, p).clicked() {
                acts.push(Act::Cmd(Cmd::SetView(None)));
            }
            if theme::caption_button(ui, pb, "users", "People", false, p).clicked() {
                app.modal = Some(Modal::People { open: None, ask: None });
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 2, right: 2, top: 4, bottom: 2 })).show(ui, |ui| super::view_pane(app, ui, m, p, acts, v));
        return;
    }

    // Title bar: "<channel> - Yap" and the caption buttons (people, ping,
    // pinned, pictures, search, lock), laid out first so the title and its
    // note shorten with "…" instead of running under them.
    egui::Panel::top("chat-title").exact_size(theme::TITLE_H).resizable(false).show_separator_line(false).frame(egui::Frame::new()).show(ui, |ui| {
        let rect = ui.max_rect();
        let title = match &channel {
            Some(c) => format!("{} - Yap", c.name),
            None => "Yap".into(),
        };
        let personal = channel.as_ref().is_some_and(|c| c.is_personal());
        let buttons = !never_connected(m);
        let b = Rect::from_min_size(egui::pos2(rect.right() - 24.0, rect.top() + 2.0), vec2(22.0, 18.0));
        let pb = b.translate(vec2(-24.0, 0.0));
        let pinb = pb.translate(vec2(-26.0, 0.0));
        let shb = pinb.translate(vec2(-24.0, 0.0));
        let sb = shb.translate(vec2(-24.0, 0.0));
        let lock = (personal && m.vault_key.is_some()).then(|| sb.translate(vec2(-26.0, 0.0)));
        let text_right = if buttons { lock.unwrap_or(sb).left() - 6.0 } else { rect.right() - 4.0 };
        let end = theme::paint_title_upto(ui.painter(), rect, p, Some(if personal { "lock" } else { "chat" }), &title, text_right);
        if personal {
            // A quieter note after the title, like the web's header note.
            title_note(ui, end, rect.center().y, content::personal_note(m), text_right, p);
        }
        if !buttons {
            return; // nothing for them to do yet
        }
        if theme::caption_button(ui, b, "users", "People", matches!(app.modal, Some(Modal::People { .. })), p).clicked() {
            app.modal = Some(Modal::People { open: None, ask: None });
        }
        // People waiting (only the owner and mods let them in).
        let waiting = if m.am_moderator() { m.join_requests.len() } else { 0 };
        if waiting > 0 {
            // Inside the window's edge (the bevel would cut it off).
            let badge = Rect::from_center_size(b.right_top() + vec2(-5.0, 5.0), vec2(13.0, 13.0)).round();
            ui.painter().rect_filled(badge, 0.0, theme::BADGE);
            theme::outline(ui.painter(), badge.expand(1.0), Color32::BLACK);
            ui.painter().text(badge.center(), Align2::CENTER_CENTER, waiting.to_string(), theme::bold_font(10.0), Color32::WHITE);
        }
        if theme::caption_button(ui, pb, "megaphone", "Ping everyone (chime + phone push)", false, p).clicked() {
            app.modal = Some(Modal::Ping { note: String::new() });
        }
        // Pinned (with how many) and Search.
        let pins_open = matches!(app.side, Some(super::side::Side::Pins));
        if theme::caption_button(ui, pinb, "pin", "Pinned messages", pins_open, p).clicked() {
            super::side::toggle(app, super::side::Side::Pins, m, acts);
        }
        if let Some(n) = m.pins.get(&m.config.channel).map(|l| l.len()).filter(|n| *n > 0) {
            let badge = Rect::from_center_size(pinb.right_top() + vec2(-2.0, 2.0), vec2(13.0, 13.0)).round();
            ui.painter().rect_filled(badge, 0.0, p.select);
            theme::outline(ui.painter(), badge.expand(1.0), Color32::BLACK);
            ui.painter().text(badge.center(), Align2::CENTER_CENTER, n.to_string(), theme::bold_font(10.0), p.on_select);
        }
        let shelf_open = matches!(app.side, Some(super::side::Side::Shelf));
        if theme::caption_button(ui, shb, "images", "Pictures and GIFs in this channel", shelf_open, p).clicked() {
            super::side::toggle(app, super::side::Side::Shelf, m, acts);
        }
        let search_open = matches!(app.side, Some(super::side::Side::Search { .. }));
        if theme::caption_button(ui, sb, "search", &format!("Search messages ({})", super::SEARCH_KEYS), search_open, p).clicked() {
            super::side::toggle(app, super::side::Side::search(), m, acts);
        }
        if let Some(lb) = lock
            && theme::caption_button(ui, lb, "lock", "Lock it on this device", false, p).clicked()
        {
            acts.push(Act::Cmd(Cmd::Vault(crate::core::VaultOp::Lock)));
        }
    });

    if never_connected(m) {
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 3, bottom: 0 })).show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            theme::well(ui, rect, p, |ui| not_connected(app, ui, m, p, acts));
        });
        return;
    }

    // Information bars: connection lost, choose a password, people in the waiting room…
    if super::sections::has_bars(app, m) {
        egui::Panel::top("chat-bars").resizable(false).show_separator_line(false).frame(egui::Frame::new()).show(ui, |ui| super::sections::bars(app, ui, m, acts));
    }

    // Composer + typing line.
    egui::Panel::bottom("composer")
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 3, bottom: 0 }))
        .show(ui, |ui| composer(app, ui, m, p, acts, &composer_hint(m, channel.as_ref())));

    egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 3, bottom: 0 })).show(ui, |ui| {
        let rect = ui.available_rect_before_wrap();
        theme::well(ui, rect, p, |ui| message_list(app, ui, m, p, acts));
    });
}

pub fn message_list(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let channel_id = m.config.channel.clone();
    let channel = m.channel(&channel_id).cloned();
    let h = m.history.get(&channel_id);
    // A jump the core found (maybe after loading older pages): scroll to it, flash it.
    if let Some((ch, id)) = &m.jump_to {
        if *ch == channel_id {
            app.jump = Some(id.clone());
        }
        acts.push(Act::ClearJump);
    }
    let jumping = app.jump.is_some();
    let st = &mut app.scroll;
    if st.channel != channel_id {
        *st = ScrollState { channel: channel_id.clone(), ..Default::default() };
        st.want_bottom = 4;
    }
    match m.scroll {
        Some(ScrollHint::Bottom) => st.want_bottom = 4,
        Some(ScrollHint::Anchor) if st.anchor.is_none() && st.set_offset.is_none() => st.anchor = Some((st.content_h, st.offset)),
        _ => {}
    }
    if m.scroll.is_some() {
        acts.push(Act::ClearScroll);
    }
    if jumping {
        st.want_bottom = 0;
        st.set_offset = None;
        st.anchor = None;
    }
    // Since you left: the first unread message (the server's answer, or the
    // first loaded one when this page reaches back past what you'd read).
    let unread: Option<(String, i64)> = m.read_marks.get(&channel_id).copied().and_then(|mark| {
        let h = h?;
        let counted = m.unread_counts.as_ref().and_then(|c| c.get(&channel_id)).and_then(|u| u.first.as_ref()).filter(|f| f.ts > mark).map(|f| f.id.clone());
        let reaches = !h.has_more || h.messages.first().is_some_and(|x| x.ts <= mark);
        let loaded = h.messages.iter().find(|x| x.ts > mark && x.uid != m.config.uid).map(|x| x.id.clone());
        counted.or(if reaches { loaded } else { None }).map(|id| (id, mark))
    });
    let mut divider: Option<Rect> = None;
    let mut area = egui::ScrollArea::vertical().id_salt(("messages", &channel_id)).auto_shrink(false).stick_to_bottom(!std::mem::take(&mut st.unstick));
    if let Some(o) = st.set_offset.take() {
        area = area.vertical_scroll_offset(o);
    }
    let out = theme::scroll(ui, area, p, |ui| {
        ui.set_width(ui.available_width());
        let Some(h) = h.filter(|h| !(h.loading && !h.older)) else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(theme::text(if m.status == Status::Online { "Loading…" } else { "" }, p.gray));
            });
            return;
        };
        if h.has_more {
            ui.add_sized(vec2(ui.available_width(), 30.0), egui::Label::new(theme::text(if h.loading { "Loading older messages…" } else { "" }, p.gray)));
        } else if let Some(c) = &channel {
            super::sections::channel_start(app, ui, m, c, p, acts);
        }
        let pending: Vec<&Message> = m.pending.iter().filter(|(_, x)| x.channel_id == channel_id).map(|(_, x)| x).collect();
        // This session's own quiet lines ("Theo joined…") go where they happened.
        let mut notes: Vec<&crate::core::LocalNote> = m.local_notes.iter().filter(|n| n.channel_id == channel_id).collect();
        notes.sort_by_key(|n| n.ts);
        let mut notes = notes.into_iter().peekable();
        let mut prev: Option<&Message> = None;
        for msg in h.messages.iter().chain(pending) {
            let mut after_note = false;
            while !msg.pending
                && let Some(n) = notes.next_if(|n| n.ts < msg.ts)
            {
                super::sections::local_note(ui, n, p);
                after_note = true;
            }
            if unread.as_ref().is_some_and(|(id, _)| *id == msg.id) {
                let words = super::extras::since_words(unread.as_ref().map(|u| u.1).unwrap_or(0), app.h12);
                divider = Some(super::extras::unread_divider(ui, &words, p));
            }
            let new_day = prev.is_none_or(|pr| super::local_time(pr.ts).date_naive() != super::local_time(msg.ts).date_naive());
            if new_day {
                super::sections::day_divider(ui, msg.ts, p);
            }
            let grouped = !new_day && !after_note && prev.is_some_and(|pr| pr.uid == msg.uid && msg.ts.saturating_sub(pr.ts) < GROUP_MS);
            message_row(app, ui, m, msg, grouped, p, acts);
            prev = Some(msg);
        }
        for n in notes {
            super::sections::local_note(ui, n, p);
        }
        ui.add_space(8.0);
    });
    // "Jump to first unread" while the divider is above the screen (or not
    // loaded yet), centred in what a floating side panel leaves uncovered.
    let viewport = match app.side_float {
        Some(f) if f.intersects(out.inner_rect) => Rect::from_min_max(out.inner_rect.min, egui::pos2(f.left().max(out.inner_rect.left()), out.inner_rect.bottom())),
        _ => out.inner_rect,
    };
    if let Some((id, _)) = &unread
        && viewport.width() > 180.0
        && divider.is_none_or(|d| d.bottom() < out.inner_rect.top())
        && super::extras::jump_pill(ui, viewport, p)
    {
        if divider.is_some() {
            app.jump = Some(id.clone());
        } else {
            acts.push(Act::Cmd(Cmd::JumpTo { channel: channel_id.clone(), id: id.clone() }));
        }
    }
    // "↓ New messages": something new came in below while you're scrolled up.
    let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
    let at_bottom = out.state.offset.y >= max - 4.0 || app.scroll.want_bottom > 0 || app.scroll.set_offset.is_some();
    let newest = h.and_then(|h| h.messages.last()).map(|x| (x.id.as_str(), x.ts, x.uid == m.config.uid));
    if app.scroll.track_new(newest, at_bottom) && viewport.width() > 180.0 && super::extras::new_below_pill(ui, viewport, p) {
        app.scroll.want_bottom = 4;
        app.scroll.new_below = false;
        ui.ctx().request_repaint();
    }
    let st = &mut app.scroll;
    let new_h = out.content_size.y;
    if let Some((old_h, old_off)) = st.anchor.take()
        && new_h > old_h + 1.0 {
            // Older messages went in above: keep the same messages on screen.
            st.set_offset = Some(old_off + (new_h - old_h));
            ui.ctx().request_discard("keep scroll anchor");
        }
    st.content_h = new_h;
    st.offset = out.state.offset.y;
    if st.want_bottom > 0 && h.is_some_and(|h| !h.loading || h.older) {
        let max = (new_h - out.inner_rect.height()).max(0.0);
        if st.offset < max - 1.0 {
            st.set_offset = Some(max);
            ui.ctx().request_repaint();
        }
        st.want_bottom -= 1;
    }
    // Near the top: fetch the next older page.
    if let Some(h) = h
        && h.has_more
        && !h.loading
        && !h.messages.is_empty()
        && (st.offset < 300.0 || new_h <= out.inner_rect.height())
    {
        acts.push(Act::Cmd(Cmd::LoadOlder(channel_id)));
    }
}

/// One line of the chat log, AIM style: icon, **Name:** message, time on the right.
fn message_row(app: &mut App, ui: &mut egui::Ui, m: &Model, msg: &Message, grouped: bool, p: &Pal, acts: &mut Vec<Act>) {
    let user = m.users.get(&msg.uid);
    let name = user.map(|u| u.name.as_str()).unwrap_or("Unknown");
    let mine = msg.uid == m.config.uid;
    let mention = m.mentions_me(msg);
    let bg = ui.painter().add(egui::Shape::Noop);
    let top = ui.cursor().top();
    ui.add_space(if grouped { 0.0 } else { 4.0 });
    let full_w = ui.available_width();
    let time = fmt_time(msg.ts, app.h12);
    let time_w = ui.painter().layout_no_wrap(time.clone(), theme::font(theme::small()), p.gray).size().x;
    let body_w = (full_w - GUTTER - time_w - 16.0).max(80.0);
    let start = ui.cursor().min;
    let body_rect = Rect::from_min_size(egui::pos2(start.x + GUTTER, start.y + 1.0), vec2(body_w, 100_000.0));
    let name_color = if mine { p.me } else { p.them };
    let shown = content::shown(app, m, msg);
    ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.set_width(body_w);
        if msg.pending {
            ui.set_opacity(0.55);
        }
        ui.spacing_mut().item_spacing.y = 2.0;
        // A reply: "↪ Name  first line…" over it; click jumps to the original.
        if let Some(r) = &msg.reply {
            let (row, _) = ui.allocate_exact_size(vec2(body_w, 16.0), egui::Sense::hover());
            let who = if r.uid == m.config.uid { p.me } else { p.them };
            if content::paint_quote(ui, m, r, row.left_top(), body_w, false, who, p.gray, theme::small()).is_some_and(|q| q.clicked()) {
                content::jump_to_reply(app, m, r, acts);
            }
        }
        let prefix = (!grouped).then_some((name, name_color));
        if let Some(roll) = &msg.roll {
            if let Some((n, c)) = prefix {
                ui.label(egui::RichText::new(format!("{n}:")).font(theme::bold_font(theme::chat())).color(c));
            }
            super::extras::roll_view(ui, roll, p, false);
        } else if let Some(why) = shown.lock {
            if let Some((n, c)) = prefix {
                ui.label(egui::RichText::new(format!("{n}:")).font(theme::bold_font(theme::chat())).color(c));
            }
            if content::lock_line(ui, why, p.gray) {
                app.modal = Some(Modal::Vault(super::dialogs::VaultDialog::new(super::dialogs::VaultMode::Unlock)));
            }
        } else if !shown.text.is_empty() {
            let blocks = app.body(&shown.text);
            let suffix = msg.edited_at.filter(|t| *t > 0).map(|_| ("(edited)", p.gray));
            let r = markup::show(ui, &blocks, p, &app.images, &m.config.server, prefix, suffix, app.spoilers.contains(&msg.id));
            if let Some(url) = r.clicked_link {
                acts.push(Act::OpenUrl(url));
            }
            if r.clicked_spoiler {
                app.spoilers.insert(msg.id.clone());
            }
        } else if let Some((n, c)) = prefix {
            ui.label(egui::RichText::new(format!("{n}:")).font(theme::bold_font(theme::chat())).color(c));
        }
        if let Some(chars) = shown.long {
            content::more_toggle(app, ui, &msg.id, chars, shown.folded, p.link);
        }
        let pic = match (&shown.image, &shown.sealed_image, m.vault_key) {
            (Some(img), _, _) => Some(Pic::Plain(img)),
            (None, Some(img), Some(k)) => Some(Pic::Sealed(img, k)),
            _ => None,
        };
        if let Some(pic) = pic
            && let Some(url) = picture(app, ui, &pic, &m.config.server, 0.0, p)
        {
            acts.push(Act::OpenUrl(url));
        }
        if let Some(g) = &shown.gif {
            gif(app, ui, g, p);
        }
        super::extras::under_message(app, ui, m, msg, &shown.text, false, p, acts);
        if !msg.reactions.is_empty() {
            reactions(app, ui, m, msg, p, acts);
        }
    });
    ui.add_space(1.0);
    let rect = Rect::from_min_max(egui::pos2(ui.min_rect().left(), top), egui::pos2(ui.min_rect().right(), ui.cursor().top()));
    let hovered = ui.rect_contains_pointer(rect) && app.react_picker.is_none();
    let flash = content::flash(app, ui, &msg.id, rect);
    // The message you're editing: a select tint and Windows' dotted focus box.
    let editing = content::editing_mark(app, ui, &msg.id, rect);
    if mention || flash > 0.0 || editing {
        let mut shapes = Vec::new();
        if mention {
            shapes.push(egui::Shape::rect_filled(rect, 0.0, p.mention_bg));
        }
        if editing {
            shapes.push(egui::Shape::rect_filled(rect, 0.0, p.select.gamma_multiply(0.16)));
            let r = rect.shrink(0.5);
            let ring = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            shapes.extend(egui::Shape::dashed_line(&ring, egui::Stroke::new(1.0, p.text), 1.0, 1.0));
        }
        if flash > 0.0 {
            shapes.push(egui::Shape::rect_filled(rect, 0.0, p.select.gamma_multiply(0.3 * flash)));
        }
        ui.painter().set(bg, egui::Shape::Vec(shapes));
    }
    // Buddy icon (first line of a group), and the time on the right.
    // A reply's quote line sits above: the icon and time line up with the name.
    let quote_h = if msg.reply.is_some() { 18.0 } else { 0.0 };
    if !grouped {
        avatar(ui, &app.images, &m.config.server, egui::pos2(rect.left() + 14.0, top + 4.0 + 10.0 + quote_h), 16.0, face(user), false, p);
    }
    let time_y = top + quote_h + if grouped { 1.0 } else { 5.0 };
    if !grouped || hovered {
        let t = ui.painter().text(egui::pos2(rect.right() - 8.0, time_y), Align2::RIGHT_TOP, time, theme::font(theme::small()), p.gray);
        theme::tip(ui.interact(t, ui.id().with(("time", &msg.id)), egui::Sense::hover()), fmt_stamp(msg.ts, app.h12));
    }
    // Pinned: a small pin where the time column starts.
    if msg.pinned_at.is_some() {
        let pr = Rect::from_center_size(egui::pos2(rect.right() - 8.0 - time_w - 10.0, time_y + 6.0), vec2(11.0, 11.0));
        icons::paint(ui.painter(), pr, "pin", p.danger);
        theme::tip(ui.interact(pr.expand(2.0), ui.id().with(("pinned", &msg.id)), egui::Sense::hover()), "Pinned");
    }
    // Hover toolbar (content::message_actions picks the buttons and does what they say).
    if hovered && !msg.pending {
        let actions = content::message_actions(m, msg, &shown);
        let n = actions.len() as f32;
        let bar = Rect::from_min_size(egui::pos2(rect.right() - 8.0 - time_w - 8.0 - (21.0 * n + 4.0), (top - 10.0).max(ui.clip_rect().top())), vec2(21.0 * n + 4.0, 24.0)).round();
        theme::panel(ui.painter(), bar, p, p.face, Bevel::Raised);
        let pinned = msg.pinned_at.is_some();
        for (i, action) in actions.into_iter().enumerate() {
            let b = Rect::from_min_size(bar.min + vec2(2.0 + 21.0 * i as f32, 3.0), vec2(20.0, 18.0));
            let color = if action == content::MsgAction::Delete { p.danger } else { p.text };
            let on = action == content::MsgAction::Pin && pinned;
            if ui.place(b, |ui: &mut egui::Ui| icons::tool(ui, action.icon(), vec2(20.0, 18.0), color, action.tip(pinned), on)).clicked() {
                content::run_action(app, ui, m, msg, action, b.left_bottom() + vec2(-250.0, 6.0), acts);
            }
        }
    }
}

/// Reaction buttons under a message: click to toggle yours (yours stay pushed in).
fn reactions(app: &mut App, ui: &mut egui::Ui, m: &Model, msg: &Message, p: &Pal, acts: &mut Vec<Act>) {
    ui.add_space(1.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for r in &msg.reactions {
            if let Some(id) = super::extras::doodle_token(&r.emoji) {
                super::extras::doodle_reaction(app, ui, m, msg, &r.emoji, id, &r.uids, p);
                continue;
            }
            let mine = r.uids.contains(&m.config.uid);
            let count = r.uids.len().to_string();
            let count_g = ui.painter().layout_no_wrap(count, theme::bold_font(theme::small()), p.text);
            let emoji = custom_token(&r.emoji);
            let emoji_g = if emoji.is_none() { Some(ui.painter().layout_no_wrap(r.emoji.clone(), theme::font(13.0), p.text)) } else { None };
            let ew = emoji_g.as_ref().map(|g| g.size().x).unwrap_or(14.0);
            let size = vec2((ew + count_g.size().x + 14.0).round(), 20.0);
            let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
            let pressed = mine || resp.is_pointer_button_down_on();
            ui.painter().rect_filled(rect, 0.0, p.face);
            if mine {
                theme::dither(ui, rect, p.hilight);
            }
            theme::bevel(ui.painter(), rect, p, if pressed { Bevel::Pressed } else { Bevel::Raised });
            let off = if pressed { vec2(1.0, 1.0) } else { Vec2::ZERO };
            let ec = egui::pos2(rect.left() + 5.0 + ew / 2.0, rect.center().y) + off;
            match (&emoji, emoji_g) {
                (Some(id), _) => {
                    let url = markup::emoji_url(&m.config.server, id);
                    if let Some((tex, sz)) = app.images.get(&url, resp.hovered()) {
                        let s = 14.0 / sz.x.max(sz.y);
                        ui.painter().image(tex, Rect::from_center_size(ec, sz * s), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                }
                (None, Some(g)) => ui.painter().galley(ec - g.size() / 2.0, g, p.text),
                _ => {}
            }
            ui.painter().galley(egui::pos2(ec.x + ew / 2.0 + 4.0, rect.center().y - count_g.size().y / 2.0 + off.y), count_g, p.text);
            let names: Vec<String> = r.uids.iter().map(|u| m.user_name(u)).collect();
            let label = custom_name(&r.emoji).map(|n| format!(":{n}:")).unwrap_or_else(|| r.emoji.clone());
            if theme::tip(resp, format!("{label} — {}", names.join(", "))).clicked() {
                acts.push(Act::Cmd(Cmd::React { id: msg.id.clone(), emoji: r.emoji.clone(), on: !mine }));
            }
        }
    });
}

/// `<:name:id>` → id.
pub fn custom_token(s: &str) -> Option<String> {
    let inner = s.strip_prefix("<:")?.strip_suffix('>')?;
    let (_, id) = inner.split_once(':')?;
    (id.len() == 32).then(|| id.to_string())
}

pub fn custom_name(s: &str) -> Option<String> {
    let inner = s.strip_prefix("<:")?.strip_suffix('>')?;
    inner.split_once(':').map(|(n, _)| n.to_string())
}

pub const QUICK_REACTIONS: [&str; 16] = ["👍", "❤️", "😂", "🎉", "😮", "😢", "🔥", "👀", "🙏", "💯", "✅", "😎", "😍", "👏", "🙌", "😅"];

/// A small floating window with a title bar (pickers).
pub fn picker_window<R>(ui: &mut egui::Ui, title: &str, width: f32, p: &Pal, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    super::sidebar::popup_frame(p)
        .show(ui, |ui| {
            ui.set_width(width);
            theme::title_bar(ui, p, None, title);
            let r = egui::Frame::new().inner_margin(egui::Margin { left: 3, right: 3, top: 5, bottom: 3 }).show(ui, add).inner;
            theme::menu_bevel(ui, p);
            r
        })
        .inner
}

/// An emoji tile: navy under the pointer.
fn tile(ui: &mut egui::Ui, size: f32, p: &Pal) -> (Rect, egui::Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, p.select);
    }
    (rect, resp)
}

/// Pick an emoji to react with: quick picks, custom emojis, or search by shortcode.
pub fn react_picker(app: &mut App, ctx: &egui::Context, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let Some((msg_id, pos, mut query)) = app.react_picker.clone() else { return };
    let mut chosen: Option<String> = None;
    let mut doodle = false;
    let area = egui::Area::new(egui::Id::new("react-picker")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        picker_window(ui, "Add a Reaction", 272.0, p, |ui| {
            let r = theme::field(ui, egui::TextEdit::singleline(&mut query).hint_text("Search emoji").desired_width(f32::INFINITY), p);
            r.request_focus();
            ui.add_space(3.0);
            let q = query.trim().trim_matches(':').to_lowercase();
            let mut chars: Vec<String> = Vec::new();
            if q.is_empty() {
                chars.extend(QUICK_REACTIONS.iter().map(|s| s.to_string()));
            } else if let Some(codes) = app.shortcodes.get() {
                let mut hits: Vec<(&String, &String)> = codes.iter().filter(|(k, _)| k.contains(&q)).collect();
                hits.sort_by_key(|(k, _)| (!k.starts_with(&q), k.len()));
                for (_, c) in hits {
                    if !chars.contains(c) {
                        chars.push(c.clone());
                    }
                    if chars.len() >= 40 {
                        break;
                    }
                }
            }
            let mut customs: Vec<&crate::protocol::Emoji> = m.emojis.values().filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q)).collect();
            customs.sort_by_key(|e| e.name.to_lowercase());
            // As tall as what's in it (no big empty well), scrolling past 200.
            let tile_px = 31.0;
            let per_row = ((ui.available_width() - 8.0) / tile_px).floor().max(1.0);
            let rows = ((chars.len() + customs.len()) as f32 / per_row).ceil();
            let height = if rows == 0.0 { 36.0 } else { (rows * tile_px + 8.0).clamp(40.0, 200.0) };
            let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), height));
            ui.allocate_rect(rect, egui::Sense::hover());
            theme::well(ui, rect, p, |ui| {
                theme::scroll(ui, egui::ScrollArea::vertical().auto_shrink(false), p, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        for c in &chars {
                            let (rect, resp) = tile(ui, tile_px, p);
                            ui.painter().text(rect.center(), Align2::CENTER_CENTER, c, theme::font(19.0), p.text);
                            if resp.clicked() {
                                chosen = Some(c.clone());
                            }
                        }
                        for e in customs {
                            let (rect, resp) = tile(ui, tile_px, p);
                            if let Some((tex, sz)) = app.images.get(&markup::emoji_url(&m.config.server, &e.id), resp.hovered()) {
                                let s = 22.0 / sz.x.max(sz.y);
                                ui.painter().image(tex, Rect::from_center_size(rect.center(), sz * s), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            if theme::tip(resp, format!(":{}:", e.name)).clicked() {
                                chosen = Some(format!("<:{}:{}>", e.name, e.id));
                            }
                        }
                    });
                    if !q.is_empty() && chars.is_empty() && m.emojis.values().all(|e| !e.name.to_lowercase().contains(&q)) {
                        ui.label(theme::text("No match.", p.gray));
                    }
                });
            });
            ui.add_space(3.0);
            if theme::tip(theme::button_sized(ui, "Draw one…", theme::Btn::Normal, true, vec2(100.0, 23.0), p), "React with a tiny drawing (a doodle)").clicked() {
                doodle = true;
            }
        });
    });
    let outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| !area.response.rect.contains(pos));
    if doodle {
        super::extras::open_doodle(app, &msg_id);
    } else if let Some(e) = chosen {
        acts.push(Act::Cmd(Cmd::React { id: msg_id, emoji: e, on: true }));
        app.react_picker = None;
    } else if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.react_picker = None;
    } else if let Some(state) = app.react_picker.as_mut() {
        state.2 = query;
    }
}

/// A picture's size as a message says it (layout only): positive, and not
/// so big that scaling it to fit comes out as 0 × ∞.
pub fn sane_size(w: f32, h: f32) -> bool {
    (0.5..=1e5).contains(&w) && (0.5..=1e5).contains(&h)
}

pub fn gif(app: &mut App, ui: &mut egui::Ui, g: &Gif, p: &Pal) {
    let (w, h) = if sane_size(g.width, g.height) { (g.width, g.height) } else { (300.0, 200.0) };
    let s = (300.0 / w).min(300.0 / h);
    let size = vec2(w * s, h * s).round();
    ui.add_space(3.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    let hovered = resp.hovered();
    // Calm by default: the still frame; it plays while you hover it.
    let still = g.still.as_ref().or(g.webp.as_ref()).or(g.gif.as_ref());
    let moving = g.webp.as_ref().or(g.gif.as_ref());
    let url = if hovered { moving.or(still) } else { still };
    let mut drawn = false;
    if let Some(u) = url
        && let Some((tex, _)) = app.images.get(u, hovered)
    {
        ui.painter().image(tex, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        drawn = true;
    }
    if !drawn {
        // Hovering before the animation loads: keep showing the still.
        if let Some(u) = still
            && let Some((tex, _)) = app.images.get(u, false)
        {
            ui.painter().image(tex, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            ui.painter().rect_filled(rect, 0.0, p.code_bg);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, if g.title.is_empty() { "GIF" } else { &g.title }, theme::font(theme::ui()), p.gray);
        }
    }
    theme::outline(ui.painter(), rect.expand(1.0), p.shadow);
    if !hovered {
        let badge = Rect::from_min_size(rect.left_bottom() + vec2(5.0, -19.0), vec2(28.0, 14.0)).round();
        ui.painter().rect_filled(badge, 0.0, Color32::from_black_alpha(170));
        ui.painter().text(badge.center(), Align2::CENTER_CENTER, "GIF", theme::bold_font(10.0), Color32::WHITE);
    }
    theme::tip(resp, g.title.clone());
}

/// What the tray says: ("GIF" / "Picture", the line under it). The size
/// comes first, so it's what stays when a narrow tray shortens the line.
pub fn staged_words(app: &App) -> Option<(&'static str, String)> {
    Some(match app.staged.as_ref()? {
        super::Staged::Gif { gif, .. } => ("GIF", format!("{} · Enter to send", if gif.title.is_empty() { "from KLIPY" } else { &gif.title })),
        super::Staged::Picture { slot, full, .. } => match slot.lock().as_ref() {
            Some(Err(e)) => ("Picture", format!("Can't send this one: {e}")),
            Some(Ok(pic)) => ("Picture", format!("{} · Add a message if you like · Enter to send", pic.describe())),
            None => ("Picture", format!("{} · Add a message if you like · Enter to send", if *full { "Getting it ready…" } else { "Shrinking…" })),
        },
    })
}

/// The tray's line under the title, from `x` to `right` ("…" if it's too long).
pub fn staged_line(ui: &egui::Ui, text: String, x: f32, y: f32, right: f32, font: egui::FontId, color: Color32) {
    let g = theme::elide(ui.painter(), text, font, color, right - x);
    ui.painter().galley(egui::pos2(x, y - g.size().y / 2.0), g, color);
}

/// The tray's "Full res" check box (pictures only), right edge at `right`,
/// centred on `y`. Returns where it starts (the tray's text stops there).
pub fn full_res_toggle(app: &mut App, ui: &mut egui::Ui, right: f32, y: f32, p: &Pal) -> Option<f32> {
    let Some(super::Staged::Picture { full, .. }) = app.staged.as_ref() else { return None };
    let mut on = *full;
    let w = 19.0 + ui.painter().layout_no_wrap("Full res".into(), theme::font(theme::ui()), p.text).size().x + 2.0;
    let r = Rect::from_min_size(egui::pos2(right - w, y - 12.0), vec2(w, 24.0));
    let resp = ui.place(r, |ui: &mut egui::Ui| theme::checkbox(ui, &mut on, "Full res", p));
    if theme::tip(resp, "Send the original picture instead of a smaller copy (uses more space)").changed() {
        app.set_full_res(on);
    }
    Some(r.left())
}

/// Draw `tex` fitted into `rect` (keeps its shape).
pub fn paint_fitted(ui: &egui::Ui, rect: Rect, tex: Option<(egui::TextureId, Vec2)>, radius: f32, p: &Pal) -> Rect {
    match tex {
        Some((t, size)) => {
            let s = (rect.width() / size.x).min(rect.height() / size.y);
            let r = Rect::from_center_size(rect.center(), size * s).round();
            if radius > 0.0 {
                theme::image_rounded(ui.painter(), r, radius, t);
            } else {
                ui.painter().image(t, r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            r
        }
        None => {
            ui.painter().rect_filled(rect, radius, p.code_bg);
            rect
        }
    }
}

/// 98: the tray is a sunken well with the thumbnail, what it is, and ×.
fn staged_tray(app: &mut App, ui: &mut egui::Ui, p: &Pal) {
    let Some((title, sub)) = staged_words(app) else { return };
    let thumb = app.staged_thumb(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), egui::Sense::hover());
    theme::panel(ui.painter(), rect, p, p.window, Bevel::Sunken);
    let box_ = Rect::from_min_size(rect.min + vec2(6.0, 5.0), vec2(48.0, 48.0));
    let r = paint_fitted(ui, box_, thumb, 0.0, p);
    theme::outline(ui.painter(), r.expand(1.0), p.shadow);
    let x = box_.right() + 10.0;
    ui.painter().text(egui::pos2(x, rect.center().y - 8.0), Align2::LEFT_CENTER, title, theme::bold_font(theme::ui()), p.text);
    let text_right = full_res_toggle(app, ui, rect.right() - 40.0, rect.center().y, p).map_or(rect.right() - 34.0, |l| l - 8.0);
    staged_line(ui, sub, x, rect.center().y + 9.0, text_right, theme::font(theme::small()), p.gray);
    let b = Rect::from_min_size(egui::pos2(rect.right() - 30.0, rect.center().y - 11.0), vec2(24.0, 22.0));
    if ui.place(b, |ui: &mut egui::Ui| icons::tool(ui, "x", vec2(24.0, 22.0), p.text, "Don't send this (Esc)", false)).clicked() {
        app.staged = None;
    }
}

/// Longest side of a picture in the chat.
pub const PICTURE_MAX: f32 = 360.0;

/// A picture to draw: a plain one, or one sealed in an encrypted note.
pub enum Pic<'a> {
    Plain(&'a crate::protocol::ImageRef),
    Sealed(&'a crate::vault::NoteImage, crate::vault::Key),
}

impl Pic<'_> {
    pub fn size(&self) -> (f32, f32) {
        let (w, h) = match self {
            Pic::Plain(i) => (i.width, i.height),
            Pic::Sealed(i, _) => (i.width, i.height),
        };
        if sane_size(w, h) { (w, h) } else { (PICTURE_MAX, PICTURE_MAX * 0.75) }
    }
}

/// A picture in a message: ≤ 360 px, plays (GIFs) under the pointer, click
/// opens it full size in the browser (returns that URL; not for sealed ones,
/// which only this app can open). `radius`: rounded corners (0: square).
pub fn picture(app: &mut App, ui: &mut egui::Ui, pic: &Pic<'_>, server: &str, radius: f32, p: &Pal) -> Option<String> {
    let (w, h) = pic.size();
    let s = (PICTURE_MAX / w).min(PICTURE_MAX / h).min(1.0).min(ui.available_width() / w);
    let size = vec2(w * s, h * s).round();
    ui.add_space(3.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = resp.hovered();
    let (url, tex, sealed) = match pic {
        Pic::Plain(img) => {
            let url = format!("{server}/media/{}", img.id);
            let tex = app.images.get(&url, hovered).or_else(|| app.images.get(&url, false));
            (url, tex, false)
        }
        Pic::Sealed(img, key) => {
            let url = format!("{server}/media/{}", img.id);
            let tex = app.images.get_sealed(&url, *key);
            (url, tex, true)
        }
    };
    match tex {
        Some((t, _)) if radius > 0.0 => theme::image_rounded(ui.painter(), rect, radius, t),
        Some((t, _)) => {
            ui.painter().image(t, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(rect, radius, p.code_bg);
            if sealed {
                let failed = app.images.failed(&url, true);
                icons::paint(ui.painter(), Rect::from_center_size(rect.center() - vec2(0.0, if failed { 9.0 } else { 0.0 }), vec2(18.0, 18.0)), "lock", p.gray);
                if failed {
                    ui.painter().text(rect.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, "Can't open", theme::font(theme::small()), p.gray);
                }
            } else {
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Picture", theme::font(theme::ui()), p.gray);
            }
        }
    }
    if radius == 0.0 {
        theme::outline(ui.painter(), rect.expand(1.0), p.shadow);
    }
    if sealed {
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if theme::tip(resp, "Encrypted picture: only your devices can open it. Click to see it full size.").clicked()
            && let Pic::Sealed(img, key) = pic
        {
            app.modal = Some(Modal::Viewer(super::dialogs::Viewer { url, key: Some(*key), title: "Picture".into(), size: (img.width, img.height), pixel: false, remove: None }));
        }
        return None;
    }
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    theme::tip(resp, "Open full size").clicked().then_some(url)
}

fn composer(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, hint: &str) {
    let id = egui::Id::new("composer");
    let mut send = composer_keys(app, ui, m, acts, id);
    // The panel sizes itself from last frame's contents: when the edit bar or
    // the tray comes or goes, lay the frame out again right away.
    let shape = (app.editing.is_some(), app.staged.is_some(), app.replying.is_some());
    let key = egui::Id::new("composer-shape");
    if ui.ctx().data(|d| d.get_temp::<(bool, bool, bool)>(key)) != Some(shape) {
        ui.ctx().data_mut(|d| d.insert_temp(key, shape));
        ui.ctx().request_discard("composer changed shape");
    }
    if app.editing.is_some() {
        let light = theme::light();
        theme::info_bar(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(13.0, 13.0), egui::Sense::hover());
                icons::paint(ui.painter(), r, "pencil", theme::INFO_TEXT);
                ui.label(egui::RichText::new("Editing your message — Enter saves, Esc cancels.").font(theme::font(theme::ui())).color(theme::INFO_TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::button_sized(ui, "Cancel", theme::Btn::Normal, true, vec2(60.0, 21.0), &light).clicked() {
                        cancel_edit(app);
                    }
                });
            });
        });
        ui.add_space(3.0);
    }
    if let Some(r) = app.replying.clone() {
        let light = theme::light();
        theme::info_bar(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (ir, _) = ui.allocate_exact_size(vec2(13.0, 13.0), egui::Sense::hover());
                icons::paint(ui.painter(), ir, "reply", theme::INFO_TEXT);
                ui.label(egui::RichText::new("Replying to").font(theme::font(theme::ui())).color(theme::INFO_TEXT));
                ui.label(egui::RichText::new(content::replying_to(m, &r)).font(theme::bold_font(theme::ui())).color(theme::INFO_TEXT));
                // What you're answering, in a few words (like the quote it will become).
                let hint = ui.painter().layout_no_wrap("Esc to cancel".into(), theme::font(theme::small()), theme::INFO_TEXT);
                let room = ui.available_width() - 60.0 - hint.size().x - 16.0;
                let snippet = content::replying_snippet(app, m, &r);
                if !snippet.is_empty() && room > 40.0 {
                    let mut job = egui::text::LayoutJob::single_section(snippet, egui::TextFormat { font_id: theme::font(theme::ui()), color: Color32::from_gray(0x55), ..Default::default() });
                    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
                    theme::job_label(ui, job);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::button_sized(ui, "Cancel", theme::Btn::Normal, true, vec2(60.0, 21.0), &light).clicked() {
                        app.replying = None;
                        app.focus_composer = true;
                    }
                    ui.label(egui::RichText::new("Esc to cancel").font(theme::font(theme::small())).color(Color32::from_gray(0x55)));
                });
            });
        });
        ui.add_space(3.0);
    }
    if app.staged.is_some() {
        staged_tray(app, ui, p);
        ui.add_space(3.0);
    }
    let locked = content::locked_here(m);
    // Toolbar: emoji + GIF + picture (flat until pointed at, like an AIM IM window).
    let mut emoji_btn = Rect::NOTHING;
    let mut gif_btn = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 1.0;
        let r = icons::tool(ui, "smile", vec2(26.0, 22.0), p.text, "Custom emojis", app.emoji_picker.is_some());
        emoji_btn = r.rect;
        if r.clicked() {
            app.emoji_picker = if app.emoji_picker.is_some() { None } else { Some(String::new()) };
        }
        if m.server_config.as_ref().is_some_and(|c| c.gifs.is_some()) {
            let open = app.gif_picker.open;
            let (r, resp) = ui.allocate_exact_size(vec2(34.0, 22.0), egui::Sense::click());
            let pressed = open || resp.is_pointer_button_down_on();
            if open {
                theme::dither(ui, r, p.hilight);
            }
            if pressed {
                theme::bevel(ui.painter(), r, p, Bevel::ThinIn);
            } else if resp.hovered() {
                theme::bevel(ui.painter(), r, p, Bevel::ThinOut);
            }
            ui.painter().text(r.center() + if pressed { vec2(1.0, 1.0) } else { Vec2::ZERO }, Align2::CENTER_CENTER, "GIF", theme::bold_font(theme::small()), p.text);
            if theme::tip(resp, "GIFs (KLIPY)").clicked() {
                app.emoji_picker = None;
                super::gifs::toggle(app, ui.ctx(), m);
            }
            gif_btn = Some(r);
        }
        let busy = m.sending_picture || app.file_pick.is_some();
        if icons::tool(ui, "image", vec2(26.0, 22.0), if busy { p.gray } else { p.text }, "Send a picture (or paste or drop one)", busy).clicked() && !busy {
            acts.push(Act::PickFile(super::PickFor::Picture));
        }
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let w = ui.available_width() - 68.0;
        let bg = ui.painter().add(egui::Shape::Noop);
        let out = ui.allocate_ui_with_layout(vec2(w, 200.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 2, right: 2, top: 2, bottom: 2 }).show(ui, |ui| {
                theme::scroll(ui, egui::ScrollArea::vertical().id_salt("composer-scroll").max_height(196.0), p, |ui| {
                    egui::TextEdit::multiline(&mut app.composer)
                        .id(id)
                        .interactive(!locked)
                        .desired_rows(2)
                        .desired_width(w - 4.0)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::symmetric(4, 3))
                        .font(theme::font(theme::chat()))
                        .text_color(p.text)
                        // One line, ending in "…", never wrapped.
                        .hint_text(egui::RichText::new(super::sections::fit_text(ui.painter(), hint, theme::font(theme::chat()), w - 16.0)).font(theme::font(theme::chat())).color(p.gray))
                        .show(ui)
                })
            })
        });
        let out = out.inner;
        let well = out.response.rect;
        ui.painter().set(bg, egui::Shape::rect_filled(well, 0.0, if locked { p.face } else { p.window }));
        theme::bevel(ui.painter(), well, p, Bevel::Sunken);
        let resp = out.inner.inner.response;
        if app.focus_composer && app.modal.is_none() {
            resp.request_focus();
            app.focus_composer = false;
        }
        if resp.changed() && !app.composer.trim().is_empty() {
            acts.push(Act::Cmd(Cmd::Typing));
        }
        // Send: the default button.
        let (r, sresp) = ui.allocate_exact_size(vec2(64.0, well.height()), if locked { egui::Sense::hover() } else { egui::Sense::click() });
        let pressed = sresp.is_pointer_button_down_on();
        theme::paint_button(ui, r, true, pressed, p);
        let off = if pressed { vec2(1.0, 1.0) } else { Vec2::ZERO };
        icons::paint(ui.painter(), Rect::from_center_size(r.center() + vec2(-17.0, 0.0) + off, vec2(12.0, 12.0)), "send", if locked { p.gray } else { p.text });
        let label = if app.editing.is_some() { "Save" } else { "Send" };
        ui.painter().text(r.center() + vec2(6.0, 0.0) + off, Align2::CENTER_CENTER, label, theme::bold_font(theme::ui()), if locked { p.gray } else { p.text });
        if theme::tip(sresp, if app.editing.is_some() { "Save the edit (Enter)" } else { "Send (Enter)" }).clicked() {
            send = true;
        }
    });
    if app.emoji_picker.is_some() {
        emoji_picker(app, ui, m, p, acts, emoji_btn);
    }
    if let Some(r) = gif_btn {
        super::gifs::show(app, ui.ctx(), m, p, acts, r);
    }
    if send {
        send_message(app, ui, m, acts, id);
    }
    // Status line under the chat: who's typing (or "Ready").
    ui.add_space(3.0);
    let words = super::sections::typing_words(&m.typers(&m.config.channel));
    let (text, color) = if words.is_empty() { ("Ready".to_string(), p.gray) } else { (words, p.text) };
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), egui::Sense::hover());
    theme::status_cell(ui.painter(), r, p);
    ui.painter().with_clip_rect(r.shrink(2.0)).text(egui::pos2(r.left() + 6.0, r.center().y), Align2::LEFT_CENTER, text, theme::font(theme::small()), color);
}

/// Editing one of our messages: which one, and the draft to give back after.
pub struct Editing {
    pub id: String,
    pub draft: String,
}

/// Messages the server refused (too long, the replied-to message is gone…):
/// their text goes back into their channel's message box, if that's empty,
/// so nothing typed is lost.
fn restore_refused(app: &mut App, m: &Model, acts: &mut Vec<Act>) {
    if m.composer_restore.is_empty() {
        return;
    }
    for (channel, text) in &m.composer_restore {
        let text = crate::core::plain_emojis(text);
        let slot = if *channel != app.drafts.channel {
            app.drafts.text.entry(channel.clone()).or_default()
        } else if let Some(e) = app.editing.as_mut() {
            &mut e.draft
        } else {
            app.focus_composer = true;
            &mut app.composer
        };
        if slot.trim().is_empty() {
            *slot = text;
        }
    }
    acts.push(Act::ClearRestored(m.composer_restore.len()));
}

/// Put one of our messages in the composer to edit it (an encrypted note:
/// what it says, if this device can open it).
pub fn start_edit(app: &mut App, m: &Model, msg: &Message) {
    let text = if crate::vault::is_note(&msg.body) {
        match content::note(app, m, &msg.body) {
            Ok(n) => n.text.clone(),
            Err(_) => return,
        }
    } else {
        msg.body.clone()
    };
    app.staged = None;
    app.replying = None;
    match app.editing.as_mut() {
        Some(e) => e.id = msg.id.clone(),
        None => app.editing = Some(Editing { id: msg.id.clone(), draft: std::mem::take(&mut app.composer) }),
    }
    app.composer = crate::core::plain_emojis(&text);
    app.emoji_picker = None;
    app.focus_composer = true;
    app.edit_scroll = Some(msg.id.clone());
    // (Don't let a pending "stay at the end" pull the list away from it.)
    app.scroll.want_bottom = 0;
}

/// Esc: forget the edit and give the draft back.
pub fn cancel_edit(app: &mut App) {
    if let Some(e) = app.editing.take() {
        app.composer = e.draft;
        app.focus_composer = true;
    }
}

/// Keys the composer handles before the text box sees them: Enter sends
/// (Shift+Enter is a newline), Esc cancels an edit, Up in an empty box edits
/// your last message, and Cmd/Ctrl+V with a picture on the clipboard sends it.
pub fn composer_keys(app: &mut App, ui: &egui::Ui, m: &Model, acts: &mut Vec<Act>, id: egui::Id) -> bool {
    let focused = ui.ctx().memory(|mem| mem.has_focus(id));
    let send = focused && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    if app.editing.is_some() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cancel_edit(app);
    }
    if app.staged.is_some() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.staged = None;
    }
    if app.replying.is_some() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.replying = None;
    }
    if focused
        && app.editing.is_none()
        && app.composer.is_empty()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp))
        && let Some(msg) = m.last_own_message(&m.config.channel).cloned()
    {
        start_edit(app, m, &msg);
    }
    // Pasting: egui only hands us text; a picture-only clipboard shows up as a
    // bare V key-up with Cmd/Ctrl held.
    let (now, text_pasted, v_up) = ui.input(|i| {
        let pasted = i.events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
        let v_up = i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::V, pressed: false, modifiers, .. } if modifiers.command));
        (i.time, pasted, v_up)
    });
    if text_pasted {
        app.last_text_paste = now;
    }
    if focused && v_up && now - app.last_text_paste > 1.0 && m.status == Status::Online
        && let Some(png) = clipboard_picture()
    {
        acts.push(Act::StagePicture(png));
    }
    send
}

/// The clipboard's picture as PNG bytes, if it holds one (and no text).
fn clipboard_picture() -> Option<Vec<u8>> {
    let mut cb = arboard::Clipboard::new().ok()?;
    if cb.get_text().is_ok_and(|t| !t.is_empty()) {
        return None;
    }
    let img = cb.get_image().ok()?;
    crate::picture::png_from_rgba(img.width as u32, img.height as u32, img.bytes.into_owned()).ok()
}

/// Send (or, while editing, save) what's in the composer — custom emojis and
/// :shortcodes: encoded.
pub fn send_message(app: &mut App, ui: &egui::Ui, m: &Model, acts: &mut Vec<Act>, id: egui::Id) {
    if content::locked_here(m) {
        acts.push(Act::Toast("Unlock your Personal space to write.".into()));
        return;
    }
    let text = app.composer.trim_end().to_string();
    // Encrypted notes may be longer (the server takes 240,000 characters of ciphertext).
    let sealed = m.vault.is_some() && m.channel(&m.config.channel).is_some_and(|c| c.is_personal());
    if if sealed { text.len() > 170_000 } else { text.chars().count() > MAX_BODY } {
        acts.push(Act::Toast(format!("That's {} characters; the limit is {}.", content::thousands(text.chars().count()), content::thousands(MAX_BODY))));
        return;
    }
    if let Some(e) = app.editing.take() {
        let mut body = markup::encode_emojis(&text, &m.emojis_by_name());
        if let Some(codes) = app.shortcodes.get() {
            body = encode_shortcodes(&body, codes, m);
        }
        let original = m.history.get(&m.config.channel).and_then(|h| h.messages.iter().find(|x| x.id == e.id));
        let keeps_media = original.is_some_and(|x| {
            if crate::vault::is_note(&x.body) {
                content::note(app, m, &x.body).is_ok_and(|n| n.gif.is_some() || n.image.is_some())
            } else {
                x.gif.is_some() || x.image.is_some()
            }
        });
        if body.trim().is_empty() && !keeps_media {
            acts.push(Act::Toast("An edit can't be empty — delete the message instead.".into()));
            app.editing = Some(e);
        } else if m.status != Status::Online {
            acts.push(Act::Toast("Not connected yet — your edit is still here.".into()));
            app.editing = Some(e);
        } else {
            acts.push(Act::Cmd(Cmd::Edit { id: e.id, body }));
            app.composer = e.draft;
        }
        ui.ctx().memory_mut(|mem| mem.request_focus(id));
        return;
    }
    if let Some(staged) = app.staged.take() {
        // What's in the tray goes out, with the text as its caption.
        if m.status != Status::Online {
            acts.push(Act::Toast("Not connected yet — it's still waiting to be sent.".into()));
            app.staged = Some(staged);
            return;
        }
        let mut caption = markup::encode_emojis(&text, &m.emojis_by_name());
        if let Some(codes) = app.shortcodes.get() {
            caption = encode_shortcodes(&caption, codes, m);
        }
        let reply_to = take_reply(app, m);
        match staged {
            super::Staged::Gif { gif, query } => {
                super::gifs::report_share(m, &gif.slug, &query);
                acts.push(Act::Cmd(Cmd::SendGif { gif, caption, reply_to }));
            }
            super::Staged::Picture { slot, .. } => acts.push(Act::Cmd(Cmd::SendStagedPicture { slot, caption, reply_to })),
        }
        app.composer.clear();
        ui.ctx().memory_mut(|mem| mem.request_focus(id));
        return;
    }
    if m.status == Status::Online
        && let Some(poll) = crate::core::parse_poll(&text)
    {
        match poll {
            Ok((question, options)) if !m.channel(&m.config.channel).is_some_and(|c| c.is_personal()) => {
                acts.push(Act::Cmd(Cmd::SendPoll { question, options }));
                app.composer.clear();
                app.replying = None;
            }
            Ok(_) => acts.push(Act::Toast("Polls are for channels, not your Personal space.".into())),
            Err(why) => acts.push(Act::Toast(why.into())),
        }
        ui.ctx().memory_mut(|mem| mem.request_focus(id));
        return;
    }
    if !text.trim().is_empty() && m.status == Status::Online {
        let mut body = markup::encode_emojis(&text, &m.emojis_by_name());
        if let Some(codes) = app.shortcodes.get() {
            body = encode_shortcodes(&body, codes, m);
        }
        let reply_to = take_reply(app, m);
        acts.push(Act::Cmd(Cmd::Send { body, reply_to }));
        app.composer.clear();
    } else if m.status != Status::Online && !text.trim().is_empty() {
        acts.push(Act::Toast("Not connected yet — your message is still here.".into()));
    }
    ui.ctx().memory_mut(|mem| mem.request_focus(id));
}

/// The message being replied to (only if it's in this channel; never in Personal space).
fn take_reply(app: &mut App, m: &Model) -> Option<String> {
    let r = app.replying.take()?;
    let personal = m.channel(&m.config.channel).is_some_and(|c| c.is_personal());
    (r.channel == m.config.channel && !personal).then_some(r.id)
}

/// `:smile:` -> 😄 for standard shortcodes (not custom emoji names, not in code).
pub fn encode_shortcodes(text: &str, codes: &std::collections::HashMap<String, String>, m: &Model) -> String {
    let custom = m.emojis_by_name();
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    let mut i = 0;
    let b = text.as_bytes();
    while i < b.len() {
        if b[i] == b'`' {
            in_code = !in_code;
            out.push('`');
            i += 1;
            continue;
        }
        if !in_code && b[i] == b':' && !(i > 0 && b[i - 1] == b'<') {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'+' || b[j] == b'-') {
                j += 1;
            }
            if j < b.len() && b[j] == b':' && j > start {
                let name = &text[start..j];
                if !custom.contains_key(&name.to_lowercase())
                    && let Some(ch) = codes.get(name)
                {
                    out.push_str(ch);
                    i = j + 1;
                    continue;
                }
            }
        }
        let c = text[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

pub fn emoji_picker(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, anchor: Rect) {
    let ctx = ui.ctx().clone();
    let mut emojis: Vec<&crate::protocol::Emoji> = m.emojis.values().collect();
    emojis.sort_by_key(|e| e.name.to_lowercase());
    let size = vec2(330.0, 330.0);
    let pos = anchor.left_top() - vec2(0.0, size.y + 6.0);
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("emoji-picker")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(&ctx, |ui| {
        picker_window(ui, "Custom Emojis", size.x - 6.0, p, |ui| {
            let q = app.emoji_picker.get_or_insert_with(String::new);
            theme::field(ui, egui::TextEdit::singleline(q).hint_text("Find an emoji").desired_width(f32::INFINITY), p);
            let q = q.to_lowercase();
            ui.add_space(3.0);
            let shown: Vec<_> = emojis.iter().filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q)).collect();
            let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), size.y - 120.0));
            ui.allocate_rect(rect, egui::Sense::hover());
            theme::well(ui, rect, p, |ui| {
                theme::scroll(ui, egui::ScrollArea::vertical().auto_shrink(false), p, |ui| {
                    if m.emojis.is_empty() {
                        ui.label(theme::text("No custom emojis yet.", p.gray));
                    } else if shown.is_empty() {
                        ui.label(theme::text("No match.", p.gray));
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        for e in shown {
                            let (r, resp) = tile(ui, 36.0, p);
                            let url = markup::emoji_url(&m.config.server, &e.id);
                            if let Some((tex, sz)) = app.images.get(&url, resp.hovered()) {
                                let s = 26.0 / sz.x.max(sz.y);
                                ui.painter().image(tex, Rect::from_center_size(r.center(), sz * s), Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            let resp = theme::tip(resp, format!(":{}:", e.name));
                            if resp.clicked() {
                                let sep = if app.composer.is_empty() || app.composer.ends_with(char::is_whitespace) { "" } else { " " };
                                app.composer.push_str(&format!("{sep}:{}: ", e.name));
                                app.focus_composer = true;
                                if !ui.input(|i| i.modifiers.shift) {
                                    close = true;
                                }
                            }
                            theme::context_menu(&resp, p, |ui| {
                                if theme::menu_item(ui, &format!("Delete :{}:", e.name), false, p).clicked() {
                                    acts.push(Act::Cmd(Cmd::DeleteEmoji(e.id.clone())));
                                    ui.close();
                                }
                            });
                        }
                    });
                });
            });
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                if theme::button_sized(ui, "Add emoji…", theme::Btn::Normal, true, vec2(80.0, 23.0), p).clicked() {
                    acts.push(Act::PickFile(super::PickFor::Emoji));
                }
                ui.label(egui::RichText::new("Right-click one to delete it.").font(theme::font(theme::small())).color(p.gray));
            });
        });
    });
    let outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| !area.response.rect.contains(pos) && !anchor.contains(pos));
    // Right-click menus live in their own layer; don't close under them.
    let menu_open = egui::Popup::is_any_open(&ctx);
    if close || (outside && !menu_open) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.emoji_picker = None;
    }
}

#[cfg(test)]
mod offline_tests {
    use super::offline_words;

    #[test]
    fn connection_errors_in_plain_words() {
        assert_eq!(offline_words("IO error: Connection refused (os error 61)"), "Nothing answered at that address.");
        assert_eq!(offline_words("IO error: failed to lookup address information: nodename nor servname provided, or not known"), "That address wasn't found. Is this computer online?");
        assert_eq!(offline_words("IO error: Connection timed out (os error 60)"), "The server didn't answer in time.");
        assert_eq!(offline_words("IO error: Network is unreachable (os error 51)"), "This computer can't reach the internet right now.");
        assert_eq!(offline_words("HTTP error: 502 Bad Gateway"), "The server answered, but not like a Yap server.");
        assert_eq!(offline_words("Connection reset without closing handshake"), "The connection dropped.");
    }
}

#[cfg(test)]
mod draft_tests {
    use super::{Drafts, ScrollState};

    #[test]
    fn drafts_stay_with_their_channel() {
        let mut d: Drafts<&str> = Drafts::default();
        let mut composer = String::new();
        // The first channel shown keeps what's there (a screenshot's staged picture).
        let mut tray = Some("pic");
        assert!(d.switch("general", &mut composer, &mut tray));
        assert_eq!(tray, Some("pic"));
        composer.push_str("half a thought");
        // To another channel: an empty box and tray; general keeps both.
        assert!(d.switch("games", &mut composer, &mut tray));
        assert_eq!((composer.as_str(), tray), ("", None));
        composer.push_str("gg");
        // Back: general's text and picture come back, games keeps "gg".
        d.switch("general", &mut composer, &mut tray);
        assert_eq!((composer.as_str(), tray), ("half a thought", Some("pic")));
        assert!(!d.switch("general", &mut composer, &mut tray), "already there");
        composer.clear();
        tray = None;
        d.switch("games", &mut composer, &mut tray);
        assert_eq!((composer.as_str(), tray), ("gg", None));
        // Sent in general meanwhile (the box emptied): nothing comes back there.
        d.switch("general", &mut composer, &mut tray);
        assert_eq!((composer.as_str(), tray), ("", None));
    }

    #[test]
    fn new_messages_pill() {
        let mut st = ScrollState::default();
        // The first look at a channel: nothing is "new".
        assert!(!st.track_new(Some(("a", 1, false)), false));
        // Scrolled up, someone writes: the pill shows, and stays.
        assert!(st.track_new(Some(("b", 2, false)), false));
        assert!(st.track_new(Some(("b", 2, false)), false));
        // At the bottom: gone.
        assert!(!st.track_new(Some(("b", 2, false)), true));
        // Our own message, or an older one coming back (a newer one deleted), isn't news.
        assert!(!st.track_new(Some(("c", 3, true)), false));
        assert!(!st.track_new(Some(("b", 2, false)), false));
        // At the bottom when it arrives: no pill.
        assert!(!st.track_new(Some(("d", 4, false)), true));
        // Fresh history after a reconnect (nothing known first): no pill for what was there.
        assert!(!st.track_new(None, false));
        assert!(!st.track_new(Some(("e", 5, false)), false));
    }
}

#[cfg(test)]
mod size_tests {
    /// Only a size that lays out (real pictures are 1–8000 px) is used as given.
    #[test]
    fn sane_sizes() {
        assert!(super::sane_size(1.0, 1.0) && super::sane_size(8000.0, 600.0));
        for (w, h) in [(0.0, 10.0), (-5.0, 10.0), (f32::INFINITY, 10.0), (f32::NAN, 10.0), (10.0, 1e30)] {
            assert!(!super::sane_size(w, h), "{w} × {h}");
        }
    }
}
