//! The UI size: Settings › Look › Size (Auto by default) and Cmd/Ctrl + − 0;
//! the window's size and minimum follow it.

use super::*;

/// Size when there's nothing better to go on (and Auto's usual answer).
pub const AUTO_SIZE: f32 = 1.15;
/// Where Cmd/Ctrl + and − step to (like a browser's zoom).
pub(super) const SIZE_STEPS: [f32; 6] = [0.9, 1.0, 1.1, 1.25, 1.5, 1.75];
/// Auto size: a little bigger than 100% everywhere, and bigger still on a
/// 4K-class monitor the OS runs at 100–125% (where everything comes out tiny).
/// Worked out in `App::apply_size` (it needs a settled zoom); this reads it.
pub fn auto_size(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(egui::Id::new("yap-auto-size"))).unwrap_or(AUTO_SIZE)
}

impl App {
    /// The UI size: Settings › Look › Size (Auto by default) and Cmd/Ctrl + − 0.
    pub(super) fn apply_size(&mut self, ctx: &egui::Context) {
        let (zin, zout, zreset) = ctx.input_mut(|i| {
            let k = |key| egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key);
            let zin = i.consume_shortcut(&k(egui::Key::Plus)) | i.consume_shortcut(&k(egui::Key::Equals));
            (zin, i.consume_shortcut(&k(egui::Key::Minus)), i.consume_shortcut(&k(egui::Key::Num0)))
        });
        // The monitor's size arrives in points at the zoom of the frame before,
        // so only trust it once the zoom has held still for a couple of frames.
        let zoom = ctx.zoom_factor();
        if (zoom - self.zoom_seen).abs() > 0.0001 {
            self.zoom_seen = zoom;
            self.zoom_still = 0;
        } else {
            self.zoom_still = self.zoom_still.saturating_add(1);
        }
        let physical = ctx.input(|i| i.viewport().monitor_size.zip(i.viewport().native_pixels_per_point).map(|(m, native)| (m * zoom * native, native)));
        if self.zoom_still >= 2
            && let Some((mon, native)) = physical
        {
            let auto = if mon.x.max(mon.y) >= 3400.0 && native <= 1.25 { 1.3 } else { AUTO_SIZE };
            if self.auto != Some(auto) {
                self.auto = Some(auto);
                ctx.data_mut(|d| d.insert_temp(egui::Id::new("yap-auto-size"), auto));
            }
        }
        let size = self.shared.model.lock().config.size;
        let auto = auto_size(ctx);
        let now = size.unwrap_or(auto);
        let next = if zreset {
            Some(None)
        } else if zin {
            Some(Some(SIZE_STEPS.iter().copied().find(|s| *s > now + 0.001).unwrap_or(crate::config::SIZE_MAX)))
        } else if zout {
            Some(Some(SIZE_STEPS.iter().rev().copied().find(|s| *s < now - 0.001).unwrap_or(crate::config::SIZE_MIN)))
        } else {
            None
        };
        if let Some(next) = next
            && next != size
        {
            let mut m = self.shared.model.lock();
            m.config.size = next;
            m.config_dirty = true;
            m.toast(match next {
                Some(s) => format!("Size {}%", (s * 100.0).round()),
                None => format!("Size: Auto ({}%)", (auto * 100.0).round()),
            });
        }
        let want = next.unwrap_or(size).unwrap_or(auto);
        if (ctx.zoom_factor() - want).abs() > 0.001 {
            ctx.set_zoom_factor(want);
        }
        // Keep the window's smallest size in step with the zoom (but on screen),
        // and once the size is settled, give the bigger UI a bigger window.
        let monitor = physical.map(|(m, native)| m / (want * native));
        // (A new zoom takes effect next frame, and the size is converted with it.)
        if !self.sized
            && self.auto.is_some()
            && (zoom - want).abs() < 0.001
            && let Some(mon) = monitor
        {
            self.sized = true;
            let mut size = vec2(1120.0f32.min(mon.x * 0.85), 740.0f32.min(mon.y * 0.85));
            // Screenshot aid (debug builds): YAP_DEBUG_WINDOW=680x440 (points), e.g. the smallest window,
            // optionally +X+Y where it goes (a window that's entirely covered isn't drawn, so
            // shots taken side by side need their own spots).
            if cfg!(debug_assertions)
                && let Ok(v) = std::env::var("YAP_DEBUG_WINDOW")
            {
                let mut parts = v.split('+');
                let dims = parts.next().and_then(|d| d.split_once('x')).and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                let at: Vec<f32> = parts.filter_map(|n| n.parse().ok()).collect();
                if let Some((w, h)) = dims {
                    size = vec2(w, h);
                    // …and put it at the top left of the main screen (screenshots).
                    let (x, y) = if at.len() == 2 { (at[0], at[1]) } else { (40.0, 60.0) };
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(x, y)));
                    if at.len() == 2 {
                        // Over other apps' windows, so a spot mostly off screen still counts as shown.
                        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
                    }
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
        if (self.min_size_zoom - want).abs() > 0.001 {
            self.min_size_zoom = want;
            let mon = monitor.unwrap_or(vec2(f32::INFINITY, f32::INFINITY));
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(vec2(MIN_W.min(mon.x * 0.9), MIN_H.min(mon.y * 0.85))));
        }
    }
}
