//! Windows 98's status bar along the bottom: where you're connected, voice,
//! the server's Minecraft, updates.

use super::*;

/// The status bar along the bottom: connection, voice, Minecraft, updates.
pub(super) fn status_bar(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    egui::Panel::bottom("statusbar").exact_size(24.0).resizable(false).show_separator_line(false).frame(egui::Frame::new().fill(p.face)).show(ui, |ui| {
        let bar = ui.max_rect();
        ui.painter().rect_filled(Rect::from_min_max(bar.min, egui::pos2(bar.right(), bar.top() + 1.0)), 0.0, p.hilight);
        let cell_h = 18.0;
        let y = bar.center().y + 0.5;
        let cell = |ui: &egui::Ui, x: f32, w: f32| -> Rect {
            let r = Rect::from_min_size(egui::pos2(x, y - cell_h / 2.0), vec2(w, cell_h)).round();
            theme::status_cell(ui.painter(), r, p);
            r
        };
        // Connection.
        // Connected shows no light (no "online" indicator, on purpose): just where you are.
        let host = m.config.server.split("://").nth(1).unwrap_or(&m.config.server).trim_end_matches('/').to_string();
        let (light, label) = match m.status {
            Status::Online => (None, host),
            Status::Offline => (Some(Color32::from_rgb(0xff, 0, 0)), "Reconnecting…".to_string()),
            Status::Pending => (Some(p.warn), "Waiting room".to_string()),
            Status::Setup | Status::SignIn | Status::BadKey => (Some(p.warn), "Not signed in".to_string()),
            _ => (Some(p.warn), "Connecting…".to_string()),
        };
        let r = cell(ui, bar.left() + 3.0, 124.0);
        let x = r.right() + 3.0;
        let tx = match light {
            Some(light) => {
                let led = Rect::from_center_size(egui::pos2(r.left() + 10.0, r.center().y), vec2(8.0, 8.0)).round();
                ui.painter().rect_filled(led, 0.0, light);
                theme::outline(ui.painter(), led.expand(1.0), p.dk);
                r.left() + 20.0
            }
            None => r.left() + 6.0,
        };
        ui.painter().with_clip_rect(r.shrink(2.0)).text(egui::pos2(tx, r.center().y), egui::Align2::LEFT_CENTER, label, theme::font(theme::small()), p.text);
        let detail = if m.status == Status::Offline { format!("{}\n({})", super::chat::offline_words(&m.status_detail), m.status_detail) } else { m.config.server.clone() };
        theme::tip(ui.interact(r, ui.id().with("sb-conn"), egui::Sense::hover()), detail);

        // Right side, from the right edge: updates.
        let mut right = bar.right() - 3.0;
        let update_cell = match &m.update {
            UpdateState::Available(a) => Some((format!("Update to {}", a.version), true, a.notes.join("\n"))),
            UpdateState::Downloading { version, done, total } => {
                let pct = if *total > 0 { done * 100 / total } else { 0 };
                Some((format!("Downloading {version}… {pct}%"), false, String::new()))
            }
            UpdateState::Installed(v) => Some((format!("Restarting into {v}…"), false, String::new())),
            _ => None,
        };
        if let Some((label, clickable, notes)) = update_cell {
            let g = ui.painter().layout_no_wrap(label.clone(), theme::font(theme::small()), p.text);
            let w = g.size().x + if clickable { 30.0 } else { 14.0 };
            let r = Rect::from_min_size(egui::pos2(right - w, y - cell_h / 2.0), vec2(w, cell_h)).round();
            right -= w + 3.0;
            if clickable {
                let resp = ui.interact(r, ui.id().with("sb-update"), egui::Sense::click());
                let pressed = resp.is_pointer_button_down_on();
                theme::paint_button(ui, r, false, pressed, p);
                let off = if pressed { vec2(1.0, 1.0) } else { Vec2::ZERO };
                icons::paint(ui.painter(), Rect::from_center_size(egui::pos2(r.left() + 11.0, r.center().y) + off, vec2(12.0, 12.0)), "download", p.text);
                ui.painter().galley(egui::pos2(r.left() + 21.0, r.center().y - g.size().y / 2.0) + off, g, p.text);
                let tip = if notes.is_empty() { "A new version of Yap is ready to install.".to_string() } else { format!("What's new:\n{notes}") };
                if theme::tip(resp, tip).clicked() {
                    app.modal = Some(dialogs::Modal::Update);
                }
            } else {
                theme::status_cell(ui.painter(), r, p);
                ui.painter().galley(egui::pos2(r.left() + 7.0, r.center().y - g.size().y / 2.0), g, p.text);
            }
        }

        // Minecraft.
        if let Some(mc) = m.server_config.as_ref().and_then(|c| c.minecraft.as_ref()) {
            let st = m.minecraft.as_ref();
            let state = match st {
                None => "checking…".to_string(),
                Some(s) if s.online => s.players.as_ref().map(|x| format!("{}/{} playing", x.online, x.max)).unwrap_or_else(|| "online".into()),
                Some(_) => "offline".to_string(),
            };
            let label = format!("{}: {state}", mc.name);
            let g = ui.painter().layout_no_wrap(label, theme::font(theme::small()), p.text);
            let w = (g.size().x + 30.0).min((right - x - 120.0).max(60.0));
            let r = Rect::from_min_size(egui::pos2(right - w, y - cell_h / 2.0), vec2(w, cell_h)).round();
            right -= w + 3.0;
            theme::status_cell(ui.painter(), r, p);
            let online = st.is_some_and(|s| s.online);
            icons::paint(ui.painter(), Rect::from_center_size(egui::pos2(r.left() + 11.0, r.center().y), vec2(12.0, 12.0)), "box", if online { p.ok } else { p.gray });
            ui.painter().with_clip_rect(r.shrink(2.0)).galley(egui::pos2(r.left() + 21.0, r.center().y - g.size().y / 2.0), g, p.text);
            let mut tip = format!("{}\nClick to copy the address.", mc.address);
            if let Some(s) = st {
                if let Some(motd) = &s.motd {
                    tip.push_str(&format!("\n{motd}"));
                }
                if let Some(v) = &s.version {
                    tip.push_str(&format!("\nVersion {v}"));
                }
                if let Some(pl) = s.players.as_ref().filter(|x| !x.names.is_empty()) {
                    tip.push_str(&format!("\nOn now: {}", pl.names.join(", ")));
                }
            }
            let resp = ui.interact(r, ui.id().with("sb-mc"), egui::Sense::click());
            if theme::tip(resp, tip).clicked() {
                acts.push(Act::Copy(mc.address.clone(), "Minecraft address copied."));
            }
        }

        // Voice: fills what's left.
        let w = (right - x).max(40.0);
        let r = cell(ui, x, w);
        let (text, color) = match (&m.voice_wanted, &m.voice_channel) {
            (Some(want), Some(now)) if want == now => {
                let name = m.channel(now).map(|c| c.name.clone()).unwrap_or_default();
                let bitrate = m.voice_stats.as_ref().map(|s| s.bitrate).unwrap_or(m.config.bitrate);
                let mut t = format!("Voice: {name} · Opus {}k", bitrate / 1000);
                if let Some(s) = m.voice_stats.as_ref().filter(|s| s.send_kbps > 0) {
                    t.push_str(&format!(" · out {} · in {} kbps", s.send_kbps, s.recv_kbps));
                }
                if m.config.deafened {
                    t.push_str(" · deafened");
                } else if m.config.muted {
                    t.push_str(" · muted");
                }
                (t, p.text)
            }
            (Some(_), _) => ("Voice: connecting…".to_string(), p.text),
            _ => ("Not in voice".to_string(), p.gray),
        };
        ui.painter().with_clip_rect(r.shrink(2.0)).text(egui::pos2(r.left() + 6.0, r.center().y), egui::Align2::LEFT_CENTER, text, theme::font(theme::small()), color);
    });
}
