//! The few plain HTTP calls (blocking; run them on a worker thread).

use crate::config::percent_encode;
use serde::Deserialize;
use std::time::Duration;

/// GET with the invite key in the `X-Yap-Key` header (never in the URL).
fn get(url: &str, key: &str) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let req = agent().get(url);
    if key.is_empty() { req.call() } else { req.header("X-Yap-Key", key).call() }
}

/// Middleware for every HTTP agent in the app: plain `http://` only to this
/// computer (`config::insecure`). (Redirects are followed inside it; only the
/// server we already trust with the key could send one.)
pub fn secure_only(req: ureq::http::Request<ureq::SendBody>, next: ureq::middleware::MiddlewareNext) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let uri = req.uri();
    if uri.scheme_str() != Some("https") && !uri.host().is_some_and(crate::config::is_loopback) {
        return Err(ureq::Error::RequireHttpsOnly(format!("{uri} (plain http:// only works for a server on this computer)")));
    }
    next.handle(req)
}

/// The settings every agent here shares: `timeout`, the server's error pages
/// read as answers, our user agent, and `secure_only`.
pub fn agent_config(timeout: Duration) -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .user_agent(concat!("yap-desktop/", env!("CARGO_PKG_VERSION")))
        .middleware(secure_only)
}

fn agent() -> ureq::Agent {
    agent_config(Duration::from_secs(10)).build().into()
}

/// `/api/check`'s answer; only `ok` matters here (the reply's `keyRequired`
/// and `protocol` aren't read, so they aren't kept).
#[derive(Deserialize, Debug, Clone)]
pub struct CheckResult {
    pub ok: bool,
}

/// `GET /api/check` — tells "wrong key" apart from "server down".
pub fn check(server: &str, key: &str) -> Result<CheckResult, String> {
    let mut res = get(&format!("{server}/api/check"), key).map_err(|e| format!("Can't reach {server}: {e}"))?;
    if res.status() != 200 {
        return Err(format!("{server} answered HTTP {}", res.status()));
    }
    res.body_mut()
        .read_json::<CheckResult>()
        .map_err(|_| format!("{server} doesn't look like a Yap server."))
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Urls {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize, Debug, Clone)]
pub struct IceServer {
    pub urls: Urls,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub credential: Option<String>,
}

impl IceServer {
    pub fn url_list(&self) -> Vec<String> {
        match &self.urls {
            Urls::One(u) => vec![u.clone()],
            Urls::Many(v) => v.clone(),
        }
    }
}

/// Does this list have a TURN relay this app can use (`turn:`; the stack
/// can't do `turns:`)? Relay-only voice needs one.
pub fn has_relay(ice: &[IceServer]) -> bool {
    ice.iter().any(|s| s.url_list().iter().any(|u| u.starts_with("turn:")))
}

#[derive(Deserialize)]
struct IceResponse {
    #[serde(rename = "iceServers")]
    ice_servers: Vec<IceServer>,
}

/// `GET /api/ice` — STUN always, short-lived TURN when the server has a key;
/// `ice_fallback` when it can't be asked.
pub fn ice(server: &str, key: &str) -> Vec<IceServer> {
    fetch_ice(server, key).unwrap_or_else(|_| ice_fallback())
}

/// A public STUN server, for when the server can't be asked (never a relay).
pub fn ice_fallback() -> Vec<IceServer> {
    vec![IceServer {
        urls: Urls::One("stun:stun.cloudflare.com:3478".into()),
        username: None,
        credential: None,
    }]
}

/// `GET /api/ice` as the server answers it, or why it couldn't be asked (Hide
/// my IP needs to tell "no relay" from "couldn't ask").
pub fn fetch_ice(server: &str, key: &str) -> Result<Vec<IceServer>, String> {
    let mut res = get(&format!("{server}/api/ice"), key).map_err(|e| e.to_string())?;
    if res.status() != 200 {
        return Err(format!("HTTP {}", res.status()));
    }
    res.body_mut().read_json::<IceResponse>().map(|r| r.ice_servers).map_err(|e| e.to_string())
}

#[derive(Deserialize)]
struct UploadResponse {
    ok: bool,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    emoji: Option<crate::protocol::Emoji>,
}

/// `POST /api/emoji?sid&name` (key in `X-Yap-Key`) with the raw image bytes.
/// `ut` = this connection's `ready.uploadToken` (session ids alone are public).
pub fn upload_emoji(server: &str, key: &str, sid: &str, ut: &str, name: &str, bytes: Vec<u8>) -> Result<crate::protocol::Emoji, String> {
    let url = format!("{server}/api/emoji?sid={}&ut={}&name={}", percent_encode(sid), percent_encode(ut), percent_encode(name));
    let mut req = agent().post(&url).header("Content-Type", "application/octet-stream");
    if !key.is_empty() {
        req = req.header("X-Yap-Key", key);
    }
    let mut res = req
        .send(&bytes[..])
        .map_err(|e| format!("Upload failed: {e}"))?;
    let status = res.status();
    let body: UploadResponse = res
        .body_mut()
        .read_json()
        .map_err(|_| format!("Upload failed (HTTP {status})."))?;
    if !body.ok {
        return Err(body.message.unwrap_or_else(|| "Upload failed.".into()));
    }
    body.emoji.ok_or_else(|| "Upload failed.".into())
}

#[derive(Deserialize)]
struct ImageResponse {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

/// `POST /api/image?sid&ut` (key in `X-Yap-Key`) with the picture's bytes → its media id.
pub fn upload_image(server: &str, key: &str, sid: &str, ut: &str, bytes: Vec<u8>, mime: &str) -> Result<String, String> {
    let url = format!("{server}/api/image?sid={}&ut={}", percent_encode(sid), percent_encode(ut));
    let mut req = agent_config(Duration::from_secs(120))
        .build()
        .new_agent()
        .post(&url)
        .header("Content-Type", mime);
    if !key.is_empty() {
        req = req.header("X-Yap-Key", key);
    }
    let mut res = req.send(&bytes[..]).map_err(|e| format!("upload failed: {e}"))?;
    let status = res.status();
    let body: ImageResponse = res.body_mut().read_json().map_err(|_| format!("upload failed (HTTP {status})"))?;
    match (body.ok, body.id) {
        (true, Some(id)) => Ok(id),
        _ => Err(body.message.unwrap_or_else(|| format!("upload failed (HTTP {status})"))),
    }
}

/// `POST /api/blob?sid&ut`: an encrypted picture (opaque bytes) → its media id.
pub fn upload_blob(server: &str, key: &str, sid: &str, ut: &str, bytes: Vec<u8>) -> Result<String, String> {
    let url = format!("{server}/api/blob?sid={}&ut={}", percent_encode(sid), percent_encode(ut));
    let mut req = agent_config(Duration::from_secs(120))
        .build()
        .new_agent()
        .post(&url)
        .header("Content-Type", "application/octet-stream");
    if !key.is_empty() {
        req = req.header("X-Yap-Key", key);
    }
    let mut res = req.send(&bytes[..]).map_err(|e| format!("upload failed: {e}"))?;
    let status = res.status();
    let body: ImageResponse = res.body_mut().read_json().map_err(|_| format!("upload failed (HTTP {status})"))?;
    match (body.ok, body.id) {
        (true, Some(id)) => Ok(id),
        _ => Err(body.message.unwrap_or_else(|| format!("upload failed (HTTP {status})"))),
    }
}

#[derive(serde::Deserialize)]
struct DrawingResponse {
    #[serde(default)]
    ok: bool,
    drawing: Option<crate::protocol::SavedDrawing>,
    message: Option<String>,
}

/// `POST /api/drawing?sid&ut`: a PNG of the board → the gallery entry.
/// A drawing for the gallery: PNG or GIF (`mime`; the server sniffs the bytes).
pub fn upload_drawing(server: &str, key: &str, sid: &str, ut: &str, bytes: Vec<u8>, mime: &str) -> Result<crate::protocol::SavedDrawing, String> {
    let url = format!("{server}/api/drawing?sid={}&ut={}", percent_encode(sid), percent_encode(ut));
    let mut req = agent_config(Duration::from_secs(120))
        .build()
        .new_agent()
        .post(&url)
        .header("Content-Type", mime);
    if !key.is_empty() {
        req = req.header("X-Yap-Key", key);
    }
    let mut res = req.send(&bytes[..]).map_err(|e| format!("upload failed: {e}"))?;
    let status = res.status();
    let body: DrawingResponse = res.body_mut().read_json().map_err(|_| format!("upload failed (HTTP {status})"))?;
    match (body.ok, body.drawing) {
        (true, Some(d)) => Ok(d),
        _ => Err(body.message.unwrap_or_else(|| format!("upload failed (HTTP {status})"))),
    }
}

/// Fetch a picture's bytes and its type (≤ 8 MB).
pub fn get_bytes(url: &str) -> Result<(Vec<u8>, String), String> {
    let mut res = agent().get(url).call().map_err(|e| e.to_string())?;
    let mime = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("image/webp").to_string();
    let bytes = res.body_mut().with_config().limit(8 * 1024 * 1024).read_to_vec().map_err(|e| e.to_string())?;
    Ok((bytes, mime))
}

/// Fetch a small JSON resource (e.g. the standard emoji list).
pub fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let mut res = agent().get(url).call().map_err(|e| e.to_string())?;
    let status = res.status();
    let v: serde_json::Value = res.body_mut().read_json().map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    Ok(v)
}

/// One of the sidebar "Places" from `/api/config`.
#[derive(Deserialize, Debug, Clone)]
pub struct Link {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MinecraftConfig {
    pub name: String,
    pub address: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GifConfig {
    #[serde(rename = "appKey")]
    pub app_key: String,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct ServerConfig {
    #[serde(default)]
    pub gifs: Option<GifConfig>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub minecraft: Option<MinecraftConfig>,
}

/// `GET /api/config` — KLIPY key, sidebar links, the Minecraft server.
pub fn server_config(server: &str, key: &str) -> Result<ServerConfig, String> {
    let mut res = get(&format!("{server}/api/config"), key).map_err(|e| e.to_string())?;
    if res.status() != 200 {
        return Err(format!("HTTP {}", res.status()));
    }
    res.body_mut().read_json::<ServerConfig>().map_err(|e| e.to_string())
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct McPlayers {
    #[serde(default)]
    pub online: u32,
    #[serde(default)]
    pub max: u32,
    #[serde(default)]
    pub names: Vec<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct McStatus {
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub motd: Option<String>,
    #[serde(default)]
    pub players: Option<McPlayers>,
}

/// `GET /api/minecraft` — is the server's Minecraft server up? Only asked when
/// `/api/config` names one; a server without one answers 404.
pub fn minecraft(server: &str, key: &str) -> Result<McStatus, String> {
    let mut res = get(&format!("{server}/api/minecraft"), key).map_err(|e| e.to_string())?;
    if res.status() != 200 {
        return Err(format!("HTTP {}", res.status()));
    }
    res.body_mut().read_json::<McStatus>().map_err(|e| e.to_string())
}

pub fn post_json(url: &str, body: &serde_json::Value) -> Result<(), String> {
    agent()
        .post(url)
        .header("Content-Type", "application/json")
        .send(body.to_string().as_bytes())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn upload_agent() -> ureq::Agent {
    agent_config(Duration::from_secs(60)).build().new_agent()
}

#[derive(Deserialize)]
struct SoundResponse {
    #[serde(default)]
    ok: bool,
    sound: Option<crate::protocol::Sound>,
    message: Option<String>,
}

/// `POST /api/sound?sid&ut&kind&name&emoji`: a WAV (PCM16 mono) → the sound.
#[allow(clippy::too_many_arguments)]
pub fn upload_sound(server: &str, key: &str, sid: &str, ut: &str, kind: &str, name: &str, emoji: Option<&str>, wav: Vec<u8>) -> Result<crate::protocol::Sound, String> {
    let mut url = format!("{server}/api/sound?sid={}&ut={}&kind={}&name={}", percent_encode(sid), percent_encode(ut), percent_encode(kind), percent_encode(name));
    if let Some(e) = emoji.filter(|e| !e.is_empty()) {
        url.push_str(&format!("&emoji={}", percent_encode(e)));
    }
    let mut req = upload_agent().post(&url).header("Content-Type", "audio/wav");
    if !key.is_empty() {
        req = req.header("X-Yap-Key", key);
    }
    let mut res = req.send(&wav[..]).map_err(|e| format!("upload failed: {e}"))?;
    let status = res.status();
    let body: SoundResponse = res.body_mut().read_json().map_err(|_| format!("upload failed (HTTP {status})"))?;
    match (body.ok, body.sound) {
        (true, Some(s)) => Ok(s),
        _ => Err(body.message.unwrap_or_else(|| format!("upload failed (HTTP {status})"))),
    }
}

/// `POST /api/doodle?sid&ut`: a doodle (GIF/PNG ≤ 64 KB, ≤ 256 px) → its id.
pub fn upload_doodle(server: &str, key: &str, sid: &str, ut: &str, bytes: Vec<u8>) -> Result<String, String> {
    let url = format!("{server}/api/doodle?sid={}&ut={}", percent_encode(sid), percent_encode(ut));
    let mime = if bytes.starts_with(b"GIF8") { "image/gif" } else { "image/png" };
    let mut req = upload_agent().post(&url).header("Content-Type", mime);
    if !key.is_empty() {
        req = req.header("X-Yap-Key", key);
    }
    let mut res = req.send(&bytes[..]).map_err(|e| format!("upload failed: {e}"))?;
    let status = res.status();
    let body: ImageResponse = res.body_mut().read_json().map_err(|_| format!("upload failed (HTTP {status})"))?;
    match (body.ok, body.id) {
        (true, Some(id)) => Ok(id),
        _ => Err(body.message.unwrap_or_else(|| format!("upload failed (HTTP {status})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every request goes over https://, or plain http:// to this computer;
    /// anything else is refused before a byte is sent.
    #[test]
    fn plain_http_is_refused_before_sending() {
        for url in ["http://yap.example.invalid/api/check", "http://192.168.1.4:8787/api/ice", "http://localhost@yap.example.invalid/"] {
            assert!(matches!(agent().get(url).call(), Err(ureq::Error::RequireHttpsOnly(_))), "{url}");
            assert!(check(url.trim_end_matches("/api/check"), "key").is_err());
        }
        // This computer: tried (nothing listens on port 9).
        let local = agent().get("http://127.0.0.1:9/api/check").call();
        assert!(local.is_err() && !matches!(local, Err(ureq::Error::RequireHttpsOnly(_))));
    }

    #[test]
    fn server_config_extras_are_optional() {
        // A server with no Minecraft, no board link, no GIFs and no pushes.
        let c: ServerConfig = serde_json::from_str(r#"{"gifs":null,"links":[],"push":null}"#).unwrap();
        assert!(c.minecraft.is_none() && c.gifs.is_none() && c.links.is_empty());
        let c: ServerConfig = serde_json::from_str("{}").unwrap();
        assert!(c.minecraft.is_none() && c.gifs.is_none() && c.links.is_empty());
        // Everything on (and fields this app doesn't use are ignored).
        let c: ServerConfig = serde_json::from_str(
            r#"{"gifs":{"provider":"klipy","appKey":"k"},"links":[{"name":"Board","url":"https://x/board","view":"board"}],"minecraft":{"name":"Minecraft","address":"127.0.0.1:9"},"push":{"publicKey":"p"},"later":1}"#,
        )
        .unwrap();
        assert_eq!(c.gifs.unwrap().app_key, "k");
        assert_eq!(c.minecraft.unwrap().address, "127.0.0.1:9");
        assert_eq!(c.links[0].view.as_deref(), Some("board"));
    }
}
