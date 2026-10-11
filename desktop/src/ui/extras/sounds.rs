//! The soundboard popover.

use super::*;

/// "▶ Ada played 📯 Airhorn", for a few seconds after a play.
pub fn last_sound(m: &Model) -> Option<&str> {
    m.last_sound.as_ref().filter(|(_, at)| at.elapsed().as_secs() < 5).map(|(t, _)| t.as_str())
}

/// The soundboard: the server's clips as tiles; click plays it for everyone
/// in your voice channel. Right-click: hear it here / delete it.
pub fn soundboard_popover(app: &mut App, ctx: &egui::Context, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let Some(pos) = app.soundboard else { return };
    let mut close = false;
    let moderator = m.am_moderator();
    let area = egui::Area::new(egui::Id::new("soundboard")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        super::super::chat::picker_window(ui, "Soundboard", 330.0, p, |ui| {
            let mut board: Vec<&crate::protocol::Sound> = m.sounds.iter().filter(|s| s.kind == "board").collect();
            board.sort_by_key(|s| s.name.to_lowercase());
            let in_voice = m.voice_channel.is_some();
            if !in_voice {
                dialogs::note(ui, "Join a voice channel to play sounds for the people in it.", p);
            }
            if m.config.sound_mute {
                dialogs::note(ui, "You've muted the soundboard (Settings › Voice): others' plays are silent here.", p);
            }
            if board.is_empty() {
                ui.label(theme::text("No sounds yet. Add the first one!", theme::dim(p)));
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for s in board {
                    let (rect, resp) = ui.allocate_exact_size(vec2(74.0, 52.0), if in_voice { Sense::click() } else { Sense::hover() });
                    let hovered = resp.hovered() && in_voice;
                    ui.painter().rect_filled(rect, 0.0, p.face);
                    theme::bevel(ui.painter(), rect, p, if resp.is_pointer_button_down_on() { Bevel::Pressed } else { Bevel::Raised });
                    let fg = p.text;
                    // The emoji, if the font has it (egui's emoji font misses newer ones), else a note.
                    let font = egui::FontId::proportional(18.0);
                    match s.emoji.clone().filter(|e| ui.fonts_mut(|f| f.has_glyphs(&font, e))) {
                        Some(e) => {
                            ui.painter().text(rect.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, e, font, fg);
                        }
                        None => icons::paint(ui.painter(), Rect::from_center_size(rect.center() - vec2(0.0, 8.0), vec2(18.0, 18.0)), "music", theme::accent(p)),
                    }
                    let mut job = egui::text::LayoutJob::simple_singleline(s.name.clone(), theme::small_font(theme::SMALL), fg);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 8.0);
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(rect.center().x - g.size().x / 2.0, rect.bottom() - 6.0 - g.size().y), g, fg);
                    if hovered {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let tip = format!("{} · {:.1} s · by {}{}", s.name, s.ms as f32 / 1000.0, m.user_name(&s.uid), if in_voice { "\nClick: play it for everyone in your voice channel" } else { "" });
                    let resp = theme::tip(resp, tip);
                    if resp.clicked() && in_voice {
                        acts.push(Act::Cmd(Cmd::PlaySound(s.id.clone())));
                    }
                    let (id, mine) = (s.id.clone(), s.uid == m.config.uid);
                    theme::context_menu(&resp, p, |ui| {
                        if theme::menu_item(ui, "Hear it here only", false, p).clicked() {
                            acts.push(Act::Cmd(Cmd::PreviewSound(crate::core::Preview::Sound(id.clone()))));
                            ui.close();
                        }
                        if (mine || moderator) && theme::menu_item(ui, "Delete this sound", false, p).clicked() {
                            acts.push(Act::Cmd(Cmd::DeleteSound(id.clone())));
                            ui.close();
                        }
                    });
                }
            });
            if let Some(t) = last_sound(m) {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(format!("▶ {t}")).font(theme::small_font(theme::SMALL)).color(theme::dim(p)));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if theme::button_sized(ui, "Add a sound…", theme::Btn::Normal, m.sounds.iter().filter(|s| s.kind == "board").count() < 60, vec2(100.0, 23.0), p).clicked() {
                    acts.push(Act::PickFile(super::super::PickFor::Sound { chime: false }));
                    close = true;
                }
                ui.label(egui::RichText::new("Right-click a sound for more.").font(theme::small_font(theme::SMALL)).color(theme::dim(p)));
            });
        });
    });
    let menu_open = egui::Popup::is_any_open(ctx);
    let outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|q| !area.response.rect.contains(q));
    if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) || (outside && !menu_open && app.soundboard_opened + 0.2 < ctx.input(|i| i.time)) {
        app.soundboard = None;
    }
}

/// Open (or close) the soundboard above `anchor`.
pub fn toggle_soundboard(app: &mut App, ctx: &egui::Context, anchor: Rect) {
    if app.soundboard.is_some() {
        app.soundboard = None;
    } else {
        app.soundboard = Some(anchor.left_bottom() + vec2(0.0, 4.0));
        app.soundboard_opened = ctx.input(|i| i.time);
    }
}
