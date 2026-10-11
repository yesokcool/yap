//! Left window: you, the voice panel, and the channel list (text, voice with
//! who's in it, places).

use super::sections::{self, Look, RowLook};
use super::{Act, App, avatar, dialogs::Modal, face, icons, theme, theme::Bevel, theme::Pal, theme::RoundRect};
use crate::core::{Cmd, Model, Status};
use eframe::egui::{self, Align2, Color32, Rect, pos2, vec2};

pub fn sidebar(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    theme::window_bevel(ui, p);
    let title = theme::title_bar(ui, p, Some("chat"), "Yap");
    let b = Rect::from_min_size(pos2(title.right() - 24.0, title.top() + 2.0), vec2(22.0, 18.0));
    if theme::caption_button(ui, b, "link", "Copy invite link", false, p).clicked() {
        acts.push(Act::Copy(crate::config::invite_link(&m.config), "Invite link copied."));
    }
    // The connection light: only while (re)connecting — no "online" light, on purpose.
    if m.status != Status::Online {
        let led = Rect::from_center_size(pos2(b.left() - 12.0, title.center().y), vec2(10.0, 10.0)).round();
        let light = if m.status == Status::Offline { Color32::from_rgb(0xff, 0, 0) } else { p.warn };
        ui.painter().rect_filled(led, 0.0, light);
        theme::outline(ui.painter(), led.expand(1.0), p.dk);
        theme::tip(
            ui.interact(led.expand(3.0), ui.id().with("conn"), egui::Sense::hover()),
            if m.status == Status::Offline { format!("Reconnecting… {}", super::chat::offline_words(&m.status_detail)) } else { "Connecting…".to_string() },
        );
    }
    ui.add_space(3.0);

    // You first, AIM-style: your nameplate, Mute / Deafen, then the call.
    user_panel(app, ui, m, p, acts);
    voice_panel(app, ui, m, p, acts);
    ui.add_space(5.0);
    theme::etched_hline(ui.painter(), ui.available_rect_before_wrap().x_range(), ui.cursor().top(), p);
    ui.add_space(5.0);

    // Catch up: only while something is new somewhere.
    let unread = super::extras::unread_channels(m).len();
    if unread > 0 {
        let label = if unread == 1 { "Catch up · 1 channel".to_string() } else { format!("Catch up · {unread} channels") };
        let r = icon_button(ui, "inbox", p.link, &label, vec2(ui.available_width(), 24.0), false, false, p);
        if theme::tip(r.clone(), "What's new since you last looked").clicked() {
            super::extras::toggle_catch_up(app, ui.ctx(), r.rect, acts);
        }
        ui.add_space(5.0);
    }
    // The version / updates / privacy strip stays pinned under the list.
    let rect = super::sidebar_footer(app, ui, m, acts);
    let rows = RowLook::of(ui);
    let rect = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - 3.0));
    theme::well(ui, rect, p, |ui| {
        theme::scroll(ui, egui::ScrollArea::vertical().id_salt("channels").auto_shrink(false), p, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 2, right: 2, top: 0, bottom: 8 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                // Your Personal space first (only you see it).
                for c in m.channels.iter().filter(|c| c.is_personal()) {
                    let active = c.id == m.config.channel && m.view.is_none() && !super::screen::in_pane(app, m);
                    let unread = m.is_unread(c) && !active;
                    ui.add_space(4.0);
                    let (_, resp) = rows.row(ui, "personal", &c.id, "lock", &c.name, Look { active, bold: unread, ..Default::default() });
                    if theme::tip(resp, "Only you can see this: notes, links and pictures to grab on your other devices.").clicked() {
                        super::screen::leaving(app, m, acts);
                        acts.push(Act::Cmd(Cmd::OpenChannel(c.id.clone())));
                        app.focus_composer = true;
                    }
                }
                if m.channels.is_empty() {
                    // Never connected yet: nothing to list (the chat pane says why).
                    return;
                }
                if rows.heading(ui, "Text channels", Some("New text channel")) {
                    app.modal = Some(Modal::NewChannel { name: String::new(), voice: false });
                }
                for c in m.channels.iter().filter(|c| c.is_text()) {
                    let active = c.id == m.config.channel && m.view.is_none() && !super::screen::in_pane(app, m);
                    let unread = m.is_unread(c) && !active;
                    let (r, resp) = rows.row(ui, "text", &c.id, "chat", &c.name, Look { active, bold: unread, ..Default::default() });
                    if let Some(n) = m.mentions.get(&c.id).filter(|n| **n > 0) {
                        sections::badge(ui.painter(), r.right_center() - vec2(5.0, 0.0), *n as usize);
                    } else if resp.hovered() && !active {
                        // Archive (it disappears for everyone; messages are kept).
                        let b = Rect::from_min_size(pos2(r.right() - 22.0, r.top() + 1.0), vec2(20.0, 18.0));
                        if ui.place(b, |ui: &mut egui::Ui| icons::tool(ui, "archive", vec2(20.0, 18.0), p.gray, "Archive channel", false)).clicked() {
                            app.modal = Some(Modal::ConfirmArchive { id: c.id.clone(), name: c.name.clone() });
                            continue;
                        }
                    }
                    if resp.clicked() {
                        super::screen::leaving(app, m, acts);
                        acts.push(Act::Cmd(Cmd::OpenChannel(c.id.clone())));
                        app.focus_composer = true;
                    }
                }
                if rows.heading(ui, "Voice channels", Some("New voice channel")) {
                    app.modal = Some(Modal::NewChannel { name: String::new(), voice: true });
                }
                for c in m.channels.iter().filter(|c| c.is_voice()) {
                    let joined = m.voice_channel.as_deref() == Some(c.id.as_str());
                    let look = Look { bold: joined, tint: joined.then_some(p.ok), ..Default::default() };
                    let (r, resp) = rows.row(ui, "voice", &c.id, "volume", &c.name, look);
                    let resp = theme::tip(resp, if joined { "You are here" } else { "Join voice" });
                    if resp.hovered() && !joined {
                        let b = Rect::from_min_size(pos2(r.right() - 22.0, r.top() + 1.0), vec2(20.0, 18.0));
                        if ui.place(b, |ui: &mut egui::Ui| icons::tool(ui, "archive", vec2(20.0, 18.0), p.gray, "Archive channel", false)).clicked() {
                            app.modal = Some(Modal::ConfirmArchive { id: c.id.clone(), name: c.name.clone() });
                            continue;
                        }
                    }
                    if resp.clicked() && !joined {
                        acts.push(Act::Cmd(Cmd::JoinVoice(c.id.clone())));
                    }
                    sections::voice_members(app, ui, m, &rows, &c.id, joined, acts);
                }
                sections::places(app, ui, m, &rows, acts);
                sections::hangs(app, ui, m, &rows, acts);
            });
        });
    });
}

fn voice_panel(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let Some(wanted) = &m.voice_wanted else { return };
    let channel = m.channel(wanted).map(|c| c.name.clone()).unwrap_or_default();
    let connected = m.voice_channel.as_ref() == Some(wanted) && m.peer_states.values().all(|s| s == "connected");
    ui.add_space(6.0);
    let out = egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 4, top: 5, bottom: 5 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                // Our socket dropped, the call goes on (the web's "Reconnecting… voice is still on", on two lines).
                let (status, color) = if m.resuming {
                    ("Reconnecting…", p.warn)
                } else if connected {
                    ("Voice connected", p.ok)
                } else {
                    ("Connecting…", p.warn)
                };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let (r, _) = ui.allocate_exact_size(vec2(13.0, 13.0), egui::Sense::hover());
                    icons::paint(ui.painter(), r, "signal", color);
                    ui.label(egui::RichText::new(status).font(theme::bold_font(theme::ui())).color(color));
                });
                // Just the channel, like the web (the status bar
                // along the bottom has the bitrates).
                let mut details = channel.clone();
                if m.resuming {
                    details.push_str(" · voice is still on");
                }
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.label(egui::RichText::new(details).font(theme::font(theme::small())).color(p.gray));
                    // ⋯: the channel's sound quality (the web moved it out of the panel).
                    let r = icons::tool(ui, "more", vec2(18.0, 14.0), p.text, "Call settings: sound quality for everyone here", app.voice_more.is_some());
                    if r.clicked() {
                        super::toggle_voice_more(app, ui.ctx(), r.rect);
                    }
                });
                if let Some((text, tip)) = super::shared_quality(m) {
                    theme::tip(ui.label(egui::RichText::new(text).font(theme::font(theme::small())).color(p.text)), tip);
                }
                if !m.mic_ok && m.voice_channel.is_some() {
                    ui.label(egui::RichText::new("Listening only — no mic").font(theme::font(theme::small())).color(p.danger));
                }
                if m.mic_silent {
                    theme::tip(ui.label(egui::RichText::new(super::SILENT_TEXT).font(theme::font(theme::small())).color(p.danger)), super::SILENT_TIP);
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::tip(icon_button(ui, "phone-off", p.danger, "Leave", vec2(0.0, 21.0), false, false, p), "Disconnect from voice").clicked() {
                    acts.push(Act::Cmd(Cmd::LeaveVoice));
                }
            });
        });
        let w = ui.available_width() - 4.0;
        // The soundboard.
        if m.voice_channel.is_some() {
            ui.add_space(3.0);
            let r = icon_button(ui, "music", p.text, "Soundboard", vec2(w, 22.0), false, false, p);
            if theme::tip(r.clone(), "Play a short sound for everyone in this channel").clicked() {
                super::extras::toggle_soundboard(app, ui.ctx(), r.rect);
            }
            if let Some(t) = super::extras::last_sound(m) {
                ui.label(egui::RichText::new(format!("▶ {t}")).font(theme::font(theme::small())).color(p.gray));
            }
            share_controls(app, ui, m, p, acts, w);
        }
    });
    theme::bevel(ui.painter(), out.response.rect, p, Bevel::Etched);
}

/// Share screen; while sharing: Stop sharing, who's watching, the quality,
/// the sound, and Share something else.
fn share_controls(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, w: f32) {
    ui.add_space(3.0);
    let Some(share) = &m.screen.sharing else {
        let r = icon_button(ui, "monitor", p.text, "Share screen", vec2(w, 22.0), false, false, p);
        if theme::tip(r, "Share your screen with this channel").clicked() {
            super::screen::open_share_dialog(app, m, false);
        }
        return;
    };
    let r = icon_button(ui, "monitor", p.text, "Stop sharing", vec2(w, 22.0), true, false, p);
    if theme::tip(r, "Stop sharing your screen").clicked() {
        acts.push(Act::Cmd(Cmd::StopSharing));
    }
    ui.add_space(2.0);
    let line = super::screen::watchers_line(m);
    let mut job = egui::text::LayoutJob::single_section(line, egui::TextFormat { font_id: theme::font(theme::small()), color: p.text, ..Default::default() });
    job.wrap.max_width = w;
    theme::job_label(ui, job);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        theme::combo(ui, "share-quality", share.quality.label(), 78.0, p, |ui| {
            for q in crate::screen::Quality::ALL {
                if theme::tip(theme::menu_item(ui, q.label(), q == share.quality, p), q.hint()).clicked() && q != share.quality {
                    acts.push(Act::Cmd(Cmd::ShareQuality(q)));
                }
            }
        });
        if crate::screen::capture::sound_supported() {
            let mut on = share.sound;
            if theme::tip(theme::checkbox(ui, &mut on, "Share sound", p), "The computer's sound goes along (never this call's own)").changed() {
                acts.push(Act::Cmd(Cmd::ShareSound(on)));
            }
        }
    });
    ui.add_space(2.0);
    let r = theme::button_sized(ui, "Share something else", theme::Btn::Normal, true, vec2(w, 22.0), p);
    if theme::tip(r, "Another window or screen, without stopping").clicked() {
        super::screen::open_share_dialog(app, m, true);
    }
}

/// A 98 push button with an icon and a label. `on`: stays pushed in (dithered)
/// with the label in the danger colour; `big`: 18 px icon, bold label.
#[allow(clippy::too_many_arguments)]
fn icon_button(ui: &mut egui::Ui, icon: &str, icon_color: Color32, label: &str, min: egui::Vec2, on: bool, big: bool, p: &Pal) -> egui::Response {
    let font = if big { theme::bold_font(theme::ui()) } else { theme::font(theme::ui()) };
    let icon_px = if big { 18.0 } else { 13.0 };
    let gap = if big { 6.0 } else { 4.0 };
    let g = ui.painter().layout_no_wrap(label.to_string(), font, p.text);
    let content_w = icon_px + gap + g.size().x;
    let size = vec2((content_w + 16.0).max(min.x).round(), min.y);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let pressed = on || resp.is_pointer_button_down_on();
    ui.painter().rect_filled(rect, 0.0, p.face);
    if on {
        theme::dither(ui, rect, p.hilight);
    }
    theme::bevel(ui.painter(), rect, p, if pressed { Bevel::Pressed } else { Bevel::Raised });
    let off = if pressed { vec2(1.0, 1.0) } else { vec2(0.0, 0.0) };
    let x = (rect.center().x - content_w / 2.0).round();
    let fg = if on { p.danger } else { p.text };
    icons::paint(ui.painter(), Rect::from_center_size(pos2(x + icon_px / 2.0, rect.center().y) + off, vec2(icon_px, icon_px)), icon, if on { p.danger } else { icon_color });
    ui.painter().galley(pos2(x + icon_px + gap, rect.center().y - g.size().y / 2.0) + off, g, fg);
    resp
}

/// You, at the bottom of the Channels window: the whole card opens Settings
/// (on Profile); under it, big Mute and Deafen buttons that stay pushed in.
fn user_panel(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), egui::Sense::click());
    let pressed = resp.is_pointer_button_down_on();
    if pressed {
        theme::bevel(ui.painter(), rect, p, Bevel::ThinIn);
    } else if resp.hovered() {
        theme::bevel(ui.painter(), rect, p, Bevel::ThinOut);
    }
    let off = if pressed { vec2(1.0, 1.0) } else { vec2(0.0, 0.0) };
    let me_sid = m.sid.clone().unwrap_or_default();
    let speaking = m.speaking.contains(&me_sid);
    let c = pos2(rect.left() + 5.0 + 16.0, rect.center().y) + off;
    let me = m.users.get(&m.config.uid);
    let f = super::Face { name: &m.config.name, color: &m.config.color, avatar: me.and_then(|u| u.avatar.as_deref()) };
    avatar(ui, &app.images, &m.config.server, c, 32.0, f, speaking, p);
    let gear = Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y) + off, vec2(14.0, 14.0));
    icons::paint(ui.painter(), gear, "settings", if resp.hovered() { p.text } else { p.gray });
    let tx = c.x + 16.0 + 7.0;
    let clip = Rect::from_min_max(pos2(tx, rect.top()), pos2(gear.left() - 4.0, rect.bottom()));
    let painter = ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
    painter.text(pos2(tx, c.y - 7.0), Align2::LEFT_CENTER, &m.config.name, theme::bold_font(theme::ui()), p.text);
    let away = me.and_then(|u| u.away.as_ref()).filter(|a| a.on(crate::core::now_ms())).map(|a| format!("Away: {}", a.text));
    painter.text(pos2(tx, c.y + 8.0), Align2::LEFT_CENTER, away.as_deref().unwrap_or("Profile & settings"), theme::font(theme::small()), p.gray);
    if theme::tip(resp, "Your profile and settings").clicked() {
        app.settings = Some(super::dialogs::SettingsState::new(&m.config).with_tab(super::dialogs::TAB_PROFILE));
        app.modal = Some(Modal::Settings);
    }
    ui.add_space(4.0);
    let muted = m.config.muted || m.config.deafened;
    let deaf = m.config.deafened;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let w = ((ui.available_width() - 4.0) / 2.0).floor();
        let r = icon_button(ui, if muted { "mic-off" } else { "mic" }, p.text, if muted { "Muted" } else { "Mute" }, vec2(w, 36.0), muted, true, p);
        if theme::tip(r, if muted { "Unmute your mic" } else { "Mute your mic" }).clicked() {
            acts.push(Act::Cmd(Cmd::ToggleMute));
        }
        let r = icon_button(ui, if deaf { "headphones-off" } else { "headphones" }, p.text, if deaf { "Deafened" } else { "Deafen" }, vec2(w, 36.0), deaf, true, p);
        if theme::tip(r, if deaf { "Undeafen" } else { "Deafen: hear nobody, and mute" }).clicked() {
            acts.push(Act::Cmd(Cmd::ToggleDeafen));
        }
    });
}

/// Someone in a voice channel, clicked (or right-clicked): their volume
/// (0–200 %, like the web's popover), Watch screen while they share, and
/// the owner/mod actions the web shows there.
pub fn volume_popup(app: &mut App, ctx: &egui::Context, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let Some((uid, pos)) = app.volume_popup.clone() else { return };
    let Some(user) = m.users.get(&uid).cloned() else {
        app.volume_popup = None;
        return;
    };
    let (moderator, owner) = (m.am_moderator(), m.users.get(&m.config.uid).is_some_and(|u| u.role == "owner"));
    // Sharing their screen in voice (their session that is).
    let sharing = m.sessions.values().find(|s| s.uid == uid && s.voice.as_ref().is_some_and(|v| v.screen)).map(|s| s.sid.clone());
    let watching = sharing.as_ref().is_some_and(|sid| m.screen.watching.as_ref().is_some_and(|w| &w.sid == sid));
    let mut ask = app.volume_ask.take();
    let area = egui::Area::new(egui::Id::new("volume-popup")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        popup_frame(p).show(ui, |ui| {
            ui.set_width(250.0);
            let (t, _) = ui.allocate_exact_size(vec2(250.0, theme::TITLE_H), egui::Sense::hover());
            theme::paint_title(ui.painter(), t, p, None, "");
            avatar(ui, &app.images, &m.config.server, pos2(t.left() + 12.0, t.center().y), 16.0, face(Some(&user)), false, p);
            let badge = match user.role.as_str() {
                "owner" => " (owner)",
                "mod" => " (mod)",
                _ => "",
            };
            ui.painter().text(pos2(t.left() + 25.0, t.center().y), Align2::LEFT_CENTER, format!("{}{badge}", user.name), theme::bold_font(theme::ui()), p.title_text);
            if let Some(a) = user.away.as_ref().filter(|a| a.on(crate::core::now_ms())) {
                egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 6, top: 6, bottom: 0 }).show(ui, |ui| {
                    ui.label(egui::RichText::new(format!("Away: {}", a.text)).font(theme::font(theme::small())).italics().color(p.gray));
                });
            }
            if let Some(sid) = sharing.as_ref().filter(|_| !watching) {
                egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 6, top: 6, bottom: 0 }).show(ui, |ui| {
                    if theme::button(ui, "Watch screen", theme::Btn::Default, true, p).clicked() {
                        acts.push(Act::Cmd(Cmd::WatchScreen(sid.clone())));
                        app.volume_popup = None;
                    }
                });
            }
            // The same controls as People: volume (a limiter past 100 %), then mod, PIN reset, ban.
            super::dialogs::person_controls(ui, m, p, acts, &user, moderator, owner, &mut ask, 6);
            theme::menu_bevel(ui, p);
        });
    });
    app.volume_ask = ask;
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && !area.response.rect.contains(ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default());
    if clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.volume_popup = None;
    }
}

/// Floating windows (pickers, popovers): face fill; call `theme::menu_bevel` last inside.
pub fn popup_frame(p: &Pal) -> egui::Frame {
    egui::Frame::new().fill(p.face).inner_margin(3)
}
