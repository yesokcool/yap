//! Share Screen: what to share (a screen or a window, listed by the OS
//! backend in the background: it can take a moment, and on a Mac the first
//! time asks for Screen Recording permission), how sharp, and the sound.
//! "Share something else" opens the same list for a share that's running.
//!
//! macOS 14+: no list. The dialog keeps the quality and the sound, and its
//! Share… button opens Apple's own picker (a window, an app or a whole
//! screen), so macOS 15+ doesn't keep asking whether Yap may record the screen.

use super::*;
use crate::screen::Quality;
use crate::screen::capture::{self, Source, SourceKind};

type SourceList = Arc<Mutex<Option<Result<Vec<Source>, String>>>>;

pub struct ShareDialog {
    /// Switching what a running share shows (no quality or sound here).
    pub switching: bool,
    /// What's shared is picked in Apple's picker after this dialog (macOS 14+).
    system_picker: bool,
    list: SourceList,
    picked: Option<String>,
    quality: Quality,
    sound: bool,
}

impl ShareDialog {
    pub fn new(config: &Config, switching: bool) -> Self {
        Self::make(config, switching, capture::system_picker())
    }

    /// Yap's own list even where Apple's picker is used (it couldn't open).
    pub fn with_list(config: &Config, switching: bool) -> Self {
        Self::make(config, switching, false)
    }

    fn make(config: &Config, switching: bool, system_picker: bool) -> Self {
        let list: SourceList = Arc::new(Mutex::new(None));
        // (Never listed with Apple's picker: listing is what makes macOS ask again and again.)
        if !system_picker {
            let out = list.clone();
            let _ = std::thread::Builder::new().name("yap-screen-sources".into()).spawn(move || {
                *out.lock() = Some(capture::sources());
            });
        }
        Self { switching, system_picker, list, picked: None, quality: config.share_quality, sound: config.share_sound }
    }
}

pub(super) fn share(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, d: &mut ShareDialog, keep: &mut bool) {
    let mut k = true;
    let title = if d.switching { "Share something else" } else { "Share Screen" };
    window(ui, p, title, Some("monitor"), 440.0, Some(&mut k), |ui| {
        if d.system_picker {
            return system_picker(ui, m, p, acts, d, keep);
        }
        let list = d.list.lock().clone();
        ui.label(theme::text("Share", p.text));
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 170.0), egui::Sense::hover());
        let mut double = false;
        theme::well(ui, rect, p, |ui| match &list {
            None => {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(theme::text("Looking for screens and windows…", p.gray));
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
            }
            Some(Err(e)) => {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(6.0);
                    ui.set_max_width(rect.width() - 16.0);
                    ui.label(theme::text(e, p.text));
                });
            }
            Some(Ok(sources)) => {
                if d.picked.is_none() {
                    d.picked = sources.first().map(|s| s.id.clone());
                }
                theme::scroll(ui, egui::ScrollArea::vertical().id_salt("share-sources").auto_shrink([false, false]), p, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for s in sources {
                        let on = d.picked.as_deref() == Some(s.id.as_str());
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), egui::Sense::click());
                        if on {
                            ui.painter().rect_filled(r, 0.0, p.select);
                        }
                        let fg = if on { p.on_select } else { p.text };
                        let icon = if s.kind == SourceKind::Screen { "monitor" } else { "external" };
                        super::super::icons::paint(ui.painter(), Rect::from_center_size(pos2(r.left() + 12.0, r.center().y), vec2(13.0, 13.0)), icon, fg);
                        let g = theme::elide(ui.painter(), s.name.clone(), theme::font(theme::ui()), fg, r.width() - 32.0);
                        ui.painter().galley(pos2(r.left() + 24.0, r.center().y - g.size().y / 2.0), g, fg);
                        if resp.clicked() {
                            d.picked = Some(s.id.clone());
                        }
                        if resp.double_clicked() {
                            d.picked = Some(s.id.clone());
                            double = true;
                        }
                    }
                });
            }
        });
        if !d.switching {
            ui.add_space(6.0);
            quality_and_sound(ui, p, d);
        }
        let picked = list.as_ref().and_then(|l| l.as_ref().ok()).and_then(|l| l.iter().find(|s| Some(&s.id) == d.picked.as_ref())).cloned();
        let mut go = double;
        actions(ui, |ui| {
            if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                *keep = false;
            }
            go |= primary_button(ui, "Share", p, picked.is_some()).clicked();
        });
        if go && let Some(source) = picked {
            if d.switching {
                acts.push(Act::Cmd(Cmd::ShareSource(source)));
            } else if m.voice_channel.is_some() {
                acts.push(Act::Cmd(Cmd::ShareScreen { source, quality: d.quality, sound: d.sound }));
            }
            *keep = false;
        }
    });
    *keep &= k;
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        *keep = false;
    }
}

/// macOS 14+: the quality and the sound here; what to share in Apple's picker next.
fn system_picker(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, d: &mut ShareDialog, keep: &mut bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::text("Next, macOS asks what to share: a window, an app or a whole screen.", p.text));
    });
    ui.add_space(6.0);
    quality_and_sound(ui, p, d);
    let mut go = false;
    actions(ui, |ui| {
        if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
            *keep = false;
        }
        go = primary_button(ui, "Share…", p, true).clicked();
    });
    if go {
        if m.voice_channel.is_some() {
            acts.push(Act::PickScreen { quality: d.quality, sound: d.sound, switching: false });
        }
        *keep = false;
    }
}

/// Smooth / Sharp / Max with what each is for, and Share sound.
fn quality_and_sound(ui: &mut egui::Ui, p: &Pal, d: &mut ShareDialog) {
    theme::group(ui, "Quality", p, |ui| {
        for q in Quality::ALL {
            ui.horizontal(|ui| {
                if theme::radio(ui, d.quality == q, q.label(), p).clicked() {
                    d.quality = q;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_space(18.0);
                ui.label(egui::RichText::new(q.hint()).font(theme::font(theme::small())).color(p.gray));
            });
        }
    });
    if capture::sound_supported() {
        theme::checkbox(ui, &mut d.sound, "Share sound", p);
        note(ui, "The computer's sound goes along (never this call's own).", p);
    } else {
        note(ui, sound_note(), p);
    }
}

/// Why there's no Share sound here.
fn sound_note() -> &'static str {
    if cfg!(target_os = "linux") {
        "Sound isn't shared on Linux: Yap can't leave its own call out of it."
    } else if cfg!(target_os = "macos") {
        "Sharing sound needs macOS 13 or newer."
    } else {
        "Sharing sound needs Windows 10 version 2004 or newer."
    }
}
