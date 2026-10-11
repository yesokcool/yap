//! The Gallery: Draw's saved drawings on the walls, paging, and the viewer.

use super::*;

/// The saved drawings worth hanging, newest first.
pub(super) fn gallery(m: &Model) -> Vec<&SavedDrawing> {
    m.space.gallery.iter().filter(|d| hangable(d)).take(60).collect()
}

pub(super) fn pages(n: usize) -> usize {
    n.div_ceil(PER_WALL).max(1)
}

/// The Gallery's walls: twelve frames, the drawings in them with crisp pixels
/// (pixel art), a name plate under each, and hard-edged beams of light.
pub(super) fn gallery_walls(pen: &Pen, env: &Env) {
    let g = &env.wall;
    for f in FRAMES.iter() {
        let cx = f.x + f.w / 2.0;
        let toward = if f.row == 1 { -1.0 } else { 1.0 }; // which way the light falls: into the aisle
        let wall_y = if f.row == 1 { f.y + f.h + ART_PLATE + 6.0 } else { f.y - 6.0 };
        let reach = f.y + if f.row == 1 { -40.0 } else { f.h + ART_PLATE + 40.0 };
        let beam = [(cx - 24.0, wall_y), (cx + 24.0, wall_y), (f.x + f.w + 14.0, reach), (f.x - 14.0, reach)];
        pen.poly(&beam, rgba(255, 250, 210, 0.13));
        // The little lamp on the wall.
        pen.fill(cx - 12.0, wall_y - if toward > 0.0 { 6.0 } else { 0.0 }, 24.0, 6.0, rgb(0x16141f));
        pen.disc(cx, wall_y + toward * 2.0, 3.0, rgb(0xffe9a8));
    }
    let first_empty = g.items.len();
    for f in FRAMES.iter() {
        let Some(item) = g.items.get(f.i) else {
            pen.dashed_rect(f.x + 1.0, f.y + 1.0, f.w - 2.0, f.h - 2.0, 6.0, 5.0, Stroke::new(2.0, Color32::from_white_alpha(56)));
            if f.i == first_empty {
                let c = Color32::from_white_alpha(140);
                if g.total > 0 {
                    pen.text(f.x + f.w / 2.0, f.y + f.h / 2.0, Align2::CENTER_CENTER, "more soon…", 10.0, true, c);
                } else {
                    pen.text(f.x + f.w / 2.0, f.y + f.h / 2.0 - 6.0, Align2::CENTER_CENTER, "Save a drawing", 10.0, true, c);
                    pen.text(f.x + f.w / 2.0, f.y + f.h / 2.0 + 8.0, Align2::CENTER_CENTER, "in Draw ✏", 10.0, true, c);
                }
            }
            continue;
        };
        // The frame: dark wood with a thin gold lip, then the drawing itself.
        pen.fill(f.x, f.y, f.w, f.h, rgb(0x2b2018));
        pen.outline(f.x + 2.5, f.y + 2.5, f.w - 5.0, f.h - 5.0, Stroke::new(1.0, rgb(0xc9a227)));
        let (ax, ay) = (f.x + ART_BORDER, f.y + ART_BORDER);
        pen.fill(ax, ay, ART_W, ART_H, Color32::WHITE);
        if pen.seen(ax, ay, ART_W, ART_H)
            && let Some((tex, _)) = env.images.get_pixel(&format!("{}/media/{}", env.m.config.server, item.media), false)
        {
            pen.p.image(tex, pen.rect(ax, ay, ART_W, ART_H), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        if g.focus == f.i as i32 {
            pen.outline(f.x - 3.0, f.y - 3.0, f.w + 6.0, f.h + 6.0, Stroke::new(3.0, rgb(0xffd166)));
        }
        // Name plate: the artist in their colour, and the date.
        let who = env.m.users.get(&item.uid);
        let py = f.y + f.h + 1.0;
        pen.fill(f.x + 6.0, py, f.w - 12.0, ART_PLATE - 1.0, rgb(0x15131c));
        let name = who.map(|u| u.name.as_str()).unwrap_or("Someone");
        let name = if name.chars().count() > 11 { format!("{}…", name.chars().take(10).collect::<String>()) } else { name.to_string() };
        let color = who.map(|u| theme::hex_color(&u.color)).unwrap_or(rgb(0xdddddd));
        pen.text(f.x + 9.0, py + ART_PLATE / 2.0, Align2::LEFT_CENTER, name, 9.0, true, color);
        let when = if item.ts > 0 { super::super::local_time(item.ts).format("%b %-d").to_string() } else { String::new() };
        pen.text(f.x + f.w - 9.0, py + ART_PLATE / 2.0, Align2::RIGHT_CENTER, when, 9.0, false, rgb(0x9a96a8));
    }
    // Which wall you're looking at, at both ends of the aisle.
    if g.pages > 1 {
        let mid_y = FRAMES[0].look.y + (FRAMES[6].y - FRAMES[0].look.y) / 2.0;
        for x in [GALLERY.x + T + 20.0, GALLERY.x + GALLERY.w - T - 20.0] {
            pen.text(x, mid_y, Align2::CENTER_CENTER, format!("{}/{}", g.page + 1, g.pages), 11.0, true, Color32::from_white_alpha(178));
        }
    }
}

/// The big viewer: the real image (GIFs play), ‹ › through every drawing,
/// who made it and when; Esc or ✕ closes it.
#[allow(clippy::too_many_arguments)]
pub(super) fn viewer(st: &mut SpaceUi, ui: &mut egui::Ui, m: &Model, p: &Pal, view: Rect, list: &[&SavedDrawing], images: &super::super::images::Images, h12: bool) {
    let Some(i) = st.viewing_index(list) else { return };
    let d = list[i];
    let who = m.users.get(&d.uid);
    let name = who.map(|u| u.name.clone()).unwrap_or_else(|| "Someone".into());
    let url = format!("{}/media/{}", m.config.server, d.media);
    let tex = images.get_pixel(&url, true);
    // Like the web: at most 700 wide, the page's 8:5 box, ‹ › either side.
    let width = (view.width() - 40.0).clamp(240.0, 700.0);
    let nav = vec2(24.0, 64.0);
    let box_w = width - 20.0 - (nav.x + 4.0) * 2.0;
    let box_size = vec2(box_w, (box_w * 5.0 / 8.0).min(view.height() - 150.0).max(80.0));
    let mut keep = true;
    let mut step = 0i64;
    // Centred on the world (not the whole window).
    egui::Area::new(ui.id().with("space-viewer")).order(egui::Order::Foreground).pivot(Align2::CENTER_CENTER).fixed_pos(view.center()).constrain_to(view).show(ui.ctx(), |ui| {
        super::super::dialogs::dialog_frame(p).show(ui, |ui| {
            super::super::dialogs::window(ui, p, &format!("Gallery · {name}"), Some("image"), width, Some(&mut keep), |ui| {
                // The row's height up front, so the arrows sit in its middle.
                ui.allocate_ui_with_layout(vec2(ui.available_width(), box_size.y), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let arrow = |ui: &mut egui::Ui, glyph: &str, on: bool, tip: &str| {
                        let r = theme::button_sized(ui, "", theme::Btn::Normal, on, nav, p);
                        let c = if on { p.text } else { p.gray };
                        ui.painter().text(r.rect.center(), Align2::CENTER_CENTER, glyph, egui::FontId::proportional(18.0), c);
                        theme::tip(r, tip).clicked() && on
                    };
                    if arrow(ui, "‹", i > 0, "Newer (←)") {
                        step = -1;
                    }
                    let (r, _) = ui.allocate_exact_size(box_size, Sense::hover());
                    theme::panel(ui.painter(), r, p, Color32::WHITE, theme::Bevel::Sunken);
                    match tex {
                        Some((t, natural)) => {
                            // object-fit: contain, crisp pixels.
                            let inner = r.shrink(2.0);
                            let s = (inner.width() / natural.x).min(inner.height() / natural.y);
                            let img = Rect::from_center_size(inner.center(), natural * s);
                            ui.painter().image(t, img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        None => {
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, "Loading…", theme::font(theme::ui()), Color32::GRAY);
                        }
                    }
                    if arrow(ui, "›", i + 1 < list.len(), "Older (→)") {
                        step = 1;
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.5, who.map(|u| theme::hex_color(&u.color)).unwrap_or(Color32::GRAY));
                    ui.label(egui::RichText::new(&name).font(theme::bold_font(theme::ui())).color(p.text));
                    if d.ts > 0 {
                        let when = format!("{}, {}", super::super::local_time(d.ts).format("%b %-d, %Y"), super::super::fmt_time(d.ts, h12));
                        ui.label(egui::RichText::new(when).font(theme::font(theme::small())).color(p.gray));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("{} of {}", i + 1, list.len())).font(theme::font(theme::small())).color(p.gray));
                    });
                });
            });
        });
    });
    if !keep {
        st.close_viewer(true);
    } else if step != 0 {
        st.show_drawing(list, i as i64 + step);
    }
}

impl SpaceUi {
    pub(super) fn viewing_index(&self, list: &[&SavedDrawing]) -> Option<usize> {
        let id = self.viewing.as_ref()?;
        list.iter().position(|d| &d.id == id)
    }

    /// The drawing hanging in a frame on the wall you're looking at.
    pub(super) fn drawing_in(&self, frame: &Frame, n: usize) -> Option<usize> {
        let i = self.page * PER_WALL + frame.i;
        (i < n).then_some(i)
    }

    /// Stop in front of a drawing and it opens; walk away and it closes.
    pub(super) fn step_gallery(&mut self, c: &Ctx, list: &[&SavedDrawing]) {
        let Some((x, y, _)) = self.me else { return };
        let n = list.len();
        self.page = self.page.min(pages(n) - 1);
        if self.viewing.is_some() {
            let far = self.view_anchor.is_none_or(|a| (x - a.0).hypot(y - a.1) > 110.0);
            if zone_of(x, y) != GALLERY_ZONE || far || self.viewing_index(list).is_none() {
                self.close_viewer(false);
            }
            return;
        }
        let frame = if zone_of(x, y) == GALLERY_ZONE { frame_at(x, y) } else { None };
        let Some((frame, index)) = frame.and_then(|f| self.drawing_in(f, n).map(|i| (f, i))) else {
            self.dwell.0 = -1;
            self.dismissed = -1;
            return;
        };
        if frame.i as i32 != self.dwell.0 || self.vel != (0.0, 0.0) {
            self.dwell = (frame.i as i32, c.now);
        }
        if frame.i as i32 != self.dismissed && seat_at(x, y).is_none() && c.now - self.dwell.1 >= LOOK_MS {
            self.open_viewer(list, index, frame);
        }
    }

    /// E: look at the drawing in front of you, unless a seat is right there.
    pub(super) fn look_or_sit(&mut self, c: &mut Ctx, list: &[&SavedDrawing]) {
        if let Some((x, y, _)) = self.me
            && seat_at(x, y).is_none()
            && zone_of(x, y) == GALLERY_ZONE
        {
            let near = SEATS.iter().any(|s| (s.0 - x).hypot(s.1 - y) < 50.0);
            if let Some(frame) = frame_at(x, y)
                && let Some(index) = self.drawing_in(frame, list.len())
                && !near
            {
                return self.open_viewer(list, index, frame);
            }
        }
        self.toggle_sit(c);
    }

    pub(super) fn open_viewer(&mut self, list: &[&SavedDrawing], index: usize, frame: &Frame) {
        self.view_anchor = self.me.map(|(x, y, _)| (x, y));
        self.dismissed = frame.i as i32;
        self.show_drawing(list, index as i64);
    }

    pub(super) fn show_drawing(&mut self, list: &[&SavedDrawing], index: i64) {
        if list.is_empty() {
            return;
        }
        let i = index.clamp(0, list.len() as i64 - 1) as usize;
        self.viewing = Some(list[i].id.clone());
        // Keep the walls on the same twelve as the viewer.
        self.page = i / PER_WALL;
    }

    /// `by_hand`: Esc or ✕ — then it stays shut until you step to another drawing.
    pub(super) fn close_viewer(&mut self, by_hand: bool) {
        if self.viewing.is_none() {
            return;
        }
        self.viewing = None;
        self.view_anchor = None;
        if !by_hand {
            self.dismissed = -1;
        }
    }
}
