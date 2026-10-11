//! "Yap Update": what's new, then download + verify + restart, on click only.

use super::*;

/// "Yap Update": what's new, then download + verify + restart, on click only.
/// A build without an update key says where new versions come from instead.
pub(super) fn update(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, keep: &mut bool) {
    if !crate::update::enabled() {
        let mut k = true;
        window(ui, p, "Yap Update", Some("download"), 360.0, Some(&mut k), |ui| {
            ui.label(egui::RichText::new(format!("You have Yap {}.", crate::update::current_version())).font(theme::bold_font(theme::ui())).color(p.text));
            note(ui, crate::update::OFF_TEXT, p);
            actions(ui, |ui| {
                if theme::button(ui, "OK", Btn::Normal, true, p).clicked() {
                    *keep = false;
                }
                if primary_button(ui, "Open the download page", p, !m.config.server.is_empty()).clicked() {
                    acts.push(Act::OpenUrl(format!("{}/download", m.config.server)));
                    *keep = false;
                }
            });
        });
        *keep &= k;
        return;
    }
    let busy = matches!(m.update, UpdateState::Downloading { .. } | UpdateState::Installed(_));
    let mut k = true;
    window(ui, p, "Yap Update", Some("download"), 360.0, if busy { None } else { Some(&mut k) }, |ui| match &m.update {
        UpdateState::Available(a) => {
            ui.label(egui::RichText::new(format!("Yap {} is ready to install.", a.version)).font(theme::bold_font(theme::ui())).color(p.text));
            // "Released Oct 7, 2026" (the manifest says 2026-10-07).
            let date = chrono::NaiveDate::parse_from_str(&a.date, "%Y-%m-%d").map(|d| d.format("%b %-d, %Y").to_string()).unwrap_or_else(|_| a.date.clone());
            note(ui, format!("You have {}.{}", crate::update::current_version(), if date.is_empty() { String::new() } else { format!(" Released {date}.") }), p);
            if !a.notes.is_empty() {
                ui.add_space(4.0);
                ui.label(theme::text("What's new:", p.text));
                // As tall as the notes wrap to (up to 160, then it scrolls).
                let wrap = ui.available_width() - 44.0; // (the well, its margins and a scroll bar)
                let lines: f32 = a.notes.iter().map(|n| ui.painter().layout(format!("• {n}"), theme::font(theme::ui()), p.text, wrap).size().y + ui.spacing().item_spacing.y).sum();
                let h = (lines + 14.0).clamp(40.0, 160.0);
                let r = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), h));
                ui.allocate_rect(r, egui::Sense::hover());
                theme::well(ui, r, p, |ui| {
                    theme::scroll(ui, egui::ScrollArea::vertical().auto_shrink(false), p, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                            for n in &a.notes {
                                ui.label(theme::text(format!("• {n}"), p.text));
                            }
                        });
                    });
                });
            }
            ui.add_space(4.0);
            note(ui, "Yap downloads it, checks it was signed by Yap's release key, then restarts.", p);
            actions(ui, |ui| {
                if theme::button(ui, "Later", Btn::Normal, true, p).clicked() {
                    *keep = false;
                }
                if primary_button(ui, "Install and restart", p, true).clicked() {
                    acts.push(Act::Cmd(Cmd::InstallUpdate));
                }
            });
        }
        UpdateState::Downloading { version, done, total } => {
            ui.label(theme::text(format!("Downloading Yap {version}…"), p.text));
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), egui::Sense::hover());
            let frac = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
            theme::meter(ui, r, frac, p.meter, p);
            note(ui, format!("{:.1} of {:.1} MB", *done as f64 / 1e6, *total as f64 / 1e6), p);
        }
        UpdateState::Installed(v) => {
            ui.label(theme::text(format!("Yap {v} is installed. Restarting…"), p.text));
        }
        UpdateState::Failed(e) => {
            ui.label(egui::RichText::new("The update didn't install.").font(theme::bold_font(theme::ui())).color(p.text));
            ui.label(theme::text(e, p.danger));
            note(ui, "Nothing was changed. You can try again.", p);
            actions(ui, |ui| {
                if theme::button(ui, "Close", Btn::Normal, true, p).clicked() {
                    *keep = false;
                }
                if primary_button(ui, "Try again", p, true).clicked() {
                    acts.push(Act::Cmd(Cmd::CheckUpdate { manual: true }));
                }
            });
        }
        UpdateState::Checking => {
            ui.label(theme::text("Checking for updates…", p.text));
        }
        UpdateState::Idle => {
            // Never checked yet (still connecting): check now rather than say "up to date".
            let asked = egui::Id::new("update-idle-check");
            if !ui.data(|d| d.get_temp::<bool>(asked).unwrap_or(false)) {
                ui.data_mut(|d| d.insert_temp(asked, true));
                acts.push(Act::Cmd(Cmd::CheckUpdate { manual: true }));
            }
            ui.label(theme::text("Checking for updates…", p.text));
        }
        UpdateState::UpToDate => {
            ui.label(theme::text(format!("You're up to date (Yap {}).", crate::update::current_version()), p.text));
            actions(ui, |ui| {
                if primary_button(ui, "OK", p, true).clicked() {
                    *keep = false;
                }
            });
        }
    });
    *keep &= k;
}
