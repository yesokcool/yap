//! Watching someone's screen: big in the chat's window (fit to it, black
//! bars, never cropped or stretched), Full screen (or a double-click), Pop
//! out (a window of its own), Stop watching, and the share's sound volume.
//! The words when there's no picture are the web's: "Connecting to Ana's
//! screen…", "Ana's screen is paused." (no picture for 3 s), "Ana stopped
//! sharing." (the view then closes itself).

use super::dialogs::{self, Modal};
use super::theme::{self, Pal, RoundRect};
use super::{Act, App};
use crate::core::{Cmd, Model};
use eframe::egui::{self, Color32, Rect, Sense, pos2, vec2};
use std::time::Duration;

/// No new picture for this long: "paused".
const PAUSED_AFTER: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct ScreenUi {
    tex: Option<egui::TextureHandle>,
    size: [usize; 2],
    /// Whose picture the texture holds.
    sid: Option<String>,
    /// The whole window is the screen.
    pub full: bool,
    /// In a window of its own (the chat comes back here).
    pub popped: bool,
}

impl ScreenUi {
    /// Take the newest decoded picture, if there's one, into the texture.
    pub fn pump(&mut self, ctx: &egui::Context, shared: &crate::core::Shared, m: &Model) {
        let watching = m.screen.watching.as_ref().map(|w| w.sid.clone());
        if watching != self.sid {
            self.sid = watching.clone();
            self.tex = None;
        }
        if watching.is_none() {
            if self.full {
                self.full = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            self.popped = false;
            return;
        }
        // The decoder shrinks what's bigger than this GPU takes; the texture is checked again anyway.
        let max = ctx.input(|i| i.max_texture_side);
        shared.screen.watch.set_max_side(max);
        if let Some(p) = shared.screen.watch.take_picture().filter(|p| p.rgba.len() == p.width as usize * p.height as usize * 4) {
            let img = super::images::fit_texture(egui::ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.rgba), max);
            let size = img.size;
            match self.tex.as_mut() {
                Some(t) if self.size == size => t.set(img, egui::TextureOptions::LINEAR),
                _ => {
                    self.tex = Some(ctx.load_texture("screen-share", img, egui::TextureOptions::LINEAR));
                    self.size = size;
                }
            }
        }
    }

    fn set_full(&mut self, ctx: &egui::Context, on: bool) {
        self.full = on;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
    }
}

/// Is the screen showing in the chat's window (watching, not popped out, not full screen)?
pub fn in_pane(app: &App, m: &Model) -> bool {
    m.screen.watching.is_some() && !app.screen.popped && !app.screen.full
}

/// Leaving the screen's view (another channel, a Place) stops watching,
/// unless it's popped out (like the web).
pub fn leaving(app: &App, m: &Model, acts: &mut Vec<Act>) {
    if m.screen.watching.is_some() && !app.screen.popped {
        acts.push(Act::Cmd(Cmd::StopWatching));
    }
}

fn name(m: &Model) -> String {
    m.screen.watching.as_ref().map(|w| m.user_name(&w.uid)).unwrap_or_default()
}

/// The words over the picture, when there's something to say.
fn state_words(app: &App, m: &Model, last: Option<std::time::Instant>) -> Option<String> {
    let w = m.screen.watching.as_ref()?;
    let name = m.user_name(&w.uid);
    if w.ended.is_some() {
        return Some(format!("{name} stopped sharing."));
    }
    match last {
        None if app.screen.tex.is_none() => Some(format!("Connecting to {name}'s screen…")),
        Some(t) if t.elapsed() > PAUSED_AFTER => Some(format!("{name}'s screen is paused.")),
        _ => None,
    }
}

/// The picture fit into `rect` on black (bars where the shapes differ), the
/// state words over it. A double-click toggles full screen.
fn video(app: &mut App, ui: &mut egui::Ui, rect: Rect, m: &Model, last: Option<std::time::Instant>) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with("screen-video"), Sense::click());
    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
    let words = state_words(app, m, last);
    if let Some(t) = &app.screen.tex
        && m.screen.watching.as_ref().is_some_and(|w| w.ended.is_none())
    {
        let size = t.size_vec2();
        let s = (rect.width() / size.x).min(rect.height() / size.y);
        let r = Rect::from_center_size(rect.center(), size * s).round();
        ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    if let Some(w) = words {
        // A tooltip-yellow note in the middle (98's information-bar look).
        let g = ui.painter().layout(w, theme::bold_font(theme::ui()), theme::INFO_TEXT, (rect.width() - 40.0).max(80.0));
        let box_ = Rect::from_center_size(rect.center(), g.size() + vec2(20.0, 12.0)).round();
        ui.painter().rect_filled(box_, 0.0, theme::INFO);
        theme::outline(ui.painter(), box_, Color32::BLACK);
        ui.painter().galley(box_.center() - g.size() / 2.0, g, theme::INFO_TEXT);
    }
    resp
}

/// The share's sound, apart from their voice (0–100 %, remembered per person here).
fn sound_slider(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, width: f32) {
    let Some(w) = m.screen.watching.as_ref() else { return };
    let mut v = (m.screen_volume(&w.uid) * 100.0).round();
    ui.label(theme::text("Sound", p.text));
    let r = theme::trackbar(ui, &mut v, 0.0..=100.0, 5.0, width, p);
    if theme::tip(r, "How loud their screen's sound plays here (their voice has its own volume)").changed() {
        let (uid, vol) = (w.uid.clone(), v / 100.0);
        acts.push(Act::Config(Box::new(move |c| {
            c.screen_volumes.insert(uid, vol);
        })));
        acts.push(Act::Cmd(Cmd::ScreenVolume { uid: w.uid.clone() }));
    }
    ui.label(theme::text(format!("{}%", v as i32), p.text));
}

/// In the chat's window: a title bar, the buttons and the sound, the picture.
pub fn pane(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let who = name(m);
    let last = app.shared.screen.watch.last_at();
    egui::Panel::top("screen-title").exact_size(theme::TITLE_H).resizable(false).show_separator_line(false).frame(egui::Frame::new()).show(ui, |ui| {
        let rect = ui.max_rect();
        let b = Rect::from_min_size(pos2(rect.right() - 24.0, rect.top() + 2.0), vec2(22.0, 18.0));
        theme::paint_title_upto(ui.painter(), rect, p, Some("monitor"), &format!("{who}'s screen - Yap"), b.left() - 6.0);
        if theme::caption_button(ui, b, "x", "Stop watching", false, p).clicked() {
            acts.push(Act::Cmd(Cmd::StopWatching));
        }
    });
    egui::Panel::top("screen-bar").resizable(false).show_separator_line(false).frame(egui::Frame::new().inner_margin(egui::Margin { left: 4, right: 4, top: 4, bottom: 3 })).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if theme::tip(theme::button(ui, "Pop out", theme::Btn::Normal, true, p), "Keep watching in a window of its own while you use the rest of Yap").clicked() {
                app.screen.popped = true;
            }
            if theme::tip(theme::button(ui, "Full screen", theme::Btn::Normal, true, p), "Full screen (or double-click the picture; Esc to leave)").clicked() {
                let ctx = ui.ctx().clone();
                app.screen.set_full(&ctx, true);
            }
            if theme::button(ui, "Stop watching", theme::Btn::Normal, true, p).clicked() {
                acts.push(Act::Cmd(Cmd::StopWatching));
            }
            ui.add_space(8.0);
            let w = (ui.available_width() - 90.0).clamp(60.0, 160.0);
            sound_slider(ui, m, p, acts, w);
        });
    });
    egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin { left: 2, right: 2, top: 2, bottom: 2 })).show(ui, |ui| {
        let rect = ui.max_rect();
        theme::bevel(ui.painter(), rect, p, theme::Bevel::Sunken);
        let inner = rect.shrink(2.0);
        if video(app, ui, inner, m, last).double_clicked() {
            let ctx = ui.ctx().clone();
            app.screen.set_full(&ctx, true);
        }
    });
}

/// Full screen: the picture is the whole window (Esc or a double-click leaves).
pub fn fullscreen(app: &mut App, ui: &mut egui::Ui, m: &Model) {
    let last = app.shared.screen.watch.last_at();
    let ctx = ui.ctx().clone();
    let rect = ui.max_rect();
    let resp = video(app, ui, rect, m, last);
    // A quiet hint along the bottom while the pointer moves.
    if ctx.input(|i| i.pointer.time_since_last_movement() < 2.0) {
        let hint = "Esc or double-click to leave full screen";
        let g = ui.painter().layout_no_wrap(hint.into(), theme::font(theme::small()), Color32::from_gray(200));
        let at = pos2(rect.center().x - g.size().x / 2.0, rect.bottom() - 28.0);
        ui.painter().rect_filled(Rect::from_min_size(at - vec2(8.0, 4.0), g.size() + vec2(16.0, 8.0)), 0.0, Color32::from_black_alpha(160));
        ui.painter().galley(at, g, Color32::from_gray(200));
        ctx.request_repaint_after(Duration::from_millis(500));
    }
    if resp.double_clicked() || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.screen.set_full(&ctx, false);
    }
}

/// Popped out: a window of its own with the picture, Pop in and Stop watching.
pub fn popout(app: &mut App, ctx: &egui::Context, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    if !app.screen.popped || m.screen.watching.is_none() {
        return;
    }
    let who = name(m);
    let last = app.shared.screen.watch.last_at();
    let mut close = false;
    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("yap-screen-popout"),
        egui::ViewportBuilder::default().with_title(format!("{who}'s screen - Yap")).with_inner_size([960.0, 600.0]).with_min_inner_size([320.0, 200.0]),
        |ui, _class| {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(p.face).inner_margin(3)).show(ui, |ui| {
                egui::Panel::top("screen-pop-bar").resizable(false).show_separator_line(false).frame(egui::Frame::new().inner_margin(egui::Margin { left: 2, right: 2, top: 2, bottom: 4 })).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if theme::tip(theme::button(ui, "Pop in", theme::Btn::Normal, true, p), "Back into Yap's window").clicked() {
                            close = true;
                        }
                        if theme::button(ui, "Stop watching", theme::Btn::Normal, true, p).clicked() {
                            acts.push(Act::Cmd(Cmd::StopWatching));
                        }
                        ui.add_space(8.0);
                        sound_slider(ui, m, p, acts, 140.0);
                    });
                });
                egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
                    let rect = ui.max_rect();
                    theme::bevel(ui.painter(), rect, p, theme::Bevel::Sunken);
                    video(app, ui, rect.shrink(2.0), m, last);
                });
            });
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                close = true;
            }
        },
    );
    if close {
        app.screen.popped = false;
    }
}

/// Share screen (or Share something else, while sharing). On macOS 14+ what's shared is
/// picked in Apple's own picker: Share screen still opens ours first (the quality and the
/// sound), Share something else goes straight to Apple's, for the running share.
pub fn open_share_dialog(app: &mut App, m: &Model, switching: bool) {
    if switching && crate::screen::capture::system_picker() {
        if !crate::screen::capture::pick_again() {
            let (quality, sound) = m.screen.sharing.as_ref().map(|s| (s.quality, s.sound)).unwrap_or((m.config.share_quality, m.config.share_sound));
            app.pick_screen(quality, sound, true);
        }
        return;
    }
    app.modal = Some(Modal::Share(dialogs::ShareDialog::new(&m.config, switching)));
}

/// "Watching: Ana, Ben" / "No one's watching yet".
pub fn watchers_line(m: &Model) -> String {
    let names: Vec<String> = m.screen.watchers.iter().filter_map(|s| m.sessions.get(s)).map(|s| m.user_name(&s.uid)).collect();
    if names.is_empty() { "No one's watching yet".into() } else { format!("Watching: {}", names.join(", ")) }
}
