//! Doodle reactions (PROTOCOL.md › Extras › Doodle reactions): a small pixel
//! box (48 × 48, drawn at 2×, up to 3 wiggle frames like Draw), saved as a
//! GIF or PNG (≤ 64 KB, ≤ 256 × 256), uploaded to `/api/doodle`, then sent as
//! a `<d:ID>` reaction.
//!
//! The picture is on white, like Draw's paper: a reaction sits on light and
//! dark chat backgrounds alike, and dark ink on a transparent doodle would
//! vanish in the dark theme.

use super::dialogs::{actions, note, window};
use super::{Act, theme, theme::Pal};
use crate::core::{Cmd, Model};
use eframe::egui::{self, Color32, Rect, Sense, pos2, vec2};

/// The canvas, in doodle pixels.
pub const SIDE: usize = 48;
/// Output pixels per doodle pixel ("drawn at 2×").
pub const SCALE: usize = 2;
/// The saved image's side (96).
pub const OUT: usize = SIDE * SCALE;
const FRAMES: u8 = 3;
const FRAME_MS: u32 = 140;
/// The server's limits.
pub const MAX_BYTES: usize = 64 * 1024;
#[cfg(test)]
const MAX_PX: usize = 256;
/// On screen: each doodle pixel this many points.
const CELL: f32 = 6.0;
/// The wiggle: how far a point moves (doodle pixels) and the noise's grain.
const WIGGLE: f32 = 0.55;
const GRAIN: f32 = 36.0;

const PAPER: [u8; 3] = [255, 255, 255];
/// A small Paint box.
const COLORS: [&str; 10] = ["#000000", "#808080", "#e5484d", "#f76b15", "#ffd166", "#30a46c", "#0090ff", "#8e4ec6", "#d6409f", "#8d5524"];
const SIZES: [(f32, &str); 3] = [(1.0, "S"), (2.0, "M"), (4.0, "L")];

/// One stroke: points in doodle pixels (0..48); the eraser paints paper.
#[derive(Clone, Debug)]
pub struct Line {
    pub color: [u8; 3],
    pub width: f32,
    pub pts: Vec<(f32, f32)>,
}

/// The doodle box for one message.
pub struct DoodleBox {
    /// The message it reacts to.
    pub message: String,
    pub lines: Vec<Line>,
    color: usize,
    size: f32,
    eraser: bool,
    pub wiggle: bool,
    drawing: bool,
    /// Bumped on every change; the textures follow.
    version: u64,
    shown: u64,
    tex: Vec<egui::TextureHandle>,
}

impl DoodleBox {
    pub fn new(message: String) -> Self {
        Self { message, lines: Vec::new(), color: 0, size: SIZES[1].0, eraser: false, wiggle: true, drawing: false, version: 1, shown: 0, tex: Vec::new() }
    }
}

impl DoodleBox {
    /// Screenshot aid (debug builds): a smiley already drawn.
    pub fn sample() -> Self {
        let mut d = Self::new(String::new());
        let ring = |cx: f32, cy: f32, r: f32, from: f32, to: f32| (0..=24).map(|i| from + (to - from) * i as f32 / 24.0).map(|a: f32| (cx + r * a.cos(), cy + r * a.sin())).collect::<Vec<_>>();
        d.lines.push(Line { color: hex(COLORS[4]), width: 4.0, pts: ring(24.0, 24.0, 16.0, 0.0, std::f32::consts::TAU) });
        d.lines.push(Line { color: hex(COLORS[0]), width: 2.0, pts: ring(24.0, 25.0, 9.0, 0.5, 2.6) });
        d.lines.push(Line { color: hex(COLORS[0]), width: 2.0, pts: vec![(18.0, 19.0), (18.0, 21.0)] });
        d.lines.push(Line { color: hex(COLORS[0]), width: 2.0, pts: vec![(30.0, 19.0), (30.0, 21.0)] });
        d.lines.push(Line { color: hex(COLORS[2]), width: 1.0, pts: vec![(6.0, 42.0), (12.0, 38.0), (18.0, 43.0)] });
        d
    }
}

fn hex(c: &str) -> [u8; 3] {
    crate::drawing::hex(c)
}

/// The same spot always moves the same way in a frame (Draw's position-seeded noise).
fn wobble(x: f32, y: f32, frame: u8) -> (f32, f32) {
    let f = |axis| crate::drawing::field(x * GRAIN, y * GRAIN, frame, axis) / 22.0 * WIGGLE;
    (x + f(0), y + f(1))
}

/// RGBA, OUT × OUT, on paper; `frame`: a wiggle frame (None = straight).
pub fn render(lines: &[Line], frame: Option<u8>) -> Vec<u8> {
    let mut px = vec![255u8; OUT * OUT * 4];
    for l in lines {
        let pts: Vec<(f32, f32)> = l
            .pts
            .iter()
            .map(|&(x, y)| match frame {
                Some(f) => wobble(x, y, f),
                None => (x, y),
            })
            .map(|(x, y)| (x * SCALE as f32, y * SCALE as f32))
            .collect();
        let width = l.width * SCALE as f32;
        match pts.len() {
            0 => {}
            1 => capsule(&mut px, pts[0], pts[0], width, l.color),
            _ => {
                for w in pts.windows(2) {
                    capsule(&mut px, w[0], w[1], width, l.color);
                }
            }
        }
    }
    px
}

/// A round-capped segment, every pixel on or off (like Draw).
fn capsule(px: &mut [u8], a: (f32, f32), b: (f32, f32), width: f32, c: [u8; 3]) {
    let r = width / 2.0 + 0.07;
    let (x0, x1) = ((a.0.min(b.0) - r).floor() as i32, (a.0.max(b.0) + r).ceil() as i32);
    let (y0, y1) = ((a.1.min(b.1) - r).floor() as i32, (a.1.max(b.1) + r).ceil() as i32);
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    for y in y0.max(0)..=y1.min(OUT as i32 - 1) {
        for x in x0.max(0)..=x1.min(OUT as i32 - 1) {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let t = if len2 > 0.0 { (((cx - a.0) * dx + (cy - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let (qx, qy) = (a.0 + dx * t - cx, a.1 + dy * t - cy);
            if qx * qx + qy * qy <= r * r {
                let i = (y as usize * OUT + x as usize) * 4;
                px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
    }
}

/// The file to upload: a looping 3-frame GIF with the wiggle, else a PNG.
pub fn encode(lines: &[Line], wiggle: bool) -> Result<Vec<u8>, String> {
    let bytes = if wiggle {
        let frames: Vec<Vec<u8>> = (0..FRAMES).map(|f| render(lines, Some(f))).collect();
        crate::drawing::encode_gif(&frames, OUT, OUT, FRAME_MS)?
    } else {
        let img = image::RgbaImage::from_raw(OUT as u32, OUT as u32, render(lines, None)).ok_or("bad doodle")?;
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img).to_rgb8().write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        out
    };
    if bytes.len() > MAX_BYTES {
        return Err("That doodle is too detailed to send (over 64 KB).".into());
    }
    Ok(bytes)
}

/// The dialog's inside (drawn in a Modal by `dialogs::modals`).
pub fn show(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, d: &mut DoodleBox, keep: &mut bool) {
    let now = ui.input(|i| i.time);
    let mut k = true;
    let canvas = SIDE as f32 * CELL;
    window(ui, p, "Draw a reaction", Some("brush"), canvas + 20.0 + 64.0, Some(&mut k), |ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        // Colours, then sizes and tools.
        ui.horizontal_wrapped(|ui| {
            for (i, c) in COLORS.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                let on = !d.eraser && d.color == i;
                ui.painter().rect_filled(r.shrink(2.0), 0.0, theme::hex_color(c));
                let edge = if on || resp.hovered() { p.text } else { p.gray.gamma_multiply(0.5) };
                ui.painter().rect_stroke(r.shrink(if on { 0.5 } else { 2.0 }), 0.0, egui::Stroke::new(if on { 2.0 } else { 1.0 }, edge), egui::StrokeKind::Inside);
                if theme::tip(resp, *c).clicked() {
                    d.color = i;
                    d.eraser = false;
                }
            }
        });
        ui.horizontal(|ui| {
            for (w, label) in SIZES {
                let on = (d.size - w).abs() < 0.1;
                if theme::tip(theme::toggle_button(ui, label, on, vec2(28.0, 24.0), p), format!("Pen {label}")).clicked() {
                    d.size = w;
                }
            }
            ui.add_space(4.0);
            if theme::tip(theme::toggle_button(ui, "Eraser", d.eraser, vec2(60.0, 24.0), p), "Paint the paper back").clicked() {
                d.eraser = !d.eraser;
            }
            if theme::tip(theme::toggle_button(ui, "Wiggle", d.wiggle, vec2(60.0, 24.0), p), "Wobbly lines, like Draw (sent as a little GIF)").clicked() {
                d.wiggle = !d.wiggle;
                d.version += 1;
            }
        });
        // The canvas.
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), canvas + 4.0), Sense::hover());
        let rect = Rect::from_center_size(row.center(), vec2(canvas, canvas));
        let resp = ui.interact(rect, ui.id().with("doodle-canvas"), Sense::drag());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.is_pointer_button_down_on()
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        {
            let c = (((pos.x - rect.left()) / CELL).clamp(0.0, SIDE as f32), ((pos.y - rect.top()) / CELL).clamp(0.0, SIDE as f32));
            if !d.drawing {
                d.drawing = true;
                let color = if d.eraser { PAPER } else { hex(COLORS[d.color]) };
                let width = if d.eraser { d.size.max(2.0) * 1.5 } else { d.size };
                d.lines.push(Line { color, width, pts: vec![c] });
                d.version += 1;
            } else if let Some(l) = d.lines.last_mut()
                && l.pts.last().is_none_or(|q| (q.0 - c.0).hypot(q.1 - c.1) >= 0.35)
                && l.pts.len() < 2000
            {
                l.pts.push(c);
                d.version += 1;
            }
        } else {
            d.drawing = false;
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) && d.lines.pop().is_some() {
            d.version += 1;
        }
        // Textures follow the strokes (each wiggle frame its own, crisp pixels).
        if d.shown != d.version || d.tex.is_empty() {
            d.shown = d.version;
            let frames: Vec<Option<u8>> = if d.wiggle { (0..FRAMES).map(Some).collect() } else { vec![None] };
            d.tex = frames
                .into_iter()
                .enumerate()
                .map(|(i, f)| ui.ctx().load_texture(format!("doodle-{i}"), egui::ColorImage::from_rgba_unmultiplied([OUT, OUT], &render(&d.lines, f)), egui::TextureOptions::NEAREST))
                .collect();
        }
        let frame = if d.tex.len() > 1 { ((now * 1000.0) as u64 / FRAME_MS as u64) as usize % d.tex.len() } else { 0 };
        if d.tex.len() > 1 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(FRAME_MS as u64));
        }
        if let Some(t) = d.tex.get(frame) {
            ui.painter().image(t.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        theme::bevel(ui.painter(), rect.expand(2.0), p, theme::Bevel::Sunken);
        if d.lines.is_empty() {
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Draw here", theme::font(theme::ui()), Color32::from_gray(170));
        }
        ui.horizontal(|ui| {
            if theme::tip(theme::button_sized(ui, "Undo", theme::Btn::Normal, !d.lines.is_empty(), vec2(56.0, 23.0), p), "Undo the last stroke (Ctrl/⌘+Z)").clicked() {
                d.lines.pop();
                d.version += 1;
            }
            if theme::button_sized(ui, "Clear", theme::Btn::Normal, !d.lines.is_empty(), vec2(56.0, 23.0), p).clicked() {
                d.lines.clear();
                d.version += 1;
            }
            note(ui, if d.wiggle { "It wiggles, like Draw." } else { "A still picture." }, p);
        });
        let ok = !d.lines.is_empty() && m.status == crate::core::Status::Online;
        actions(ui, |ui| {
            if theme::button(ui, "Cancel", theme::Btn::Normal, true, p).clicked() {
                *keep = false;
            }
            if super::primary_button(ui, "React with it", p, ok).clicked() {
                match encode(&d.lines, d.wiggle) {
                    Ok(bytes) => {
                        acts.push(Act::Cmd(Cmd::Doodle { message: d.message.clone(), bytes }));
                        *keep = false;
                    }
                    Err(e) => acts.push(Act::Toast(e)),
                }
            }
        });
    });
    *keep &= k;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scribble() -> Vec<Line> {
        let mut lines = vec![];
        for i in 0..6 {
            let pts = (0..40).map(|k| (4.0 + k as f32, 6.0 + i as f32 * 7.0 + ((k as f32) * 0.7).sin() * 3.0)).collect();
            lines.push(Line { color: hex(COLORS[i % COLORS.len()]), width: SIZES[i % 3].0, pts });
        }
        lines.push(Line { color: PAPER, width: 3.0, pts: vec![(10.0, 10.0), (30.0, 30.0)] });
        lines
    }

    #[test]
    fn gif_is_a_looping_three_frame_96px_doodle_under_64_kb() {
        let gif = encode(&scribble(), true).unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        assert_eq!([u16::from_le_bytes([gif[6], gif[7]]), u16::from_le_bytes([gif[8], gif[9]])], [OUT as u16, OUT as u16]);
        const { assert!(OUT <= MAX_PX) };
        let frames = gif.windows(3).filter(|w| *w == [0x21, 0xf9, 0x04]).count();
        assert_eq!(frames, 3, "three wiggle frames");
        assert!(gif.windows(11).any(|w| w == b"NETSCAPE2.0"), "loops");
        assert!(gif.len() < MAX_BYTES, "{} bytes", gif.len());
        // The frames really differ (it wiggles), and ink is on.
        assert_ne!(render(&scribble(), Some(0)), render(&scribble(), Some(1)));
        assert!(render(&scribble(), None).chunks(4).any(|px| px[..3] != PAPER));
    }

    #[test]
    fn png_without_the_wiggle() {
        let png = encode(&scribble(), false).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (OUT as u32, OUT as u32));
        assert!(png.len() < MAX_BYTES);
        // A dot is a doodle too.
        assert!(encode(&[Line { color: [0, 0, 0], width: 1.0, pts: vec![(24.0, 24.0)] }], false).is_ok());
    }
}
