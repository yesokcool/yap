//! The one WebSocket to the server: heartbeat (`ping` every 15 s, dead after
//! 10 s without `pong`) and reconnect with jittered exponential backoff
//! (0.5 s -> 10 s), exactly as PROTOCOL.md describes. An attempt that hasn't
//! connected in 15 s counts as failed (a browser has its own such limit).
//!
//! The invite key can change while we're connected (the owner reset the main
//! link: `key`, `ready.key`), so each new attempt reads the current one.

use crate::protocol::ServerMsg;
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::tungstenite::{self, Message as WsMessage};

const HEARTBEAT: Duration = Duration::from_secs(15);
const PONG_TIMEOUT: Duration = Duration::from_secs(10);
/// One attempt (TCP, TLS, the upgrade) gives up after this: a network that
/// swallows packets (a captive portal, Wi‑Fi waking up) would otherwise hold
/// it for the OS's own timeout, or forever in the TLS handshake.
const CONNECT_TIMEOUT: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(15) };
/// The server closes a socket with this when its invite key is wrong and it
/// didn't sign in with a device token (PROTOCOL.md › HTTP › Resetting the main link).
/// (It also closes with 4003 "banned" or "denied", right after `error banned`
/// or `error denied`, which the core acts on first, and an OBS link's sockets
/// with "revoked".)
pub const WRONG_KEY_CLOSE: u16 = 4003;

/// Does this close say our invite key is wrong?
fn wrong_key(code: Option<u16>, reason: &str) -> bool {
    code == Some(WRONG_KEY_CLOSE) && ![" banned", " denied", " revoked"].iter().any(|w| reason.ends_with(w))
}

#[derive(Debug)]
pub enum NetEvent {
    Open,
    Msg(Box<ServerMsg>),
    /// The socket closed (or never opened). `unauthorized` = the invite key is
    /// wrong: a 401 on the upgrade (older servers), or a 4003 close.
    Closed { unauthorized: bool, reason: String },
}

pub struct NetHandle {
    pub out: mpsc::UnboundedSender<String>,
    /// The invite key the next attempt uses (a new main link replaces it).
    pub key: Arc<Mutex<String>>,
    pub reconnect_now: Arc<Notify>,
    pub stop: Arc<Notify>,
    /// Tests: close the socket now (it reconnects like after any blip).
    pub drop_now: Arc<Notify>,
}

/// A key can travel as a WebSocket subprotocol only if it's a valid token.
fn key_is_token(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// The upgrade request: key as the `["yap", "key.<key>"]` subprotocols (the
/// server answers `yap`), so it never appears in a URL. Keys that can't be a
/// token fall back to `?key=`.
pub fn ws_request(server: &str, key: &str) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request, String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let in_header = key_is_token(key);
    let url = ws_url(server, if in_header { "" } else { key });
    // ws:// only to this computer: the key, the token and the password would go in the clear.
    if let Some(why) = crate::config::insecure(&url) {
        return Err(why.to_string());
    }
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    // The server names this device from it in your devices list ("Desktop app (Mac)").
    let os = if cfg!(target_os = "macos") { "Mac" } else if cfg!(windows) { "Windows" } else { "Linux" };
    if let Ok(ua) = format!("yap-desktop/{} ({os})", env!("CARGO_PKG_VERSION")).parse() {
        req.headers_mut().insert("User-Agent", ua);
    }
    if in_header {
        let value = format!("yap, key.{key}").parse().map_err(|_| "bad key".to_string())?;
        req.headers_mut().insert("Sec-WebSocket-Protocol", value);
    }
    Ok(req)
}

pub fn ws_url(server: &str, key: &str) -> String {
    let base = if let Some(rest) = server.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = server.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("wss://{server}")
    };
    if key.is_empty() { format!("{base}/ws") } else { format!("{base}/ws?key={}", crate::config::percent_encode(key)) }
}

pub fn spawn(server: String, key: String, events: mpsc::UnboundedSender<NetEvent>) -> NetHandle {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let reconnect_now = Arc::new(Notify::new());
    let stop = Arc::new(Notify::new());
    let drop_now = Arc::new(Notify::new());
    let key = Arc::new(Mutex::new(key));
    let (rn, st, dn, current_key) = (reconnect_now.clone(), stop.clone(), drop_now.clone(), key.clone());
    tokio::spawn(async move {
        let mut retry: u32 = 0;
        loop {
            let key = current_key.lock().clone();
            let request = match ws_request(&server, &key) {
                Ok(r) => r,
                Err(e) => {
                    let _ = events.send(NetEvent::Closed { unauthorized: false, reason: e });
                    return;
                }
            };
            let attempt = tokio::select! {
                r = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request)) => r,
                _ = st.notified() => return,
            };
            let (unauthorized, reason) = match attempt {
                Ok(Ok((ws, _resp))) => {
                    retry = 0;
                    // Drop anything queued while we were offline; the core re-sends what matters on `ready`.
                    while out_rx.try_recv().is_ok() {}
                    if events.send(NetEvent::Open).is_err() {
                        return;
                    }
                    let (reason, code) = tokio::select! {
                        r = run(ws, &mut out_rx, &events) => r,
                        _ = dn.notified() => ("dropped (test)".to_string(), None),
                        _ = st.notified() => return,
                    };
                    (wrong_key(code, &reason), reason)
                }
                Ok(Err(tungstenite::Error::Http(resp))) => {
                    let code = resp.status().as_u16();
                    (code == 401, format!("HTTP {code}"))
                }
                Ok(Err(e)) => (false, e.to_string()),
                Err(_) => (false, "timed out connecting".to_string()),
            };
            if events.send(NetEvent::Closed { unauthorized, reason }).is_err() {
                return;
            }
            let base = (500u64.saturating_mul(1u64 << retry.min(10))).min(10_000);
            let jitter = 0.75 + (crate::config::random_u32() as f64 / u32::MAX as f64) * 0.5;
            retry = retry.saturating_add(1);
            let delay = Duration::from_millis((base as f64 * jitter) as u64);
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = rn.notified() => { retry = 0; }
                _ = st.notified() => return,
            }
        }
    });
    NetHandle { out: out_tx, key, reconnect_now, stop, drop_now }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Until the socket ends: why, and the server's close code if it sent one.
async fn run(ws: Ws, out_rx: &mut mpsc::UnboundedReceiver<String>, events: &mpsc::UnboundedSender<NetEvent>) -> (String, Option<u16>) {
    let (mut write, mut read) = ws.split();
    let mut hb = tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
    // Set when a ping goes out; cleared by the pong.
    let mut pong_deadline: Option<tokio::time::Instant> = None;
    loop {
        let deadline = pong_deadline;
        tokio::select! {
            frame = read.next() => match frame {
                Some(Ok(WsMessage::Text(text))) => {
                    let text = text.as_str();
                    if text == "pong" {
                        pong_deadline = None;
                        continue;
                    }
                    match serde_json::from_str::<ServerMsg>(text) {
                        Ok(msg) => {
                            if events.send(NetEvent::Msg(Box::new(msg))).is_err() {
                                return ("client gone".into(), None);
                            }
                        }
                        Err(e) => log::warn!("bad frame from server: {e}: {}", head(text, 200)),
                    }
                }
                Some(Ok(WsMessage::Close(frame))) => return closed_by_server(frame.as_ref()),
                None => return ("closed by server".into(), None),
                Some(Ok(_)) => {}
                Some(Err(e)) => return (e.to_string(), None),
            },
            out = out_rx.recv() => match out {
                Some(text) => {
                    if let Err(e) = write.send(WsMessage::Text(text.into())).await {
                        return (e.to_string(), None);
                    }
                }
                None => return ("client gone".into(), None),
            },
            _ = hb.tick() => {
                if let Err(e) = write.send(WsMessage::Text("ping".into())).await {
                    return (e.to_string(), None);
                }
                pong_deadline.get_or_insert(tokio::time::Instant::now() + PONG_TIMEOUT);
            }
            // A ping that's gone 10 s without a pong means the socket is dead even if
            // the OS hasn't noticed (sleeping laptop, dropped Wi-Fi).
            _ = async { tokio::time::sleep_until(deadline.unwrap()).await }, if deadline.is_some() => {
                return ("heartbeat timeout".into(), None);
            }
        }
    }
}

/// At most `max` bytes of `text`, cut at a character boundary (a byte offset
/// inside an emoji or an accented letter would panic, and release builds abort).
fn head(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The server's close frame: its code and words ("closed by server: 4003 wrong invite key").
fn closed_by_server(frame: Option<&tungstenite::protocol::CloseFrame>) -> (String, Option<u16>) {
    match frame {
        Some(f) => {
            let code = u16::from(f.code);
            let words = if f.reason.is_empty() { String::new() } else { format!(" {}", f.reason.as_str()) };
            (format!("closed by server: {code}{words}"), Some(code))
        }
        None => ("closed by server".into(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};

    /// A server that takes the connection but never answers the upgrade: the
    /// attempt gives up (and the app says so, then tries again) instead of
    /// "Connecting…" for ever.
    #[tokio::test]
    async fn a_silent_server_times_out() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let mut kept = Vec::new();
            while let Ok((s, _)) = listener.accept().await {
                kept.push(s);
            }
        });
        let (tx, mut rx) = mpsc::unbounded_channel();
        let net = spawn(format!("http://{addr}"), "k".into(), tx);
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("an answer in time");
        match ev {
            Some(NetEvent::Closed { unauthorized: false, reason }) => assert!(reason.contains("timed out"), "{reason}"),
            other => panic!("expected a timeout, got {other:?}"),
        }
        net.stop.notify_waiters();
        held.abort();
    }

    /// A bad frame's log line never cuts a character in half (that panicked, and the app aborted).
    #[test]
    fn log_lines_cut_at_characters() {
        assert_eq!(head("hello", 200), "hello");
        let text = format!("{}é and more", "a".repeat(199));
        assert_eq!(head(&text, 200), "a".repeat(199), "é starts at byte 199 and takes two");
        assert_eq!(head("😀😀", 5), "😀");
        assert_eq!(head("😀", 3), "");
    }

    /// A 4003 close is "wrong invite key"; any other close is just a closed socket.
    #[test]
    fn close_codes() {
        let wrong = CloseFrame { code: CloseCode::from(4003), reason: "wrong invite key".into() };
        assert_eq!(closed_by_server(Some(&wrong)), ("closed by server: 4003 wrong invite key".to_string(), Some(WRONG_KEY_CLOSE)));
        let revoked = CloseFrame { code: CloseCode::from(4001), reason: "".into() };
        assert_eq!(closed_by_server(Some(&revoked)), ("closed by server: 4001".to_string(), Some(4001)));
        assert_eq!(closed_by_server(None), ("closed by server".to_string(), None));
        let (reason, code) = closed_by_server(Some(&wrong));
        assert!(wrong_key(code, &reason));
        let banned = CloseFrame { code: CloseCode::from(4003), reason: "banned".into() };
        let (reason, code) = closed_by_server(Some(&banned));
        assert!(!wrong_key(code, &reason), "a ban isn't a key problem (`error banned` came first)");
        assert!(!wrong_key(Some(4003), "closed by server: 4003 denied"));
        assert!(!wrong_key(Some(4001), "closed by server: 4001 signed out"));
        assert!(!wrong_key(None, "heartbeat timeout"));
    }
}
