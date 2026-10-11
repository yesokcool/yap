//! Windows 98 / AIM, in egui — the same tokens as web/style.css: grey
//! 3D-bevelled windows on a teal desktop, navy gradient title bars, white text
//! wells, Tahoma, square corners and no soft shadows (bevels only). Dark
//! ("Charcoal", the default) is a period colour scheme; light is "Windows
//! Standard".
//!
//! egui's own widgets are mostly replaced by small 98-style ones here
//! (buttons, check boxes, radio buttons, fields, combo boxes, trackbars,
//! property-sheet tabs, group boxes), because its visuals can't draw bevels.

use eframe::egui::emath::GuiRounding;
use eframe::egui::{self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, Response, Sense, Stroke, TextStyle, Ui, Vec2, pos2, vec2};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    pub desktop: Color32,
    pub face: Color32,
    pub hilight: Color32,
    pub light: Color32,
    pub shadow: Color32,
    pub dk: Color32,
    pub window: Color32,
    pub text: Color32,
    pub gray: Color32,
    pub title_a: Color32,
    pub title_b: Color32,
    pub title_text: Color32,
    pub select: Color32,
    pub on_select: Color32,
    pub link: Color32,
    /// Your own name in the chat log.
    pub me: Color32,
    /// Everyone else's.
    pub them: Color32,
    pub ok: Color32,
    pub danger: Color32,
    pub warn: Color32,
    pub mention_bg: Color32,
    pub code_bg: Color32,
    pub meter: Color32,
}

/// Tooltip / information-bar yellow (both schemes), with black text.
pub const INFO: Color32 = rgb(0xffffe1);
pub const INFO_TEXT: Color32 = Color32::BLACK;
/// Unread-mention badges.
pub const BADGE: Color32 = rgb(0xc00000);

/// Chrome text (labels, buttons, lists).
pub const UI: f32 = 12.0;
/// Chat text.
pub const CHAT: f32 = 13.0;
/// Small print.
pub const SMALL: f32 = 11.0;
/// Title-bar height.
pub const TITLE_H: f32 = 22.0;
/// Scroll-bar width.
pub const BAR: f32 = 16.0;

/// Load the look: Windows 98's fonts and style.
pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    install_style(ctx);
}

/// Chrome text size (shared views).
pub fn ui() -> f32 {
    UI
}

/// Message text.
pub fn chat() -> f32 {
    CHAT
}

pub fn small() -> f32 {
    SMALL
}

/// Snap a rect to whole points (crisp 1-pt bevels).
pub trait RoundRect {
    fn round(self) -> Self;
}

impl RoundRect for Rect {
    fn round(self) -> Self {
        Rect::from_min_max(self.min.round(), self.max.round())
    }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// "Charcoal" (the default).
pub fn dark() -> Pal {
    Pal {
        dark: true,
        desktop: rgb(0x0b2a2a),
        face: rgb(0x3a3a3a),
        hilight: rgb(0x707070),
        light: rgb(0x525252),
        shadow: rgb(0x1f1f1f),
        dk: rgb(0x000000),
        window: rgb(0x1c1c1c),
        text: rgb(0xe6e6e6),
        gray: rgb(0x8a8a8a),
        title_a: rgb(0x000080),
        title_b: rgb(0x1084d0),
        title_text: rgb(0xffffff),
        select: rgb(0x0b3d91),
        on_select: rgb(0xffffff),
        link: rgb(0x8ab4ff),
        me: rgb(0xff7070),
        them: rgb(0x7fb2ff),
        ok: rgb(0x3fd13f),
        danger: rgb(0xff6060),
        warn: rgb(0xe0b030),
        mention_bg: rgb(0x3b3720),
        code_bg: rgb(0x262626),
        meter: rgb(0x1084d0),
    }
}

/// "Windows Standard".
pub fn light() -> Pal {
    Pal {
        dark: false,
        desktop: rgb(0x008080),
        face: rgb(0xc0c0c0),
        hilight: rgb(0xffffff),
        light: rgb(0xdfdfdf),
        shadow: rgb(0x808080),
        dk: rgb(0x000000),
        window: rgb(0xffffff),
        text: rgb(0x000000),
        gray: rgb(0x808080),
        title_a: rgb(0x000080),
        title_b: rgb(0x1084d0),
        title_text: rgb(0xffffff),
        select: rgb(0x000080),
        on_select: rgb(0xffffff),
        link: rgb(0x0000ff),
        me: rgb(0xff0000),
        them: rgb(0x0000ff),
        ok: rgb(0x008000),
        danger: rgb(0xc00000),
        warn: rgb(0x806000),
        mention_bg: rgb(0xffffe1),
        code_bg: rgb(0xf0f0f0),
        meter: rgb(0x000080),
    }
}

pub fn pal(ctx: &egui::Context) -> Pal {
    if ctx.global_style().visuals.dark_mode { dark() } else { light() }
}

/// A picture with rounded corners (round avatars in Space).
pub fn image_rounded(painter: &egui::Painter, rect: Rect, radius: f32, tex: egui::TextureId) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.add(egui::epaint::RectShape::filled(rect, egui::CornerRadius::same(radius.clamp(0.0, 255.0) as u8), Color32::WHITE).with_texture(tex, uv));
}

/// Parse "#rrggbb".
pub fn hex_color(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    if s.len() == 6
        && let Ok(v) = u32::from_str_radix(s, 16)
    {
        return rgb(v);
    }
    rgb(0x808080)
}

// ── Type ───────────────────────────────────────────────────────────────

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, bold())
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

pub fn text(s: impl Into<String>, color: Color32) -> egui::RichText {
    egui::RichText::new(s).font(font(ui())).color(color)
}

/// A tooltip: pale yellow, black ring, black text (in both schemes).
pub fn tip(resp: Response, text: impl Into<String>) -> Response {
    let text = text.into();
    if text.is_empty() {
        return resp;
    }
    resp.on_hover_ui(|ui| {
        ui.set_max_width(320.0);
        ui.label(egui::RichText::new(text).font(font(SMALL + 0.5)).color(INFO_TEXT));
    })
}

// ── Shared looks (the extras' cards, accents, small print) ──

/// A small card's look: (fill, border, radius).
pub fn card_look(p: &Pal) -> (Color32, Stroke, f32) {
    (p.face, Stroke::NONE, 0.0)
}

/// The accent: the link blue.
pub fn accent(p: &Pal) -> Color32 {
    p.link
}

/// The quieter text colour.
pub fn dim(p: &Pal) -> Color32 {
    p.gray
}

/// Small print at `size`.
pub fn small_font(size: f32) -> FontId {
    font(size)
}

/// Bold at `size`.
pub fn strong_font(size: f32) -> FontId {
    bold_font(size)
}

// ── Bevels ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Bevel {
    /// Buttons.
    Raised,
    /// Buttons held down / toggled on.
    Pressed,
    /// Window frames, menus, dialogs.
    Window,
    /// Fields, lists, wells.
    Sunken,
    /// Status-bar panels, meters.
    ThinIn,
    /// Toolbar buttons under the pointer.
    ThinOut,
    /// Group boxes and separators.
    Etched,
}

/// One 1-pt ring inside `r`: top + left in `tl`, bottom + right in `br`.
fn ring(painter: &egui::Painter, r: Rect, tl: Color32, br: Color32) {
    painter.rect_filled(Rect::from_min_max(r.min, pos2(r.right() - 1.0, r.top() + 1.0)), 0.0, tl);
    painter.rect_filled(Rect::from_min_max(r.min, pos2(r.left() + 1.0, r.bottom() - 1.0)), 0.0, tl);
    painter.rect_filled(Rect::from_min_max(pos2(r.left(), r.bottom() - 1.0), r.max), 0.0, br);
    painter.rect_filled(Rect::from_min_max(pos2(r.right() - 1.0, r.top()), r.max), 0.0, br);
}

/// Light comes from the top-left; two 1-pt rings (one for the thin kinds).
pub fn bevel(painter: &egui::Painter, rect: Rect, p: &Pal, kind: Bevel) {
    let r = rect.round_to_pixels(painter.pixels_per_point());
    let inner = r.shrink(1.0);
    match kind {
        Bevel::Raised => {
            ring(painter, r, p.hilight, p.dk);
            ring(painter, inner, p.light, p.shadow);
        }
        Bevel::Pressed => {
            ring(painter, r, p.dk, p.hilight);
            ring(painter, inner, p.shadow, p.light);
        }
        Bevel::Window => {
            ring(painter, r, p.light, p.dk);
            ring(painter, inner, p.hilight, p.shadow);
        }
        Bevel::Sunken => {
            ring(painter, r, p.shadow, p.hilight);
            ring(painter, inner, p.dk, p.light);
        }
        Bevel::ThinIn => ring(painter, r, p.shadow, p.hilight),
        Bevel::ThinOut => ring(painter, r, p.hilight, p.shadow),
        Bevel::Etched => {
            ring(painter, r, p.shadow, p.hilight);
            ring(painter, inner, p.hilight, p.shadow);
        }
    }
}

/// Fill + bevel.
pub fn panel(painter: &egui::Painter, rect: Rect, p: &Pal, fill: Color32, kind: Bevel) {
    painter.rect_filled(rect.round_to_pixels(painter.pixels_per_point()), 0.0, fill);
    bevel(painter, rect, p, kind);
}

/// A 1-pt black ring (default buttons, badges, tooltips).
pub fn outline(painter: &egui::Painter, rect: Rect, color: Color32) {
    ring(painter, rect.round_to_pixels(painter.pixels_per_point()), color, color);
}

/// An etched horizontal rule (2 pt: shadow over hilight).
pub fn etched_hline(painter: &egui::Painter, x: egui::Rangef, y: f32, p: &Pal) {
    let y = y.round();
    painter.rect_filled(Rect::from_x_y_ranges(x, y..=y + 1.0), 0.0, p.shadow);
    painter.rect_filled(Rect::from_x_y_ranges(x, y + 1.0..=y + 2.0), 0.0, p.hilight);
}

/// The 50% checkerboard of `color` over whatever is below (toggled buttons, scroll tracks).
pub fn dither(ui: &Ui, rect: Rect, color: Color32) {
    let id = egui::Id::new("yap-dither");
    let tex = match ui.ctx().data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        Some(t) => t,
        None => {
            let w = [255u8, 255, 255, 255];
            let c = [0u8, 0, 0, 0];
            let px: Vec<u8> = [w, c, c, w].concat();
            let t = ui.ctx().load_texture("dither", egui::ColorImage::from_rgba_unmultiplied([2, 2], &px), egui::TextureOptions::NEAREST_REPEAT);
            ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
            t
        }
    };
    let rect = rect.round_to_pixels(ui.painter().pixels_per_point());
    let uv = Rect::from_min_size(pos2(0.0, 0.0), rect.size() / 2.0);
    ui.painter().image(tex.id(), rect, uv, color);
}

/// Horizontal gradient (title bars).
pub fn gradient(painter: &egui::Painter, rect: Rect, a: Color32, b: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), a);
    mesh.colored_vertex(rect.right_top(), b);
    mesh.colored_vertex(rect.right_bottom(), b);
    mesh.colored_vertex(rect.left_bottom(), a);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

// ── Windows ────────────────────────────────────────────────────────────

/// Gap between the windows on the desktop.
pub const GAP: i8 = 6;
/// Padding between a window's bevel and its contents.
pub const PAD: i8 = 3;

/// A window on the desktop: face fill; call [`window_bevel`] inside.
pub fn window_frame(p: &Pal, outer: egui::Margin) -> egui::Frame {
    egui::Frame::new().fill(p.face).inner_margin(PAD).outer_margin(outer)
}

/// Paint the window frame bevel around the content rect of a [`window_frame`].
pub fn window_bevel(ui: &Ui, p: &Pal) {
    bevel(ui.painter(), ui.max_rect().expand(PAD as f32), p, Bevel::Window);
}

/// Paint a title bar (navy when `active`, grey otherwise).
/// Every window's title bar looks active (navy): swapping navy / grey with
/// focus made the sidebar's bar flash on every click (changed 2026-10-05).
pub fn paint_title(painter: &egui::Painter, rect: Rect, p: &Pal, icon: Option<&str>, title: &str) {
    paint_title_upto(painter, rect, p, icon, title, rect.right());
}

/// [`paint_title`] with the title cut short with "…" at `text_right`
/// (caption buttons sit past it). Returns where the title's text ends.
pub fn paint_title_upto(painter: &egui::Painter, rect: Rect, p: &Pal, icon: Option<&str>, title: &str, text_right: f32) -> f32 {
    let rect = rect.round_to_pixels(painter.pixels_per_point());
    let (a, b, fg) = (p.title_a, p.title_b, p.title_text);
    gradient(painter, rect, a, b);
    let mut x = rect.left() + 4.0;
    if let Some(i) = icon {
        super::icons::paint(painter, Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(13.0, 13.0)), i, fg);
        x += 18.0;
    }
    let g = elide(painter, title, bold_font(UI), fg, text_right.min(rect.right()) - x);
    let end = x + g.size().x;
    painter.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
    end
}

/// One line of text, cut short with "…" past `max_w`.
pub fn elide(painter: &egui::Painter, text: impl Into<String>, font: FontId, color: Color32, max_w: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.into(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(1.0));
    painter.layout_job(job)
}

/// A full-width title bar at the top of `ui`.
pub fn title_bar(ui: &mut Ui, p: &Pal, icon: Option<&str>, title: &str) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), TITLE_H), Sense::hover());
    paint_title(ui.painter(), rect, p, icon, title);
    rect
}

/// A caption button (the _ □ × boxes) at `rect`, raised, with an icon.
pub fn caption_button(ui: &mut Ui, rect: Rect, icon: &str, tooltip: &str, on: bool, p: &Pal) -> Response {
    let resp = ui.interact(rect, ui.id().with(("caption", icon)), Sense::click());
    let pressed = resp.is_pointer_button_down_on() || on;
    panel(ui.painter(), rect, p, p.face, if pressed { Bevel::Pressed } else { Bevel::Raised });
    let c = rect.center() + if pressed { vec2(1.0, 1.0) } else { Vec2::ZERO };
    super::icons::paint(ui.painter(), Rect::from_center_size(c, vec2(11.0, 11.0)), icon, p.text);
    focus_mark(ui, &resp, rect.shrink(3.0));
    tip(resp, tooltip)
}

/// A sunken white well filling `rect`; `add` gets the inside.
pub fn well<R>(ui: &mut Ui, rect: Rect, p: &Pal, add: impl FnOnce(&mut Ui) -> R) -> R {
    panel(ui.painter(), rect, p, p.window, Bevel::Sunken);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(2.0)).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.set_clip_rect(rect.shrink(2.0).intersect(ui.clip_rect()));
        add(ui)
    })
    .inner
}

/// A yellow information bar (connection lost, update ready, waiting room).
pub fn info_bar<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let out = egui::Frame::new().fill(INFO).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    outline(ui.painter(), out.response.rect, Color32::BLACK);
    out.inner
}

/// A status-bar cell: thin sunken ring around `rect`.
pub fn status_cell(painter: &egui::Painter, rect: Rect, p: &Pal) {
    bevel(painter, rect, p, Bevel::ThinIn);
}

// ── Controls ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Normal,
    /// The dialog's default button: bold, with an extra black ring.
    Default,
    /// Something you can't take back (sign out, clear): red words, never the default.
    Danger,
}

/// Keyboard focus (Tab; a click never focuses a button, so the mouse never
/// sees it): Windows 98's dotted rectangle round `inner`.
pub fn focus_mark(ui: &Ui, resp: &Response, inner: Rect) {
    // (`has_focus`, but YAP_DEBUG_FOCUSED's shots count as a focused window too.)
    let window = ui.input(|i| i.focused) || cfg!(debug_assertions) && std::env::var_os("YAP_DEBUG_FOCUSED").is_some();
    if !(window && ui.memory(|m| m.has_focus(resp.id))) {
        return;
    }
    dotted_rect(ui.painter(), inner, pal(ui.ctx()).text);
}

/// The 98 focus rectangle: a dotted line round `rect` (1 pt dots, 1 pt gaps).
fn dotted_rect(painter: &egui::Painter, rect: Rect, color: Color32) {
    let r = rect.round();
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.0, color), 1.0, 1.0));
}

/// A push button: at least 75×23, raised, presses in (label shifts 1 pt).
pub fn button(ui: &mut Ui, label: &str, kind: Btn, enabled: bool, p: &Pal) -> Response {
    button_sized(ui, label, kind, enabled, vec2(75.0, 23.0), p)
}

pub fn button_sized(ui: &mut Ui, label: &str, kind: Btn, enabled: bool, min: Vec2, p: &Pal) -> Response {
    let f = if kind == Btn::Default { bold_font(UI) } else { font(UI) };
    let g = ui.painter().layout_no_wrap(label.to_string(), f.clone(), p.text);
    let size = vec2((g.size().x + 20.0).max(min.x).round(), min.y);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let pressed = enabled && resp.is_pointer_button_down_on();
    paint_button(ui, rect, kind == Btn::Default, pressed, p);
    let c = rect.center() + if pressed { vec2(1.0, 1.0) } else { Vec2::ZERO };
    let fg = if kind == Btn::Danger { p.danger } else { p.text };
    paint_label(ui.painter(), c, Align2::CENTER_CENTER, label, f, fg, enabled, p);
    focus_mark(ui, &resp, rect.shrink(4.0));
    resp
}

/// How wide [`button`] makes a button with this label (to centre or fit a row).
pub fn button_width(ui: &Ui, label: &str, kind: Btn) -> f32 {
    let f = if kind == Btn::Default { bold_font(UI) } else { font(UI) };
    (ui.painter().layout_no_wrap(label.to_string(), f, Color32::WHITE).size().x + 20.0).max(75.0).round()
}

/// Text, or embossed grey when disabled.
#[allow(clippy::too_many_arguments)]
pub fn paint_label(painter: &egui::Painter, at: Pos2, align: Align2, text: &str, f: FontId, color: Color32, enabled: bool, p: &Pal) {
    if enabled {
        painter.text(at, align, text, f, color);
    } else {
        painter.text(at + vec2(1.0, 1.0), align, text, f.clone(), p.hilight);
        painter.text(at, align, text, f, p.gray);
    }
}

pub fn paint_button(ui: &Ui, rect: Rect, default: bool, pressed: bool, p: &Pal) {
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, p.face);
    let r = if default {
        outline(painter, rect, p.dk);
        rect.shrink(1.0)
    } else {
        rect
    };
    bevel(painter, r, p, if pressed { Bevel::Pressed } else { Bevel::Raised });
}

/// A toggle button that stays pushed in (dithered) while `on`.
pub fn toggle_button(ui: &mut Ui, label: &str, on: bool, min: Vec2, p: &Pal) -> Response {
    let f = if on { bold_font(UI) } else { font(UI) };
    let g = ui.painter().layout_no_wrap(label.to_string(), bold_font(UI), p.text);
    let size = vec2((g.size().x + 16.0).max(min.x).round(), min.y);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let pressed = on || resp.is_pointer_button_down_on();
    ui.painter().rect_filled(rect, 0.0, p.face);
    if on {
        dither(ui, rect, p.hilight);
    }
    bevel(ui.painter(), rect, p, if pressed { Bevel::Pressed } else { Bevel::Raised });
    let c = rect.center() + if pressed { vec2(1.0, 1.0) } else { Vec2::ZERO };
    ui.painter().text(c, Align2::CENTER_CENTER, label, f, p.text);
    focus_mark(ui, &resp, rect.shrink(4.0));
    resp
}

/// The classic 7×7 check mark, drawn in pixels.
fn check_mark(painter: &egui::Painter, origin: Pos2, color: Color32) {
    // Each column is three pixels tall; these are the tops.
    for (x, top) in [2, 3, 4, 3, 2, 1, 0].iter().enumerate() {
        let r = Rect::from_min_size(origin + vec2(x as f32, *top as f32), vec2(1.0, 3.0));
        painter.rect_filled(r, 0.0, color);
    }
}

/// A check box with its label. `changed()` when toggled.
pub fn checkbox(ui: &mut Ui, value: &mut bool, label: &str, p: &Pal) -> Response {
    let wrap = (ui.available_width() - 19.0).max(40.0);
    let g = ui.painter().layout(label.to_string(), font(UI), p.text, wrap);
    let size = vec2(19.0 + g.size().x, g.size().y.max(15.0));
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    let first_line = g.rows.first().map(|r| r.height()).unwrap_or(14.0);
    let b = Rect::from_min_size(pos2(rect.left(), rect.top() + (first_line - 13.0) / 2.0).round(), vec2(13.0, 13.0));
    let pressed = resp.is_pointer_button_down_on();
    panel(ui.painter(), b, p, if pressed { p.face } else { p.window }, Bevel::Sunken);
    if *value {
        check_mark(ui.painter(), b.min + vec2(3.0, 3.0), p.text);
    }
    let label_rect = Rect::from_min_size(pos2(rect.left() + 19.0, rect.top()), g.size());
    ui.painter().galley(label_rect.min, g, p.text);
    focus_mark(ui, &resp, label_rect.expand2(vec2(2.0, 1.0)));
    if resp.clicked() {
        *value = !*value;
        resp.mark_changed();
    }
    resp
}

/// A radio button with its label. Returns the response; `clicked()` selects.
pub fn radio(ui: &mut Ui, selected: bool, label: &str, p: &Pal) -> Response {
    let g = ui.painter().layout_no_wrap(label.to_string(), font(UI), p.text);
    let size = vec2(18.0 + g.size().x, g.size().y.max(15.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let c = pos2(rect.left() + 6.0, rect.center().y);
    let painter = ui.painter();
    // Round, sunken: dark top-left arc, light bottom-right arc.
    painter.circle_filled(c, 6.0, p.hilight);
    painter.circle_filled(c - vec2(0.5, 0.5), 5.5, p.shadow);
    painter.circle_filled(c, 5.0, p.light);
    painter.circle_filled(c - vec2(0.5, 0.5), 4.5, p.dk);
    painter.circle_filled(c, 4.0, if resp.is_pointer_button_down_on() { p.face } else { p.window });
    if selected {
        painter.circle_filled(c, 2.0, p.text);
    }
    let label_rect = Rect::from_min_size(pos2(rect.left() + 18.0, rect.center().y - g.size().y / 2.0), g.size());
    painter.galley(label_rect.min, g, p.text);
    focus_mark(ui, &resp, label_rect.expand2(vec2(2.0, 1.0)));
    resp
}

/// A row of radio buttons for an enum.
pub fn radio_row<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)], p: &Pal) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for (v, label) in options {
            if radio(ui, *value == *v, label, p).clicked() && *value != *v {
                *value = *v;
                changed = true;
            }
        }
    });
    changed
}

/// A sunken text field (wraps any `TextEdit`).
pub fn field(ui: &mut Ui, edit: egui::TextEdit<'_>, p: &Pal) -> Response {
    let resp = ui.add(edit.text_color(p.text).frame(egui::Frame::new().fill(p.window).inner_margin(egui::Margin { left: 5, right: 5, top: 4, bottom: 4 })));
    bevel(ui.painter(), resp.rect, p, Bevel::Sunken);
    resp
}

/// The ▼ glyph.
pub fn arrow_down(painter: &egui::Painter, c: Pos2, color: Color32) {
    let c = c.round();
    for i in 0..4 {
        let w = 7.0 - 2.0 * i as f32;
        painter.rect_filled(Rect::from_min_size(pos2(c.x - 3.5 + i as f32, c.y - 2.0 + i as f32), vec2(w, 1.0)), 0.0, color);
    }
}

/// Menus and drop-down lists: face fill, window bevel (painted by [`menu_bevel`]).
pub fn menu_frame(p: &Pal) -> egui::Frame {
    egui::Frame::new().fill(p.face).inner_margin(3).corner_radius(0)
}

pub fn menu_bevel(ui: &Ui, p: &Pal) {
    bevel(ui.painter(), ui.min_rect().expand(3.0), p, Bevel::Window);
}

/// A menu item: navy under the pointer.
pub fn menu_item(ui: &mut Ui, label: &str, checked: bool, p: &Pal) -> Response {
    let w = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
    let hot = resp.hovered();
    if hot {
        ui.painter().rect_filled(rect, 0.0, p.select);
    }
    let fg = if hot { p.on_select } else { p.text };
    if checked {
        check_mark(ui.painter(), pos2(rect.left() + 5.0, rect.center().y - 3.0).round(), fg);
    }
    let clip = Rect::from_min_max(pos2(rect.left() + 18.0, rect.top()), pos2(rect.right() - 4.0, rect.bottom()));
    ui.painter().with_clip_rect(clip.intersect(ui.clip_rect())).text(pos2(rect.left() + 18.0, rect.center().y), Align2::LEFT_CENTER, label, font(UI), fg);
    resp
}

/// A right-click menu on `resp`.
pub fn context_menu(resp: &Response, p: &Pal, add: impl FnOnce(&mut Ui)) {
    egui::Popup::context_menu(resp).frame(menu_frame(p)).show(|ui| {
        ui.set_min_width(160.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui);
        menu_bevel(ui, p);
    });
}

/// A drop-down list: sunken white field with a ▼ button; `add` draws the items.
pub fn combo(ui: &mut Ui, id: &str, selected: &str, width: f32, p: &Pal, add: impl FnOnce(&mut Ui)) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click());
    let id = ui.id().with(id);
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    panel(ui.painter(), rect, p, p.window, Bevel::Sunken);
    let btn = Rect::from_min_max(pos2(rect.right() - 2.0 - 16.0, rect.top() + 2.0), rect.max - vec2(2.0, 2.0));
    let pressed = open || resp.is_pointer_button_down_on();
    panel(ui.painter(), btn, p, p.face, if pressed { Bevel::Pressed } else { Bevel::Raised });
    arrow_down(ui.painter(), btn.center() + if pressed { vec2(1.0, 1.0) } else { Vec2::ZERO }, p.text);
    let text_rect = Rect::from_min_max(rect.min + vec2(3.0, 3.0), pos2(btn.left() - 2.0, rect.bottom() - 3.0));
    if open || resp.has_focus() {
        ui.painter().rect_filled(text_rect, 0.0, p.select);
    }
    // The choice, shortened with "…" if it doesn't fit.
    let fg = if open || resp.has_focus() { p.on_select } else { p.text };
    let g = elide(ui.painter(), selected, font(UI), fg, text_rect.width() - 4.0);
    ui.painter().galley(pos2(text_rect.left() + 2.0, text_rect.center().y - g.size().y / 2.0), g, fg);
    egui::Popup::menu(&resp).id(id).frame(menu_frame(p)).width(width).show(|ui| {
        ui.set_min_width(width - 6.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().max_height(260.0).show(ui, add);
        menu_bevel(ui, p);
    });
    resp
}

/// A trackbar (slider): sunken groove, raised rectangular thumb.
pub fn trackbar(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, step: f32, width: f32, p: &Pal) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    let groove = Rect::from_min_max(pos2(rect.left() + 5.0, rect.center().y - 2.0), pos2(rect.right() - 5.0, rect.center().y + 2.0)).round();
    if let Some(pos) = resp.interact_pointer_pos() {
        let t = ((pos.x - groove.left()) / groove.width()).clamp(0.0, 1.0);
        let mut v = lo + t * (hi - lo);
        if step > 0.0 {
            v = (v / step).round() * step;
        }
        v = v.clamp(lo, hi);
        if v != *value {
            *value = v;
            resp.mark_changed();
        }
    }
    panel(ui.painter(), groove, p, p.window, Bevel::Sunken);
    let t = if hi > lo { ((*value - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    let x = groove.left() + t * groove.width();
    let thumb = Rect::from_center_size(pos2(x, rect.center().y), vec2(11.0, 21.0)).round();
    panel(ui.painter(), thumb, p, p.face, Bevel::Raised);
    resp
}

/// A group box: an etched frame with `title` sitting on its top edge.
pub fn group<R>(ui: &mut Ui, title: &str, p: &Pal, add: impl FnOnce(&mut Ui) -> R) -> R {
    let titled = !title.is_empty();
    let out = egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: if titled { 16 } else { 10 }, bottom: 8 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui)
    });
    let r = out.response.rect;
    let top = r.top() + if titled { 6.0 } else { 0.0 };
    bevel(ui.painter(), Rect::from_min_max(pos2(r.left(), top), r.max), p, Bevel::Etched);
    if !title.is_empty() {
        let g = ui.painter().layout_no_wrap(title.to_string(), font(UI), p.text);
        let at = pos2(r.left() + 8.0, top - g.size().y / 2.0);
        ui.painter().rect_filled(Rect::from_min_size(at - vec2(3.0, 0.0), g.size() + vec2(6.0, 0.0)), 0.0, p.face);
        ui.painter().galley(at, g, p.text);
    }
    ui.add_space(6.0);
    out.inner
}

/// A text layout drawn as it is: unlike `ui.label`, it keeps the job's own
/// wrap width and truncation (a Label resets them to the layout's).
pub fn job_label(ui: &mut Ui, job: egui::text::LayoutJob) -> Response {
    let g = ui.painter().layout_job(job);
    let (rect, resp) = ui.allocate_exact_size(g.size(), Sense::hover());
    ui.painter().galley(rect.min, g, Color32::PLACEHOLDER);
    resp
}

/// "Advanced", folded until clicked (Settings › Voice): Windows 98's
/// "Advanced >>" button that opens more of the sheet (its options in an
/// etched box under it).
pub fn advanced(ui: &mut Ui, open: &mut bool, p: &Pal, add: impl FnOnce(&mut Ui)) -> bool {
    ui.add_space(2.0);
    if button(ui, if *open { "Advanced <<" } else { "Advanced >>" }, Btn::Normal, true, p).clicked() {
        *open = !*open;
    }
    if *open {
        ui.add_space(4.0);
        group(ui, "", p, add);
    }
    *open
}

/// Property-sheet tabs over a framed panel; `add` draws the current page.
pub fn tabs<R>(ui: &mut Ui, current: &mut usize, labels: &[&str], p: &Pal, add: impl FnOnce(&mut Ui, usize) -> R) -> R {
    let row_h = 22.0;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
    let mut x = row.left() + 2.0;
    let mut selected = Rect::NOTHING;
    for (i, label) in labels.iter().enumerate() {
        let g = ui.painter().layout_no_wrap(label.to_string(), font(UI), p.text);
        let w = (g.size().x + 16.0).round();
        let mut r = Rect::from_min_max(pos2(x, row.top() + 2.0), pos2(x + w, row.bottom()));
        let resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click());
        if resp.clicked() {
            *current = i;
        }
        let on = *current == i;
        if on {
            r = Rect::from_min_max(pos2(r.left() - 2.0, row.top()), pos2(r.right() + 2.0, row.bottom() + 1.0));
            selected = r;
        }
        let painter = ui.painter();
        painter.rect_filled(r, 0.0, p.face);
        // hilight left + top, light inside; dk right, shadow inside. No bottom edge.
        painter.rect_filled(Rect::from_min_max(r.min, pos2(r.left() + 1.0, r.bottom())), 0.0, p.hilight);
        painter.rect_filled(Rect::from_min_max(r.min, pos2(r.right() - 1.0, r.top() + 1.0)), 0.0, p.hilight);
        painter.rect_filled(Rect::from_min_max(r.min + vec2(1.0, 1.0), pos2(r.left() + 2.0, r.bottom())), 0.0, p.light);
        painter.rect_filled(Rect::from_min_max(r.min + vec2(1.0, 1.0), pos2(r.right() - 2.0, r.top() + 2.0)), 0.0, p.light);
        painter.rect_filled(Rect::from_min_max(pos2(r.right() - 1.0, r.top() + 1.0), r.max), 0.0, p.dk);
        painter.rect_filled(Rect::from_min_max(pos2(r.right() - 2.0, r.top() + 2.0), pos2(r.right() - 1.0, r.bottom())), 0.0, p.shadow);
        painter.text(pos2(r.center().x, r.top() + 10.0 + if on { 0.0 } else { -1.0 }), Align2::CENTER_CENTER, *label, font(UI), p.text);
        let label_rect = Rect::from_center_size(pos2(r.center().x, r.top() + 10.0 + if on { 0.0 } else { -1.0 }), g.size() + vec2(4.0, 0.0));
        focus_mark(ui, &resp, label_rect);
        x += w - 1.0;
    }
    let out = egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 12, bottom: 10 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add(ui, *current)
    });
    let panel_rect = Rect::from_min_max(pos2(row.left(), row.bottom()), out.response.rect.max);
    bevel(ui.painter(), panel_rect, p, Bevel::Window);
    if selected.is_positive() {
        // The selected tab opens into the page.
        ui.painter().rect_filled(Rect::from_min_max(pos2(selected.left() + 1.0, row.bottom()), pos2(selected.right() - 2.0, row.bottom() + 2.0)), 0.0, p.face);
    }
    out.inner
}

/// A segmented level meter in a thin sunken box. `frac` 0..1.
pub fn meter(ui: &Ui, rect: Rect, frac: f32, color: Color32, p: &Pal) {
    panel(ui.painter(), rect, p, p.window, Bevel::ThinIn);
    let inner = rect.shrink(2.0);
    let end = inner.left() + inner.width() * frac.clamp(0.0, 1.0);
    let mut x = inner.left();
    while x < end {
        let w = (end - x).min(8.0);
        ui.painter().rect_filled(Rect::from_min_size(pos2(x, inner.top()), vec2(w, inner.height())), 0.0, color);
        x += 10.0;
    }
}

/// The voice-activity threshold on a [`meter`] at `frac` (0..1).
pub fn meter_mark(ui: &Ui, rect: Rect, frac: f32, p: &Pal) {
    let x = (rect.left() + 2.0 + (rect.width() - 4.0) * frac.clamp(0.0, 1.0)).round();
    ui.painter().rect_filled(Rect::from_min_max(pos2(x, rect.top()), pos2(x + 2.0, rect.bottom())), 0.0, p.text);
}

/// A ScrollArea whose bar looks like 98's: dithered track, bevelled thumb.
pub fn scroll<R>(ui: &mut Ui, area: egui::ScrollArea, p: &Pal, add: impl FnOnce(&mut Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
    let out = area.show(ui, add);
    let inner = out.inner_rect;
    let content = out.content_size.y;
    if content > inner.height() + 0.5 && inner.height() > 8.0 {
        let track = Rect::from_min_max(pos2(inner.right(), inner.top()), pos2(inner.right() + BAR, inner.bottom()));
        let painter = ui.painter();
        painter.rect_filled(track, 0.0, p.face);
        dither(ui, track, p.hilight);
        let max_off = content - inner.height();
        let h = (track.height() * inner.height() / content).max(18.0).min(track.height());
        let y = egui::remap_clamp(out.state.offset.y, 0.0..=max_off, track.top()..=track.bottom() - h);
        let thumb = Rect::from_min_size(pos2(track.left(), y), vec2(BAR, h));
        panel(ui.painter(), thumb, p, p.face, Bevel::Raised);
    }
    out
}

/// The desktop behind the windows: one flat colour.
pub fn desktop(painter: &egui::Painter, rect: Rect, p: &Pal) {
    painter.rect_filled(rect, 0.0, p.desktop);
}

fn visuals(p: Pal) -> egui::Visuals {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.desktop;
    // Only tooltips use the stock popup frame (menus and dialogs bring their own).
    v.window_fill = INFO;
    v.window_stroke = Stroke::new(1.0, Color32::BLACK);
    v.window_corner_radius = egui::CornerRadius::ZERO;
    v.menu_corner_radius = egui::CornerRadius::ZERO;
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;
    v.extreme_bg_color = p.window;
    v.text_edit_bg_color = Some(p.window);
    v.faint_bg_color = p.face;
    v.code_bg_color = p.code_bg;
    v.hyperlink_color = p.link;
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.gray);
    v.selection.bg_fill = p.select;
    v.selection.stroke = Stroke::new(1.0, p.on_select);
    v.text_cursor.stroke = Stroke::new(1.0, p.text);
    v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.5 };
    v.slider_trailing_fill = false;
    v.button_frame = true;
    v.indent_has_left_vline = false;
    v.interact_cursor = None;
    let w = &mut v.widgets;
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = egui::CornerRadius::ZERO;
        state.fg_stroke = Stroke::new(1.0, p.text);
        state.expansion = 0.0;
        state.bg_fill = p.face;
        state.weak_bg_fill = p.face;
        state.bg_stroke = Stroke::new(1.0, p.shadow);
    }
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.shadow);
    v
}

pub fn install_style(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| *s = egui::Style::default());
    ctx.set_visuals_of(egui::Theme::Light, visuals(light()));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(dark()));
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, font(SMALL)),
            (TextStyle::Body, font(UI)),
            (TextStyle::Button, font(UI)),
            (TextStyle::Heading, bold_font(13.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        s.spacing.item_spacing = vec2(6.0, 4.0);
        s.spacing.button_padding = vec2(8.0, 3.0);
        s.spacing.interact_size.y = 21.0;
        s.spacing.menu_margin = egui::Margin::same(3);
        s.spacing.scroll = egui::style::ScrollStyle::solid();
        s.spacing.scroll.bar_width = BAR;
        s.spacing.scroll.bar_inner_margin = 0.0;
        s.spacing.scroll.bar_outer_margin = 0.0;
        s.spacing.scroll.handle_min_length = 18.0;
        s.spacing.scroll.fade.strength = 0.0;
        s.spacing.tooltip_width = 320.0;
        s.interaction.selectable_labels = false;
        s.interaction.tooltip_delay = 0.4;
        s.animation_time = 0.0;
        s.url_in_tooltip = true;
    });
}

/// Tahoma when the system has it (Windows, macOS), else the system sans
/// (Segoe UI / DejaVu Sans); egui's bundled fonts cover emoji and symbols.
pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = FontDefinitions::default();
    let fallback: Vec<String> = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mono_fallback: Vec<String> = defs.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    let add = |defs: &mut FontDefinitions, name: &str, path: Option<String>| -> bool {
        let Some(path) = path else { return false };
        match std::fs::read(&path) {
            Ok(bytes) => {
                defs.font_data.insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let (regular, bold_path, mono, extra) = system_font_paths();
    let has_regular = add(&mut defs, "sys", regular);
    let has_bold = add(&mut defs, "sys-bold", bold_path);
    let has_mono = add(&mut defs, "sys-mono", mono);
    let mut extras = Vec::new();
    for (i, p) in extra.into_iter().enumerate() {
        let name = format!("sys-extra-{i}");
        if add(&mut defs, &name, Some(p)) {
            extras.push(name);
        }
    }
    let mut prop: Vec<String> = Vec::new();
    if has_regular {
        prop.push("sys".into());
    }
    prop.extend(extras.iter().cloned());
    prop.extend(fallback.iter().cloned());
    // Last resort for symbols nothing else has (arrows and the like).
    if add(&mut defs, "sys-symbols", last_resort_font()) {
        prop.push("sys-symbols".into());
    }
    let mut bold_family: Vec<String> = Vec::new();
    if has_bold {
        bold_family.push("sys-bold".into());
    }
    bold_family.extend(prop.iter().cloned());
    let mut mono_family: Vec<String> = Vec::new();
    if has_mono {
        mono_family.push("sys-mono".into());
    }
    mono_family.extend(mono_fallback);
    mono_family.extend(prop.iter().cloned());
    add_hand_font(&mut defs, &prop);
    defs.families.insert(FontFamily::Proportional, prop);
    defs.families.insert(bold(), bold_family);
    defs.families.insert(FontFamily::Monospace, mono_family);
    ctx.set_fonts(defs);
}

/// The drawing board's text font family (the web uses Comic Sans and friends).
pub fn hand() -> FontFamily {
    FontFamily::Name("hand".into())
}

/// Add the "hand" family: Comic Sans MS if the system has it (Windows, macOS),
/// else Comic Neue / the default sans, then `fallback`.
pub fn add_hand_font(defs: &mut FontDefinitions, fallback: &[String]) {
    let candidates = crate::drawing::hand_font_paths();
    let mut family = Vec::new();
    if let Some(bytes) = first_existing(&candidates.iter().map(|s| s.to_string()).collect::<Vec<_>>()).and_then(|p| std::fs::read(p).ok()) {
        defs.font_data.insert("hand".into(), Arc::new(FontData::from_owned(bytes)));
        family.push("hand".to_string());
    }
    family.extend(fallback.iter().cloned());
    defs.families.insert(hand(), family);
}

/// A symbols font that covers arrows etc. (used after every other font).
pub fn last_resort_font() -> Option<String> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/Apple Symbols.ttf"]
    } else if cfg!(target_os = "linux") {
        &["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/TTF/DejaVuSans.ttf", "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf"]
    } else {
        &["C:\\Windows\\Fonts\\seguisym.ttf"]
    };
    first_existing(&candidates.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

fn first_existing(paths: &[String]) -> Option<String> {
    paths.iter().find(|p| std::path::Path::new(p).exists()).cloned()
}

#[cfg(target_os = "windows")]
fn system_font_paths() -> (Option<String>, Option<String>, Option<String>, Vec<String>) {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()) + "\\Fonts\\";
    let f = |names: &[&str]| first_existing(&names.iter().map(|n| format!("{dir}{n}")).collect::<Vec<_>>());
    (
        f(&["tahoma.ttf", "segoeui.ttf"]),
        f(&["tahomabd.ttf", "segoeuib.ttf"]),
        f(&["lucon.ttf", "consola.ttf", "cour.ttf"]),
        vec![format!("{dir}segoeui.ttf"), format!("{dir}seguisym.ttf"), format!("{dir}seguiemj.ttf")],
    )
}

#[cfg(target_os = "linux")]
fn system_font_paths() -> (Option<String>, Option<String>, Option<String>, Vec<String>) {
    let fc = |pattern: &str| -> Option<String> {
        let out = std::process::Command::new("fc-match").args(["-f", "%{file}", pattern]).output().ok()?;
        let path = String::from_utf8(out.stdout).ok()?;
        // Only TrueType/OpenType outlines (not bitmap or collection-only fonts).
        (path.ends_with(".ttf") || path.ends_with(".otf")).then_some(path)
    };
    let guess = |names: &[&str]| first_existing(&names.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    // fontconfig answers "Tahoma" with Tahoma if it's installed, else its best sans.
    let regular = fc("Tahoma").or_else(|| fc("sans-serif")).or_else(|| {
        guess(&[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        ])
    });
    let bold = fc("Tahoma:bold").or_else(|| fc("sans-serif:bold")).or_else(|| {
        guess(&[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Bold.ttf",
        ])
    });
    let mono = fc("monospace").or_else(|| guess(&["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", "/usr/share/fonts/TTF/DejaVuSansMono.ttf"]));
    (regular, bold, mono, vec![])
}

#[cfg(target_os = "macos")]
fn system_font_paths() -> (Option<String>, Option<String>, Option<String>, Vec<String>) {
    let s = |names: &[&str]| first_existing(&names.iter().map(|n| format!("/System/Library/Fonts/Supplemental/{n}")).collect::<Vec<_>>());
    (s(&["Tahoma.ttf", "Verdana.ttf", "Arial.ttf"]), s(&["Tahoma Bold.ttf", "Verdana Bold.ttf", "Arial Bold.ttf"]), s(&["Courier New.ttf"]), vec![])
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn system_font_paths() -> (Option<String>, Option<String>, Option<String>, Vec<String>) {
    (None, None, None, vec![])
}
