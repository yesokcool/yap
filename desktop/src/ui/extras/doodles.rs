//! Doodle reactions: a tiny wiggly drawing as a reaction.

use super::*;

/// `<d:ID>` → the doodle's media id.
pub fn doodle_token(s: &str) -> Option<&str> {
    let id = s.strip_prefix("<d:")?.strip_suffix('>')?;
    (id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())).then_some(id)
}

/// A doodle reaction, drawn about two lines tall; returns the click.
#[allow(clippy::too_many_arguments)]
pub fn doodle_reaction(app: &mut App, ui: &mut egui::Ui, m: &Model, msg: &Message, emoji: &str, id: &str, uids: &[String], p: &Pal) -> egui::Response {
    let side = 40.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(side + 6.0, side + 6.0), Sense::click());
    let url = format!("{}/media/{id}", m.config.server);
    let mine = uids.contains(&m.config.uid);
    ui.painter().rect_filled(rect, 0.0, p.face);
    theme::bevel(ui.painter(), rect, p, if mine { Bevel::Pressed } else { Bevel::Raised });
    let inner = rect.shrink(3.0);
    match app.images.get_pixel(&url, resp.hovered()) {
        Some((tex, sz)) => {
            let s = (inner.width() / sz.x).min(inner.height() / sz.y);
            ui.painter().image(tex, Rect::from_center_size(inner.center(), sz * s), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(inner, 0.0, p.code_bg);
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let names: Vec<String> = uids.iter().map(|u| m.user_name(u)).collect();
    let resp = theme::tip(resp, format!("Doodle by {} — click to see it big", names.join(", ")));
    if resp.clicked() {
        app.modal = Some(dialogs::Modal::Viewer(dialogs::Viewer {
            url,
            key: None,
            title: format!("Doodle by {}", names.join(", ")),
            size: (0.0, 0.0),
            pixel: true,
            remove: mine.then(|| (msg.id.clone(), emoji.to_string())),
        }));
    }
    resp
}

/// The "Draw one…" entry for the reaction picker: opens the doodle box.
pub fn open_doodle(app: &mut App, message: &str) {
    app.react_picker = None;
    app.modal = Some(dialogs::Modal::Doodle(super::super::doodle::DoodleBox::new(message.to_string())));
}
