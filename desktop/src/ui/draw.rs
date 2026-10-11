//! Draw: the shared drawing board inside the app (web/js/draw.js). Strokes
//! travel in board units (10000 × 6250); live pieces go out as `draw.seg`,
//! finished strokes as `draw.stroke`. Pen / line / box / circle / text,
//! eraser, sizes, Paint's colour box, undo, clear, the "wiggle" line boil
//! (the same position-seeded noise as the web, so everyone sees the same
//! wobble), Save (PNG) / GIF (the wiggle, looping) to the gallery and
//! Downloads, Share in the chat, name tags at the tip of other people's strokes.
//!
//! WigglyPaint-style pixels: the page is drawn at 640 × 400 with every pixel on
//! or off (`crate::drawing`) and shown blown up with hard pixel edges.
//!
//! Three shared layers (PROTOCOL.md › Draw layers): each is its own sheet
//! (and texture), stacked bottom to top on the paper. Which layer you draw on
//! and which ones you hide are yours alone (never sent).

use super::{Act, App, icons, theme, theme::Pal, theme::RoundRect};
use crate::core::{AfterSave, Cmd, Model};
use crate::drawing::{self, BOARD_H, BOARD_W, LAYERS, PIX_H, PIX_W, Pixels, WIGGLE_FRAMES, text_size};
use crate::protocol::Stroke;
use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, pos2, vec2};

const PAPER: Color32 = Color32::WHITE;
const SEND_EVERY: f64 = 0.04;
const WIGGLE_SECS: f64 = drawing::WIGGLE_MS as f64 / 1000.0;
/// Paint's colour box: dark row on top, bright row below.
const COLORS: [&str; 28] = [
    "#000000", "#808080", "#800000", "#808000", "#008000", "#008080", "#000080", "#800080", "#808040", "#004040", "#0080ff", "#004080", "#8000ff", "#804000", "#ffffff", "#c0c0c0", "#ff0000", "#ffff00", "#00ff00", "#00ffff", "#0000ff", "#ff00ff", "#ffff80", "#00ff80", "#80ffff", "#8080ff", "#ff0080", "#ff8040",
];
const SIZES: [(f32, &str); 4] = [(14.0, "S"), (32.0, "M"), (70.0, "L"), (160.0, "XL")];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Pen,
    Line,
    Rect,
    Ellipse,
    Text,
}

/// A stroke being drawn here.
struct Drawing {
    sk: String,
    tool: Tool,
    /// The layer it goes on (the one you were on when it started).
    l: u8,
    p: Vec<f32>,
    start: (f32, f32),
    sent: usize,
    last_send: f64,
}

/// The committed page: for each wiggle frame (one with Wiggle off), one
/// transparent sheet per layer, drawn like the web: a new stroke goes on top of
/// its layer; undo / clear / a wipe redraw that layer, Wiggle redraws all.
#[derive(Default)]
struct Pages {
    version: u64,
    wiggle: bool,
    /// Per layer: ids of the strokes on its sheets, in order.
    drawn: [Vec<String>; LAYERS],
    /// The sheet of (frame, layer) is at `frame * LAYERS + layer`.
    px: Vec<Pixels>,
    tex: Vec<egui::TextureHandle>,
    /// Per layer: bumped whenever its sheets change (its chip's picture follows).
    stamp: [u64; LAYERS],
}

/// A layer chip's picture: the page shrunk 10× (each pixel = a 10 × 10 block,
/// inked in proportion so thin lines still show).
const THUMB: [usize; 2] = [PIX_W / 10, PIX_H / 10];

pub struct DrawUi {
    color: String,
    size: f32,
    eraser: bool,
    tool: Tool,
    wiggle: bool,
    frame: u8,
    frame_at: f64,
    drawing: Option<Drawing>,
    /// Text tool: where it goes (board units), what's typed so far, focus
    /// once, and the layer it goes on.
    text: Option<((f32, f32), String, bool, u8)>,
    confirm_clear: bool,
    /// The layer new strokes, shapes, text and the eraser go on (0 = bottom).
    layer: u8,
    /// Layers not shown on this screen (just for you).
    hidden: [bool; LAYERS],
    pages: Pages,
    /// Per layer, while strokes are in progress on it (others' and mine): its
    /// committed sheet with them drawn on, shown in its place. Redrawn each
    /// frame they're there.
    live: Vec<Pixels>,
    live_tex: [Option<egui::TextureHandle>; LAYERS],
    /// Per layer: its chip's picture, the `Pages::stamp` it was made from, and
    /// whether it shows strokes in progress (so it's redone when they end).
    thumbs: [Option<egui::TextureHandle>; LAYERS],
    thumb_stamp: [u64; LAYERS],
    thumb_live: [bool; LAYERS],
    debug_saved: bool,
}

impl Default for DrawUi {
    fn default() -> Self {
        let mut st = Self {
            color: COLORS[0].into(),
            size: SIZES[1].0,
            eraser: false,
            tool: Tool::Pen,
            wiggle: true,
            frame: 0,
            frame_at: 0.0,
            drawing: None,
            text: None,
            confirm_clear: false,
            layer: 0,
            hidden: [false; LAYERS],
            pages: Pages::default(),
            live: Vec::new(),
            live_tex: Default::default(),
            thumbs: Default::default(),
            thumb_stamp: [0; LAYERS],
            thumb_live: [false; LAYERS],
            debug_saved: false,
        };
        // Screenshot aids (debug builds): YAP_DEBUG_DRAW_LAYERS=<active>[:<hidden>…]
        // (1–3, e.g. "3:1" = drawing on layer 3, layer 1 hidden) and
        // YAP_DEBUG_DRAW_CONFIRM=1 (Clear's choices open).
        if cfg!(debug_assertions) {
            if let Ok(v) = std::env::var("YAP_DEBUG_DRAW_LAYERS") {
                let mut parts = v.split(':').filter_map(|n| n.trim().parse::<usize>().ok()).filter(|n| (1..=LAYERS).contains(n));
                if let Some(a) = parts.next() {
                    st.layer = (a - 1) as u8;
                }
                for h in parts {
                    st.hidden[h - 1] = true;
                }
            }
            st.confirm_clear = std::env::var_os("YAP_DEBUG_DRAW_CONFIRM").is_some();
        }
        st
    }
}

impl DrawUi {
    /// Draw on layer `l` from now on (and see it again if it was hidden).
    fn pick_layer(&mut self, l: u8) {
        self.layer = l.min(LAYERS as u8 - 1);
        self.hidden[self.layer as usize] = false;
    }

    /// Starting to draw on the active layer shows it again.
    fn show_active(&mut self) {
        self.hidden[self.layer as usize] = false;
    }

    /// The strokes you can see (hidden layers left out): what Save, GIF and Share export.
    fn visible(&self, m: &Model) -> Vec<Stroke> {
        m.draw.strokes.iter().filter(|s| !self.hidden[drawing::layer(s)]).cloned().collect()
    }
}

/// Big pixels when the page is shown larger than 640 × 400; smooth when smaller.
const PIXELLY: egui::TextureOptions = egui::TextureOptions { minification: egui::TextureFilter::Linear, ..egui::TextureOptions::NEAREST };

fn id() -> String {
    crate::config::random_hex(10)
}

fn color_image(px: &Pixels) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([PIX_W, PIX_H], &px.rgba)
}

/// A layer's little picture for its chip (see `THUMB`).
fn thumb_image(px: &Pixels) -> egui::ColorImage {
    let (bw, bh) = (PIX_W / THUMB[0], PIX_H / THUMB[1]);
    let mut out = vec![0u8; THUMB[0] * THUMB[1] * 4];
    for ty in 0..THUMB[1] {
        for tx in 0..THUMB[0] {
            let (mut n, mut sum) = (0u32, [0u32; 3]);
            for y in ty * bh..(ty + 1) * bh {
                for x in tx * bw..(tx + 1) * bw {
                    let i = (y * PIX_W + x) * 4;
                    if px.rgba[i + 3] != 0 {
                        n += 1;
                        for (c, v) in sum.iter_mut().zip(&px.rgba[i..i + 3]) {
                            *c += *v as u32;
                        }
                    }
                }
            }
            if n > 0 {
                let o = (ty * THUMB[0] + tx) * 4;
                let alpha = (n * 255 * 4 / (bw * bh) as u32).min(255);
                out[o..o + 4].copy_from_slice(&[(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, alpha as u8]);
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied(THUMB, &out)
}

/// Put `img` into `slot` (made the first time).
fn upload(ctx: &egui::Context, slot: &mut Option<egui::TextureHandle>, name: String, img: egui::ColorImage, opts: egui::TextureOptions) {
    match slot {
        Some(t) => t.set(img, opts),
        None => *slot = Some(ctx.load_texture(name, img, opts)),
    }
}

/// The finished stroke for a shape dragged from (x0, y0) to (x1, y1).
fn shape_points(tool: Tool, x0: f32, y0: f32, x1: f32, y1: f32) -> (Vec<f32>, Option<String>) {
    match tool {
        Tool::Line => (vec![x0, y0, x1, y1], Some("poly".into())),
        Tool::Rect => (vec![x0, y0, x1, y0, x1, y1, x0, y1, x0, y0], Some("poly".into())),
        _ => {
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let (rx, ry) = ((x1 - x0).abs() / 2.0, (y1 - y0).abs() / 2.0);
            let n = ((rx + ry) / 40.0).round().clamp(24.0, 96.0) as usize;
            let mut p = Vec::with_capacity((n + 2) * 2);
            for i in 0..=n + 1 {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                p.push((cx + rx * a.cos()).round());
                p.push((cy + ry * a.sin()).round());
            }
            (p, None)
        }
    }
}

/// Bring the committed sheets up to date with the board, and their textures:
/// only the layers that changed are redrawn and uploaded.
fn sync_pages(st: &mut DrawUi, ctx: &egui::Context, m: &Model) {
    let pg = &mut st.pages;
    let frames = if st.wiggle { WIGGLE_FRAMES as usize } else { 1 };
    let count = frames * LAYERS;
    if pg.version == m.draw.version && pg.wiggle == st.wiggle && pg.px.len() == count && pg.tex.len() == count {
        return;
    }
    let fresh = pg.wiggle != st.wiggle || pg.px.len() != count;
    if fresh {
        pg.px = vec![Pixels::new(); count];
        pg.drawn = Default::default();
    }
    let mut changed = [fresh; LAYERS];
    for (l, drawn) in pg.drawn.iter_mut().enumerate() {
        let strokes: Vec<&Stroke> = m.draw.strokes.iter().filter(|s| drawing::layer(s) == l).collect();
        let on_top = strokes.len() >= drawn.len() && strokes.iter().zip(drawn.iter()).all(|(s, id)| s.id == *id);
        if on_top && strokes.len() == drawn.len() {
            continue;
        }
        let from = if on_top {
            drawn.len()
        } else {
            for f in 0..frames {
                pg.px[f * LAYERS + l].clear();
            }
            drawn.clear();
            0
        };
        for f in 0..frames {
            let frame = st.wiggle.then_some(f as u8);
            let px = &mut pg.px[f * LAYERS + l];
            for s in &strokes[from..] {
                drawing::draw_stroke(px, s, frame);
            }
        }
        drawn.extend(strokes[from..].iter().map(|s| s.id.clone()));
        changed[l] = true;
    }
    pg.version = m.draw.version;
    pg.wiggle = st.wiggle;
    if pg.tex.len() != count {
        pg.tex = pg.px.iter().enumerate().map(|(i, px)| ctx.load_texture(format!("draw-page-{}-{}", i / LAYERS, i % LAYERS), color_image(px), PIXELLY)).collect();
    } else {
        for (i, (t, px)) in pg.tex.iter_mut().zip(&pg.px).enumerate() {
            if changed[i % LAYERS] {
                t.set(color_image(px), PIXELLY);
            }
        }
    }
    for (stamp, c) in pg.stamp.iter_mut().zip(changed) {
        if c {
            *stamp += 1;
        }
    }
}

/// The board: tools on top, the page, the gallery under it.
pub fn show(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let now = ui.input(|i| i.time);
    let st = &mut app.draw;
    // Wiggle frames cycle ~7×/s (only while the board is open).
    if st.wiggle {
        if now - st.frame_at >= WIGGLE_SECS {
            st.frame = (st.frame + 1) % WIGGLE_FRAMES;
            st.frame_at = now;
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(WIGGLE_SECS));
    }
    sync_pages(st, ui.ctx(), m);
    keys(app, ui, m, acts);
    toolbar(app, ui, m, p, acts);
    ui.add_space(6.0);
    let gallery_h = 76.0;
    let stage = ui.available_rect_before_wrap();
    let stage = Rect::from_min_max(stage.min, pos2(stage.right(), stage.bottom() - gallery_h)).shrink(6.0);
    let ratio = PIX_W as f32 / PIX_H as f32;
    let w = stage.width().min(stage.height() * ratio).max(10.0);
    let paper = Rect::from_center_size(stage.center(), vec2(w, w / ratio)).round();
    let outer = Rect::from_min_max(stage.min - vec2(6.0, 6.0), stage.max + vec2(6.0, 6.0));
    ui.allocate_rect(outer, Sense::hover());
    // Screenshot aid (debug builds): YAP_DEBUG_DRAW_SAVE=1 archives the board to the gallery once (as the GIF).
    if cfg!(debug_assertions) && m.draw.loaded && !app.draw.debug_saved && std::env::var_os("YAP_DEBUG_DRAW_SAVE").is_some() {
        app.draw.debug_saved = true;
        acts.push(save(m.draw.strokes.clone(), app.draw.wiggle, Export::Archive));
    }
    board(app, ui, m, paper, acts);
    gallery(app, ui, m, p, acts);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Export {
    /// Save: a PNG to Downloads and the gallery.
    Png,
    /// GIF: the wiggle (one frame with Wiggle off) to Downloads and the gallery.
    Gif,
    /// Share: post it in the chat (the GIF with Wiggle on, the PNG otherwise).
    Share,
    /// Into the gallery only (Clear's archive), as the GIF.
    Archive,
    /// Archive, then a fresh page for everyone.
    Clear,
    /// Archive, then this layer emptied for everyone.
    Wipe(u8),
}

/// Encoding and uploading happen off the UI thread (the core). `strokes`:
/// what you see for Save / GIF / Share, the whole page for the archives.
fn save(strokes: Vec<Stroke>, wiggle: bool, what: Export) -> Act {
    let gif = match what {
        Export::Png => false,
        Export::Share => wiggle,
        Export::Gif | Export::Archive | Export::Clear | Export::Wipe(_) => true,
    };
    let then = match what {
        Export::Clear => Some(AfterSave::Clear),
        Export::Wipe(l) => Some(AfterSave::Wipe(l)),
        _ => None,
    };
    Act::Cmd(Cmd::SaveDrawing { strokes, gif, wiggle, download: matches!(what, Export::Png | Export::Gif), share: what == Export::Share, then })
}

fn keys(app: &mut App, ui: &mut egui::Ui, _m: &Model, acts: &mut Vec<Act>) {
    if ui.ctx().egui_wants_keyboard_input() || app.modal.is_some() {
        return;
    }
    let st = &mut app.draw;
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
        acts.push(Act::Cmd(Cmd::DrawUndo));
    }
    ui.input(|i| {
        if i.key_pressed(egui::Key::E) {
            st.eraser = !st.eraser;
            if st.eraser {
                st.tool = Tool::Pen;
            }
        }
        for (k, t) in [(egui::Key::P, Tool::Pen), (egui::Key::B, Tool::Pen), (egui::Key::L, Tool::Line), (egui::Key::R, Tool::Rect), (egui::Key::O, Tool::Ellipse), (egui::Key::T, Tool::Text)] {
            if i.key_pressed(k) && i.modifiers.is_none() {
                st.tool = t;
                if t != Tool::Pen {
                    st.eraser = false;
                }
            }
        }
        // 1 2 3: the layer you draw on.
        for (k, l) in [(egui::Key::Num1, 0), (egui::Key::Num2, 1), (egui::Key::Num3, 2)] {
            if i.key_pressed(k) && i.modifiers.is_none() {
                st.pick_layer(l);
            }
        }
    });
}

/// A layer chip (the buttons take this size as is).
const CHIP: egui::Vec2 = vec2(84.0, 28.0);

/// Where a layer sits, for its tooltip.
fn layer_place(l: usize) -> &'static str {
    match l {
        0 => " (bottom)",
        1 => " (middle)",
        _ => " (top)",
    }
}

/// The Layers group: a chip per layer, bottom (1) to top (3) — its little
/// picture, its number, pressed while it's the one you draw on, and an eye to
/// hide it on your screen only.
fn layer_chips(st: &mut DrawUi, ui: &mut egui::Ui, p: &Pal) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    for l in 0..LAYERS {
        let (on, hidden, n) = (st.layer as usize == l, st.hidden[l], l + 1);
        let chip = theme::toggle_button(ui, "", on, CHIP, p);
        let rect = chip.rect;
        // The contents press in with the button.
        let shift = if on || chip.is_pointer_button_down_on() { vec2(1.0, 1.0) } else { vec2(0.0, 0.0) };
        let fg = p.text;
        let painter = ui.painter();
        painter.text(pos2(rect.left() + 12.0, rect.center().y) + shift, Align2::CENTER_CENTER, n.to_string(), theme::bold_font(12.0), fg);
        let pic = Rect::from_min_size(pos2(rect.left() + 22.0, (rect.center().y - 11.0).round()), vec2(36.0, 22.0)).translate(shift);
        // A hidden layer's picture fades.
        let fade = if hidden { 0.35 } else { 1.0 };
        painter.rect_filled(pic, 0.0, PAPER.gamma_multiply(fade));
        if let Some(t) = &st.thumbs[l] {
            painter.image(t.id(), pic, uv, Color32::WHITE.gamma_multiply(fade));
        }
        theme::bevel(painter, pic.expand(1.0), p, theme::Bevel::ThinIn);
        // The eye: on top of the chip, so it takes its own clicks.
        let eye = Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), vec2(18.0, 18.0)).translate(shift);
        let er = ui.interact(eye, ui.id().with(("draw-layer-eye", l)), Sense::click());
        if er.hovered() {
            ui.painter().rect_filled(eye, 0.0, fg.gamma_multiply(0.15));
        }
        icons::paint(ui.painter(), eye.shrink(2.0), if hidden { "eye-off" } else { "eye" }, if hidden { fg.gamma_multiply(0.7) } else { fg });
        let eye_tip = if hidden { format!("Show layer {n}") } else { format!("Hide layer {n} (just for you)") };
        if theme::tip(er, eye_tip).clicked() {
            st.hidden[l] = !hidden;
        }
        if theme::tip(chip, format!("Layer {n}{} — press {n}", layer_place(l))).clicked() {
            st.pick_layer(l as u8);
        }
    }
}

fn toolbar(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let tip = |r: egui::Response, t: &str| theme::tip(r, t);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        // Paint's colour box: two rows.
        let (cr, _) = ui.allocate_exact_size(vec2(14.0 * 15.0, 30.0), Sense::hover());
        for (i, c) in COLORS.iter().enumerate() {
            let (row, col) = (i / 14, i % 14);
            let r = Rect::from_min_size(pos2(cr.left() + col as f32 * 15.0, cr.top() + row as f32 * 15.0), vec2(14.0, 14.0));
            let resp = ui.interact(r, ui.id().with(("draw-color", i)), Sense::click());
            ui.painter().rect_filled(r, 0.0, theme::hex_color(c));
            let on = !app.draw.eraser && app.draw.color.eq_ignore_ascii_case(c);
            let edge = if on { p.select } else if resp.hovered() { p.text } else { p.gray.gamma_multiply(0.6) };
            ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(if on { 2.0 } else { 1.0 }, edge), egui::StrokeKind::Inside);
            if tip(resp, c).clicked() {
                app.draw.color = c.to_string();
                app.draw.eraser = false;
            }
        }
        ui.separator();
        for (w, label) in SIZES {
            let on = (app.draw.size - w).abs() < 0.1;
            let r = theme::toggle_button(ui, label, on, vec2(30.0, 24.0), p);
            if tip(r, &format!("Brush {label}")).clicked() {
                app.draw.size = w;
            }
        }
        ui.separator();
        for (t, icon, name) in [(Tool::Pen, "pencil", "Pen (P)"), (Tool::Line, "", "Line (L)"), (Tool::Rect, "", "Box (R)"), (Tool::Ellipse, "", "Circle (O)"), (Tool::Text, "", "Text (T)")] {
            let on = app.draw.tool == t && !(t == Tool::Pen && app.draw.eraser);
            let r = theme::toggle_button(ui, "  ", on, vec2(32.0, 24.0), p);
            // The tool's glyph, drawn (fonts differ in which symbols they have).
            let c = r.rect.center();
            let fg = if on { p.on_select } else { p.text };
            match t {
                Tool::Pen => icons::paint(ui.painter(), Rect::from_center_size(c, vec2(14.0, 14.0)), icon, fg),
                Tool::Line => {
                    ui.painter().line_segment([c + vec2(-6.0, 6.0), c + vec2(6.0, -6.0)], egui::Stroke::new(1.8, fg));
                }
                Tool::Rect => {
                    ui.painter().rect_stroke(Rect::from_center_size(c, vec2(14.0, 10.0)), 0.0, egui::Stroke::new(1.6, fg), egui::StrokeKind::Middle);
                }
                Tool::Ellipse => {
                    ui.painter().circle_stroke(c, 6.5, egui::Stroke::new(1.6, fg));
                }
                Tool::Text => {
                    ui.painter().text(c, Align2::CENTER_CENTER, "A", theme::bold_font(14.0), fg);
                }
            }
            if tip(r, name).clicked() {
                app.draw.tool = t;
                if t != Tool::Pen {
                    app.draw.eraser = false;
                }
            }
        }
        ui.separator();
        let r = theme::toggle_button(ui, "Eraser", app.draw.eraser, vec2(64.0, 24.0), p);
        if tip(r, "Eraser (E)").clicked() {
            app.draw.eraser = !app.draw.eraser;
            if app.draw.eraser {
                app.draw.tool = Tool::Pen;
            }
        }
        let r = theme::toggle_button(ui, "Wiggle", app.draw.wiggle, vec2(64.0, 24.0), p);
        if tip(r, "Wobbly lines, like WigglyPaint").clicked() {
            app.draw.wiggle = !app.draw.wiggle;
        }
        ui.separator();
        // One group of a known size, so it wraps as a whole.
        let size = vec2(LAYERS as f32 * CHIP.x + (LAYERS - 1) as f32 * ui.spacing().item_spacing.x, CHIP.y);
        ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| layer_chips(&mut app.draw, ui, p));
        ui.separator();
        let has_strokes = !m.draw.strokes.is_empty();
        if tip(theme::button_sized(ui, "Undo", theme::Btn::Normal, !m.draw.mine.is_empty(), vec2(56.0, 24.0), p), "Undo your last stroke (Ctrl+Z)").clicked() {
            acts.push(Act::Cmd(Cmd::DrawUndo));
        }
        let online = m.status == crate::core::Status::Online;
        // Save, GIF and Share: what you see (hidden layers left out).
        let nothing = if !has_strokes { Some("Draw something first.") } else if !m.draw.strokes.iter().any(|s| !app.draw.hidden[drawing::layer(s)]) { Some("The drawing is on hidden layers.") } else { None };
        if tip(theme::button_sized(ui, "Save", theme::Btn::Normal, online, vec2(56.0, 24.0), p), "Download a PNG and keep it in the gallery").clicked() {
            acts.push(save(app.draw.visible(m), app.draw.wiggle, Export::Png));
        }
        if tip(theme::button_sized(ui, "GIF", theme::Btn::Normal, online, vec2(56.0, 24.0), p), "Download an animated GIF of the page (it wiggles) and keep it in the gallery").clicked() {
            acts.push(match nothing {
                None => save(app.draw.visible(m), app.draw.wiggle, Export::Gif),
                Some(why) => Act::Toast(why.into()),
            });
        }
        if tip(theme::button_sized(ui, "Share", theme::Btn::Normal, online, vec2(56.0, 24.0), p), "Post this drawing in the chat").clicked() {
            acts.push(match nothing {
                None => save(app.draw.visible(m), app.draw.wiggle, Export::Share),
                Some(why) => Act::Toast(why.into()),
            });
        }
        if app.draw.confirm_clear {
            let l = app.draw.layer;
            // The question and its answers together, on a row of their own
            // (wrapping on a narrow window, so the buttons never fall off it).
            ui.end_row();
            ui.label(theme::text("Clear for everyone? This page goes to the gallery first.", p.danger));
            // Both archive the whole page first (every layer, as the wiggling GIF), like the web.
            // Both are danger buttons and neither is the default, like the web.
            if theme::button_sized(ui, &format!("Clear layer {}", l + 1), theme::Btn::Danger, true, vec2(56.0, 24.0), p).clicked() {
                app.draw.confirm_clear = false;
                acts.push(if m.draw.strokes.iter().any(|s| drawing::layer(s) == l as usize) {
                    save(m.draw.strokes.clone(), app.draw.wiggle, Export::Wipe(l))
                } else {
                    Act::Toast(format!("Layer {} is already empty.", l + 1))
                });
            }
            if theme::button_sized(ui, "Clear the page", theme::Btn::Danger, true, vec2(56.0, 24.0), p).clicked() {
                app.draw.confirm_clear = false;
                acts.push(if has_strokes { save(m.draw.strokes.clone(), app.draw.wiggle, Export::Clear) } else { Act::Cmd(Cmd::DrawClear) });
            }
            if theme::button_sized(ui, "Cancel", theme::Btn::Normal, true, vec2(56.0, 24.0), p).clicked() {
                app.draw.confirm_clear = false;
            }
        } else if tip(theme::button_sized(ui, "Clear", theme::Btn::Danger, online, vec2(56.0, 24.0), p), "Clear a layer or the whole page for everyone (saves the page first)").clicked() {
            app.draw.confirm_clear = true;
        }
    });
}

/// The page: each layer's sheet bottom to top (with the strokes in progress
/// on it), name tags, input.
fn board(app: &mut App, ui: &mut egui::Ui, m: &Model, paper: Rect, acts: &mut Vec<Act>) {
    let now = ui.input(|i| i.time);
    let me = m.config.uid.clone();
    let st = &mut app.draw;
    let frame = st.wiggle.then_some(st.frame);
    let painter = ui.painter().with_clip_rect(paper);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.rect_filled(paper, 0.0, PAPER);
    // Strokes in progress, each on its own layer: others' (name at the tip) and mine.
    let mut live: [Vec<Stroke>; LAYERS] = Default::default();
    let mut tags = Vec::new();
    for l in m.draw.live.values().filter(|l| l.at.elapsed().as_secs() < 8) {
        let s = Stroke { c: l.c.clone(), w: l.w, e: l.e, p: l.p.clone(), l: l.l, ..Default::default() };
        let on = drawing::layer(&s);
        live[on].push(s);
        if !st.hidden[on]
            && l.p.len() >= 2
            && let Some(u) = m.sessions.get(&l.sid).and_then(|x| m.users.get(&x.uid))
        {
            let tip = pos2(paper.left() + l.p[l.p.len() - 2] * paper.width() / BOARD_W, paper.top() + l.p[l.p.len() - 1] * paper.width() / BOARD_W);
            let g = ui.painter().layout_no_wrap(u.name.clone(), theme::bold_font(11.0), Color32::WHITE);
            let r = Rect::from_min_size(tip + vec2(8.0, 6.0), g.size() + vec2(8.0, 4.0));
            tags.push(Shape::rect_filled(r, 2.0, theme::hex_color(&u.color)));
            tags.push(Shape::galley(r.min + vec2(4.0, 2.0), g, Color32::WHITE));
        }
    }
    if let Some(d) = &st.drawing {
        let (pp, k) = if d.tool == Tool::Pen { (d.p.clone(), None) } else { shape_points(d.tool, d.start.0, d.start.1, d.p[d.p.len() - 2], d.p[d.p.len() - 1]) };
        live[d.l as usize].push(Stroke { c: st.color.clone(), w: st.size, e: st.eraser && d.tool == Tool::Pen, p: pp, k, l: d.l, ..Default::default() });
    }
    // Each layer: its committed sheet, or (while strokes are in progress on it)
    // a copy with them drawn on, the eraser really erasing — so ink on a lower
    // layer stays under the layers above it while it's drawn.
    let at = |l: usize| frame.unwrap_or(0) as usize * LAYERS + l;
    if st.live.len() < LAYERS {
        st.live.resize_with(LAYERS, Pixels::new);
    }
    for (l, strokes) in live.iter().enumerate() {
        let shown = !st.hidden[l];
        if strokes.is_empty() || !shown {
            if shown && let Some(t) = st.pages.tex.get(at(l)) {
                painter.image(t.id(), paper, uv, Color32::WHITE);
            }
            // Its chip shows the committed layer (frame 0).
            if (st.thumb_live[l] || st.thumb_stamp[l] != st.pages.stamp[l] || st.thumbs[l].is_none())
                && let Some(px) = st.pages.px.get(l)
            {
                upload(ui.ctx(), &mut st.thumbs[l], format!("draw-thumb-{l}"), thumb_image(px), egui::TextureOptions::LINEAR);
                st.thumb_stamp[l] = st.pages.stamp[l];
                st.thumb_live[l] = false;
            }
            continue;
        }
        let sheet = &mut st.live[l];
        match st.pages.px.get(at(l)) {
            Some(base) => sheet.rgba.copy_from_slice(&base.rgba),
            None => sheet.clear(),
        }
        for s in strokes {
            drawing::draw_stroke(sheet, s, frame);
        }
        upload(ui.ctx(), &mut st.live_tex[l], format!("draw-live-{l}"), color_image(sheet), PIXELLY);
        if let Some(t) = &st.live_tex[l] {
            painter.image(t.id(), paper, uv, Color32::WHITE);
        }
        upload(ui.ctx(), &mut st.thumbs[l], format!("draw-thumb-{l}"), thumb_image(sheet), egui::TextureOptions::LINEAR);
        st.thumb_live[l] = true;
    }
    painter.extend(tags);
    if !m.draw.live.is_empty() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    let p = theme::pal(ui.ctx());
    ui.painter().rect_stroke(paper, 0.0, egui::Stroke::new(1.0, p.gray.gamma_multiply(0.5)), egui::StrokeKind::Outside);
    if !m.draw.loaded {
        painter.text(paper.center(), Align2::CENTER_CENTER, "Opening the board…", theme::font(13.0), Color32::GRAY);
    }
    // Input.
    let resp = ui.interact(paper, ui.id().with("draw-paper"), Sense::click_and_drag());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if st.tool == Tool::Text { egui::CursorIcon::Text } else { egui::CursorIcon::Crosshair });
    }
    let to_board = |pos: Pos2| -> (f32, f32) { (((pos.x - paper.left()) / paper.width() * BOARD_W).round(), ((pos.y - paper.top()) / paper.height() * BOARD_H).round()) };
    if st.tool == Tool::Text {
        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            st.show_active();
            st.text = Some((to_board(pos), String::new(), true, st.layer));
        }
    } else if resp.drag_started() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let (x, y) = to_board(pos);
            st.show_active();
            st.drawing = Some(Drawing { sk: id(), tool: st.tool, l: st.layer, p: vec![x, y], start: (x, y), sent: 0, last_send: 0.0 });
        }
    } else if resp.dragged()
        && let Some(d) = st.drawing.as_mut()
    {
            // Every pointer move this frame (smoother than one point per frame).
            let moves: Vec<Pos2> = ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::PointerMoved(p) = e { Some(*p) } else { None }).collect());
            for pos in moves {
                let (x, y) = to_board(pos);
                if d.tool == Tool::Pen {
                    let n = d.p.len();
                    if n < 2 || (d.p[n - 2] - x).abs() + (d.p[n - 1] - y).abs() > 2.0 {
                        d.p.push(x);
                        d.p.push(y);
                    }
                } else {
                    d.p.truncate(2);
                    d.p.push(x);
                    d.p.push(y);
                }
            }
            if d.tool == Tool::Pen && now - d.last_send >= SEND_EVERY {
                let from = d.sent.saturating_sub(2);
                if d.p.len() - from >= 2 {
                    acts.push(Act::Cmd(Cmd::DrawSeg { sk: d.sk.clone(), c: st.color.clone(), w: st.size, e: st.eraser, p: d.p[from..].to_vec(), l: d.l }));
                    d.sent = d.p.len();
                    d.last_send = now;
                }
            }
    }
    if (resp.drag_stopped() || resp.clicked() && st.tool == Tool::Pen)
        && let Some(d) = st.drawing.take()
    {
        let (pp, k) = if d.tool == Tool::Pen {
            (d.p.clone(), None)
        } else {
            let n = d.p.len();
            shape_points(d.tool, d.start.0, d.start.1, d.p[n - 2], d.p[n - 1])
        };
        let stroke = Stroke { id: d.sk, uid: me.clone(), c: st.color.clone(), w: st.size, e: st.eraser && d.tool == Tool::Pen, p: pp, k, text: None, l: d.l };
        acts.push(Act::Cmd(Cmd::DrawStroke(stroke)));
    } else if resp.clicked()
        && st.tool == Tool::Pen
        && let Some(pos) = resp.interact_pointer_pos()
    {
        // A click is a dot.
        let (x, y) = to_board(pos);
        st.show_active();
        acts.push(Act::Cmd(Cmd::DrawStroke(Stroke { id: id(), uid: me.clone(), c: st.color.clone(), w: st.size, e: st.eraser, p: vec![x, y], l: st.layer, ..Default::default() })));
    }
    // Text tool: a little box where you clicked; Enter places it, Esc cancels.
    if let Some(((x, y), text, focus, layer)) = st.text.as_mut() {
        let at = pos2(paper.left() + *x * paper.width() / BOARD_W, paper.top() + *y * paper.width() / BOARD_W);
        let size = (text_size(st.size) * paper.width() / BOARD_W).max(10.0);
        let color = theme::hex_color(&st.color);
        let mut done: Option<bool> = None;
        egui::Area::new(ui.id().with("draw-text")).fixed_pos(at).order(egui::Order::Foreground).show(ui.ctx(), |ui| {
            let r = ui.add(egui::TextEdit::singleline(text).font(egui::FontId::new(size, theme::hand())).text_color(color).char_limit(200).desired_width(260.0).hint_text("Type, then Enter"));
            if *focus {
                r.request_focus();
                *focus = false;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            } else if r.lost_focus() {
                done = Some(true);
            }
        });
        if let Some(keep) = done {
            let text = text.trim().to_string();
            let (x, y, l) = (*x, *y, *layer);
            st.text = None;
            if keep && !text.is_empty() {
                acts.push(Act::Cmd(Cmd::DrawStroke(Stroke { id: id(), uid: me, c: st.color.clone(), w: st.size, e: false, p: vec![x, y], k: Some("text".into()), text: Some(text), l })));
            }
        }
    }
}

/// Saved drawings: thumbnails under the board (click: open full size).
fn gallery(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        if m.draw.gallery.is_empty() {
            ui.label(egui::RichText::new("Saved drawings show up here.").font(theme::font(theme::small())).color(p.gray));
            return;
        }
        ui.label(egui::RichText::new("Gallery").font(theme::bold_font(theme::small())).color(p.gray));
        egui::ScrollArea::horizontal().id_salt("draw-gallery").max_height(64.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                for d in m.draw.gallery.iter().take(40) {
                    let url = format!("{}/media/{}", m.config.server, d.media);
                    let (r, resp) = ui.allocate_exact_size(vec2(96.0, 60.0), Sense::click());
                    ui.painter().rect_filled(r, 0.0, PAPER);
                    // Nearest-neighbour, like the page; GIFs wiggle on hover.
                    let tex = if resp.hovered() { app.images.get_pixel(&url, true) } else { None }.or_else(|| app.images.get_pixel(&url, false));
                    if let Some((tex, _)) = tex {
                        ui.painter().image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, if resp.hovered() { p.select } else { p.gray.gamma_multiply(0.5) }), egui::StrokeKind::Inside);
                    let who = m.user_name(&d.uid);
                    if theme::tip(resp, format!("{who} · {}\nClick to open full size", super::fmt_stamp(d.ts, app.h12))).clicked() {
                        acts.push(Act::OpenUrl(url));
                    }
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_are_the_webs() {
        let (p, k) = shape_points(Tool::Rect, 10.0, 20.0, 110.0, 220.0);
        assert_eq!(p, vec![10.0, 20.0, 110.0, 20.0, 110.0, 220.0, 10.0, 220.0, 10.0, 20.0]);
        assert_eq!(k.as_deref(), Some("poly"));
        let (p, k) = shape_points(Tool::Ellipse, 0.0, 0.0, 2000.0, 1000.0);
        assert!(k.is_none() && p.len() >= 2 * 26 && p.len() % 2 == 0);
        assert_eq!(text_size(14.0), 112.0_f32.max(110.0));
    }
}
