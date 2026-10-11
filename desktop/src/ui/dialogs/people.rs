//! The People sheet: everyone A–Z (no online/offline, on purpose), the
//! waiting room, and someone's volume and the owner/mod actions.

use super::*;

/// What People asks about someone before doing it (like the web's confirm sheets).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    ResetPin,
    Ban,
}

/// The People sheet: everyone A–Z with role badges, who's in voice, the
/// waiting room, and (click someone) their volume and the owner/mod actions.
#[allow(clippy::too_many_arguments)]
pub(super) fn people(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, open: &mut Option<String>, ask: &mut Option<(String, Ask)>, keep: &mut bool) {
    let mut k = true;
    let mut everyone: Vec<&crate::protocol::User> = m.users.values().filter(|u| u.is_member()).collect();
    everyone.sort_by_key(|u| u.name.to_lowercase());
    let i_own = m.users.get(&m.config.uid).is_some_and(|u| u.role == "owner");
    let moderator = m.am_moderator();
    let voice_of = |uid: &str| m.sessions.values().find(|s| s.uid == uid && !s.is_obs() && s.voice.is_some()).and_then(|s| s.voice.as_ref());
    // As tall as what's in it (up to most of the window).
    let warned = m.join_requests.iter().filter(|u| u.invite.as_ref().is_some_and(|i| !i.used_by.is_empty())).count();
    // (An "invited by …" line that wraps makes its card taller: about as wide as it'll get.)
    let wrapped: f32 = m.join_requests.iter().filter_map(|u| u.invited_by.as_ref()).map(|b| {
        let g = ui.painter().layout(format!("invited by {b}"), theme::font(theme::small()), Color32::PLACEHOLDER, 120.0);
        (g.size().y - g.rows.first().map_or(0.0, |r| r.rect().height())).max(0.0)
    }).sum();
    let content_h = 40.0 + m.join_requests.len() as f32 * 50.0 + wrapped + warned as f32 * 30.0 + if m.join_requests.is_empty() { 0.0 } else { 30.0 } + everyone.len() as f32 * 36.0 + if open.is_some() { 130.0 } else { 0.0 };
    let height = content_h.min((ui.ctx().content_rect().height() - 200.0).max(220.0));
    window(ui, p, "People", Some("users"), 380.0, Some(&mut k), |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
        theme::well(ui, rect, p, |ui| {
            theme::scroll(ui, egui::ScrollArea::vertical().id_salt("people").auto_shrink(false), p, |ui| {
                egui::Frame::new().inner_margin(egui::Margin { left: 4, right: 4, top: 2, bottom: 6 }).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let heading = |ui: &mut egui::Ui, text: String| {
                        ui.add_space(6.0);
                        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), egui::Sense::hover());
                        let f = theme::bold_font(theme::ui());
                        ui.painter().text(pos2(r.left() + 4.0, r.center().y), egui::Align2::LEFT_CENTER, text, f, p.text);
                        ui.add_space(2.0);
                    };
                    if !m.join_requests.is_empty() {
                        heading(ui, format!("Waiting to join ({})", m.join_requests.len()));
                        for u in &m.join_requests {
                            // A personal link someone else already used: a warning line under it (W13).
                            let warning = u.invite.as_ref().and_then(|i| i.warning());
                            let warn_w = ui.available_width() - 20.0 - 26.0 - 20.0;
                            let warn_h = warning.as_ref().map_or(0.0, |w| {
                                let size = theme::small();
                                let g = ui.painter().layout(w.clone(), theme::font(size), Color32::PLACEHOLDER, warn_w - size - 4.0);
                                g.size().y + 8.0
                            });
                            let fg = theme::INFO_TEXT;
                            // The words stop short of the buttons and wrap there
                            // ("invited by Sam (Omar)" is wide).
                            let words_w = (ui.available_width() - 38.0 - 174.0 - 4.0).max(60.0);
                            let mut name = egui::text::LayoutJob::simple_singleline(u.name.clone(), theme::bold_font(theme::ui()), fg);
                            name.wrap = egui::text::TextWrapping::truncate_at_width(words_w);
                            let name = ui.painter().layout_job(name);
                            let sub = u.invited_by.as_ref().map(|b| format!("invited by {b}")).unwrap_or_else(|| "wants to join".into());
                            let sub = ui.painter().layout(sub, theme::font(theme::small()), fg, words_w);
                            let row_h = sub.rows.first().map_or(0.0, |r| r.rect().height());
                            let extra = (sub.size().y - row_h).max(0.0);
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0 + extra + warn_h), egui::Sense::hover());
                            let card = r.shrink2(vec2(0.0, 2.0));
                            ui.painter().rect_filled(card, 0.0, theme::INFO);
                            let c = pos2(card.left() + 18.0, card.top() + 23.0);
                            super::super::avatar(ui, &app.images, &m.config.server, c, 26.0, super::super::face(Some(u)), false, p);
                            if let Some(w) = &warning {
                                let at = Rect::from_min_size(pos2(c.x - 13.0, card.top() + 44.0 + extra), vec2(card.width() - 10.0, warn_h));
                                ui.scope_builder(egui::UiBuilder::new().max_rect(at), |ui| super::super::sections::invite_warning(ui, w, true, warn_w));
                            }
                            let b2 = Rect::from_min_size(pos2(card.right() - 8.0 - 80.0, c.y - 12.0), vec2(80.0, 24.0));
                            let b1 = b2.translate(vec2(-86.0, 0.0));
                            ui.painter().galley(pos2(c.x + 20.0, c.y - 8.0 - name.size().y / 2.0), name, fg);
                            ui.painter().galley(pos2(c.x + 20.0, c.y + 8.0 - row_h / 2.0), sub, fg);
                            let light = theme::light();
                            if ui.place(b1, |ui: &mut egui::Ui| theme::button_sized(ui, "Let in", Btn::Default, true, vec2(80.0, 24.0), &light)).clicked() {
                                acts.push(Act::Cmd(Cmd::Approve { uid: u.id.clone(), approved: true }));
                            }
                            if ui.place(b2, |ui: &mut egui::Ui| theme::button_sized(ui, "Turn away", Btn::Normal, true, vec2(80.0, 24.0), &light)).clicked() {
                                acts.push(Act::Cmd(Cmd::Approve { uid: u.id.clone(), approved: false }));
                            }
                        }
                    }
                    heading(ui, format!("Everyone ({})", everyone.len()));
                    for u in &everyone {
                        let me = u.id == m.config.uid;
                        let voice = voice_of(&u.id);
                        let is_open = open.as_deref() == Some(u.id.as_str()) && !me;
                        let (rect, resp) = super::super::row(ui, 36.0, ("person", &u.id));
                        let hovered = resp.hovered() && !me;
                        let fg = if is_open { p.on_select } else { p.text };
                        if is_open {
                            ui.painter().rect_filled(rect, 0.0, p.select);
                        }
                        let c = pos2(rect.left() + 18.0, rect.center().y);
                        let speaking = m.sessions.values().any(|s| s.uid == u.id && m.speaking.contains(&s.sid));
                        super::super::avatar(ui, &app.images, &m.config.server, c, 26.0, super::super::face(Some(u)), speaking, p);
                        let mut job = egui::text::LayoutJob::default();
                        let name_fg = fg;
                        job.append(&u.name, 0.0, egui::TextFormat { font_id: theme::bold_font(theme::ui()), color: name_fg, ..Default::default() });
                        let badge_color = if is_open { p.on_select } else if u.role == "owner" { p.warn } else { p.gray };
                        let (owner, moder, you) = ("(owner)", "(mod)", "(you)");
                        match u.role.as_str() {
                            "owner" => job.append(owner, 6.0, egui::TextFormat { font_id: theme::font(theme::small()), color: badge_color, ..Default::default() }),
                            "mod" => job.append(moder, 6.0, egui::TextFormat { font_id: theme::font(theme::small()), color: badge_color, ..Default::default() }),
                            _ => {}
                        }
                        if me {
                            job.append(you, 6.0, egui::TextFormat { font_id: theme::font(theme::small()), color: if is_open { p.on_select } else { p.gray }, ..Default::default() });
                        }
                        let g = ui.painter().layout_job(job);
                        let nx = c.x + 20.0;
                        let away = u.away.as_ref().filter(|a| a.on(crate::core::now_ms()));
                        let ny = if voice.is_some() || away.is_some() { c.y - 7.0 } else { c.y };
                        let w = g.size().x;
                        ui.painter().with_clip_rect(rect).galley(pos2(nx, ny - g.size().y / 2.0), g, fg);
                        if hovered && !is_open {
                            ui.painter().hline(nx..=nx + w, (ny + 7.0).round(), egui::Stroke::new(1.0, fg));
                        }
                        let sub = if is_open { p.on_select } else { p.gray };
                        let mut sx = nx;
                        if let Some(v) = voice {
                            let ch = m.channel(&v.channel_id).map(|c| c.name.clone()).unwrap_or_default();
                            super::super::icons::paint(ui.painter(), Rect::from_center_size(pos2(sx + 5.0, c.y + 8.0), vec2(11.0, 11.0)), "volume", sub);
                            let r = ui.painter().text(pos2(sx + 13.0, c.y + 8.0), egui::Align2::LEFT_CENTER, format!("in {ch}"), theme::font(theme::small()), sub);
                            sx = r.right() + 8.0;
                        }
                        // Away (AIM style): shown here, in context, never as a dot.
                        if let Some(a) = away {
                            super::super::icons::paint(ui.painter(), Rect::from_center_size(pos2(sx + 5.0, c.y + 8.0), vec2(11.0, 11.0)), "moon", sub);
                            // The web's "· back tomorrow, 9:00 AM" (a bare time read as today).
                            let till = a.until.map(|t| format!(" · back {}", super::super::extras::until_words(t, app.h12))).unwrap_or_default();
                            let mut job = egui::text::LayoutJob::simple_singleline(format!("{}{till}", a.text), theme::font(theme::small()), sub);
                            job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - sx - 20.0).max(30.0));
                            let g = ui.painter().layout_job(job);
                            ui.painter().galley(pos2(sx + 13.0, c.y + 8.0 - g.size().y / 2.0), g, sub);
                        }
                        if hovered {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if resp.clicked() && !me {
                            *open = if is_open { None } else { Some(u.id.clone()) };
                            *ask = None;
                        }
                        if is_open {
                            person_controls(ui, m, p, acts, u, moderator, i_own, ask, 38);
                        }
                    }
                });
            });
        });
        note(ui, "Yap doesn't show who's online, on purpose: drop in and chill. Click someone for their volume.", p);
    });
    *keep &= k;
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        *keep = false;
    }
}

/// Under an opened person (and in the voice list's popup): volume, then
/// (owner/mods) mod, PIN reset, ban (those two ask first, in the web's
/// words). `indent`: from the left edge.
#[allow(clippy::too_many_arguments)]
pub fn person_controls(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, u: &crate::protocol::User, moderator: bool, owner: bool, ask: &mut Option<(String, Ask)>, indent: i8) {
    egui::Frame::new().inner_margin(egui::Margin { left: indent, right: 8, top: 4, bottom: 8 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        let mut vol = (m.config.volume_for(&u.id) * 100.0).round();
        ui.horizontal(|ui| {
            ui.label(theme::text("Volume", p.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(theme::text(format!("{}%", vol as i32), p.text)));
        });
        let w = ui.available_width();
        if theme::tip(theme::trackbar(ui, &mut vol, 0.0..=200.0, 5.0, w, p), "Past 100% boosts quiet people").changed() {
            let (uid, v) = (u.id.clone(), vol / 100.0);
            acts.push(Act::Config(Box::new(move |c| {
                c.volumes.insert(uid, v);
            })));
            acts.push(Act::Cmd(Cmd::SetVolume { uid: u.id.clone() }));
        }
        let can_ban = moderator && u.role != "owner";
        let can_promote = owner && (u.role == "member" || u.role == "mod");
        if !(can_ban || can_promote) {
            return;
        }
        if let Some(what) = ask.as_ref().filter(|(uid, _)| *uid == u.id).map(|(_, what)| *what) {
            let (question, ok) = match what {
                Ask::ResetPin => (format!("Reset {}'s PIN? They get a temporary PIN to sign in with; their old PIN or password stops working.", u.name), "Reset PIN"),
                Ask::Ban => (format!("Ban {}? They're signed out and can't come back. Their messages stay (mods can delete them).", u.name), "Ban"),
            };
            ui.label(theme::text(question, p.danger));
            ui.horizontal(|ui| {
                if theme::button(ui, ok, Btn::Default, true, p).clicked() {
                    acts.push(Act::Cmd(match what {
                        Ask::ResetPin => Cmd::ResetPin { uid: u.id.clone() },
                        Ask::Ban => Cmd::Ban { uid: u.id.clone() },
                    }));
                    *ask = None;
                }
                if theme::button(ui, "Cancel", Btn::Normal, true, p).clicked() {
                    *ask = None;
                }
            });
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if can_promote {
                let (label, role) = if u.role == "mod" { ("Remove mod", "member") } else { ("Make mod", "mod") };
                if theme::button(ui, label, Btn::Normal, true, p).clicked() {
                    acts.push(Act::Cmd(Cmd::SetRole { uid: u.id.clone(), role: role.into() }));
                }
            }
            if can_ban {
                if theme::button(ui, "Reset PIN…", Btn::Normal, true, p).clicked() {
                    *ask = Some((u.id.clone(), Ask::ResetPin));
                }
                if theme::button(ui, "Ban…", Btn::Normal, true, p).clicked() {
                    *ask = Some((u.id.clone(), Ask::Ban));
                }
            }
        });
    });
}
