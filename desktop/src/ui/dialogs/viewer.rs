//! A picture full size inside the app (encrypted ones only open here) or a doodle.

use super::*;

/// A picture shown full size in the app: a sealed one from the encrypted
/// Personal space (only this app can open it), or a doodle.
pub struct Viewer {
    pub url: String,
    /// Opens a sealed picture.
    pub key: Option<crate::vault::Key>,
    pub title: String,
    /// The picture's size if known (0 = from the texture once it loads).
    pub size: (f32, f32),
    /// Pixel art (doodles): nearest-neighbour, and blown up to fill the window.
    pub pixel: bool,
    /// Your own doodle reaction: (message id, emoji) to take it back.
    pub remove: Option<(String, String)>,
}

pub(super) fn viewer(app: &mut App, ui: &mut egui::Ui, p: &Pal, acts: &mut Vec<Act>, v: &mut Viewer, keep: &mut bool) {
    let tex = if v.pixel { app.images.get_pixel(&v.url, true) } else { app.images.get_full(&v.url, v.key) };
    if let Some((_, sz)) = tex
        && !crate::ui::chat::sane_size(v.size.0, v.size.1)
    {
        v.size = (sz.x, sz.y);
    }
    let screen = ui.ctx().content_rect();
    let max = vec2((screen.width() * 0.82 - 40.0).max(200.0), (screen.height() * 0.78 - 110.0).max(140.0));
    let (w, h) = if crate::ui::chat::sane_size(v.size.0, v.size.1) { v.size } else { (480.0, 320.0) };
    let grow = if v.pixel { 8.0 } else { 1.0 };
    let s = (max.x / w).min(max.y / h).min(grow);
    let size = vec2(w * s, h * s).round();
    let mut k = true;
    window(ui, p, &v.title, Some("image"), size.x.max(300.0) + 20.0, Some(&mut k), |ui| {
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), size.y), egui::Sense::hover());
        let r = Rect::from_center_size(row.center(), size);
        match tex {
            Some((t, _)) => {
                ui.painter().image(t, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                theme::outline(ui.painter(), r.expand(1.0), p.shadow);
            }
            None => {
                ui.painter().rect_filled(r, 0.0, p.code_bg);
                let failed = !v.pixel && app.images.failed_full(&v.url, v.key.is_some());
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, if failed { "Can't open this picture" } else { "Opening…" }, theme::font(theme::ui()), p.gray);
            }
        }
        if v.key.is_some() {
            note(ui, "Encrypted: only your devices can open it.", p);
        }
        actions(ui, |ui| {
            if primary_button(ui, "Close", p, true).clicked() {
                *keep = false;
            }
            if theme::button_sized(ui, "Save to Downloads", Btn::Normal, tex.is_some(), vec2(130.0, 23.0), p).clicked() {
                acts.push(Act::Cmd(Cmd::SavePicture { url: v.url.clone(), key: v.key }));
            }
            if v.key.is_none() && !v.pixel && theme::button_sized(ui, "Open in browser", Btn::Normal, true, vec2(110.0, 23.0), p).clicked() {
                acts.push(Act::OpenUrl(v.url.clone()));
            }
            if let Some((id, emoji)) = &v.remove
                && theme::button_sized(ui, "Remove my doodle", Btn::Normal, true, vec2(120.0, 23.0), p).clicked()
            {
                acts.push(Act::Cmd(Cmd::React { id: id.clone(), emoji: emoji.clone(), on: false }));
                *keep = false;
            }
        });
    });
    *keep &= k;
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        *keep = false;
    }
}
