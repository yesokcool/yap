//! The quick switcher (Ctrl/⌘+K): channels, places, people, actions (and
//! Copy a link to what's open).

use super::*;

/// Ctrl+K (⌘K): jump to a channel, a place, a person or an action by typing.
pub struct Switcher {
    pub q: String,
    pub sel: usize,
    focused: bool,
}

impl Switcher {
    pub fn new() -> Self {
        Self { q: String::new(), sel: 0, focused: false }
    }
}

#[derive(Clone)]
pub(super) enum Hit {
    Channel(String),
    Place(usize),
    Person(String),
    Action(&'static str),
    Search,
}

/// Actions the switcher offers: (id, label, icon).
pub(super) const ACTIONS: [(&str, &str, &str); 9] = [
    ("settings", "Settings", "settings"),
    ("search", "Search messages…", "search"),
    ("catchup", "Catch up", "inbox"),
    ("shelf", "Pictures and GIFs here", "images"),
    ("pins", "Pinned messages", "pin"),
    ("ping", "Ping everyone…", "megaphone"),
    ("hang", "Plan a hang…", "calendar"),
    ("away", "Set an away message…", "moon"),
    ("soundboard", "Soundboard", "music"),
];

pub(super) fn score(name: &str, q: &str) -> Option<u8> {
    if q.is_empty() {
        return Some(3);
    }
    let n = name.to_lowercase();
    if n.starts_with(q) {
        Some(0)
    } else if n.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(q)) {
        Some(1)
    } else if n.contains(q) {
        Some(2)
    } else {
        None
    }
}

pub fn switcher(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, sw: &mut Switcher, keep: &mut bool) {
    let q = sw.q.trim().to_lowercase();
    let places = super::super::places(m);
    // (score, kind order, label, sub, icon, hit)
    let mut hits: Vec<(u8, u8, String, &'static str, &'static str, Hit)> = Vec::new();
    for c in m.channels.iter().filter(|c| c.is_chat() || c.is_voice()) {
        if let Some(s) = score(&c.name, &q) {
            let (sub, icon) = if c.is_voice() { ("voice", "volume") } else if c.is_personal() { ("only you", "lock") } else { ("channel", "chat") };
            hits.push((s, 0, c.name.clone(), sub, icon, Hit::Channel(c.id.clone())));
        }
    }
    for (i, pl) in places.iter().enumerate() {
        if let Some(s) = score(&pl.name, &q) {
            hits.push((s, 1, pl.name.clone(), "place", pl.icon, Hit::Place(i)));
        }
    }
    if !q.is_empty() {
        let mut people: Vec<&crate::protocol::User> = m.users.values().filter(|u| u.is_member()).collect();
        people.sort_by_key(|u| u.name.to_lowercase());
        for u in people {
            if let Some(s) = score(&u.name, &q) {
                hits.push((s, 2, u.name.clone(), "person", "users", Hit::Person(u.id.clone())));
            }
        }
    }
    for (id, label, icon) in ACTIONS {
        if id == "search" || id == "soundboard" && m.voice_channel.is_none() {
            continue;
        }
        if let Some(s) = score(label, &q) {
            hits.push((s.max(if q.is_empty() { 4 } else { 0 }), 3, label.to_string(), "action", icon, Hit::Action(id)));
        }
    }
    // A link to what's open (a text channel, Draw or Space), not to a screen you're watching.
    if let Some((name, _)) = super::super::here_link(m).filter(|_| !super::super::screen::in_pane(app, m)) {
        let label = format!("Copy a link to {name}");
        if let Some(s) = score(&label, &q) {
            hits.push((s.max(if q.is_empty() { 4 } else { 0 }), 3, label, "action", "link", Hit::Action("link")));
        }
    }
    hits.sort_by_key(|h| (h.0, h.1));
    hits.truncate(if q.is_empty() { 12 } else { 9 });
    // The last row is always search.
    hits.push((9, 9, if q.is_empty() { "Search messages…".to_string() } else { format!("Search messages for “{}”", sw.q.trim()) }, "search", "search", Hit::Search));
    sw.sel = sw.sel.min(hits.len().saturating_sub(1));
    let (up, down, enter) = ui.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)));
    if up {
        sw.sel = sw.sel.saturating_sub(1);
    }
    if down && sw.sel + 1 < hits.len() {
        sw.sel += 1;
    }
    let mut chosen: Option<Hit> = None;
    let mut k = true;
    let shortcut = if cfg!(target_os = "macos") { "⌘K" } else { "Ctrl+K" };
    dialogs::window(ui, p, "Go to…", Some("command"), 440.0, Some(&mut k), |ui| {
        let r = theme::field(ui, egui::TextEdit::singleline(&mut sw.q).hint_text("Channels, places, people, actions…").desired_width(f32::INFINITY), p);
        if !sw.focused {
            r.request_focus();
            sw.focused = true;
        }
        if r.changed() {
            sw.sel = 0;
        }
        ui.add_space(4.0);
        ui.spacing_mut().item_spacing.y = 1.0;
        for (i, (_, _, label, sub, icon, hit)) in hits.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            if resp.hovered() && ui.input(|i| i.pointer.delta().length() > 0.0) {
                sw.sel = i;
            }
            let on = sw.sel == i;
            if on {
                ui.painter().rect_filled(rect, 0.0, p.select);
            }
            let fg = if on { p.on_select } else { p.text };
            let subc = if on { fg.gamma_multiply(0.8) } else { theme::dim(p) };
            icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 13.0, rect.center().y), vec2(14.0, 14.0)), icon, if on { fg } else { theme::accent(p) });
            let mut job = egui::text::LayoutJob::simple_singleline(label.clone(), theme::font(theme::ui()), fg);
            job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 110.0);
            let g = ui.painter().layout_job(job);
            ui.painter().galley(pos2(rect.left() + 28.0, rect.center().y - g.size().y / 2.0), g, fg);
            ui.painter().text(pos2(rect.right() - 8.0, rect.center().y), Align2::RIGHT_CENTER, *sub, theme::small_font(theme::SMALL), subc);
            if resp.clicked() {
                chosen = Some(hit.clone());
            }
        }
        if hits.is_empty() {
            ui.label(theme::text("Nothing matches.", theme::dim(p)));
        }
        ui.add_space(4.0);
        dialogs::note(ui, format!("↑ ↓ to pick · Enter to open · Esc to close · {shortcut} anytime"), p);
    });
    if enter {
        chosen = hits.get(sw.sel).map(|h| h.5.clone());
    }
    *keep &= k;
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        *keep = false;
    }
    let Some(hit) = chosen else { return };
    *keep = false;
    match hit {
        Hit::Channel(id) => {
            let voice = m.channel(&id).is_some_and(|c| c.is_voice());
            acts.push(Act::Cmd(if voice { Cmd::JoinVoice(id) } else { Cmd::OpenChannel(id) }));
            app.focus_composer = true;
        }
        Hit::Place(i) => {
            if let Some(pl) = places.get(i) {
                super::super::open_place(pl, acts);
            }
        }
        Hit::Person(uid) => app.next_modal = Some(dialogs::Modal::People { open: Some(uid), ask: None }),
        Hit::Search => {
            app.side = Some(super::super::side::Side::Search { q: sw.q.trim().to_string(), everywhere: true, edited: 0.0, sent: None, focus: true });
        }
        Hit::Action(id) => match id {
            "settings" => acts.push(Act::OpenSettings(dialogs::TAB_LOOK)),
            "away" => acts.push(Act::OpenSettings(dialogs::TAB_PROFILE)),
            "search" => app.side = Some(super::super::side::Side::search()),
            "catchup" => {
                app.catch_up = Some(pos2(280.0, 120.0));
                app.catch_up_opened = ui.input(|i| i.time);
                acts.push(Act::Cmd(Cmd::UnreadCounts));
            }
            "shelf" => {
                acts.push(Act::Cmd(Cmd::Shelf { channel: m.config.channel.clone(), more: false }));
                app.side = Some(super::super::side::Side::Shelf);
            }
            "pins" => {
                acts.push(Act::Cmd(Cmd::Pins(m.config.channel.clone())));
                app.side = Some(super::super::side::Side::Pins);
            }
            "link" => {
                if let Some((_, link)) = super::super::here_link(m) {
                    acts.push(Act::Copy(link, super::super::LINK_COPIED));
                }
            }
            "ping" => app.next_modal = Some(dialogs::Modal::Ping { note: String::new() }),
            "hang" => app.next_modal = Some(dialogs::Modal::PlanHang(PlanHang::new(m))),
            "soundboard" => {
                app.soundboard = Some(pos2(280.0, 160.0));
                app.soundboard_opened = ui.input(|i| i.time);
            }
            _ => {}
        },
    }
}
