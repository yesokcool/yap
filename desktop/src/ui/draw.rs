//! Draw: the shared drawing board inside the app (web/js/draw.js). Strokes
//! travel in board units (10000 × 6250); live pieces go out as `draw.seg`,
//! finished strokes as `draw.stroke`. Pen / line / box / circle / text,
//! eraser, sizes, Paint's colour box, undo, clear, the "wiggle" line boil
//! (the same position-seeded noise as the web, so everyone sees the same
//! wobble), name tags at the tip of other people's strokes.
//!
//! Drawings and the board (PROTOCOL.md › Draw drawings and the board): one
//! drawing is on the easel, the same for everyone, and beside it a cork
//! bulletin board holds the pinned ones. New drawing (the one on the easel
//! goes on the board first), a card's "Put on the easel", Save to board,
//! Download (PNG or the wiggling GIF), Share in the chat, and History: look
//! at an earlier version (just you), then go back to it for everyone.
//!
//! WigglyPaint-style pixels: the page is drawn at 640 × 400 with every pixel on
//! or off (`crate::drawing`) and shown blown up with hard pixel edges.
//!
//! Three shared layers (PROTOCOL.md › Draw layers): each is its own sheet
//! (and texture), stacked bottom to top on the paper. Which layer you draw on
//! and which ones you hide are yours alone (never sent).

use super::{Act, App, icons, theme, theme::Pal, theme::RoundRect};
use crate::core::{Cmd, EaselMove, Model};
use crate::drawing::{self, BOARD_H, BOARD_W, LAYERS, PIX_H, PIX_W, Pixels, WIGGLE_FRAMES, text_size};
use crate::protocol::{DrawPage, DrawVersion, Stroke};
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
    /// The drawing it was started on (it stays there if the easel moves meanwhile).
    pg: Option<i64>,
    p: Vec<f32>,
    start: (f32, f32),
    sent: usize,
    last_send: f64,
}

/// Text tool: where it goes (board units), what's typed so far, focus once,
/// the layer and the drawing it goes on.
struct TextBox {
    at: (f32, f32),
    text: String,
    focus: bool,
    l: u8,
    pg: Option<i64>,
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
    text: Option<TextBox>,
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
    /// The popover that's open (Download, History, a card, what was taken down) and what it's under.
    pop: Option<Pop>,
    /// The drawing on the easel last frame: when it changes, what was open closes
    /// and the board scrolls to its card.
    seen_page: Option<i64>,
    scroll_easel: bool,
    /// The cork's texture (tiled).
    cork: Option<egui::TextureHandle>,
    debug_saved: bool,
    debug_pop: Option<String>,
    /// 1: ask for History, 2: waiting for it, then look at the oldest version.
    debug_look: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PopKind {
    Download,
    History,
    Card(i64),
    Down,
}

#[derive(Clone, Copy)]
struct Pop {
    kind: PopKind,
    anchor: Rect,
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
            pop: None,
            seen_page: None,
            scroll_easel: false,
            cork: None,
            debug_saved: false,
            debug_pop: None,
            debug_look: 0,
        };
        // Screenshot aids (debug builds): YAP_DEBUG_DRAW_LAYERS=<active>[:<hidden>…]
        // (1–3, e.g. "3:1" = drawing on layer 3, layer 1 hidden) and
        // YAP_DEBUG_DRAW_CONFIRM=1 (Clear's choices open), YAP_DEBUG_DRAW_POP=
        // download|history|card|down (that popover open; card = the first one on
        // the board) and YAP_DEBUG_DRAW_LOOK=1 (looking at the oldest version).
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
            st.debug_pop = std::env::var("YAP_DEBUG_DRAW_POP").ok();
            st.debug_look = if std::env::var_os("YAP_DEBUG_DRAW_LOOK").is_some() { 1 } else { 0 };
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

    /// The strokes you can see (hidden layers left out): what Download and Share export.
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

/// Bring the committed sheets up to date with the page (the drawing on the
/// easel, or the version you're looking at), and their textures: only the
/// layers that changed are redrawn and uploaded.
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
        let strokes: Vec<&Stroke> = m.draw.shown().iter().filter(|s| drawing::layer(s) == l).collect();
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

/// Draw: tools on top, the easel's bar, the page, and the bulletin board
/// beside it (under it on a narrow window).
pub fn show(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let now = ui.input(|i| i.time);
    let st = &mut app.draw;
    // Wiggle frames cycle ~7×/s (only while Draw is open).
    if st.wiggle {
        if now - st.frame_at >= WIGGLE_SECS {
            st.frame = (st.frame + 1) % WIGGLE_FRAMES;
            st.frame_at = now;
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(WIGGLE_SECS));
    }
    sync_pages(st, ui.ctx(), m);
    // The easel moved (or Draw just opened): what was open was about the drawing
    // before, and the board shows the new one's card.
    if st.seen_page != m.draw.page {
        st.seen_page = m.draw.page;
        st.pop = None;
        st.confirm_clear = false;
        st.scroll_easel = true;
    }
    debug_aids(app, m, acts);
    keys(app, ui, m, acts);
    toolbar(app, ui, m, p, acts);
    ui.add_space(4.0);
    easel_bar(app, ui, m, p, acts);
    ui.add_space(6.0);
    let area = ui.available_rect_before_wrap();
    let (paper, cork_rect, narrow) = layout(area);
    ui.allocate_rect(area, Sense::hover());
    page(app, ui, m, paper, acts);
    cork(app, ui, m, p, cork_rect, narrow, acts);
    popover(app, ui, m, p, acts);
}

/// Where the page and the board go. Wide: the board on the right (a quarter
/// of the width, more when the page can't use it), the two centred together.
/// Narrow: a strip of cards under the page.
fn layout(area: Rect) -> (Rect, Rect, bool) {
    let ratio = PIX_W as f32 / PIX_H as f32;
    let (gap, pad) = (10.0, 6.0);
    if area.width() < 720.0 {
        // The page at the top, the strip right under it.
        let strip = (area.height() * 0.3).clamp(120.0, 170.0);
        let stage = Rect::from_min_max(area.min, pos2(area.right(), area.bottom() - strip - gap)).shrink(pad);
        let w = stage.width().min(stage.height() * ratio).max(10.0);
        let paper = Rect::from_min_size(pos2(stage.center().x - w / 2.0, stage.top()), vec2(w, w / ratio)).round();
        let top = paper.bottom() + pad + gap;
        return (paper, Rect::from_min_max(pos2(area.left(), top), pos2(area.right(), top + strip)).round(), true);
    }
    let mut board_w = (area.width() * 0.27).clamp(200.0, 320.0);
    let w = (area.width() - board_w - gap - 2.0 * pad).min((area.height() - 2.0 * pad) * ratio).max(10.0);
    let spare = (area.width() - (w + 2.0 * pad + gap + board_w)).max(0.0);
    let more = spare.min(420.0 - board_w);
    board_w += more;
    let left = area.left() + (spare - more) / 2.0;
    let paper = Rect::from_min_size(pos2(left + pad, area.top() + pad), vec2(w, w / ratio)).round();
    let board = Rect::from_min_max(pos2(paper.right() + pad + gap, area.top()), pos2(paper.right() + pad + gap + board_w, area.bottom())).round();
    (paper, board, false)
}

/// Screenshot aids (debug builds): YAP_DEBUG_DRAW_SAVE=1 saves the drawing to
/// the board once; YAP_DEBUG_DRAW_LOOK=1 looks at the oldest version.
fn debug_aids(app: &mut App, m: &Model, acts: &mut Vec<Act>) {
    if !cfg!(debug_assertions) || !m.draw.loaded {
        return;
    }
    let st = &mut app.draw;
    if !st.debug_saved && std::env::var_os("YAP_DEBUG_DRAW_SAVE").is_some() {
        st.debug_saved = true;
        acts.push(Act::Cmd(Cmd::DrawSaveToBoard));
    }
    if st.debug_look == 1 {
        st.debug_look = 2;
        acts.push(Act::Cmd(Cmd::DrawHistory));
    } else if st.debug_look == 2
        && let Some(list) = &m.draw.versions
    {
        st.debug_look = 0;
        if let Some(v) = list.iter().rev().find(|v| !v.now) {
            acts.push(Act::Cmd(Cmd::DrawLook(v.id.clone())));
        }
    }
}

fn keys(app: &mut App, ui: &mut egui::Ui, m: &Model, acts: &mut Vec<Act>) {
    if ui.ctx().egui_wants_keyboard_input() || app.modal.is_some() {
        return;
    }
    let st = &mut app.draw;
    // Esc: closes what's open, else stops looking at an earlier version.
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        if st.pop.is_some() || st.confirm_clear {
            st.pop = None;
            st.confirm_clear = false;
        } else if m.draw.preview.is_some() {
            acts.push(Act::Cmd(Cmd::DrawNow));
        }
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) && m.draw.preview.is_none() {
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
        let looking = m.draw.preview.is_some();
        let online = m.status == crate::core::Status::Online;
        if tip(theme::button_sized(ui, "Undo", theme::Btn::Normal, !looking && !m.draw.mine.is_empty(), vec2(56.0, 24.0), p), "Undo your last stroke (Ctrl+Z)").clicked() {
            acts.push(Act::Cmd(Cmd::DrawUndo));
        }
        if app.draw.confirm_clear {
            let l = app.draw.layer;
            // The question and its answers together, on a row of their own
            // (wrapping on a narrow window, so the buttons never fall off it).
            ui.end_row();
            ui.label(theme::text("Clear for everyone? History can bring it back.", p.danger));
            // Both are danger buttons and neither is the default, like the web.
            if theme::button_sized(ui, &format!("Clear layer {}", l + 1), theme::Btn::Danger, true, vec2(56.0, 24.0), p).clicked() {
                app.draw.confirm_clear = false;
                acts.push(if m.draw.strokes.iter().any(|s| drawing::layer(s) == l as usize) { Act::Cmd(Cmd::DrawWipe { l, all: false }) } else { Act::Toast("Nothing to clear.".into()) });
            }
            if theme::button_sized(ui, "Clear the whole drawing", theme::Btn::Danger, true, vec2(56.0, 24.0), p).clicked() {
                app.draw.confirm_clear = false;
                acts.push(if m.draw.strokes.is_empty() { Act::Toast("Nothing to clear.".into()) } else { Act::Cmd(Cmd::DrawWipe { l: 0, all: true }) });
            }
            if theme::button_sized(ui, "Cancel", theme::Btn::Normal, true, vec2(56.0, 24.0), p).clicked() {
                app.draw.confirm_clear = false;
            }
        } else if tip(theme::button_sized(ui, "Clear", theme::Btn::Danger, online && !looking, vec2(56.0, 24.0), p), "Clear a layer or the whole drawing for everyone (History can bring it back)").clicked() {
            app.draw.confirm_clear = true;
            app.draw.pop = None;
        }
    });
}

/// The page: each layer's sheet bottom to top (with the strokes in progress
/// on it), name tags, input. An earlier version you're looking at shows
/// without the strokes in progress, with a yellow edge, and isn't drawn on.
fn page(app: &mut App, ui: &mut egui::Ui, m: &Model, paper: Rect, acts: &mut Vec<Act>) {
    let now = ui.input(|i| i.time);
    let me = m.config.uid.clone();
    let pg = m.draw.page;
    let looking = m.draw.preview.is_some();
    let st = &mut app.draw;
    let frame = st.wiggle.then_some(st.frame);
    let painter = ui.painter().with_clip_rect(paper);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    painter.rect_filled(paper, 0.0, PAPER);
    // Strokes in progress, each on its own layer: others' (name at the tip) and mine.
    let mut live: [Vec<Stroke>; LAYERS] = Default::default();
    let mut tags = Vec::new();
    for l in m.draw.live.values().filter(|l| !looking && l.at.elapsed().as_secs() < 8) {
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
    if looking {
        ui.painter().rect_stroke(paper, 0.0, egui::Stroke::new(4.0, theme::INFO), egui::StrokeKind::Outside);
        ui.painter().rect_stroke(paper.expand(4.0), 0.0, egui::Stroke::new(1.0, Color32::BLACK), egui::StrokeKind::Outside);
    } else {
        ui.painter().rect_stroke(paper, 0.0, egui::Stroke::new(1.0, p.gray.gamma_multiply(0.5)), egui::StrokeKind::Outside);
    }
    if !m.draw.loaded {
        painter.text(paper.center(), Align2::CENTER_CENTER, "Opening Draw…", theme::font(13.0), Color32::GRAY);
    }
    // Input.
    let resp = ui.interact(paper, ui.id().with("draw-paper"), Sense::click_and_drag());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if st.tool == Tool::Text { egui::CursorIcon::Text } else { egui::CursorIcon::Crosshair });
    }
    // Looking at an earlier version: it isn't the real page, so nothing goes on it.
    if looking {
        if resp.drag_started() || resp.clicked() {
            acts.push(Act::Toast("You're looking at an earlier version. Back to now to draw.".into()));
        }
        st.drawing = None;
        st.text = None;
        return;
    }
    // (A drag goes on past the page's edge; the server refuses a stroke with a point more than 500 outside it.)
    let to_board = |pos: Pos2| -> (f32, f32) {
        (((pos.x - paper.left()) / paper.width() * BOARD_W).round().clamp(-500.0, BOARD_W + 500.0), ((pos.y - paper.top()) / paper.height() * BOARD_H).round().clamp(-500.0, BOARD_H + 500.0))
    };
    if st.tool == Tool::Text {
        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            st.show_active();
            st.text = Some(TextBox { at: to_board(pos), text: String::new(), focus: true, l: st.layer, pg });
        }
    } else if resp.drag_started() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let (x, y) = to_board(pos);
            st.show_active();
            st.drawing = Some(Drawing { sk: id(), tool: st.tool, l: st.layer, pg, p: vec![x, y], start: (x, y), sent: 0, last_send: 0.0 });
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
                    if d.p.len() >= crate::drawing::MAX_NUMBERS {
                        // As long as a stroke may be (the server refuses a longer one whole): it's
                        // kept, and the line goes on as a new stroke from here.
                        let full = std::mem::replace(d, Drawing { sk: id(), tool: d.tool, l: d.l, pg: d.pg, p: vec![x, y], start: (x, y), sent: 0, last_send: 0.0 });
                        acts.push(Act::Cmd(Cmd::DrawStroke(Stroke { id: full.sk, uid: me.clone(), c: st.color.clone(), w: st.size, e: st.eraser, p: full.p, k: None, text: None, l: full.l }, full.pg)));
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
                    acts.push(Act::Cmd(Cmd::DrawSeg { sk: d.sk.clone(), c: st.color.clone(), w: st.size, e: st.eraser, p: d.p[from..].to_vec(), l: d.l, pg: d.pg }));
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
        acts.push(Act::Cmd(Cmd::DrawStroke(stroke, d.pg)));
    } else if resp.clicked()
        && st.tool == Tool::Pen
        && let Some(pos) = resp.interact_pointer_pos()
    {
        // A click is a dot.
        let (x, y) = to_board(pos);
        st.show_active();
        acts.push(Act::Cmd(Cmd::DrawStroke(Stroke { id: id(), uid: me.clone(), c: st.color.clone(), w: st.size, e: st.eraser, p: vec![x, y], l: st.layer, ..Default::default() }, pg)));
    }
    // Text tool: a little box where you clicked; Enter places it, Esc cancels.
    if let Some(TextBox { at: (x, y), text, focus, l: layer, pg: on }) = st.text.as_mut() {
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
            let (x, y, l, on) = (*x, *y, *layer, *on);
            st.text = None;
            if keep && !text.is_empty() {
                acts.push(Act::Cmd(Cmd::DrawStroke(Stroke { id: id(), uid: me, c: st.color.clone(), w: st.size, e: false, p: vec![x, y], k: Some("text".into()), text: Some(text), l }, on)));
            }
        }
    }
}

// ── The easel's bar ─────────────────────────────────────────────────────────

/// "Ada", "Ada & Ben", "Ada, Ben +2" (people the server doesn't know are left out).
fn names(m: &Model, uids: &[String]) -> String {
    let names: Vec<&str> = uids.iter().filter_map(|u| m.users.get(u).map(|x| x.name.as_str())).collect();
    if names.len() <= 2 {
        return names.join(" & ");
    }
    format!("{}, {} +{}", names[0], names[1], names.len() - 2)
}

/// Who drew what's on the easel now, in the order they started.
fn artists_now(m: &Model) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in &m.draw.strokes {
        if !out.contains(&s.uid) {
            out.push(s.uid.clone());
        }
    }
    out
}

/// Someone's name, or "someone".
fn who(m: &Model, uid: Option<&str>) -> String {
    uid.and_then(|u| m.users.get(u)).map_or_else(|| "someone".into(), |u| u.name.clone())
}

/// What a version was kept for, like the web.
fn version_label(m: &Model, why: &str, uid: Option<&str>) -> String {
    let name = who(m, uid);
    match why {
        "save" => format!("Saved to the board by {name}"),
        "wipe" => format!("Before {name} cleared a layer"),
        "restore" => format!("Before {name} went back"),
        "leave" => format!("When {name} switched drawings"),
        _ => "Kept while drawing".into(),
    }
}

/// Whether the board has the drawing on the easel as it is.
fn easel_status(m: &Model) -> &'static str {
    match m.draw.easel() {
        None => "",
        Some(p) if p.pinned_at.is_none() => {
            if m.draw.strokes.is_empty() {
                "new"
            } else {
                "not on the board yet"
            }
        }
        Some(p) if p.down_at.is_some() => "taken down from the board",
        Some(p) if p.dirty => "on the board · changed since it was saved",
        Some(_) => "on the board",
    }
}

/// What opened `kind` is here this frame: the popover stays under it (the
/// window may have been resized, or the board scrolled).
fn follow(st: &mut DrawUi, kind: PopKind, anchor: Rect) {
    if let Some(pop) = st.pop.as_mut()
        && pop.kind == kind
    {
        pop.anchor = anchor;
    }
}

/// Open `kind` under `anchor`, or close it when it's the one open.
fn toggle_pop(st: &mut DrawUi, kind: PopKind, anchor: Rect) {
    st.pop = if st.pop.is_some_and(|p| p.kind == kind) { None } else { Some(Pop { kind, anchor }) };
    st.confirm_clear = false;
}

/// Over the page: New drawing, which drawing is on the easel and whether the
/// board has it as it is, then Save to board, Download, Share and History.
/// While you look at an earlier version: a yellow bar saying which, with
/// Go back to this and Back to now.
fn easel_bar(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let online = m.status == crate::core::Status::Online;
    let busy = m.draw.busy;
    let size = vec2(56.0, 24.0);
    if let Some(v) = &m.draw.preview {
        let text = format!("{} — {}. Only you see this.", super::format::fmt_short(v.ts, app.h12), version_label(m, &v.why, v.uid.as_deref()));
        theme::info_bar(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                ui.label(theme::text(text, theme::INFO_TEXT));
                if theme::tip(theme::button_sized(ui, "Go back to this", theme::Btn::Default, online, size, p), "Put the drawing back like this for everyone. What's there now is kept in History.").clicked() {
                    acts.push(Act::Cmd(Cmd::DrawRestore));
                }
                if theme::button_sized(ui, "Back to now", theme::Btn::Normal, true, size, p).clicked() {
                    acts.push(Act::Cmd(Cmd::DrawNow));
                }
            });
        });
        return;
    }
    let st = &mut app.draw;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        let r = theme::tip(theme::button_sized(ui, "+ New drawing", theme::Btn::Normal, online && !busy, size, p), "Start a blank drawing for everyone. This one stays on the board.");
        if r.clicked() {
            st.pop = None;
            acts.push(Act::Cmd(Cmd::DrawMove(EaselMove::New)));
        }
        let title = m.draw.page.map_or_else(|| "Draw".to_string(), |n| format!("Drawing {n}"));
        let status = easel_status(m);
        let mut job = egui::text::LayoutJob::default();
        job.append(&title, 0.0, egui::TextFormat::simple(theme::bold_font(theme::ui()), p.text));
        if !status.is_empty() {
            job.append(&format!(" — {status}"), 0.0, egui::TextFormat::simple(theme::font(theme::ui()), p.gray));
        }
        ui.add(egui::Label::new(job).truncate());
        ui.separator();
        let r = theme::tip(theme::button_sized(ui, "Save to board", theme::Btn::Normal, online && !busy, size, p), "Pin this drawing to the board, or update its picture there");
        if r.clicked() {
            st.pop = None;
            acts.push(Act::Cmd(Cmd::DrawSaveToBoard));
        }
        let r = theme::tip(theme::button_sized(ui, "Download", theme::Btn::Normal, true, size, p), "Save a picture of it to your computer");
        follow(st, PopKind::Download, r.rect);
        if r.clicked() {
            toggle_pop(st, PopKind::Download, r.rect);
        }
        if st.debug_pop.as_deref() == Some("download") {
            st.debug_pop = None;
            st.pop = Some(Pop { kind: PopKind::Download, anchor: r.rect });
        }
        let r = theme::tip(theme::button_sized(ui, "Share", theme::Btn::Normal, online, size, p), "Post this drawing in the chat");
        if r.clicked() {
            st.pop = None;
            let strokes = st.visible(m);
            acts.push(if strokes.is_empty() { Act::Toast("Draw something first.".into()) } else { Act::Cmd(Cmd::DrawShare { strokes, wiggle: st.wiggle }) });
        }
        let r = theme::tip(theme::button_sized(ui, "History", theme::Btn::Normal, online, size, p), "Earlier versions of this drawing: look at one, and go back to it if you want");
        follow(st, PopKind::History, r.rect);
        let debug = st.debug_pop.as_deref() == Some("history") && m.draw.loaded;
        if r.clicked() || debug {
            if debug {
                st.debug_pop = None;
            }
            toggle_pop(st, PopKind::History, r.rect);
            if st.pop.is_some() {
                acts.push(Act::Cmd(Cmd::DrawHistory));
            }
        }
    });
}

// ── The bulletin board ──────────────────────────────────────────────────────

const CORK: Color32 = Color32::from_rgb(0xc0, 0x91, 0x5a);
const WOOD: Color32 = Color32::from_rgb(0x7d, 0x4f, 0x26);
const CARD_PAPER: Color32 = Color32::from_rgb(0xff, 0xfd, 0xf4);
const CARD_GHOST: Color32 = Color32::from_rgb(0xf4, 0xec, 0xd8);
const EASEL_RED: Color32 = Color32::from_rgb(0xc0, 0x00, 0x00);
const PIN_COLORS: [Color32; 6] = [
    Color32::from_rgb(0xe5, 0x48, 0x4d),
    Color32::from_rgb(0x3e, 0x63, 0xdd),
    Color32::from_rgb(0x30, 0xa4, 0x6c),
    Color32::from_rgb(0xf5, 0xd9, 0x0a),
    Color32::from_rgb(0x8e, 0x4e, 0xc6),
    Color32::from_rgb(0xf7, 0x6b, 0x15),
];
const CORK_TILE: usize = 18;

/// Cork, like the web's: small dark and light dots on brown, a few bigger
/// ones, tiled. It's a thing in the room, so the same in light and dark.
fn cork_image() -> egui::ColorImage {
    let mix = |c: Color32, a: f32| -> [u8; 3] {
        let m = |x: u8, y: u8| (x as f32 * (1.0 - a) + y as f32 * a).round() as u8;
        [m(CORK.r(), c.r()), m(CORK.g(), c.g()), m(CORK.b(), c.b())]
    };
    let dark = Color32::from_rgb(92, 52, 16);
    let light = Color32::from_rgb(255, 238, 206);
    let mut rgba = Vec::with_capacity(CORK_TILE * CORK_TILE * 4);
    for y in 0..CORK_TILE {
        for x in 0..CORK_TILE {
            let c = if (x % 6 == 0) && (y % 6 == 0) {
                mix(dark, 0.4)
            } else if (x + 7) % 9 == 0 && (y + 6) % 9 == 0 {
                mix(light, 0.35)
            } else if matches!((x, y), (5, 1) | (6, 1) | (5, 2) | (14, 11) | (14, 12) | (15, 11)) {
                mix(Color32::from_rgb(120, 70, 25), 0.25)
            } else {
                [CORK.r(), CORK.g(), CORK.b()]
            };
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([CORK_TILE, CORK_TILE], &rgba)
}

/// What's on the board, in order.
enum Item {
    Card(DrawPage),
    /// Nothing pinned yet: how it works.
    Empty,
    /// "Taken down (n)".
    Down(usize),
}

/// The drawings pinned to the board, newest first. The one on the easel shows
/// as it is right now (first, when it isn't pinned), and what was taken down
/// is behind a note at the end.
fn items(m: &Model) -> Vec<Item> {
    let d = &m.draw;
    let pinned: Vec<&DrawPage> = d.board.iter().filter(|p| p.pinned_at.is_some() && p.down_at.is_none()).collect();
    let mut out = Vec::new();
    if let Some(id) = d.page
        && !pinned.iter().any(|p| p.id == id)
    {
        out.push(Item::Card(d.easel().cloned().unwrap_or(DrawPage { id, ..Default::default() })));
    }
    out.extend(pinned.into_iter().cloned().map(Item::Card));
    if !d.board.iter().any(|p| p.pinned_at.is_some() && p.down_at.is_none()) {
        out.push(Item::Empty);
    }
    let down = d.board.iter().filter(|p| p.down_at.is_some() && Some(p.id) != d.page).count();
    if down > 0 {
        out.push(Item::Down(down));
    }
    out
}

/// The board: cork in a wooden frame, a grid of cards (a strip on a narrow window).
fn cork(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, rect: Rect, narrow: bool, acts: &mut Vec<Act>) {
    if rect.width() < 40.0 || rect.height() < 40.0 {
        return;
    }
    let st = &mut app.draw;
    if st.cork.is_none() {
        let opts = egui::TextureOptions { wrap_mode: egui::TextureWrapMode::Repeat, ..egui::TextureOptions::NEAREST };
        st.cork = Some(ui.ctx().load_texture("draw-cork", cork_image(), opts));
    }
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, WOOD);
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, Color32::from_rgb(0x3b, 0x22, 0x0c)), egui::StrokeKind::Inside);
    let inner = rect.shrink(7.0);
    if let Some(t) = &st.cork {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(inner.width() / CORK_TILE as f32, inner.height() / CORK_TILE as f32));
        painter.image(t.id(), inner, uv, Color32::WHITE);
    }
    // The cork sits a little under the frame.
    let shade = Color32::from_black_alpha(90);
    painter.rect_filled(Rect::from_min_size(inner.min, vec2(inner.width(), 3.0)), 0.0, shade);
    painter.rect_filled(Rect::from_min_size(inner.min, vec2(3.0, inner.height())), 0.0, shade);
    let list = items(m);
    let frame = if st.wiggle { st.frame as usize } else { 0 };
    let mut debug_card = st.debug_pop.as_deref() == Some("card") && m.draw.loaded;
    let debug_down = st.debug_pop.as_deref() == Some("down") && m.draw.loaded;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        ui.set_clip_rect(inner.intersect(ui.clip_rect()));
        let mut place = |ui: &mut egui::Ui, item: &Item, r: Rect, st: &mut DrawUi| match item {
            Item::Card(page) => {
                let resp = card(ui, st, m, page, r, frame, &app.images);
                follow(st, PopKind::Card(page.id), r);
                if st.scroll_easel && Some(page.id) == m.draw.page {
                    st.scroll_easel = false;
                    ui.scroll_to_rect(r.expand(14.0), None);
                }
                if resp.clicked() || std::mem::take(&mut debug_card) {
                    toggle_pop(st, PopKind::Card(page.id), r);
                }
            }
            Item::Empty => {
                ui.painter().rect_filled(r.translate(vec2(1.0, 2.0)), 0.0, Color32::from_black_alpha(80));
                ui.painter().rect_filled(r, 0.0, CARD_PAPER);
                let g = ui.painter().layout(
                    "Save to board pins the drawing on the easel here. Click a pinned one to put it back on the easel.".into(),
                    theme::font(theme::small()),
                    Color32::from_gray(0x33),
                    r.width() - 16.0,
                );
                ui.painter().galley(r.min + vec2(8.0, 8.0), g, Color32::from_gray(0x33));
            }
            Item::Down(n) => {
                let resp = ui.interact(r, ui.id().with("draw-down-list"), Sense::click());
                follow(st, PopKind::Down, r);
                ui.painter().rect_filled(r.translate(vec2(1.0, 1.0)), 0.0, Color32::from_black_alpha(80));
                ui.painter().rect_filled(r, 0.0, CARD_PAPER.gamma_multiply(if resp.hovered() { 1.0 } else { 0.92 }));
                ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("Taken down ({n})"), theme::font(theme::small()), Color32::from_gray(0x33));
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if theme::tip(resp, "Drawings taken off the board: put one back").clicked() || debug_down {
                    toggle_pop(st, PopKind::Down, r);
                }
            }
        };
        if narrow {
            egui::ScrollArea::horizontal().id_salt("draw-cork").auto_shrink([false, false]).show(ui, |ui| {
                let h = inner.height();
                let (cw, gap, top) = (116.0, 12.0, 16.0);
                let card_h = (h - top - 10.0).min(card_height(cw));
                let width: f32 = list.iter().map(|i| match i {
                    Item::Card(_) => cw,
                    Item::Empty => 260.0,
                    Item::Down(_) => 110.0,
                }).sum::<f32>()
                    + gap * (list.len() as f32 + 1.0);
                let (row, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
                let mut x = row.left() + gap;
                for item in &list {
                    let w = match item {
                        Item::Card(_) => cw,
                        Item::Empty => 260.0,
                        Item::Down(_) => 110.0,
                    };
                    let r = match item {
                        Item::Card(_) => Rect::from_min_size(pos2(x, row.top() + top), vec2(w, card_h)),
                        Item::Empty => Rect::from_min_size(pos2(x, row.top() + top), vec2(w, 56.0)),
                        Item::Down(_) => Rect::from_center_size(pos2(x + w / 2.0, row.center().y), vec2(w, 22.0)),
                    };
                    place(ui, item, r.round(), st);
                    x += w + gap;
                }
            });
        } else {
            egui::ScrollArea::vertical().id_salt("draw-cork").auto_shrink([false, false]).show(ui, |ui| {
                let avail = ui.available_width();
                let (pad, gap_x, gap_y) = (12.0, 10.0, 20.0);
                let cols = ((avail - 2.0 * pad + gap_x) / (100.0 + gap_x)).floor().max(1.0);
                let cw = ((avail - 2.0 * pad - (cols - 1.0) * gap_x) / cols).floor();
                let ch = card_height(cw);
                ui.add_space(18.0);
                let cards: Vec<&Item> = list.iter().filter(|i| matches!(i, Item::Card(_))).collect();
                for row in cards.chunks(cols as usize) {
                    let (r, _) = ui.allocate_exact_size(vec2(avail, ch), Sense::hover());
                    for (i, item) in row.iter().enumerate() {
                        let at = Rect::from_min_size(pos2(r.left() + pad + i as f32 * (cw + gap_x), r.top()), vec2(cw, ch));
                        place(ui, item, at.round(), st);
                    }
                    ui.add_space(gap_y);
                }
                for item in list.iter().filter(|i| !matches!(i, Item::Card(_))) {
                    let h = if matches!(item, Item::Empty) { 70.0 } else { 22.0 };
                    let (r, _) = ui.allocate_exact_size(vec2(avail, h), Sense::hover());
                    let at = if matches!(item, Item::Empty) { Rect::from_min_max(pos2(r.left() + pad, r.top()), pos2(r.right() - pad, r.bottom())) } else { Rect::from_center_size(r.center(), vec2(120.0, h)) };
                    place(ui, item, at.round(), st);
                    ui.add_space(12.0);
                }
            });
        }
    });
    if (debug_down || !debug_card) && matches!(st.debug_pop.as_deref(), Some("card" | "down")) && m.draw.loaded {
        st.debug_pop = None;
    }
    // (No card for the easel: nothing to scroll to.)
    st.scroll_easel = false;
    let _ = (p, acts);
}

/// A card's height for a width: the photo (8:5), its caption and date.
fn card_height(w: f32) -> f32 {
    5.0 + ((w - 10.0) * 5.0 / 8.0).round() + 4.0 + 15.0 + 13.0 + 4.0
}

/// A polaroid on the cork, pinned: its picture (the one on the easel as it
/// is now), "Drawing N · who", and when (or "● On the easel").
fn card(ui: &mut egui::Ui, st: &DrawUi, m: &Model, page: &DrawPage, rect: Rect, frame: usize, images: &super::images::Images) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(("draw-card", page.id)), Sense::click());
    let easel = Some(page.id) == m.draw.page;
    let ghost = easel && (page.pinned_at.is_none() || page.down_at.is_some());
    let lift = if resp.hovered() { vec2(-1.0, -1.0) } else { vec2(0.0, 0.0) };
    let r = rect.translate(lift);
    let painter = ui.painter();
    let shadow = if resp.hovered() { 4.0 } else { 2.0 };
    painter.rect_filled(r.translate(vec2(shadow / 2.0, shadow)), 0.0, Color32::from_black_alpha(if resp.hovered() { 110 } else { 90 }));
    painter.rect_filled(r, 0.0, if ghost { CARD_GHOST } else { CARD_PAPER });
    if easel {
        painter.rect_stroke(r, 0.0, egui::Stroke::new(2.0, EASEL_RED), egui::StrokeKind::Outside);
    }
    let photo = Rect::from_min_size(r.min + vec2(5.0, 5.0), vec2(r.width() - 10.0, ((r.width() - 10.0) * 5.0 / 8.0).round()));
    painter.rect_filled(photo, 0.0, PAPER);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let url = page.media.as_ref().map(|id| format!("{}/media/{id}", m.config.server));
    if easel && m.draw.preview.is_none() {
        // The drawing on the easel, live: every layer's sheet.
        for l in 0..LAYERS {
            if let Some(t) = st.pages.tex.get(frame * LAYERS + l).or_else(|| st.pages.tex.get(l)) {
                painter.image(t.id(), photo, uv, Color32::WHITE);
            }
        }
    } else if let Some(url) = &url {
        // Nearest-neighbour, like the page; a GIF wiggles while pointed at.
        let tex = if resp.hovered() { images.get_pixel(url, true) } else { None }.or_else(|| images.get_pixel(url, false));
        if let Some((tex, _)) = tex {
            painter.image(tex, photo, uv, Color32::WHITE);
        }
    } else if !easel {
        painter.text(photo.center(), Align2::CENTER_CENTER, "No picture yet", theme::font(10.0), Color32::from_gray(0x77));
    }
    painter.rect_stroke(photo, 0.0, egui::Stroke::new(1.0, Color32::from_black_alpha(50)), egui::StrokeKind::Outside);
    let by = names(m, &if easel { artists_now(m) } else { page.artists.clone() });
    let ink = Color32::from_gray(0x1a);
    let mut job = egui::text::LayoutJob::default();
    job.append(&format!("Drawing {}", page.id), 0.0, egui::TextFormat::simple(theme::bold_font(11.0), ink));
    if !by.is_empty() {
        job.append(&format!(" · {by}"), 0.0, egui::TextFormat::simple(theme::font(11.0), ink));
    }
    job.wrap = egui::text::TextWrapping::truncate_at_width(r.width() - 10.0);
    let g = painter.layout_job(job);
    painter.galley(pos2(r.left() + 5.0, photo.bottom() + 4.0), g, ink);
    let (date, color, font) = if easel {
        ("● On the easel".to_string(), EASEL_RED, theme::bold_font(10.0))
    } else {
        let ts = page.saved_at.or(page.pinned_at).unwrap_or(page.created_at);
        (super::format::local_time(ts).format("%b %-d").to_string(), Color32::from_gray(0x66), theme::font(10.0))
    };
    painter.text(pos2(r.left() + 5.0, photo.bottom() + 19.0), Align2::LEFT_TOP, date, font, color);
    // The pushpin.
    let pin = pos2(r.center().x, r.top());
    let c = PIN_COLORS[page.id.rem_euclid(PIN_COLORS.len() as i64) as usize];
    painter.circle_filled(pin + vec2(1.0, 2.0), 7.0, Color32::from_black_alpha(110));
    painter.circle_filled(pin, 7.0, c);
    painter.circle_filled(pin + vec2(-2.0, -2.0), 2.0, Color32::from_white_alpha(200));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let label = format!("Drawing {}{}{}", page.id, if by.is_empty() { String::new() } else { format!(" by {by}") }, if easel { ", on the easel" } else { "" });
    theme::tip(resp, label)
}

// ── Popovers ────────────────────────────────────────────────────────────────

/// A version's or a taken-down drawing's little picture (hatched when it has none).
fn thumb(ui: &mut egui::Ui, images: &super::images::Images, m: &Model, media: Option<&str>) {
    let (r, _) = ui.allocate_exact_size(vec2(48.0, 30.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(r, 0.0, PAPER);
    match media.and_then(|id| images.get_pixel(&format!("{}/media/{id}", m.config.server), false)) {
        Some((tex, _)) => {
            painter.image(tex, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            let hatch = egui::Stroke::new(1.0, Color32::from_gray(0xcc));
            let clip = painter.with_clip_rect(r);
            let mut x = r.left() - r.height();
            while x < r.right() {
                clip.line_segment([pos2(x, r.bottom()), pos2(x + r.height(), r.top())], hatch);
                x += 6.0;
            }
        }
    }
    painter.rect_stroke(r, 0.0, egui::Stroke::new(1.0, Color32::from_black_alpha(60)), egui::StrokeKind::Outside);
}

fn note(ui: &mut egui::Ui, text: impl Into<String>, p: &Pal) {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).font(theme::font(theme::small())).color(p.gray)).wrap());
}

/// The open popover: Download, History, a card's choices, or what was taken down.
fn popover(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let Some(pop) = app.draw.pop else { return };
    let ctx = ui.ctx().clone();
    let width = match pop.kind {
        PopKind::History => 300.0,
        PopKind::Down => 290.0,
        _ => 250.0,
    };
    // Under what opened it, toward the middle of the window.
    let x = if pop.anchor.center().x > ctx.content_rect().center().x { pop.anchor.right() - width } else { pop.anchor.left() };
    let pos = pos2(x, pop.anchor.bottom() + 4.0);
    let title = match pop.kind {
        PopKind::Download => "Download".to_string(),
        PopKind::History => m.draw.page.map_or_else(|| "History".into(), |n| format!("History of Drawing {n}")),
        PopKind::Card(id) => format!("Drawing {id}"),
        PopKind::Down => "Taken down".into(),
    };
    let mut close = false;
    let size = vec2(56.0, 23.0);
    let area = egui::Area::new(egui::Id::new("draw-pop")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(&ctx, |ui| {
        super::sidebar::popup_frame(p).show(ui, |ui| {
            ui.set_width(width);
            let (t, _) = ui.allocate_exact_size(vec2(width, theme::TITLE_H), Sense::hover());
            theme::paint_title(ui.painter(), t, p, None, &title);
            egui::Frame::new().inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                match pop.kind {
                    PopKind::Download => {
                        ui.horizontal(|ui| {
                            for (label, gif) in [("Picture (PNG)", false), ("Wiggling GIF", true)] {
                                if theme::button_sized(ui, label, theme::Btn::Normal, true, size, p).clicked() {
                                    close = true;
                                    let strokes = app.draw.visible(m);
                                    acts.push(if strokes.is_empty() { Act::Toast("Draw something first.".into()) } else { Act::Cmd(Cmd::DrawDownload { strokes, gif, wiggle: app.draw.wiggle }) });
                                }
                            }
                        });
                        if app.draw.hidden.iter().any(|h| *h) {
                            note(ui, "What you see: hidden layers are left out.", p);
                        }
                    }
                    PopKind::History => history_list(app, ui, m, p, acts, &mut close),
                    PopKind::Card(id) => match m.draw.board.iter().find(|x| x.id == id).cloned().or_else(|| (Some(id) == m.draw.page).then(|| DrawPage { id, ..Default::default() })) {
                        Some(page) => card_choices(app, ui, m, p, acts, &page, &mut close),
                        None => close = true,
                    },
                    PopKind::Down => {
                        note(ui, "Drawings taken off the board. Put one back to pin it where it was.", p);
                        let down: Vec<&DrawPage> = m.draw.board.iter().filter(|x| x.down_at.is_some() && Some(x.id) != m.draw.page).collect();
                        if down.is_empty() {
                            close = true;
                        }
                        theme::well(ui, Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), (down.len() as f32 * 38.0 + 6.0).min(300.0))), p, |ui| {
                            egui::ScrollArea::vertical().id_salt("draw-down").show(ui, |ui| {
                                for page in down {
                                    ui.horizontal(|ui| {
                                        thumb(ui, &app.images, m, page.media.as_deref());
                                        ui.vertical(|ui| {
                                            ui.spacing_mut().item_spacing.y = 0.0;
                                            ui.label(egui::RichText::new(format!("Drawing {}", page.id)).font(theme::bold_font(theme::small())).color(p.text));
                                            let when = page.down_at.map(|t| super::format::fmt_short(t, app.h12)).unwrap_or_default();
                                            ui.label(egui::RichText::new(format!("by {}, {when}", who(m, page.down_by.as_deref()))).font(theme::font(theme::small())).color(p.gray));
                                        });
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if theme::button_sized(ui, "Put back", theme::Btn::Normal, true, vec2(56.0, 22.0), p).clicked() {
                                                acts.push(Act::Cmd(Cmd::DrawPin { page: page.id, on: true }));
                                                close = true;
                                            }
                                        });
                                    });
                                }
                            });
                        });
                    }
                }
            });
            theme::menu_bevel(ui, p);
        });
    });
    let pressed_outside = ctx.input(|i| i.pointer.any_pressed()) && {
        let at = ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default();
        !area.response.rect.contains(at) && !pop.anchor.contains(at)
    };
    if close || pressed_outside {
        app.draw.pop = None;
    }
}

/// History: the versions, newest first; one click looks at one.
fn history_list(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, close: &mut bool) {
    let Some(list) = &m.draw.versions else {
        note(ui, "Loading…", p);
        return;
    };
    if list.is_empty() {
        note(ui, "Nothing kept yet. Versions are kept every few minutes while people draw, when it's saved to the board, and before a layer is cleared.", p);
        return;
    }
    note(ui, "Pick one to look at it (only you see it), then go back to it for everyone if you want.", p);
    let h = (list.len() as f32 * 38.0 + 6.0).min(320.0);
    theme::well(ui, Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), h)), p, |ui| {
        egui::ScrollArea::vertical().id_salt("draw-versions").show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for v in list {
                version_row(app, ui, m, p, v, acts, close);
            }
        });
    });
}

/// One version: its picture (if it was saved with one), when, why, how many strokes.
fn version_row(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, v: &DrawVersion, acts: &mut Vec<Act>, close: &mut bool) {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 36.0), if v.now { Sense::hover() } else { Sense::click() });
    let hot = !v.now && resp.hovered();
    if hot {
        ui.painter().rect_filled(r, 0.0, p.select);
    }
    let (fg, dim) = if hot { (p.on_select, p.on_select) } else { (p.text, p.gray) };
    ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink2(vec2(4.0, 3.0))).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        thumb(ui, &app.images, m, v.media.as_deref());
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut job = egui::text::LayoutJob::default();
            job.append(&super::format::fmt_short(v.ts, app.h12), 0.0, egui::TextFormat::simple(theme::bold_font(theme::small()), fg));
            if v.now {
                job.append("  Now", 0.0, egui::TextFormat::simple(theme::bold_font(theme::small()), p.ok));
            }
            ui.label(job);
            let strokes = if v.n == 1 { "stroke" } else { "strokes" };
            ui.label(egui::RichText::new(format!("{} · {} {strokes}", version_label(m, &v.why, v.uid.as_deref()), v.n)).font(theme::font(theme::small())).color(dim));
        });
    });
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let tip = if v.now { "This is what's on the easel now" } else { "Look at this version (only you see it)" };
    if theme::tip(resp, tip).clicked() && !v.now {
        acts.push(Act::Cmd(Cmd::DrawLook(v.id.clone())));
        *close = true;
    }
}

/// A card's choices: put it on the easel, download it, take it down (or put it back).
fn card_choices(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, page: &DrawPage, close: &mut bool) {
    let easel = Some(page.id) == m.draw.page;
    let by = names(m, &if easel { artists_now(m) } else { page.artists.clone() });
    let saved = page.saved_at.map_or_else(|| "not saved yet".to_string(), |t| format!("saved {}", super::format::fmt_short(t, app.h12)));
    note(ui, if by.is_empty() { saved } else { format!("By {by} · {saved}") }, p);
    let size = vec2(56.0, 23.0);
    let online = m.status == crate::core::Status::Online;
    ui.horizontal_wrapped(|ui| {
        if page.down_at.is_some() {
            if theme::button_sized(ui, "Put back on the board", theme::Btn::Normal, online, size, p).clicked() {
                acts.push(Act::Cmd(Cmd::DrawPin { page: page.id, on: true }));
                *close = true;
            }
            return;
        }
        if !easel && theme::button_sized(ui, "Put on the easel", theme::Btn::Default, online && !m.draw.busy, size, p).clicked() {
            acts.push(Act::Cmd(Cmd::DrawMove(EaselMove::Open(page.id))));
            *close = true;
        }
        if (easel || page.media.is_some()) && theme::button_sized(ui, "Download", theme::Btn::Normal, true, size, p).clicked() {
            *close = true;
            if easel {
                let strokes = app.draw.visible(m);
                acts.push(if strokes.is_empty() { Act::Toast("Draw something first.".into()) } else { Act::Cmd(Cmd::DrawDownload { strokes, gif: false, wiggle: app.draw.wiggle }) });
            } else if let Some(media) = &page.media {
                acts.push(Act::Cmd(Cmd::DrawDownloadPinned { page: page.id, media: media.clone() }));
            }
        }
        if page.pinned_at.is_some() && theme::button_sized(ui, "Take down", theme::Btn::Normal, online, size, p).clicked() {
            acts.push(Act::Cmd(Cmd::DrawPin { page: page.id, on: false }));
            *close = true;
        }
    });
    let hint = if easel {
        "It's on the easel now.".to_string()
    } else if page.down_at.is_some() {
        format!("Taken down by {}, {}.", who(m, page.down_by.as_deref()), page.down_at.map(|t| super::format::fmt_short(t, app.h12)).unwrap_or_default())
    } else {
        format!("Everyone on Draw switches to it. Drawing {}, on the easel now, stays on the board.", m.draw.page.unwrap_or_default())
    };
    note(ui, hint, p);
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
