//! Plain text and small pure helpers: chimes, quiet hours, spoilers,
//! YouTube links, polls, @mentions, emoji tokens, the device-side search.

use super::*;

/// One built-in chime: id, label, notes (Hz), step, length, volume, wave.
pub type ChimeDef = (&'static str, &'static str, &'static [f32], f32, f32, f32, crate::audio::Wave);
/// Built-in join chimes (PROTOCOL.md › Extras › Join chimes).
pub const CHIMES: [ChimeDef; 8] = [
    ("default", "Classic blip", &[659.25, 880.0], 0.075, 0.11, 0.05, crate::audio::Wave::Sine),
    ("ding", "Ding", &[1318.51], 0.0, 0.5, 0.08, crate::audio::Wave::Sine),
    ("chord", "Chord", &[523.25, 659.25, 783.99], 0.0, 0.6, 0.05, crate::audio::Wave::Triangle),
    ("tada", "Ta-da", &[392.0, 523.25, 659.25, 783.99, 1046.5], 0.07, 0.25, 0.06, crate::audio::Wave::Triangle),
    ("bubble", "Bubble", &[440.0, 660.0, 990.0], 0.04, 0.08, 0.06, crate::audio::Wave::Sine),
    ("knock", "Knock knock", &[160.0, 160.0], 0.16, 0.09, 0.14, crate::audio::Wave::Sine),
    ("arcade", "Arcade", &[523.25, 783.99, 1046.5, 1567.98], 0.05, 0.09, 0.04, crate::audio::Wave::Square),
    ("none", "Silent", &[], 0.0, 0.0, 0.0, crate::audio::Wave::Sine),
];

/// A built-in chime, synthesized (mono 48 kHz); None for "none" or an unknown id.
pub fn chime_samples(id: &str) -> Option<Vec<f32>> {
    let (_, _, notes, step, length, volume, wave) = CHIMES.iter().find(|c| c.0 == id)?;
    if notes.is_empty() {
        return None;
    }
    Some(crate::audio::synth(&crate::audio::Tone { notes, step: *step, length: *length, volume: *volume, wave: *wave }, crate::audio::RATE))
}

/// Hold this back for quiet hours? Like the server: a snooze (`until`) or
/// the daily window (`from` = `to`: none) is quiet time, and @mentions and
/// replies (`mention`) still come through when `mentions` is on.
pub fn quiet_now(q: &QuietHours, mention: bool, now_ms: i64, local_minutes: u32) -> bool {
    let snoozed = q.until.is_some_and(|u| u > now_ms);
    let window = q.from != q.to && if q.from < q.to { local_minutes >= q.from && local_minutes < q.to } else { local_minutes >= q.from || local_minutes < q.to };
    (snoozed || window) && !(q.mentions && mention)
}

/// `||spoilers||` as ▒▒▒ for plain-text places (notifications, quotes, search).
/// The rule every client uses: `\|\|((?:(?!\|\|)[^\n])+)\|\|` (one line, no `||`
/// inside; several per line are separate; unclosed `||` stays plain), never in code.
pub fn hide_spoilers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut in_code = false;
    while !rest.is_empty() {
        if rest.starts_with('`') {
            in_code = !in_code;
            out.push('`');
            rest = &rest[1..];
            continue;
        }
        if !in_code
            && let Some(after) = rest.strip_prefix("||")
            && let Some(end) = after.find("||")
            && end > 0
            && !after[..end].contains(['\n', '`'])
        {
            out.push_str("▒▒▒");
            rest = &after[end + 2..];
            continue;
        }
        let c = rest.chars().next().unwrap();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// The YouTube video a message links to (its first YouTube link), the same
/// rule as the web's `youtubeLinkIn`: every `http(s)://…` run up to a space or
/// `<`, trailing punctuation dropped, then watch / youtu.be / shorts / embed /
/// live / v / e links on youtube.com, youtube-nocookie.com and their www., m.
/// and music. hosts.
pub fn youtube_id(body: &str) -> Option<String> {
    let ok = |id: &str| id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let mut from = 0;
    while let Some(off) = body[from..].find("http") {
        let i = from + off;
        from = i + 4;
        if body[..i].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let Some(after) = body[i..].strip_prefix("https://").or_else(|| body[i..].strip_prefix("http://")) else { continue };
        let end = after.find(|c: char| c.is_whitespace() || c == '<').unwrap_or(after.len());
        let url = after[..end].trim_end_matches([')', '.', ',', '!', '?', ';', ':', '\'', '"']);
        if let Some(id) = youtube_link_id(url).filter(|id| ok(id)) {
            return Some(id);
        }
    }
    None
}

/// The video id in one link (without its scheme), if it's a YouTube video link.
fn youtube_link_id(url: &str) -> Option<String> {
    let (host, tail) = url.split_at(url.find(['/', '?', '#']).unwrap_or(url.len()));
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).or_else(|| host.strip_prefix("music.")).unwrap_or(&host);
    let tail = tail.split('#').next().unwrap_or("");
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
    match host {
        "youtu.be" => path.split('/').nth(1).map(str::to_string),
        "youtube.com" | "youtube-nocookie.com" if path == "/watch" => query.split('&').find_map(|kv| kv.strip_prefix("v=")).map(str::to_string),
        "youtube.com" | "youtube-nocookie.com" => {
            let mut parts = path.split('/').skip(1);
            match parts.next() {
                Some("shorts" | "embed" | "live" | "v" | "e") => parts.next().filter(|s| !s.is_empty()).map(str::to_string),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `/poll Question | A | B` → (question, options), or why not.
pub fn parse_poll(text: &str) -> Option<Result<(String, Vec<String>), &'static str>> {
    let rest = text.trim().strip_prefix("/poll")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let parts: Vec<String> = rest.split('|').map(|p| p.trim().to_string()).collect();
    let question = parts.first().cloned().unwrap_or_default();
    let options: Vec<String> = parts.into_iter().skip(1).filter(|o| !o.is_empty()).collect();
    if question.is_empty() || options.len() < 2 {
        return Some(Err("Write it as /poll Question | Option A | Option B (2–10 options)."));
    }
    if options.len() > 10 {
        return Some(Err("A poll has at most 10 options."));
    }
    if question.chars().count() > 300 || options.iter().any(|o| o.chars().count() > 80) {
        return Some(Err("Questions are up to 300 characters and options up to 80."));
    }
    let mut seen = HashSet::new();
    if !options.iter().all(|o| seen.insert(o.to_lowercase())) {
        return Some(Err("Each option has to be different."));
    }
    Some(Ok((question, options)))
}

/// `\w` as JavaScript means it (ASCII letters, digits, `_`).
pub fn js_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Cover `[from, to)` with `x` (same length, a word character on both sides,
/// like web/js/mentions.js's placeholders).
fn cover(t: &mut [char], from: usize, to: usize) {
    for c in &mut t[from..to] {
        *c = 'x';
    }
}

fn starts_at(t: &[char], i: usize, s: &str) -> bool {
    s.chars().enumerate().all(|(k, ch)| t.get(i + k) == Some(&ch))
}

/// `<:name:id>` (a custom emoji token) starting at `i`: its length in chars.
/// The web's `<:[A-Za-z0-9_]{2,32}:[a-f0-9]{32}>`.
pub fn emoji_token_len(t: &[char], i: usize) -> Option<usize> {
    if t.get(i) != Some(&'<') || t.get(i + 1) != Some(&':') {
        return None;
    }
    let name = t[i + 2..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '_').count();
    if !(2..=32).contains(&name) || t.get(i + 2 + name) != Some(&':') {
        return None;
    }
    let id = i + 3 + name;
    let hex = t.get(id..id + 32)?;
    (hex.iter().all(|c| c.is_ascii_digit() || ('a'..='f').contains(c)) && t.get(id + 32) == Some(&'>')).then_some(3 + name + 33)
}

/// Who a message @-mentions, by the one rule every client and the server use
/// (web/js/mentions.js): a member's name in any case, longest names first and
/// each mention used once (so "@Sam Lee" is Sam Lee, never also Sam), not
/// right after a letter, digit, `_` or `@` (no e-mail addresses), never
/// inside code, links or custom emoji tokens. Those are covered up first and
/// count as a word, so one right against the name blocks it ("`x`@Ben" and
/// "<:wave:…>@Ben" mention nobody, "<:wave:…> @Ben" mentions Ben). The
/// highlighter (`ui::markup`) follows the same rule. Returns (uids
/// mentioned, whether it says @everyone / @here).
pub fn find_mentions<'a>(body: &str, users: impl IntoIterator<Item = (&'a str, &'a str)>) -> (Vec<String>, bool) {
    let mut t: Vec<char> = body.chars().collect();
    // Code blocks ```…```, then `code`, then custom emojis, then links: covered up.
    let mut i = 0;
    while i + 3 <= t.len() {
        if starts_at(&t, i, "```")
            && let Some(end) = (i + 3..t.len().saturating_sub(2)).find(|&k| starts_at(&t, k, "```"))
        {
            cover(&mut t, i, end + 3);
            i = end + 3;
            continue;
        }
        i += 1;
    }
    let mut i = 0;
    while i < t.len() {
        if t[i] == '`'
            && let Some(len) = t[i + 1..].iter().position(|&c| c == '`' || c == '\n').filter(|&n| n > 0 && t[i + 1 + n] == '`')
        {
            cover(&mut t, i, i + len + 2);
            i += len + 2;
            continue;
        }
        i += 1;
    }
    let mut i = 0;
    while i < t.len() {
        if let Some(len) = emoji_token_len(&t, i) {
            cover(&mut t, i, i + len);
            i += len;
            continue;
        }
        i += 1;
    }
    let mut i = 0;
    while i < t.len() {
        let boundary = i == 0 || !js_word(t[i - 1]);
        let scheme = if starts_at(&t, i, "https://") { 8 } else if starts_at(&t, i, "http://") { 7 } else { 0 };
        if boundary && scheme > 0 {
            let rest = t[i + scheme..].iter().take_while(|c| !c.is_whitespace() && **c != '<').count();
            if rest > 0 {
                cover(&mut t, i, i + scheme + rest);
                i += scheme + rest;
                continue;
            }
        }
        i += 1;
    }
    let mut text: Vec<char> = t.iter().collect::<String>().to_lowercase().chars().collect();
    let at = |text: &[char], needle: &[char]| -> Vec<usize> {
        if needle.is_empty() || text.len() < needle.len() {
            return Vec::new();
        }
        (0..=text.len() - needle.len())
            .filter(|&i| {
                text[i..i + needle.len()] == *needle
                    && (i == 0 || !(js_word(text[i - 1]) || text[i - 1] == '@'))
                    && text.get(i + needle.len()).is_none_or(|c| !js_word(*c))
            })
            .collect()
    };
    let everyone = !at(&text, &"@everyone".chars().collect::<Vec<_>>()).is_empty() || !at(&text, &"@here".chars().collect::<Vec<_>>()).is_empty();
    let mut people: Vec<(&str, &str)> = users.into_iter().filter(|(_, name)| !name.is_empty()).collect();
    people.sort_by_key(|(_, name)| std::cmp::Reverse(name.chars().count()));
    let mut uids = Vec::new();
    for (id, name) in people {
        let needle: Vec<char> = format!("@{}", name.to_lowercase()).chars().collect();
        let found = at(&text, &needle);
        if found.is_empty() {
            continue;
        }
        uids.push(id.to_string());
        // Taken: a shorter name can't match inside these any more.
        for i in found {
            cover(&mut text, i, i + needle.len());
        }
    }
    (uids, everyone)
}

/// A message as plain text, for every place that shows a snippet of one —
/// reply quotes, the reply bar, pins, search, Catch up and notifications —
/// like the web's `plainText` (web/js/format.js): the formatting marks go
/// (`**bold**`, `__bold__`, `*italic*`, `_italic_`, `~~strike~~`, `` `code` ``,
/// ```` ``` ```` blocks, "> "), spoilers become ▒▒▒ and custom emojis their
/// `:name:`. Code and links are kept as written (an underscore in a link isn't
/// italics). `line`: folded onto one line.
pub fn plain_text(text: &str, line: bool) -> String {
    // Code, custom emojis and links are set aside first (private-use markers
    // around a slot number), so nothing below touches them.
    let t: Vec<char> = text.chars().collect();
    let mut slots: Vec<String> = vec![];
    let mut s: Vec<char> = Vec::with_capacity(t.len());
    let mut keep = |s: &mut Vec<char>, kept: String| {
        s.push('\u{e000}');
        s.extend(slots.len().to_string().chars());
        s.push('\u{e001}');
        slots.push(kept);
    };
    let mut i = 0;
    while i < t.len() {
        if starts_at(&t, i, "```")
            && let Some(end) = (i + 3..t.len().saturating_sub(2)).find(|&j| starts_at(&t, j, "```"))
        {
            // ```lang\n…```: the language tag and the newlines at either end go.
            let mut from = i + 3;
            let tag = t[from..end].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '_' || **c == '+' || **c == '-').count();
            if t.get(from + tag) == Some(&'\n') {
                from += tag;
            }
            let code: String = t[from..end].iter().collect();
            keep(&mut s, code.trim_matches('\n').to_string());
            i = end + 3;
            continue;
        }
        if t[i] == '`'
            && let Some(len) = t[i + 1..].iter().position(|c| *c == '`' || *c == '\n')
            && len > 0
            && t[i + 1 + len] == '`'
        {
            keep(&mut s, t[i + 1..i + 1 + len].iter().collect());
            i += len + 2;
            continue;
        }
        if let Some(len) = emoji_token_len(&t, i) {
            let name: String = t[i + 2..].iter().take_while(|c| **c != ':').collect();
            keep(&mut s, format!(":{name}:"));
            i += len;
            continue;
        }
        if (starts_at(&t, i, "http://") || starts_at(&t, i, "https://")) && (i == 0 || !js_word(t[i - 1])) {
            let len = t[i..].iter().take_while(|c| !c.is_whitespace() && **c != '<').count();
            keep(&mut s, t[i..i + len].iter().collect());
            i += len;
            continue;
        }
        s.push(t[i]);
        i += 1;
    }
    let s: Vec<char> = hide_spoilers(&s.iter().collect::<String>()).chars().collect();
    let s = unwrap_pair(&s, "**");
    let s = unwrap_pair(&s, "__");
    let s = unwrap_single(&s, '*');
    let s = unwrap_single(&s, '_');
    let s = unwrap_pair(&s, "~~");
    // "> " quote marks at the start of each line.
    let mut out = String::with_capacity(s.len());
    for (k, l) in s.iter().collect::<String>().split('\n').enumerate() {
        if k > 0 {
            out.push('\n');
        }
        out.push_str(l.strip_prefix('>').map(|r| r.strip_prefix(' ').unwrap_or(r)).unwrap_or(l));
    }
    if line {
        out = out.split('\n').map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        out = out.trim().to_string();
    }
    // The slots back.
    let mut done = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(a) = rest.find('\u{e000}') {
        done.push_str(&rest[..a]);
        let tail = &rest[a + '\u{e000}'.len_utf8()..];
        match tail.find('\u{e001}').and_then(|b| tail[..b].parse::<usize>().ok().map(|n| (b, n))) {
            Some((b, n)) => {
                done.push_str(slots.get(n).map(String::as_str).unwrap_or(""));
                rest = &tail[b + '\u{e001}'.len_utf8()..];
            }
            None => rest = tail,
        }
    }
    done.push_str(rest);
    done
}

/// At most `n` characters, cut at a word where there's one nearby, with one
/// "…" (never "end.…") — the web's `clip()` (web/js/format.js).
pub fn clip(text: &str, n: usize) -> String {
    let t: Vec<char> = text.chars().collect();
    if t.len() <= n {
        return text.to_string();
    }
    let cut = &t[..n];
    // A cut that ends a word keeps it; else back to the last space, if it's not too far.
    let space = if t[n].is_whitespace() { Some(n) } else { cut.iter().rposition(|c| *c == ' ') };
    let keep = match space {
        Some(s) if s as f32 > n as f32 * 0.6 => &cut[..s],
        _ => cut,
    };
    let mut out: String = keep.iter().collect();
    let trimmed = out.trim_end_matches(|c: char| c.is_whitespace() || ".,;:!?…—-".contains(c)).len();
    out.truncate(trimmed);
    out.push('…');
    out
}

/// `**x**` / `__x__` / `~~x~~` → x (x starts and ends with a non-space; may span lines).
fn unwrap_pair(t: &[char], mark: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        if starts_at(t, i, mark)
            && t.get(i + 2).is_some_and(|c| !c.is_whitespace())
            && let Some(end) = (i + 3..t.len()).find(|&j| starts_at(t, j, mark) && !t[j - 1].is_whitespace())
        {
            out.extend_from_slice(&t[i + 2..end]);
            i = end + 2;
            continue;
        }
        out.push(t[i]);
        i += 1;
    }
    out
}

/// `*x*` / `_x_` → x: not inside a word or next to another mark, one line, x
/// starts and ends with a non-space (the web's rule).
fn unwrap_single(t: &[char], mark: char) -> Vec<char> {
    let mut out: Vec<char> = Vec::with_capacity(t.len());
    let mut i = 0;
    while i < t.len() {
        let before_ok = i == 0 || (t[i - 1] != mark && !js_word(t[i - 1]));
        if t[i] == mark
            && before_ok
            && t.get(i + 1).is_some_and(|c| !c.is_whitespace() && *c != mark)
            && let Some(len) = t[i + 1..].iter().position(|c| *c == mark || *c == '\n')
            && t[i + 1 + len] == mark
            && !t[i + len].is_whitespace()
            && t.get(i + 2 + len).is_none_or(|c| *c != mark && !js_word(*c))
        {
            out.extend_from_slice(&t[i + 1..i + 1 + len]);
            i += len + 2;
            continue;
        }
        out.push(t[i]);
        i += 1;
    }
    out
}

/// `<:name:id>` tokens back to `:name:` (notifications, previews).
pub fn plain_emojis(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("<:") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 2..];
        match tail.find(':').zip(tail.find('>')) {
            Some((c, e)) if c < e && e - c == 33 => {
                out.push(':');
                out.push_str(&tail[..c]);
                out.push(':');
                rest = &tail[e + 1..];
            }
            _ => {
                out.push_str("<:");
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The query's words / `from:` / `has:` against the Personal space's
/// encrypted notes, opened with this device's key (newest first).
pub fn local_matches(m: &Model, channel_id: &str, q: &str) -> Vec<Message> {
    let Some(key) = m.vault_key else { return vec![] };
    let Some(h) = m.history.get(channel_id) else { return vec![] };
    let mut words: Vec<String> = vec![];
    let mut has: Vec<String> = vec![];
    let mut from_ok = true;
    for token in query_tokens(q) {
        let lower = token.to_lowercase();
        if let Some(name) = lower.strip_prefix("from:") {
            from_ok &= m.config.name.to_lowercase().starts_with(name.trim_matches('"'));
        } else if let Some(what) = lower.strip_prefix("has:") {
            has.push(what.to_string());
        } else {
            let w = lower.trim_matches('"').to_string();
            if !w.is_empty() {
                words.push(w);
            }
        }
    }
    if !from_ok {
        return vec![];
    }
    let mut out: Vec<Message> = h
        .messages
        .iter()
        .filter(|x| crate::vault::is_note(&x.body))
        .filter(|x| {
            let Ok(note) = crate::vault::decrypt_note(&key, &x.body) else { return false };
            let text = note.text.to_lowercase();
            words.iter().all(|w| text.contains(w.as_str()))
                && has.iter().all(|what| {
                    if what.starts_with("image") || what.starts_with("picture") {
                        note.image.is_some()
                    } else if what.starts_with("gif") {
                        note.gif.is_some()
                    } else if what.starts_with("link") {
                        text.contains("http")
                    } else {
                        true
                    }
                })
        })
        .cloned()
        .collect();
    out.reverse();
    out
}

/// Split a query into tokens; "quoted phrases" stay together.
pub fn query_tokens(q: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for ch in q.chars() {
        if ch == '"' {
            quoted = !quoted;
            cur.push(ch);
        } else if ch.is_whitespace() && !quoted {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod extras_tests {
    use super::*;

    #[test]
    fn youtube_links() {
        assert_eq!(youtube_id("watch https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://youtu.be/dQw4w9WgXcQ?si=x").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://m.youtube.com/shorts/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://music.youtube.com/watch?feature=x&v=dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://youtube.com/live/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(youtube_id("https://youtu.be/short"), None);
        assert_eq!(youtube_id("https://example.com/watch?v=dQw4w9WgXcQ"), None);
        assert_eq!(youtube_id("(see https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ)."), Some("dQw4w9WgXcQ".into()));
        assert_eq!(youtube_id("first https://example.com then https://youtu.be/dQw4w9WgXcQ!"), Some("dQw4w9WgXcQ".into()));
        assert_eq!(youtube_id("nohttps://youtu.be/dQw4w9WgXcQ"), None);
        assert_eq!(youtube_id("https://youtube.com/watchx?v=dQw4w9WgXcQ"), None);
    }

    #[test]
    fn polls_parse() {
        assert_eq!(parse_poll("/poll Pizza? | yes | no").unwrap().unwrap(), ("Pizza?".to_string(), vec!["yes".to_string(), "no".to_string()]));
        assert!(parse_poll("/poll just a question").unwrap().is_err());
        assert!(parse_poll("/poll Q | a | A").unwrap().is_err(), "duplicates");
        assert!(parse_poll("/pollution is bad").is_none());
        assert!(parse_poll("hello").is_none());
    }

    #[test]
    fn quiet_hours() {
        let q = QuietHours { from: 23 * 60, to: 8 * 60, tz: "UTC".into(), mentions: true, until: None };
        assert!(quiet_now(&q, false, 0, 23 * 60 + 30));
        assert!(quiet_now(&q, false, 0, 7 * 60));
        assert!(!quiet_now(&q, false, 0, 12 * 60));
        assert!(!quiet_now(&q, true, 0, 2 * 60), "mentions come through");
        let snooze = QuietHours { from: 0, to: 0, tz: "UTC".into(), mentions: false, until: Some(1000) };
        assert!(quiet_now(&snooze, true, 500, 12 * 60));
        assert!(!quiet_now(&snooze, true, 2000, 12 * 60));
    }

    #[test]
    fn chimes_synthesize() {
        assert!(chime_samples("tada").is_some_and(|s| s.len() > 10_000));
        assert!(chime_samples("none").is_none());
        assert_eq!(CHIMES.len(), 8);
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn query_tokens_keep_phrases() {
        assert_eq!(query_tokens(r#"pizza from:ad "movie night"  has:image"#), vec!["pizza", "from:ad", "\"movie night\"", "has:image"]);
    }

    #[test]
    fn encrypted_notes_match_on_the_device() {
        let shared = Shared::new(crate::config::Config { name: "Ada".into(), ..Default::default() }, std::env::temp_dir().join("yap-search-test.json"), Arc::new(|| {}));
        let (_, key) = crate::vault::create("purple tiger river", 1000);
        let mut m = shared.model.lock();
        m.vault_key = Some(key);
        let note = |id: &str, text: &str, image: bool| Message {
            id: id.into(),
            channel_id: "p_u".into(),
            uid: "u".into(),
            body: crate::vault::encrypt_note(&key, &crate::vault::Note { text: text.into(), gif: None, image: image.then(|| crate::vault::NoteImage { id: "x".into(), width: 1.0, height: 1.0, mime: "image/png".into() }) }),
            ts: 1,
            gif: None,
            image: None,
            reactions: vec![],
            edited_at: None,
            reply: None,
            pinned_at: None,
            poll: None,
            roll: None,
            reply_to: None,
            pending: false,
        };
        m.history.insert("p_u".into(), History { messages: vec![note("a", "Secret Pizza recipe", false), note("b", "tacos", true), note("c", "pizza pic", true)], has_more: false, loading: false, older: false });
        let ids = |q: &str| local_matches(&m, "p_u", q).into_iter().map(|x| x.id).collect::<Vec<_>>();
        assert_eq!(ids("pizza"), vec!["c", "a"]);
        assert_eq!(ids("pizza has:image"), vec!["c"]);
        assert_eq!(ids("\"pizza recipe\""), vec!["a"]);
        assert_eq!(ids("pizza from:ad"), vec!["c", "a"]);
        assert!(ids("pizza from:bob").is_empty());
    }
}

#[cfg(test)]
pub(crate) mod mention_tests {
    use super::*;

    pub(crate) const PEOPLE: [(&str, &str); 4] = [("sam", "Sam"), ("samlee", "Sam Lee"), ("ben", "Ben"), ("jo", "Jo")];

    fn who(body: &str) -> Vec<String> {
        let mut v = find_mentions(body, PEOPLE).0;
        v.sort();
        v
    }

    /// The examples of test/unit.test.mjs (one rule: web/js/mentions.js).
    pub(crate) fn shared_cases() -> Vec<(String, Vec<&'static str>)> {
        let wave = format!("<:wave:{}>", "a".repeat(32));
        let mut cases: Vec<(String, Vec<&'static str>)> = SHARED.iter().map(|(b, w)| (b.to_string(), w.to_vec())).collect();
        // Custom emojis are covered up like code: one right against the @ makes it no mention (on both sides).
        cases.push((format!("{wave}@ben"), vec![]));
        cases.push((format!("@ben{wave}"), vec![]));
        cases.push((format!("{wave} @ben"), vec!["ben"]));
        cases.push((format!("@ben {wave}"), vec!["ben"]));
        cases
    }

    const SHARED: [(&str, &[&str]); 23] = [
        ("@Sam Lee hi", &["samlee"]),
        ("@sam lee and @SAM", &["samlee", "sam"]),
        ("@Sam Lee, @Sam Lee!", &["samlee"]),
        ("@Sam Leeds", &["sam"]),
        ("hi @ben!", &["ben"]),
        ("(@Ben)", &["ben"]),
        ("@ben\n@jo", &["ben", "jo"]),
        ("@Ben@Jo", &["ben"]),
        ("mail sam@ben.com", &[]),
        ("x@ben", &[]),
        ("@@ben", &[]),
        ("@bento", &[]),
        ("@ben_x", &[]),
        ("@ben2", &[]),
        ("`@ben` in code", &[]),
        ("```\n@ben\n```", &[]),
        ("https://youtube.com/@Ben", &[]),
        ("see https://x.com/a then @jo", &["jo"]),
        ("no one", &[]),
        ("`x`@ben", &[]),
        ("@ben`x`", &[]),
        ("`x` @ben", &["ben"]),
        ("☀️@ben", &["ben"]),
    ];

    #[test]
    fn the_shared_rule() {
        for (body, want) in shared_cases() {
            let mut want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
            want.sort();
            assert_eq!(who(&body), want, "{body:?}");
        }
        for (body, everyone) in [("@everyone game time", true), ("@here?", true), ("me@everyone.com", false), ("`@everyone`", false), ("@everyones", false)] {
            assert_eq!(find_mentions(body, PEOPLE).1, everyone, "{body:?}");
        }
    }

    #[test]
    fn custom_emoji_tokens_are_covered_like_code() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(emoji_token_len(&format!("<:wave:{id}>").chars().collect::<Vec<_>>(), 0), Some(40), "<:wave: + 32 + >");
        // Not a token: a short name, a short or upper-case id, no closing >.
        for bad in [format!("<:w:{id}>"), format!("<:wave:{}>", &id[1..]), format!("<:wave:{}>", id.to_uppercase()), format!("<:wave:{id}")] {
            assert_eq!(emoji_token_len(&bad.chars().collect::<Vec<_>>(), 0), None, "{bad}");
        }
        // A name inside a token is never a mention.
        assert!(who(&format!("<:ben:{id}>")).is_empty());
        assert_eq!(who(&format!("<:wave:{id}> hi @jo<:wave:{id}> and @ben!")), vec!["ben".to_string()]);
    }

    #[test]
    fn longest_name_first_and_used_once() {
        // "@Sam Lee" is Sam Lee, never also Sam (the bug this rule fixed).
        assert_eq!(who("hi @Sam Lee!"), vec!["samlee".to_string()]);
        assert_eq!(who("mail a@bob.com"), Vec::<String>::new());
    }
}

#[cfg(test)]
mod plain_tests {
    use super::plain_text;

    /// The web's own examples (test/unit.test.mjs › plainText).
    #[test]
    fn snippets_drop_the_formatting_marks_like_the_web() {
        assert_eq!(plain_text("who's around tonight? thinking **Overcooked** around 9 🍳", false), "who's around tonight? thinking Overcooked around 9 🍳");
        assert_eq!(plain_text("__bold__ *it* _it_ ~~gone~~ `x_y` and ```js\nconst a_b = 1\n```", false), "bold it it gone x_y and const a_b = 1");
        assert_eq!(plain_text("> quoted\nreply", false), "quoted\nreply");
        assert_eq!(plain_text("> quoted\nreply", true), "quoted reply");
        assert_eq!(plain_text("see https://a.com/x_y_z and _this_", false), "see https://a.com/x_y_z and this");
        assert_eq!(plain_text("the ending: ||the dog lives|| 🐶", false), "the ending: ▒▒▒ 🐶");
        assert_eq!(plain_text(&format!("gets cold <:parrot:{}>", "a".repeat(32)), false), "gets cold :parrot:");
        assert_eq!(plain_text("2 * 3 * 4 = 24, file_name_here", false), "2 * 3 * 4 = 24, file_name_here");
        assert_eq!(plain_text("", false), "");
        // Spoilers inside code stay code; a lone mark stays.
        assert_eq!(plain_text("`||c||` and **", false), "||c|| and **");
        assert_eq!(plain_text("@Sam you coming? bring the _good_ snacks", true), "@Sam you coming? bring the good snacks");
    }

    /// The web's own examples (test/unit.test.mjs › clip).
    #[test]
    fn clip_cuts_at_a_word_with_one_ellipsis() {
        use super::clip;
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("the cafe at the far end. Then back", 24), "the cafe at the far end…");
        assert_eq!(clip("supercalifragilistic", 10), "supercalif…");
    }
}
