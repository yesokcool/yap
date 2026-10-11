//! Message text: the web client's tiny Markdown subset (web/js/format.js) —
//! ```blocks```, `code`, **bold**, *italic*/_italic_, ~~strike~~, "> quotes",
//! bare links, `||spoilers||` — plus custom emoji tokens `<:name:id>` drawn as
//! inline images.
//!
//! Text is laid out as one wrapped egui `LayoutJob`; each emoji is an invisible
//! placeholder glyph whose position we read back to paint the image over it.

use super::images::Images;
use super::theme::{self, Pal};
use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, text::LayoutJob, text::TextFormat};
use std::collections::HashMap;

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub quote: bool,
    /// `||like this||`: a solid block until the message is clicked.
    pub spoiler: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Span {
    Text(String, Style),
    /// A bare link (the style says if it's inside a spoiler).
    Link(String, Style),
    Emoji { name: String, id: String, spoiler: bool },
    /// `@name`, `@everyone`, `@here`.
    Mention(String, Style),
    Newline,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Para(Vec<Span>),
    Code(String),
}

const EMOJI_PLACEHOLDER: char = '\u{E000}';
/// What `prev` is right after code, a link or a custom emoji: like the web's
/// placeholders, they count as a word for @mentions (one right against a
/// name blocks it, as in core::find_mentions), but not for italics.
const COVERED: char = '\u{E001}';

pub fn emoji_url(server: &str, id: &str) -> String {
    format!("{server}/emoji/{id}")
}

/// `<:name:id>` at the start of `s` → (name, id, byte length).
fn emoji_token(s: &str) -> Option<(String, String, usize)> {
    let rest = s.strip_prefix("<:")?;
    let colon = rest.find(':')?;
    let name = &rest[..colon];
    if !(2..=32).contains(&name.len()) || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let after = &rest[colon + 1..];
    let id = after.get(..32)?;
    if !id.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)) || after.as_bytes().get(32) != Some(&b'>') {
        return None;
    }
    Some((name.to_string(), id.to_string(), 2 + colon + 1 + 33))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Nothing a mention can't touch on its left: a word character (as
/// JavaScript's `\w` means it), `@`, or covered code / link / emoji.
fn blocks_mention_before(prev: Option<char>) -> bool {
    prev.is_some_and(|c| crate::core::js_word(c) || c == '@' || c == COVERED)
}

/// …and on its right: a word character, or code / a custom emoji starting
/// right there (`end_covered`: the line ends against a code block).
fn blocks_mention_after(after: &str, end_covered: bool) -> bool {
    match after.chars().next() {
        None => end_covered,
        Some(c) if crate::core::js_word(c) => true,
        Some('`') => after[1..].find('`').is_some_and(|end| end > 0),
        Some('<') => emoji_token(after).is_some(),
        Some(_) => false,
    }
}

/// `names`: everyone's lowercase display name, longest first (so "@Sam Lee"
/// beats "@Sam").
pub fn parse(text: &str, names: &[String]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut rest = text;
    // Right after a code block with no line break between (it counts as a word for @mentions).
    let mut after_block = false;
    loop {
        match rest.find("```") {
            Some(start) => {
                let after = &rest[start + 3..];
                if let Some(end) = after.find("```") {
                    if start > 0 {
                        let before = &rest[..start];
                        blocks.push(Block::Para(parse_lines(before, names, after_block, !before.ends_with('\n'))));
                    }
                    let mut code = &after[..end];
                    // ```lang\n…
                    if let Some(nl) = code.find('\n')
                        && code[..nl].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-')
                    {
                        code = &code[nl + 1..];
                    }
                    blocks.push(Block::Code(code.trim_matches('\n').to_string()));
                    rest = &after[end + 3..];
                    after_block = !rest.starts_with('\n');
                    rest = rest.strip_prefix('\n').unwrap_or(rest);
                    continue;
                }
                blocks.push(Block::Para(parse_lines(rest, names, after_block, false)));
                break;
            }
            None => {
                if !rest.is_empty() {
                    blocks.push(Block::Para(parse_lines(rest, names, after_block, false)));
                }
                break;
            }
        }
    }
    blocks.retain(|b| !matches!(b, Block::Para(s) if s.is_empty()));
    blocks
}

/// `start_covered` / `end_covered`: the text sits right against a code block
/// at its start / end (no line break between).
fn parse_lines(text: &str, names: &[String], start_covered: bool, end_covered: bool) -> Vec<Span> {
    let text = text.trim_end_matches('\n');
    let mut out = Vec::new();
    let last = text.split('\n').count() - 1;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push(Span::Newline);
        }
        let (line, style, quoted) = match line.strip_prefix("> ").or_else(|| line.strip_prefix('>')) {
            Some(q) => (q, Style { quote: true, ..Style::default() }, true),
            None => (line, Style::default(), false),
        };
        let start = (i == 0 && start_covered && !quoted).then_some(COVERED);
        inline_at(line, style, names, &mut out, start, i == last && end_covered);
    }
    out
}

/// `@who` at the start of `rest`, for a known name / everyone / here → byte length.
fn mention_at(rest: &str, names: &[String], end_covered: bool) -> Option<usize> {
    let tail = rest.strip_prefix('@')?;
    let lower = tail.to_lowercase();
    let fits = |n: &str| lower.starts_with(n) && tail.is_char_boundary(n.len()) && !blocks_mention_after(&tail[n.len()..], end_covered);
    ["everyone", "here"].iter().map(|s| s.to_string()).chain(names.iter().cloned()).find(|n| !n.is_empty() && fits(n)).map(|n| 1 + n.len())
}

fn inline(s: &str, style: Style, names: &[String], out: &mut Vec<Span>) {
    inline_at(s, style, names, out, None, false)
}

/// `prev`: what comes before `s` (COVERED right after a code block);
/// `end_covered`: `s` ends against one.
fn inline_at(s: &str, style: Style, names: &[String], out: &mut Vec<Span>, prev: Option<char>, end_covered: bool) {
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<Span>| {
        if !buf.is_empty() {
            out.push(Span::Text(std::mem::take(buf), style));
        }
    };
    let mut i = 0;
    let mut prev = prev;
    'outer: while i < s.len() {
        let rest = &s[i..];
        // `code`
        if rest.starts_with('`')
            && let Some(end) = rest[1..].find('`')
            && end > 0
        {
            flush(&mut buf, out);
            out.push(Span::Text(rest[1..1 + end].to_string(), Style { code: true, ..style }));
            i += end + 2;
            prev = Some(COVERED);
            continue;
        }
        // ||spoiler||: one line, at least one character, no || inside.
        if !style.spoiler
            && let Some(after) = rest.strip_prefix("||")
            && let Some(end) = after.find("||")
            && end > 0
        {
            flush(&mut buf, out);
            inline(&after[..end], Style { spoiler: true, ..style }, names, out);
            i += 2 + end + 2;
            prev = Some('|');
            continue;
        }
        // <:name:id>
        if let Some((name, id, len)) = emoji_token(rest) {
            flush(&mut buf, out);
            out.push(Span::Emoji { name, id, spoiler: style.spoiler });
            i += len;
            prev = Some(COVERED);
            continue;
        }
        // @mentions
        if rest.starts_with('@')
            && !blocks_mention_before(prev)
            && let Some(len) = mention_at(rest, names, end_covered)
        {
            flush(&mut buf, out);
            out.push(Span::Mention(rest[..len].to_string(), style));
            i += len;
            prev = Some('x');
            continue;
        }
        // Bare links
        if (rest.starts_with("https://") || rest.starts_with("http://")) && !prev.is_some_and(crate::core::js_word) {
            let end = rest.find(|c: char| c.is_whitespace() || c == '<').unwrap_or(rest.len());
            let mut url = &rest[..end];
            while let Some(last) = url.chars().last() {
                if ").,!?;:'\"".contains(last) && url.len() > 8 {
                    url = &url[..url.len() - last.len_utf8()];
                } else {
                    break;
                }
            }
            if url.len() > "https://".len() {
                flush(&mut buf, out);
                out.push(Span::Link(url.to_string(), style));
                i += url.len();
                prev = Some(COVERED);
                continue;
            }
        }
        // **bold**, ~~strike~~, *italic*, _italic_
        for marker in ["**", "~~", "*", "_"] {
            if !rest.starts_with(marker) {
                continue;
            }
            let single = marker.len() == 1;
            let inner_start = marker.len();
            let Some(first) = rest[inner_start..].chars().next() else { continue };
            if first.is_whitespace() || (single && (prev.is_some_and(is_word) || prev == Some(marker.chars().next().unwrap()))) {
                continue;
            }
            // Find a closing marker preceded by non-space (and, for single markers, not followed by a word char).
            let body = &rest[inner_start..];
            let mut search = 0;
            while let Some(pos) = body[search..].find(marker) {
                let at = search + pos;
                if at == 0 {
                    search = at + marker.len();
                    continue;
                }
                let before = body[..at].chars().last().unwrap();
                let after = body[at + marker.len()..].chars().next();
                let inner = &body[..at];
                let ok = !before.is_whitespace()
                    && (!single || (!after.is_some_and(is_word) && after != marker.chars().next() && !inner.contains('\n')));
                if ok && !(single && marker == "*" && inner.starts_with('*')) {
                    flush(&mut buf, out);
                    let st = match marker {
                        "**" => Style { bold: true, ..style },
                        "~~" => Style { strike: true, ..style },
                        _ => Style { italic: true, ..style },
                    };
                    inline(inner, st, names, out);
                    i += inner_start + at + marker.len();
                    prev = marker.chars().next();
                    continue 'outer;
                }
                search = at + marker.len();
            }
        }
        let c = rest.chars().next().unwrap();
        buf.push(c);
        i += c.len_utf8();
        prev = Some(c);
    }
    flush(&mut buf, out);
}

/// The block a hidden spoiler is drawn as.
fn spoiler_color(p: &Pal) -> Color32 {
    if p.dark { Color32::from_gray(0x5a) } else { Color32::from_gray(0x9a) }
}

fn is_emoji_char(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2300..=0x23FF | 0x2B00..=0x2BFF | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x21AA)
}

/// A short all-emoji message (unicode and/or custom) renders big.
pub fn is_jumbo(blocks: &[Block]) -> bool {
    let [Block::Para(spans)] = blocks else { return false };
    let mut count = 0;
    for s in spans {
        match s {
            Span::Emoji { .. } => count += 1,
            Span::Text(t, st) if *st == Style::default() => {
                for c in t.chars() {
                    if is_emoji_char(c) {
                        count += 1;
                    } else if !(c.is_whitespace() || c == '\u{200D}' || c == '\u{FE0F}' || ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)) {
                        return false;
                    }
                }
            }
            Span::Newline => {}
            _ => return false,
        }
    }
    (1..=8).contains(&count)
}

/// `:name:` → `<:name:id>` for known custom emojis (case-insensitive), never inside code.
pub fn encode_emojis(text: &str, by_name: &HashMap<String, crate::protocol::Emoji>) -> String {
    if by_name.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut in_block = false;
    for (bi, block_part) in text.split("```").enumerate() {
        if bi > 0 {
            out.push_str("```");
            in_block = !in_block;
        }
        if in_block {
            out.push_str(block_part);
            continue;
        }
        out.push_str(&encode_inline(block_part, by_name));
    }
    out
}

/// Leave `inline code` alone (a backtick pair on one line), encode the rest.
fn encode_inline(s: &str, by_name: &HashMap<String, crate::protocol::Emoji>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(p) = s[i..].find('`') {
        let start = i + p;
        let after = &s[start + 1..];
        match after.find(['`', '\n']) {
            Some(q) if q > 0 && after.as_bytes()[q] == b'`' => {
                out.push_str(&replace_names(&s[i..start], by_name));
                out.push_str(&s[start..start + q + 2]);
                i = start + q + 2;
            }
            _ => {
                out.push_str(&replace_names(&s[i..start + 1], by_name));
                i = start + 1;
            }
        }
    }
    out.push_str(&replace_names(&s[i..], by_name));
    out
}

fn replace_names(s: &str, by_name: &HashMap<String, crate::protocol::Emoji>) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b':' && !(i > 0 && b[i - 1] == b'<') {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                j += 1;
            }
            if j < b.len() && b[j] == b':' && (2..=32).contains(&(j - start)) {
                let name = &s[start..j];
                // Not already a token's `:name:` followed by an id.
                if let Some(e) = by_name.get(&name.to_lowercase()) {
                    out.push_str(&format!("<:{}:{}>", e.name, e.id));
                    i = j + 1;
                    continue;
                }
            }
        }
        let c = s[i..].chars().next().unwrap();
        out.push(c);
        i += c.len_utf8();
    }
    out
}

pub struct Rendered {
    pub clicked_link: Option<String>,
    /// A hidden spoiler was clicked (reveal the message's spoilers).
    pub clicked_spoiler: bool,
}

struct Placed {
    emoji: Vec<(usize, String)>,
    /// Hidden spoilers (char ranges): click to reveal.
    spoilers: Vec<(usize, usize)>,
    links: Vec<(usize, usize, String)>,
    /// Quote lines (char ranges) get a bar painted at the left.
    quotes: Vec<(usize, usize)>,
}

#[allow(clippy::too_many_arguments)]
fn build_job(spans: &[Span], p: &Pal, size: f32, jumbo: bool, wrap: f32, placeholder_size: f32, emoji_px: f32, prefix: Option<(&str, Color32)>, suffix: Option<(&str, Color32)>, reveal: bool) -> (LayoutJob, Placed) {
    let mut job = LayoutJob { wrap: egui::text::TextWrapping { max_width: wrap, ..Default::default() }, ..Default::default() };
    let mut placed = Placed { emoji: vec![], spoilers: vec![], links: vec![], quotes: vec![] };
    // Hidden spoilers: a solid block (the text is there, transparent, so it wraps the same).
    let block = spoiler_color(p);
    let hide = |f: &mut TextFormat| {
        f.color = Color32::TRANSPARENT;
        f.background = block;
        f.underline = Stroke::NONE;
        f.strikethrough = Stroke::NONE;
    };
    let mut chars = 0usize;
    let text_size = if jumbo { 36.0 } else { size };
    let has_emoji = spans.iter().any(|s| matches!(s, Span::Emoji { .. }));
    let line_height = Some(if jumbo {
        emoji_px.max(text_size) + 8.0
    } else if has_emoji {
        (size * 1.4).max(emoji_px + 2.0)
    } else {
        (size * 1.4).round()
    });
    let base = |st: &Style| -> TextFormat {
        let mut f = TextFormat {
            font_id: if st.code {
                FontId::monospace(text_size * 0.88)
            } else if st.bold {
                FontId::new(text_size, theme::bold())
            } else {
                FontId::new(text_size, FontFamily::Proportional)
            },
            color: if st.quote { p.gray } else { p.text },
            italics: st.italic,
            line_height,
            valign: egui::Align::Center,
            ..Default::default()
        };
        if st.strike {
            f.strikethrough = Stroke::new(1.0, f.color);
        }
        if st.code {
            f.background = p.code_bg;
        }
        f
    };
    let mut line_start = true;
    if let Some((name, color)) = prefix {
        // AIM: the sender's name in bold, a colon, then the message.
        let f = TextFormat { font_id: FontId::new(size, theme::bold()), color, line_height, valign: egui::Align::Center, ..Default::default() };
        let t = format!("{name}: ");
        chars += t.chars().count();
        job.append(&t, 0.0, f);
    }
    for span in spans {
        match span {
            Span::Text(t, st) => {
                if st.quote && line_start {
                    // Indent; the bar is painted after layout.
                    job.append("   ", 0.0, base(st));
                    chars += 3;
                }
                let n = t.chars().count();
                if st.quote {
                    match placed.quotes.last_mut() {
                        Some(q) if q.1 == chars => q.1 = chars + n,
                        _ => placed.quotes.push((chars, chars + n)),
                    }
                }
                let mut f = base(st);
                if st.spoiler && !reveal {
                    hide(&mut f);
                    placed.spoilers.push((chars, chars + n));
                } else if st.spoiler {
                    f.background = block.gamma_multiply(0.25);
                }
                job.append(t, 0.0, f);
                chars += n;
                line_start = false;
            }
            Span::Mention(who, st) => {
                let mut f = base(&Style { bold: true, ..Style::default() });
                f.color = p.link;
                let n = who.chars().count();
                if st.spoiler && !reveal {
                    hide(&mut f);
                    placed.spoilers.push((chars, chars + n));
                }
                job.append(who, 0.0, f);
                chars += n;
                line_start = false;
            }
            Span::Link(url, st) => {
                let mut f = base(&Style::default());
                f.color = p.link;
                f.underline = Stroke::new(1.0, p.link);
                let n = url.chars().count();
                if st.spoiler && !reveal {
                    hide(&mut f);
                    placed.spoilers.push((chars, chars + n));
                } else {
                    placed.links.push((chars, chars + n, url.clone()));
                }
                job.append(url, 0.0, f);
                chars += n;
                line_start = false;
            }
            Span::Emoji { id, spoiler, .. } => {
                let mut f = TextFormat {
                    font_id: FontId::new(placeholder_size, FontFamily::Proportional),
                    color: Color32::TRANSPARENT,
                    line_height,
                    valign: egui::Align::Center,
                    ..Default::default()
                };
                if *spoiler && !reveal {
                    hide(&mut f);
                    placed.spoilers.push((chars, chars + 1));
                } else {
                    placed.emoji.push((chars, id.clone()));
                }
                job.append(&EMOJI_PLACEHOLDER.to_string(), 1.0, f);
                chars += 1;
                line_start = false;
            }
            Span::Newline => {
                job.append("\n", 0.0, base(&Style::default()));
                chars += 1;
                line_start = true;
            }
        }
    }
    if let Some((text, color)) = suffix {
        // A quiet marker after the text, e.g. "(edited)".
        let f = TextFormat { font_id: FontId::new((size - 2.0).max(10.0), FontFamily::Proportional), color, line_height, valign: egui::Align::Center, ..Default::default() };
        job.append(&format!(" {text}"), 0.0, f);
    }
    (job, placed)
}

/// Draw a message body. Returns the link the user clicked, if any.
/// `prefix`: the sender's name and colour, written inline before the first
/// paragraph (or on its own line when the message starts with code).
/// `suffix`: a small marker after the last paragraph (e.g. "(edited)").
/// `reveal`: show the message's spoilers (it was clicked).
#[allow(clippy::too_many_arguments)]
pub fn show(ui: &mut egui::Ui, blocks: &[Block], p: &Pal, images: &Images, server: &str, prefix: Option<(&str, Color32)>, suffix: Option<(&str, Color32)>, reveal: bool) -> Rendered {
    let mut result = Rendered { clicked_link: None, clicked_spoiler: false };
    let jumbo = is_jumbo(blocks);
    let emoji_px = if jumbo { 44.0 } else { 20.0 };
    let size = theme::chat();
    let mut prefix = prefix;
    if let Some((name, color)) = prefix
        && !matches!(blocks.first(), Some(Block::Para(_)))
    {
        ui.label(egui::RichText::new(format!("{name}:")).font(FontId::new(size, theme::bold())).color(color));
        prefix = None;
    }
    let last_para = blocks.iter().rposition(|b| matches!(b, Block::Para(_)));
    let suffix_after_code = !matches!(blocks.last(), Some(Block::Para(_)));
    for (bi, block) in blocks.iter().enumerate() {
        match block {
            Block::Code(code) => {
                let out = egui::Frame::new().fill(p.code_bg).corner_radius(0).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(code).font(FontId::monospace(12.0)).color(p.text)).wrap());
                });
                theme::bevel(ui.painter(), out.response.rect, p, theme::Bevel::ThinIn);
            }
            Block::Para(spans) => {
                let prefix = prefix.take();
                let wrap = ui.available_width();
                // Size the placeholder glyph so its advance ≈ the emoji width.
                let per_pt = ui.ctx().fonts_mut(|f| f.glyph_width(&FontId::new(100.0, FontFamily::Proportional), EMOJI_PLACEHOLDER)) / 100.0;
                let placeholder = if per_pt > 0.01 { (emoji_px + 2.0) / per_pt } else { emoji_px };
                let suf = if Some(bi) == last_para && !suffix_after_code { suffix } else { None };
                let (job, placed) = build_job(spans, p, size, jumbo, wrap, placeholder, emoji_px, prefix, suf, reveal);
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let (rect, resp) = ui.allocate_exact_size(galley.size(), egui::Sense::click());
                let origin = rect.min;
                ui.painter().galley(origin, galley.clone(), p.text);
                for (a, b) in &placed.quotes {
                    let top = galley.pos_from_cursor(egui::text::CCursor::new(*a));
                    let bottom = galley.pos_from_cursor(egui::text::CCursor::new(b.saturating_sub(1)));
                    let bar = |x: f32, y0: f32, y1: f32| egui::Rect::from_min_max(egui::pos2(origin.x + x + 2.0, origin.y + y0 + 2.0), egui::pos2(origin.x + x + 5.0, origin.y + y1 - 2.0));
                    // A quote right after Windows 98's "Name: " starts mid-line: its bar
                    // goes just before the quote there (not over the name), like the web's.
                    let indent = galley.pos_from_cursor(egui::text::CCursor::new(a.saturating_sub(3)));
                    if indent.min.x > 1.0 && (indent.min.y - top.min.y).abs() < 1.0 {
                        ui.painter().rect_filled(bar(indent.min.x, top.min.y, top.max.y), 0.0, p.gray);
                        if bottom.max.y > top.max.y + 1.0 {
                            ui.painter().rect_filled(bar(0.0, top.max.y + 2.0, bottom.max.y), 0.0, p.gray);
                        }
                    } else {
                        ui.painter().rect_filled(bar(0.0, top.min.y, bottom.max.y), 0.0, p.gray);
                    }
                }
                for (idx, id) in &placed.emoji {
                    let r = galley.pos_from_cursor(egui::text::CCursor::new(*idx));
                    let next = galley.pos_from_cursor(egui::text::CCursor::new(*idx + 1));
                    let w = if (next.min.y - r.min.y).abs() < 1.0 { next.min.x - r.min.x } else { emoji_px };
                    let center = egui::pos2(origin.x + r.min.x + w / 2.0, origin.y + r.center().y);
                    let target = egui::Rect::from_center_size(center, egui::vec2(emoji_px, emoji_px));
                    let url = emoji_url(server, id);
                    let hovered = ui.rect_contains_pointer(target);
                    if let Some((tex, sz)) = images.get(&url, hovered) {
                        let scale = (emoji_px / sz.x.max(sz.y)).min(emoji_px / 16.0);
                        let draw = egui::Rect::from_center_size(center, sz * scale);
                        ui.painter().image(tex, draw, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    } else {
                        ui.painter().rect_filled(target.shrink(2.0), 0.0, p.code_bg);
                    }
                }
                if !placed.spoilers.is_empty()
                    && let Some(pos) = resp.hover_pos()
                {
                    let c = galley.cursor_from_pos(pos - origin).index.0;
                    if placed.spoilers.iter().any(|(a, b)| c >= *a && c <= *b) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            result.clicked_spoiler = true;
                        }
                    }
                }
                if !placed.links.is_empty()
                    && let Some(pos) = resp.hover_pos()
                {
                    let local = pos - origin;
                    let c = galley.cursor_from_pos(local).index.0;
                    let hit = [c, c.saturating_sub(1)].into_iter().find(|&i| {
                        let a = galley.pos_from_cursor(egui::text::CCursor::new(i));
                        let b = galley.pos_from_cursor(egui::text::CCursor::new(i + 1));
                        let same_row = (a.min.y - b.min.y).abs() < 1.0;
                        local.y >= a.min.y && local.y <= a.max.y && local.x >= a.min.x && (!same_row || local.x < b.min.x)
                    });
                    if let Some(i) = hit
                        && let Some((_, _, url)) = placed.links.iter().find(|(s, e, _)| i >= *s && i < *e)
                    {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        if resp.clicked() {
                            result.clicked_link = Some(url.clone());
                        }
                        resp.on_hover_text_at_pointer(url.as_str());
                    }
                }
            }
        }
    }
    if let Some((text, color)) = suffix
        && suffix_after_code
        && !blocks.is_empty()
    {
        ui.label(egui::RichText::new(text).font(FontId::new((size - 2.0).max(10.0), FontFamily::Proportional)).color(color));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_t(s: &str) -> Vec<Block> {
        parse(s, &["sam lee".to_string(), "sam".to_string()])
    }

    /// The highlight agrees exactly with the rule (core::find_mentions) on the
    /// shared examples of test/unit.test.mjs.
    #[test]
    fn highlights_exactly_who_the_rule_mentions() {
        use crate::core::mention_tests::{PEOPLE, shared_cases};
        let mut names: Vec<String> = PEOPLE.iter().map(|(_, n)| n.to_lowercase()).collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        fn spans(b: &[Block]) -> Vec<&Span> {
            b.iter().flat_map(|b| if let Block::Para(s) = b { s.iter().collect() } else { vec![] }).collect()
        }
        let mut extra = shared_cases();
        let wave = format!("<:wave:{}>", "a".repeat(32));
        // Next to code blocks, links and emojis on the same line.
        for (body, want) in [
            ("```x```@ben".to_string(), vec![]),
            ("@ben```x```".to_string(), vec![]),
            ("```x```\n@ben".to_string(), vec!["ben"]),
            ("@ben\n```x```".to_string(), vec!["ben"]),
            ("```x``` @ben".to_string(), vec!["ben"]),
            ("**`x`**@ben".to_string(), vec!["ben"]),
            ("||`x`@ben||".to_string(), vec![]),
            ("see https://x.com/a. @jo".to_string(), vec!["jo"]),
            (format!("{wave}{wave} @jo{wave}"), vec![]),
            ("é@ben".to_string(), vec!["ben"]),
        ] {
            extra.push((body, want));
        }
        for (body, want) in extra {
            let mut highlighted: Vec<String> = spans(&parse(&body, &names))
                .into_iter()
                .filter_map(|s| match s {
                    Span::Mention(who, _) => PEOPLE.iter().find(|(_, n)| format!("@{}", n.to_lowercase()) == who.to_lowercase()).map(|(id, _)| id.to_string()),
                    _ => None,
                })
                .collect();
            highlighted.sort();
            highlighted.dedup();
            let mut rule = crate::core::find_mentions(&body, PEOPLE).0;
            rule.sort();
            let mut want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
            want.sort();
            assert_eq!(rule, want, "rule: {body:?}");
            assert_eq!(highlighted, want, "highlight: {body:?}");
        }
    }

    #[test]
    fn mentions() {
        let b = parse_t("hi @Sam Lee and @here, mail@sam.com @samuel");
        let Block::Para(spans) = &b[0] else { panic!() };
        assert!(spans.contains(&Span::Mention("@Sam Lee".into(), Style::default())));
        assert!(spans.contains(&Span::Mention("@here".into(), Style::default())));
        assert_eq!(spans.iter().filter(|s| matches!(s, Span::Mention(..))).count(), 2);
    }

    fn t(s: &str) -> Span {
        Span::Text(s.into(), Style::default())
    }

    #[test]
    fn formatting() {
        let b = parse_t("some **bold**, `code`, and a link https://example.com/x?a=1&b=2.");
        let Block::Para(spans) = &b[0] else { panic!() };
        assert_eq!(spans[0], t("some "));
        assert_eq!(spans[1], Span::Text("bold".into(), Style { bold: true, ..Style::default() }));
        assert!(spans.contains(&Span::Text("code".into(), Style { code: true, ..Style::default() })));
        assert!(spans.contains(&Span::Link("https://example.com/x?a=1&b=2".into(), Style::default())));
        assert_eq!(spans.last(), Some(&t(".")));
    }

    #[test]
    fn no_italics_inside_words() {
        let b = parse_t("snake_case_name and 2*3*4");
        assert_eq!(b, vec![Block::Para(vec![t("snake_case_name and 2*3*4")])]);
        let b = parse_t("_hi_ *there*");
        let Block::Para(spans) = &b[0] else { panic!() };
        assert_eq!(spans[0], Span::Text("hi".into(), Style { italic: true, ..Style::default() }));
    }

    #[test]
    fn code_blocks_and_emoji() {
        let id = "0123456789abcdef0123456789abcdef";
        let b = parse_t(&format!("hi <:party:{id}>\n```rust\nlet x = 1;\n```\nafter"));
        assert_eq!(b.len(), 3);
        assert_eq!(b[1], Block::Code("let x = 1;".into()));
        let Block::Para(spans) = &b[0] else { panic!() };
        assert!(spans.contains(&Span::Emoji { name: "party".into(), id: id.into(), spoiler: false }));
        assert!(is_jumbo(&parse_t(&format!("<:party:{id}> <:party:{id}>"))));
        assert!(is_jumbo(&parse_t("🎉🎉")));
        assert!(!is_jumbo(&parse_t("🎉 yay")));
    }

    #[test]
    fn encodes_known_emoji_outside_code() {
        let mut m = HashMap::new();
        m.insert("party".to_string(), crate::protocol::Emoji { id: "a".repeat(32), name: "Party".into(), mime: String::new(), uid: String::new(), created_at: 0 });
        let out = encode_emojis("yay :party: `:party:` :PARTY: :nope: ```:party:```", &m);
        let tok = format!("<:Party:{}>", "a".repeat(32));
        assert_eq!(out, format!("yay {tok} `:party:` {tok} :nope: ```:party:```"));
    }

    #[test]
    fn spoilers() {
        let sp = Style { spoiler: true, ..Style::default() };
        let b = parse_t("the end: ||he was a ghost|| and ||**bold**|| `||code||` ||open");
        let Block::Para(spans) = &b[0] else { panic!() };
        assert_eq!(spans[1], Span::Text("he was a ghost".into(), sp));
        assert_eq!(spans[3], Span::Text("bold".into(), Style { bold: true, ..sp }));
        assert!(spans.contains(&Span::Text("||code||".into(), Style { code: true, ..Style::default() })));
        assert_eq!(spans.last(), Some(&t(" ||open")));
        // Not across lines; |||| is plain.
        for plain in ["||a\nb||", "||||"] {
            let b = parse_t(plain);
            let Block::Para(spans) = &b[0] else { panic!() };
            assert!(spans.iter().all(|s| !matches!(s, Span::Text(_, st) if st.spoiler)), "{plain:?}");
        }
        assert_eq!(crate::core::hide_spoilers("x ||y|| z ||w|| `||c||` ||q"), "x ▒▒▒ z ▒▒▒ `||c||` ||q");
    }

    #[test]
    fn quotes() {
        let b = parse_t("> quoted\nplain");
        let Block::Para(spans) = &b[0] else { panic!() };
        assert_eq!(spans[0], Span::Text("quoted".into(), Style { quote: true, ..Style::default() }));
        assert_eq!(spans[1], Span::Newline);
    }
}
