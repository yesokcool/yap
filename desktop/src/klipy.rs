//! KLIPY API client (PROTOCOL.md › GIFs): called straight from this app, never
//! through the Yap server; media URLs are used exactly as returned.
//! `YAP_KLIPY_API` overrides the API base (tests point it at a local mock).

use crate::protocol::Gif;
use sha2::Digest;

pub const PER_PAGE: u32 = 24;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Trending,
    Recent,
    Search,
}

pub fn api_base() -> String {
    std::env::var("YAP_KLIPY_API").unwrap_or_else(|_| "https://api.klipy.com/api/v1".into())
}

/// First 16 bytes (hex) of SHA-256("yap:" + uid): stable and anonymous, same on every client.
pub fn customer_id(uid: &str) -> String {
    let d = sha2::Sha256::digest(format!("yap:{uid}").as_bytes());
    d[..16].iter().map(|b| format!("{b:02x}")).collect()
}

fn locale() -> String {
    sys_locale::get_locale().and_then(|l| l.replace('_', "-").split('-').nth(1).map(str::to_lowercase)).unwrap_or_default()
}

/// KLIPY item → the Gif we put in a message (medium size, URLs untouched).
pub fn to_gif(item: &serde_json::Value) -> Option<Gif> {
    let file = item.get("file")?;
    let md = file.get("md").or_else(|| file.get("hd"))?;
    let size = md.get("mp4").or_else(|| md.get("webp")).or_else(|| md.get("gif"))?;
    let url = |k: &str| md.get(k).and_then(|x| x.get("url")).and_then(|u| u.as_str()).map(str::to_string);
    Some(Gif {
        provider: "klipy".into(),
        slug: item.get("slug")?.as_str()?.to_string(),
        title: item.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        width: size.get("width")?.as_f64()? as f32,
        height: size.get("height")?.as_f64()? as f32,
        mp4: url("mp4"),
        webm: url("webm"),
        webp: url("webp"),
        gif: url("gif"),
        still: url("jpg"),
    })
}

/// One page of results (blocking). Returns (gifs, has_next).
pub fn fetch(key: &str, uid: &str, mode: Mode, query: &str, page: u32) -> Result<(Vec<Gif>, bool), String> {
    let cid = customer_id(uid);
    let base = format!("{}/{}/gifs", api_base(), crate::config::percent_encode(key));
    let url = match mode {
        Mode::Recent => format!("{base}/recent/{cid}?page={page}&per_page={PER_PAGE}"),
        Mode::Search => format!(
            "{base}/search?q={}&page={page}&per_page={PER_PAGE}&customer_id={cid}&locale={}",
            crate::config::percent_encode(query),
            locale()
        ),
        Mode::Trending => format!("{base}/trending?page={page}&per_page={PER_PAGE}&customer_id={cid}&locale={}", locale()),
    };
    let v = crate::http::get_json(&url)?;
    let data = v.get("data").cloned().unwrap_or_default();
    let gifs = data.get("data").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(to_gif).collect()).unwrap_or_default();
    Ok((gifs, data.get("has_next").and_then(|x| x.as_bool()).unwrap_or(false)))
}

/// Tell KLIPY a GIF was shared (feeds "Recent" and their ranking). Blocking.
pub fn report_share(key: &str, uid: &str, slug: &str, query: &str) {
    let mut body = serde_json::json!({ "customer_id": customer_id(uid) });
    if !query.is_empty() {
        body["q"] = serde_json::json!(query);
    }
    let url = format!("{}/{}/gifs/share/{}", api_base(), crate::config::percent_encode(key), crate::config::percent_encode(slug));
    let _ = crate::http::post_json(&url, &body);
}

#[cfg(test)]
mod tests {
    #[test]
    fn customer_id_matches_web() {
        // Web: first 16 bytes hex of SHA-256("yap:" + uid).
        assert_eq!(super::customer_id("abc"), "2a7273c0bd6a463eafa90e9f50093331");
        let item = serde_json::json!({ "slug": "cats-0", "title": "cats 0", "file": { "md": {
            "mp4": { "url": "https://static.klipy.com/a.mp4", "width": 498, "height": 280 },
            "webp": { "url": "https://static.klipy.com/a.webp", "width": 498, "height": 280 },
            "jpg": { "url": "https://static.klipy.com/a.jpg", "width": 498, "height": 280 } } } });
        let g = super::to_gif(&item).unwrap();
        assert_eq!((g.slug.as_str(), g.width, g.height), ("cats-0", 498.0, 280.0));
        assert_eq!(g.still.as_deref(), Some("https://static.klipy.com/a.jpg"));
        assert!(g.gif.is_none());
    }
}
