//! Times and days, in the user's locale's 12- or 24-hour clock.

pub fn local_time(ts: i64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ts).single().unwrap_or_else(chrono::Local::now)
}

pub fn fmt_time(ts: i64, h12: bool) -> String {
    let t = local_time(ts);
    if h12 { t.format("%-I:%M %p").to_string() } else { t.format("%H:%M").to_string() }
}

pub fn fmt_stamp(ts: i64, h12: bool) -> String {
    let t = local_time(ts).date_naive();
    let today = chrono::Local::now().date_naive();
    let time = fmt_time(ts, h12);
    if t == today {
        format!("Today at {time}")
    } else if Some(t) == today.pred_opt() {
        format!("Yesterday at {time}")
    } else {
        format!("{}, {time}", local_time(ts).format("%b %-d, %Y"))
    }
}

/// A result's "when", like the web's: the time today, else "Oct 6, 7:19 PM"
/// ("Oct 6, 2025" from another year).
pub fn fmt_short(ts: i64, h12: bool) -> String {
    let t = local_time(ts);
    let now = chrono::Local::now();
    if t.date_naive() == now.date_naive() {
        fmt_time(ts, h12)
    } else if chrono::Datelike::year(&t) == chrono::Datelike::year(&now) {
        format!("{}, {}", t.format("%b %-d"), fmt_time(ts, h12))
    } else {
        t.format("%b %-d, %Y").to_string()
    }
}

pub fn fmt_day(ts: i64) -> String {
    local_time(ts).format("%A, %B %-d, %Y").to_string()
}

pub(super) fn uses_12h() -> bool {
    let loc = sys_locale::get_locale().unwrap_or_else(|| "en-US".into()).replace('_', "-");
    ["en-US", "en-CA", "en-AU", "en-NZ", "en-PH", "en-IN", "es-US", "es-MX", "en"].iter().any(|l| loc == *l || loc.starts_with(&format!("{l}-")) && l.len() > 2)
}
