//! Sign On (the invite link, name, password, color) and the waiting room.

use super::*;

pub(super) type CheckSlot = Arc<Mutex<Option<Result<crate::http::CheckResult, String>>>>;
pub struct Welcome {
    link: String,
    name: String,
    /// The password (signing in, an old 6-digit PIN works here too).
    password: String,
    color: String,
    /// "I'm new" (signup) vs "I have an account" (login).
    pub(super) new_account: bool,
    /// Show what's typed in the password field.
    show: bool,
    error: Option<String>,
    checking: Option<(CheckSlot, String, Option<String>)>,
    /// Switching servers from settings while online.
    pub forced: bool,
}

impl Welcome {
    pub fn from_config(c: &Config) -> Self {
        Self {
            link: if c.server.is_empty() { String::new() } else { crate::config::invite_link(c) },
            name: c.name.clone(),
            password: String::new(),
            color: c.color.clone(),
            new_account: c.name.is_empty(),
            show: false,
            error: None,
            checking: None,
            forced: false,
        }
    }

    /// "Use a different invite link…": the Sign On window over everything,
    /// with an empty link (the current server stays until a new one checks out).
    pub fn other_server(c: &Config) -> Self {
        Self { link: String::new(), forced: true, ..Self::from_config(c) }
    }

    fn auth(&self) -> Auth {
        let name = self.name.trim().chars().take(32).collect::<String>();
        if self.new_account {
            Auth::Signup { name, pin: self.password.clone(), color: self.color.clone() }
        } else {
            Auth::Login { name, pin: self.password.clone() }
        }
    }
}

/// A window centred on the desktop (Sign On, waiting room).
pub(super) fn card<R>(ui: &mut egui::Ui, p: &Pal, title: &str, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Area::new(egui::Id::new(("card", title)))
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, -12.0))
        .order(egui::Order::Middle)
        .constrain(true)
        .show(ui.ctx(), |ui| dialog_frame(p).show(ui, |ui| window(ui, p, title, Some("chat"), width, None, add)).inner)
        .inner
}

/// The Sign On header: the logo panel (fixed colours, like the web client).
pub(super) fn logo(ui: &mut egui::Ui, p: &Pal) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 76.0), egui::Sense::hover());
    let painter = ui.painter();
    let mut mesh = egui::Mesh::default();
    let (top, bottom) = (Color32::WHITE, Color32::from_rgb(0xdd, 0xe8, 0xff));
    mesh.colored_vertex(r.left_top(), top);
    mesh.colored_vertex(r.right_top(), top);
    mesh.colored_vertex(r.right_bottom(), bottom);
    mesh.colored_vertex(r.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
    theme::bevel(painter, r, p, Bevel::Sunken);
    let navy = Color32::from_rgb(0, 0, 0x80);
    let word = painter.layout_no_wrap("Yap".into(), theme::bold_font(34.0), navy);
    let sub = painter.layout_no_wrap("Text and voice with friends".into(), theme::font(theme::small()), Color32::from_rgb(0x33, 0x33, 0x33));
    let w = 44.0 + 10.0 + word.size().x.max(sub.size().x);
    let x0 = r.center().x - w / 2.0;
    let icon = Rect::from_center_size(pos2(x0 + 22.0, r.center().y), vec2(44.0, 44.0));
    super::super::icons::paint(painter, icon, "chat", navy);
    let tx = x0 + 54.0;
    let wh = word.size().y;
    painter.galley(pos2(tx, r.center().y - wh / 2.0 - 7.0), word, navy);
    painter.galley(pos2(tx + 2.0, r.center().y + wh / 2.0 - 7.0), sub, Color32::BLACK);
}

pub fn welcome(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    // The invite link is needed until we know a server that accepts our key.
    let need_link = app.welcome.forced || m.config.server.is_empty() || m.status == Status::BadKey || m.status == Status::Setup;
    // An invite-key check finished?
    if let Some((slot, server, key)) = app.welcome.checking.take() {
        let result = slot.lock().take();
        match result {
            None => {
                app.welcome.checking = Some((slot, server, key));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            Some(Ok(check)) if check.ok => {
                acts.push(Act::Config(Box::new(move |c| {
                    c.server = server;
                    c.key = key.unwrap_or_default();
                })));
                acts.push(Act::Cmd(Cmd::Reconnect));
                acts.push(Act::Cmd(Cmd::SignIn(app.welcome.auth())));
                app.welcome.password.clear();
                app.welcome.forced = false;
                app.welcome.error = None;
            }
            Some(Ok(_)) => app.welcome.error = Some("That invite key didn't work.".into()),
            Some(Err(e)) => app.welcome.error = Some(e),
        }
    }
    let busy = app.welcome.checking.is_some();
    let server_error = (m.status == Status::SignIn || m.status == Status::BadKey) && !m.status_detail.is_empty() && !app.welcome.forced;
    card(ui, p, "Sign On", 340.0, |ui| {
        logo(ui, p);
        ui.add_space(4.0);
        // The web's words.
        let new = app.welcome.new_account;
        note(
            ui,
            match (need_link, new) {
                (true, true) => "Paste your invite link, then pick a name friends will see and a password.",
                (true, false) => "Paste your invite link, then sign in with your name and password.",
                (false, true) => "Pick a name friends will see and a password. It’s how you sign in on your other devices.",
                (false, false) => "Sign in with your name and password — same you, same messages, on any device.",
            },
            p,
        );
        let mut submit = false;
        // Ready to type: the first empty field has the cursor (once nothing else does).
        let free = ui.memory(|mem| mem.focused().is_none());
        let mut focus_next = free;
        if need_link {
            field_label(ui, "Invite link:", p);
            let r = text_field(ui, &mut app.welcome.link, 400, "https://…/#key=…", p);
            submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if focus_next && app.welcome.link.trim().is_empty() {
                r.request_focus();
                focus_next = false;
            }
        }
        ui.add_space(4.0);
        let mut new_account = app.welcome.new_account;
        theme::radio_row(ui, &mut new_account, &[(true, "I'm new"), (false, "I have an account")], p);
        app.welcome.new_account = new_account;
        field_label(ui, "Screen name:", p);
        let r = text_field(ui, &mut app.welcome.name, 32, "", p);
        submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if focus_next && app.welcome.name.trim().is_empty() {
            r.request_focus();
            focus_next = false;
        }
        field_label(ui, "Password:", p);
        ui.horizontal(|ui| {
            let r = password_field(ui, &mut app.welcome.password, app.welcome.show, p);
            submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if focus_next && app.welcome.password.is_empty() {
                r.request_focus();
            }
            ui.add_space(6.0);
            theme::checkbox(ui, &mut app.welcome.show, "Show", p);
        });
        if app.welcome.new_account {
            note(ui, PASSWORD_RULE, p);
            field_label(ui, "Color:", p);
            if let Some(c) = swatches(ui, &app.welcome.color, p) {
                app.welcome.color = c;
            }
        }
        let error = app.welcome.error.clone().or_else(|| server_error.then(|| m.status_detail.clone()));
        if let Some(e) = error {
            ui.add_space(4.0);
            ui.label(theme::text(e, p.danger));
        }
        // Signing in takes whatever was set (an old PIN too); a new account's
        // password is 8 characters or more (the server judges the rest).
        let password_ok = if app.welcome.new_account { new_password_ok(&app.welcome.password) } else { !app.welcome.password.is_empty() };
        let ok = !app.welcome.name.trim().is_empty() && password_ok && (!need_link || !app.welcome.link.trim().is_empty()) && !busy;
        actions(ui, |ui| {
            if app.welcome.forced && theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                app.welcome.forced = false;
            }
            let label = match (busy, app.welcome.new_account) {
                (true, _) => "Checking…",
                (_, true) => "Sign Up",
                (_, false) => "Sign On",
            };
            submit |= primary_button(ui, label, p, ok).clicked();
        });
        if !ok {
            submit = false;
        }
        if submit {
            if need_link {
                let (server, key) = crate::config::parse_invite(&app.welcome.link);
                if server.is_empty() {
                    app.welcome.error = Some("That doesn't look like an invite link.".into());
                } else {
                    let slot: CheckSlot = Arc::new(Mutex::new(None));
                    let (s2, k2, slot2, ctx) = (server.clone(), key.clone().unwrap_or_default(), slot.clone(), ui.ctx().clone());
                    let _ = std::thread::Builder::new().name("yap-check".into()).spawn(move || {
                        *slot2.lock() = Some(crate::http::check(&s2, &k2));
                        ctx.request_repaint();
                    });
                    app.welcome.error = None;
                    app.welcome.checking = Some((slot, server, key));
                }
            } else {
                acts.push(Act::Cmd(Cmd::SignIn(app.welcome.auth())));
                app.welcome.password.clear();
                app.welcome.error = None;
            }
        }
    });
}

pub fn waiting(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let pending = m.status == Status::Pending;
    card(ui, p, "Yap", 340.0, |ui| {
        if pending {
            // Only the owner and mods let people in (PROTOCOL.md › Identity › Waiting room).
            ui.label(egui::RichText::new("Waiting for the owner or a mod to let you in.").font(theme::bold_font(theme::ui())).color(p.text));
            ui.add_space(2.0);
            // A marquee progress bar.
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), egui::Sense::hover());
            theme::panel(ui.painter(), r, p, p.window, Bevel::ThinIn);
            let inner = r.shrink(2.0);
            let t = ui.input(|i| i.time) as f32;
            let span = inner.width() + 60.0;
            let start = inner.left() - 60.0 + (t * 90.0) % span;
            for k in 0..5 {
                let x = start + k as f32 * 10.0;
                let b = Rect::from_min_max(pos2(x.max(inner.left()), inner.top()), pos2((x + 8.0).min(inner.right()), inner.bottom()));
                if b.width() > 0.0 {
                    ui.painter().rect_filled(b, 0.0, p.meter);
                }
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
            ui.add_space(4.0);
            // (The web's "Keep this tab open" is "Keep Yap open" here.)
            ui.label(theme::text(format!("Hang tight, {}. This opens by itself as soon as you're let in. Keep Yap\u{a0}open.", m.config.name), p.text));
        } else {
            ui.label(egui::RichText::new("You're not in").font(theme::bold_font(theme::ui())).color(p.text));
            ui.label(theme::text(&m.status_detail, p.danger));
        }
        actions(ui, |ui| {
            if theme::button(ui, "Use a different invite link…", Btn::Normal, true, p).clicked() {
                app.welcome = Welcome::from_config(&m.config);
                app.welcome.link.clear();
                acts.push(Act::Config(Box::new(|c| c.server.clear())));
                acts.push(Act::Cmd(Cmd::Reconnect));
            }
        });
    });
}
