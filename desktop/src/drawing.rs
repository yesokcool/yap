//! The drawing board's pixels, WigglyPaint-style (web/js/draw.js): the page is
//! drawn at a fixed 640 × 400 and shown blown up with hard pixel edges. Every
//! pixel is on or off (the web anti-aliases, then snaps alpha ≥ 110 to on;
//! here a pixel is on when its centre is inside the shape, which comes to the
//! same thing). Strokes travel in board units (10000 × 6250).
//!
//! Three layers (PROTOCOL.md › Draw layers): each is its own transparent sheet
//! with its strokes drawn in order (so an eraser only clears its own layer),
//! stacked bottom to top on white paper.
//!
//! Also here, because headless test bots use them too: the PNG ("Save", on
//! white, straight lines) and the animated GIF ("GIF", Clear's archive, Share
//! with Wiggle on): the three wiggle frames, 140 ms each, looping.

use crate::protocol::Stroke;
use std::sync::LazyLock;

pub const BOARD_W: f32 = 10000.0;
pub const BOARD_H: f32 = 6250.0;
pub const PIX_W: usize = 640;
pub const PIX_H: usize = 400;
pub const WIGGLE_FRAMES: u8 = 3;
pub const WIGGLE_MS: u32 = 140;
/// Layers on the page: 0 = bottom, 2 = top.
pub const LAYERS: usize = 3;
const SCALE: f32 = PIX_W as f32 / BOARD_W;
/// A pixel is on when its centre is within the shape plus this much (the
/// web's 110/255 alpha snap keeps pixels about 43 % covered).
const SNAP: f32 = 0.07;
/// What the server takes in a stroke (`cleanStroke` in worker/validate.ts):
/// widths, where its points may be, how many numbers and how much text. A
/// stroke from a server that doesn't check is drawn within them too.
pub const MAX_WIDTH: f32 = 400.0;
const MIN_COORD: f32 = -500.0;
const MAX_COORD: f32 = 10500.0;
pub const MAX_NUMBERS: usize = 8000;
const MAX_TEXT: usize = 200;

/// Straight RGBA, alpha 0 or 255.
#[derive(Clone)]
pub struct Pixels {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

impl Pixels {
    pub fn new() -> Self {
        Pixels { w: PIX_W, h: PIX_H, rgba: vec![0; PIX_W * PIX_H * 4] }
    }

    pub fn clear(&mut self) {
        self.rgba.fill(0);
    }

    fn set(&mut self, x: i32, y: i32, c: Option<[u8; 3]>) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 4;
        match c {
            Some([r, g, b]) => self.rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]),
            None => self.rgba[i..i + 4].copy_from_slice(&[0, 0, 0, 0]),
        }
    }

    /// Paint (Some colour) or erase (None) a round-capped segment `width` px thick.
    fn capsule(&mut self, a: (f32, f32), b: (f32, f32), width: f32, c: Option<[u8; 3]>) {
        // Ink: a pixel ~43 % covered turns on; eraser: ~57 % covered turns off.
        let r = if c.is_some() { width / 2.0 + SNAP } else { (width / 2.0 - SNAP).max(0.45) };
        let (x0, x1) = ((a.0.min(b.0) - r).floor() as i32, (a.0.max(b.0) + r).ceil() as i32);
        let (y0, y1) = ((a.1.min(b.1) - r).floor() as i32, (a.1.max(b.1) + r).ceil() as i32);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        for y in y0.max(0)..=y1.min(self.h as i32 - 1) {
            for x in x0.max(0)..=x1.min(self.w as i32 - 1) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = if len2 > 0.0 { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let (qx, qy) = (a.0 + dx * t - px, a.1 + dy * t - py);
                if qx * qx + qy * qy <= r * r {
                    self.set(x, y, c);
                }
            }
        }
    }

    /// Fill closed contours (non-zero winding), a pixel per centre inside.
    fn fill(&mut self, contours: &[Vec<(f32, f32)>], c: Option<[u8; 3]>) {
        let mut edges: Vec<((f32, f32), (f32, f32))> = Vec::new();
        for pts in contours {
            for i in 0..pts.len() {
                let a = pts[i];
                let b = pts[(i + 1) % pts.len()];
                if a.1 != b.1 {
                    edges.push((a, b));
                }
            }
        }
        if edges.is_empty() {
            return;
        }
        let ymin = edges.iter().map(|e| e.0.1.min(e.1.1)).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
        let ymax = edges.iter().map(|e| e.0.1.max(e.1.1)).fold(f32::MIN, f32::max).ceil().min(self.h as f32 - 1.0) as i32;
        let mut xs: Vec<(f32, i32)> = Vec::new();
        for y in ymin..=ymax {
            let yc = y as f32 + 0.5;
            xs.clear();
            for &(a, b) in &edges {
                let (lo, hi, dir) = if a.1 < b.1 { (a, b, 1) } else { (b, a, -1) };
                if yc >= lo.1 && yc < hi.1 {
                    xs.push((lo.0 + (yc - lo.1) / (hi.1 - lo.1) * (hi.0 - lo.0), dir));
                }
            }
            xs.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut wind = 0;
            for i in 0..xs.len() {
                wind += xs[i].1;
                if wind != 0 && i + 1 < xs.len() {
                    // (Only across the sheet: an edge far off it is no reason to count to it.)
                    let (from, to) = (xs[i].0, xs[i + 1].0.min(self.w as f32));
                    let mut x = (from - 0.5).ceil().max(0.0) as i32;
                    while (x as f32 + 0.5) < to {
                        self.set(x, y, c);
                        x += 1;
                    }
                }
            }
        }
    }

    /// Lay `top` over this sheet: its ink covers, its clear pixels let this show.
    pub fn stack(&mut self, top: &Pixels) {
        for (px, t) in self.rgba.as_chunks_mut::<4>().0.iter_mut().zip(top.rgba.as_chunks::<4>().0) {
            if t[3] != 0 {
                *px = *t;
            }
        }
    }

    /// The page on white paper (what Save and the GIF frames show).
    pub fn on_white(&self) -> Vec<u8> {
        let mut out = self.rgba.clone();
        for px in out.as_chunks_mut::<4>().0 {
            if px[3] == 0 {
                px.copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        out
    }
}

impl Default for Pixels {
    fn default() -> Self {
        Self::new()
    }
}

pub fn hex(c: &str) -> [u8; 3] {
    let h = c.trim_start_matches('#');
    let v = u32::from_str_radix(h.get(..6).unwrap_or("000000"), 16).unwrap_or(0);
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// Text size (board units) for a brush width.
pub fn text_size(w: f32) -> f32 {
    (w * 8.0).max(110.0)
}

// ── Wiggle ("line boil"): web/js/draw.js's position-seeded value noise ──

fn hash(n: i32) -> f32 {
    let mut n = n;
    n = (n ^ 61) ^ ((n as u32 >> 16) as i32);
    n = n.wrapping_add(n.wrapping_shl(3));
    n ^= (n as u32 >> 4) as i32;
    n = n.wrapping_mul(0x27d4eb2d);
    n ^= (n as u32 >> 15) as i32;
    ((n as u32) as f64 / 4294967295.0) as f32
}

const CELL: f32 = 220.0;
const AMP: f32 = 22.0;

/// The same spot always moves the same way in a given frame, whichever stroke
/// is there (so an eraser stays on top of the ink it removed).
pub fn field(x: f32, y: f32, frame: u8, axis: u32) -> f32 {
    let gx = (x / CELL).floor();
    let gy = (y / CELL).floor();
    let tx = x / CELL - gx;
    let ty = y / CELL - gy;
    let at = |i: f32, j: f32| -> f32 {
        let a = (i as i64).wrapping_mul(73856093) as i32;
        let b = (j as i64).wrapping_mul(19349663) as i32;
        let c = (frame as i64).wrapping_mul(83492791) as i32;
        let d = (axis as i64).wrapping_mul(2654435761) as i32;
        hash(a ^ b ^ c ^ d) - 0.5
    };
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let top = at(gx, gy) + (at(gx + 1.0, gy) - at(gx, gy)) * sx;
    let bottom = at(gx, gy + 1.0) + (at(gx + 1.0, gy + 1.0) - at(gx, gy + 1.0)) * sx;
    (top + (bottom - top) * sy) * 2.0 * AMP
}

fn wobble(p: &[f32], frame: u8) -> Vec<f32> {
    let mut out = p.to_vec();
    for i in (0..p.len().saturating_sub(1)).step_by(2) {
        out[i] = p[i] + field(p[i], p[i + 1], frame, 0);
        out[i + 1] = p[i + 1] + field(p[i], p[i + 1], frame, 1);
    }
    out
}

// ── Text: glyph outlines from the hand font (Comic Sans if the system has it) ──

struct HandFont {
    bytes: Vec<u8>,
    index: u32,
}

/// Comic Sans MS where the system has it (Windows, macOS), else Comic Neue.
pub fn hand_font_paths() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["/System/Library/Fonts/Supplemental/Comic Sans MS.ttf", "/Library/Fonts/Comic Sans MS.ttf"]
    } else if cfg!(target_os = "windows") {
        &["C:\\Windows\\Fonts\\comic.ttf"]
    } else {
        &["/usr/share/fonts/truetype/msttcorefonts/Comic_Sans_MS.ttf", "/usr/share/fonts/TTF/ComicNeue-Regular.ttf", "/usr/share/fonts/truetype/comic-neue/ComicNeue-Regular.ttf", "/usr/share/fonts/opentype/comic-neue/ComicNeue-Regular.otf"]
    }
}

static HAND: LazyLock<Option<HandFont>> = LazyLock::new(|| {
    let candidates = hand_font_paths();
    if let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) {
        return Some(HandFont { bytes, index: 0 });
    }
    // egui's own sans (always there).
    let defs = eframe::egui::FontDefinitions::default();
    let data = defs.font_data.get("Ubuntu-Light").or_else(|| defs.font_data.values().next())?;
    Some(HandFont { bytes: data.font.to_vec(), index: data.index })
});

#[derive(Default)]
struct Pen {
    contours: Vec<Vec<(f32, f32)>>,
    cur: Vec<(f32, f32)>,
    last: (f32, f32),
}

impl Pen {
    fn flush(&mut self) {
        if self.cur.len() > 2 {
            self.contours.push(std::mem::take(&mut self.cur));
        }
        self.cur.clear();
    }
}

impl skrifa::outline::OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.cur.push((x, y));
        self.last = (x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.cur.push((x, y));
        self.last = (x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (x0, y0) = self.last;
        for k in 1..=6 {
            let t = k as f32 / 6.0;
            let u = 1.0 - t;
            self.cur.push((u * u * x0 + 2.0 * u * t * cx + t * t * x, u * u * y0 + 2.0 * u * t * cy + t * t * y));
        }
        self.last = (x, y);
    }
    fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        let (x0, y0) = self.last;
        for k in 1..=8 {
            let t = k as f32 / 8.0;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.cur.push((a * x0 + b * c0x + c * c1x + d * x, a * y0 + b * c0y + c * c1y + d * y));
        }
        self.last = (x, y);
    }
    fn close(&mut self) {
        self.flush();
    }
}

/// `text` at `origin` (pixels; the top of the em box, like canvas's
/// textBaseline "top"), `size` px, turned by `angle` around the origin.
fn text_contours(text: &str, origin: (f32, f32), size: f32, angle: f32) -> Vec<Vec<(f32, f32)>> {
    use skrifa::MetadataProvider;
    let Some(hf) = HAND.as_ref() else { return vec![] };
    let Ok(font) = skrifa::FontRef::from_index(&hf.bytes, hf.index) else { return vec![] };
    let sz = skrifa::instance::Size::new(size);
    let loc = skrifa::instance::LocationRef::default();
    let ascent = font.metrics(sz, loc).ascent;
    let charmap = font.charmap();
    let glyphs = font.outline_glyphs();
    let advances = font.glyph_metrics(sz, loc);
    let (sin, cos) = angle.sin_cos();
    let mut out = Vec::new();
    let mut pen_x = 0.0;
    for ch in text.chars() {
        let Some(gid) = charmap.map(ch) else { continue };
        let mut pen = Pen::default();
        if let Some(g) = glyphs.get(gid) {
            let _ = g.draw(skrifa::outline::DrawSettings::unhinted(sz, loc), &mut pen);
        }
        pen.flush();
        for c in pen.contours {
            out.push(
                c.into_iter()
                    .map(|(gx, gy)| {
                        let (lx, ly) = (pen_x + gx, ascent - gy);
                        (origin.0 + lx * cos - ly * sin, origin.1 + lx * sin + ly * cos)
                    })
                    .collect(),
            );
        }
        pen_x += advances.advance_width(gid).unwrap_or(size * 0.5);
    }
    out
}

/// Which layer a stroke is on (anything past the top counts as the top).
pub fn layer(s: &Stroke) -> usize {
    (s.l as usize).min(LAYERS - 1)
}

/// Draw one stroke on its layer's sheet. `frame`: wiggle frame (None =
/// straight lines). An eraser clears pixels (they turn transparent).
pub fn draw_stroke(px: &mut Pixels, s: &Stroke, frame: Option<u8>) {
    let colour = if s.e { None } else { Some(hex(&s.c)) };
    // Within what the server takes (MAX_WIDTH…): the same for every real stroke.
    let w = if s.w.is_finite() { s.w.clamp(1.0, MAX_WIDTH) } else { 1.0 };
    let coord = |v: f32| if v.is_finite() { v.clamp(MIN_COORD, MAX_COORD) } else { 0.0 };
    if s.k.as_deref() == Some("text") {
        if s.p.len() < 2 {
            return;
        }
        let (x, y) = (coord(s.p[0]), coord(s.p[1]));
        let (dx, dy, angle) = match frame {
            Some(f) => (field(x, y, f, 0), field(x, y, f, 1), field(x + 500.0, y, f, 2) / 900.0),
            None => (0.0, 0.0, 0.0),
        };
        let text: String = s.text.as_deref().unwrap_or("").chars().take(MAX_TEXT).collect();
        let contours = text_contours(&text, ((x + dx) * SCALE, (y + dy) * SCALE), text_size(w) * SCALE, angle);
        px.fill(&contours, colour);
        return;
    }
    let p: Vec<f32> = s.p.iter().take(MAX_NUMBERS).map(|v| coord(*v)).collect();
    let p = match frame {
        Some(f) if p.len() > 2 => wobble(&p, f),
        _ => p,
    };
    if p.len() < 2 {
        return;
    }
    let width = (w * SCALE).max(1.0);
    let at = |i: usize| (p[i] * SCALE, p[i + 1] * SCALE);
    if p.len() < 4 {
        px.capsule(at(0), at(0), width, colour);
        return;
    }
    let mut pts: Vec<(f32, f32)> = Vec::with_capacity(p.len());
    if s.k.as_deref() == Some("poly") {
        for i in (0..p.len() - 1).step_by(2) {
            pts.push(at(i));
        }
    } else {
        // Smoothed like the web: quadratic curves through the midpoints.
        pts.push(at(0));
        let mut prev = at(0);
        let mut i = 2;
        while i + 3 < p.len() {
            let ctrl = at(i);
            let end = ((p[i] + p[i + 2]) / 2.0 * SCALE, (p[i + 1] + p[i + 3]) / 2.0 * SCALE);
            let len = (ctrl.0 - prev.0).hypot(ctrl.1 - prev.1) + (end.0 - ctrl.0).hypot(end.1 - ctrl.1);
            let n = ((len / 2.0).ceil() as usize).clamp(1, 16);
            for k in 1..=n {
                let t = k as f32 / n as f32;
                let u = 1.0 - t;
                pts.push((u * u * prev.0 + 2.0 * u * t * ctrl.0 + t * t * end.0, u * u * prev.1 + 2.0 * u * t * ctrl.1 + t * t * end.1));
            }
            prev = end;
            i += 2;
        }
        pts.push(at(p.len() - 2));
    }
    for w in pts.windows(2) {
        px.capsule(w[0], w[1], width, colour);
    }
}

/// One layer's strokes, in order, on a transparent sheet.
pub fn render_layer(strokes: &[Stroke], l: usize, frame: Option<u8>) -> Pixels {
    let mut px = Pixels::new();
    for s in strokes.iter().filter(|s| layer(s) == l) {
        draw_stroke(&mut px, s, frame);
    }
    px
}

/// The page: every layer on its own sheet, stacked bottom to top (still
/// transparent where nothing's drawn).
pub fn render(strokes: &[Stroke], frame: Option<u8>) -> Pixels {
    let mut page = render_layer(strokes, 0, frame);
    for l in 1..LAYERS {
        if strokes.iter().any(|s| layer(s) == l) {
            page.stack(&render_layer(strokes, l, frame));
        }
    }
    page
}

/// "Save": a 640 × 400 PNG on white, straight lines.
pub fn png(strokes: &[Stroke]) -> Result<Vec<u8>, String> {
    let page = render(strokes, None).on_white();
    let img = image::RgbaImage::from_raw(PIX_W as u32, PIX_H as u32, page).ok_or("bad page")?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img).to_rgb8().write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out)
}

/// "GIF": the wiggle frames (one with Wiggle off), 140 ms each, looping.
pub fn gif(strokes: &[Stroke], wiggle: bool) -> Result<Vec<u8>, String> {
    let frames: Vec<Vec<u8>> = if wiggle { (0..WIGGLE_FRAMES).map(|f| render(strokes, Some(f)).on_white()).collect() } else { vec![render(strokes, None).on_white()] };
    encode_gif(&frames, PIX_W, PIX_H, WIGGLE_MS)
}

/// GIF89a from opaque RGBA frames: one global palette (a drawing has a handful
/// of pen colours; past 256, its 40 most used plus a 6×6×6 cube, every pixel
/// snapped to the nearest), NETSCAPE2.0 looping when there's more than one.
pub fn encode_gif(frames: &[Vec<u8>], w: usize, h: usize, delay_ms: u32) -> Result<Vec<u8>, String> {
    let mut counts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for f in frames {
        for px in f.as_chunks::<4>().0 {
            *counts.entry(u32::from_be_bytes([0, px[0], px[1], px[2]])).or_insert(0) += 1;
        }
    }
    let mut palette: Vec<u32> = if counts.len() <= 256 {
        let mut v: Vec<u32> = counts.keys().copied().collect();
        v.sort();
        v
    } else {
        let mut by_use: Vec<(u32, u32)> = counts.iter().map(|(c, n)| (*c, *n)).collect();
        by_use.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut v: Vec<u32> = by_use.iter().take(40).map(|(c, _)| *c).collect();
        for r in 0..6u32 {
            for g in 0..6u32 {
                for b in 0..6u32 {
                    let c = (r * 51) << 16 | (g * 51) << 8 | (b * 51);
                    if !v.contains(&c) {
                        v.push(c);
                    }
                }
            }
        }
        v.truncate(256);
        v
    };
    let mut size = 2;
    while size < palette.len() {
        size *= 2;
    }
    let index: std::collections::HashMap<u32, u8> = palette.iter().enumerate().map(|(i, c)| (*c, i as u8)).collect();
    let nearest = |c: u32| -> u8 {
        if let Some(i) = index.get(&c) {
            return *i;
        }
        let (r, g, b) = ((c >> 16) as i32 & 255, (c >> 8) as i32 & 255, c as i32 & 255);
        palette.iter().enumerate().min_by_key(|(_, p)| {
            let (pr, pg, pb) = ((**p >> 16) as i32 & 255, (**p >> 8) as i32 & 255, **p as i32 & 255);
            (pr - r).pow(2) + (pg - g).pow(2) + (pb - b).pow(2)
        }).map(|(i, _)| i as u8).unwrap_or(0)
    };
    let mut cache: std::collections::HashMap<u32, u8> = std::collections::HashMap::new();
    let indexed: Vec<Vec<u8>> = frames
        .iter()
        .map(|f| {
            f.as_chunks::<4>().0.iter()
                .map(|px| {
                    let c = u32::from_be_bytes([0, px[0], px[1], px[2]]);
                    *cache.entry(c).or_insert_with(|| nearest(c))
                })
                .collect()
        })
        .collect();
    palette.resize(size, 0);
    let rgb: Vec<u8> = palette.iter().flat_map(|c| [(c >> 16) as u8, (c >> 8) as u8, *c as u8]).collect();
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, w as u16, h as u16, &rgb).map_err(|e| e.to_string())?;
        if frames.len() > 1 {
            enc.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
        }
        for px in indexed {
            let mut frame = gif::Frame { width: w as u16, height: h as u16, buffer: std::borrow::Cow::Owned(px), ..Default::default() };
            frame.delay = (delay_ms / 10).max(2) as u16;
            enc.write_frame(&frame).map_err(|e| e.to_string())?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stroke from a server that doesn't check (huge width, a point a
    /// billion units off, a mile of text) draws like one it would take, quickly.
    #[test]
    fn strokes_out_of_bounds_stay_cheap() {
        let start = std::time::Instant::now();
        let mut px = Pixels::new();
        let text = Stroke { k: Some("text".into()), text: Some("I".repeat(10_000)), p: vec![1e11, 3000.0], w: 1e7, c: "#000000".into(), ..Default::default() };
        draw_stroke(&mut px, &text, None);
        draw_stroke(&mut px, &Stroke { p: vec![-1e11, 3000.0], ..text.clone() }, Some(1));
        let wide = Stroke { p: (0..20_000).map(|i| (i % 100) as f32 * 100.0).collect(), w: 1e9, c: "#000000".into(), ..Default::default() };
        draw_stroke(&mut px, &wide, None);
        assert!(start.elapsed() < std::time::Duration::from_secs(20), "{:?}", start.elapsed());
        // Within the limits nothing changes.
        let ok = Stroke { p: vec![100.0, 100.0, 5000.0, 3000.0], w: 14.0, c: "#ff0000".into(), ..Default::default() };
        let (mut a, mut b) = (Pixels::new(), Pixels::new());
        draw_stroke(&mut a, &ok, None);
        draw_stroke(&mut b, &Stroke { w: 14.0, ..ok.clone() }, None);
        assert!(a.rgba == b.rgba && a.rgba.iter().any(|v| *v != 0));
    }

    fn line(c: &str, w: f32, p: Vec<f32>) -> Stroke {
        Stroke { id: crate::config::random_hex(6), c: c.into(), w, p, k: Some("poly".into()), ..Default::default() }
    }

    /// The wiggle noise matches web/js/draw.js (values computed with its code).
    #[test]
    fn wiggle_matches_the_web() {
        let want = [(1234.0, 5678.0, 1, 0, 12.30081), (9999.0, 6000.0, 2, 1, -7.39783), (0.0, 0.0, 0, 0, 11.11366), (4321.5, 123.25, 1, 2, -1.37979), (7000.0, 3000.0, 2, 0, 3.89254)];
        for (x, y, f, axis, v) in want {
            let got = field(x, y, f, axis);
            assert!((got - v).abs() < 0.001, "field({x}, {y}, {f}, {axis}) = {got}, web says {v}");
        }
    }

    #[test]
    fn pixels_are_on_or_off_and_erasers_erase() {
        let strokes = vec![line("#ff0000", 70.0, vec![1000.0, 1000.0, 9000.0, 1000.0]), Stroke { id: "e".into(), c: "#000000".into(), w: 160.0, e: true, p: vec![5000.0, 1000.0], ..Default::default() }];
        let px = render(&strokes, None);
        assert!(px.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0 || p[3] == 255), "no partial alpha");
        let at = |x: usize, y: usize| &px.rgba[(y * PIX_W + x) * 4..(y * PIX_W + x) * 4 + 4];
        assert_eq!(at(100, 64), &[255, 0, 0, 255], "on the red line");
        assert_eq!(at(320, 64)[3], 0, "the eraser dot cleared the middle");
        assert_eq!(at(100, 200)[3], 0, "paper elsewhere");
        // A 4.5 px line is 4–6 px tall.
        let tall = (50..80).filter(|y| at(200, *y)[3] == 255).count();
        assert!((4..=6).contains(&tall), "line height {tall}");
    }

    /// An eraser only clears its own layer; upper layers cover lower ones
    /// whatever order the strokes came in.
    #[test]
    fn layers_stack_and_erasers_stay_on_their_layer() {
        let red = line("#ff0000", 70.0, vec![1000.0, 1000.0, 9000.0, 1000.0]);
        let mut blue = line("#0000ff", 70.0, vec![5000.0, 200.0, 5000.0, 3000.0]);
        blue.l = 2;
        let mut rub = Stroke { id: "e".into(), c: "#000000".into(), w: 160.0, e: true, p: vec![2500.0, 1000.0], l: 1, ..Default::default() };
        // The top layer drawn first, the bottom one after: the top still wins.
        let strokes = vec![blue.clone(), red.clone(), rub.clone()];
        let px = render(&strokes, None);
        let at = |px: &Pixels, x: usize, y: usize| px.rgba[(y * PIX_W + x) * 4..(y * PIX_W + x) * 4 + 4].to_vec();
        assert_eq!(at(&px, 160, 64), vec![255, 0, 0, 255], "the eraser on layer 1 left layer 0's ink alone");
        assert_eq!(at(&px, 320, 64), vec![0, 0, 255, 255], "layer 2 covers layer 0");
        assert_eq!(at(&px, 320, 150), vec![0, 0, 255, 255], "layer 2 on its own");
        assert_eq!(at(&px, 100, 200)[3], 0, "paper elsewhere");
        // The same eraser on the bottom layer does erase it.
        rub.l = 0;
        let px = render(&[blue.clone(), red.clone(), rub], None);
        assert_eq!(at(&px, 160, 64)[3], 0, "an eraser on layer 0 clears layer 0");
        // An eraser on the top layer can't make a hole through the layers under it.
        let top_rub = Stroke { id: "e2".into(), c: "#000000".into(), w: 160.0, e: true, p: vec![5000.0, 1000.0], l: 2, ..Default::default() };
        let px = render(&[red, blue, top_rub], None);
        assert_eq!(at(&px, 320, 64), vec![255, 0, 0, 255], "erasing layer 2 shows layer 0 again");
        assert_eq!(at(&px, 320, 150), vec![0, 0, 255, 255], "the rest of layer 2 stays");
    }

    #[test]
    fn text_draws_pixels() {
        let s = Stroke { id: "t".into(), c: "#008000".into(), w: 32.0, p: vec![1200.0, 4500.0], k: Some("text".into()), text: Some("hi there".into()), ..Default::default() };
        let px = render(&[s], None);
        let on = px.rgba.as_chunks::<4>().0.iter().filter(|p| p[3] == 255).count();
        assert!(on > 100, "text lit {on} pixels");
    }

    /// Encode → decode (with the gif crate's decoder): 640 × 400, 3 frames, 14 cs, looping.
    #[test]
    fn gif_round_trip() {
        let strokes = vec![line("#0080ff", 32.0, vec![1000.0, 1000.0, 4000.0, 2500.0, 6000.0, 1500.0]), line("#ff0000", 14.0, vec![5000.0, 1000.0, 8000.0, 1000.0, 8000.0, 3000.0, 5000.0, 3000.0, 5000.0, 1000.0])];
        let bytes = gif(&strokes, true).unwrap();
        assert_eq!(&bytes[..6], b"GIF89a");
        let mut opts = gif::DecodeOptions::new();
        opts.set_color_output(gif::ColorOutput::RGBA);
        let mut dec = opts.read_info(std::io::Cursor::new(&bytes)).unwrap();
        assert_eq!((dec.width(), dec.height()), (640, 400));
        assert_eq!(dec.repeat(), gif::Repeat::Infinite);
        let mut frames = Vec::new();
        while let Some(f) = dec.read_next_frame().unwrap() {
            assert_eq!(f.delay, 14);
            frames.push(f.buffer.to_vec());
        }
        assert_eq!(frames.len(), 3);
        // The frames wiggle (differ) and match what we drew, pixel for pixel.
        assert_ne!(frames[0], frames[1]);
        assert_eq!(frames[0], render(&strokes, Some(0)).on_white());
        // Wiggle off: one frame, no loop block needed.
        let one = gif(&strokes, false).unwrap();
        let mut dec = gif::DecodeOptions::new().read_info(std::io::Cursor::new(&one)).unwrap();
        let mut n = 0;
        while dec.read_next_frame().unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, 1);
        // And the PNG is 640 × 400.
        let png = png(&strokes).unwrap();
        let img = image::load_from_memory(&png).unwrap();
        assert_eq!((img.width(), img.height()), (640, 400));
    }
}
