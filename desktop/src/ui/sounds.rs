//! Adding a soundboard clip or your join chime: pick a file (WAV, MP3, M4A,
//! OGG, FLAC, Opus…), trim it to ≤ 5 s (a chime: ≤ 2 s), hear it, then it's
//! mixed to mono, resampled to 24 kHz, encoded as a 16-bit WAV and posted to
//! `/api/sound` (PROTOCOL.md › Extras › Soundboard). The server measures the
//! clip's loudness and caps how loud it ever plays.

use super::dialogs::{actions, field_label, note, window};
use super::{Act, theme, theme::Pal};
use crate::audio::decode::{self, Decoded};
use crate::core::{Cmd, Model, Preview};
use eframe::egui::{self, Rect, Sense, pos2, vec2};
use parking_lot::Mutex;
use std::sync::Arc;

/// Longest soundboard clip / join chime (seconds).
const BOARD_MAX: f32 = 5.0;
const CHIME_MAX: f32 = 2.0;
/// Shortest clip worth sending.
const MIN_LEN: f32 = 0.2;
/// Waveform columns.
const BARS: usize = 140;

type Slot = Arc<Mutex<Option<Result<Arc<Decoded>, String>>>>;

/// The upload dialog's state.
pub struct Upload {
    /// "board" or "chime".
    pub kind: String,
    pub file_name: String,
    /// Decoding in the background.
    slot: Slot,
    decoded: Option<Result<Arc<Decoded>, String>>,
    /// The waveform (peak per column, 0..1, over the whole file).
    peaks: Vec<f32>,
    /// The part to send (seconds).
    start: f32,
    len: f32,
    name: String,
    emoji: String,
}

impl Upload {
    pub fn new(kind: &str, file_name: &str, bytes: Vec<u8>) -> Self {
        let slot: Slot = Arc::new(Mutex::new(None));
        let (out, data, name) = (slot.clone(), bytes, file_name.to_string());
        let _ = std::thread::Builder::new().name("yap-sound-decode".into()).spawn(move || {
            let r = decode::decode(&data, &name).map(Arc::new);
            *out.lock() = Some(r);
        });
        let stem = std::path::Path::new(file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name: String = stem.replace(['_', '-'], " ").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(24).collect();
        Self { kind: kind.into(), file_name: file_name.into(), slot, decoded: None, peaks: vec![], start: 0.0, len: 0.0, name, emoji: String::new() }
    }

    fn chime(&self) -> bool {
        self.kind == "chime"
    }

    fn max_len(&self) -> f32 {
        if self.chime() { CHIME_MAX } else { BOARD_MAX }
    }

    /// The decode just finished: the waveform and the default selection
    /// (from where the sound starts, as long as it may be).
    fn ready(&mut self, d: &Decoded) {
        let total = d.seconds();
        let per = (d.samples.len() / BARS).max(1);
        let peaks: Vec<f32> = d.samples.chunks(per).map(|c| c.iter().fold(0f32, |a, x| a.max(x.abs()))).collect();
        let top = peaks.iter().fold(0f32, |a, x| a.max(*x)).max(1e-6);
        self.peaks = peaks.into_iter().map(|x| x / top).collect();
        self.start = (decode::sound_start(d) as f32 / d.rate.max(1) as f32).min((total - MIN_LEN).max(0.0));
        self.len = self.max_len().min(total - self.start).max(0.0);
    }
}

/// The emoji field holds one emoji (or nothing).
fn emoji_ok(e: &str) -> bool {
    let e = e.trim();
    e.is_empty() || (e.chars().count() <= 8 && !e.chars().any(|c| c.is_ascii_alphanumeric() || c.is_whitespace()))
}

fn seconds(s: f32) -> String {
    format!("{s:.1} s")
}

/// The dialog's inside (drawn in a Modal by `dialogs::modals`).
pub fn show(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, u: &mut Upload, keep: &mut bool) {
    if u.decoded.is_none() {
        let done = u.slot.lock().take();
        if let Some(r) = done {
            if let Ok(d) = &r {
                let d = d.clone();
                u.ready(&d);
            }
            u.decoded = Some(r);
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    let chime = u.chime();
    let max = u.max_len();
    let title = if chime { "Your join chime" } else { "Add a sound" };
    let mut k = true;
    window(ui, p, title, Some("volume"), 400.0, Some(&mut k), |ui| {
        // (No-break spaces: "(≤ 5 s)" never splits over two lines.)
        ui.label(theme::text(if chime { "Everyone hears it when you join their voice channel (≤\u{a0}2\u{a0}s)." } else { "Anyone in your voice channel can play it (≤\u{a0}5\u{a0}s)." }, p.text));
        let decoded = u.decoded.clone();
        let mut upload: Option<Vec<u8>> = None;
        match decoded {
            None => {
                ui.add_space(6.0);
                note(ui, format!("Reading {}…", u.file_name), p);
                ui.add_space(6.0);
            }
            Some(Err(e)) => {
                ui.add_space(6.0);
                ui.label(theme::text("Yap can't read that file — try WAV or MP3.", p.danger));
                note(ui, format!("{}: {e}", u.file_name), p);
            }
            Some(Ok(d)) => {
                let total = d.seconds();
                note(ui, format!("{} · {}", u.file_name, seconds(total)), p);
                ui.add_space(2.0);
                waveform(ui, u, total, p);
                // Start and length (the strip moves the start too).
                let span = (total - MIN_LEN).max(0.0);
                let longest = max.min(total - u.start).max(MIN_LEN.min(total));
                egui::Grid::new("sound-trim").num_columns(3).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
                    ui.label(theme::text("Start", p.text));
                    let mut s = u.start;
                    if theme::trackbar(ui, &mut s, 0.0..=span.max(0.01), 0.05, 240.0, p).changed() {
                        u.start = s.min(span);
                        u.len = u.len.min(total - u.start).max(MIN_LEN.min(total));
                    }
                    ui.label(theme::text(seconds(u.start), p.text));
                    ui.end_row();
                    ui.label(theme::text("Length", p.text));
                    let mut l = u.len;
                    if theme::trackbar(ui, &mut l, MIN_LEN.min(longest)..=longest.max(0.01), 0.1, 240.0, p).changed() {
                        u.len = l.clamp(MIN_LEN.min(longest), longest);
                    }
                    ui.label(theme::text(seconds(u.len), p.text));
                    ui.end_row();
                });
                let clip = |u: &Upload| decode::clip_wav(&d, u.start, u.len);
                if total > max {
                    note(ui, format!("Press or drag on the wave to pick the part (up to {}).", seconds(max)), p);
                }
                if theme::tip(theme::button_sized(ui, "Play", theme::Btn::Normal, u.len > 0.0, vec2(72.0, 23.0), p), "Hear the part you picked (only you)").clicked() {
                    acts.push(Act::Cmd(Cmd::PreviewSound(Preview::Wav(Arc::new(clip(u))))));
                }
                if !chime {
                    field_label(ui, "Name:", p);
                    theme::field(ui, egui::TextEdit::singleline(&mut u.name).char_limit(24).hint_text("Airhorn").desired_width(f32::INFINITY), p);
                    field_label(ui, "Emoji (optional):", p);
                    theme::field(ui, egui::TextEdit::singleline(&mut u.emoji).char_limit(8).hint_text("🎉").desired_width(60.0), p);
                    if !emoji_ok(&u.emoji) {
                        ui.label(theme::text("Just one emoji (or leave it empty).", p.danger));
                    }
                } else {
                    note(ui, "It replaces the chime you have now. You can go back to a built-in one in Settings › Voice.", p);
                }
                let named = chime || (1..=24).contains(&u.name.trim().chars().count());
                let ok = named && emoji_ok(&u.emoji) && u.len >= MIN_LEN.min(total) && m.status == crate::core::Status::Online;
                actions(ui, |ui| {
                    if theme::button(ui, "Cancel", theme::Btn::Normal, true, p).clicked() {
                        *keep = false;
                    }
                    if super::primary_button(ui, if chime { "Use it" } else { "Add sound" }, p, ok).clicked() {
                        upload = Some(clip(u));
                    }
                });
            }
        }
        if !matches!(u.decoded, Some(Ok(_))) {
            actions(ui, |ui| {
                if theme::button(ui, "Cancel", theme::Btn::Normal, true, p).clicked() {
                    *keep = false;
                }
            });
        }
        if let Some(wav) = upload {
            let emoji = Some(u.emoji.trim().to_string()).filter(|e| !e.is_empty());
            let name = if chime { "chime".to_string() } else { u.name.trim().to_string() };
            acts.push(Act::Cmd(Cmd::UploadSound { kind: u.kind.clone(), name, emoji, wav }));
            *keep = false;
        }
    });
    *keep &= k;
}

/// The whole file as a strip of peaks, the chosen part lit; press or drag to
/// move the start there.
fn waveform(ui: &mut egui::Ui, u: &mut Upload, total: f32, p: &Pal) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click_and_drag());
    let inner = rect.shrink(4.0);
    if let Some(pos) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on())
        && total > 0.0
    {
        let t = ((pos.x - inner.left()) / inner.width()).clamp(0.0, 1.0) * total;
        u.start = t.min((total - MIN_LEN).max(0.0));
        u.len = u.len.max(MIN_LEN).min(total - u.start).min(u.max_len());
        if u.len < MIN_LEN {
            u.len = (total - u.start).max(0.0);
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let (start, len, peaks) = (u.start, u.len, u.peaks.clone());
    theme::well(ui, rect, p, |ui| {
        // (Fill the well, so the layout carries on under it.)
        ui.allocate_space(ui.available_size());
        let painter = ui.painter();
        let x_of = |t: f32| inner.left() + (t / total.max(1e-6)) * inner.width();
        let sel = Rect::from_min_max(pos2(x_of(start), inner.top()), pos2(x_of(start + len), inner.bottom()));
        painter.rect_filled(sel, 0.0, p.select.gamma_multiply(0.22));
        let n = peaks.len().max(1);
        let w = inner.width() / n as f32;
        for (i, peak) in peaks.iter().enumerate() {
            let x = inner.left() + (i as f32 + 0.5) * w;
            let t = (i as f32 + 0.5) / n as f32 * total;
            let h = (peak * inner.height() * 0.95).max(1.0);
            let on = t >= start && t <= start + len;
            painter.line_segment([pos2(x, inner.center().y - h / 2.0), pos2(x, inner.center().y + h / 2.0)], egui::Stroke::new((w * 0.6).max(1.0), if on { p.link } else { p.gray.gamma_multiply(0.6) }));
        }
        for x in [sel.left(), sel.right()] {
            painter.line_segment([pos2(x, inner.top()), pos2(x, inner.bottom())], egui::Stroke::new(2.0, p.select));
        }
    });
    theme::tip(resp, "Press or drag to choose where the sound starts");
}

#[cfg(test)]
mod tests {
    #[test]
    fn emoji_field() {
        assert!(super::emoji_ok(""));
        assert!(super::emoji_ok("📯"));
        assert!(super::emoji_ok(" 🎉 "));
        assert!(!super::emoji_ok("ab"));
        assert!(!super::emoji_ok("📯 📯"));
    }
}
