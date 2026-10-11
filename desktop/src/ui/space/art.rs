//! Drawing the world (web/js/space-art.js): the floor, the track, the
//! pitch, decorations, the pond, the piano, effects, and the players.

use super::*;

/// Draws in world units (one unit = one point), offset by the camera.
pub(super) struct Pen {
    pub(super) p: egui::Painter,
    /// Where the world's (0, 0) is on screen.
    pub(super) origin: Pos2,
}

impl Pen {
    pub(super) fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.origin.x + x, self.origin.y + y)
    }

    pub(super) fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.at(x, y), vec2(w, h))
    }

    pub(super) fn seen(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.rect(x, y, w, h).intersects(self.p.clip_rect())
    }

    pub(super) fn fill(&self, x: f32, y: f32, w: f32, h: f32, c: Color32) {
        self.p.rect_filled(self.rect(x, y, w, h), 0.0, c);
    }

    /// A filled box with a 3-unit edge (the retro furniture look).
    pub(super) fn boxed(&self, x: f32, y: f32, w: f32, h: f32, fill: Color32, edge: Color32) {
        let r = self.rect(x, y, w, h);
        self.p.rect_filled(r, 0.0, fill);
        self.p.rect_stroke(r, 0.0, Stroke::new(3.0, edge), egui::StrokeKind::Middle);
    }

    pub(super) fn outline(&self, x: f32, y: f32, w: f32, h: f32, stroke: Stroke) {
        self.p.rect_stroke(self.rect(x, y, w, h), 0.0, stroke, egui::StrokeKind::Middle);
    }

    pub(super) fn disc(&self, x: f32, y: f32, r: f32, c: Color32) {
        self.p.circle_filled(self.at(x, y), r, c);
    }

    pub(super) fn ring(&self, x: f32, y: f32, r: f32, stroke: Stroke) {
        self.p.circle_stroke(self.at(x, y), r, stroke);
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn ellipse(&self, cx: f32, cy: f32, rx: f32, ry: f32, rot: f32, fill: Color32) {
        let pts: Vec<Pos2> = (0..36)
            .map(|i| {
                let a = i as f32 / 36.0 * TAU;
                let (px, py) = (rx * a.cos(), ry * a.sin());
                self.at(cx + px * rot.cos() - py * rot.sin(), cy + px * rot.sin() + py * rot.cos())
            })
            .collect();
        self.p.add(Shape::convex_polygon(pts, fill, Stroke::NONE));
    }

    pub(super) fn poly(&self, pts: &[(f32, f32)], fill: Color32) {
        self.p.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| self.at(x, y)).collect(), fill, Stroke::NONE));
    }

    pub(super) fn line(&self, pts: &[(f32, f32)], stroke: Stroke) {
        self.p.add(Shape::line(pts.iter().map(|&(x, y)| self.at(x, y)).collect(), stroke));
    }

    /// Text centred (per `align`) on x, y.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn text(&self, x: f32, y: f32, align: Align2, s: impl ToString, size: f32, bold: bool, c: Color32) -> Rect {
        let family = if bold { theme::bold() } else { egui::FontFamily::Proportional };
        self.p.text(self.at(x, y), align, s.to_string(), egui::FontId::new(size, family), c)
    }

    pub(super) fn dashed(&self, a: (f32, f32), b: (f32, f32), dash: f32, gap: f32, stroke: Stroke) {
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        if len <= 0.0 {
            return;
        }
        let mut t = 0.0;
        while t < len {
            let t1 = (t + dash).min(len);
            let p = |t: f32| (a.0 + (b.0 - a.0) * t / len, a.1 + (b.1 - a.1) * t / len);
            self.line(&[p(t), p(t1)], stroke);
            t += dash + gap;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn dashed_rect(&self, x: f32, y: f32, w: f32, h: f32, dash: f32, gap: f32, stroke: Stroke) {
        let c = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
        for i in 0..4 {
            self.dashed(c[i], c[(i + 1) % 4], dash, gap, stroke);
        }
    }
}

/// A quadratic Bézier from `a` via `c` to `b`, as `n` points (after `a`).
pub(super) fn quad(a: (f32, f32), c: (f32, f32), b: (f32, f32), n: usize) -> Vec<(f32, f32)> {
    (1..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            (u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0, u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1)
        })
        .collect()
}

pub(super) fn dashed_circle(painter: &egui::Painter, c: Pos2, r: f32, dash: f32, gap: f32, stroke: Stroke) {
    let n = ((TAU * r) / (dash + gap)).floor().max(1.0) as usize;
    for i in 0..n {
        let a0 = i as f32 / n as f32 * TAU;
        let a1 = a0 + dash / r;
        let pts: Vec<Pos2> = (0..=6).map(|k| c + vec2((a0 + (a1 - a0) * k as f32 / 6.0).cos(), (a0 + (a1 - a0) * k as f32 / 6.0).sin()) * r).collect();
        painter.add(Shape::line(pts, stroke));
    }
}

/// The wall you're looking at in the Gallery.
pub(super) struct Wall<'a> {
    pub(super) items: Vec<&'a SavedDrawing>,
    /// The frame lit up (dwelled on, or open in the viewer): 0–11, or -1.
    pub(super) focus: i32,
    pub(super) page: usize,
    pub(super) pages: usize,
    pub(super) total: usize,
}

/// What the furniture needs to know.
pub(super) struct Env<'a> {
    pub(super) m: &'a Model,
    pub(super) images: &'a super::super::images::Images,
    pub(super) now: f64,
    pub(super) clock: f64,
    pub(super) tag_on: bool,
    pub(super) score: [u32; 2],
    /// Piano keys and pads someone stands on ("w3", "b1", "p2").
    pub(super) keys_down: HashSet<String>,
    /// Best laps here, fastest first.
    pub(super) laps: Vec<(String, f64)>,
    pub(super) wall: Wall<'a>,
}

pub(super) fn world(pen: &Pen, env: &Env) {
    floor(pen);
    // Floor lettering, under the furniture's tops but over the floors.
    for r in &ROOMS {
        let (lx, ly) = r.label.unwrap_or((r.r.x + r.r.w / 2.0, r.r.y + r.r.h / 2.0));
        pen.text(lx, ly, Align2::CENTER_CENTER, r.name.to_uppercase(), if r.r.w < 450.0 { 38.0 } else { 44.0 }, true, Color32::from_white_alpha(46));
    }
    for &(kind, x, y, extra) in DECOR.iter() {
        decor(pen, kind, x, y, extra, env);
    }
    // Walls: chunky, with a highlight edge (retro bevel), and the name signs.
    for w in WALLS.iter() {
        pen.fill(w.x, w.y, w.w, w.h, rgb(EDGE));
        pen.fill(w.x, w.y, w.w, 3.0, Color32::from_white_alpha(36));
    }
    for r in &ROOMS {
        for (x, y) in plaque_spots(r) {
            plaque(pen, r.name, x, y);
        }
    }
}

pub(super) fn floor(pen: &Pen) {
    // Plaza: checkerboard tiles (only what's on screen).
    let mut y = 0.0;
    while y < WORLD_H {
        let mut x = 0.0;
        while x < WORLD_W {
            if pen.seen(x, y, 80.0, 80.0) {
                let c = if ((x / 80.0) as i32 + (y / 80.0) as i32) % 2 == 1 { rgb(0x2b2850) } else { rgb(0x312d5c) };
                pen.fill(x, y, 80.0, 80.0, c);
            }
            x += 80.0;
        }
        y += 80.0;
    }
    track(pen);
    pen.text(PLAZA_SPOT.0, PLAZA_SPOT.1, Align2::CENTER_CENTER, PLAZA.to_uppercase(), 44.0, true, Color32::from_white_alpha(20));
    for r in &ROOMS {
        let R { x, y, w, h } = r.r;
        pen.fill(x, y, w, h, rgb(r.floor));
        if r.name == "Garden" || r.name == "Pond" {
            for i in 0..50 {
                pen.fill(x + ((i * 137) as f32 % w), y + ((i * 89) as f32 % h), 6.0, 10.0, Color32::from_white_alpha(13));
            }
        }
        if r.open {
            // No walls, so a low dashed edge shows where the room (and its voice) ends.
            pen.dashed_rect(x + 2.0, y + 2.0, w - 4.0, h - 4.0, 12.0, 10.0, Stroke::new(3.0, Color32::from_white_alpha(71)));
        }
        match r.name {
            "Pitch" => pitch(pen),
            // A stage rug under the instruments.
            "Music" => pen.fill(x + 40.0, y + 34.0, w - 80.0, h - 70.0, Color32::from_black_alpha(46)),
            "Gallery" => {
                // Parquet.
                let s = Stroke::new(2.0, Color32::from_black_alpha(41));
                let mut py = y + 30.0;
                while py < y + h {
                    pen.line(&[(x, py), (x + w, py)], s);
                    py += 30.0;
                }
                let mut px = x;
                let mut i = 0;
                while px < x + w {
                    let mut py = y + (i % 2) as f32 * 30.0;
                    while py < y + h {
                        pen.line(&[(px, py), (px, (py + 30.0).min(y + h))], s);
                        py += 60.0;
                    }
                    px += 90.0;
                    i += 1;
                }
            }
            _ => {}
        }
    }
}

/// The race track: a lane all the way round, run clockwise, start/finish on the left.
pub(super) fn track(pen: &Pen) {
    let lane = rgb(0x7d3b2a);
    pen.fill(0.0, 0.0, WORLD_W, LANE, lane);
    pen.fill(0.0, WORLD_H - LANE, WORLD_W, LANE, lane);
    pen.fill(0.0, LANE, LANE, WORLD_H - LANE * 2.0, lane);
    pen.fill(WORLD_W - LANE, LANE, LANE, WORLD_H - LANE * 2.0, lane);
    // Lane lines.
    pen.dashed_rect(56.0, 56.0, WORLD_W - 112.0, WORLD_H - 112.0, 16.0, 14.0, Stroke::new(2.0, Color32::from_white_alpha(89)));
    // Chevrons pointing the way round.
    let chev = Stroke::new(4.0, Color32::from_white_alpha(71));
    let chevron = |x: f32, y: f32, a: f32| pen.line(&[(x + (a + 2.5).cos() * 12.0, y + (a + 2.5).sin() * 12.0), (x, y), (x + (a - 2.5).cos() * 12.0, y + (a - 2.5).sin() * 12.0)], chev);
    let mut x = 400.0;
    while x < WORLD_W - 200.0 {
        chevron(x, 56.0, 0.0);
        chevron(x, WORLD_H - 56.0, PI);
        x += 400.0;
    }
    let mut y = 400.0;
    while y < WORLD_H - 200.0 {
        chevron(WORLD_W - 56.0, y, FRAC_PI_2);
        if (y - START_Y).abs() > 60.0 {
            chevron(56.0, y, -FRAC_PI_2);
        }
        y += 400.0;
    }
    // Corner flags (checkpoints, in order).
    for (i, c) in CORNERS.iter().enumerate() {
        let (fx, fy) = (c.x + c.w / 2.0, c.y + c.h / 2.0);
        pen.poly(&[(fx - 4.0, fy - 18.0), (fx + 14.0, fy - 12.0), (fx - 4.0, fy - 6.0)], rgb(0xffd166));
        pen.fill(fx - 6.0, fy - 20.0, 3.0, 30.0, rgb(0xe6e6e6));
        pen.text(fx + 4.0, fy + 18.0, Align2::CENTER_CENTER, i + 1, 11.0, true, rgb(0xe6e6e6));
    }
    // Start/finish: a chequered line across the left lane.
    for i in 0..11 {
        for row in 0..2 {
            pen.fill(12.0 + i as f32 * 8.0, START_Y - 8.0 + row as f32 * 8.0, 8.0, 8.0, if (i + row) % 2 == 1 { rgb(0x111111) } else { rgb(0xf5f5f5) });
        }
    }
    pen.text(56.0, START_Y - 20.0, Align2::CENTER_CENTER, "FINISH", 11.0, true, Color32::from_white_alpha(191));
    pen.text(56.0, START_Y + 22.0, Align2::CENTER_CENTER, "▲ RUN", 11.0, true, Color32::from_white_alpha(191));
}

/// A room's name sign on its wall (on a post for the open Garden).
pub(super) fn plaque(pen: &Pen, name: &str, x: f32, y: f32) {
    let g = pen.p.layout_no_wrap(name.to_string(), egui::FontId::new(13.0, theme::bold()), rgb(0x2a1d0a));
    let w = g.size().x + 16.0;
    let rect = Rect::from_center_size(pen.at(x, y), vec2(w, 20.0));
    pen.p.rect_filled(rect, 0.0, rgb(0xe8d9a8));
    pen.p.rect_stroke(rect, 0.0, Stroke::new(2.0, rgb(EDGE)), egui::StrokeKind::Middle);
    pen.p.galley(rect.center() - g.size() / 2.0 + vec2(0.0, 1.0), g, rgb(0x2a1d0a));
}

pub(super) fn pitch(pen: &Pen) {
    let r = PITCH;
    let mut i = 0;
    while (i as f32) * 60.0 < r.w {
        let c = if i % 2 == 1 { rgb(0x3d8a46) } else { rgb(0x45964f) };
        pen.fill(r.x + i as f32 * 60.0, r.y, 60.0f32.min(r.w - i as f32 * 60.0), r.h, c);
        i += 1;
    }
    let line = Stroke::new(3.0, Color32::from_white_alpha(178));
    let left = NET_X[0] + NET_DEPTH;
    let right = NET_X[1] - NET_DEPTH;
    let top = r.y + T + 18.0;
    let bottom = r.y + r.h - T - 18.0;
    pen.outline(left, top, right - left, bottom - top, line);
    pen.line(&[(MID.0, top), (MID.0, bottom)], line);
    pen.ring(MID.0, MID.1, 70.0, line);
    pen.outline(left, MID.1 - 130.0, 100.0, 260.0, line);
    pen.outline(right - 100.0, MID.1 - 130.0, 100.0, 260.0, line);
    pen.disc(MID.0, MID.1, 5.0, Color32::from_white_alpha(204));
}

pub(super) fn decor(pen: &Pen, kind: &str, x: f32, y: f32, extra: Option<f32>, env: &Env) {
    let edge = rgb(EDGE);
    let bx = |bx: f32, by: f32, bw: f32, bh: f32, fill: u32| pen.boxed(bx, by, bw, bh, rgb(fill), edge);
    match kind {
        "sofa" => {
            bx(x - 80.0, y - 34.0, 160.0, 68.0, 0xe45b8f);
            bx(x - 80.0, y - 34.0, 160.0, 16.0, 0xc94677);
        }
        "rug" => pen.ellipse(x, y, 190.0, 90.0, 0.0, rgba(255, 214, 102, 0.35)),
        "ctable" => {
            bx(x - 55.0, y - 26.0, 110.0, 52.0, 0xc79a5b);
            pen.disc(x - 22.0, y, 8.0, Color32::WHITE);
            pen.disc(x + 20.0, y - 6.0, 7.0, rgb(0x7afcff));
        }
        "shelf" => {
            bx(x - 150.0, y - 16.0, 300.0, 32.0, 0x5a3a22);
            let books = [0xe45b8f, 0xffd166, 0x7afcff, 0x8aff80, 0xff9f45, 0xb48cff];
            for i in 0..18 {
                pen.fill(x - 142.0 + i as f32 * 16.0, y - 11.0, 11.0, 22.0, rgb(books[i % 6]));
            }
        }
        "plant" => {
            bx(x - 18.0, y - 4.0, 36.0, 30.0, 0xb4693e);
            pen.disc(x, y - 16.0, 26.0, rgb(0x5cc46a));
        }
        "lamp" => {
            pen.disc(x, y, 80.0, rgba(255, 220, 140, 0.07));
            pen.disc(x, y, 46.0, rgba(255, 220, 140, 0.10));
            pen.disc(x, y, 14.0, rgb(0x3a3350));
            pen.disc(x, y, 8.0, rgb(0xffe9a8));
        }
        "screen" => screen(pen, x, y, env.now),
        "couch" => {
            // Facing the screen (up): the backrest is at the bottom.
            bx(x - 80.0, y - 26.0, 160.0, 52.0, 0xb8333f);
            bx(x - 80.0, y + 16.0, 160.0, 16.0, 0x8e2430);
            bx(x - 88.0, y - 26.0, 12.0, 58.0, 0x8e2430);
            bx(x + 76.0, y - 26.0, 12.0, 58.0, 0x8e2430);
        }
        "popcorn" => {
            bx(x - 34.0, y - 26.0, 68.0, 52.0, 0xc0392b);
            for i in 0..4 {
                pen.fill(x - 30.0 + i as f32 * 17.0, y - 22.0, 8.0, 44.0, Color32::WHITE);
            }
            for i in 0..6 {
                pen.disc(x - 25.0 + i as f32 * 10.0, y - 28.0 + (i % 2) as f32 * 4.0, 8.0, rgb(0xfff3b0));
            }
        }
        "piano" => piano(pen, &env.keys_down),
        "pads" => {
            for (i, (px, py, drum, color)) in PADS.iter().enumerate() {
                let down = env.keys_down.contains(&format!("p{i}"));
                pen.disc(*px, *py, PAD_R + 4.0, edge);
                pen.disc(*px, *py, PAD_R, if down { Color32::WHITE } else { rgb(*color) });
                pen.disc(*px, *py, PAD_R * 0.55, if down { rgb(*color) } else { Color32::from_white_alpha(64) });
                pen.text(*px, py + PAD_R + 12.0, Align2::CENTER_CENTER, drum.to_uppercase(), 10.0, true, Color32::from_white_alpha(204));
            }
        }
        "mic" => {
            pen.disc(x, y, 14.0, rgb(0x3a3350));
            pen.line(&[(x, y), (x + 14.0, y - 22.0)], Stroke::new(3.0, rgb(0xc0c0c0)));
            pen.disc(x + 16.0, y - 25.0, 7.0, rgb(0xd0d0d0));
        }
        "speaker" => {
            bx(x - 32.0, y - 40.0, 64.0, 80.0, 0x1b1b22);
            pen.disc(x, y - 14.0, 15.0, rgb(0x3a3a48));
            pen.disc(x, y - 14.0, 6.0, rgb(0x111111));
            pen.disc(x, y + 22.0, 9.0, rgb(0x3a3a48));
        }
        "cabinet" => {
            bx(x - 36.0, y - 50.0, 72.0, 100.0, 0x1f2a44);
            let screens = [0xff6ad5, 0x7afcff, 0xfff07a, 0x8aff80];
            pen.fill(x - 24.0, y - 38.0, 48.0, 34.0, rgb(screens[((x / 90.0).floor() as usize) % 4]));
            pen.disc(x - 10.0, y + 22.0, 6.0, rgb(0xe5484d));
            pen.disc(x + 12.0, y + 22.0, 6.0, rgb(0xffd166));
        }
        "stool" => {
            pen.disc(x, y, 17.0, edge);
            pen.disc(x, y, 14.0, rgb(0xc0392b));
            pen.disc(x, y, 7.0, rgb(0xe74c3c));
        }
        "pool" => {
            bx(x - 125.0, y - 65.0, 250.0, 130.0, 0x6b4423);
            pen.fill(x - 110.0, y - 50.0, 220.0, 100.0, rgb(0x1f7a4a));
            for (px, py) in [(-110.0, -50.0), (0.0, -52.0), (110.0, -50.0), (-110.0, 50.0), (0.0, 52.0), (110.0, 50.0)] {
                pen.disc(x + px, y + py, 9.0, edge);
            }
            pen.disc(x - 60.0, y, 8.0, Color32::WHITE);
            let balls = [0xffd166, 0xe5484d, 0x5b6cff, 0x30a46c, EDGE, 0xf76b15];
            for (i, (px, py)) in [(30.0, 0.0), (44.0, -8.0), (44.0, 8.0), (58.0, -16.0), (58.0, 0.0), (58.0, 16.0)].into_iter().enumerate() {
                pen.disc(x + px, y + py, 7.0, rgb(balls[i]));
            }
        }
        "claw" => {
            bx(x - 42.0, y - 46.0, 84.0, 92.0, 0xd6409f);
            pen.fill(x - 32.0, y - 36.0, 64.0, 50.0, rgba(168, 230, 255, 0.55));
            let toys = [0xffd166, 0x8aff80, 0xff9f45, 0xb48cff];
            for (i, (px, py)) in [(-16.0, 4.0), (0.0, 8.0), (14.0, 2.0), (-4.0, -6.0)].into_iter().enumerate() {
                pen.disc(x + px, y + py, 7.0, rgb(toys[i]));
            }
            pen.line(&[(x + 6.0, y - 36.0), (x + 6.0, y - 18.0)], Stroke::new(2.0, edge));
        }
        "gallery" => gallery_walls(pen, env),
        "tree" => {
            pen.ellipse(x + 10.0, y + 30.0, 60.0, 22.0, 0.0, Color32::from_black_alpha(64));
            pen.disc(x, y, 58.0, rgb(0x2f7a3a));
            pen.disc(x - 14.0, y - 14.0, 34.0, rgb(0x3f9a4a));
        }
        "fountain" => {
            pen.disc(x, y, 98.0, edge);
            pen.disc(x, y, 95.0, rgb(0x8d99a6));
            pen.disc(x, y, 82.0, rgb(0x3f8fd0));
            ripples(pen, x, y, 24.0, 56.0, env.clock);
            pen.disc(x, y, 18.0, rgb(0x8d99a6));
            pen.disc(x, y, 9.0, rgb(0xdff3ff));
        }
        "pond" => pond(pen, env.clock),
        "dock" => {
            for r in DOCK {
                pen.boxed(r.x, r.y, r.w, r.h, rgb(0xa0703f), rgb(0x4a2f17));
            }
            let plank = Stroke::new(2.0, rgba(74, 47, 23, 0.7));
            let mut py = DOCK[0].y + 18.0;
            while py < DOCK[1].y {
                pen.line(&[(DOCK[0].x, py), (DOCK[0].x + DOCK[0].w, py)], plank);
                py += 18.0;
            }
            let mut px = DOCK[1].x + 18.0;
            while px < DOCK[1].x + DOCK[1].w {
                pen.line(&[(px, DOCK[1].y), (px, DOCK[1].y + DOCK[1].h)], plank);
                px += 18.0;
            }
            for px in [DOCK[1].x, DOCK[1].x + DOCK[1].w] {
                pen.disc(px, DOCK[1].y + DOCK[1].h, 6.0, rgb(0x4a2f17));
            }
        }
        "reeds" => {
            for i in 0..6 {
                let rx = x + (i as f32 - 2.5) * 7.0;
                let lean = if i % 2 == 1 { 4.0 } else { -4.0 };
                pen.line(&[(rx, y + 18.0), (rx + lean, y - 16.0 - (i % 3) as f32 * 6.0)], Stroke::new(3.0, rgb(0x3f7a2c)));
                pen.disc(rx + lean, y - 18.0 - (i % 3) as f32 * 6.0, 4.0, rgb(0x7a4a22));
            }
        }
        "bench" => {
            bx(x - 58.0, y - 14.0, 116.0, 28.0, 0xa0522d);
            // The backrest is on the side away from where you face.
            let back_y = if extra.unwrap_or(0.0) > 0.0 { y - 22.0 } else { y + 14.0 };
            bx(x - 58.0, back_y, 116.0, 8.0, 0x7a3d1f);
        }
        "flowers" => {
            let cols = [0xff7eb6, 0xffd166, 0xffffff];
            for i in 0..7 {
                pen.disc(x + (i as f32).cos() * 40.0, y + (i as f32 * 2.0).sin() * 18.0, 7.0, rgb(cols[i % 3]));
            }
        }
        "net" => {
            let side = extra.unwrap_or(0.0) as usize;
            let nx = if side == 1 { x - NET_DEPTH } else { x };
            pen.fill(nx, y - NET_HALF, NET_DEPTH, NET_HALF * 2.0, Color32::from_white_alpha(31));
            let mesh = Stroke::new(1.0, Color32::from_white_alpha(89));
            let mut gx = nx + 8.0;
            while gx < nx + NET_DEPTH {
                pen.line(&[(gx, y - NET_HALF), (gx, y + NET_HALF)], mesh);
                gx += 9.0;
            }
            let mut gy = y - NET_HALF + 9.0;
            while gy < y + NET_HALF {
                pen.line(&[(nx, gy), (nx + NET_DEPTH, gy)], mesh);
                gy += 9.0;
            }
            // Blue's goal on the left, Red's on the right.
            let stripe_x = if side == 1 { nx } else { nx + NET_DEPTH - 5.0 };
            pen.fill(stripe_x, y - NET_HALF, 5.0, NET_HALF * 2.0, if side == 1 { rgb(0xe5484d) } else { rgb(0x5b6cff) });
            for post in POSTS.iter().skip(side * 2).take(2) {
                bx(post.x, post.y, post.w, post.h, 0xf5f5f5);
            }
        }
        "scoreboard" => {
            pen.boxed(x - 100.0, y - 24.0, 200.0, 48.0, edge, rgb(0x3a3350));
            let [blue, red] = env.score;
            pen.text(x - 62.0, y - 10.0, Align2::CENTER_CENTER, "BLUE", 10.0, true, rgb(0x9aa3ff));
            pen.text(x + 62.0, y - 10.0, Align2::CENTER_CENTER, "RED", 10.0, true, rgb(0xff9a9e));
            pen.text(x, y + 1.0, Align2::CENTER_CENTER, format!("{blue} : {red}"), 24.0, true, Color32::WHITE);
        }
        "tent" => {
            let pts = vec![pen.at(x - 50.0, y + 36.0), pen.at(x + 50.0, y + 36.0), pen.at(x + 38.0, y - 36.0), pen.at(x - 38.0, y - 36.0)];
            pen.p.add(Shape::convex_polygon(pts, rgb(0xe07a2e), Stroke::new(3.0, edge)));
            pen.line(&[(x, y - 36.0), (x, y + 36.0)], Stroke::new(3.0, rgb(0xa5521a)));
            pen.poly(&[(x - 16.0, y + 36.0), (x + 16.0, y + 36.0), (x, y + 6.0)], rgb(0x3b2412));
        }
        "rock" => {
            pen.ellipse(x, y, 26.0, 18.0, 0.3, rgb(0x6f6a7d));
            pen.ellipse(x - 6.0, y - 6.0, 10.0, 6.0, 0.3, Color32::from_white_alpha(38));
        }
        "log" => bx(x - 40.0, y - 14.0, 80.0, 28.0, 0x6b4423),
        "fire" => {
            // The real fire: walk into it and you catch.
            let t = (env.now / 120.0) as f32;
            pen.disc(x, y, 46.0, rgba(255, 140, 40, 0.16));
            for (i, col) in [0xff6b35, 0xffb347, 0xfff07a].into_iter().enumerate() {
                let i = i as f32;
                pen.ellipse(x, y - i * 10.0, 34.0 - i * 10.0 + (t + i).sin() * 4.0, 44.0 - i * 12.0 + (t + i).cos() * 4.0, 0.0, rgb(col));
            }
        }
        "tagspot" => {
            let on = env.tag_on;
            let color = if on { rgb(0xff7a7a) } else { rgb(0xffd166) };
            pen.disc(x, y, TAG_SPOT.2, if on { rgba(229, 72, 77, 0.28) } else { rgba(255, 214, 102, 0.16) });
            dashed_circle(&pen.p, pen.at(x, y), TAG_SPOT.2, 8.0, 6.0, Stroke::new(3.0, color));
            pen.text(x, y, Align2::CENTER_CENTER, "TAG", 20.0, true, color);
            pen.text(x, y + TAG_SPOT.2 + 14.0, Align2::CENTER_CENTER, if on { "game on!" } else { "step in to play" }, 12.0, true, color);
        }
        "laps" => {
            // The lap records board, on the Plaza (220 wide, like space-art.js).
            pen.boxed(x - 110.0, y - 46.0, 220.0, 92.0, edge, rgb(0xffd166));
            pen.text(x, y - 30.0, Align2::CENTER_CENTER, "🏁 LAP RECORDS", 13.0, true, rgb(0xffd166));
            if env.laps.is_empty() {
                pen.text(x, y - 8.0, Align2::CENTER_CENTER, "Run a lap of the track:", 12.0, false, Color32::WHITE);
                pen.text(x, y + 10.0, Align2::CENTER_CENTER, "start on the left, go clockwise", 12.0, false, Color32::WHITE);
                pen.text(x, y + 28.0, Align2::CENTER_CENTER, "(hold Shift to run)", 12.0, false, Color32::from_white_alpha(153));
            } else {
                for (i, (name, ms)) in env.laps.iter().take(3).enumerate() {
                    let name = if name.chars().count() > 14 { format!("{}…", name.chars().take(13).collect::<String>()) } else { name.clone() };
                    let ry = y - 8.0 + i as f32 * 18.0;
                    pen.text(x - 96.0, ry, Align2::LEFT_CENTER, format!("{}. {name}", i + 1), 12.0, false, Color32::WHITE);
                    pen.text(x + 96.0, ry, Align2::RIGHT_CENTER, secs(*ms), 12.0, false, Color32::WHITE);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn ripples(pen: &Pen, x: f32, y: f32, from: f32, span: f32, clock: f64) {
    for i in 0..3 {
        let r = from + ((clock / 30.0 + i as f64 * (span as f64 / 3.0)) % span as f64) as f32;
        let alpha = 0.5 * (1.0 - (r - from) / span);
        pen.ring(x, y, r, Stroke::new(2.0, Color32::WHITE.gamma_multiply(alpha)));
    }
}

/// The pond: water, lily pads, two ducks paddling round.
pub(super) fn pond(pen: &Pen, clock: f64) {
    let (x, y, rx, ry) = POND;
    pen.ellipse(x, y, rx + 10.0, ry + 10.0, 0.0, rgb(0xc9b77a)); // a sandy edge
    pen.ellipse(x, y, rx, ry, 0.0, rgb(0x2f7fb8));
    pen.ellipse(x - 30.0, y - 40.0, rx * 0.6, ry * 0.45, 0.0, Color32::from_white_alpha(20));
    for (px, py, r) in [(x - 95.0, y - 50.0, 14.0), (x + 90.0, y + 40.0, 16.0), (x - 60.0, y + 95.0, 12.0), (x + 105.0, y - 85.0, 11.0), (x - 110.0, y + 30.0, 10.0)] {
        // A lily pad: a disc with a notch.
        let mut pts = vec![(px, py)];
        for k in 0..=20 {
            let a = 0.4 + (TAU - 0.5) * k as f32 / 20.0;
            pts.push((px + a.cos() * r, py + a.sin() * r));
        }
        pen.poly(&pts, rgb(0x4caf50));
    }
    pen.disc(x - 60.0, y + 95.0, 4.0, rgb(0xff9ecb));
    let t = (clock / 5000.0) as f32;
    for (phase, size) in [(0.0f32, 1.0f32), (PI, 0.85)] {
        let a = t * if size == 1.0 { 1.0 } else { -1.3 } + phase;
        let (dx, dy) = (x + a.cos() * rx * 0.55, y + 40.0 + a.sin() * ry * 0.5);
        let heading = a + if size == 1.0 { FRAC_PI_2 } else { -FRAC_PI_2 };
        let (cs, sn) = (heading.cos(), heading.sin());
        let tf = |lx: f32, ly: f32| (dx + (lx * cs - ly * sn) * size, dy + (lx * sn + ly * cs) * size);
        let (wx, wy) = tf(-10.0, 0.0);
        pen.ellipse(wx, wy, 16.0 * size, 9.0 * size, heading, Color32::from_white_alpha(64));
        let (bx, by) = tf(0.0, 0.0);
        pen.disc(bx, by, 10.0 * size, rgb(0xfff4d6));
        let (hx, hy) = tf(9.0, 0.0);
        pen.disc(hx, hy, 6.0 * size, rgb(0x2e7d32));
        pen.poly(&[tf(14.0, -2.0), tf(20.0, 0.0), tf(14.0, 2.0)], rgb(0xff9800));
    }
}

/// The floor piano: white keys, black keys, a lit key under anyone standing on it.
pub(super) fn piano(pen: &Pen, down: &HashSet<String>) {
    let (px, py) = PIANO;
    let n = WHITE.len() as f32;
    pen.fill(px - 6.0, py - 8.0, KEY_W * n + 12.0, KEY_H + 14.0, rgb(EDGE));
    for i in 0..WHITE.len() {
        pen.fill(px + i as f32 * KEY_W + 1.0, py, KEY_W - 2.0, KEY_H, if down.contains(&format!("w{i}")) { rgb(0xffe08a) } else { rgb(0xf4f1e8) });
    }
    for (i, _) in BLACK {
        let cx = px + (i + 1) as f32 * KEY_W;
        pen.fill(cx - BLACK_W / 2.0, py, BLACK_W, BLACK_H, if down.contains(&format!("b{i}")) { rgb(0x8f7bff) } else { rgb(0x16141f) });
    }
    pen.text(px + KEY_W * n / 2.0, py + KEY_H + 22.0, Align2::CENTER_CENTER, "♪ walk on the keys and the drums ♪", 12.0, false, Color32::from_white_alpha(191));
}

/// The cinema screen: a new shot every couple of seconds, and the projector's beam.
pub(super) fn screen(pen: &Pen, x: f32, y: f32, now: f64) {
    let half = 230.0;
    pen.poly(&[(x - half, y + 30.0), (x + half, y + 30.0), (x + 30.0, y + 330.0), (x - 30.0, y + 330.0)], Color32::from_white_alpha(13));
    pen.fill(x - half - 12.0, y - 34.0, half * 2.0 + 24.0, 68.0, rgb(EDGE));
    let (sx, sy, sw, sh) = (x - half, y - 26.0, half * 2.0, 52.0);
    let rect = pen.rect(sx, sy, sw, sh);
    let p = Pen { p: pen.p.with_clip_rect(rect.intersect(pen.p.clip_rect())), origin: pen.origin };
    match ((now / 2500.0).floor() as i64) % 4 {
        0 => {
            // Sunset: a vertical gradient, the sun, hills.
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(rect.left_top(), rgb(0xff9f45));
            mesh.colored_vertex(rect.right_top(), rgb(0xff9f45));
            mesh.colored_vertex(rect.right_bottom(), rgb(0x8e4ec6));
            mesh.colored_vertex(rect.left_bottom(), rgb(0x8e4ec6));
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            p.p.add(Shape::mesh(mesh));
            p.disc(x + 100.0, sy + sh - 6.0, 22.0, rgb(0xffe07a));
            for i in 0..8 {
                let (x0, x1) = (sx + i as f32 * sw / 8.0, sx + (i + 1) as f32 * sw / 8.0);
                let (y0, y1) = (sy + sh - 10.0 - (i % 2) as f32 * 12.0, sy + sh - 10.0 - ((i + 1) % 2) as f32 * 12.0);
                p.poly(&[(x0, y0), (x1, y1), (x1, sy + sh), (x0, sy + sh)], rgb(0x2b1d3a));
            }
        }
        1 => {
            p.fill(sx, sy, sw, sh, rgb(0x0b1030));
            for i in 0..34 {
                p.fill(sx + ((i * 97) as f32 % sw), sy + ((i * 37) as f32 % sh), 2.0, 2.0, Color32::WHITE);
            }
            p.disc(x - 120.0, y, 18.0, rgb(0xff7eb6));
        }
        2 => {
            p.fill(sx, sy, sw, sh, rgb(0x2a8fd6));
            for row in 0..3 {
                let pts: Vec<(f32, f32)> = (0..=24).map(|i| (sx + i as f32 * 20.0, sy + 14.0 + row as f32 * 14.0 + (i % 2) as f32 * 4.0)).collect();
                p.line(&pts, Stroke::new(2.0, rgb(0xbfe6ff)));
            }
            p.poly(&[(x + 40.0, sy + 8.0), (x + 40.0, sy + 34.0), (x + 64.0, sy + 34.0)], Color32::WHITE);
        }
        _ => {
            p.fill(sx, sy, sw, sh, rgb(0x111111));
            p.text(x, y, Align2::CENTER_CENTER, "YAP PICTURES", 24.0, true, rgb(0xffd166));
        }
    }
}

pub(super) fn ball(pen: &Pen, b: &Ball) {
    let (x, y) = (b.x + b.ox, b.y + b.oy);
    pen.ellipse(x + 3.0, y + BALL_R - 2.0, BALL_R, 5.0, 0.0, Color32::from_black_alpha(77));
    pen.disc(x, y, BALL_R, Color32::WHITE);
    pen.ring(x, y, BALL_R, Stroke::new(2.0, rgb(EDGE)));
    // Patches turn as it rolls.
    let spin = (b.x + b.y) / BALL_R;
    for i in 0..3 {
        let a = spin + i as f32 * TAU / 3.0;
        pen.disc(x + a.cos() * 7.0, y + a.sin() * 7.0, 3.2, rgb(EDGE));
    }
}

/// A water balloon in flight: its shadow on the ground, the balloon `lift` above it.
pub(super) fn balloon(pen: &Pen, x: f32, y: f32, lift: f32) {
    pen.ellipse(x, y + 8.0, 8.0, 4.0, 0.0, Color32::from_black_alpha(64));
    let by = y - lift;
    pen.ellipse(x, by, 9.0, 11.0, 0.0, rgb(0x4fa3ff));
    pen.p.add(Shape::closed_line((0..36).map(|i| {
        let a = i as f32 / 36.0 * TAU;
        pen.at(x + 9.0 * a.cos(), by + 11.0 * a.sin())
    }).collect(), Stroke::new(2.0, rgb(0x1f5d9e))));
    pen.disc(x - 3.0, by - 4.0, 2.5, Color32::from_white_alpha(204));
    pen.poly(&[(x - 3.0, by + 13.0), (x + 3.0, by + 13.0), (x, by + 9.0)], rgb(0x1f5d9e));
}

pub(super) fn puddle(pen: &Pen, x: f32, y: f32, alpha: f32) {
    pen.ellipse(x, y, 13.0, 6.0, 0.0, rgb(0x5fb8ff).gamma_multiply(alpha));
    pen.ellipse(x - 4.0, y - 1.0, 4.0, 1.5, 0.0, Color32::from_white_alpha(128).gamma_multiply(alpha));
}

/// Someone on fire: flickering flames over their head and embers floating up.
pub(super) fn flames(pen: &Pen, x: f32, y: f32, clock: f64) {
    let t = clock as f32;
    for (color, scale) in [(0xff5a1f, 1.0f32), (0xffb347, 0.62), (0xfff07a, 0.3)] {
        for i in 0..5 {
            let k = (i as f32 - 2.0).abs();
            let fx = x + (i as f32 - 2.0) * 9.0;
            let base = y - 8.0 - k * 4.0;
            let h = (26.0 - k * 6.0 + (t / 70.0 + i as f32 * 1.9).sin() * 6.0) * scale;
            let w = 7.0 * (0.6 + scale * 0.4);
            let tip = (fx + (t / 90.0 + i as f32).sin() * 2.0, base - h - 8.0 * scale);
            let mut pts = vec![(fx - w, base)];
            pts.extend(quad((fx - w, base), (fx - w, base - h * 0.6), tip, 6));
            pts.extend(quad(tip, (fx + w, base - h * 0.6), (fx + w, base), 6));
            pen.poly(&pts, rgb(color));
        }
    }
    for i in 0..6 {
        let ph = ((t / 900.0 + i as f32 / 6.0) % 1.0).abs();
        let c = if i % 2 == 1 { rgb(0xffb347) } else { rgb(0xff7a1a) };
        pen.disc(x + (i as f32 * 2.7 + ph * 5.0).sin() * 18.0, y - RADIUS - 6.0 - ph * 48.0, 2.4, c.gamma_multiply(1.0 - ph));
    }
}

/// Someone dripping: a wet sheen on them and drops falling off.
pub(super) fn wet(pen: &Pen, x: f32, y: f32, clock: f64) {
    let t = clock as f32;
    pen.disc(x, y, RADIUS, rgba(90, 170, 255, 0.22));
    let arc: Vec<(f32, f32)> = (0..=10).map(|k| {
        let a = -2.7 + 1.5 * k as f32 / 10.0;
        (x + a.cos() * (RADIUS - 5.0), y + a.sin() * (RADIUS - 5.0))
    }).collect();
    pen.line(&arc, Stroke::new(3.0, rgba(170, 225, 255, 0.9)));
    for i in 0..4 {
        let ph = (t / 650.0 + i as f32 * 0.27) % 1.0;
        let dx = x + (i as f32 - 1.5) * 13.0;
        let dy = y + RADIUS * 0.3 + ph * 34.0;
        let mut pts = vec![(dx, dy - 6.0)];
        pts.extend(quad((dx, dy - 6.0), (dx + 4.0, dy), (dx, dy + 3.0), 5));
        pts.extend(quad((dx, dy + 3.0), (dx - 4.0, dy), (dx, dy - 6.0), 5));
        pen.poly(&pts, rgb(0x6cc4ff).gamma_multiply(1.0 - ph));
    }
}

pub(super) const CONFETTI: [u32; 6] = [0xff6ad5, 0x7afcff, 0xfff07a, 0x8aff80, 0xff9f45, 0x9aa3ff];
/// Short-lived bursts: steam (fire meets water), a splash, a note from an
/// instrument, confetti for a goal. `age` in ms; false once it's over.
pub(super) fn burst(pen: &Pen, b: &Burst, x: f32, y: f32, age: f32) -> bool {
    let life = match b.kind {
        BurstKind::Steam => 900.0,
        BurstKind::Splash => 500.0,
        BurstKind::Note(..) => 1200.0,
        BurstKind::Confetti(_) => 2400.0,
    };
    if age > life {
        return false;
    }
    let ph = age / life;
    match &b.kind {
        BurstKind::Steam => {
            for i in 0..5 {
                let c = rgb(0xe8eef4).gamma_multiply((1.0 - ph) * 0.7);
                pen.disc(x + (i as f32 * 1.7).sin() * 16.0 * (0.4 + ph), y - RADIUS - ph * 42.0 - i as f32 * 5.0, 7.0 + ph * 10.0, c);
            }
        }
        BurstKind::Splash => {
            pen.ring(x, y, 8.0 + ph * 24.0, Stroke::new(2.0, rgb(0x8fd3ff).gamma_multiply(1.0 - ph)));
            for i in 0..8 {
                let a = i as f32 * PI / 4.0;
                pen.disc(x + a.cos() * (6.0 + ph * 30.0), y + a.sin() * (6.0 + ph * 30.0) - (ph * PI).sin() * 10.0, 3.0, rgb(0x6cc4ff).gamma_multiply(1.0 - ph));
            }
        }
        BurstKind::Note(glyph, color) => {
            pen.text(x + (ph * 6.0).sin() * 8.0, y - ph * 44.0 - 8.0, Align2::CENTER_CENTER, glyph, 22.0, true, color.gamma_multiply(1.0 - ph));
        }
        BurstKind::Confetti(_) => {
            for i in 0..16 {
                let a = -FRAC_PI_2 + ((i as f32 - 7.5) / 8.0) * 1.3;
                let v = 60.0 + (i % 4) as f32 * 18.0;
                let c = rgb(CONFETTI[i % CONFETTI.len()]).gamma_multiply(1.0 - ph * ph);
                pen.fill(x + a.cos() * v * ph * 1.4, y - RADIUS - 10.0 + a.sin() * v * ph + ph * ph * 120.0, 5.0, 8.0, c);
            }
        }
    }
    true
}

pub(super) fn emote(painter: &egui::Painter, m: &Model, sid: &str, c: Pos2) {
    let Some((e, at)) = m.space.emotes.get(sid) else { return };
    let t = at.elapsed().as_secs_f32();
    if t >= EMOTE_SECS {
        return;
    }
    let pop = (t / 0.16).min(1.0);
    let scale = 0.6 + 0.4 * pop + if t < 0.3 { (t / 0.3 * PI).sin() * 0.15 } else { 0.0 };
    let alpha = if t > EMOTE_SECS - 0.4 { (EMOTE_SECS - t) / 0.4 } else { 1.0 };
    let c = c - vec2(0.0, RADIUS + 58.0 + (t * 1000.0 / 120.0).min(14.0));
    painter.circle_filled(c, 24.0 * scale, Color32::WHITE.gamma_multiply(alpha));
    painter.add(Shape::convex_polygon(vec![c + vec2(-6.0, 20.0) * scale, c + vec2(0.0, 32.0) * scale, c + vec2(6.0, 20.0) * scale], Color32::WHITE.gamma_multiply(alpha), Stroke::NONE));
    painter.text(c + vec2(0.0, 2.0), Align2::CENTER_CENTER, e, egui::FontId::proportional(26.0 * scale), Color32::BLACK.gamma_multiply(alpha));
}

/// How someone is drawn right now.
#[derive(Clone, Copy)]
pub(super) struct Look {
    pub(super) me: bool,
    pub(super) sitting: bool,
    pub(super) it: bool,
    pub(super) burning: bool,
    pub(super) wet: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn player(images: &super::super::images::Images, pen: &Pen, m: &Model, sid: &str, at: (f32, f32), d: f32, look: Look, now: f64, clock: f64) {
    let user = m.sessions.get(sid).and_then(|s| m.users.get(&s.uid)).or_else(|| if look.me { m.users.get(&m.config.uid) } else { None });
    let Some(user) = user else { return };
    let (x, y) = if look.sitting { (at.0, at.1 + 8.0) } else { at };
    let painter = &pen.p;
    let c = pen.at(x, y);
    // Shadow, the glow of the fire, the "it" ring, the facing nub.
    pen.ellipse(x, y + RADIUS - 2.0, RADIUS * 0.9, 7.0, 0.0, Color32::from_black_alpha(77));
    if look.burning {
        pen.disc(x, y, RADIUS + 10.0, rgba(255, 110, 30, 0.28));
    }
    if look.it {
        let beat = ((clock % 1000.0) / 1000.0) as f32;
        painter.circle_stroke(c, RADIUS + 9.0 + (beat * TAU).sin() * 3.0, Stroke::new(4.0, rgb(0xff4d4f)));
    }
    painter.circle_filled(c + vec2(d.cos(), d.sin()) * (RADIUS + 6.0), 5.0, Color32::WHITE);
    let label = format!("{}{}{}", if look.it { "IT · " } else { "" }, user.name, if look.me { " (you)" } else { "" });
    match user.look.as_deref().filter(|l| !l.is_empty()) {
        Some(emoji) => {
            let bob = if look.sitting { 0.0 } else { ((now / 260.0) as f32 + c.x).sin() * 2.0 };
            painter.text(c + vec2(0.0, 2.0 + bob), Align2::CENTER_CENTER, emoji, egui::FontId::proportional(RADIUS * 2.0), Color32::WHITE);
            if look.me {
                painter.circle_stroke(c, RADIUS + 4.0, Stroke::new(2.0, Color32::from_white_alpha(178)));
            }
        }
        None => {
            painter.circle_filled(c, RADIUS, theme::hex_color(&user.color));
            let pic = user.avatar.as_deref().filter(|a| !a.is_empty()).and_then(|id| images.get_square(&format!("{}/media/{id}", m.config.server)));
            match pic {
                Some(tex) => theme::image_rounded(painter, Rect::from_center_size(c, vec2((RADIUS - 3.0) * 2.0, (RADIUS - 3.0) * 2.0)), RADIUS - 3.0, tex),
                None => {
                    let initial: String = user.name.trim().chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_else(|| "?".into());
                    painter.text(c + vec2(0.0, 1.0), Align2::CENTER_CENTER, initial, egui::FontId::new(20.0, theme::bold()), Color32::WHITE);
                }
            }
            painter.circle_stroke(c, RADIUS, Stroke::new(3.0, if look.me { Color32::WHITE } else { rgb(EDGE) }));
        }
    }
    if look.wet {
        wet(pen, x, y, clock);
    }
    if look.burning {
        flames(pen, x, y, clock);
    }
    // Name tag (red for "it").
    let g = painter.layout_no_wrap(label, egui::FontId::new(13.0, theme::bold()), Color32::WHITE);
    let r = Rect::from_center_size(pos2(c.x, c.y - RADIUS - 20.0), vec2(g.size().x + 12.0, 20.0));
    painter.rect_filled(r, 0.0, if look.it { rgb(0xc62828) } else { Color32::from_rgba_unmultiplied(13, 11, 30, 217) });
    painter.rect_stroke(r, 0.0, Stroke::new(2.0, rgb(0x06050d)), egui::StrokeKind::Middle);
    painter.galley(r.center() - g.size() / 2.0, g, Color32::WHITE);
}
