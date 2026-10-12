//! Voice = WebRTC mesh (see PROTOCOL.md › Voice and web/js/voice.js).
//!
//! Every remote session in our channel gets one `RTCPeerConnection` with a
//! single sendrecv Opus transceiver, driven by one task that handles that
//! peer's signals strictly in order. The newcomer offers; glare is resolved
//! with perfect negotiation where the larger sid is polite. Every local
//! description gets `tune_opus()` before it's applied.
//!
//! Calls outlive the WebSocket (PROTOCOL.md › Voice › Keeping calls across
//! reconnects): while it's down, connections stay up and their outgoing
//! signals wait in a queue (`suspend`); afterwards each connection is moved
//! to the far side's new session id (`rekey`) or ours (`set_my_sid`) instead
//! of being rebuilt. The core decides which. A connection whose path dies
//! restarts ICE (impolite after 3.5 s `disconnected`, polite after 10 s,
//! with freshly fetched ICE servers), and is rebuilt after three tries.
//!
//! Screen sharing rides on the same connections (PROTOCOL.md › Screen
//! sharing): a peer that asks for our screen gets an H.264 video track (and
//! the share's sound as a second Opus track) in the share's own stream; a
//! sharer's tracks are told apart from their voice by transceiver (the voice
//! is always the first). See `crate::screen`.

pub mod loss;
pub mod red;
pub mod sdp;

use crate::audio::{AudioEngine, AudioFrame, FRAME};
use crate::screen::{EncodedAudio, EncodedVideo, Screen};
use crate::http::IceServer;
use crate::protocol::{CandidateJson, SdpJson, SignalData};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};

use rtc::media::Sample;
use rtc::rtp::extension::audio_level_extension::AudioLevelExtension;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters, RTCRtpHeaderExtensionCapability,
    RtpCodecKind,
};
use rtc::rtp_transceiver::RTCRtpTransceiverDirection;
use rtc::statistics::stats::ice_candidate_pair::RTCStatsIceCandidatePairState;
use webrtc::media_stream::MediaStreamTrack;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCBundlePolicy, RTCConfiguration, RTCConfigurationBuilder,
    RTCIceCandidateInit, RTCIceServer, RTCIceTransportPolicy, RTCPeerConnectionIceEvent, RTCPeerConnectionState, RTCSdpType,
    RTCSessionDescription, RTCStatsReportEntry, Registry, SettingEngineBuilder, StatsSelector, configure_nack, configure_rtcp_reports,
};
use rtc::interceptor::{Slot, TwccReceiverBuilder};
use rtc::rtp::packetizer::Packetizer as _;
use rtc::rtp_transceiver::rtp_sender::RTCPFeedback;
use webrtc::rtp_transceiver::{RtpSender, RtpTransceiver};

const OPUS_PT: u8 = 111;
/// `red/48000/2` (RFC 2198) carrying Opus: the payload type browsers use.
const RED_PT: u8 = 63;
/// H.264 for screen video: constrained baseline, packetization mode 1 (what
/// every client offers and accepts; PROTOCOL.md › Screen sharing).
const H264_PT: u8 = 102;
const H264_FMTP: &str = "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f";
const AUDIO_LEVEL_URI: &str = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";
/// ICE restarts (PROTOCOL.md › Voice › ICE restarts).
const RESTART_IMPOLITE: Duration = Duration::from_millis(3500);
const RESTART_POLITE: Duration = Duration::from_secs(10);
const RESTART_AGAIN: Duration = Duration::from_secs(10);
const RESTARTS_BEFORE_REBUILD: u32 = 3;
/// `/api/ice` is fetched again for a restart, at most this often.
const ICE_REFETCH: Duration = Duration::from_secs(30);
const STATS_EVERY: Duration = Duration::from_secs(2);

/// RED on the voice (PROTOCOL.md › Voice › Redundancy). `YAP_RED=0` leaves it
/// out (plain Opus both ways, like an app from before it): a switch for
/// measuring what it does.
pub fn red_enabled() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("YAP_RED").map_or(true, |v| v != "0"));
    *ON
}

#[derive(Debug, Clone)]
pub enum VoiceEvent {
    /// connectionState of one peer: new/connecting/connected/disconnected/failed/closed.
    PeerState { sid: String, state: String },
    /// ICE restarts didn't bring it back: throw the connection away and offer a new one.
    Rebuild { sid: String },
    /// An ICE restart (for the call report).
    IceRestart { sid: String, attempt: u32 },
    /// They started a brand-new connection (a new DTLS fingerprint): we followed.
    FreshStart { sid: String },
}

/// Live numbers for one peer (read by the UI, the core and the headless reporter).
pub struct PeerInfo {
    /// Stable for the connection's life (the sid can change, this can't).
    pub id: u64,
    sid: RwLock<String>,
    polite: AtomicBool,
    pub state: Mutex<String>,
    pub bytes_sent: AtomicU64,
    pub packets_sent: AtomicU64,
    /// Of `bytes_sent`, the Opus itself (the rest is RED's copies and headers).
    pub opus_bytes_sent: AtomicU64,
    /// We send them RED (`red+opus`), not plain Opus.
    pub send_red: AtomicBool,
    /// RED payload types they may send us (ours and theirs, a byte each; 0 = none).
    red_pts: std::sync::atomic::AtomicU16,
    /// Bitrate our encoder is running at for this peer (bps).
    pub send_bitrate: AtomicU32,
    /// Our applied Opus fmtp (what we asked to receive) and theirs.
    pub local_fmtp: Mutex<String>,
    pub remote_fmtp: Mutex<String>,
    /// Our local description mentions transport-cc (it shouldn't, on audio).
    pub local_twcc: AtomicBool,
    /// Their app keeps calls across reconnects (`Session.voice.resume`).
    pub resumable: AtomicBool,
    /// ICE failed since the connection was last up.
    pub failed: AtomicBool,
    pub connected_at: Mutex<Option<Instant>>,
    /// Round trip (ms) of the selected candidate pair; 0 = not known yet.
    pub rtt_ms: AtomicU32,
    /// Share of our packets they report lost (RTCP receiver reports), smoothed, in ‰.
    pub loss_permille: AtomicU32,
    /// What they report about our voice (remote-inbound-rtp; PROTOCOL.md ›
    /// Connection hints › Your own connection): reports so far, the latest
    /// one's fraction lost (‰), and how many of our packets they've lost in all.
    pub their_reports: AtomicU32,
    pub their_fraction: AtomicU32,
    pub their_lost: AtomicU64,
    /// The encoder's packet-loss setting (%): 0 = clean (CELT), else Hybrid + FEC.
    pub loss_hint: AtomicU8,
    /// ICE restarts since it was last connected.
    pub restarts: AtomicU32,
    /// The candidate pair in use, by type only (host, srflx, prflx, relay; never addresses).
    pub path: Mutex<Option<(&'static str, &'static str)>>,
    /// The DTLS fingerprint of their current description.
    remote_fp: Mutex<Option<String>>,
    /// They asked to watch our screen (`signal {screen: "want"}`).
    pub wants_screen: AtomicBool,
    /// Our screen goes out to them right now.
    pub sending_screen: AtomicBool,
}

impl PeerInfo {
    pub fn sid(&self) -> String {
        self.sid.read().clone()
    }
    pub fn polite(&self) -> bool {
        self.polite.load(Ordering::Relaxed)
    }
    fn is_red(&self, pt: u8) -> bool {
        let r = self.red_pts.load(Ordering::Relaxed);
        pt != 0 && (pt == r as u8 || pt == (r >> 8) as u8)
    }
}

enum PeerMsg {
    Signal(SignalData),
    NegotiationNeeded,
    State(RTCPeerConnectionState),
    Renegotiate,
    /// Our share started or stopped, its sound was switched, or they asked
    /// for (or are done with) our screen: add or remove the share's tracks.
    ScreenSync,
    /// Back online: if an offer of ours is unanswered (and wasn't just sent from the queue), send it again.
    Resync { sent: Vec<String> },
    Close,
}

struct PeerHandle {
    tx: mpsc::UnboundedSender<PeerMsg>,
    info: Arc<PeerInfo>,
}

/// Settings every peer reads live.
pub struct VoiceSettings {
    pub bitrate: AtomicU32,
    /// The share's sound volume per sharer (sid → 0–1), for when their sound track arrives.
    pub screen_volumes: Mutex<HashMap<String, f32>>,
}

/// The mixer's name for a sharer's screen sound (their voice is just their sid).
pub fn screen_key(sid: &str) -> String {
    format!("screen:{sid}")
}

pub type IceFetch = Arc<dyn Fn() -> Vec<IceServer> + Send + Sync>;

/// The ICE servers new connections and restarts use.
struct Ice {
    servers: Vec<IceServer>,
    /// Only through the TURN relay (people in the call never see our IP).
    relay_only: bool,
    fetched_at: Option<Instant>,
    fetch: Option<IceFetch>,
}

impl Ice {
    /// Fresh servers for an ICE restart (relay credentials can expire): `/api/ice`
    /// again, at most every 30 s. Relay-only never takes a list without a relay.
    async fn fresh(ice: &Arc<Mutex<Ice>>) -> (Vec<IceServer>, bool) {
        let fetch = {
            let i = ice.lock();
            i.fetch.clone().filter(|_| i.fetched_at.is_none_or(|t| t.elapsed() > ICE_REFETCH))
        };
        if let Some(f) = fetch {
            ice.lock().fetched_at = Some(Instant::now());
            if let Ok(servers) = tokio::task::spawn_blocking(move || f()).await {
                let mut i = ice.lock();
                if !servers.is_empty() && (!i.relay_only || crate::http::has_relay(&servers)) {
                    i.servers = servers;
                }
            }
        }
        let i = ice.lock();
        (i.servers.clone(), i.relay_only)
    }
}

pub struct VoiceEngine {
    my_sid: String,
    ice: Arc<Mutex<Ice>>,
    peers: HashMap<String, PeerHandle>,
    next_id: u64,
    /// Is the socket up? While it isn't, signals wait in `outbox`.
    online: bool,
    outbox: HashMap<u64, Vec<SignalData>>,
    pub audio: Arc<AudioEngine>,
    pub screen: Arc<Screen>,
    pub settings: Arc<VoiceSettings>,
    signal_out: mpsc::UnboundedSender<(u64, SignalData)>,
    events: mpsc::UnboundedSender<VoiceEvent>,
}

impl VoiceEngine {
    pub fn new(audio: Arc<AudioEngine>, screen: Arc<Screen>, bitrate: u32, signal_out: mpsc::UnboundedSender<(u64, SignalData)>, events: mpsc::UnboundedSender<VoiceEvent>) -> Self {
        Self {
            my_sid: String::new(),
            ice: Arc::new(Mutex::new(Ice { servers: Vec::new(), relay_only: false, fetched_at: None, fetch: None })),
            peers: HashMap::new(),
            next_id: 1,
            online: true,
            outbox: HashMap::new(),
            audio,
            screen,
            settings: Arc::new(VoiceSettings { bitrate: AtomicU32::new(bitrate), screen_volumes: Mutex::new(HashMap::new()) }),
            signal_out,
            events,
        }
    }

    /// Our session id (a new one after every reconnect): kept connections
    /// recompute who's polite.
    pub fn set_my_sid(&mut self, sid: String) {
        self.my_sid = sid;
        for p in self.peers.values() {
            p.info.polite.store(self.my_sid > p.info.sid(), Ordering::Relaxed);
        }
    }

    /// The servers new peers use; `relay_only` (Settings › Voice › Hide my IP
    /// address, only when there's a TURN relay) gathers relay candidates only.
    pub fn set_ice(&mut self, servers: Vec<IceServer>, relay_only: bool) {
        let mut i = self.ice.lock();
        i.servers = servers;
        i.relay_only = relay_only;
        i.fetched_at = Some(Instant::now());
    }

    /// How to fetch `/api/ice` again (for ICE restarts).
    pub fn set_ice_fetch(&mut self, f: IceFetch) {
        self.ice.lock().fetch = Some(f);
    }

    pub fn has_relay(&self) -> bool {
        crate::http::has_relay(&self.ice.lock().servers)
    }

    pub fn has_peer(&self, sid: &str) -> bool {
        self.peers.contains_key(sid)
    }

    pub fn peer(&self, sid: &str) -> Option<Arc<PeerInfo>> {
        self.peers.get(sid).map(|p| p.info.clone())
    }

    pub fn peers(&self) -> Vec<Arc<PeerInfo>> {
        self.peers.values().map(|p| p.info.clone()).collect()
    }

    pub fn sids(&self) -> Vec<String> {
        self.peers.keys().cloned().collect()
    }

    /// We just joined: offer to everyone already in the channel.
    pub fn enter(&mut self, sids: &[String], volume_of: impl Fn(&str) -> f32) {
        for sid in sids {
            if *sid != self.my_sid && !self.peers.contains_key(sid) {
                self.create_peer(sid, true, volume_of(sid));
            }
        }
    }

    pub fn handle_signal(&mut self, from: &str, data: SignalData, volume: f32) {
        // Fresh start: an offer with a different DTLS fingerprint = the far side
        // started a brand-new connection (it couldn't keep ours): start over too.
        let had = self.peers.get(from).and_then(|p| p.info.remote_fp.lock().clone().map(|fp| (fp, p.info.resumable.load(Ordering::Relaxed))));
        if let (Some((had, resumable)), Some(desc)) = (had, data.sdp.as_ref())
            && desc.kind == "offer"
            && sdp::fingerprint(&desc.sdp).is_some_and(|now| now != had)
        {
            log::info!("peer {from}: a new connection (fingerprint changed); starting over");
            let _ = self.events.send(VoiceEvent::FreshStart { sid: from.to_string() });
            self.drop_peer(from);
            self.create_peer(from, false, volume);
            self.set_resumable(from, resumable);
        }
        if !self.peers.contains_key(from) {
            // Only an offer may open a new connection.
            if data.sdp.as_ref().is_none_or(|s| s.kind != "offer") {
                return;
            }
            self.create_peer(from, false, volume);
        }
        if let Some(p) = self.peers.get(from) {
            let _ = p.tx.send(PeerMsg::Signal(data));
        }
    }

    pub fn drop_peer(&mut self, sid: &str) {
        if let Some(p) = self.peers.remove(sid) {
            self.outbox.remove(&p.info.id);
            let _ = p.tx.send(PeerMsg::Close);
        }
        self.audio.remove_stream(sid);
        self.audio.remove_stream(&screen_key(sid));
    }

    /// Someone asked for our screen (`on`), or is done with it.
    pub fn set_wants_screen(&self, sid: &str, on: bool) {
        if let Some(p) = self.peers.get(sid) {
            p.info.wants_screen.store(on, Ordering::Relaxed);
            let _ = p.tx.send(PeerMsg::ScreenSync);
        }
    }

    /// Our share started or stopped, or its sound was switched: every connection follows.
    pub fn screen_changed(&self) {
        for p in self.peers.values() {
            let _ = p.tx.send(PeerMsg::ScreenSync);
        }
    }

    /// Our share stopped: forget who asked (they ask again for the next one).
    pub fn forget_screen_requests(&self) {
        for p in self.peers.values() {
            p.info.wants_screen.store(false, Ordering::Relaxed);
        }
    }

    /// Who asked to watch our screen (their sids).
    pub fn watchers(&self) -> Vec<String> {
        self.peers.iter().filter(|(_, p)| p.info.wants_screen.load(Ordering::Relaxed)).map(|(s, _)| s.clone()).collect()
    }

    /// The volume of a sharer's screen sound (0–1), now and for when it arrives.
    pub fn set_screen_volume(&self, sid: &str, volume: f32) {
        self.settings.screen_volumes.lock().insert(sid.to_string(), volume);
        self.audio.set_volume(&screen_key(sid), volume);
    }

    /// Hang up on everyone (and stop queueing: the next call starts fresh).
    pub fn hangup_all(&mut self) {
        let sids: Vec<String> = self.peers.keys().cloned().collect();
        for sid in sids {
            self.drop_peer(&sid);
        }
        self.outbox.clear();
        self.online = true;
    }

    /// Bitrate (or other SDP-visible setting) changed: re-offer to everyone.
    pub fn renegotiate_all(&self) {
        for p in self.peers.values() {
            let _ = p.tx.send(PeerMsg::Renegotiate);
        }
    }

    /// Our socket dropped mid-call: keep every connection; signals wait until we're back.
    pub fn suspend(&mut self) {
        self.online = false;
    }

    /// A signal from a connection: where to send it now, or None (queued while
    /// offline, or the connection is gone).
    pub fn route(&mut self, id: u64, data: SignalData) -> Option<(String, SignalData)> {
        let sid = self.peers.values().find(|p| p.info.id == id)?.info.sid();
        if self.online {
            Some((sid, data))
        } else {
            self.outbox.entry(id).or_default().push(data);
            None
        }
    }

    /// Send what waited for this peer, and re-send an unanswered offer.
    fn resync(&mut self, sid: &str) -> Vec<(String, SignalData)> {
        let Some(p) = self.peers.get(sid) else { return Vec::new() };
        let queued = self.outbox.remove(&p.info.id).unwrap_or_default();
        let sent = queued.iter().filter_map(|d| d.sdp.as_ref().map(|s| s.sdp.clone())).collect();
        let _ = p.tx.send(PeerMsg::Resync { sent });
        queued.into_iter().map(|d| (sid.to_string(), d)).collect()
    }

    /// Move a connection to the far side's new session id (they resumed the
    /// same call): same connection, no new ICE or DTLS, no gap. Returns the
    /// signals to send now, or None if there was nothing to move.
    pub fn rekey(&mut self, from: &str, to: &str) -> Option<Vec<(String, SignalData)>> {
        if from == to || !self.peers.contains_key(from) {
            return None;
        }
        if self.peers.contains_key(to) {
            // Already connected to the new session (it offered first): the old one is spare.
            self.drop_peer(from);
            return None;
        }
        let p = self.peers.remove(from)?;
        *p.info.sid.write() = to.to_string();
        p.info.polite.store(self.my_sid.as_str() > to, Ordering::Relaxed);
        self.audio.rename_stream(from, to);
        self.audio.rename_stream(&screen_key(from), &screen_key(to));
        self.screen.watch.rename(from, to);
        let mut vols = self.settings.screen_volumes.lock();
        if let Some(v) = vols.remove(from) {
            vols.insert(to.to_string(), v);
        }
        drop(vols);
        self.peers.insert(to.to_string(), p);
        Some(if self.online { self.resync(to) } else { Vec::new() })
    }

    /// Our own call was handed over to our new session (our `voice` echo came
    /// back with `prev`). `here` = the other sessions in the channel now.
    /// Connections to people who keep calls stay; older apps hung up on us
    /// when our socket dropped, so they (and anyone new) get a fresh offer, as
    /// from a newcomer. Connections to people not back yet are left alone:
    /// they may be reconnecting too (a deploy drops everyone at once).
    pub fn handover(&mut self, here: &[String], volume_of: impl Fn(&str) -> f32) -> Vec<(String, SignalData)> {
        self.online = true;
        let mut out = Vec::new();
        let kept: Vec<(String, bool)> = self.peers.iter().map(|(s, p)| (s.clone(), p.info.resumable.load(Ordering::Relaxed))).collect();
        for (sid, resumable) in kept {
            if !here.contains(&sid) {
                continue;
            }
            if resumable {
                out.extend(self.resync(&sid));
            } else {
                self.drop_peer(&sid);
            }
        }
        self.enter(here, volume_of);
        out
    }

    /// Throw a connection away and offer a brand-new one (the far side sees a
    /// new fingerprint and follows).
    pub fn restart(&mut self, sid: &str, volume: f32) {
        let Some(resumable) = self.peers.get(sid).map(|p| p.info.resumable.load(Ordering::Relaxed)) else { return };
        self.drop_peer(sid);
        self.create_peer(sid, true, volume);
        self.set_resumable(sid, resumable);
    }

    pub fn set_resumable(&self, sid: &str, on: bool) {
        if let Some(p) = self.peers.get(sid) {
            p.info.resumable.store(on, Ordering::Relaxed);
        }
    }

    fn create_peer(&mut self, sid: &str, initiator: bool, volume: f32) {
        let (tx, rx) = mpsc::unbounded_channel();
        let id = self.next_id;
        self.next_id += 1;
        let info = Arc::new(PeerInfo {
            id,
            sid: RwLock::new(sid.to_string()),
            polite: AtomicBool::new(self.my_sid.as_str() > sid),
            state: Mutex::new("new".into()),
            bytes_sent: AtomicU64::new(0),
            packets_sent: AtomicU64::new(0),
            opus_bytes_sent: AtomicU64::new(0),
            send_red: AtomicBool::new(false),
            red_pts: std::sync::atomic::AtomicU16::new(0),
            send_bitrate: AtomicU32::new(self.settings.bitrate.load(Ordering::Relaxed)),
            local_fmtp: Mutex::new(String::new()),
            remote_fmtp: Mutex::new(String::new()),
            local_twcc: AtomicBool::new(false),
            resumable: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            connected_at: Mutex::new(None),
            rtt_ms: AtomicU32::new(0),
            loss_permille: AtomicU32::new(0),
            their_reports: AtomicU32::new(0),
            their_fraction: AtomicU32::new(0),
            their_lost: AtomicU64::new(0),
            loss_hint: AtomicU8::new(0),
            restarts: AtomicU32::new(0),
            path: Mutex::new(None),
            remote_fp: Mutex::new(None),
            wants_screen: AtomicBool::new(false),
            sending_screen: AtomicBool::new(false),
        });
        let ctx = PeerCtx {
            loss: Arc::new(loss::LossReport::default()),
            initiator,
            volume,
            ice: self.ice.clone(),
            audio: self.audio.clone(),
            screen: self.screen.clone(),
            settings: self.settings.clone(),
            signal_out: self.signal_out.clone(),
            events: self.events.clone(),
            info: info.clone(),
            tx: tx.clone(),
        };
        tokio::spawn(run_peer(ctx, rx));
        self.peers.insert(sid.to_string(), PeerHandle { tx, info });
    }
}

#[derive(Clone)]
struct PeerCtx {
    /// What the far side reports about our stream (RTCP report blocks).
    loss: Arc<loss::LossReport>,
    initiator: bool,
    volume: f32,
    ice: Arc<Mutex<Ice>>,
    audio: Arc<AudioEngine>,
    screen: Arc<Screen>,
    settings: Arc<VoiceSettings>,
    signal_out: mpsc::UnboundedSender<(u64, SignalData)>,
    events: mpsc::UnboundedSender<VoiceEvent>,
    info: Arc<PeerInfo>,
    tx: mpsc::UnboundedSender<PeerMsg>,
}

/// What the encoder task needs to know, updated after each negotiation.
struct SendCfg {
    /// Opus's negotiated payload type.
    payload_type: AtomicU8,
    /// RED's, when we send RED (0 = plain Opus).
    red_pt: AtomicU8,
    /// Remote's maxaveragebitrate (0 = none given).
    remote_cap: AtomicU32,
    fec: AtomicBool,
    connected: AtomicBool,
}

/// Opus as we register it: the fmtp already says what we want to receive, so
/// our first offer carries it (webrtc-rs refuses munged local SDP).
fn opus_codec(bitrate: u32) -> RTCRtpCodec {
    RTCRtpCodec {
        mime_type: "audio/opus".to_owned(),
        clock_rate: 48_000,
        channels: 2,
        sdp_fmtp_line: sdp::tuned_fmtp(bitrate, false),
        rtcp_feedback: vec![],
    }
}

/// RED as we register it: one earlier Opus frame per packet (distance 1).
fn red_codec() -> RTCRtpCodec {
    RTCRtpCodec { mime_type: "audio/red".to_owned(), clock_rate: 48_000, channels: 2, sdp_fmtp_line: red::fmtp(OPUS_PT), rtcp_feedback: vec![] }
}

fn is_mime(c: &RTCRtpCodecParameters, mime: &str) -> bool {
    c.rtp_codec.mime_type.eq_ignore_ascii_case(mime)
}

/// `codecs` with `first` moved just in front of `then` (None: one of them
/// isn't there, or it's already ahead): RED before Opus on the voice.
fn put_before(codecs: &[RTCRtpCodecParameters], first: &str, then: &str) -> Option<Vec<RTCRtpCodecParameters>> {
    let a = codecs.iter().position(|c| is_mime(c, first))?;
    let b = codecs.iter().position(|c| is_mime(c, then))?;
    if a < b {
        return None;
    }
    let mut out = codecs.to_vec();
    let moved = out.remove(a);
    out.insert(b, moved);
    Some(out)
}

/// H.264 as we register it for screen video.
fn h264_codec() -> RTCRtpCodec {
    RTCRtpCodec { mime_type: "video/H264".to_owned(), clock_rate: 90_000, channels: 0, sdp_fmtp_line: H264_FMTP.to_owned(), rtcp_feedback: vec![] }
}

fn ice_servers(ice: &[IceServer]) -> Vec<RTCIceServer> {
    ice.iter()
        .filter_map(|s| {
            // turns: (TLS) isn't supported by the stack; plain turn: over UDP/TCP is.
            let urls: Vec<String> = s.url_list().into_iter().filter(|u| u.starts_with("stun:") || u.starts_with("turn:")).collect();
            (!urls.is_empty()).then(|| RTCIceServer {
                urls,
                username: s.username.clone().unwrap_or_default(),
                credential: s.credential.clone().unwrap_or_default(),
            })
        })
        .collect()
}

fn rtc_config(servers: &[IceServer], relay_only: bool) -> RTCConfiguration {
    RTCConfigurationBuilder::new()
        .with_ice_servers(ice_servers(servers))
        .with_ice_transport_policy(if relay_only { RTCIceTransportPolicy::Relay } else { RTCIceTransportPolicy::All })
        .with_bundle_policy(RTCBundlePolicy::MaxBundle)
        .build()
}

async fn build_pc(ctx: &PeerCtx, handler: Arc<Handler>, with_ice: bool) -> Result<Arc<dyn PeerConnection>, String> {
    let mut media = MediaEngine::default();
    // RED first, then Opus (a screen share's audio takes its codecs from its
    // Opus track: Opus first there, RED after it).
    if red_enabled() {
        media.register_codec(RTCRtpCodecParameters { rtp_codec: red_codec(), payload_type: RED_PT }, RtpCodecKind::Audio).map_err(|e| e.to_string())?;
    }
    media
        .register_codec(
            RTCRtpCodecParameters { rtp_codec: opus_codec(ctx.settings.bitrate.load(Ordering::Relaxed)), payload_type: OPUS_PT },
            RtpCodecKind::Audio,
        )
        .map_err(|e| e.to_string())?;
    media
        .register_header_extension(
            RTCRtpHeaderExtensionCapability { uri: AUDIO_LEVEL_URI.to_string() },
            RtpCodecKind::Audio,
            Some(RTCRtpTransceiverDirection::Sendrecv),
        )
        .map_err(|e| e.to_string())?;
    // Screen video: H.264 only (every client takes it). Keyframes on request
    // (PLI, FIR), NACK (below), and transport-cc on video only, so a browser
    // sharing to us can tell how much it can send.
    media
        .register_codec(RTCRtpCodecParameters { rtp_codec: h264_codec(), payload_type: H264_PT }, RtpCodecKind::Video)
        .map_err(|e| e.to_string())?;
    media.register_feedback(RTCPFeedback { typ: "ccm".to_owned(), parameter: "fir".to_owned() }, RtpCodecKind::Video);
    media.register_feedback(RTCPFeedback { typ: "transport-cc".to_owned(), parameter: String::new() }, RtpCodecKind::Video);
    media
        .register_header_extension(RTCRtpHeaderExtensionCapability { uri: rtc::sdp::extmap::TRANSPORT_CC_URI.to_owned() }, RtpCodecKind::Video, None)
        .map_err(|e| e.to_string())?;
    // `register_default_interceptors` minus transport-wide congestion control
    // on audio: that negotiates transport-cc there, which only makes browsers
    // pad the stream with bandwidth probes (PROTOCOL.md › Voice › No
    // transport-cc on audio). Receiver/sender reports stay: the encoder's loss
    // hint reads them. The transport-cc receiver only answers what carries its
    // header extension, i.e. screen video. Plus our own reader of their report
    // blocks and keyframe requests, near the wire end of the read path.
    let registry = configure_rtcp_reports(configure_nack(Registry::new(), &mut media))
        .with(Slot::TwccReceiver, TwccReceiverBuilder::new().build())
        .with(Slot::from(8_500usize), loss::LossWatch::new(ctx.loss.clone()));
    let (servers, relay_only) = {
        let i = ctx.ice.lock();
        (if with_ice { i.servers.clone() } else { Vec::new() }, i.relay_only)
    };
    let pc = PeerConnectionBuilder::new()
        .with_configuration(rtc_config(&servers, relay_only))
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        // An ICE restart re-gathers on fresh sockets: after a network change the
        // old candidates name addresses that may be gone.
        .with_setting_engine(SettingEngineBuilder::new().with_discard_local_candidates_during_ice_restart(true).build())
        .with_handler(handler)
        .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
        .build()
        .await
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(pc))
}

struct Handler {
    info: Arc<PeerInfo>,
    tx: mpsc::UnboundedSender<PeerMsg>,
    signal_out: mpsc::UnboundedSender<(u64, SignalData)>,
    audio: Arc<AudioEngine>,
    screen: Arc<Screen>,
    settings: Arc<VoiceSettings>,
    volume: f32,
    /// Our m-line's mid, for outgoing candidates (the stack leaves it blank).
    mid: Arc<Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_negotiation_needed(&self) {
        let _ = self.tx.send(PeerMsg::NegotiationNeeded);
    }

    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        let Ok(init) = event.candidate.to_json() else { return };
        if init.candidate.is_empty() {
            return; // never send end-of-candidates
        }
        let mid = self.mid.lock().clone().unwrap_or_else(|| "0".into());
        let data = SignalData {
            sdp: None,
            candidate: Some(CandidateJson {
                candidate: init.candidate,
                sdp_mid: Some(mid),
                sdp_mline_index: Some(0),
                username_fragment: init.username_fragment,
            }),
        };
        let _ = self.signal_out.send((self.info.id, data));
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        let _ = self.tx.send(PeerMsg::State(state));
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let kind = track.kind().await;
        let (info, audio, screen, settings, volume) = (self.info.clone(), self.audio.clone(), self.screen.clone(), self.settings.clone(), self.volume);
        tokio::spawn(async move {
            // The first event says which transceiver (and SSRC) this is.
            let mut first = None;
            let (transceiver, ssrc) = match track.poll().await {
                Some(TrackRemoteEvent::OnOpen(init)) => (Some(usize::from(init.receiver_id)), init.ssrc),
                Some(TrackRemoteEvent::OnEnded) | None => return,
                Some(other) => {
                    first = Some(other);
                    (None, 0)
                }
            };
            if kind == RtpCodecKind::Video {
                // Someone's screen (we asked for it).
                let i = info.clone();
                log::info!("peer {}: their screen's video arrived", info.sid());
                crate::screen::receive(track, ssrc, move || i.sid(), screen.watch.clone()).await;
                return;
            }
            // The voice is always the first transceiver; any later audio is a screen share's sound.
            let voice = transceiver.is_none_or(|t| t == 0);
            let key = if voice { info.sid() } else { screen_key(&info.sid()) };
            let vol = if voice { volume } else { settings.screen_volumes.lock().get(&info.sid()).copied().unwrap_or(1.0) };
            if !voice {
                log::info!("peer {}: their screen's sound arrived", info.sid());
            }
            let stream = audio.add_stream(&key, vol);
            let mut next = first;
            loop {
                let ev = match next.take() {
                    Some(ev) => ev,
                    None => match track.poll().await {
                        Some(ev) => ev,
                        None => break,
                    },
                };
                match ev {
                    // Padding-only packets (empty payload) are counted and ignored
                    // (the jitter buffer goes by timestamp, so they aren't loss).
                    // RED packets carry the frame before too (PROTOCOL.md › Voice › Redundancy).
                    TrackRemoteEvent::OnRtpPacket(p) if info.is_red(p.header.payload_type) => stream.push_red(p.header.sequence_number, p.header.timestamp, &p.payload),
                    TrackRemoteEvent::OnRtpPacket(p) => stream.push_rtp(p.header.sequence_number, p.header.timestamp, &p.payload),
                    TrackRemoteEvent::OnEnded => break,
                    _ => {}
                }
            }
            if !voice {
                // (Unless a newer screen sound replaced it meanwhile.)
                if audio.stream(&screen_key(&info.sid())).is_some_and(|s| Arc::ptr_eq(&s, &stream)) {
                    audio.remove_stream(&screen_key(&info.sid()));
                }
            }
        });
    }
}

/// Our screen share going out on one connection: the video track (and the
/// sound's, when it's on), with the tasks that feed them.
struct ScreenOut {
    video: Arc<dyn RtpSender>,
    video_pt: Arc<AtomicU8>,
    sound: Option<SoundOut>,
    writer: tokio::task::JoinHandle<()>,
}

/// The share's sound going out: its sender, negotiated payload type, and feeder.
struct SoundOut {
    sender: Arc<dyn RtpSender>,
    pt: Arc<AtomicU8>,
    writer: tokio::task::JoinHandle<()>,
}

/// A local track in the share's stream (`a=msid:<share> <track>`): the web
/// takes a track that arrives with a stream for a screen share.
fn screen_track(stream: &str, kind: RtpCodecKind, ssrc: u32, codec: RTCRtpCodec) -> Option<Arc<TrackLocalStaticSample>> {
    let what = if kind == RtpCodecKind::Video { "video" } else { "sound" };
    let track = MediaStreamTrack::new(
        stream.to_owned(),
        format!("yap-screen-{what}-{}", crate::config::random_hex(6)),
        format!("yap-screen-{what}"),
        kind,
        vec![RTCRtpEncodingParameters { rtp_coding_parameters: RTCRtpCodingParameters { ssrc: Some(ssrc), ..Default::default() }, codec, ..Default::default() }],
    );
    TrackLocalStaticSample::new(Instant::now(), track).map(Arc::new).map_err(|e| log::error!("screen track: {e}")).ok()
}

/// Forward our encoded pictures to one viewer, packetized here so the RTP
/// timestamps follow the real clock (a picture repeated after a quiet half
/// second is half a second later). Starts at a keyframe; asks for one when
/// they do, or when it fell behind.
async fn run_video_writer(mut frames: broadcast::Receiver<Arc<EncodedVideo>>, track: Arc<TrackLocalStaticSample>, ssrc: u32, pt: Arc<AtomicU8>, ctx: PeerCtx) {
    let share = ctx.screen.share.clone();
    let mut packetizer = rtc::rtp::packetizer::new_packetizer(
        Instant::now(),
        1200,
        0,
        ssrc,
        Box::new(rtc::rtp::codec::h264::H264Payloader::default()),
        Box::new(rtc::rtp::sequence::new_random_sequencer()),
        90_000,
    );
    let mut started = false;
    let mut asked = false;
    let mut origin: Option<(Instant, u32)> = None;
    let mut reports = 0;
    loop {
        let f = match frames.recv().await {
            Ok(f) => f,
            Err(broadcast::error::RecvError::Lagged(_)) => {
                started = false;
                share.request_keyframe();
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let payload = pt.load(Ordering::Relaxed);
        if payload == 0 || !ctx.info.sending_screen.load(Ordering::Relaxed) {
            continue; // not negotiated yet
        }
        if !asked {
            // Negotiated: start them on a fresh keyframe.
            asked = true;
            share.request_keyframe();
        }
        if ctx.loss.keyframe.swap(false, Ordering::Relaxed) {
            share.request_keyframe();
        }
        let (fraction, n) = ctx.loss.video();
        if n != reports {
            reports = n;
            share.report_loss(ctx.info.id, fraction);
        }
        if !started {
            if !f.keyframe {
                continue;
            }
            started = true;
        }
        // 90 kHz from the first picture, by the clock.
        let (t0, base) = *origin.get_or_insert((f.at, crate::config::random_u32()));
        let ts = base.wrapping_add((f.at.saturating_duration_since(t0).as_secs_f64() * 90_000.0) as u32);
        let packets = match packetizer.packetize(f.at, &f.data, 0) {
            Ok(p) => p,
            Err(e) => {
                log::debug!("screen: packetize: {e}");
                continue;
            }
        };
        for mut pkt in packets {
            pkt.header.payload_type = payload;
            pkt.header.timestamp = ts;
            if let Err(e) = track.write_rtp(pkt).await {
                log::debug!("peer {}: screen video: {e}", ctx.info.sid());
                break;
            }
        }
    }
}

/// Forward our share's sound (20 ms Opus packets) to one viewer.
async fn run_sound_writer(mut packets: broadcast::Receiver<Arc<EncodedAudio>>, track: Arc<TrackLocalStaticSample>, ssrc: u32, pt: Arc<AtomicU8>) {
    loop {
        let a = match packets.recv().await {
            Ok(a) => a,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let payload = pt.load(Ordering::Relaxed);
        if payload == 0 {
            continue;
        }
        let sample = Sample { data: a.data.clone(), duration: Duration::from_millis(20), ..Sample::new(Instant::now()) };
        let _ = track.sample_writer(ssrc, payload).write_sample(&sample).await;
    }
}

struct Peer {
    ctx: PeerCtx,
    pc: Arc<dyn PeerConnection>,
    track: Arc<TrackLocalStaticSample>,
    sender: Option<Arc<dyn RtpSender>>,
    cfg: Arc<SendCfg>,
    mid: Arc<Mutex<Option<String>>>,
    ignore_offer: bool,
    state: RTCPeerConnectionState,
    /// When to try bringing a dead path back (an ICE restart).
    recover_at: Option<tokio::time::Instant>,
    /// Smoothed share of our packets they lose (0–1), and how many reports that's from.
    loss: f64,
    reports: u32,
    /// Our screen share going out to them, while it does.
    screen_out: Option<ScreenOut>,
}

async fn run_peer(ctx: PeerCtx, mut rx: mpsc::UnboundedReceiver<PeerMsg>) {
    let mid = Arc::new(Mutex::new(None));
    let handler = Arc::new(Handler {
        info: ctx.info.clone(),
        tx: ctx.tx.clone(),
        signal_out: ctx.signal_out.clone(),
        audio: ctx.audio.clone(),
        screen: ctx.screen.clone(),
        settings: ctx.settings.clone(),
        volume: ctx.volume,
        mid: mid.clone(),
    });
    let relay_only = ctx.ice.lock().relay_only;
    let pc = match build_pc(&ctx, handler.clone(), true).await {
        Ok(pc) => pc,
        Err(e) if relay_only => {
            // Never fall back to host candidates: that's exactly the IP address it hides.
            log::error!("peer {}: couldn't build a relay-only connection: {e}", ctx.info.sid());
            set_state(&ctx, "failed");
            return;
        }
        Err(e) => {
            log::warn!("peer {}: couldn't build with ICE servers ({e}); retrying with host candidates only", ctx.info.sid());
            match build_pc(&ctx, handler, false).await {
                Ok(pc) => pc,
                Err(e) => {
                    log::error!("peer {}: {e}", ctx.info.sid());
                    set_state(&ctx, "failed");
                    return;
                }
            }
        }
    };
    let ssrc = crate::config::random_u32();
    ctx.loss.ssrc.store(ssrc, Ordering::Relaxed);
    // Stream id "-" (`a=msid:- <track>`): the voice track belongs to no stream,
    // like a browser's `addTransceiver(track)`. Web clients treat a track that
    // arrives with a stream as someone's screen share (video + tab audio).
    let track = MediaStreamTrack::new(
        "-".to_owned(),
        format!("yap-audio-{}", crate::config::random_hex(6)),
        "yap-mic".to_owned(),
        RtpCodecKind::Audio,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters { ssrc: Some(ssrc), ..Default::default() },
            codec: opus_codec(ctx.settings.bitrate.load(Ordering::Relaxed)),
            ..Default::default()
        }],
    );
    let track = match TrackLocalStaticSample::new(Instant::now(), track) {
        Ok(t) => Arc::new(t),
        Err(e) => {
            log::error!("peer {}: track: {e}", ctx.info.sid());
            let _ = pc.close().await;
            return;
        }
    };
    let cfg = Arc::new(SendCfg {
        payload_type: AtomicU8::new(OPUS_PT),
        red_pt: AtomicU8::new(0),
        remote_cap: AtomicU32::new(0),
        fec: AtomicBool::new(true),
        connected: AtomicBool::new(false),
    });
    let encoder = tokio::spawn(run_encoder(ctx.audio.subscribe(), track.clone(), ssrc, cfg.clone(), ctx.settings.clone(), ctx.info.clone()));
    let mut peer = Peer { ctx, pc, track, sender: None, cfg, mid, ignore_offer: false, state: RTCPeerConnectionState::New, recover_at: None, loss: 0.0, reports: 0, screen_out: None };
    log::info!(
        "peer {}: created ({}, {})",
        peer.sid(),
        if peer.ctx.initiator { "offering" } else { "answering" },
        if peer.ctx.info.polite() { "polite" } else { "impolite" }
    );

    if peer.ctx.initiator {
        // Our single sendrecv transceiver, then the offer.
        match peer.pc.add_track(peer.track.clone() as Arc<dyn TrackLocal>).await {
            Ok(s) => peer.sender = Some(s),
            Err(e) => log::error!("peer {}: add_track: {e}", peer.sid()),
        }
        peer.make_offer().await;
    }

    let mut stats = tokio::time::interval(STATS_EVERY);
    stats.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let recover = peer.recover_at;
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(PeerMsg::Signal(data)) => peer.handle_signal(data).await,
                Some(PeerMsg::NegotiationNeeded) => peer.on_negotiation_needed().await,
                Some(PeerMsg::Renegotiate) => peer.make_offer().await,
                Some(PeerMsg::ScreenSync) => peer.sync_screen().await,
                Some(PeerMsg::State(state)) => peer.on_state(state),
                Some(PeerMsg::Resync { sent }) => peer.resync(&sent).await,
                Some(PeerMsg::Close) | None => break,
            },
            _ = async { tokio::time::sleep_until(recover.unwrap()).await }, if recover.is_some() => peer.recover().await,
            _ = stats.tick() => peer.poll_stats().await,
        }
    }
    encoder.abort();
    if let Some(out) = peer.screen_out.take() {
        out.writer.abort();
        if let Some(sound) = out.sound {
            sound.writer.abort();
        }
        peer.ctx.screen.share.remove_viewer(peer.ctx.info.id);
    }
    let _ = peer.pc.close().await;
    set_state(&peer.ctx, "closed");
    log::info!("peer {}: closed", peer.sid());
}

fn set_state(ctx: &PeerCtx, state: &str) {
    *ctx.info.state.lock() = state.to_string();
    let _ = ctx.events.send(VoiceEvent::PeerState { sid: ctx.info.sid(), state: state.to_string() });
}

fn state_name(s: RTCPeerConnectionState) -> &'static str {
    match s {
        RTCPeerConnectionState::New => "new",
        RTCPeerConnectionState::Connecting => "connecting",
        RTCPeerConnectionState::Connected => "connected",
        RTCPeerConnectionState::Disconnected => "disconnected",
        RTCPeerConnectionState::Failed => "failed",
        RTCPeerConnectionState::Closed => "closed",
        _ => "new",
    }
}

/// The encoder's packet-loss setting for a loss rate (0–1), with hysteresis
/// so it doesn't flip back and forth: libwebrtc's levels (0, 1, 5, 10, 20 %),
/// each entered a little above its threshold and left a little below it.
/// 0 keeps Opus in CELT (clean networks, a third of Hybrid's CPU); anything
/// else makes it carry in-band FEC (Hybrid).
pub fn loss_level(loss: f64, current: u8) -> u8 {
    const LEVELS: [(f64, f64, u8); 4] = [(0.20, 0.02, 20), (0.10, 0.01, 10), (0.05, 0.01, 5), (0.01, 0.0, 1)];
    for (at, margin, level) in LEVELS {
        // Going up to a level needs a bit more than its threshold; staying needs a bit less.
        let need = if current >= level { at - margin } else { at + margin };
        if loss >= need {
            return level;
        }
    }
    0
}

impl Peer {
    async fn is_stable(&self) -> bool {
        self.pc.pending_local_description().await.is_none() && self.pc.pending_remote_description().await.is_none()
    }

    fn bitrate(&self) -> u32 {
        self.ctx.settings.bitrate.load(Ordering::Relaxed)
    }

    fn sid(&self) -> String {
        self.ctx.info.sid()
    }

    fn send_sdp(&self, kind: &str, sdp: String) {
        let data = SignalData { sdp: Some(SdpJson { kind: kind.to_string(), sdp }), candidate: None };
        let _ = self.ctx.signal_out.send((self.ctx.info.id, data));
    }

    fn on_state(&mut self, state: RTCPeerConnectionState) {
        let name = state_name(state);
        log::info!("peer {}: {name}", self.sid());
        self.state = state;
        set_state(&self.ctx, name);
        self.cfg.connected.store(state == RTCPeerConnectionState::Connected, Ordering::Relaxed);
        self.watch_path(state);
    }

    /// The path died (Wi-Fi → cellular, a NAT rebinding, a sleeping laptop):
    /// the impolite side restarts ICE after 3.5 s of `disconnected` (at once
    /// on `failed`); the polite side after 10 s, in case the other app only
    /// restarts on `failed`. Then again every 10 s; after the third try with
    /// someone whose app can start over, the connection is rebuilt.
    fn watch_path(&mut self, state: RTCPeerConnectionState) {
        let info = &self.ctx.info;
        self.recover_at = None;
        match state {
            RTCPeerConnectionState::Connected => {
                info.restarts.store(0, Ordering::Relaxed);
                info.failed.store(false, Ordering::Relaxed);
                *info.connected_at.lock() = Some(Instant::now());
                return;
            }
            RTCPeerConnectionState::Failed => info.failed.store(true, Ordering::Relaxed),
            RTCPeerConnectionState::Disconnected => {}
            _ => return,
        }
        let wait = if info.polite() {
            RESTART_POLITE
        } else if state == RTCPeerConnectionState::Failed {
            Duration::ZERO
        } else {
            RESTART_IMPOLITE
        };
        self.recover_at = Some(tokio::time::Instant::now() + wait);
    }

    async fn recover(&mut self) {
        self.recover_at = None;
        if !matches!(self.state, RTCPeerConnectionState::Disconnected | RTCPeerConnectionState::Failed) {
            return;
        }
        let info = self.ctx.info.clone();
        let tries = info.restarts.fetch_add(1, Ordering::Relaxed) + 1;
        if tries > RESTARTS_BEFORE_REBUILD && !info.polite() && info.resumable.load(Ordering::Relaxed) {
            log::info!("peer {}: still down after {RESTARTS_BEFORE_REBUILD} ICE restarts; starting over", self.sid());
            let _ = self.ctx.events.send(VoiceEvent::Rebuild { sid: self.sid() });
            return;
        }
        let (servers, relay_only) = Ice::fresh(&self.ctx.ice).await;
        if let Err(e) = self.pc.set_configuration(rtc_config(&servers, relay_only)).await {
            log::debug!("peer {}: set_configuration: {e}", self.sid());
        }
        log::info!("peer {}: restarting ICE (try {tries}, {})", self.sid(), if info.polite() { "polite" } else { "impolite" });
        let _ = self.ctx.events.send(VoiceEvent::IceRestart { sid: self.sid(), attempt: tries });
        if let Err(e) = self.pc.restart_ice().await {
            log::warn!("peer {}: restart_ice: {e}", self.sid());
        }
        self.make_offer().await;
        self.recover_at = Some(tokio::time::Instant::now() + RESTART_AGAIN);
    }

    /// Round trip, and how much of what we send they lose (→ the encoder's loss hint).
    async fn poll_stats(&mut self) {
        if self.state != RTCPeerConnectionState::Connected {
            return;
        }
        let report = self.pc.get_stats(Instant::now(), StatsSelector::None).await;
        let mut rtt = None;
        let mut pair = None;
        for e in report.iter() {
            match e {
                RTCStatsReportEntry::IceCandidatePair(p) if p.nominated && p.state == RTCStatsIceCandidatePairState::Succeeded => {
                    pair = Some((p.local_candidate_id.clone(), p.remote_candidate_id.clone()));
                    if p.current_round_trip_time > 0.0 {
                        rtt = Some(p.current_round_trip_time * 1000.0);
                    }
                }
                RTCStatsReportEntry::RemoteInboundRtp(r) if r.round_trip_time > 0.0 && rtt.is_none() => {
                    rtt = Some(r.round_trip_time * 1000.0);
                }
                _ => {}
            }
        }
        // The pair in use, by candidate type only (the call report; never an address).
        if let Some((local, remote)) = pair {
            let kind = |id: &str, local: bool| {
                report.iter().find_map(|e| match e {
                    // (The pair names "candidate:…"; the candidate's own id is "RTCLocalIceCandidate_candidate:…".)
                    RTCStatsReportEntry::LocalCandidate(c) if local && c.stats.id.ends_with(id) => crate::core::report::candidate_kind(&c.candidate_type.to_string()),
                    RTCStatsReportEntry::RemoteCandidate(c) if !local && c.stats.id.ends_with(id) => crate::core::report::candidate_kind(&c.candidate_type.to_string()),
                    _ => None,
                })
            };
            // A remote candidate they never signalled (learned from their checks) isn't in the
            // stats at all: that's what peer-reflexive means.
            if let (Some(l), Some(r)) = (kind(&local, true), kind(&remote, false).or(Some("prflx"))) {
                *self.ctx.info.path.lock() = Some((l, r));
            }
        }
        // A fresh report from them about our stream (see loss.rs).
        let (fraction, reports) = self.ctx.loss.latest();
        let lost = (reports != self.reports).then_some(fraction);
        self.reports = reports;
        let info = &self.ctx.info;
        if reports > 0 {
            info.their_fraction.store((fraction * 1000.0).round() as u32, Ordering::Relaxed);
            info.their_lost.store(self.ctx.loss.total_lost() as u64, Ordering::Relaxed);
            info.their_reports.store(reports, Ordering::Relaxed);
        }
        if let Some(r) = rtt {
            info.rtt_ms.store(r.round().max(1.0) as u32, Ordering::Relaxed);
        }
        if let Some(l) = lost.filter(|l| l.is_finite()) {
            // Up at once, down gently (a report comes every second or few).
            self.loss = l.max(self.loss * 0.7 + l * 0.3).clamp(0.0, 1.0);
            info.loss_permille.store((self.loss * 1000.0).round() as u32, Ordering::Relaxed);
            let level = loss_level(self.loss, info.loss_hint.load(Ordering::Relaxed));
            if info.loss_hint.swap(level, Ordering::Relaxed) != level {
                log::info!("peer {}: they lose {:.1}% of our packets → encoder loss hint {level}%", self.sid(), self.loss * 100.0);
            }
        }
    }

    /// After a reconnect/resume: if an offer of ours is still unanswered (its
    /// answer may have gone to our old session), send it again.
    async fn resync(&self, sent: &[String]) {
        if let Some(local) = self.pc.pending_local_description().await
            && local.sdp_type == RTCSdpType::Offer
            && !sent.contains(&local.sdp)
        {
            log::info!("peer {}: re-sending our unanswered offer", self.sid());
            self.send_sdp("offer", local.sdp);
        }
    }

    /// The voice's transceiver: always the first one (id 0), whoever offered.
    /// (`get_transceivers` comes out of a HashMap, in no particular order:
    /// with a screen share's transceivers there, "the first in the list" is
    /// any of them.)
    async fn voice_transceiver(&self) -> Option<Arc<dyn RtpTransceiver>> {
        self.pc.get_transceivers().await.into_iter().find(|t| t.id() == 0)
    }

    async fn after_local_description(&self) {
        if let Some(t) = self.voice_transceiver().await
            && let Ok(Some(mid)) = t.mid().await
        {
            *self.mid.lock() = Some(mid);
        }
        if let Some(local) = self.pc.local_description().await {
            self.ctx.info.local_twcc.store(local.sdp.contains("transport-cc"), Ordering::Relaxed);
            if let Some(f) = sdp::opus_fmtp(&local.sdp) {
                *self.ctx.info.local_fmtp.lock() = f;
            }
        }
    }

    async fn after_negotiation(&self) {
        // Payload types the remote agreed to (rtc's write_rtp requires the negotiated ones):
        // Opus's, and on the voice RED's if both sides do it. Screen video's is its first.
        let negotiated = |sender: Arc<dyn RtpSender>, mime: Option<&'static str>| async move {
            let codecs = sender.get_parameters().await.ok()?.rtp_parameters.codecs;
            match mime {
                Some(m) => codecs.iter().find(|c| is_mime(c, m)).map(|c| c.payload_type),
                None => codecs.first().map(|c| c.payload_type),
            }
        };
        if let Some(sender) = &self.sender {
            if let Some(pt) = negotiated(sender.clone(), Some("audio/opus")).await {
                self.cfg.payload_type.store(pt, Ordering::Relaxed);
            }
            let red = if red_enabled() { negotiated(sender.clone(), Some("audio/red")).await.unwrap_or(0) } else { 0 };
            if self.cfg.red_pt.swap(red, Ordering::Relaxed) != red {
                log::info!("peer {}: sending {}", self.sid(), if red > 0 { "red+opus (each packet also carries the frame before)" } else { "plain opus" });
            }
            self.ctx.info.send_red.store(red > 0, Ordering::Relaxed);
        }
        // What they may send us as RED: the payload type in our description and in theirs.
        let local = self.pc.local_description().await.and_then(|d| sdp::red_pt(&d.sdp)).unwrap_or(0);
        let remote = self.pc.remote_description().await.and_then(|d| sdp::red_pt(&d.sdp)).unwrap_or(0);
        self.ctx.info.red_pts.store(((remote as u16) << 8) | local as u16, Ordering::Relaxed);
        if let (Some(l), Some(r)) = (self.pc.local_description().await, self.pc.remote_description().await) {
            log::info!("peer {}: voice codecs: ours {:?}, theirs {:?}", self.sid(), sdp::voice_codecs(&l.sdp), sdp::voice_codecs(&r.sdp));
        }
        // The same for our screen share's tracks: they start once it's known.
        if let Some(out) = &self.screen_out {
            if let Some(pt) = negotiated(out.video.clone(), None).await {
                out.video_pt.store(pt, Ordering::Relaxed);
            }
            if let Some(sound) = &out.sound
                && let Some(p) = negotiated(sound.sender.clone(), Some("audio/opus")).await
            {
                sound.pt.store(p, Ordering::Relaxed);
            }
        }
    }

    /// Send our screen to them while we share and they want it; stop when
    /// either ends; add or drop the share's sound as it's switched.
    async fn sync_screen(&mut self) {
        let share = self.ctx.screen.share.clone();
        let want = share.sharing() && self.ctx.info.wants_screen.load(Ordering::Relaxed);
        let sound = want && share.sound();
        let mut changed = false;
        if want && self.screen_out.is_none() {
            let ssrc = crate::config::random_u32();
            let Some(track) = screen_track(&share.stream_id, RtpCodecKind::Video, ssrc, h264_codec()) else { return };
            let sender = match self.pc.add_track(track.clone() as Arc<dyn TrackLocal>).await {
                Ok(s) => s,
                Err(e) => return log::warn!("peer {}: add screen video: {e}", self.sid()),
            };
            self.ctx.loss.video_ssrc.store(ssrc, Ordering::Relaxed);
            self.ctx.info.sending_screen.store(true, Ordering::Relaxed);
            let video_pt = Arc::new(AtomicU8::new(0));
            let writer = tokio::spawn(run_video_writer(share.video.subscribe(), track, ssrc, video_pt.clone(), self.ctx.clone()));
            self.screen_out = Some(ScreenOut { video: sender, video_pt, sound: None, writer });
            share.add_viewer(self.ctx.info.id);
            log::info!("peer {}: sending our screen", self.sid());
            changed = true;
        } else if !want && let Some(out) = self.screen_out.take() {
            out.writer.abort();
            if let Err(e) = self.pc.remove_track(&out.video).await {
                log::debug!("peer {}: remove screen video: {e}", self.sid());
            }
            if let Some(sound) = out.sound {
                sound.writer.abort();
                let _ = self.pc.remove_track(&sound.sender).await;
            }
            self.ctx.loss.video_ssrc.store(0, Ordering::Relaxed);
            self.ctx.info.sending_screen.store(false, Ordering::Relaxed);
            share.remove_viewer(self.ctx.info.id);
            log::info!("peer {}: no longer sending our screen", self.sid());
            changed = true;
        }
        let bitrate = self.bitrate();
        if let Some(out) = self.screen_out.as_mut() {
            if sound && out.sound.is_none() {
                // Encoded once as stereo music at 160 kbps (crate::screen); the
                // track's codec is the registered Opus, as the stack requires.
                let codec = opus_codec(bitrate);
                let ssrc = crate::config::random_u32();
                if let Some(track) = screen_track(&share.stream_id, RtpCodecKind::Audio, ssrc, codec) {
                    match self.pc.add_track(track.clone() as Arc<dyn TrackLocal>).await {
                        Ok(sender) => {
                            let pt = Arc::new(AtomicU8::new(0));
                            let writer = tokio::spawn(run_sound_writer(share.audio.subscribe(), track, ssrc, pt.clone()));
                            out.sound = Some(SoundOut { sender, pt, writer });
                            changed = true;
                        }
                        Err(e) => log::warn!("peer {}: add screen sound: {e}", self.sid()),
                    }
                }
            } else if !sound && let Some(s) = out.sound.take() {
                s.writer.abort();
                let _ = self.pc.remove_track(&s.sender).await;
                changed = true;
            }
        }
        if changed {
            self.make_offer().await;
        }
    }

    /// Our local descriptions should already carry the tuned fmtp; say so if not.
    fn check_tuned(&self, local_sdp: &str) {
        let tuned = sdp::tune_opus(local_sdp, self.bitrate(), false);
        if tuned.trim_end() != local_sdp.trim_end() {
            log::info!(
                "peer {}: our Opus fmtp is {:?} (wanted {:?}); a mid-call quality change applies to receiving on the next join",
                self.sid(),
                sdp::opus_fmtp(local_sdp).unwrap_or_default(),
                sdp::tuned_fmtp(self.bitrate(), false)
            );
        }
    }

    /// webrtc-rs's own negotiationneeded, from `add_track`. The offering side
    /// already offered right after adding the track, so this one is late: when
    /// the far side answers within a few ms (a LAN), it lands after that answer
    /// and a second, identical offer went out (same ICE credentials and
    /// fingerprint, now with candidates; seen in 1 of 4 iPhone↔desktop runs).
    /// Harmless, but a needless round trip that could cross a real one. So:
    /// if our track already goes out in the negotiated description, there's
    /// nothing to offer. (ICE restarts and quality changes offer by
    /// themselves; they never wait for this event.)
    async fn on_negotiation_needed(&mut self) {
        use webrtc::media_stream::Track as _;
        // What we mean to send: the mic, and our screen's tracks while they go out.
        let mut ours = vec![self.track.track_id().await];
        if let Some(out) = &self.screen_out {
            ours.push(out.video.track().track_id().await);
            if let Some(s) = &out.sound {
                ours.push(s.sender.track().track_id().await);
            }
        }
        if self.is_stable().await
            && let Some(current) = self.pc.current_local_description().await
            && ours.iter().all(|t| sdp::sends_track(&current.sdp, t))
            && sdp::sending_tracks(&current.sdp) == ours.len()
        {
            log::debug!("peer {}: negotiationneeded with nothing left to negotiate: no second offer", self.sid());
            return;
        }
        self.make_offer().await;
    }

    /// RED first on the voice, then Opus (PROTOCOL.md › Voice › Redundancy),
    /// in every offer and answer: browsers send RED only when it leads. The
    /// first offer lists both (ours); after that, what was negotiated, so an
    /// app without RED never gets it in an answer. A screen share's sound the
    /// other way round, Opus first (RED after it only where they listed it):
    /// answering a browser's offer, webrtc-rs puts the codecs whose fmtp
    /// matches ours exactly first, and for a share's sound that's RED.
    async fn order_audio_codecs(&self) {
        if !red_enabled() {
            return;
        }
        let red = || RTCRtpCodecParameters { rtp_codec: red_codec(), payload_type: RED_PT };
        let opus = || RTCRtpCodecParameters { rtp_codec: opus_codec(self.bitrate()), payload_type: OPUS_PT };
        let remote = self.pc.remote_description().await;
        // (A transceiver made from their offer has no sender or receiver to ask yet, so a
        // screen's sound goes by what their description lists for its mid.)
        let theirs = remote.as_ref().map(|d| sdp::audio_sections(&d.sdp)).unwrap_or_default();
        for t in self.pc.get_transceivers().await {
            let voice = t.id() == 0;
            let want = if voice {
                let Some(sender) = &self.sender else { continue };
                if remote.is_none() {
                    Some(vec![red(), opus()])
                } else {
                    let Ok(params) = sender.get_parameters().await else { continue };
                    put_before(&params.rtp_parameters.codecs, "audio/red", "audio/opus")
                }
            } else {
                let Ok(Some(mid)) = t.mid().await else { continue };
                let Some((_, listed)) = theirs.iter().find(|(m, _)| m.as_deref() == Some(mid.as_str())) else { continue };
                listed.iter().any(|c| c == "opus").then(|| {
                    let mut codecs = vec![opus()];
                    if listed.iter().any(|c| c == "red") {
                        codecs.push(red());
                    }
                    codecs
                })
            };
            if let Some(codecs) = want
                && let Err(e) = t.set_codec_preferences(codecs).await
            {
                log::debug!("peer {}: {} first: {e}", self.sid(), if voice { "RED" } else { "Opus" });
            }
        }
    }

    async fn make_offer(&mut self) {
        if !self.is_stable().await {
            return;
        }
        self.order_audio_codecs().await;
        let offer = match self.pc.create_offer(None).await {
            Ok(o) => o,
            Err(e) => return log::warn!("peer {}: create_offer: {e}", self.sid()),
        };
        self.check_tuned(&offer.sdp);
        let generated = offer.sdp.clone();
        if let Err(e) = self.pc.set_local_description(offer).await {
            return log::warn!("peer {}: set_local_description(offer): {e}", self.sid());
        }
        self.after_local_description().await;
        let sdp = self.pc.local_description().await.map(|d| d.sdp).unwrap_or(generated);
        self.send_sdp("offer", sdp);
    }

    async fn handle_signal(&mut self, data: SignalData) {
        if let Some(desc) = data.sdp {
            let is_offer = desc.kind == "offer";
            let collision = is_offer && !self.is_stable().await;
            self.ignore_offer = !self.ctx.info.polite() && collision;
            if self.ignore_offer {
                log::info!("peer {}: ignoring colliding offer (impolite)", self.sid());
                return;
            }
            if collision {
                // Polite: roll back our offer and take theirs.
                if let Ok(rb) = RTCSessionDescription::rollback(None)
                    && let Err(e) = self.pc.set_local_description(rb).await
                {
                    log::warn!("peer {}: rollback: {e}", self.sid());
                }
            }
            // webrtc-rs builds its answers (and re-offers) from the codec parameters
            // it negotiated from the *remote* description, and refuses munged local
            // SDP. So we put our receive wishes into the remote's Opus fmtp before
            // applying it (and take transport-cc off its audio, so our answer can't
            // carry it); the peer's real wishes are read from the original below.
            let applied = sdp::tune_opus(&desc.sdp, self.bitrate(), false);
            let remote = match desc.kind.as_str() {
                "offer" => RTCSessionDescription::offer(applied),
                "answer" => RTCSessionDescription::answer(applied),
                "pranswer" => RTCSessionDescription::pranswer(applied),
                _ => return,
            };
            let remote = match remote {
                Ok(r) => r,
                Err(e) => return log::warn!("peer {}: remote {} didn't parse: {e}", self.sid(), desc.kind),
            };
            if let Err(e) = self.pc.set_remote_description(remote).await {
                return log::warn!("peer {}: set_remote_description({}): {e}", self.sid(), desc.kind);
            }
            if let Some(fp) = sdp::fingerprint(&desc.sdp) {
                *self.ctx.info.remote_fp.lock() = Some(fp);
            }
            if let Some(f) = sdp::opus_fmtp(&desc.sdp) {
                let cap = sdp::fmtp_param(&f, "maxaveragebitrate").and_then(|v| v.parse().ok()).unwrap_or(0);
                self.cfg.remote_cap.store(cap, Ordering::Relaxed);
                self.cfg.fec.store(sdp::fmtp_param(&f, "useinbandfec").as_deref() != Some("0"), Ordering::Relaxed);
                *self.ctx.info.remote_fmtp.lock() = f;
            }
            if is_offer {
                // Answer with our mic on the transceiver they offered.
                if self.sender.is_none() {
                    match self.pc.add_track(self.track.clone() as Arc<dyn TrackLocal>).await {
                        Ok(s) => self.sender = Some(s),
                        Err(e) => log::warn!("peer {}: add_track: {e}", self.sid()),
                    }
                }
                self.order_audio_codecs().await;
                let answer = match self.pc.create_answer(None).await {
                    Ok(a) => a,
                    Err(e) => return log::warn!("peer {}: create_answer: {e}", self.sid()),
                };
                self.check_tuned(&answer.sdp);
                let generated = answer.sdp.clone();
                if let Err(e) = self.pc.set_local_description(answer).await {
                    return log::warn!("peer {}: set_local_description(answer): {e}", self.sid());
                }
                self.after_local_description().await;
                let sdp = self.pc.local_description().await.map(|d| d.sdp).unwrap_or(generated);
                self.send_sdp("answer", sdp);
            }
            self.after_negotiation().await;
        } else if let Some(c) = data.candidate {
            if c.candidate.is_empty() {
                return;
            }
            let init = RTCIceCandidateInit {
                candidate: c.candidate,
                sdp_mid: c.sdp_mid,
                sdp_mline_index: c.sdp_mline_index,
                username_fragment: c.username_fragment,
                url: None,
            };
            if let Err(e) = self.pc.add_ice_candidate(init).await
                && !self.ignore_offer
            {
                log::debug!("peer {}: add_ice_candidate: {e}", self.sid());
            }
        }
    }
}

/// An Opus encoder like a peer's: VoIP, complexity 10, VBR, no DTX, mono, 20 ms.
/// `loss` (%) 0 keeps it in CELT; above that it carries in-band FEC (Hybrid).
pub fn make_encoder(bitrate: u32, fec: bool, loss: u8) -> Option<ropus::Encoder> {
    ropus::Encoder::builder(crate::audio::RATE, ropus::Channels::Mono, ropus::Application::Voip)
        .bitrate(ropus::Bitrate::try_bits(bitrate).unwrap_or(ropus::Bitrate::Max))
        .complexity(10)
        .vbr(true)
        .dtx(false)
        .inband_fec(if fec { ropus::InbandFec::Enabled } else { ropus::InbandFec::Disabled })
        .packet_loss_perc(loss)
        .build()
        .map_err(|e| log::error!("opus encoder: {e:?}"))
        .ok()
}

/// Encode every 20 ms mic frame for this peer and send it, with the RFC 6464
/// audio-level extension so browsers show our speaking ring.
async fn run_encoder(
    mut frames: broadcast::Receiver<Arc<AudioFrame>>,
    track: Arc<TrackLocalStaticSample>,
    ssrc: u32,
    cfg: Arc<SendCfg>,
    settings: Arc<VoiceSettings>,
    info: Arc<PeerInfo>,
) {
    let mut current = (0u32, true);
    let mut loss = 0u8;
    let mut encoder: Option<ropus::Encoder> = None;
    let mut buf = vec![0u8; 4000];
    // RED: the frame before (sent again with this one) and the packet being packed.
    let mut prev: Vec<u8> = Vec::new();
    let mut packed: Vec<u8> = Vec::with_capacity(red::MAX_PAYLOAD);
    loop {
        let frame = match frames.recv().await {
            Ok(f) => f,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if !cfg.connected.load(Ordering::Relaxed) {
            continue;
        }
        // We send them at most what they asked to receive (their maxaveragebitrate).
        // Browsers don't do the same, so what they send us runs at their own setting
        // (PROTOCOL.md › Voice: the receiver's value is a request, not a cap).
        let local = settings.bitrate.load(Ordering::Relaxed);
        let cap = cfg.remote_cap.load(Ordering::Relaxed);
        let bitrate = if cap > 0 { local.min(cap) } else { local };
        let fec = cfg.fec.load(Ordering::Relaxed);
        let want_loss = info.loss_hint.load(Ordering::Relaxed);
        if encoder.is_none() || current != (bitrate, fec) {
            match encoder.as_mut() {
                Some(e) if current.1 == fec => {
                    let _ = e.set_bitrate(ropus::Bitrate::try_bits(bitrate).unwrap_or(ropus::Bitrate::Max));
                }
                _ => {
                    encoder = make_encoder(bitrate, fec, want_loss);
                    loss = want_loss;
                }
            }
            current = (bitrate, fec);
            info.send_bitrate.store(bitrate, Ordering::Relaxed);
        }
        let Some(enc) = encoder.as_mut() else { continue };
        if loss != want_loss {
            let _ = enc.set_packet_loss_perc(want_loss);
            loss = want_loss;
        }
        let len = match enc.encode(&frame.pcm, &mut buf) {
            Ok(n) => n,
            Err(e) => {
                log::warn!("opus encode: {e:?}");
                continue;
            }
        };
        let opus = cfg.payload_type.load(Ordering::Relaxed);
        let red_pt = cfg.red_pt.load(Ordering::Relaxed);
        // RED: this frame plus the one before (PROTOCOL.md › Voice › Redundancy). Each
        // sample written moves the RTP timestamp on by one frame, so `prev` is exactly
        // 960 back; after a stretch of plain Opus there's no frame before to repeat.
        let (payload, pt) = if red_pt > 0 {
            red::pack(&mut packed, opus, &buf[..len], &prev, FRAME as u32);
            prev.clear();
            prev.extend_from_slice(&buf[..len]);
            (&packed[..], red_pt)
        } else {
            prev.clear();
            (&buf[..len], opus)
        };
        let sample = Sample {
            data: bytes::Bytes::copy_from_slice(payload),
            duration: Duration::from_millis((FRAME as u64 * 1000) / crate::audio::RATE as u64),
            ..Sample::new(frame.at)
        };
        let wire = payload.len();
        let result = track
            .sample_writer(ssrc, pt)
            .with_audio_level(AudioLevelExtension { level: frame.level_dbov, voice: frame.voice })
            .write_sample(&sample)
            .await;
        if result.is_ok() {
            info.bytes_sent.fetch_add(wire as u64, Ordering::Relaxed);
            info.opus_bytes_sent.fetch_add(len as u64, Ordering::Relaxed);
            info.packets_sent.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loss_levels_with_hysteresis() {
        assert_eq!(loss_level(0.0, 0), 0);
        assert_eq!(loss_level(0.009, 0), 0);
        assert_eq!(loss_level(0.012, 0), 1);
        assert_eq!(loss_level(0.05, 0), 1, "5 % needs 6 % from below");
        assert_eq!(loss_level(0.061, 1), 5);
        assert_eq!(loss_level(0.045, 5), 5, "and stays down to 4 %");
        assert_eq!(loss_level(0.035, 5), 1);
        assert_eq!(loss_level(0.25, 0), 20);
        assert_eq!(loss_level(0.19, 20), 20);
        assert_eq!(loss_level(0.17, 20), 10);
    }

    /// The encoder's mode follows the hint: CELT when clean, Hybrid (with FEC) when lossy.
    #[test]
    fn clean_is_celt_lossy_is_hybrid() {
        let pcm: Vec<i16> = (0..960).map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16).collect();
        let mode = |loss: u8| {
            let mut enc = make_encoder(128_000, true, loss).unwrap();
            let mut buf = vec![0u8; 4000];
            let mut toc = 0;
            for _ in 0..20 {
                let n = enc.encode(&pcm, &mut buf).unwrap();
                assert!(n > 0);
                toc = buf[0] >> 3;
            }
            toc
        };
        assert!(mode(0) >= 16, "CELT config when clean (toc config {})", mode(0));
        let lossy = mode(5);
        assert!((12..16).contains(&lossy), "Hybrid config when lossy (toc config {lossy})");
    }

    /// One codec moved just in front of another; nothing else moves.
    #[test]
    fn codec_order() {
        let c = |rtp_codec: RTCRtpCodec, payload_type| RTCRtpCodecParameters { rtp_codec, payload_type };
        let cn = RTCRtpCodec { mime_type: "audio/CN".into(), clock_rate: 8000, channels: 1, sdp_fmtp_line: String::new(), rtcp_feedback: vec![] };
        let names = |l: &[RTCRtpCodecParameters]| l.iter().map(|c| c.payload_type).collect::<Vec<_>>();
        let opus_red = [c(opus_codec(128_000), OPUS_PT), c(cn.clone(), 13), c(red_codec(), RED_PT)];
        assert_eq!(names(&put_before(&opus_red, "audio/red", "audio/opus").unwrap()), [RED_PT, OPUS_PT, 13]);
        assert_eq!(put_before(&opus_red, "audio/opus", "audio/red"), None, "Opus already leads");
        let red_opus = [c(red_codec(), RED_PT), c(cn, 13), c(opus_codec(128_000), OPUS_PT)];
        assert_eq!(names(&put_before(&red_opus, "audio/opus", "audio/red").unwrap()), [OPUS_PT, RED_PT, 13]);
        assert_eq!(put_before(&red_opus, "audio/red", "audio/opus"), None, "RED already leads");
        assert_eq!(put_before(&red_opus[2..], "audio/red", "audio/opus"), None, "no RED: plain Opus, as negotiated");
    }
}
