//! Places (the server's links, Draw, Space, the web-only views) and the
//! bits of the sidebar around them: who's on a place, what's on in Watch
//! together, the LIVE badge, the voice channel's quality picker, and the
//! footer (version, updates, privacy).

use super::*;

/// One sidebar Place: the server's links, plus the web-only views.
pub struct Place {
    pub name: String,
    pub icon: &'static str,
    /// Where an outside link goes (the server's links only).
    pub url: String,
    pub tip: String,
    /// Server link id (can be removed); None for Draw / Space.
    pub id: Option<String>,
    /// Opens inside the app (Draw, Space) instead of the browser.
    pub view: Option<core::View>,
    /// Watch together (in the browser; its audience is listed under it).
    pub watch: bool,
    /// A place in the web app (`watch`, `when`): opens there signed in.
    pub web: Option<&'static str>,
    /// Its name in a link to it (`#place=space`): Draw, Space, Watch, When's
    /// good and the board; None for the server's other links.
    pub link: Option<&'static str>,
}

/// Clicking a place: a shared view opens in the main pane; the web app's
/// places open in the browser signed in (a `device.handoff` pass); links open as they are.
pub fn open_place(place: &Place, acts: &mut Vec<Act>) {
    match (place.view, place.web) {
        (Some(v), _) => acts.push(Act::Cmd(Cmd::SetView(Some(v)))),
        (None, Some(w)) => acts.push(Act::Cmd(Cmd::OpenWeb { place: Some(w.into()), video: None })),
        (None, None) => acts.push(Act::OpenUrl(place.url.clone())),
    }
}

/// A place this server doesn't have, or a channel that's gone (the web's words).
pub const NO_SUCH_PLACE: &str = "That link is to a place this server doesn't have.";
pub const NO_SUCH_CHANNEL: &str = "That channel is gone (or isn't yours to see).";

/// A link in a message, clicked. One to a place or a channel on this server
/// (PROTOCOL.md › Links to places and channels) opens here, the way the
/// sidebar opens it (a message in it: scrolled to and flashed); one to a
/// place this server doesn't have, or a channel that's gone, says so.
/// Anything else, another server's link or an invite or sign-in link
/// among them, opens in the browser.
pub fn open_link(app: &mut App, m: &Model, url: String, acts: &mut Vec<Act>) {
    match core::links::parse(&m.config.server, &url) {
        None => acts.push(Act::OpenUrl(url)),
        Some(core::links::Target::Place(name)) => match places(m).into_iter().find(|p| p.link == Some(name.as_str())) {
            Some(place) => {
                if place.view.is_some() {
                    screen::leaving(app, m, acts);
                }
                open_place(&place, acts);
            }
            None => acts.push(Act::Toast(NO_SUCH_PLACE.into())),
        },
        Some(core::links::Target::Channel { id, message }) => {
            if !m.channel(&id).is_some_and(|c| c.is_chat()) {
                return acts.push(Act::Toast(NO_SUCH_CHANNEL.into()));
            }
            screen::leaving(app, m, acts);
            match message {
                Some(msg) => content::jump_in(app, m, &id, &msg, acts),
                None => acts.push(Act::Cmd(Cmd::OpenChannel(id))),
            }
            app.focus_composer = true;
        }
    }
}

/// The main pane while a shared view is open (voice keeps going).
pub fn view_pane(app: &mut App, ui: &mut egui::Ui, m: &Model, p: &Pal, acts: &mut Vec<Act>, v: core::View) {
    match v {
        core::View::Draw => draw::show(app, ui, m, p, acts),
        core::View::Space => space::show(app, ui, m, p, acts),
    }
}

/// A view's title and icon.
pub fn view_title(v: core::View) -> (&'static str, &'static str) {
    match v {
        core::View::Draw => ("Draw", "brush"),
        core::View::Space => ("Space", "map"),
    }
}

pub fn places(m: &Model) -> Vec<Place> {
    let mut items: Vec<Place> = m
        .places()
        .into_iter()
        .map(|l| {
            let url = match l.view.as_deref() {
                Some(view) => format!("{}/{view}", m.config.server),
                None => l.url.clone(),
            };
            let tip = l.description.clone().filter(|d| !d.is_empty()).unwrap_or_else(|| l.url.clone());
            let board = l.view.as_deref() == Some("board");
            let icon = if board { "board" } else { "external" };
            Place { name: l.name, icon, url, tip, id: (!l.id.is_empty()).then_some(l.id), view: None, watch: false, web: None, link: board.then_some("board") }
        })
        .collect();
    items.push(Place { name: "Draw".into(), icon: "brush", url: String::new(), tip: "The shared drawing board (voice keeps going)".into(), id: None, view: Some(core::View::Draw), watch: false, web: None, link: Some("draw") });
    items.push(Place { name: "Space".into(), icon: "map", url: String::new(), tip: "Walk around together (voice keeps going)".into(), id: None, view: Some(core::View::Space), watch: false, web: None, link: Some("space") });
    // Watch together (a shared YouTube player) and When's good (a week
    // planner) live on the web: straight to them in the browser, signed in.
    items.push(Place { name: "Watch".into(), icon: "tv", url: String::new(), tip: "Watch YouTube together (opens Yap in your browser, signed in)".into(), id: None, view: None, watch: true, web: Some("watch"), link: Some("watch") });
    items.push(Place { name: "When's good".into(), icon: "calendar", url: String::new(), tip: "Find a time that works for everyone (opens Yap in your browser, signed in)".into(), id: None, view: None, watch: false, web: Some("when"), link: Some("when") });
    items
}

/// Copy link's toast (the web's words).
pub const LINK_COPIED: &str = "Link copied. Anyone here who opens it lands right here.";

/// The link to a place (PROTOCOL.md › Links to places and channels): never
/// the invite key, so it's only for people who already use Yap here.
pub fn place_link(m: &Model, place: &Place) -> Option<String> {
    place.link.map(|l| core::links::place_link(&m.config.server, l))
}

/// The link to a text channel (not voice, not your Personal space).
pub fn channel_link(m: &Model, c: &crate::protocol::Channel) -> Option<String> {
    c.is_text().then(|| core::links::channel_link(&m.config.server, &c.id))
}

/// What's open, as (its name, its link), if it has one: Draw or Space, or a
/// text channel.
pub fn here_link(m: &Model) -> Option<(String, String)> {
    match m.view {
        Some(v) => Some((view_title(v).0.into(), core::links::place_link(&m.config.server, v.name()))),
        None => m.channel(&m.config.channel).and_then(|c| Some((c.name.clone(), channel_link(m, c)?))),
    }
}

/// Copy link, in a row's right-click menu.
pub fn copy_link_item(ui: &mut egui::Ui, link: &str, p: &Pal, acts: &mut Vec<Act>) {
    if theme::menu_item(ui, "Copy link", false, p).clicked() {
        acts.push(Act::Copy(link.to_string(), LINK_COPIED));
        ui.close();
    }
}

/// Who's at a place: on a shared view, or in the Watch audience (people, A–Z; each once).
pub fn place_people<'a>(m: &'a Model, place: &Place) -> Vec<(&'a User, String)> {
    let at = |s: &crate::protocol::Session| match place.view {
        Some(v) => s.view.as_deref() == Some(v.name()),
        None => place.watch && s.watching,
    };
    let mut out: Vec<(&User, String)> = Vec::new();
    for s in m.sessions.values().filter(|s| at(s) && !s.is_obs()) {
        if let Some(u) = m.users.get(&s.uid)
            && !out.iter().any(|(x, _)| x.id == u.id)
        {
            out.push((u, s.sid.clone()));
        }
    }
    out.sort_by_key(|(u, _)| u.name.to_lowercase());
    out
}

/// What's on in Watch together ("▶ title"), for the line under its entry.
pub fn watch_now(m: &Model, place: &Place) -> Option<String> {
    let w = m.watch.as_ref().filter(|_| place.watch)?;
    w.video_id.as_ref()?;
    Some(w.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "A video".into()))
}

/// "▶ title" under Watch: a play triangle and the title on one line (… past `max_w`).
pub fn now_playing(ui: &egui::Ui, left_center: Pos2, title: &str, font: egui::FontId, color: Color32, max_w: f32) {
    let c = left_center + vec2(4.0, 0.0);
    ui.painter().add(egui::Shape::convex_polygon(vec![c + vec2(-3.5, -4.5), c + vec2(4.5, 0.0), c + vec2(-3.5, 4.5)], color, egui::Stroke::NONE));
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_string(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: (max_w - 14.0).max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let g = ui.painter().layout_job(job);
    ui.painter().galley(egui::pos2(left_center.x + 14.0, left_center.y - g.size().y / 2.0), g, color);
}

/// The links pinned at the very bottom of the left panel, outside the
/// scrolling list (like the web's `.sidebar-footer`): "v0.x · what's new"
/// (the changelog), Updates (this app's own updates) and Privacy, under an
/// etched line.
/// Paints at the bottom of what's left of `ui` and returns the rect above it.
pub fn sidebar_footer(app: &mut App, ui: &mut egui::Ui, m: &Model, acts: &mut Vec<Act>) -> Rect {
    let avail = ui.available_rect_before_wrap();
    let p = theme::pal(ui.ctx());
    let (font, color, hover, pad, gap) = (theme::font(11.0), p.gray, p.link, egui::Margin { left: 6, right: 6, top: 4, bottom: 4 }, 10.0);
    #[derive(Clone, Copy, PartialEq)]
    enum Link {
        Changelog,
        Updates,
        Privacy,
    }
    let items: [(String, Link, &'static str); 3] = [
        (format!("v{} · what's new", env!("CARGO_PKG_VERSION")), Link::Changelog, "What's new: the Yap changelog"),
        ("Updates".into(), Link::Updates, if crate::update::enabled() { "Updates for this app (signed; installs only when you click)" } else { "Where new versions of this app come from" }),
        ("Privacy".into(), Link::Privacy, "What Yap keeps, and what it doesn't"),
    ];
    // Lay out (wrapping when the panel is narrow), then pin it to the bottom.
    let left = avail.left() + pad.left as f32;
    let right = avail.right() - pad.right as f32;
    let layout = |t: &str| ui.painter().layout_no_wrap(t.to_string(), font.clone(), color);
    let space = gap;
    type Placed = (egui::Pos2, Option<std::sync::Arc<egui::Galley>>, std::sync::Arc<egui::Galley>, Link, &'static str);
    let mut placed: Vec<Placed> = Vec::new();
    let (mut x, mut y, mut line_h) = (left, 0.0f32, 0.0f32);
    for (i, (label, link, tip)) in items.iter().enumerate() {
        let mut dot: Option<std::sync::Arc<egui::Galley>> = None;
        let g = layout(label);
        let w = dot.as_ref().map_or(0.0, |d| d.size().x) + g.size().x;
        if i > 0 && x + space + w > right {
            // A new line starts without its "·".
            x = left;
            y += line_h;
            dot = None;
        } else if i > 0 {
            x += space;
        }
        line_h = line_h.max(g.size().y);
        x += dot.as_ref().map_or(0.0, |d| d.size().x) + g.size().x;
        placed.push((egui::pos2(x - dot.as_ref().map_or(0.0, |d| d.size().x) - g.size().x, y), dot, g, *link, tip));
    }
    let height = (y + line_h + (pad.top + pad.bottom) as f32).ceil();
    let strip = Rect::from_min_max(egui::pos2(avail.left(), avail.bottom() - height), avail.max);
    theme::etched_hline(ui.painter(), strip.x_range(), strip.top(), &theme::pal(ui.ctx()));
    let top = strip.top() + pad.top as f32;
    for (at, dot, g, link, tip) in placed {
        let mut at = egui::pos2(at.x, top + at.y);
        if let Some(d) = dot {
            let w = d.size().x;
            ui.painter().galley(at, d, color);
            at.x += w;
        }
        let r = Rect::from_min_size(at, g.size());
        let resp = ui.interact(r, ui.id().with(("sidebar-footer", link as u8)), egui::Sense::click());
        let fg = if resp.hovered() { hover } else { color };
        ui.painter().galley_with_override_text_color(at, g, fg);
        if resp.hovered() {
            ui.painter().hline(r.x_range(), (r.bottom() - 1.0).round(), egui::Stroke::new(1.0, fg));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if theme::tip(resp, tip).clicked() {
            match link {
                Link::Changelog => acts.push(Act::OpenUrl(format!("{}/changelog", m.config.server))),
                Link::Privacy => acts.push(Act::OpenUrl(format!("{}/privacy", m.config.server))),
                Link::Updates => {
                    if crate::update::enabled() && !matches!(m.update, UpdateState::Available(_) | UpdateState::Downloading { .. } | UpdateState::Installed(_) | UpdateState::Checking) {
                        acts.push(Act::Cmd(Cmd::CheckUpdate { manual: true }));
                    }
                    app.modal = Some(dialogs::Modal::Update);
                }
            }
        }
    }
    Rect::from_min_max(avail.min, egui::pos2(avail.right(), strip.top()))
}

/// The red LIVE badge by someone sharing their screen in voice (`me`: our
/// own share); returns its click response.
pub fn live_badge(ui: &mut egui::Ui, left_center: Pos2, id: &str, red: Color32, name: &str, me: bool) -> egui::Response {
    let g = ui.painter().layout_no_wrap("LIVE".into(), theme::bold_font(9.5), Color32::WHITE);
    let r = Rect::from_min_size(egui::pos2(left_center.x, left_center.y - 7.0), vec2(g.size().x + 8.0, 14.0)).round();
    let resp = ui.interact(r, ui.id().with(("live", id)), egui::Sense::click());
    let fill = if resp.hovered() { red.gamma_multiply(0.85) } else { red };
    ui.painter().rect_filled(r, 0.0, fill);
    ui.painter().galley(egui::pos2(r.left() + 4.0, r.center().y - g.size().y / 2.0), g, Color32::WHITE);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    // The web's words.
    theme::tip(resp, if me { "You’re sharing your screen (click to stop)".to_string() } else { format!("Watch {name}'s screen") })
}

/// Voice-channel quality choices: None = each person's own setting.
pub const CHANNEL_BITRATES: [Option<u32>; 6] = [None, Some(64_000), Some(96_000), Some(128_000), Some(256_000), Some(510_000)];
/// The web's words: "Each person's own (yours: 128k)" / "128k for everyone".
pub fn bitrate_label(choice: Option<u32>, own: u32) -> String {
    match choice {
        None => format!("Each person's own (yours: {}k)", own / 1000),
        Some(b) => format!("{}k for everyone", b / 1000),
    }
}

/// "Sound: 256k for everyone" under the voice panel's details, only when the
/// channel sets one quality for everyone (the picker itself is under ⋯).
pub fn shared_quality(m: &Model) -> Option<(String, String)> {
    let ch = m.my_voice_channel()?;
    let b = ch.bitrate?;
    Some((format!("Sound: {}k for everyone", b / 1000), format!("Set for everyone in {} (⋯ changes it)", ch.name)))
}

/// The call report's words (PROTOCOL.md › Voice › Call report; the web's).
const CALL_REPORT_TIP: &str = "What Yap noticed about this call: connections, lost audio, devices. No messages or IP addresses.";

/// Copy call report (the call's ⋯) / Copy last call report (Settings › Voice):
/// disabled, with "No call yet.", before the first call.
pub fn copy_call_report(ui: &mut egui::Ui, shared: &core::Shared, label: &str, p: &Pal, acts: &mut Vec<Act>) {
    let any = shared.calls.lock().any();
    ui.horizontal_wrapped(|ui| {
        if theme::tip(theme::button(ui, label, theme::Btn::Normal, any, p), CALL_REPORT_TIP).clicked()
            && let Some(text) = shared.calls.lock().text()
        {
            acts.push(Act::Copy(text, "Call report copied."));
        }
        if !any {
            super::dialogs::note(ui, "No call yet.", p);
        }
    });
}

/// Settings › Voice: the last call's report ("Copy call report" while in one).
pub fn call_report_field(ui: &mut egui::Ui, shared: &core::Shared, p: &Pal, acts: &mut Vec<Act>) {
    ui.add_space(2.0);
    super::dialogs::field_label(ui, "Call report:", p);
    let label = if shared.calls.lock().in_call() { "Copy call report" } else { "Copy last call report" };
    copy_call_report(ui, shared, label, p, acts);
    super::dialogs::note(ui, "What Yap noticed about your last call: connections, lost audio, devices. No messages or IP addresses. Send it along when a call sounded off.", p);
}

/// Open (or close) the voice panel's ⋯ popover under `anchor`.
pub fn toggle_voice_more(app: &mut App, ctx: &egui::Context, anchor: Rect) {
    if app.voice_more.is_some() {
        app.voice_more = None;
    } else {
        app.voice_more = Some(anchor.left_bottom() + vec2(0.0, 4.0));
        app.voice_more_opened = ctx.input(|i| i.time);
    }
}

/// The voice panel's ⋯: the sound quality for everyone in this channel (or
/// each person's own), and the call report, like the web's popover.
pub fn voice_more_popover(app: &mut App, ctx: &egui::Context, m: &Model, p: &Pal, acts: &mut Vec<Act>) {
    let Some(pos) = app.voice_more else { return };
    let Some(ch) = m.my_voice_channel() else {
        app.voice_more = None;
        return;
    };
    let current = ch.bitrate;
    let own = m.config.bitrate;
    let id = ch.id.clone();
    let area = egui::Area::new(egui::Id::new("voice-more")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        super::chat::picker_window(ui, &ch.name, 300.0, p, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(theme::text("Sound quality for everyone here:", p.text));
            theme::combo(ui, "voice-quality", &bitrate_label(current, own), ui.available_width(), p, |ui| {
                for choice in CHANNEL_BITRATES {
                    if theme::menu_item(ui, &bitrate_label(choice, own), choice == current, p).clicked() && choice != current {
                        acts.push(Act::Cmd(Cmd::SetChannelBitrate { channel_id: id.clone(), bitrate: choice }));
                    }
                }
            });
            ui.add_space(2.0);
            ui.label(egui::RichText::new("Your own quality is in Settings › Voice › Advanced.").font(theme::font(theme::small())).color(theme::dim(p)));
            ui.add_space(4.0);
            copy_call_report(ui, &app.shared, "Copy call report", p, acts);
        });
    });
    let menu_open = egui::Popup::is_any_open(ctx);
    let outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).is_some_and(|q| !area.response.rect.contains(q));
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || (outside && !menu_open && app.voice_more_opened + 0.2 < ctx.input(|i| i.time)) {
        app.voice_more = None;
    }
}
