//! The dialogs, as 98-style windows: `Modal` and `modals()`, which shows the open one; the chrome
//! they share (frame, title bar, buttons row, fields, notes, color swatches);
//! and the small ones inline (new channel, confirmations, ping, add a place,
//! choose a password, the emoji upload). The bigger ones have files of their own:
//! `welcome` (Sign On, the waiting room), `settings`, `people`, `vault`,
//! `update`, `viewer` and `emoji`.

use super::{Act, App, primary_button, theme, theme::Bevel, theme::Btn, theme::Pal};
use crate::config::{BITRATES, COLORS, Config, InputMode, ThemePref};
use crate::core::{Auth, Cmd, HearState, Model, Status, UpdateState};
use eframe::egui::{self, Color32, Rect, pos2, vec2};
use parking_lot::Mutex;
use std::sync::Arc;

mod welcome;
mod settings;
mod people;
mod vault;
mod update;
mod viewer;
mod emoji;
mod share;

pub use welcome::*;
pub use settings::*;
use people::*;
pub use people::{Ask, person_controls};
pub use vault::*;
use update::*;
pub use viewer::*;
pub use emoji::*;
pub use share::*;

pub enum Modal {
    Settings,
    NewChannel { name: String, voice: bool },
    ConfirmDelete { id: String },
    ConfirmArchive { id: String, name: String },
    EmojiUpload(EmojiUpload),
    /// "Choose a password" (the once-per-device prompt) / "Set a password"
    /// (an account with no password at all).
    Password(PasswordForm),
    /// A picture full size inside the app (encrypted ones only open here).
    Viewer(Viewer),
    /// Draw a doodle reaction.
    Doodle(super::doodle::DoodleBox),
    /// Add a soundboard clip or your join chime.
    SoundUpload(super::sounds::Upload),
    /// The quick switcher (Ctrl/⌘+K).
    Switcher(super::extras::Switcher),
    /// Plan a hang.
    PlanHang(super::extras::PlanHang),
    Ping { note: String },
    Update,
    AddLink { name: String, url: String },
    /// Everyone, A–Z (no online/offline, on purpose); `open`: whose controls
    /// are showing; `ask`: asking "really reset their password / ban them?".
    People { open: Option<String>, ask: Option<(String, people::Ask)> },
    /// The Personal space passcode: set up, unlock, change, start over.
    Vault(VaultDialog),
    /// Share Screen (or, while sharing, Share something else).
    Share(ShareDialog),
}

/// Dialog windows: face fill; [`window`] draws the title bar, body and bevel.
pub fn dialog_frame(p: &Pal) -> egui::Frame {
    egui::Frame::new().fill(p.face).inner_margin(3)
}

/// The inside of a 98 dialog: title bar (with a × box when `close` is given),
/// a padded body, and the window bevel around it all.
pub fn window<R>(ui: &mut egui::Ui, p: &Pal, title: &str, icon: Option<&str>, width: f32, close: Option<&mut bool>, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.set_width(width);
    let t = theme::title_bar(ui, p, icon, title);
    if let Some(keep) = close {
        let b = Rect::from_min_size(pos2(t.right() - 20.0, t.top() + 3.0), vec2(18.0, 16.0));
        if theme::caption_button(ui, b, "x", "Close", false, p).clicked() {
            *keep = false;
        }
    }
    let r = egui::Frame::new()
        .inner_margin(egui::Margin { left: 10, right: 10, top: 10, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(width - 20.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            add(ui)
        })
        .inner;
    theme::bevel(ui.painter(), ui.min_rect().expand(3.0), p, Bevel::Window);
    r
}

/// Buttons along the bottom-right of a dialog (add them right to left).
pub fn actions(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(8.0);
    // One row tall (a plain right-to-left layout would centre the buttons in all
    // the height left on screen, and push the dialog off the top of a short window).
    let h = 26.0;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        add(ui)
    });
}

/// The colour choices: sunken squares, the chosen one ringed.
fn swatches(ui: &mut egui::Ui, selected: &str, p: &Pal) -> Option<String> {
    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for c in COLORS {
            let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
            theme::panel(ui.painter(), r, p, theme::hex_color(c), Bevel::Sunken);
            if c.eq_ignore_ascii_case(selected) {
                theme::outline(ui.painter(), r.expand(2.0), p.text);
            }
            if theme::tip(resp, c).clicked() {
                picked = Some(c.to_string());
            }
        }
    });
    picked
}

pub fn field_label(ui: &mut egui::Ui, text: &str, p: &Pal) {
    ui.add_space(4.0);
    ui.label(theme::text(text, p.text));
}

pub fn note(ui: &mut egui::Ui, text: impl Into<String>, p: &Pal) {
    ui.label(egui::RichText::new(text).font(theme::font(theme::small())).color(p.gray));
}

/// New passwords: 8–128 characters (PROTOCOL.md › Identity; the server also
/// turns away guessable ones, in its own words).
const PASSWORD_MIN: usize = 8;
const PASSWORD_MAX: usize = 128;
/// Under every new password.
const PASSWORD_RULE: &str = "8 characters or more.";

/// A password field: masked unless `show`. Any characters (an old 6-digit PIN
/// still signs in here too).
fn password_field(ui: &mut egui::Ui, value: &mut String, show: bool, p: &Pal) -> egui::Response {
    theme::field(ui, egui::TextEdit::singleline(value).password(!show).char_limit(PASSWORD_MAX).desired_width(200.0), p)
}

/// Long enough for a new password (the server judges the rest).
fn new_password_ok(value: &str) -> bool {
    (PASSWORD_MIN..=PASSWORD_MAX).contains(&value.chars().count())
}

/// Choosing or changing your password (Settings › Account, and the "Choose
/// a password" dialog): what's typed, Show, and the server's answer.
#[derive(Default)]
pub struct PasswordForm {
    current: String,
    new: String,
    show: bool,
    /// Sent; waiting for `password_seq` to move past this.
    waiting: Option<u64>,
    /// What the server said ("That password is too easy to guess.").
    error: Option<String>,
}

impl PasswordForm {
    /// Did the server answer? `Some(true)`: saved (the fields are emptied);
    /// `Some(false)`: turned away, its words in `error`.
    fn poll(&mut self, m: &Model) -> Option<bool> {
        let sent = self.waiting?;
        if m.password_seq == sent {
            return None;
        }
        self.waiting = None;
        match &m.password_error {
            Some(e) => {
                self.error = Some(e.clone());
                Some(false)
            }
            None => {
                self.current.clear();
                self.new.clear();
                self.error = None;
                Some(true)
            }
        }
    }

    /// Everything's there to send.
    fn ready(&self, m: &Model) -> bool {
        self.waiting.is_none() && new_password_ok(&self.new) && (m.needs_pin || !self.current.is_empty())
    }

    fn submit(&mut self, m: &Model, acts: &mut Vec<Act>) {
        let current = (!m.needs_pin).then(|| self.current.clone());
        acts.push(Act::Cmd(Cmd::SetPassword { password: self.new.clone(), current }));
        self.waiting = Some(m.password_seq);
        self.error = None;
    }

    /// The fields: Current password ("Current password or PIN" while on a
    /// PIN; none for an account with no password), New password with its rule
    /// and Show, then the server's answer. Returns whether Enter was pressed
    /// in one; `focus`: put the cursor in the first empty one.
    fn fields(&mut self, ui: &mut egui::Ui, m: &Model, p: &Pal, focus: bool) -> bool {
        let mut enter = false;
        let mut focus = focus && ui.memory(|mem| mem.focused().is_none());
        if !m.needs_pin {
            field_label(ui, if m.secret_kind == "pin" { "Current password or PIN:" } else { "Current password:" }, p);
            let r = password_field(ui, &mut self.current, self.show, p);
            enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if focus && self.current.is_empty() {
                r.request_focus();
                focus = false;
            }
        }
        field_label(ui, "New password:", p);
        ui.horizontal(|ui| {
            let r = password_field(ui, &mut self.new, self.show, p);
            enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if focus && self.new.is_empty() {
                r.request_focus();
            }
            ui.add_space(6.0);
            theme::checkbox(ui, &mut self.show, "Show", p);
        });
        note(ui, PASSWORD_RULE, p);
        // (The server signs out every other device of yours, OBS links too, when the password changes.)
        note(ui, "Your other devices will be signed out.", p);
        if let Some(e) = &self.error {
            ui.label(theme::text(e, p.danger));
        }
        enter
    }
}

fn text_field(ui: &mut egui::Ui, value: &mut String, limit: usize, hint: &str, p: &Pal) -> egui::Response {
    theme::field(ui, egui::TextEdit::singleline(value).char_limit(limit).hint_text(hint).desired_width(f32::INFINITY), p)
}

pub fn modals(app: &mut App, ctx: &egui::Context, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    if app.welcome.forced {
        return;
    }
    if let Some((name, password)) = &m.pin_reset {
        // Dialogs float over the desktop: nothing behind them is dimmed (egui dims by default).
        egui::Modal::new(egui::Id::new("pin-reset")).frame(dialog_frame(p)).backdrop_color(Color32::TRANSPARENT).show(ctx, |ui| {
            window(ui, p, "Temporary password", None, 340.0, None, |ui| {
                // The web's words: "Cleo's temporary password: maple-river-stone-42. Send it to Cleo; …"
                let plain = || egui::TextFormat { font_id: theme::font(theme::ui()), color: p.text, ..Default::default() };
                let mut job = egui::text::LayoutJob::default();
                job.append(&format!("{name}'s temporary password: "), 0.0, plain());
                job.append(password, 0.0, egui::TextFormat { font_id: theme::mono(theme::ui() + 1.0), color: p.text, background: p.window, ..Default::default() });
                job.append(&format!(". Send it to {name}; they'll choose their own after signing in."), 0.0, plain());
                job.wrap.max_width = ui.available_width();
                theme::job_label(ui, job);
                actions(ui, |ui| {
                    if primary_button(ui, "Done", p, true).clicked() {
                        acts.push(Act::ClearPinReset);
                    }
                    if theme::button(ui, "Copy", Btn::Normal, true, p).clicked() {
                        acts.push(Act::Copy(password.clone(), "Password copied."));
                    }
                });
            });
        });
        return;
    }
    let Some(modal) = app.modal.take() else { return };
    let mut keep = true;
    let mut modal = modal;
    let resp = egui::Modal::new(egui::Id::new("modal")).frame(dialog_frame(p)).backdrop_color(Color32::TRANSPARENT).show(ctx, |ui| match &mut modal {
        Modal::Settings => settings(app, ui, m, p, acts, &mut keep),
        Modal::Share(d) => share(ui, m, p, acts, d, &mut keep),
        Modal::People { open, ask } => people(app, ui, m, p, acts, open, ask, &mut keep),
        Modal::Vault(d) => vault_dialog(ui, m, p, acts, d, &mut keep),
        Modal::Update => update(ui, m, p, acts, &mut keep),
        Modal::NewChannel { name, voice } => {
            let mut k = true;
            window(ui, p, "New channel", None, 320.0, Some(&mut k), |ui| {
                ui.label(theme::text("Kind:", p.text));
                theme::radio_row(ui, voice, &[(false, "Text"), (true, "Voice")], p);
                field_label(ui, "Name:", p);
                let r = text_field(ui, name, 32, if *voice { "Hangout" } else { "new-channel" }, p);
                r.request_focus();
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let ok = !name.trim().is_empty();
                actions(ui, |ui| {
                    if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                        keep = false;
                    }
                    if (primary_button(ui, "Create", p, ok).clicked() || enter) && ok {
                        acts.push(Act::Cmd(Cmd::CreateChannel { name: name.trim().to_string(), kind: if *voice { "voice" } else { "text" }.into() }));
                        keep = false;
                    }
                });
            });
            keep &= k;
        }
        Modal::ConfirmDelete { id } => {
            let mut k = true;
            window(ui, p, "Delete Message", None, 300.0, Some(&mut k), |ui| {
                ui.label(theme::text("Delete this message? It's erased for good, for everyone.", p.text));
                note(ui, "Tip: shift-click the bin to skip this question.", p);
                actions(ui, |ui| {
                    if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                        keep = false;
                    }
                    if theme::button(ui, "Delete", Btn::Default, true, p).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        acts.push(Act::Cmd(Cmd::Delete(id.clone())));
                        keep = false;
                    }
                });
            });
            keep &= k;
        }
        Modal::ConfirmArchive { id, name } => {
            let mut k = true;
            window(ui, p, "Archive Channel", None, 320.0, Some(&mut k), |ui| {
                ui.label(theme::text(format!("Archive {name}?"), p.text));
                note(ui, "It disappears for everyone. Its messages are kept, and the owner can bring it back.", p);
                actions(ui, |ui| {
                    if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                        keep = false;
                    }
                    if primary_button(ui, "Archive", p, true).clicked() {
                        acts.push(Act::Cmd(Cmd::ArchiveChannel(id.clone())));
                        keep = false;
                    }
                });
            });
            keep &= k;
        }
        Modal::Ping { note: text } => {
            let mut k = true;
            window(ui, p, "Ping Everyone", Some("megaphone"), 340.0, Some(&mut k), |ui| {
                // The web's words (and the server's 140-character cap on the note).
                ui.label(theme::text("Their computers chime, and phones with pushes on buzz.", p.text));
                field_label(ui, "Add a note (optional):", p);
                let r = text_field(ui, text, 140, "hop on!", p);
                r.request_focus();
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                actions(ui, |ui| {
                    if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                        keep = false;
                    }
                    if primary_button(ui, "Ping", p, true).clicked() || enter {
                        acts.push(Act::Cmd(Cmd::Ping(text.trim().to_string())));
                        keep = false;
                    }
                });
            });
            keep &= k;
        }
        Modal::AddLink { name, url } => {
            let mut k = true;
            window(ui, p, "Add a place", Some("external"), 340.0, Some(&mut k), |ui| {
                ui.label(theme::text("It shows under Places for everyone.", p.text));
                field_label(ui, "Name:", p);
                let r = text_field(ui, name, 32, "Movie night list", p);
                if r.gained_focus() || name.is_empty() && url.is_empty() && !r.has_focus() {
                    r.request_focus();
                }
                field_label(ui, "Address:", p);
                let r2 = text_field(ui, url, 300, "https://…", p);
                let enter = (r.lost_focus() || r2.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let https = url.trim().starts_with("https://") && url.trim().len() > 8;
                if !url.trim().is_empty() && !https {
                    ui.label(theme::text("Links must start with https://", p.danger));
                }
                let ok = !name.trim().is_empty() && https;
                actions(ui, |ui| {
                    if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                        keep = false;
                    }
                    if (primary_button(ui, "Add", p, ok).clicked() || enter) && ok {
                        acts.push(Act::Cmd(Cmd::AddLink { name: name.trim().to_string(), url: url.trim().to_string() }));
                        keep = false;
                    }
                });
            });
            keep &= k;
        }
        Modal::Password(form) => {
            let mut k = true;
            // Saved: the dialog's done (the core says "Password saved…").
            if form.poll(m) == Some(true) {
                keep = false;
            }
            let title = if m.needs_pin { "Set a password" } else { "Choose a password" };
            window(ui, p, title, Some("lock"), 340.0, Some(&mut k), |ui| {
                ui.label(theme::text(
                    if m.needs_pin { "Set a password so you can sign in on your other devices as yourself." } else { "You sign in with your name and this password on your other devices." },
                    p.text,
                ));
                let enter = form.fields(ui, m, p, true);
                let ok = form.ready(m);
                actions(ui, |ui| {
                    if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                        keep = false;
                    }
                    let label = if form.waiting.is_some() { "Saving…" } else { "Save password" };
                    if (primary_button(ui, label, p, ok).clicked() || enter) && ok {
                        form.submit(m, acts);
                    }
                });
            });
            keep &= k;
        }
        Modal::Viewer(v) => viewer(app, ui, p, acts, v, &mut keep),
        Modal::Doodle(d) => super::doodle::show(ui, m, p, acts, d, &mut keep),
        Modal::SoundUpload(u) => super::sounds::show(ui, m, p, acts, u, &mut keep),
        Modal::Switcher(sw) => super::extras::switcher(app, ui, m, p, acts, sw, &mut keep),
        Modal::PlanHang(d) => super::extras::plan_hang(app, ui, m, p, acts, d, &mut keep),
        Modal::EmojiUpload(up) => {
            let mut k = true;
            window(ui, p, "Add Custom Emoji", None, 320.0, Some(&mut k), |ui| {
                let r = ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), egui::Sense::hover());
                    theme::panel(ui.painter(), r, p, p.window, Bevel::Sunken);
                    if let Some(t) = &up.preview {
                        let s = 44.0 / t.size_vec2().x.max(t.size_vec2().y);
                        ui.painter().image(t.id(), Rect::from_center_size(r.center(), t.size_vec2() * s), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    ui.vertical(|ui| {
                        ui.label(theme::text("Name:", p.text));
                        theme::field(ui, egui::TextEdit::singleline(&mut up.name).char_limit(32).desired_width(200.0), p)
                    })
                    .inner
                })
                .inner;
                // Ready to rename, and Enter adds it.
                if ui.memory(|mem| mem.focused().is_none()) {
                    r.request_focus();
                }
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let valid = (2..=32).contains(&up.name.len()) && up.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !valid {
                    note(ui, "2–32 letters, numbers or underscores.", p);
                }
                if let Some(e) = &up.error {
                    ui.label(theme::text(e, p.danger));
                }
                let ok = valid && up.error.is_none();
                actions(ui, |ui| {
                    if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                        keep = false;
                    }
                    if (primary_button(ui, "Add", p, ok).clicked() || enter) && ok {
                        acts.push(Act::Cmd(Cmd::UploadEmoji { name: up.name.clone(), bytes: std::mem::take(&mut up.bytes) }));
                        keep = false;
                    }
                });
            });
            keep &= k;
        }
    });
    if resp.should_close() && !matches!(m.update, UpdateState::Downloading { .. }) {
        keep = false;
    }
    if keep {
        app.modal = Some(modal);
    } else if matches!(modal, Modal::Settings) {
        if app.settings.as_ref().is_some_and(|s| s.mic_test) {
            acts.push(Act::Cmd(Cmd::MicTest(false)));
        }
        app.settings = None;
        app.shared.ptt.cancel_binding();
    }
    // A dialog that hands over to another (the switcher opening People…).
    if let Some(next) = app.next_modal.take() {
        app.modal = Some(next);
    }
}


#[cfg(test)]
mod password_form_tests {
    use super::*;

    fn model(kind: &str, needs: bool) -> Model {
        let mut m = Model::new(Config::default());
        m.secret_kind = kind.into();
        m.needs_pin = needs;
        m
    }

    /// No PIN rules any more: 8–128 characters of anything is a new password.
    #[test]
    fn new_passwords_are_eight_or_more() {
        assert!(!new_password_ok("582039"), "a 6-digit PIN can't be set any more");
        assert!(!new_password_ok("seven77"));
        assert!(new_password_ok("correct-horse-582039"));
        assert!(!new_password_ok("ünïcödé"), "7 characters (11 bytes): counted as characters");
        assert!(new_password_ok("ünïcödé!"));
        assert!(new_password_ok(&"x".repeat(128)));
        assert!(!new_password_ok(&"x".repeat(129)));
    }

    /// Save needs the current password (or PIN) unless there's none, and goes
    /// once; the server's answer comes back through `password_seq`.
    #[test]
    fn the_form_waits_for_the_server() {
        let mut m = model("pin", false);
        let mut f = PasswordForm { new: "correct-horse-582039".into(), ..Default::default() };
        assert!(!f.ready(&m), "the current PIN is needed to change it");
        f.current = "582039".into();
        assert!(f.ready(&m));
        let mut acts = Vec::new();
        f.submit(&m, &mut acts);
        assert!(matches!(&acts[..], [Act::Cmd(Cmd::SetPassword { password, current: Some(c) })] if password == "correct-horse-582039" && c == "582039"));
        assert!(!f.ready(&m), "not twice");
        assert_eq!(f.poll(&m), None, "no answer yet");
        // Turned away: the server's words, the fields kept.
        m.password_error = Some("That password is too easy to guess.".into());
        m.password_seq += 1;
        assert_eq!(f.poll(&m), Some(false));
        assert_eq!(f.error.as_deref(), Some("That password is too easy to guess."));
        assert_eq!(f.new, "correct-horse-582039");
        // Again, saved: the fields empty out.
        f.submit(&m, &mut acts);
        m.password_error = None;
        m.password_seq += 1;
        assert_eq!(f.poll(&m), Some(true));
        assert!(f.new.is_empty() && f.current.is_empty() && f.error.is_none());
        // An account with no password at all: no current one to give.
        let m = model("pin", true);
        let mut f = PasswordForm { new: "correct-horse-582039".into(), ..Default::default() };
        assert!(f.ready(&m));
        let mut acts = Vec::new();
        f.submit(&m, &mut acts);
        assert!(matches!(&acts[..], [Act::Cmd(Cmd::SetPassword { current: None, .. })]));
    }
}
