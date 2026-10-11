//! Since you left: the "New since …" divider, its jump pill, and Catch up.

use super::*;

/// "New since 9:41 PM" (or "yesterday, 9:41 PM", "Oct 3") for the divider.
pub fn since_words(mark: i64, h12: bool) -> String {
    let day = super::super::local_time(mark).date_naive();
    let today = chrono::Local::now().date_naive();
    if day == today {
        format!("New since {}", super::super::fmt_time(mark, h12))
    } else if Some(day) == today.pred_opt() {
        format!("New since yesterday, {}", super::super::fmt_time(mark, h12))
    } else {
        format!("New since {}", super::super::local_time(mark).format("%b %-d"))
    }
}

/// The quiet "New since …" line above the first unread message. Returns its rect.
pub fn unread_divider(ui: &mut egui::Ui, words: &str, p: &Pal) -> Rect {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let (color, font) = (p.danger, theme::bold_font(theme::SMALL));
    let text = words.to_string();
    let g = ui.painter().layout_no_wrap(text, font, color);
    let y = rect.center().y.round() + 0.5;
    let gx = rect.right() - 10.0 - g.size().x;
    ui.painter().hline(rect.left() + 8.0..=gx - 8.0, y, Stroke::new(1.0, color.gamma_multiply(0.8)));
    ui.painter().galley(pos2(gx, rect.center().y - g.size().y / 2.0), g, color);
    ui.add_space(2.0);
    rect
}

/// "↑ Jump to first unread", over the top of the message list while the divider is off screen.
pub fn jump_pill(ui: &mut egui::Ui, viewport: Rect, p: &Pal) -> bool {
    let label = "↑ Jump to first unread";
    let font = theme::small_font(theme::SMALL);
    let g = ui.painter().layout_no_wrap(label.into(), font, Color32::PLACEHOLDER);
    let size = vec2(g.size().x + 22.0, 20.0);
    let r = Rect::from_center_size(pos2(viewport.center().x, viewport.top() + 8.0 + size.y / 2.0), size);
    let resp = ui.interact(r, ui.id().with("jump-unread"), Sense::click());
    let painter = ui.painter().clone().with_layer_id(egui::LayerId::new(egui::Order::Middle, ui.id().with("jump-unread-layer")));
    // The web's tooltip-yellow tab with a thin dark outline.
    painter.rect_filled(r, 0.0, theme::INFO);
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, p.dk), egui::StrokeKind::Inside);
    painter.galley(r.center() - g.size() / 2.0, g, theme::INFO_TEXT);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

/// Channels with something new (the one you're looking at excepted).
pub fn unread_channels(m: &Model) -> Vec<&crate::protocol::Channel> {
    m.channels.iter().filter(|c| m.is_unread(c)).collect()
}

/// Catch up's order, like the web's: the channels that mention you first
/// (most first), then the newest.
pub fn catch_up_order<'a>(m: &Model, mut list: Vec<&'a crate::protocol::Channel>) -> Vec<&'a crate::protocol::Channel> {
    let newest = |c: &crate::protocol::Channel| m.unread_counts.as_ref().and_then(|u| u.get(&c.id)).and_then(|u| u.first.as_ref()).map(|f| f.ts).or(c.last_message_at).unwrap_or(0);
    list.sort_by_key(|c| (std::cmp::Reverse(m.mentions_for_you(&c.id)), std::cmp::Reverse(newest(c))));
    list
}

/// Catch up: every channel with something new (the ones that mention you
/// first), how much, and the first new line; click one to open it at the
/// "New since" divider. "Mark all read" under the list.
pub fn catch_up_popover(app: &mut App, ctx: &egui::Context, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let Some(pos) = app.catch_up else { return };
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("catch-up")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        super::super::chat::picker_window(ui, "Catch up", 300.0, p, |ui| {
            let list = catch_up_order(m, unread_channels(m));
            if list.is_empty() {
                ui.label(theme::text("You're all caught up.", theme::dim(p)));
                return;
            }
            ui.spacing_mut().item_spacing.y = 2.0;
            for c in list {
                let count = m.unread_counts.as_ref().and_then(|u| u.get(&c.id));
                let for_you = m.mentions_for_you(&c.id);
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                let hovered = resp.hovered();
                let on_sel = hovered;
                if hovered {
                    ui.painter().rect_filled(rect, 0.0, p.select);
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let fg = if on_sel { p.on_select } else { p.text };
                let sub = if on_sel { p.on_select } else { theme::dim(p) };
                icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 12.0, rect.top() + 12.0), vec2(13.0, 13.0)), if c.is_personal() { "lock" } else { "chat" }, sub);
                ui.painter().text(pos2(rect.left() + 24.0, rect.top() + 12.0), Align2::LEFT_CENTER, &c.name, theme::strong_font(theme::ui()), fg);
                let n = match count {
                    Some(u) if u.n >= 999 => "999+ new".to_string(),
                    Some(u) => format!("{} new", u.n),
                    None => "…".to_string(),
                };
                let ng = ui.painter().text(pos2(rect.right() - 8.0, rect.top() + 12.0), Align2::RIGHT_CENTER, n, theme::small_font(theme::SMALL), sub);
                if for_you > 0 {
                    // "· 2 for you": the channel's mention badge and the words, left of "N new".
                    let wg = ui.painter().text(pos2(ng.left() - 4.0, rect.top() + 12.0), Align2::RIGHT_CENTER, "for you ·", theme::small_font(theme::SMALL), sub);
                    super::super::sections::badge(ui.painter(), pos2(wg.left() - 4.0, rect.top() + 12.0), for_you as usize);
                }
                if let Some(first) = count.and_then(|u| u.first.as_ref()) {
                    let line = format!("{}: {}", m.user_name(&first.uid), crate::core::plain_text(&first.body, true));
                    let line = if crate::vault::is_note(&first.body) { format!("{}: 🔒 Encrypted note", m.user_name(&first.uid)) } else { line };
                    let mut job = egui::text::LayoutJob::simple_singleline(line, theme::small_font(theme::SMALL), sub);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 32.0);
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(rect.left() + 24.0, rect.top() + 21.0), g, sub);
                }
                if resp.clicked() {
                    acts.push(Act::Cmd(Cmd::OpenChannel(c.id.clone())));
                    if let Some(first) = count.and_then(|u| u.first.as_ref()) {
                        acts.push(Act::Cmd(Cmd::JumpTo { channel: c.id.clone(), id: first.id.clone() }));
                    }
                    close = true;
                }
            }
            // Every channel read up to its newest message (the web's "Mark all read").
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::tip(theme::button(ui, "Mark all read", theme::Btn::Normal, true, p), "Every channel counts as read up to its newest message").clicked() {
                    acts.push(Act::Cmd(Cmd::MarkAllRead));
                    close = true;
                }
            });
        });
    });
    let outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|q| !area.response.rect.contains(q));
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) || outside && app.catch_up_opened + 0.2 < ctx.input(|i| i.time) {
        app.catch_up = None;
    }
}

/// Open (or close) Catch up under `anchor`.
pub fn toggle_catch_up(app: &mut App, ctx: &egui::Context, anchor: Rect, acts: &mut Vec<Act>) {
    if app.catch_up.is_some() {
        app.catch_up = None;
    } else {
        app.catch_up = Some(anchor.left_bottom() + vec2(0.0, 4.0));
        app.catch_up_opened = ctx.input(|i| i.time);
        acts.push(Act::Cmd(Cmd::UnreadCounts));
    }
}
