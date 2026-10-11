//! The Personal space passcode dialog: set up, unlock, change, start over.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VaultMode {
    Setup,
    Unlock,
    Change,
    /// Forgot it: a new passcode for new notes (old ones stay locked).
    Reset,
}

pub struct VaultDialog {
    pub mode: VaultMode,
    old: String,
    pass: String,
    again: String,
    error: Option<String>,
    /// Sent; waiting for `vault_seq` to move past this.
    waiting: Option<u64>,
    focused: bool,
}

impl VaultDialog {
    pub fn new(mode: VaultMode) -> Self {
        Self { mode, old: String::new(), pass: String::new(), again: String::new(), error: None, waiting: None, focused: false }
    }
}

/// The Personal space passcode dialog (web: #vault-dialog).
pub(super) fn vault_dialog(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, d: &mut VaultDialog, keep: &mut bool) {
    // The core finished what we sent: close, or show why not.
    if let Some(seq) = d.waiting
        && m.vault_seq > seq
    {
        d.waiting = None;
        match &m.vault_error {
            None => {
                *keep = false;
                return;
            }
            Some(e) => {
                d.error = Some(e.clone());
                d.pass.clear();
                d.again.clear();
            }
        }
    }
    let busy = d.waiting.is_some();
    let (title, text, ok) = match d.mode {
        VaultMode::Setup => ("Encrypt your Personal space", "Pick a passcode only you know: a short phrase works best (like \"purple tiger river 42\"), at least 8 characters. Yap can't see it and can't recover it — forget it and your encrypted notes are gone. Each of your devices asks for it once.", "Encrypt"),
        VaultMode::Unlock => ("Unlock your Personal space", "Enter your Personal space passcode. This computer remembers it until you lock it.", "Unlock"),
        VaultMode::Change => ("Change your passcode", "Your notes stay as they are; only the passcode that opens them changes. Your other devices will ask for the new one.", "Change"),
        VaultMode::Reset => ("Start over?", "Without the passcode, nobody can open your old encrypted notes — not you, not Yap. Starting over makes a new passcode for new notes; the old ones stay locked.", "Start over"),
    };
    let mut k = true;
    let mut cancel = false;
    window(ui, p, title, Some("lock"), 380.0, if busy { None } else { Some(&mut k) }, |ui| {
        ui.label(theme::text(text, p.text));
        let field = |ui: &mut egui::Ui, value: &mut String, hint: &str| theme::field(ui, egui::TextEdit::singleline(value).password(true).char_limit(256).hint_text(hint).desired_width(f32::INFINITY).interactive(!busy), p);
        let mut enter = false;
        let mut first = None;
        if d.mode == VaultMode::Change {
            field_label(ui, "Current passcode:", p);
            let r = field(ui, &mut d.old, "");
            enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            first.get_or_insert(r.id);
        }
        field_label(ui, if d.mode == VaultMode::Change { "New passcode:" } else { "Passcode:" }, p);
        let r = field(ui, &mut d.pass, if d.mode == VaultMode::Unlock { "" } else { "at least 8 characters" });
        enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        first.get_or_insert(r.id);
        if d.mode != VaultMode::Unlock {
            field_label(ui, "Same again:", p);
            let r = field(ui, &mut d.again, "");
            enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        }
        if !d.focused
            && let Some(id) = first
        {
            ui.ctx().memory_mut(|mem| mem.request_focus(id));
            d.focused = true;
        }
        if let Some(e) = &d.error {
            ui.add_space(2.0);
            ui.label(theme::text(e, p.danger));
        }
        let mut submit = false;
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if d.mode == VaultMode::Unlock {
                let f = ui.add(egui::Label::new(egui::RichText::new("Forgot it?").font(theme::font(theme::ui())).color(p.link).underline()).sense(egui::Sense::click()));
                if theme::tip(f, "Start over with a new passcode (old notes stay locked)").clicked() && !busy {
                    *d = VaultDialog::new(VaultMode::Reset);
                    return;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // [Unlock] [Cancel], like every other dialog here (right to left).
                if theme::button(ui, "Cancel", Btn::Normal, !busy, p).clicked() {
                    cancel = true;
                }
                let label = if busy { "Working…" } else { ok };
                if primary_button(ui, label, p, !busy).clicked() {
                    submit = true;
                }
            });
        });
        if (submit || enter) && !busy {
            let problem = if d.mode == VaultMode::Unlock { (d.pass.is_empty()).then_some("Enter your passcode.") } else { crate::vault::passcode_problem(&d.pass, &d.again) };
            match problem {
                Some(why) => d.error = Some(why.into()),
                None => {
                    d.error = None;
                    let op = match d.mode {
                        VaultMode::Setup => crate::core::VaultOp::Setup { pass: d.pass.clone() },
                        VaultMode::Unlock => crate::core::VaultOp::Unlock { pass: d.pass.clone() },
                        VaultMode::Change => crate::core::VaultOp::Change { old: d.old.clone(), new: d.pass.clone() },
                        VaultMode::Reset => crate::core::VaultOp::Reset { pass: d.pass.clone() },
                    };
                    acts.push(Act::Cmd(Cmd::Vault(op)));
                    d.waiting = Some(m.vault_seq);
                }
            }
        }
    });
    *keep &= k && !cancel;
}
