//! The extras' Settings groups: away message, sounds (blips, the soundboard,
//! your join chime), quiet hours.

use super::*;

/// Settings › Profile › Away message: what you're up to, and until when.
pub fn away_group(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, text: &mut String, until: &mut usize, h12: bool) {
    let me = m.users.get(&m.config.uid);
    let now = crate::core::now_ms();
    theme::group(ui, "Away message", p, |ui| {
        match me.and_then(|u| u.away.as_ref()).filter(|a| a.on(now)) {
            Some(a) => {
                let till = a.until.map(|u| format!(" · back {}", until_words(u, h12))).unwrap_or_default();
                ui.label(theme::text(format!("You're away: {}{till}", a.text), p.text));
                dialogs::note(ui, "It ends by itself when you send a message.", p);
                if theme::button(ui, "I'm back", theme::Btn::Normal, true, p).clicked() {
                    acts.push(Act::Cmd(Cmd::SetAway(None)));
                }
            }
            None => {
                dialogs::note(ui, "Like AIM: people who mention you get it as a quiet reply. It ends by itself.", p);
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 90.0;
                    theme::field(ui, egui::TextEdit::singleline(text).char_limit(80).hint_text("at work til 6").desired_width(w), p);
                });
                let choices = AWAY_UNTIL;
                ui.horizontal(|ui| {
                    ui.label(theme::text("Until:", p.text));
                    theme::combo(ui, "away-until", choices[(*until).min(choices.len() - 1)].0, 190.0, p, |ui| {
                        for (i, (label, _)) in choices.iter().enumerate() {
                            if theme::menu_item(ui, label, *until == i, p).clicked() {
                                *until = i;
                            }
                        }
                    });
                    if theme::button(ui, "Set", theme::Btn::Normal, !text.trim().is_empty(), p).clicked() {
                        acts.push(Act::Cmd(Cmd::SetAway(Some((text.trim().to_string(), away_until(*until))))));
                    }
                });
            }
        }
    });
}

/// Away "until" choices, like the web: (label, minutes from now; 0 = until
/// your next message, -1 = tomorrow at 9).
pub(super) const AWAY_UNTIL: [(&str, i64); 4] = [("until my next message", 0), ("for 1 hour", 60), ("for 3 hours", 180), ("until tomorrow morning", -1)];
pub(super) fn away_until(choice: usize) -> Option<i64> {
    match AWAY_UNTIL[choice.min(AWAY_UNTIL.len() - 1)].1 {
        0 => None,
        -1 => tomorrow_morning(),
        m => Some(crate::core::now_ms() + m * 60_000),
    }
}

/// Tomorrow at 9:00 here (snoozes, away messages).
pub(super) fn tomorrow_morning() -> Option<i64> {
    use chrono::TimeZone;
    let d = chrono::Local::now().date_naive() + chrono::Duration::days(1);
    d.and_hms_opt(9, 0, 0).and_then(|t| chrono::Local.from_local_datetime(&t).single()).map(|t| t.timestamp_millis())
}

/// Settings › Voice › Sounds, in the web's order: the join & mute blips,
/// your join chime, the soundboard's volume and mute.
pub fn sound_group(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    theme::group(ui, "Sounds", p, |ui| {
        let mut sounds = m.config.sounds;
        if theme::checkbox(ui, &mut sounds, "Join & mute sounds", p).changed() {
            acts.push(Act::Config(Box::new(move |c| c.sounds = sounds)));
        }
        dialogs::note(ui, "Also the chimes for @mentions and pings.", p);
        ui.add_space(4.0);
        let me = m.users.get(&m.config.uid);
        let current = me.and_then(|u| u.chime.clone()).unwrap_or_else(|| "default".into());
        let own: Option<&crate::protocol::Sound> = current.strip_prefix("s:").and_then(|id| m.sound(id)).or_else(|| m.sounds.iter().rev().find(|s| s.kind == "chime" && s.uid == m.config.uid));
        let label = |id: &str| -> String {
            match id.strip_prefix("s:") {
                Some(_) => "Your own clip".into(),
                None => crate::core::CHIMES.iter().find(|c| c.0 == id).map(|c| c.1.to_string()).unwrap_or_else(|| id.to_string()),
            }
        };
        ui.label(theme::text("Your join chime:", p.text));
        dialogs::note(ui, "What everyone hears when you join their voice channel.", p);
        ui.horizontal_wrapped(|ui| {
            theme::combo(ui, "join-chime", &label(&current), 150.0, p, |ui| {
                for (id, name, ..) in crate::core::CHIMES.iter() {
                    if theme::menu_item(ui, name, current == *id, p).clicked() && current != *id {
                        acts.push(Act::Cmd(Cmd::SetChime((*id != "default").then(|| id.to_string()))));
                        acts.push(Act::Cmd(Cmd::PreviewSound(crate::core::Preview::Chime(id.to_string()))));
                    }
                }
                if let Some(s) = own {
                    let id = format!("s:{}", s.id);
                    if theme::menu_item(ui, "Your own clip", current == id, p).clicked() && current != id {
                        acts.push(Act::Cmd(Cmd::SetChime(Some(id))));
                    }
                }
            });
            if theme::button_sized(ui, "▶ Play", theme::Btn::Normal, current != "none", vec2(60.0, 23.0), p).clicked() {
                let what = match current.strip_prefix("s:") {
                    Some(id) => crate::core::Preview::Sound(id.to_string()),
                    None => crate::core::Preview::Chime(current.clone()),
                };
                acts.push(Act::Cmd(Cmd::PreviewSound(what)));
            }
            if theme::button_sized(ui, "Use my own clip…", theme::Btn::Normal, m.status == crate::core::Status::Online, vec2(110.0, 23.0), p).clicked() {
                acts.push(Act::PickFile(super::super::PickFor::Sound { chime: true }));
            }
        });
        dialogs::note(ui, "Your own clip: up to 2 seconds; loud ones are turned down.", p);
        ui.add_space(4.0);
        let mut vol = (m.config.sound_volume * 100.0).round();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(theme::text("Soundboard volume:", p.text));
            dialogs::note(ui, "never louder than each clip's safe level", p);
        });
        ui.horizontal(|ui| {
            if theme::tip(theme::trackbar(ui, &mut vol, 0.0..=100.0, 5.0, 170.0, p), "Never louder than a clip was measured at; a limiter keeps it calm").changed() {
                let v = vol / 100.0;
                acts.push(Act::Config(Box::new(move |c| c.sound_volume = v)));
            }
            ui.label(theme::text(format!("{}%", vol as i32), p.text));
        });
        let mut mute = m.config.sound_mute;
        if theme::checkbox(ui, &mut mute, "Mute the soundboard", p).changed() {
            acts.push(Act::Config(Box::new(move |c| c.sound_mute = mute)));
            if mute {
                acts.push(Act::StopSounds);
            }
        }
        dialogs::note(ui, "At most two sounds play at once; nothing while you're deafened, and nothing from people you've turned all the way down.", p);
    });
}

/// Settings › Notifications › Quiet hours (pushes, and this app's own notifications).
pub fn quiet_group(ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, h12: bool) {
    theme::group(ui, "Quiet hours", p, |ui| {
        let tz = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
        let q = m.quiet.clone();
        let daily = q.as_ref().is_some_and(|q| q.from != q.to);
        let mut on = daily;
        let mut next = q.clone().unwrap_or(crate::protocol::QuietHours { from: 23 * 60, to: 8 * 60, tz: tz.clone(), mentions: true, until: None });
        if !daily {
            next.from = 0;
            next.to = 0;
        }
        let mut changed = false;
        if theme::checkbox(ui, &mut on, "Hold notifications every night", p).changed() {
            // Turning it on starts with 11 PM → 8 AM (any snooze stays).
            (next.from, next.to) = if on { (23 * 60, 8 * 60) } else { (0, 0) };
            changed = true;
        }
        if on {
            let label = |mins: u32| {
                let t = chrono::NaiveTime::from_hms_opt(mins / 60, mins % 60, 0).unwrap_or_default();
                if h12 { t.format("%-I:%M %p").to_string() } else { t.format("%H:%M").to_string() }
            };
            ui.horizontal(|ui| {
                ui.label(theme::text("From", p.text));
                theme::combo(ui, "quiet-from", &label(next.from), 100.0, p, |ui| {
                    for s in 0..48u32 {
                        if theme::menu_item(ui, &label(s * 30), next.from == s * 30, p).clicked() {
                            next.from = s * 30;
                            changed = true;
                        }
                    }
                });
                ui.label(theme::text("to", p.text));
                theme::combo(ui, "quiet-to", &label(next.to), 100.0, p, |ui| {
                    for s in 0..48u32 {
                        if theme::menu_item(ui, &label(s * 30), next.to == s * 30, p).clicked() {
                            next.to = s * 30;
                            changed = true;
                        }
                    }
                });
            });
        }
        let mut mentions = next.mentions;
        if theme::checkbox(ui, &mut mentions, "Let @mentions and replies through", p).changed() {
            next.mentions = mentions;
            changed = true;
        }
        let now = crate::core::now_ms();
        match next.until.filter(|u| *u > now) {
            Some(u) => {
                ui.horizontal(|ui| {
                    ui.label(theme::text(format!("Snoozed until {}", hang_when(u, h12)), p.text));
                    if theme::button_sized(ui, "Stop snoozing", theme::Btn::Normal, true, vec2(100.0, 23.0), p).clicked() {
                        next.until = None;
                        changed = true;
                    }
                });
            }
            None => {
                if theme::button_sized(ui, "Snooze until tomorrow morning", theme::Btn::Normal, m.status == crate::core::Status::Online, vec2(190.0, 23.0), p).clicked() {
                    next.until = tomorrow_morning();
                    changed = true;
                }
            }
        }
        dialogs::note(ui, format!("Phone pushes wait, and so do this app's notifications and their sounds. Sign-in alerts always come through. Time zone: {tz}."), p);
        if changed {
            next.tz = tz;
            // No window and no snooze: quiet hours are off.
            let send = (next.from != next.to || next.until.is_some_and(|u| u > now)).then_some(next);
            acts.push(Act::Cmd(Cmd::SetQuiet(send)));
        }
    });
}
