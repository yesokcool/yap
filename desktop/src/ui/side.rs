//! The panel on the right: Search (this channel or everywhere, with
//! `from:` / `has:` filters) and the channel's pinned messages. Clicking a
//! result opens its channel and jumps to it (loading older pages as needed).

use super::{Act, App, content, fmt_stamp, icons, theme, theme::Pal};
use crate::core::{Cmd, Model};
use crate::protocol::Message;
use eframe::egui::{self, Align2, Color32, Rect, Sense, pos2, vec2};

/// What the right-hand panel shows.
pub enum Side {
    Search {
        q: String,
        everywhere: bool,
        /// When the query last changed (searches after a short pause) / what was sent.
        edited: f64,
        sent: Option<(String, bool)>,
        focus: bool,
    },
    Pins,
    /// The media shelf: this channel's pictures and GIFs.
    Shelf,
}

impl Side {
    pub fn search() -> Self {
        Side::Search { q: String::new(), everywhere: false, edited: 0.0, sent: None, focus: true }
    }
}

/// Panel width.
pub const WIDTH: f32 = 320.0;

/// Toggle the panel to `want` (clicking the same header button again closes it).
pub fn toggle(app: &mut App, want: Side, m: &Model, acts: &mut Vec<Act>) {
    let same = matches!((&app.side, &want), (Some(Side::Search { .. }), Side::Search { .. }) | (Some(Side::Pins), Side::Pins) | (Some(Side::Shelf), Side::Shelf));
    if same {
        app.side = None;
        acts.push(Act::Cmd(Cmd::Search { q: String::new(), channel: None }));
        return;
    }
    if matches!(want, Side::Pins) {
        acts.push(Act::Cmd(Cmd::Pins(m.config.channel.clone())));
    }
    if matches!(want, Side::Shelf) {
        acts.push(Act::Cmd(Cmd::Shelf { channel: m.config.channel.clone(), more: false }));
    }
    app.side = Some(want);
}

/// The panel's title and icon.
pub fn title(side: &Side) -> (&'static str, &'static str) {
    match side {
        Side::Search { .. } => ("Search", "search"),
        Side::Pins => ("Pinned", "pin"),
        Side::Shelf => ("Pictures and GIFs", "images"),
    }
}

/// Close the panel (and forget the search).
fn close(app: &mut App, acts: &mut Vec<Act>) {
    app.side = None;
    acts.push(Act::Cmd(Cmd::Search { q: String::new(), channel: None }));
}

/// The panel — its window chrome, title, close box and body — filling `ui`
/// (inside the frame its container gives it), docked or floating. Returns
/// its rect.
pub fn panel(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) -> Rect {
    let Some(side) = app.side.as_ref() else { return ui.max_rect() };
    let (title, icon) = title(side);
    // Windows 98: a window with a title bar and a × box.
    theme::window_bevel(ui, p);
    let t = theme::title_bar(ui, p, Some(icon), title);
    let b = Rect::from_min_size(pos2(t.right() - 20.0, t.top() + 3.0), vec2(18.0, 16.0));
    if theme::caption_button(ui, b, "x", "Close", false, p).clicked() {
        close(app, acts);
    }
    egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 4, top: 6, bottom: 4 }).show(ui, |ui| body(app, ui, m, p, acts));
    ui.max_rect()
}

/// No room beside the chat (a narrow window): the panel floats over the
/// chat's right side at `rect` instead (Esc or × closes it).
pub fn overlay(app: &mut App, ctx: &egui::Context, m: &Model, p: &Pal, acts: &mut Vec<Act>, rect: Rect) {
    egui::Area::new(egui::Id::new("side-overlay")).order(egui::Order::Middle).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
        let frame = theme::window_frame(p, egui::Margin::ZERO);
        let inner = rect.size() - vec2(2.0, 2.0) * theme::PAD as f32;
        frame.show(ui, |ui| {
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            panel(app, ui, m, p, acts);
        });
    });
}

pub fn body(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let Some(side) = app.side.as_mut() else { return };
    ui.spacing_mut().item_spacing.y = 4.0;
    match side {
        Side::Search { q, everywhere, edited, sent, focus } => {
            let now = ui.input(|i| i.time);
            let r = theme::field(ui, egui::TextEdit::singleline(q).hint_text("Search messages").desired_width(f32::INFINITY), p);
            if *focus {
                r.request_focus();
                *focus = false;
            }
            if r.changed() {
                *edited = now;
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                let mut here = !*everywhere;
                if theme::radio_row(ui, &mut here, &[(true, "This channel"), (false, "Everywhere")], p) {
                    *everywhere = !here;
                    *edited = now - 1.0;
                }
            });
            // (No-break spaces: a line never starts with "·" or splits "a phrase".)
            ui.label(egui::RichText::new("from:Name\u{a0}· has:image\u{a0}· has:gif\u{a0}· has:link\u{a0}· \"a\u{a0}phrase\"").font(theme::font(theme::small())).color(p.gray));
            // Search after a short pause (or right away on Enter).
            let want = (q.trim().to_string(), *everywhere);
            let due = enter || (now - *edited > 0.6 && sent.as_ref() != Some(&want));
            if due && sent.as_ref() != Some(&want) {
                *sent = Some(want.clone());
                let channel = (!want.1).then(|| m.config.channel.clone());
                acts.push(Act::Cmd(Cmd::Search { q: want.0.clone(), channel }));
            } else if sent.as_ref() != Some(&want) {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }
            ui.add_space(2.0);
            let words = query_words(&want.0);
            let results: Vec<Message> = m.search.as_ref().map(|s| s.results().into_iter().cloned().collect()).unwrap_or_default();
            // Nothing found: the web's words, and from one channel a way on ("Search everywhere").
            let here_name = m.channel(&m.config.channel).map(|c| c.name.clone()).unwrap_or_else(|| "this channel".into());
            let none = matches!(&m.search, Some(s) if !s.loading && results.is_empty()) && !want.0.is_empty();
            let status = match &m.search {
                _ if want.0.is_empty() => String::new(),
                Some(s) if s.loading => "Searching…".to_string(),
                Some(_) if results.is_empty() && want.1 => format!("Nothing for “{}” anywhere.", want.0),
                Some(_) if results.is_empty() => format!("Nothing for “{}” in {here_name}.", want.0),
                Some(_) => format!("{} result{}{}", results.len(), if results.len() == 1 { "" } else { "s" }, if results.len() >= 50 { " (the newest 50)" } else { "" }),
                None => String::new(),
            };
            if !status.is_empty() {
                ui.label(egui::RichText::new(status).font(theme::font(theme::small())).color(p.gray));
            }
            if none && !want.1 {
                let r = ui.add(egui::Label::new(egui::RichText::new("Search everywhere").font(theme::font(theme::small())).color(p.link).underline()).sense(Sense::click()));
                if r.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if r.clicked() {
                    *everywhere = true;
                    *edited = now - 1.0;
                }
            }
            list(app, ui, m, p, acts, &results, &words, true);
        }
        Side::Shelf => shelf(app, ui, m, p, acts),
        Side::Pins => {
            let pins = m.pins.get(&m.config.channel);
            let name = m.channel(&m.config.channel).map(|c| c.name.clone()).unwrap_or_default();
            match pins {
                None => {
                    ui.label(theme::text("Loading…", p.gray));
                }
                Some(list_) if list_.is_empty() => {
                    ui.label(theme::text(format!("Nothing pinned in {name} yet."), p.text));
                    ui.label(egui::RichText::new("Pin a message from its toolbar (the pin), and it shows up here for everyone.").font(theme::font(theme::small())).color(p.gray));
                }
                Some(list_) => {
                    let list_ = list_.clone();
                    ui.label(egui::RichText::new(format!("{} pinned in {name}", list_.len())).font(theme::font(theme::small())).color(p.gray));
                    list(app, ui, m, p, acts, &list_, &[], false);
                }
            }
        }
    }
}

/// Result rows: where · who · when, then the text with matches highlighted.
#[allow(clippy::too_many_arguments)]
fn list(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, items: &[Message], words: &[String], show_channel: bool) {
    theme::scroll(ui, egui::ScrollArea::vertical().id_salt("side-list").auto_shrink([false, false]), p, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 0.0;
        for msg in items {
            let text = snippet_text(app, m, msg);
            let snip = window(&text, words, 150);
            let w = ui.available_width();
            // Measure the snippet to size the row.
            // Marks: yellow.
            let (mark_bg, mark_fg) = (theme::INFO, theme::INFO_TEXT);
            let job = highlight(&snip, words, theme::font(theme::ui()), p.text, mark_bg, mark_fg);
            let mut job = job;
            job.wrap.max_width = w - 16.0;
            job.wrap.max_rows = 3;
            let galley = ui.painter().layout_job(job);
            let h = 8.0 + 16.0 + 2.0 + galley.size().y + 8.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, p.select);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let on_sel = resp.hovered();
            let fg = if on_sel { p.on_select } else { p.text };
            let gray = if on_sel { p.on_select } else { p.gray };
            // Header: #channel · Name, time on the right.
            let mut head = egui::text::LayoutJob::default();
            if show_channel {
                let ch = m.channel(&msg.channel_id).map(|c| c.name.clone()).unwrap_or_else(|| "?".into());
                head.append(&ch, 0.0, egui::TextFormat { font_id: theme::font(theme::small()), color: gray, ..Default::default() });
                head.append(" · ", 0.0, egui::TextFormat { font_id: theme::font(theme::small()), color: gray, ..Default::default() });
            }
            let name_color = if on_sel { p.on_select } else if msg.uid == m.config.uid { p.me } else { p.them };
            head.append(&m.user_name(&msg.uid), 0.0, egui::TextFormat { font_id: theme::bold_font(theme::small()), color: name_color, ..Default::default() });
            // The time first (short, like the web's), then where · who in what's left, ending in "…".
            let top = rect.top() + 8.0;
            let tg = ui.painter().layout_no_wrap(super::fmt_short(msg.ts, app.h12), theme::font(theme::small()), gray);
            let time_left = rect.right() - 8.0 - tg.size().x;
            let pinned = msg.pinned_at.is_some() && show_channel;
            if pinned {
                icons::paint(ui.painter(), Rect::from_center_size(pos2(time_left - 10.0, top + 7.0), vec2(11.0, 11.0)), "pin", gray);
            }
            head.wrap = egui::text::TextWrapping::truncate_at_width((time_left - if pinned { 20.0 } else { 8.0 } - rect.left() - 8.0).max(20.0));
            let hg = ui.painter().layout_job(head);
            ui.painter().with_clip_rect(rect).galley(pos2(rect.left() + 8.0, top), hg, fg);
            ui.painter().galley(pos2(time_left, top), tg, gray);
            if on_sel {
                ui.painter().galley_with_override_text_color(pos2(rect.left() + 8.0, top + 18.0), galley, p.on_select);
            } else {
                ui.painter().galley(pos2(rect.left() + 8.0, top + 18.0), galley, p.text);
            }
            theme::etched_hline(ui.painter(), rect.x_range(), rect.bottom() - 1.0, p);
            let resp = theme::tip(resp, format!("{}\nClick to show it in the chat", fmt_stamp(msg.ts, app.h12)));
            if resp.clicked() {
                if msg.channel_id == m.config.channel && m.view.is_none() {
                    content::jump_to(app, m, &msg.id, acts);
                } else {
                    acts.push(Act::Cmd(Cmd::JumpTo { channel: msg.channel_id.clone(), id: msg.id.clone() }));
                }
            }
            // Pinned list: unpin on the right while pointing at it.
            if !show_channel && resp.hovered() {
                let b = Rect::from_center_size(pos2(rect.right() - 18.0, rect.bottom() - 16.0), vec2(22.0, 20.0));
                if ui.place(b, |ui: &mut egui::Ui| icons::tool(ui, "x", vec2(22.0, 20.0), gray, "Unpin", false)).clicked() {
                    acts.push(Act::Cmd(Cmd::Pin { id: msg.id.clone(), on: false }));
                }
            }
        }
    });
}

/// The words a query looks for (filters and quotes dropped), lowercase.
pub fn query_words(q: &str) -> Vec<String> {
    crate::core::query_tokens(q)
        .into_iter()
        .filter(|t| !t.to_lowercase().starts_with("from:") && !t.to_lowercase().starts_with("has:"))
        .map(|t| t.trim_matches('"').to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// What a result says, on one line (encrypted notes opened here).
fn snippet_text(app: &mut App, m: &Model, msg: &Message) -> String {
    let (text, gif, image) = if crate::vault::is_note(&msg.body) {
        match content::note(app, m, &msg.body) {
            Ok(n) => (crate::core::plain_text(&n.text, true), n.gif.is_some(), n.image.is_some()),
            Err(_) => ("🔒 Encrypted note".to_string(), false, false),
        }
    } else {
        (crate::core::plain_text(&msg.body, true), msg.gif.is_some(), msg.image.is_some())
    };
    // Like the web's results: "📷 " / "GIF " before the words, "(no text)" without any.
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let extra = if image { "📷 " } else if gif { "GIF " } else { "" };
    match (one.is_empty(), extra.is_empty()) {
        (false, _) => format!("{extra}{one}"),
        (true, false) => extra.trim_end().to_string(),
        (true, true) => "(no text)".into(),
    }
}

/// ≤ `max` characters of `text` around the first match.
fn window(text: &str, words: &[String], max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let first = words.iter().filter_map(|w| find(&lower, &w.chars().collect::<Vec<_>>(), 0)).min().unwrap_or(0);
    // From a word start a little before the first match, cut at a word (the web's clip()).
    let mut start = first.saturating_sub(40);
    while start > 0 && start < first && !chars[start - 1].is_whitespace() {
        start += 1;
    }
    let rest: String = chars[start..].iter().collect();
    let out = crate::core::clip(&rest, max);
    if start > 0 { format!("…{out}") } else { out }
}

fn find(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// `text` with every match of `words` on a highlight.
pub fn highlight(text: &str, words: &[String], font: egui::FontId, color: Color32, mark_bg: Color32, mark_fg: Color32) -> egui::text::LayoutJob {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let mut marked = vec![false; chars.len()];
    for w in words {
        let needle: Vec<char> = w.chars().collect();
        let mut at = 0;
        while let Some(i) = find(&lower, &needle, at) {
            for m in marked.iter_mut().skip(i).take(needle.len()) {
                *m = true;
            }
            at = i + needle.len().max(1);
        }
    }
    let mut job = egui::text::LayoutJob::default();
    let mut i = 0;
    while i < chars.len() {
        let on = marked[i];
        let mut j = i;
        while j < chars.len() && marked[j] == on {
            j += 1;
        }
        let piece: String = chars[i..j].iter().collect();
        let fmt = if on {
            egui::TextFormat { font_id: font.clone(), color: mark_fg, background: mark_bg, ..Default::default() }
        } else {
            egui::TextFormat { font_id: font.clone(), color, ..Default::default() }
        };
        job.append(&piece, 0.0, fmt);
        i = j;
    }
    job
}

/// The media shelf: every picture and GIF in the channel as a grid, newest
/// first; click one to see it in the chat. "Load more" pages back.
fn shelf(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let ch = m.config.channel.clone();
    let name = m.channel(&ch).map(|c| c.name.clone()).unwrap_or_default();
    // Opened, or the channel changed under it: (re)load.
    let st = m.shelf.get(&ch);
    if st.is_none() {
        acts.push(Act::Cmd(Cmd::Shelf { channel: ch.clone(), more: false }));
    }
    let personal = m.channel(&ch).is_some_and(|c| c.is_personal());
    let Some(st) = st else {
        ui.label(theme::text("Loading…", p.gray));
        return;
    };
    if st.items.is_empty() && st.loading {
        ui.label(theme::text("Loading…", p.gray));
        return;
    }
    if st.items.is_empty() {
        ui.label(theme::text(format!("No pictures or GIFs in {name} yet."), p.text));
        if personal && m.vault.is_some() {
            ui.label(egui::RichText::new("Encrypted pictures are inside your sealed notes, so they don't show here.").font(theme::font(theme::small())).color(p.gray));
        }
        return;
    }
    ui.label(egui::RichText::new(format!("{}{} in {name}", st.items.len(), if st.has_more { "+" } else { "" })).font(theme::font(theme::small())).color(p.gray));
    let items = st.items.clone();
    let (has_more, loading) = (st.has_more, st.loading);
    theme::scroll(ui, egui::ScrollArea::vertical().id_salt("shelf").auto_shrink([false, false]), p, |ui| {
        let w = ui.available_width();
        let cols = 3usize;
        let gap = 4.0;
        let side = ((w - gap * (cols as f32 - 1.0)) / cols as f32).floor();
        for row in items.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for msg in row {
                    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click());
                    let url = match (&msg.image, &msg.gif) {
                        (Some(img), _) => Some(format!("{}/media/{}", m.config.server, img.id)),
                        (None, Some(g)) => g.still.clone().or(g.webp.clone()).or(g.gif.clone()),
                        _ => None,
                    };
                    let hovered = resp.hovered();
                    let tex = url.as_ref().and_then(|u| app.images.get(u, hovered && msg.gif.is_none()).or_else(|| app.images.get(u, false)));
                    let r = 0.0;
                    match tex {
                        Some((t, sz)) => {
                            // Cover the square (crop the long side).
                            let s = (side / sz.x).max(side / sz.y);
                            let (uw, uh) = ((side / (sz.x * s)).min(1.0), (side / (sz.y * s)).min(1.0));
                            let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(uw, uh));
                            if r > 0.0 {
                                ui.painter().add(egui::Shape::Rect(egui::epaint::RectShape::filled(rect, r, Color32::WHITE).with_texture(t, uv)));
                            } else {
                                ui.painter().image(t, rect, uv, Color32::WHITE);
                            }
                        }
                        None => {
                            ui.painter().rect_filled(rect, r, p.code_bg);
                        }
                    }
                    if msg.gif.is_some() {
                        let badge = Rect::from_min_size(rect.left_bottom() + vec2(4.0, -16.0), vec2(24.0, 12.0));
                        ui.painter().rect_filled(badge, 2.0, Color32::from_black_alpha(160));
                        ui.painter().text(badge.center(), Align2::CENTER_CENTER, "GIF", theme::bold_font(8.5), Color32::WHITE);
                    }
                    if hovered {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        ui.painter().rect_stroke(rect, r, egui::Stroke::new(2.0, p.select), egui::StrokeKind::Inside);
                    }
                    let tip = format!("{} · {}\nClick to show it in the chat", m.user_name(&msg.uid), fmt_stamp(msg.ts, app.h12));
                    if theme::tip(resp, tip).clicked() {
                        if msg.channel_id == m.config.channel && m.view.is_none() {
                            content::jump_to(app, m, &msg.id, acts);
                        } else {
                            acts.push(Act::Cmd(Cmd::JumpTo { channel: msg.channel_id.clone(), id: msg.id.clone() }));
                        }
                    }
                }
            });
            ui.add_space(gap);
        }
        if has_more {
            ui.add_space(4.0);
            if theme::button(ui, if loading { "Loading…" } else { "Load more" }, theme::Btn::Normal, !loading, p).clicked() {
                acts.push(Act::Cmd(Cmd::Shelf { channel: m.config.channel.clone(), more: true }));
            }
        }
    });
}
