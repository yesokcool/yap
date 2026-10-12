//! On top of the world: the radar, the room panel, the game line and the HUD.

use super::*;

/// The radar's width: smaller on a small window.
pub(super) fn radar_w(view: Rect) -> f32 {
    if view.width() < 640.0 { 160.0 } else { 220.0 }
}

/// The room panel's width: whatever the radar leaves (like the web's `calc(100% - 184px)`).
pub(super) fn panel_w(view: Rect) -> f32 {
    (view.width() - radar_w(view) - 12.0 - 24.0 - 20.0 - 8.0).clamp(130.0, 232.0)
}

/// The radar: north-up, the track round the edge, rooms (yours lit, with
/// names), walls, the ball, everyone as dots ("it" ringed red, burning orange,
/// dripping blue), me as an arrow, my view as a box.
#[allow(clippy::too_many_arguments)]
pub(super) fn radar(painter: &egui::Painter, m: &Model, st: &SpaceUi, me: (f32, f32, f32), cam: Pos2, view: Rect, it8: Option<&str>, now: f64) {
    let w = radar_w(view);
    let h = w * WORLD_H / WORLD_W;
    let r = Rect::from_min_size(pos2(view.right() - w - 12.0, view.top() + 12.0), vec2(w, h));
    let s = w / WORLD_W;
    let at = |x: f32, y: f32| pos2(r.left() + x * s, r.top() + y * s);
    painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(8, 10, 18, 199));
    let lane = (LANE * s * 0.6).max(2.0);
    painter.rect_stroke(Rect::from_min_max(at(56.0, 56.0), at(WORLD_W - 56.0, WORLD_H - 56.0)), 0.0, Stroke::new(lane, rgba(200, 110, 80, 0.55)), egui::StrokeKind::Middle);
    let my_zone = zone_of(me.0, me.1);
    for (i, room) in ROOMS.iter().enumerate() {
        let fill = if i as i32 == my_zone { Color32::from_rgba_unmultiplied(120, 190, 255, 77) } else { Color32::from_white_alpha(26) };
        painter.rect_filled(Rect::from_min_max(at(room.r.x, room.r.y), at(room.r.x + room.r.w, room.r.y + room.r.h)), 0.0, fill);
    }
    for wl in SOLID.iter() {
        painter.rect_filled(Rect::from_min_size(at(wl.x, wl.y), vec2((wl.w * s).max(1.0), (wl.h * s).max(1.0))), 0.0, Color32::from_rgba_unmultiplied(190, 220, 255, 140));
    }
    let small = w < 180.0;
    let font = egui::FontId::new(if small { 7.0 } else { 9.0 }, theme::bold());
    for (i, room) in ROOMS.iter().enumerate() {
        let c = if i as i32 == my_zone { Color32::WHITE } else { Color32::from_white_alpha(153) };
        let name = if small { room.short.unwrap_or(room.name) } else { room.name };
        painter.text(at(room.r.x + room.r.w / 2.0, room.r.y + room.r.h / 2.0), Align2::CENTER_CENTER, name, font.clone(), c);
    }
    painter.text(at(PLAZA_SPOT.0, PLAZA_SPOT.1), Align2::CENTER_CENTER, PLAZA, font, if my_zone == -1 { Color32::WHITE } else { Color32::from_white_alpha(115) });
    painter.rect_stroke(Rect::from_min_size(at(cam.x.max(0.0), cam.y.max(0.0)), vec2(view.width().min(WORLD_W) * s, view.height().min(WORLD_H) * s)), 0.0, Stroke::new(1.0, Color32::from_white_alpha(140)), egui::StrokeKind::Inside);
    let bc = at(st.ball.x + st.ball.ox, st.ball.y + st.ball.oy);
    painter.circle_filled(bc, 2.5, Color32::WHITE);
    painter.circle_stroke(bc, 2.5, Stroke::new(1.0, Color32::BLACK));
    let mine = m.sid.clone().unwrap_or_default();
    let marks = |sid: &str, x: f32, y: f32, size: f32| {
        let (fire, wet) = st.fx_of(sid, &mine);
        let color = if it8 == Some(short(sid).as_str()) {
            Some(rgb(0xff4d4f))
        } else if fire > now {
            Some(rgb(0xff9f45))
        } else if wet > now {
            Some(rgb(0x6cc4ff))
        } else {
            None
        };
        if let Some(c) = color {
            painter.circle_stroke(at(x, y), size, Stroke::new(2.0, c));
        }
    };
    for (sid, (x, y)) in &st.shown {
        let Some(u) = m.sessions.get(sid).and_then(|x| m.users.get(&x.uid)) else { continue };
        painter.circle_filled(at(*x, *y), 4.0, theme::hex_color(&u.color));
        painter.circle_stroke(at(*x, *y), 4.0, Stroke::new(1.0, Color32::BLACK));
        marks(sid, *x, *y, 7.0);
    }
    let (mx, my, md) = me;
    marks(&mine, mx, my, 8.0);
    let c = at(mx, my);
    let rot = |v: egui::Vec2| c + vec2(v.x * md.cos() - v.y * md.sin(), v.x * md.sin() + v.y * md.cos());
    painter.add(Shape::convex_polygon(vec![rot(vec2(8.0, 0.0)), rot(vec2(-5.0, 5.0)), rot(vec2(-2.0, 0.0)), rot(vec2(-5.0, -5.0))], Color32::WHITE, Stroke::NONE));
    painter.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::from_white_alpha(89)), egui::StrokeKind::Inside);
}

/// Your room (top left): its name, who's here (in the call? on fire? wet?),
/// the voice choice, a hint, and in the Gallery which wall you're looking at.
#[allow(clippy::too_many_arguments)]
pub(super) fn room_panel(st: &mut SpaceUi, ui: &mut egui::Ui, m: &Model, p: &Pal, view: Rect, me: (f32, f32), list: &[&SavedDrawing], acts: &mut Vec<Act>, now: f64) {
    let zone = zone_of(me.0, me.1);
    let mine = m.sid.clone().unwrap_or_default();
    let mut here: Vec<String> = vec![mine.clone()];
    here.extend(m.space.players.iter().filter(|(_, pp)| zone_of(pp.x, pp.y) == zone).map(|(s, _)| s.clone()));
    let away = m.space.players.len() + 1 - here.len();
    let in_call = m.voice_channel.is_some();
    let dim = Color32::from_white_alpha(170);
    egui::Area::new(ui.id().with("space-room")).order(egui::Order::Middle).fixed_pos(view.left_top() + vec2(12.0, 12.0)).constrain_to(view).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(Color32::from_rgba_unmultiplied(8, 10, 18, 199)).stroke(Stroke::new(1.0, Color32::from_white_alpha(60))).inner_margin(10).show(ui, |ui| {
            let width = panel_w(view);
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(zone_name(zone)).font(egui::FontId::new(15.0, theme::bold())).color(Color32::WHITE));
                ui.label(egui::RichText::new(format!("{} here{}", here.len(), if away > 0 { format!(" · {away} elsewhere") } else { String::new() })).font(theme::font(11.0)).color(dim));
            });
            for sid in &here {
                let u = m.sessions.get(sid).and_then(|s| m.users.get(&s.uid)).or_else(|| if *sid == mine { m.users.get(&m.config.uid) } else { None });
                let Some(u) = u else { continue };
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, theme::hex_color(&u.color));
                    let (fire, wet) = st.fx_of(sid, &mine);
                    let state = if fire > now { " 🔥" } else if wet > now { " 💧" } else { "" };
                    let name = if *sid == mine { format!("{} (you){state}", u.name) } else { format!("{}{state}", u.name) };
                    ui.label(egui::RichText::new(name).font(theme::font(12.0)).color(Color32::WHITE));
                    // In the call (and muted?).
                    let call = m.sessions.get(sid).and_then(|s| s.voice.as_ref());
                    if let Some(v) = call {
                        let (r, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
                        super::super::icons::paint(ui.painter(), r, if v.muted { "mic-off" } else { "volume" }, if v.muted { rgb(0xff7a7a) } else { rgb(0x8aff80) });
                    }
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Voice").font(theme::font(11.0)).color(dim));
                let labels = [(VoiceMode::Room, "This room only"), (VoiceMode::Near, "Nearby is louder"), (VoiceMode::Everyone, "Everyone")];
                let now = labels.iter().find(|(v, _)| *v == st.voice).map(|(_, l)| *l).unwrap_or("This room only");
                theme::combo(ui, "space-voice", now, (width - 44.0).min(170.0), p, |ui| {
                    for (v, l) in labels {
                        if theme::menu_item(ui, l, v == st.voice, p).clicked() {
                            st.voice = v;
                            if v == VoiceMode::Everyone {
                                st.pan.clear();
                                let all: Vec<(String, f32, f32)> = st.vol.drain().map(|(k, _)| (k, 1.0, 0.0)).collect();
                                acts.push(Act::Cmd(Cmd::Proximity(all)));
                            }
                        }
                    }
                });
            });
            let hint = if in_call {
                match st.voice {
                    VoiceMode::Room if zone >= 0 => format!("You hear everyone in the {}, and nobody outside it.", zone_name(zone)),
                    VoiceMode::Room => "You hear everyone on the Plaza, and nobody in the rooms.".to_string(),
                    VoiceMode::Near => "You hear everyone in the call; nearby is louder.".to_string(),
                    VoiceMode::Everyone => "You hear everyone in the call at full volume.".to_string(),
                }
            } else {
                "Join a voice channel to talk with people here.".to_string()
            };
            ui.add(egui::Label::new(egui::RichText::new(hint).font(theme::font(11.0)).color(dim)).wrap());
            // In the Gallery: which wall, and buttons to turn to the next one.
            if zone == GALLERY_ZONE {
                ui.add_space(4.0);
                let n = list.len();
                let total = pages(n);
                let words = if n == 0 { "🖼 No drawings yet: save one in Draw".to_string() } else { format!("🖼 Wall {} of {total} · {n} drawing{}", st.page + 1, if n == 1 { "" } else { "s" }) };
                ui.label(egui::RichText::new(words).font(theme::font(11.0)).color(Color32::WHITE));
                if total > 1 {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if theme::tip(theme::button_sized(ui, "◀ Newer", theme::Btn::Normal, st.page > 0, vec2(80.0, 23.0), p), "Newer drawings").clicked() {
                            st.page = st.page.saturating_sub(1);
                        }
                        if theme::tip(theme::button_sized(ui, "Older ▶", theme::Btn::Normal, st.page + 1 < total, vec2(80.0, 23.0), p), "Older drawings").clicked() {
                            st.page = (st.page + 1).min(total - 1);
                        }
                    });
                }
            }
        });
    });
}

/// The game line (top centre): flashes, the tag clock and times, the lap
/// clock, the splash board.
pub(super) fn game_line(st: &SpaceUi, c: &Ctx, painter: &egui::Painter, view: Rect) {
    let mut lines: Vec<(String, bool)> = Vec::new();
    if let Some((text, until)) = &st.flash
        && c.now < *until
    {
        lines.push((text.clone(), true));
    }
    let board = |rows: &[(String, u64)]| rows.iter().take(6).map(|(n, s)| format!("{n} {s}s")).collect::<Vec<_>>().join(" · ");
    if !st.tag.over {
        let secs = ((st.tag.end_at - c.now) / 1000.0).ceil().max(0.0) as u64;
        let mine = st.tag.it.as_deref() == Some(short(&c.mine).as_str());
        let who = if mine { "You're it!".to_string() } else { format!("{} is it", st.sid_of(c, st.tag.it.as_deref()).and_then(|s| c.name(Some(&s))).unwrap_or_else(|| "Someone".into())) };
        lines.push((format!("🏃 Tag · {who} · {}:{:02}", secs / 60, secs % 60), true));
        lines.push((board(&st.tag_rows(c)), false));
        lines.push(("Touch to pass it · sitting is safe · least time as it wins".into(), false));
    } else if c.now < st.tag.result_until {
        let rows = st.tag_rows(c);
        let best: Vec<String> = rows.iter().filter(|r| Some(r.1) == rows.first().map(|f| f.1)).map(|r| r.0.clone()).collect();
        if !best.is_empty() {
            lines.push((format!("🏁 Time! {} {}: least time as it", best.join(" & "), if best.len() > 1 { "win" } else { "wins" }), true));
        }
        lines.push((board(&rows), false));
    }
    if st.race.start > 0.0 {
        let t = c.now - st.race.start;
        let clock = format!("{}:{:04.1}", (t / 60000.0).floor() as u64, (t % 60000.0) / 1000.0);
        let best = if st.race.best > 0.0 { format!(" · best {}", secs(st.race.best)) } else { String::new() };
        lines.push((format!("⏱ {clock} · corner {}/{}{best}", st.race.cp, CORNERS.len()), true));
    }
    if c.now < st.splash_board {
        let mut rows: Vec<(String, u32)> = Vec::new();
        if st.my_splashes > 0 {
            rows.push((c.m.config.name.clone(), st.my_splashes));
        }
        for (sid, n) in &st.splashes {
            if *n > 0
                && c.m.space.players.contains_key(sid)
                && let Some(name) = c.name(Some(sid))
            {
                rows.push((name, *n));
            }
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.1));
        if !rows.is_empty() {
            lines.push((format!("💦 {}", rows.iter().take(6).map(|(n, k)| format!("{n} {k}")).collect::<Vec<_>>().join(" · ")), false));
        }
    }
    if lines.is_empty() {
        return;
    }
    let font = |bold: bool| if bold { egui::FontId::new(14.0, theme::bold()) } else { theme::font(12.0) };
    let wide: f32 = lines.iter().map(|(t, b)| painter.layout_no_wrap(t.clone(), font(*b), Color32::WHITE).size().x).fold(0.0, f32::max) + 24.0;
    // No room between the room panel and the radar: the news goes under the radar (like the web).
    let (left, right) = (view.left() + 12.0 + panel_w(view) + 20.0 + 8.0, view.right() - 12.0 - radar_w(view) - 8.0);
    let narrow = view.center().x - wide / 2.0 < left || view.center().x + wide / 2.0 > right;
    let wrap = if narrow { radar_w(view) - 24.0 } else { f32::INFINITY };
    let galleys: Vec<_> = lines.into_iter().map(|(t, bold)| painter.layout(t, font(bold), Color32::WHITE, wrap)).collect();
    let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max) + 24.0;
    let h: f32 = galleys.iter().map(|g| g.size().y + 2.0).sum::<f32>() + 14.0;
    let r = if narrow {
        let rw = radar_w(view);
        Rect::from_min_size(pos2(view.right() - 12.0 - rw, view.top() + 12.0 + rw * WORLD_H / WORLD_W + 8.0), vec2(rw, h))
    } else {
        Rect::from_min_size(pos2(view.center().x - w / 2.0, view.top() + 12.0), vec2(w, h))
    };
    painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(8, 10, 18, 210));
    painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_white_alpha(60)), egui::StrokeKind::Inside);
    let mut y = r.top() + 7.0;
    for g in galleys {
        let gh = g.size().y;
        painter.galley(pos2(r.center().x - g.size().x / 2.0, y), g, Color32::WHITE);
        y += gh + 2.0;
    }
}

/// The bar under the world: character, keys, emotes, Tag, Throw.
#[allow(clippy::too_many_arguments)]
pub(super) fn hud(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, area: Rect, h: f32, acts: &mut Vec<Act>, now: f64) {
    let r = Rect::from_min_max(pos2(area.left(), area.bottom() - h), area.max);
    ui.allocate_rect(area, Sense::hover());
    ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink2(vec2(8.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let me = m.users.get(&m.config.uid);
        let look = me.and_then(|u| u.look.clone()).unwrap_or_default();
        // A small window: a compact Character button and tighter emotes, so Tag and Throw still fit.
        let compact = ui.available_width() < 110.0 + EMOTES.len() as f32 * 34.0 + 64.0 + 72.0 + 30.0;
        let (label, char_w, cell) = match (compact, look.is_empty()) {
            (true, true) => ("You".to_string(), 44.0, 24.0),
            (true, false) => (look.clone(), 36.0, 24.0),
            (false, true) => ("Character: picture".to_string(), 110.0, 28.0),
            (false, false) => (format!("Character {look}"), 110.0, 28.0),
        };
        let resp = theme::tip(theme::toggle_button(ui, &label, app.space.looks_open, vec2(char_w, 26.0), p), "Your character");
        if resp.clicked() {
            app.space.looks_open = !app.space.looks_open;
        }
        let anchor = resp.rect;
        // The key hint only when everything else still fits.
        let hint = "WASD move · Shift run · E sit · 1–6 emote · T tag · F throw";
        let hint_w = ui.painter().layout_no_wrap(hint.into(), theme::font(theme::small()), p.gray).size().x;
        let rest = EMOTES.len() as f32 * 34.0 + 64.0 + 76.0 + 30.0;
        if ui.available_width() > hint_w + rest {
            ui.label(egui::RichText::new(hint).font(theme::font(theme::small())).color(p.gray));
        }
        for (i, e) in EMOTES.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(cell, 26.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, 2.0, p.select.gamma_multiply(0.2));
            }
            ui.painter().text(r.center(), Align2::CENTER_CENTER, *e, egui::FontId::proportional(17.0), p.text);
            if theme::tip(resp, format!("{e} ({})", i + 1)).clicked() {
                acts.push(Act::Cmd(Cmd::SpaceEmote(e.to_string())));
            }
        }
        let on = !app.space.tag.over;
        let tag = theme::button_sized(ui, "🏃 Tag", theme::Btn::Normal, !on, vec2(64.0, 26.0), p);
        if theme::tip(tag, if on { "A round of tag is on" } else { "Start a round of tag (T): you're it, touch someone to pass it on" }).clicked() {
            let mine = m.sid.clone().unwrap_or_default();
            let mut c = Ctx { m, mine, now, acts };
            app.space.start_tag(&mut c);
        }
        let throw = theme::button_sized(ui, "💧 Throw", theme::Btn::Normal, true, vec2(72.0, 26.0), p);
        if theme::tip(throw, "Throw a water balloon the way you're facing (F)").clicked() {
            let mine = m.sid.clone().unwrap_or_default();
            let mut c = Ctx { m, mine, now, acts };
            app.space.throw(&mut c);
        }
        if app.space.looks_open {
            let area = egui::Area::new(ui.id().with("space-looks")).order(egui::Order::Foreground).fixed_pos(anchor.left_top() - vec2(0.0, 150.0)).show(ui.ctx(), |ui| {
                super::super::sidebar::popup_frame(p).show(ui, |ui| {
                    ui.set_width(300.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for l in LOOKS {
                            if theme::toggle_button(ui, l, look == l, vec2(32.0, 30.0), p).clicked() {
                                acts.push(Act::Cmd(Cmd::SetLook(Some(l.into()))));
                                app.space.looks_open = false;
                            }
                        }
                    });
                    ui.add_space(4.0);
                    if theme::button_sized(ui, "Use my picture", theme::Btn::Normal, true, vec2(120.0, 24.0), p).clicked() {
                        acts.push(Act::Cmd(Cmd::SetLook(None)));
                        app.space.looks_open = false;
                    }
                    theme::menu_bevel(ui, p);
                });
            });
            let pos = ui.input(|i| i.pointer.interact_pos()).unwrap_or_default();
            if ui.input(|i| i.pointer.any_pressed()) && !area.response.rect.contains(pos) && !anchor.contains(pos) {
                app.space.looks_open = false;
            }
        }
    });
}
