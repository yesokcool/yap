//! What a message shows — encrypted Personal-space notes opened (or a lock),
//! long messages folded — and the pieces replies need: the quote line over
//! a message, the bar over the composer, jumping to the original.

use super::{Act, App, icons, theme};
use crate::core::Model;
use crate::protocol::{Gif, ImageRef, Message, ReplyRef};
use crate::vault::{Note, NoteImage};
use eframe::egui::{self, Color32, Pos2, Rect, Sense, pos2, vec2};
use std::borrow::Cow;
use std::sync::Arc;

/// Messages longer than this (characters or lines) fold until you ask for the rest.
pub const LONG_CHARS: usize = 3000;
pub const LONG_LINES: usize = 30;

pub const LOCKED: &str = "Encrypted. Unlock your Personal space to read it.";
pub const CANT_OPEN: &str = "Can't open this note (it was locked with another passcode).";

/// Decrypted notes by their ciphertext (None = it doesn't open with this key).
pub type NoteCache = std::collections::HashMap<String, Option<Arc<Note>>>;

/// A message as it's drawn.
pub struct Shown<'a> {
    pub text: Cow<'a, str>,
    pub gif: Option<Cow<'a, Gif>>,
    pub image: Option<&'a ImageRef>,
    /// A picture inside an encrypted note (fetched and decrypted).
    pub sealed_image: Option<NoteImage>,
    /// An encrypted note that can't be shown: why.
    pub lock: Option<&'static str>,
    /// A long message: its length in characters.
    pub long: Option<usize>,
    /// Folded: `text` is only the start.
    pub folded: bool,
}

/// Open an encrypted note with this device's key (cached).
pub fn note(app: &mut App, m: &Model, body: &str) -> Result<Arc<Note>, &'static str> {
    let Some(key) = m.vault_key else { return Err(LOCKED) };
    if app.notes_key != Some(key) || app.notes.len() > 4000 {
        app.notes.clear();
        app.notes_key = Some(key);
    }
    app.notes.entry(body.to_string()).or_insert_with(|| crate::vault::decrypt_note(&key, body).ok().map(Arc::new)).clone().ok_or(CANT_OPEN)
}

/// What `msg` says, ready to draw.
pub fn shown<'a>(app: &mut App, m: &Model, msg: &'a Message) -> Shown<'a> {
    let mut s = Shown { text: Cow::Borrowed(msg.body.as_str()), gif: msg.gif.as_ref().map(Cow::Borrowed), image: msg.image.as_ref(), sealed_image: None, lock: None, long: None, folded: false };
    if crate::vault::is_note(&msg.body) {
        s.text = Cow::Borrowed("");
        match note(app, m, &msg.body) {
            Ok(n) => {
                s.text = Cow::Owned(n.text.clone());
                s.gif = n.gif.clone().map(Cow::Owned);
                s.sealed_image = n.image.clone();
            }
            Err(why) => s.lock = Some(why),
        }
    }
    let lines = s.text.bytes().filter(|b| *b == b'\n').count() + 1;
    let chars = if s.text.len() > LONG_CHARS { s.text.chars().count() } else { 0 };
    if chars > LONG_CHARS || lines > LONG_LINES {
        s.long = Some(chars.max(s.text.chars().count()));
        if !app.expanded.contains(&msg.id) {
            s.folded = true;
            s.text = Cow::Owned(fold(&s.text));
        }
    }
    s
}

/// The start of a long message: ≤ 30 lines and ≤ 3000 characters, then "…".
pub fn fold(text: &str) -> String {
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i >= LONG_LINES {
            break;
        }
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line);
        if out.len() > LONG_CHARS * 4 {
            break;
        }
    }
    if let Some((cut, _)) = out.char_indices().nth(LONG_CHARS) {
        out.truncate(cut);
    }
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    out.push('…');
    out
}

/// "Show more (12,345 characters)" / "Show less" under a long message.
pub fn more_toggle(app: &mut App, ui: &mut egui::Ui, id: &str, chars: usize, folded: bool, color: Color32) {
    let label = if folded { format!("Show more ({} characters)", thousands(chars)) } else { "Show less".to_string() };
    let resp = ui.add(egui::Label::new(egui::RichText::new(label).font(theme::font(theme::small())).color(color)).sense(Sense::click()));
    if resp.hovered() {
        ui.painter().hline(resp.rect.x_range(), resp.rect.bottom() - 1.0, egui::Stroke::new(1.0, color));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        if folded {
            app.expanded.insert(id.to_string());
        } else {
            app.expanded.remove(id);
        }
    }
}

pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A locked (or unopenable) note: a lock and why (wrapping on narrow
/// windows); clicking asks for the passcode.
pub fn lock_line(ui: &mut egui::Ui, why: &str, color: Color32) -> bool {
    let resp = ui
        .horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let (r, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
            icons::paint(ui.painter(), r, "lock", color);
            ui.add(egui::Label::new(egui::RichText::new(why).font(theme::font(theme::chat())).italics().color(color)).sense(Sense::click()))
        })
        .inner;
    if resp.hovered() && why == LOCKED {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    why == LOCKED && theme::tip(resp, "Unlock…").clicked()
}

// ── Replies ────────────────────────────────────────────────────────────

/// Replying to a message (the bar over the composer).
#[derive(Clone)]
pub struct Replying {
    pub id: String,
    pub uid: String,
    pub channel: String,
}

/// Reply to `msg` (not in a Personal space; not to a message still sending).
pub fn start_reply(app: &mut App, m: &Model, msg: &Message) {
    if msg.pending || m.channel(&msg.channel_id).is_some_and(|c| c.is_personal()) {
        return;
    }
    if app.editing.is_some() {
        super::chat::cancel_edit(app);
    }
    app.replying = Some(Replying { id: msg.id.clone(), uid: msg.uid.clone(), channel: msg.channel_id.clone() });
    app.emoji_picker = None;
    app.focus_composer = true;
}

/// What the reply bar quotes: the message you're answering in a few words
/// (one line; the bar cuts it with "…"), or what it is.
pub fn replying_snippet(app: &mut App, m: &Model, r: &Replying) -> String {
    let Some(msg) = m.history.get(&r.channel).and_then(|h| h.messages.iter().find(|x| x.id == r.id)) else { return String::new() };
    let (text, image, gif) = if crate::vault::is_note(&msg.body) {
        match note(app, m, &msg.body) {
            Ok(n) => (crate::core::plain_text(&n.text, true), n.image.is_some(), n.gif.is_some()),
            Err(_) => return "🔒 Encrypted note".into(),
        }
    } else {
        (crate::core::plain_text(&msg.body, true), msg.image.is_some(), msg.gif.is_some())
    };
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let one = crate::core::clip(&one, 120);
    if !one.is_empty() {
        one
    } else if image {
        "📷 A picture".into()
    } else if gif {
        "GIF".into()
    } else {
        String::new()
    }
}

/// Is this the message being edited? (Brings it into view once, if it isn't
/// already, when the edit starts.)
pub fn editing_mark(app: &mut App, ui: &egui::Ui, id: &str, rect: Rect) -> bool {
    if app.editing.as_ref().is_none_or(|e| e.id != id) {
        return false;
    }
    if app.edit_scroll.as_deref() == Some(id) {
        app.edit_scroll = None;
        if !ui.clip_rect().contains_rect(rect) {
            ui.scroll_to_rect(rect, Some(egui::Align::Center));
        }
    }
    true
}

/// Who the bar says you're answering.
pub fn replying_to(m: &Model, r: &Replying) -> String {
    if r.uid == m.config.uid { "yourself".into() } else { m.users.get(&r.uid).map(|u| u.name.clone()).unwrap_or_else(|| "someone".into()) }
}

/// The quote's words: the original's first line (or what it was).
pub fn quote_words(r: &ReplyRef) -> String {
    if r.deleted {
        return "Original message was deleted".into();
    }
    if crate::vault::is_note(&r.body) {
        return "Encrypted note".into();
    }
    let plain = crate::core::plain_text(&r.body, false);
    let first = plain.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    if !first.is_empty() {
        first
    } else if r.image {
        "Picture".into()
    } else if r.gif {
        "GIF".into()
    } else {
        String::new()
    }
}

/// "↪ Name  first line…" over a reply, starting at `at` (or ending there when
/// `right`), at most `max_w` wide. Returns the click (none for a deleted original).
#[allow(clippy::too_many_arguments)]
pub fn paint_quote(ui: &mut egui::Ui, m: &Model, r: &ReplyRef, at: Pos2, max_w: f32, right: bool, name_color: Color32, text_color: Color32, size: f32) -> Option<egui::Response> {
    let painter = ui.painter().clone();
    let icon_w = size + 4.0;
    let name = if r.deleted { String::new() } else { m.user_name(&r.uid) };
    let name_g = painter.layout_no_wrap(name, theme::bold_font(size), name_color);
    let gap = if r.deleted { 0.0 } else { 5.0 };
    let words = quote_words(r);
    let room = (max_w - icon_w - name_g.size().x - gap).max(20.0);
    let mut job = egui::text::LayoutJob::single_section(words, egui::TextFormat { font_id: theme::font(size), color: text_color, italics: r.deleted, ..Default::default() });
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let text_g = painter.layout_job(job);
    let w = icon_w + name_g.size().x + gap + text_g.size().x;
    let h = name_g.size().y.max(text_g.size().y).max(size + 2.0);
    let x = if right { at.x - w } else { at.x };
    let rect = Rect::from_min_size(pos2(x, at.y), vec2(w, h));
    let resp = (!r.deleted).then(|| ui.interact(rect, ui.id().with(("quote", &r.id, at.y as i32)), Sense::click()));
    let hovered = resp.as_ref().is_some_and(|r| r.hovered());
    let fade = if hovered { 1.0 } else { 0.85 };
    icons::paint(&painter, Rect::from_center_size(pos2(x + size / 2.0, rect.center().y), vec2(size, size)), "reply", text_color.gamma_multiply(fade));
    let nx = x + icon_w;
    painter.galley(pos2(nx, rect.center().y - name_g.size().y / 2.0), name_g.clone(), name_color);
    let tx = nx + name_g.size().x + gap;
    painter.galley(pos2(tx, rect.center().y - text_g.size().y / 2.0), text_g.clone(), text_color);
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        painter.hline(tx..=tx + text_g.size().x, (rect.bottom() - 0.5).round(), egui::Stroke::new(1.0, text_color.gamma_multiply(0.6)));
    }
    resp.map(|r| theme::tip(r, "Show the original"))
}

/// Scroll to a message in this channel and flash it, loading older pages
/// until it's found (the core gives up after 20).
pub fn jump_to(app: &mut App, m: &Model, id: &str, acts: &mut Vec<Act>) {
    jump_in(app, m, &m.config.channel, id, acts)
}

/// A reply's quote: the original may live in another channel (Save to Personal).
pub fn jump_to_reply(app: &mut App, m: &Model, r: &ReplyRef, acts: &mut Vec<Act>) {
    jump_in(app, m, r.channel_id.as_deref().unwrap_or(&m.config.channel), &r.id, acts)
}

/// Scroll to a message in `channel` (opening it first if need be) and flash it.
pub fn jump_in(app: &mut App, m: &Model, channel: &str, id: &str, acts: &mut Vec<Act>) {
    let loaded = channel == m.config.channel && m.history.get(channel).is_some_and(|h| h.messages.iter().any(|x| x.id == id));
    if loaded && m.view.is_none() {
        app.jump = Some(id.to_string());
    } else {
        acts.push(Act::Cmd(crate::core::Cmd::JumpTo { channel: channel.to_string(), id: id.to_string() }));
    }
}

/// After drawing a message: scroll to it if it's the one jumped to; how
/// strongly it flashes now (0..1).
pub fn flash(app: &mut App, ui: &egui::Ui, id: &str, rect: Rect) -> f32 {
    let now = ui.input(|i| i.time);
    if app.jump.as_deref() == Some(id) {
        app.jump = None;
        ui.scroll_to_rect(rect, Some(egui::Align::Center));
        app.flash = Some((id.to_string(), now));
    }
    match &app.flash {
        Some((fid, t0)) if fid == id => {
            let t = (now - t0) as f32;
            if t > 1.8 {
                app.flash = None;
                return 0.0;
            }
            ui.ctx().request_repaint();
            // Pulse in, hold, fade out.
            ((t / 0.15).min(1.0) * (1.0 - ((t - 0.9) / 0.9).clamp(0.0, 1.0))).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

pub const SAVE_TIP: &str = "Save to your Personal space (a bookmark: it links back here)";

// ── The hover toolbar ──────────────────────────────────────────────────

/// What a message's hover toolbar offers, in its order (this decides which
/// buttons and what they do).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgAction {
    React,
    Pin,
    Reply,
    Save,
    Edit,
    Delete,
}

impl MsgAction {
    pub fn icon(self) -> &'static str {
        match self {
            MsgAction::React => "smile-plus",
            MsgAction::Pin => "pin",
            MsgAction::Reply => "reply",
            MsgAction::Save => "bookmark",
            MsgAction::Edit => "pencil",
            MsgAction::Delete => "trash",
        }
    }

    /// The tooltip (`pinned`: the message is pinned already).
    pub fn tip(self, pinned: bool) -> &'static str {
        match self {
            MsgAction::React => "Add a reaction",
            MsgAction::Pin if pinned => "Unpin",
            MsgAction::Pin => "Pin (everyone sees it under Pinned)",
            MsgAction::Reply => "Reply",
            MsgAction::Save => SAVE_TIP,
            MsgAction::Edit => "Edit (or press Up in an empty message box)",
            MsgAction::Delete => "Delete (shift-click skips the question)",
        }
    }
}

/// The toolbar's buttons for `msg`: react and pin always; reply and save
/// where they apply; edit for your own (not locked notes or dice); delete
/// for your own, or anyone's if you're the owner or a mod.
pub fn message_actions(m: &Model, msg: &Message, shown: &Shown<'_>) -> Vec<MsgAction> {
    let mine = msg.uid == m.config.uid;
    let mut out = vec![MsgAction::React, MsgAction::Pin];
    if can_reply(m, msg) {
        out.push(MsgAction::Reply);
    }
    if can_save(m, msg) {
        out.push(MsgAction::Save);
    }
    if mine && shown.lock.is_none() && msg.roll.is_none() {
        out.push(MsgAction::Edit);
    }
    if mine || m.am_moderator() {
        out.push(MsgAction::Delete);
    }
    out
}

/// Do what a toolbar button says. `react_at`: where the reaction picker opens.
pub fn run_action(app: &mut App, ui: &egui::Ui, m: &Model, msg: &Message, action: MsgAction, react_at: Pos2, acts: &mut Vec<Act>) {
    match action {
        MsgAction::React => app.react_picker = Some((msg.id.clone(), react_at, String::new())),
        MsgAction::Pin => acts.push(Act::Cmd(crate::core::Cmd::Pin { id: msg.id.clone(), on: msg.pinned_at.is_none() })),
        MsgAction::Reply => start_reply(app, m, msg),
        MsgAction::Save => acts.push(Act::Cmd(crate::core::Cmd::SaveToPersonal { id: msg.id.clone(), comment: String::new() })),
        MsgAction::Edit => super::chat::start_edit(app, m, msg),
        MsgAction::Delete => {
            if ui.input(|i| i.modifiers.shift) {
                acts.push(Act::Cmd(crate::core::Cmd::Delete(msg.id.clone())));
            } else {
                app.modal = Some(super::dialogs::Modal::ConfirmDelete { id: msg.id.clone() });
            }
        }
    }
}

/// Save to Personal: any sent message (a bookmark that's already in your
/// Personal space excepted).
pub fn can_save(m: &Model, msg: &Message) -> bool {
    !msg.pending && m.channels.iter().any(|c| c.is_personal()) && !(m.channel(&msg.channel_id).is_some_and(|c| c.is_personal()) && msg.reply.is_some() && msg.body.is_empty())
}

/// The Reply action in a message's hover toolbar applies here.
pub fn can_reply(m: &Model, msg: &Message) -> bool {
    !msg.pending && !m.channel(&msg.channel_id).is_some_and(|c| c.is_personal())
}

/// The Personal space is encrypted and this device doesn't have the key.
pub fn locked_here(m: &Model) -> bool {
    m.vault.is_some() && m.vault_key.is_none() && m.channel(&m.config.channel).is_some_and(|c| c.is_personal())
}

/// The header note for the Personal space.
pub fn personal_note(m: &Model) -> &'static str {
    match (&m.vault, &m.vault_key) {
        (Some(_), Some(_)) => "end-to-end encrypted",
        (Some(_), None) => "locked",
        _ => "only you · not encrypted",
    }
}
