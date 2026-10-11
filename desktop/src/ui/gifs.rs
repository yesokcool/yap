//! GIF picker backed by KLIPY (see PROTOCOL.md › GIFs), following their rules:
//! API calls and media loads go straight from this app to KLIPY (never via the
//! Yap server), URLs are used exactly as returned, results stay in the order
//! given, "Powered by KLIPY" is shown, and every send is reported back.
//! `YAP_KLIPY_API` overrides the API base (tests point it at a local mock).

use super::{Act, App, chat::picker_window, theme, theme::Pal};
use crate::core::Model;
use crate::klipy::{self, Mode};
use crate::protocol::Gif;
use eframe::egui::{self, Align2, Color32, Rect, vec2};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_millis(400);

#[derive(Default)]
struct Results {
    items: Vec<Gif>,
    status: String,
    loading: bool,
    has_next: bool,
    page: u32,
    generation: u64,
}

pub struct GifPicker {
    pub open: bool,
    query: String,
    edited: Option<Instant>,
    mode: Mode,
    results: Arc<Mutex<Results>>,
    generation: u64,
}

impl Default for GifPicker {
    fn default() -> Self {
        Self { open: false, query: String::new(), edited: None, mode: Mode::Trending, results: Arc::new(Mutex::new(Results::default())), generation: 0 }
    }
}

impl GifPicker {
    fn load(&mut self, ctx: &egui::Context, m: &Model, mode: Mode, query: String, page: u32) {
        let Some(key) = m.server_config.as_ref().and_then(|c| c.gifs.as_ref()).map(|g| g.app_key.clone()) else {
            self.results.lock().status = "GIFs aren't set up on this server yet (needs a KLIPY key).".into();
            return;
        };
        if page == 1 {
            self.generation += 1;
            let mut r = self.results.lock();
            *r = Results { generation: self.generation, ..Default::default() };
        }
        self.mode = mode;
        let generation = self.generation;
        {
            let mut r = self.results.lock();
            r.loading = true;
            r.status = "Loading…".into();
            r.page = page;
        }
        let uid = m.config.uid.clone();
        let results = self.results.clone();
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("yap-klipy".into()).spawn(move || {
            let outcome = klipy::fetch(&key, &uid, mode, &query, page);
            let mut r = results.lock();
            if r.generation != generation {
                return; // a newer search replaced this one
            }
            r.loading = false;
            match outcome {
                Ok((fresh, has_next)) => {
                    r.has_next = has_next;
                    r.items.extend(fresh);
                    r.status = if !r.items.is_empty() {
                        String::new()
                    } else if mode == Mode::Recent {
                        "GIFs you send show up here.".into()
                    } else {
                        "No GIFs found.".into()
                    };
                }
                Err(e) if e.contains("429") => r.status = "GIF search is resting — the free KLIPY key allows 100 searches an hour. Try again soon.".into(),
                Err(_) => r.status = "Couldn't reach KLIPY right now.".into(),
            }
            drop(r);
            ctx.request_repaint();
        });
    }
}

pub fn report_share(m: &Model, slug: &str, query: &str) {
    let Some(key) = m.server_config.as_ref().and_then(|c| c.gifs.as_ref()).map(|g| g.app_key.clone()) else { return };
    let (uid, slug, query) = (m.config.uid.clone(), slug.to_string(), query.to_string());
    let _ = std::thread::Builder::new().name("yap-klipy-share".into()).spawn(move || klipy::report_share(&key, &uid, &slug, &query));
}

pub fn toggle(app: &mut App, ctx: &egui::Context, m: &Model) {
    let picker = &mut app.gif_picker;
    picker.open = !picker.open;
    if picker.open && (picker.results.lock().items.is_empty() || picker.mode == Mode::Recent) {
        let (mode, q) = (picker.mode, picker.query.clone());
        picker.load(ctx, m, mode, q, 1);
    }
}

pub fn show(app: &mut App, ctx: &egui::Context, m: &Model, p: &Pal, acts: &mut Vec<Act>, anchor: Rect) {
    if !app.gif_picker.open {
        return;
    }
    // Debounced search.
    if let Some(t) = app.gif_picker.edited
        && t.elapsed() >= DEBOUNCE
    {
        app.gif_picker.edited = None;
        let q = app.gif_picker.query.trim().to_string();
        let mode = if q.is_empty() { Mode::Trending } else { Mode::Search };
        app.gif_picker.load(ctx, m, mode, q, 1);
    } else if let Some(t) = app.gif_picker.edited {
        ctx.request_repaint_after(DEBOUNCE.saturating_sub(t.elapsed()));
    }
    let size = vec2(420.0, 440.0);
    // Opens from the GIF button's left edge.
    let x = anchor.left();
    let pos = egui::pos2(x.max(8.0), anchor.top() - size.y - 6.0);
    let mut picked: Option<Gif> = None;
    let mut want_more = false;
    let mut switch: Option<Mode> = None;
    let area = egui::Area::new(egui::Id::new("gif-picker")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        picker_window(ui, "GIFs", size.x - 6.0, p, |ui| {
            let r = theme::field(ui, egui::TextEdit::singleline(&mut app.gif_picker.query).hint_text("Search KLIPY").desired_width(f32::INFINITY), p);
            if r.changed() {
                app.gif_picker.edited = Some(Instant::now());
            }
            if r.gained_focus() || !r.has_focus() && app.gif_picker.query.is_empty() {
                r.request_focus();
            }
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                let mode = app.gif_picker.mode;
                if theme::toggle_button(ui, "Trending", mode == Mode::Trending, vec2(0.0, 21.0), p).clicked() {
                    switch = Some(Mode::Trending);
                }
                if theme::toggle_button(ui, "Recent", mode == Mode::Recent, vec2(0.0, 21.0), p).clicked() {
                    switch = Some(Mode::Recent);
                }
            });
            ui.add_space(4.0);
            let results = app.gif_picker.results.clone();
            let res = results.lock();
            let well = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), size.y - 140.0));
            ui.allocate_rect(well, egui::Sense::hover());
            let scroll = theme::well(ui, well, p, |ui| {
                theme::scroll(ui, egui::ScrollArea::vertical().auto_shrink([false, false]), p, |ui| {
                    // Masonry: each tile into the shorter column keeps reading order left→right, top→bottom.
                    let gap = 4.0;
                    let pad = 4.0;
                    let col_w = ((ui.available_width() - gap - 2.0 * pad) / 2.0).floor();
                    let origin = ui.cursor().min + vec2(pad, pad);
                    let mut heights = [0f32; 2];
                    for (i, g) in res.items.iter().enumerate() {
                        let col = if heights[0] <= heights[1] { 0 } else { 1 };
                        let h = if g.width > 0.0 { col_w * g.height / g.width } else { col_w * 0.6 }.clamp(40.0, 400.0).round();
                        let rect = Rect::from_min_size(origin + vec2(col as f32 * (col_w + gap), heights[col]), vec2(col_w, h));
                        heights[col] += h + gap;
                        let resp = ui.interact(rect, ui.id().with(("gif", i)), egui::Sense::click());
                        if ui.is_rect_visible(rect) {
                            let still = g.still.as_ref().or(g.webp.as_ref()).or(g.gif.as_ref());
                            let moving = g.webp.as_ref().or(g.gif.as_ref());
                            let hovered = resp.hovered();
                            let tex = if hovered { moving.and_then(|u| app.images.get(u, true)) } else { None }.or_else(|| still.and_then(|u| app.images.get(u, false)));
                            match tex {
                                Some((t, _)) => {
                                    ui.painter().image(t, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                                }
                                None => {
                                    ui.painter().rect_filled(rect, 0.0, p.code_bg);
                                }
                            }
                            if hovered {
                                ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(2.0, p.select), egui::StrokeKind::Inside);
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                        }
                        if theme::tip(resp, g.title.clone()).clicked() {
                            picked = Some(g.clone());
                        }
                    }
                    let total = heights[0].max(heights[1]) + 2.0 * pad;
                    ui.allocate_space(vec2(ui.available_width(), total));
                    if !res.status.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(12.0);
                            ui.label(theme::text(res.status.clone(), p.gray));
                        });
                    }
                    total
                })
            });
            // Near the bottom: next page.
            let total = scroll.inner;
            if res.has_next && !res.loading && scroll.state.offset.y + scroll.inner_rect.height() > total - 300.0 {
                want_more = true;
            }
            drop(res);
            ui.add_space(3.0);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), egui::Sense::click());
            theme::status_cell(ui.painter(), r, p);
            let g = ui.painter().layout_no_wrap("KLIPY".into(), theme::bold_font(theme::small()), p.text);
            let gw = g.size().x;
            ui.painter().galley(egui::pos2(r.right() - 6.0 - gw, r.center().y - g.size().y / 2.0), g, p.text);
            ui.painter().text(egui::pos2(r.right() - 9.0 - gw, r.center().y), Align2::RIGHT_CENTER, "Powered by", theme::font(theme::small()), p.gray);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                acts.push(Act::OpenUrl("https://klipy.com".into()));
            }
        });
    });
    if let Some(mode) = switch {
        app.gif_picker.query.clear();
        app.gif_picker.load(ctx, m, mode, String::new(), 1);
    } else if want_more {
        let (mode, q, page) = (app.gif_picker.mode, app.gif_picker.query.trim().to_string(), app.gif_picker.results.lock().page + 1);
        app.gif_picker.load(ctx, m, mode, q, page);
    }
    if let Some(g) = picked {
        // Preview before sending: it waits in the tray above the composer.
        let q = if app.gif_picker.mode == Mode::Search { app.gif_picker.query.trim().to_string() } else { String::new() };
        if app.editing.is_some() {
            super::chat::cancel_edit(app);
        }
        app.staged = Some(super::Staged::Gif { gif: g, query: q });
        app.gif_picker.open = false;
        app.focus_composer = true;
    }
    let outside = ctx.input(|i| i.pointer.any_pressed())
        && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pos| !area.response.rect.contains(pos) && !anchor.contains(pos));
    if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.gif_picker.open = false;
    }
}
