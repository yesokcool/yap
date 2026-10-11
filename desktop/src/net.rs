//! The one WebSocket to the server: heartbeat (`ping` every 15 s, dead after
//! 10 s without `pong`) and reconnect with jittered exponential backoff
//! (0.5 s -> 10 s), exactly as PROTOCOL.md describes.

use crate::protocol::ServerMsg;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::tungstenite::{self, Message as WsMessage};

const HEARTBEAT: Duration = Duration::from_secs(15);
const PONG_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum NetEvent {
    Open,
    Msg(Box<ServerMsg>),
    /// The socket closed (or never opened). `unauthorized` = the server said 401.
    Closed { unauthorized: bool, reason: String },
}

pub struct NetHandle {
    pub out: mpsc::UnboundedSender<String>,
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
    let (rn, st, dn) = (reconnect_now.clone(), stop.clone(), drop_now.clone());
    tokio::spawn(async move {
        let mut retry: u32 = 0;
        loop {
            let request = match ws_request(&server, &key) {
                Ok(r) => r,
                Err(e) => {
                    let _ = events.send(NetEvent::Closed { unauthorized: false, reason: e });
                    return;
                }
            };
            let attempt = tokio::select! {
                r = tokio_tungstenite::connect_async(request) => r,
                _ = st.notified() => return,
            };
            let (unauthorized, reason) = match attempt {
                Ok((ws, _resp)) => {
                    retry = 0;
                    // Drop anything queued while we were offline; the core re-sends what matters on `ready`.
                    while out_rx.try_recv().is_ok() {}
                    if events.send(NetEvent::Open).is_err() {
                        return;
                    }
                    let reason = tokio::select! {
                        r = run(ws, &mut out_rx, &events) => r,
                        _ = dn.notified() => "dropped (test)".to_string(),
                        _ = st.notified() => return,
                    };
                    (false, reason)
                }
                Err(tungstenite::Error::Http(resp)) => {
                    let code = resp.status().as_u16();
                    (code == 401, format!("HTTP {code}"))
                }
                Err(e) => (false, e.to_string()),
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
    NetHandle { out: out_tx, reconnect_now, stop, drop_now }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn run(ws: Ws, out_rx: &mut mpsc::UnboundedReceiver<String>, events: &mpsc::UnboundedSender<NetEvent>) -> String {
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
                                return "client gone".into();
                            }
                        }
                        Err(e) => log::warn!("bad frame from server: {e}: {}", &text[..text.len().min(200)]),
                    }
                }
                Some(Ok(WsMessage::Close(_))) | None => return "closed by server".into(),
                Some(Ok(_)) => {}
                Some(Err(e)) => return e.to_string(),
            },
            out = out_rx.recv() => match out {
                Some(text) => {
                    if let Err(e) = write.send(WsMessage::Text(text.into())).await {
                        return e.to_string();
                    }
                }
                None => return "client gone".into(),
            },
            _ = hb.tick() => {
                if let Err(e) = write.send(WsMessage::Text("ping".into())).await {
                    return e.to_string();
                }
                pong_deadline.get_or_insert(tokio::time::Instant::now() + PONG_TIMEOUT);
            }
            // A ping that's gone 10 s without a pong means the socket is dead even if
            // the OS hasn't noticed (sleeping laptop, dropped Wi-Fi).
            _ = async { tokio::time::sleep_until(deadline.unwrap()).await }, if deadline.is_some() => {
                return "heartbeat timeout".into();
            }
        }
    }
}
