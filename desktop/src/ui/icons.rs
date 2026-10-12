//! The web client's Lucide icons (web/js/icons.js), drawn as vector strokes.
//! A tiny SVG subset parser: line, polygon, circle, rect, and path data
//! (M L H V C S Q A Z, absolute and relative). Parsed once, then scaled.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, pos2, vec2};
use std::collections::HashMap;
use std::sync::LazyLock;

const PATHS: &[(&str, &str)] = &[
    ("hash", r#"<line x1="4" x2="20" y1="9" y2="9"/><line x1="4" x2="20" y1="15" y2="15"/><line x1="10" x2="8" y1="3" y2="21"/><line x1="16" x2="14" y1="3" y2="21"/>"#),
    ("volume", r#"<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/>"#),
    ("volume-x", r#"<polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><line x1="22" x2="16" y1="9" y2="15"/><line x1="16" x2="22" y1="9" y2="15"/>"#),
    ("mic", r#"<path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z"/><path d="M19 10v2a7 7 0 0 1-14 0v-2"/><line x1="12" x2="12" y1="19" y2="22"/>"#),
    ("mic-off", r#"<line x1="2" x2="22" y1="2" y2="22"/><path d="M18.89 13.23A7.12 7.12 0 0 0 19 12v-2"/><path d="M5 10v2a7 7 0 0 0 12 5"/><path d="M15 9.34V5a3 3 0 0 0-5.68-1.33"/><path d="M9 9v3a3 3 0 0 0 5.12 2.12"/><line x1="12" x2="12" y1="19" y2="22"/>"#),
    ("headphones", r#"<path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"/>"#),
    ("headphones-off", r#"<line x1="2" x2="22" y1="2" y2="22"/><path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 2.64-6.36"/><path d="M8.5 3.68A9 9 0 0 1 21 12v5"/><path d="M21 14h-3a2 2 0 0 0-1.4.57"/>"#),
    ("settings", r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/>"#),
    ("phone-off", r#"<path d="M10.68 13.31a16 16 0 0 0 3.41 2.6l1.27-1.27a2 2 0 0 1 2.11-.45 12.84 12.84 0 0 0 2.81.7 2 2 0 0 1 1.72 2v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.42 19.42 0 0 1-3.33-2.67m-2.67-3.34a19.79 19.79 0 0 1-3.07-8.63A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72 12.84 12.84 0 0 0 .7 2.81 2 2 0 0 1-.45 2.11L8.09 9.91"/><line x1="22" x2="2" y1="2" y2="22"/>"#),
    ("plus", r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#),
    ("users", r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>"#),
    ("trash", r#"<path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/>"#),
    ("archive", r#"<rect width="20" height="5" x="2" y="3" rx="1"/><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"/><path d="M10 12h4"/>"#),
    ("x", r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#),
    ("send", r#"<path d="M14.54 21.69a.5.5 0 0 0 .94-.03l6.5-19a.5.5 0 0 0-.64-.64l-19 6.5a.5.5 0 0 0-.03.94l7.93 3.18a2 2 0 0 1 1.11 1.11z"/><path d="m21.85 2.15-10.94 10.94"/>"#),
    ("smile", r#"<circle cx="12" cy="12" r="10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/><line x1="9" x2="9.01" y1="9" y2="9"/><line x1="15" x2="15.01" y1="9" y2="9"/>"#),
    ("signal", r#"<path d="M2 20h.01"/><path d="M7 20v-4"/><path d="M12 20v-8"/><path d="M17 20V8"/>"#),
    ("link", r#"<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>"#),
    ("check", r#"<path d="M20 6 9 17l-5-5"/>"#),
    ("chat", r#"<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>"#),
    ("megaphone", r#"<path d="m3 11 18-5v12L3 14v-3z"/><path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"/>"#),
    ("board", r#"<rect width="18" height="18" x="3" y="3" rx="2"/><path d="M9 3v18"/><path d="M15 3v18"/>"#),
    ("brush", r#"<path d="m9.06 11.9 8.07-8.06a2.85 2.85 0 1 1 4.03 4.03l-8.06 8.08"/><path d="M7.07 14.94c-1.66 0-3 1.35-3 3.02 0 1.33-2.5 1.52-2 2.02 1.08 1.1 2.49 2.02 4 2.02 2.2 0 4-1.8 4-4.04a3.01 3.01 0 0 0-3-3.02z"/>"#),
    ("map", r#"<path d="M14.1 6 9.9 4 3 7v13l6.9-3 4.2 2 6.9-3V3z"/><path d="M9.9 4v13"/><path d="M14.1 6v13"/>"#),
    ("smile-plus", r#"<path d="M22 11v1a10 10 0 1 1-9-10"/><path d="M8 14s1.5 2 4 2 4-2 4-2"/><line x1="9" x2="9.01" y1="9" y2="9"/><line x1="15" x2="15.01" y1="9" y2="9"/><path d="M16 5h6"/><path d="M19 2v6"/>"#),
    ("box", r#"<path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"/><path d="m3.3 7 8.7 5 8.7-5"/><path d="M12 22V12"/>"#),
    ("download", r#"<path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><path d="M7 10l5 5 5-5"/><line x1="12" x2="12" y1="15" y2="3"/>"#),
    ("arrow-up", r#"<path d="m5 12 7-7 7 7"/><path d="M12 19V5"/>"#),
    ("pencil", r#"<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>"#),
    ("image", r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"/>"#),
    ("monitor", r#"<rect width="20" height="14" x="2" y="3" rx="2"/><line x1="8" x2="16" y1="21" y2="21"/><line x1="12" x2="12" y1="17" y2="21"/>"#),
    ("maximize", r#"<path d="M15 3h6v6"/><path d="M9 21H3v-6"/><line x1="21" x2="14" y1="3" y2="10"/><line x1="3" x2="10" y1="21" y2="14"/>"#),
    ("tv", r#"<rect width="20" height="15" x="2" y="7" rx="2" ry="2"/><path d="m17 2-5 5-5-5"/>"#),
    ("lock", r#"<rect width="18" height="11" x="3" y="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/>"#),
    ("reply", r#"<path d="M9 17 4 12 9 7"/><path d="M20 18v-2a4 4 0 0 0-4-4H4"/>"#),
    ("calendar", r#"<rect width="18" height="18" x="3" y="4" rx="2"/><path d="M16 2v4"/><path d="M8 2v4"/><path d="M3 10h18"/><path d="M8 14h.01"/><path d="M12 14h.01"/><path d="M16 14h.01"/><path d="M8 18h.01"/><path d="M12 18h.01"/>"#),
    ("shield", r#"<path d="M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z"/>"#),
    ("search", r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#),
    ("pin", r#"<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/>"#),
    ("eraser", r#"<path d="m7 21-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21"/><path d="M22 21H7"/><path d="m5 11 9 9"/>"#),
    ("undo", r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>"#),
    ("share", r#"<path d="M4 12v8a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8"/><path d="m16 6-4-4-4 4"/><path d="M12 2v13"/>"#),
    ("arrow-left", r#"<path d="m12 19-7-7 7-7"/><path d="M19 12H5"/>"#),
    ("bookmark", r#"<path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z"/>"#),
    ("music", r#"<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>"#),
    ("poll", r#"<line x1="18" x2="18" y1="20" y2="10"/><line x1="12" x2="12" y1="20" y2="4"/><line x1="6" x2="6" y1="20" y2="14"/>"#),
    ("play", r#"<polygon points="6 3 20 12 6 21 6 3"/>"#),
    ("clock", r#"<circle cx="12" cy="12" r="10"/><path d="M12 6v6l4 2"/>"#),
    ("moon", r#"<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"/>"#),
    ("images", r#"<path d="M18 22H4a2 2 0 0 1-2-2V6"/><path d="m22 13-1.296-1.296a2.41 2.41 0 0 0-3.408 0L11 18"/><circle cx="12" cy="8" r="2"/><rect width="16" height="16" x="6" y="2" rx="2"/>"#),
    ("inbox", r#"<path d="M22 12h-6l-2 3h-4l-2-3H2"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>"#),
    ("command", r#"<path d="M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3"/>"#),
    ("dice", r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"/><path d="M16 8h.01"/><path d="M8 8h.01"/><path d="M8 16h.01"/><path d="M16 16h.01"/><path d="M12 12h.01"/>"#),
    ("external", r#"<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>"#),
    ("eye", r#"<path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0"/><circle cx="12" cy="12" r="3"/>"#),
    ("eye-off", r#"<path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49"/><path d="M14.084 14.158a3 3 0 0 1-4.242-4.242"/><path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143"/><path d="m2 2 20 20"/>"#),
    // The waiting room, Catch up's quiet lines, the voice panel's ⋯ (web/js/icons.js).
    ("user-plus", r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><line x1="19" x2="19" y1="8" y2="14"/><line x1="22" x2="16" y1="11" y2="11"/>"#),
    ("alert", r#"<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>"#),
    ("more", r#"<circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/><circle cx="5" cy="12" r="1"/>"#),
    ("sparkles", r#"<path d="M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z"/>"#),
];

type Polyline = (Vec<Pos2>, bool);

static ICONS: LazyLock<HashMap<&'static str, Vec<Polyline>>> = LazyLock::new(|| PATHS.iter().map(|(n, svg)| (*n, parse_svg(svg))).collect());

/// Paint icon `name` into `rect` (a 24×24 design scaled to fit).
pub fn paint(painter: &egui::Painter, rect: Rect, name: &str, color: Color32) {
    let Some(lines) = ICONS.get(name) else { return };
    let s = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - vec2(12.0 * s, 12.0 * s);
    let stroke = Stroke::new((1.9 * s).max(1.1), color);
    for (pts, closed) in lines {
        let pts: Vec<Pos2> = pts.iter().map(|p| origin + p.to_vec2() * s).collect();
        if pts.len() == 1 || (pts.len() == 2 && pts[0].distance(pts[1]) < 0.05 * s) {
            painter.circle_filled(pts[0], stroke.width * 0.55, color);
        } else if *closed {
            painter.add(egui::Shape::closed_line(pts, stroke));
        } else {
            painter.add(egui::Shape::line(pts, stroke));
        }
    }
}

/// A toolbar button (IE / Office style): flat until you point at it, then a
/// thin raised edge; pressed (or toggled `on`) it sinks and the icon shifts.
pub fn tool(ui: &mut egui::Ui, name: &str, size: egui::Vec2, color: Color32, tooltip: &str, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = super::theme::pal(ui.ctx());
    let pressed = resp.is_pointer_button_down_on() || on;
    if on {
        super::theme::dither(ui, rect, p.hilight);
    }
    if pressed {
        super::theme::bevel(ui.painter(), rect, &p, super::theme::Bevel::ThinIn);
    } else if resp.hovered() {
        super::theme::bevel(ui.painter(), rect, &p, super::theme::Bevel::ThinOut);
    }
    let icon = (size.y * 0.62).round().min(16.0);
    let c = rect.center() + if pressed { vec2(1.0, 1.0) } else { vec2(0.0, 0.0) };
    paint(ui.painter(), Rect::from_center_size(c, vec2(icon, icon)), name, color);
    super::theme::tip(resp, tooltip)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

fn num(tag: &str, name: &str) -> f32 {
    attr(tag, name).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

fn parse_svg(svg: &str) -> Vec<Polyline> {
    let mut out = Vec::new();
    for tag in svg.split('<').filter(|t| !t.trim().is_empty()) {
        let kind = tag.split_whitespace().next().unwrap_or("");
        match kind {
            "line" => out.push((vec![pos2(num(tag, "x1"), num(tag, "y1")), pos2(num(tag, "x2"), num(tag, "y2"))], false)),
            "polygon" => {
                let nums: Vec<f32> = attr(tag, "points").unwrap_or_default().split([' ', ',']).filter_map(|v| v.parse().ok()).collect();
                let pts = nums.chunks(2).filter(|c| c.len() == 2).map(|c| pos2(c[0], c[1])).collect();
                out.push((pts, true));
            }
            "circle" => {
                let (cx, cy, r) = (num(tag, "cx"), num(tag, "cy"), num(tag, "r"));
                let n = 32;
                let pts = (0..n).map(|i| {
                    let a = i as f32 / n as f32 * std::f32::consts::TAU;
                    pos2(cx + r * a.cos(), cy + r * a.sin())
                });
                out.push((pts.collect(), true));
            }
            "rect" => {
                let (x, y, w, h) = (num(tag, "x"), num(tag, "y"), num(tag, "width"), num(tag, "height"));
                out.push((vec![pos2(x, y), pos2(x + w, y), pos2(x + w, y + h), pos2(x, y + h)], true));
            }
            "path" => out.extend(parse_path(&attr(tag, "d").unwrap_or_default())),
            _ => {}
        }
    }
    out
}

/// Tokenize path data into commands and numbers (handles "-.5", ".5.5", flags).
fn tokens(d: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let b = d.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_ascii_alphabetic() {
            out.push(Tok::Cmd(c));
            i += 1;
        } else if c == '-' || c == '.' || c.is_ascii_digit() {
            let start = i;
            let mut seen_dot = false;
            let mut seen_exp = false;
            if c == '-' {
                i += 1;
            }
            while i < b.len() {
                let ch = b[i] as char;
                if ch.is_ascii_digit() {
                    i += 1;
                } else if ch == '.' && !seen_dot && !seen_exp {
                    seen_dot = true;
                    i += 1;
                } else if (ch == 'e' || ch == 'E') && !seen_exp {
                    seen_exp = true;
                    i += 1;
                    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
                        i += 1;
                    }
                } else {
                    break;
                }
            }
            out.push(Tok::Num(d[start..i].parse().unwrap_or(0.0)));
        } else {
            i += 1;
        }
    }
    out
}

#[derive(Clone, Copy)]
enum Tok {
    Cmd(char),
    Num(f32),
}

fn parse_path(d: &str) -> Vec<Polyline> {
    let toks = tokens(d);
    let mut out: Vec<Polyline> = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    let mut p = pos2(0.0, 0.0);
    let mut start = p;
    let mut last_ctrl: Option<Pos2> = None;
    let mut cmd = 'M';
    let mut i = 0;
    let next = |i: &mut usize| -> Option<f32> {
        match toks.get(*i) {
            Some(Tok::Num(v)) => {
                *i += 1;
                Some(*v)
            }
            _ => None,
        }
    };
    // Arc flags may be packed ("0 1" or "01"); the tokenizer reads "01" as one number.
    let mut flag_queue: Vec<f32> = Vec::new();
    let mut flag = |i: &mut usize| -> Option<f32> {
        if let Some(f) = flag_queue.pop() {
            return Some(f);
        }
        match toks.get(*i) {
            Some(Tok::Num(v)) => {
                *i += 1;
                let s = format!("{v}");
                if s.len() > 1 && !s.contains('.') && s.chars().all(|c| c == '0' || c == '1') {
                    let digits: Vec<f32> = s.chars().map(|c| if c == '1' { 1.0 } else { 0.0 }).collect();
                    for d in digits[1..].iter().rev() {
                        flag_queue.push(*d);
                    }
                    Some(digits[0])
                } else {
                    Some(*v)
                }
            }
            _ => None,
        }
    };
    let flush = |cur: &mut Vec<Pos2>, out: &mut Vec<Polyline>, closed: bool| {
        if cur.len() >= 2 || (cur.len() == 1 && !closed) {
            out.push((std::mem::take(cur), closed));
        } else {
            cur.clear();
        }
    };
    while i < toks.len() {
        if let Tok::Cmd(c) = toks[i] {
            cmd = c;
            i += 1;
            if c == 'Z' || c == 'z' {
                flush(&mut cur, &mut out, true);
                p = start;
                last_ctrl = None;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { p.to_vec2() } else { egui::Vec2::ZERO };
        match cmd.to_ascii_uppercase() {
            'M' => {
                let (Some(x), Some(y)) = (next(&mut i), next(&mut i)) else { break };
                flush(&mut cur, &mut out, false);
                p = pos2(x, y) + base;
                start = p;
                cur.push(p);
                cmd = if rel { 'l' } else { 'L' };
                last_ctrl = None;
            }
            'L' => {
                let (Some(x), Some(y)) = (next(&mut i), next(&mut i)) else { break };
                p = pos2(x, y) + base;
                cur.push(p);
                last_ctrl = None;
            }
            'H' => {
                let Some(x) = next(&mut i) else { break };
                p = pos2(if rel { p.x + x } else { x }, p.y);
                cur.push(p);
                last_ctrl = None;
            }
            'V' => {
                let Some(y) = next(&mut i) else { break };
                p = pos2(p.x, if rel { p.y + y } else { y });
                cur.push(p);
                last_ctrl = None;
            }
            'C' | 'S' => {
                let is_s = cmd.eq_ignore_ascii_case(&'S');
                let c1 = if is_s {
                    last_ctrl.map(|c| p + (p - c)).unwrap_or(p)
                } else {
                    let (Some(x), Some(y)) = (next(&mut i), next(&mut i)) else { break };
                    pos2(x, y) + base
                };
                let (Some(x2), Some(y2), Some(x), Some(y)) = (next(&mut i), next(&mut i), next(&mut i), next(&mut i)) else { break };
                let c2 = pos2(x2, y2) + base;
                let end = pos2(x, y) + base;
                for k in 1..=12 {
                    let t = k as f32 / 12.0;
                    let u = 1.0 - t;
                    let v = p.to_vec2() * (u * u * u) + c1.to_vec2() * (3.0 * u * u * t) + c2.to_vec2() * (3.0 * u * t * t) + end.to_vec2() * (t * t * t);
                    cur.push(v.to_pos2());
                }
                last_ctrl = Some(c2);
                p = end;
            }
            'Q' => {
                let (Some(x1), Some(y1), Some(x), Some(y)) = (next(&mut i), next(&mut i), next(&mut i), next(&mut i)) else { break };
                let c = pos2(x1, y1) + base;
                let end = pos2(x, y) + base;
                for k in 1..=10 {
                    let t = k as f32 / 10.0;
                    let u = 1.0 - t;
                    cur.push((p.to_vec2() * (u * u) + c.to_vec2() * (2.0 * u * t) + end.to_vec2() * (t * t)).to_pos2());
                }
                p = end;
                last_ctrl = None;
            }
            'A' => {
                let (Some(rx), Some(ry), Some(rot)) = (next(&mut i), next(&mut i), next(&mut i)) else { break };
                let (Some(large), Some(sweep)) = (flag(&mut i), flag(&mut i)) else { break };
                let (Some(x), Some(y)) = (next(&mut i), next(&mut i)) else { break };
                let end = pos2(x, y) + base;
                arc(&mut cur, p, end, rx, ry, rot, large != 0.0, sweep != 0.0);
                p = end;
                last_ctrl = None;
            }
            _ => {
                i += 1;
            }
        }
    }
    flush(&mut cur, &mut out, false);
    out
}

/// SVG elliptical arc (endpoint parameterization, SVG spec F.6.5) flattened into points.
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<Pos2>, p0: Pos2, p1: Pos2, rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool) {
    if p0 == p1 {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 {
        out.push(p1);
        return;
    }
    let phi = rot_deg.to_radians();
    let (sin, cos) = phi.sin_cos();
    let dx = (p0.x - p1.x) / 2.0;
    let dy = (p0.y - p1.y) / 2.0;
    let x1p = cos * dx + sin * dy;
    let y1p = -sin * dx + cos * dy;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = (num / den).max(0.0).sqrt();
    if large == sweep {
        coef = -coef;
    }
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * -(ry * x1p / rx);
    let cx = cos * cxp - sin * cyp + (p0.x + p1.x) / 2.0;
    let cy = sin * cxp + cos * cyp + (p0.y + p1.y) / 2.0;
    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let theta1 = angle(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dtheta = angle((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    if !sweep && dtheta > 0.0 {
        dtheta -= std::f32::consts::TAU;
    } else if sweep && dtheta < 0.0 {
        dtheta += std::f32::consts::TAU;
    }
    let steps = ((dtheta.abs() / (std::f32::consts::PI / 12.0)).ceil() as usize).max(2);
    for k in 1..=steps {
        let t = theta1 + dtheta * k as f32 / steps as f32;
        let (st, ct) = t.sin_cos();
        out.push(pos2(cx + cos * rx * ct - sin * ry * st, cy + sin * rx * ct + cos * ry * st));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_icons_parse() {
        for (name, _) in super::PATHS {
            let lines = super::ICONS.get(name).unwrap();
            assert!(!lines.is_empty(), "{name}");
            for (pts, _) in lines {
                for p in pts {
                    assert!(p.x.is_finite() && p.y.is_finite() && p.x > -2.0 && p.x < 26.0 && p.y > -2.0 && p.y < 26.0, "{name}: {p:?}");
                }
            }
        }
    }
}
