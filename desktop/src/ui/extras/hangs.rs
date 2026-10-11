//! Hangs: planned get-togethers, the "starting now" notice, Plan a hang.

use super::*;

/// "now", "Today 8:00 PM", "Tomorrow 8:00 PM", "Tue 8:00 PM", "Oct 12, 8:00 PM".
pub fn hang_when(at: i64, h12: bool) -> String {
    let now = crate::core::now_ms();
    if at <= now {
        return "now".into();
    }
    let day = super::super::local_time(at).date_naive();
    let today = chrono::Local::now().date_naive();
    let time = super::super::fmt_time(at, h12);
    let days = (day - today).num_days();
    match days {
        0 => format!("Today {time}"),
        1 => format!("Tomorrow {time}"),
        2..=6 => format!("{} {time}", super::super::local_time(at).format("%a")),
        _ => format!("{}, {time}", super::super::local_time(at).format("%b %-d")),
    }
}

/// Upcoming hangs worth listing (ones that started under 2 h ago included).
pub fn upcoming_hangs(m: &Model) -> Vec<&crate::protocol::Hang> {
    let now = crate::core::now_ms();
    m.hangs.iter().filter(|h| h.at > now - 2 * 3600 * 1000).collect()
}

/// A hang starting now (`hang.now`, or one that began in the last half hour):
/// (id, "Game night is starting · Lounge", its voice channel to join).
pub fn hang_notice(app: &App, m: &Model) -> Option<(String, String, Option<String>)> {
    let now = crate::core::now_ms();
    let h = m
        .hang_now
        .as_ref()
        .and_then(|id| m.hangs.iter().find(|h| &h.id == id))
        .filter(|h| h.at > now - 2 * 3600 * 1000)
        .or_else(|| m.hangs.iter().find(|h| h.at <= now && h.at > now - 30 * 60 * 1000))?;
    if app.hang_dismissed.as_ref() == Some(&h.id) {
        return None;
    }
    let id = &h.id;
    let ch = h.channel_id.as_deref().and_then(|c| m.channel(c));
    let text = format!("{} is starting{}", h.title, ch.map(|c| format!(" · {}", c.name)).unwrap_or_default());
    let join = ch.filter(|c| m.voice_channel.as_deref() != Some(c.id.as_str())).map(|c| c.id.clone());
    Some((id.clone(), text, join))
}

/// Who may cancel a hang: its maker, the owner and mods.
pub fn can_cancel_hang(m: &Model, h: &crate::protocol::Hang) -> bool {
    h.uid == m.config.uid || m.am_moderator()
}

/// The tooltip for a hang in the sidebar.
pub fn hang_tip(m: &Model, h: &crate::protocol::Hang, h12: bool) -> String {
    let at = super::super::local_time(h.at).format("%A, %B %-d").to_string();
    let ch = h.channel_id.as_deref().and_then(|c| m.channel(c)).map(|c| format!("\nIn {} (click to join)", c.name)).unwrap_or_default();
    format!("{}\n{at}, {}\nPlanned by {}{ch}\nEveryone free then gets one reminder.", h.title, super::super::fmt_time(h.at, h12), m.user_name(&h.uid))
}

/// "Plan a hang": a title, a day, a half-hour and maybe a voice channel.
pub struct PlanHang {
    pub title: String,
    /// Days from today (0–20).
    pub day: usize,
    /// Half-hours after midnight (0–47).
    pub slot: usize,
    pub channel: Option<String>,
}

impl PlanHang {
    pub fn new(m: &Model) -> Self {
        use chrono::Timelike;
        // The next half hour that's at least 30 minutes away (tonight at 8 if that's later today).
        let now = chrono::Local::now();
        let next = (now.hour() as usize * 2 + now.minute() as usize / 30 + 2).min(47);
        let (day, slot) = if next < 40 && now.hour() < 18 { (0, 40) } else if now.hour() >= 23 { (1, 40) } else { (0, next) };
        let channel = m.channels.iter().find(|c| c.is_voice()).map(|c| c.id.clone());
        Self { title: String::new(), day, slot, channel }
    }

    /// When it starts (ms), in your time zone.
    pub fn at(&self) -> Option<i64> {
        use chrono::TimeZone;
        let date = chrono::Local::now().date_naive() + chrono::Duration::days(self.day as i64);
        let t = date.and_hms_opt(self.slot as u32 / 2, (self.slot as u32 % 2) * 30, 0)?;
        chrono::Local.from_local_datetime(&t).single().map(|d| d.timestamp_millis())
    }
}

pub fn plan_hang(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, d: &mut PlanHang, keep: &mut bool) {
    let mut k = true;
    dialogs::window(ui, p, "Plan a hang", Some("calendar"), 360.0, Some(&mut k), |ui| {
        ui.label(theme::text("Pick a time: everyone sees it, and when it starts the people who are free get one reminder.", p.text));
        dialogs::field_label(ui, "What:", p);
        let r = theme::field(ui, egui::TextEdit::singleline(&mut d.title).char_limit(80).hint_text("Game night").desired_width(f32::INFINITY), p);
        if d.title.is_empty() && !r.has_focus() && !r.lost_focus() {
            r.request_focus();
        }
        dialogs::field_label(ui, "When:", p);
        ui.horizontal(|ui| {
            let today = chrono::Local::now().date_naive();
            let day_label = |i: usize| {
                let date = today + chrono::Duration::days(i as i64);
                match i {
                    0 => "Today".to_string(),
                    1 => "Tomorrow".to_string(),
                    _ => date.format("%a %b %-d").to_string(),
                }
            };
            theme::combo(ui, "hang-day", &day_label(d.day), 140.0, p, |ui| {
                for i in 0..21 {
                    if theme::menu_item(ui, &day_label(i), d.day == i, p).clicked() {
                        d.day = i;
                    }
                }
            });
            let slot_label = |s: usize| {
                let t = chrono::NaiveTime::from_hms_opt(s as u32 / 2, (s as u32 % 2) * 30, 0).unwrap_or_default();
                if app.h12 { t.format("%-I:%M %p").to_string() } else { t.format("%H:%M").to_string() }
            };
            theme::combo(ui, "hang-time", &slot_label(d.slot), 110.0, p, |ui| {
                for s in 0..48 {
                    if theme::menu_item(ui, &slot_label(s), d.slot == s, p).clicked() {
                        d.slot = s;
                    }
                }
            });
        });
        dialogs::field_label(ui, "Where:", p);
        let name = |id: &Option<String>| id.as_deref().and_then(|c| m.channel(c)).map(|c| c.name.clone()).unwrap_or_else(|| "No voice channel".into());
        theme::combo(ui, "hang-where", &name(&d.channel), 260.0, p, |ui| {
            if theme::menu_item(ui, "No voice channel", d.channel.is_none(), p).clicked() {
                d.channel = None;
            }
            for c in m.channels.iter().filter(|c| c.is_voice()) {
                if theme::menu_item(ui, &c.name, d.channel.as_deref() == Some(c.id.as_str()), p).clicked() {
                    d.channel = Some(c.id.clone());
                }
            }
        });
        let at = d.at();
        let now = crate::core::now_ms();
        let soon = at.is_some_and(|t| t < now + 5 * 60 * 1000);
        let far = at.is_some_and(|t| t > now + 21 * 86_400_000);
        if soon {
            ui.label(theme::text("Pick a time at least 5 minutes from now.", p.danger));
        } else if far {
            ui.label(theme::text("Up to three weeks ahead.", p.danger));
        }
        let upcoming = m.hangs.iter().filter(|h| h.at > now).count();
        if upcoming >= 5 {
            dialogs::note(ui, "There are already 5 hangs coming up; one has to happen (or be canceled) first.", p);
        }
        let ok = !d.title.trim().is_empty() && at.is_some() && !soon && !far && upcoming < 5;
        dialogs::actions(ui, |ui| {
            if theme::button(ui, "Cancel", theme::Btn::Normal, true, p).clicked() {
                *keep = false;
            }
            if super::super::primary_button(ui, "Plan it", p, ok).clicked()
                && let Some(at) = at
            {
                acts.push(Act::Cmd(Cmd::SetHang { at, title: d.title.trim().to_string(), channel: d.channel.clone() }));
                *keep = false;
            }
        });
    });
    *keep &= k;
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        *keep = false;
    }
}
