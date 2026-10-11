//! Under a message: polls, server-rolled dice, "▶ Watch together" for a
//! YouTube link, and away auto-replies.

use super::*;

/// What goes under a message's text: a poll, the Watch
/// together button for a YouTube link, and away auto-replies.
/// `right`: line things up on the right.
#[allow(clippy::too_many_arguments)]
pub fn under_message(app: &mut App, ui: &mut egui::Ui, m: &Model, msg: &Message, text: &str, right: bool, p: &Pal, acts: &mut Vec<Act>) {
    if let Some(poll) = &msg.poll {
        ui.add_space(4.0);
        poll_card(ui, m, msg, poll, p, acts);
    }
    if !msg.pending
        && let Some(id) = crate::core::youtube_id(text)
    {
        ui.add_space(3.0);
        let layout = if right { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 24.0), layout, |ui| {
            let r = theme::button_sized(ui, "▶ Watch together", theme::Btn::Normal, true, vec2(120.0, 22.0), p);
            if theme::tip(r, "Queue it in Watch together (plays now if nothing is on) and open it in your browser").clicked() {
                acts.push(Act::Cmd(Cmd::OpenWeb { place: Some("watch".into()), video: Some(id) }));
                acts.push(Act::Toast("Opening Watch together in your browser…".into()));
            }
        });
    }
    if let Some(lines) = m.away_lines.get(&msg.id) {
        let now = crate::core::now_ms();
        for (uid, text, until) in lines.iter().filter(|(_, _, u)| u.is_none_or(|u| u > now)) {
            let name = m.user_name(uid);
            let words = match until {
                Some(u) => format!("{name} is away: {text} · back {}", until_words(*u, app.h12)),
                None => format!("{name} is away: {text}"),
            };
            let color = theme::dim(p);
            let layout = if right { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 16.0), layout, |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                icons::paint(ui.painter(), r, "moon", color);
                ui.label(egui::RichText::new(words).font(theme::small_font(theme::SMALL)).italics().color(color));
            });
        }
    }
}

/// "at 6:00 PM", "tomorrow, 9:00 AM", "Thursday, 9:00 AM" (as the web says it).
pub fn until_words(ts: i64, h12: bool) -> String {
    let day = super::super::local_time(ts).date_naive();
    let today = chrono::Local::now().date_naive();
    let time = super::super::fmt_time(ts, h12);
    if day == today {
        format!("at {time}")
    } else if Some(day) == today.succ_opt() {
        format!("tomorrow, {time}")
    } else {
        format!("{}, {time}", super::super::local_time(ts).format("%A"))
    }
}

/// A poll under its question: every option as a bar (how many picked it),
/// yours ticked; click to vote, click yours again to take it back.
pub fn poll_card(ui: &mut egui::Ui, m: &Model, msg: &Message, poll: &Poll, p: &Pal, acts: &mut Vec<Act>) {
    let me = &m.config.uid;
    let total: usize = poll.votes.iter().map(Vec::len).sum();
    let mine = poll.votes.iter().position(|v| v.contains(me));
    let (fill, border, radius) = theme::card_look(p);
    let width = ui.available_width().min(420.0);
    let text_color = p.text;
    let out = egui::Frame::new().fill(fill).stroke(border).corner_radius(radius as u8).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        ui.set_width(width - 16.0);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let (r, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
            icons::paint(ui.painter(), r, "poll", theme::accent(p));
            ui.label(egui::RichText::new("Poll").font(theme::strong_font(theme::small())).color(theme::dim(p)));
        });
        for (i, option) in poll.options.iter().enumerate() {
            let n = poll.votes.get(i).map(Vec::len).unwrap_or(0);
            let picked = mine == Some(i);
            let h = 24.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), if msg.pending { Sense::hover() } else { Sense::click() });
            let frac = if total > 0 { n as f32 / total as f32 } else { 0.0 };
            let painter = ui.painter();
            // Like the web: each option a button (yours pushed in), a soft title-blue bar inside.
            painter.rect_filled(rect, 0.0, p.face);
            if n > 0 {
                let inner = rect.shrink(3.0);
                painter.rect_filled(Rect::from_min_size(inner.min, vec2((inner.width() * frac).max(3.0), inner.height())), 0.0, p.title_b.gamma_multiply(0.22));
            }
            theme::bevel(painter, rect, p, if picked || resp.is_pointer_button_down_on() { Bevel::Pressed } else { Bevel::Raised });
            let fg = text_color;
            let mut x = rect.left() + 8.0;
            if picked {
                icons::paint(painter, Rect::from_center_size(pos2(x + 6.0, rect.center().y), vec2(12.0, 12.0)), "check", fg);
                x += 16.0;
            }
            let count = if total > 0 { format!("{n} · {}%", (frac * 100.0).round()) } else { n.to_string() };
            let cc = p.text.gamma_multiply(0.8);
            let cg = painter.layout_no_wrap(count, theme::small_font(theme::SMALL), cc);
            let cw = cg.size().x;
            painter.galley(pos2(rect.right() - 8.0 - cw, rect.center().y - cg.size().y / 2.0), cg, Color32::PLACEHOLDER);
            let mut job = egui::text::LayoutJob::simple_singleline(option.clone(), if picked { theme::strong_font(theme::ui()) } else { theme::font(theme::ui()) }, fg);
            job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - 16.0 - cw - x).max(20.0));
            let g = painter.layout_job(job);
            painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
            if msg.pending {
                continue;
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let names: Vec<String> = poll.votes.get(i).map(|v| v.iter().map(|u| m.user_name(u)).collect()).unwrap_or_default();
            let tip = if names.is_empty() { if picked { "Click to take your vote back".to_string() } else { "Click to vote".to_string() } } else { format!("{}{}", names.join(", "), if picked { "\nClick to take your vote back" } else { "" }) };
            if theme::tip(resp, tip).clicked() {
                acts.push(Act::Cmd(Cmd::PollVote { id: msg.id.clone(), option: if picked { None } else { Some(i) } }));
            }
        }
        let foot = match total {
            0 => "No votes yet".to_string(),
            1 => "1 vote".to_string(),
            n => format!("{n} votes"),
        };
        ui.label(egui::RichText::new(foot).font(theme::small_font(theme::SMALL)).color(theme::dim(p)));
    });
    theme::bevel(ui.painter(), out.response.rect, p, Bevel::Etched);
}

/// A /roll or /flip, rolled by the server: the dice (or the coin) and the total.
/// Returns the card's rect.
pub fn roll_view(ui: &mut egui::Ui, roll: &Roll, p: &Pal, right: bool) -> Rect {
    let (fill, border, radius) = theme::card_look(p);
    let text = p.text;
    let flip = roll.kind == "flip";
    let layout = if right { egui::Layout::right_to_left(egui::Align::Center) } else { egui::Layout::left_to_right(egui::Align::Center) };
    let out = ui.allocate_ui_with_layout(vec2(ui.available_width(), 34.0), layout, |ui| {
        let r = egui::Frame::new().fill(fill).stroke(border).corner_radius(radius as u8).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (ir, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                if flip {
                    ui.painter().circle_filled(ir.center(), 9.0, Color32::from_rgb(0xe8, 0xb9, 0x3a));
                    ui.painter().circle_stroke(ir.center(), 9.0, Stroke::new(1.5, Color32::from_rgb(0x9c, 0x74, 0x10)));
                    let face = if roll.rolls.first() == Some(&0) { "H" } else { "T" };
                    ui.painter().text(ir.center(), Align2::CENTER_CENTER, face, theme::strong_font(11.0), Color32::from_rgb(0x5a, 0x40, 0x05));
                } else {
                    icons::paint(ui.painter(), ir, "dice", theme::accent(p));
                }
                if flip {
                    let side = if roll.rolls.first() == Some(&0) { "Heads" } else { "Tails" };
                    ui.label(egui::RichText::new(side).font(theme::strong_font(theme::chat())).color(text));
                    return;
                }
                let spec = if roll.spec.is_empty() { "1–100".to_string() } else { roll.spec.clone() };
                ui.label(egui::RichText::new(spec.clone()).font(theme::small_font(theme::SMALL)).color(theme::dim(p)));
                let several = roll.rolls.len() > 1 || spec.contains(['+', '-']);
                if several {
                    for r in roll.rolls.iter().take(20) {
                        let g = ui.painter().layout_no_wrap(r.to_string(), theme::small_font(theme::SMALL), text);
                        let (bx, _) = ui.allocate_exact_size(vec2(g.size().x.max(10.0) + 8.0, 18.0), Sense::hover());
                        ui.painter().rect_filled(bx, 0.0, p.window);
                        theme::bevel(ui.painter(), bx, p, Bevel::ThinIn);
                        ui.painter().galley(bx.center() - g.size() / 2.0, g, text);
                    }
                    ui.label(egui::RichText::new("=").font(theme::small_font(theme::SMALL)).color(theme::dim(p)));
                }
                ui.label(egui::RichText::new(roll.total.to_string()).font(theme::strong_font(theme::chat() + 3.0)).color(text));
            });
        });
        r.response.rect
    });
    let rect = out.inner;
    theme::bevel(ui.painter(), rect, p, Bevel::Etched);
    rect
}
