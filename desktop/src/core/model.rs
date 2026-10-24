//! What the UI reads: the model behind the mutex, and its parts.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// No server configured yet (first run).
    Setup,
    Connecting,
    Online,
    Offline,
    /// The server rejected the invite key.
    BadKey,
    /// We know the server but need to sign in (or sign-in failed: see status_detail).
    SignIn,
    /// In the waiting room until a member lets us in.
    Pending,
    /// Denied or banned: we stop reconnecting.
    Removed,
}

#[derive(Default, Debug)]
pub struct History {
    pub messages: Vec<Message>,
    pub has_more: bool,
    pub loading: bool,
    pub older: bool,
}

/// What the message list should do with its scroll position on the next frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollHint {
    Bottom,
    /// Older messages were prepended: keep what's on screen in place.
    Anchor,
}

#[derive(Default, Clone, Debug)]
pub struct VoiceStats {
    pub send_kbps: u32,
    pub recv_kbps: u32,
    pub bitrate: u32,
}

/// A quiet word next to someone in your call when something's off
/// (PROTOCOL.md › Voice › Connection hints); nothing when all is well.
#[derive(Clone, Debug, PartialEq)]
pub enum Hint {
    /// Their socket dropped and we're keeping the call ("reconnecting…").
    Reconnecting,
    /// ICE failed and the server has no TURN relay ("needs the voice relay").
    NeedsRelay,
    /// ICE failed although there's a relay ("can't connect").
    CantConnect,
    /// More than 3 % of their audio concealed, or a round trip over 300 ms.
    Weak { concealed: f32, rtt: Option<u32> },
}

impl Hint {
    /// The web's words (`voiceNote()` in web/js/app.js).
    pub fn text(&self) -> &'static str {
        match self {
            Hint::Reconnecting => "reconnecting…",
            Hint::NeedsRelay => "needs the voice relay",
            Hint::CantConnect => "can't connect",
            Hint::Weak { .. } => "weak connection",
        }
    }

    /// The tooltip, for `name`.
    pub fn title(&self, name: &str) -> String {
        match self {
            Hint::Reconnecting => format!("{name} lost their connection to Yap. Their voice is still coming through; it usually sorts itself out in a few seconds."),
            Hint::CantConnect => format!("Your network and {name}'s can't reach each other right now. Yap keeps trying."),
            Hint::NeedsRelay => format!(
                "Your network and {name}'s can't reach each other directly (strict firewalls or mobile networks do this). The server's owner can switch on the voice relay, then it just works — see SELF_HOSTING.md."
            ),
            Hint::Weak { concealed, rtt } => {
                let pct = (concealed * 100.0).round() as u32;
                let mut why = Vec::new();
                if pct >= 3 {
                    why.push(format!("about {pct}% of their audio is getting lost or arriving late"));
                }
                if let Some(r) = rtt.filter(|r| *r > 300) {
                    why.push(format!("the delay is {r} ms"));
                }
                format!("{name}'s voice may break up: {}. It usually passes; if not, one of you is on a busy network.", why.join(", and "))
            }
        }
    }
}

/// Settings › Voice › Hear yourself.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum HearState {
    #[default]
    Idle,
    /// Recording 4 s; `hold_key`: in a push-to-talk call, so hold the key.
    Recording { hold_key: bool },
    Playing,
    Done,
    Failed,
}

pub struct Model {
    pub config: Config,
    pub config_dirty: bool,
    pub status: Status,
    pub status_detail: String,
    pub sid: Option<String>,
    pub channels: Vec<Channel>,
    pub users: HashMap<String, User>,
    pub sessions: HashMap<String, Session>,
    pub emojis: HashMap<String, Emoji>,
    pub history: HashMap<String, History>,
    /// nonce -> optimistic message
    pub pending: Vec<(String, Message)>,
    /// (channel, text) of messages the server refused: back into that channel's
    /// message box if it's empty (the UI takes them).
    pub composer_restore: Vec<(String, String)>,
    /// channelId -> uid -> expires
    pub typing: HashMap<String, HashMap<String, Instant>>,
    pub voice_channel: Option<String>,
    pub voice_wanted: Option<String>,
    pub speaking: HashSet<String>,
    pub peer_states: HashMap<String, String>,
    pub voice_stats: Option<VoiceStats>,
    pub mic_ok: bool,
    pub mic_error: Option<String>,
    pub mic_test: bool,
    /// People in our call whose socket dropped: we keep their connection a
    /// while (shown "reconnecting…"). sid → when, and since when it's been down.
    pub ghosts: HashMap<String, Ghost>,
    /// Connection hints per session in our call.
    pub hints: HashMap<String, Hint>,
    /// Our own socket dropped and we're keeping the call ("Reconnecting… voice is still on").
    pub resuming: bool,
    /// The mic has sent nothing but digital silence for 5 s while unmuted.
    pub mic_silent: bool,
    /// Most people in the call lose more than 3 % of your audio (the middle
    /// of what they report): "Your connection is dropping some of your audio".
    pub own_weak: Option<f32>,
    /// "You're muted" by the Mute button since then (you talked while muted);
    /// true: the first time on this device, so say what the blip means.
    pub muted_nudge: Option<(Instant, bool)>,
    pub hear: HearState,
    pub toasts: VecDeque<(String, Instant)>,
    pub scroll: Option<ScrollHint>,
    pub window_focused: bool,
    /// Bumped by each @mention (or reply to you) while the window isn't in
    /// front: the UI asks the OS to point Yap out (taskbar flash, Dock bounce).
    pub attention: u64,
    pub awaiting_channel: Option<String>,
    /// People in the waiting room (members can let them in).
    pub join_requests: Vec<User>,
    /// channelId -> unread @mentions of me.
    pub mentions: HashMap<String, u32>,
    /// `/api/config`: GIF key, sidebar places, the Minecraft server.
    pub server_config: Option<crate::http::ServerConfig>,
    pub minecraft: Option<crate::http::McStatus>,
    pub update: UpdateState,
    /// Device token for the current server (from tokens.json).
    pub token: Option<String>,
    /// What the next hello should be when there's no token.
    pub auth: Option<Auth>,
    /// An account with no password at all (`ready.needsPin`): nudge to set one.
    pub needs_pin: bool,
    /// A temporary password we reset for someone (owner/mod): (name, password).
    pub pin_reset: Option<(String, String)>,
    /// Sidebar Places from the server (`ready.links` / `links`); None = not sent (older server).
    pub links: Option<Vec<Link>>,
    /// Personal invite links (Settings › Invite).
    pub invites: Vec<Invite>,
    /// The link we just made: (label, full link). Shown once.
    pub new_invite: Option<(String, String)>,
    /// Bumped when the stored main invite key changes (`key` after the owner
    /// reset the main link, or a different `ready.key`), or when resetting it
    /// was turned away (its words in `main_reset_error`).
    pub main_key_seq: u64,
    pub main_reset_error: Option<String>,
    /// The moderation log (Settings › Activity).
    pub modlog: Vec<ModLogEntry>,
    /// A picture is uploading.
    pub sending_picture: bool,
    /// This connection's upload token (`ready.uploadToken`), sent as `ut` with every upload.
    pub upload_token: Option<String>,
    /// Encrypted Personal space: the wrapped key (None = not encrypted)…
    pub vault: Option<crate::vault::Wrap>,
    /// …and the space key, once unlocked on this device (with the `wrap.ct` it opened).
    pub vault_key: Option<crate::vault::Key>,
    pub vault_key_ct: String,
    /// A vault operation is running (PBKDF2); its error, if it failed.
    pub vault_busy: bool,
    pub vault_error: Option<String>,
    /// Bumped when a vault operation finishes (ok: `vault_error` is None).
    pub vault_seq: u64,
    /// `ready.weakPin`: you signed in with an old PIN, a guessable password or
    /// a temporary one (the "Choose a password" prompt); `ready.weakReason`,
    /// which of those (None from an older server); and `ready.secretKind`,
    /// "pin" or "password".
    pub weak_pin: bool,
    pub weak_reason: Option<WeakReason>,
    pub secret_kind: String,
    /// Bumped when the server answers a password change (`account` or an
    /// error about `account.pin`); the error, if it was one.
    pub password_seq: u64,
    pub password_error: Option<String>,
    /// Settings › Account: your signed-in devices (once asked).
    pub devices: Option<Vec<crate::protocol::Device>>,
    /// "New sign-in: …. Not you?" until dismissed.
    pub signin_alert: Option<crate::protocol::Device>,
    /// Search: the query, what the server found and what matched on the device.
    pub search: Option<SearchState>,
    /// Pinned messages per channel (most recently pinned first), once asked.
    pub pins: HashMap<String, Vec<Message>>,
    /// A message to scroll to and flash (found, maybe after loading older pages).
    pub jump_to: Option<(String, String)>,
    /// The shared view open instead of the chat (Draw, Space), if any.
    pub view: Option<View>,
    pub draw: DrawModel,
    pub space: SpaceModel,
    /// Watch together: what's on (from `watch.state`), once known.
    pub watch: Option<crate::protocol::WatchState>,
    /// Space voice: how loud each session is by distance or room (1 = normal).
    pub proximity: HashMap<String, f32>,
    /// Space: where each voice comes from (sid → −1 … 1), when not the middle.
    pub pan: HashMap<String, f32>,
    // ── Extras (v0.36) ──
    /// Soundboard clips and the uploaded chimes people use.
    pub sounds: Vec<Sound>,
    /// "▶ Ada played 📯 airhorn" in the voice panel for a few seconds.
    pub last_sound: Option<(String, Instant)>,
    /// Upcoming hangs, soonest first; one starting now shows a notice.
    pub hangs: Vec<Hang>,
    pub hang_now: Option<String>,
    /// Quiet hours (also hold back this app's own notifications and their sounds).
    pub quiet: Option<QuietHours>,
    /// "Ada is away: at work til 6" lines, under the message of ours that got them.
    pub away_lines: HashMap<String, Vec<(String, String, Option<i64>)>>,
    /// Catch up: what's new per channel (`unread.counts`), once asked.
    pub unread_counts: Option<HashMap<String, UnreadCount>>,
    /// The media shelf (pictures and GIFs) per channel.
    pub shelf: HashMap<String, ShelfState>,
    /// "New since …": per channel, what you'd read when you came back to it.
    pub read_marks: HashMap<String, i64>,
    /// A web page to open (the UI opens it next frame): handoff links.
    pub open_url: Option<String>,
    /// Does the server have a TURN relay this app can use (`/api/ice`)? None = not asked yet.
    pub relay: Option<bool>,
    /// Quiet lines in a channel for this session only ("Theo joined — invited
    /// by Priya", "You're in. Say hi 👋"); never stored, never a popup.
    pub local_notes: Vec<LocalNote>,
    /// Screen sharing: ours, who watches it, and the one we watch.
    pub screen: ScreenModel,
    pub(super) failures: u32,
}

/// Screen sharing as the UI sees it (the pictures themselves are in
/// `Shared::screen`).
#[derive(Default)]
pub struct ScreenModel {
    /// Our share, while we share.
    pub sharing: Option<MyShare>,
    /// Who asked to watch it (their sids), by name.
    pub watchers: Vec<String>,
    /// Whose screen we watch.
    pub watching: Option<Watching>,
}

#[derive(Clone, Debug)]
pub struct MyShare {
    pub source: crate::screen::capture::Source,
    pub quality: crate::screen::Quality,
    /// Sending the computer's sound too.
    pub sound: bool,
}

#[derive(Clone, Debug)]
pub struct Watching {
    pub sid: String,
    pub uid: String,
    /// They stopped sharing (or left): the view says so, then closes itself 2 s later.
    pub ended: Option<Instant>,
}

/// A quiet line at its moment in a channel, for this session only
/// (PROTOCOL.md › Extras › Since you left): `icon`, then `bold` (a name in
/// its `color`, or "You're in."), then `text`.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalNote {
    pub key: String,
    pub channel_id: String,
    pub ts: i64,
    pub icon: &'static str,
    pub bold: String,
    pub color: Option<String>,
    pub text: String,
}

/// Someone whose call we keep while their socket is gone.
#[derive(Clone, Debug)]
pub struct Ghost {
    pub since: Instant,
    /// Their connection has been down (not `connected`) since then.
    pub down_since: Option<Instant>,
}

/// The media shelf of one channel.
#[derive(Default, Clone, Debug)]
pub struct ShelfState {
    pub items: Vec<Message>,
    pub has_more: bool,
    pub loading: bool,
}

/// A shared view shown inside the app.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Draw,
    Space,
}

impl View {
    pub fn name(self) -> &'static str {
        match self {
            View::Draw => "draw",
            View::Space => "space",
        }
    }
}

/// A search in progress or done.
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    pub q: String,
    /// One channel, or everywhere (None).
    pub channel: Option<String>,
    /// From the server (plain text only)…
    pub server: Vec<Message>,
    /// …and encrypted Personal-space notes that matched on this device.
    pub local: Vec<Message>,
    pub loading: bool,
    /// Older Personal-space pages loaded for the device-side search.
    pub personal_pages: u8,
}

impl SearchState {
    /// Everything that matched, newest first.
    pub fn results(&self) -> Vec<&Message> {
        let mut all: Vec<&Message> = self.server.iter().chain(self.local.iter()).collect();
        all.sort_by_key(|x| std::cmp::Reverse(x.ts));
        all.dedup_by(|a, b| a.id == b.id);
        all
    }
}

/// The shared drawing board.
#[derive(Default)]
pub struct DrawModel {
    /// Finished strokes, in order.
    pub strokes: Vec<crate::protocol::Stroke>,
    /// Strokes others are drawing right now, by "sid:sk".
    pub live: HashMap<String, LiveStroke>,
    pub gallery: Vec<crate::protocol::SavedDrawing>,
    /// My stroke ids, for undo.
    pub mine: Vec<String>,
    /// Bumped whenever `strokes` changes (the UI re-renders its cache).
    pub version: u64,
    pub loaded: bool,
}

/// Strokes in progress kept at once (one that's quiet for 8 s isn't drawn, and goes first).
const MAX_LIVE: usize = 64;

impl DrawModel {
    /// `draw.seg`: more of a stroke someone is drawing. A stroke keeps at most
    /// what a finished one may have (`drawing::MAX_NUMBERS`), and at most
    /// `MAX_LIVE` are kept: an endless stream of pieces can't grow the board.
    #[allow(clippy::too_many_arguments)]
    pub fn add_live(&mut self, sid: String, sk: &str, c: String, w: f32, e: bool, l: u8, p: Vec<f32>) {
        let key = format!("{sid}:{sk}");
        if !self.live.contains_key(&key) && self.live.len() >= MAX_LIVE {
            self.live.retain(|_, s| s.at.elapsed() < Duration::from_secs(8));
            if self.live.len() >= MAX_LIVE {
                return;
            }
        }
        let entry = self.live.entry(key).or_insert_with(|| LiveStroke { sid, c, w, e, l, p: vec![], at: Instant::now() });
        let room = crate::drawing::MAX_NUMBERS.saturating_sub(entry.p.len()) & !1;
        entry.p.extend(p.into_iter().take(room));
        entry.at = Instant::now();
    }

    /// `draw.wipe`: layer `l` emptied for everyone. Its strokes go (mine with
    /// them, so undo can't bring them back); strokes in progress stay. Returns
    /// how many went.
    pub fn wipe(&mut self, l: u8) -> usize {
        let before = self.strokes.len();
        let on = |s: &crate::protocol::Stroke| crate::drawing::layer(s) == l as usize;
        let gone: std::collections::HashSet<String> = self.strokes.iter().filter(|s| on(s)).map(|s| s.id.clone()).collect();
        self.strokes.retain(|s| !on(s));
        self.mine.retain(|id| !gone.contains(id));
        self.version += 1;
        before - self.strokes.len()
    }
}

#[derive(Clone, Debug)]
pub struct LiveStroke {
    pub sid: String,
    pub c: String,
    pub w: f32,
    pub e: bool,
    /// Layer (0 = bottom).
    pub l: u8,
    pub p: Vec<f32>,
    pub at: Instant,
}

/// Space: everyone else's spot (targets; the UI glides toward them).
#[derive(Default)]
pub struct SpaceModel {
    pub players: HashMap<String, crate::protocol::SpacePos>,
    /// Where I stood last time (from `space.state`), until the UI places me.
    pub me: Option<crate::protocol::SpacePos>,
    pub got_state: bool,
    /// Bumped on every `space.state` (the UI asks for the games' state then).
    pub state_seq: u64,
    pub emotes: HashMap<String, (String, Instant)>,
    /// Mini-game updates from others, oldest first (the UI plays them).
    pub games: Vec<(String, String, serde_json::Value)>,
    /// The Gallery: saved drawings, newest first (`draw.gallery`, then each `drawing`).
    pub gallery: Vec<crate::protocol::SavedDrawing>,
    pub gallery_loaded: bool,
}

/// How the next hello signs in. `pin` is the wire's name for the password
/// (an old 6-digit PIN still works for signing in).
#[derive(Clone, Debug)]
pub enum Auth {
    Login { name: String, pin: String },
    Signup { name: String, pin: String, color: String },
}

/// Why the "Choose a password" prompt shows (`ready.weakReason`): its words
/// differ, and Later puts off each one on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeakReason {
    /// Still on an old 6-digit PIN.
    Pin,
    /// Signed in with a temporary password from an owner/mod reset.
    Temporary,
    /// A password that's easy to guess (set before the rules got stricter).
    Guessable,
}

impl WeakReason {
    /// The wire's word (`ready.weakReason`), also what Later saves in the config.
    pub fn as_str(self) -> &'static str {
        match self {
            WeakReason::Pin => "pin",
            WeakReason::Temporary => "temporary",
            WeakReason::Guessable => "guessable",
        }
    }

    /// `ready.weakReason`; None for anything else (a newer server's new reason
    /// falls back like an older server's missing one).
    pub fn from_wire(s: &str) -> Option<Self> {
        [WeakReason::Pin, WeakReason::Temporary, WeakReason::Guessable].into_iter().find(|r| r.as_str() == s)
    }
}

impl Model {
    /// How loud someone's session plays: your volume for them × Space proximity.
    /// How loud someone's screen sound plays here (0–1; their voice volume is separate).
    pub fn screen_volume(&self, uid: &str) -> f32 {
        self.config.screen_volumes.get(uid).copied().unwrap_or(1.0).clamp(0.0, 1.0)
    }

    pub fn session_volume(&self, sid: &str) -> f32 {
        let base = self.sessions.get(sid).map(|s| self.config.volume_for(&s.uid)).unwrap_or(1.0);
        base * self.proximity.get(sid).copied().unwrap_or(1.0)
    }

    pub(crate) fn new(config: Config) -> Self {
        let status = if config.server.is_empty() { Status::Setup } else { Status::Connecting };
        Self {
            config,
            config_dirty: false,
            status,
            status_detail: String::new(),
            sid: None,
            channels: Vec::new(),
            users: HashMap::new(),
            sessions: HashMap::new(),
            emojis: HashMap::new(),
            history: HashMap::new(),
            pending: Vec::new(),
            composer_restore: Vec::new(),
            typing: HashMap::new(),
            voice_channel: None,
            voice_wanted: None,
            speaking: HashSet::new(),
            peer_states: HashMap::new(),
            voice_stats: None,
            mic_ok: false,
            mic_error: None,
            mic_test: false,
            ghosts: HashMap::new(),
            hints: HashMap::new(),
            resuming: false,
            mic_silent: false,
            muted_nudge: None,
            own_weak: None,
            hear: HearState::Idle,
            toasts: VecDeque::new(),
            scroll: None,
            window_focused: true,
            attention: 0,
            awaiting_channel: None,
            join_requests: Vec::new(),
            mentions: HashMap::new(),
            server_config: None,
            minecraft: None,
            update: UpdateState::Idle,
            token: None,
            auth: None,
            needs_pin: false,
            pin_reset: None,
            links: None,
            invites: Vec::new(),
            new_invite: None,
            main_key_seq: 0,
            main_reset_error: None,
            modlog: Vec::new(),
            sending_picture: false,
            upload_token: None,
            vault: None,
            vault_key: None,
            vault_key_ct: String::new(),
            vault_busy: false,
            vault_error: None,
            vault_seq: 0,
            weak_pin: false,
            weak_reason: None,
            secret_kind: "pin".into(),
            password_seq: 0,
            password_error: None,
            devices: None,
            signin_alert: None,
            search: None,
            pins: HashMap::new(),
            jump_to: None,
            view: None,
            draw: DrawModel::default(),
            space: SpaceModel::default(),
            watch: None,
            proximity: HashMap::new(),
            pan: HashMap::new(),
            sounds: Vec::new(),
            last_sound: None,
            hangs: Vec::new(),
            hang_now: None,
            quiet: None,
            away_lines: HashMap::new(),
            unread_counts: None,
            shelf: HashMap::new(),
            read_marks: HashMap::new(),
            open_url: None,
            relay: None,
            local_notes: Vec::new(),
            screen: ScreenModel::default(),
            failures: 0,
        }
    }

    /// Hold back this notification (and its sound)? Quiet hours, as the server does for pushes.
    pub fn quiet(&self, mention: bool) -> bool {
        use chrono::Timelike;
        let Some(q) = &self.quiet else { return false };
        let now = chrono::Local::now();
        quiet_now(q, mention, now_ms(), now.hour() * 60 + now.minute())
    }

    /// A soundboard clip or chime by id.
    pub fn sound(&self, id: &str) -> Option<&Sound> {
        self.sounds.iter().find(|s| s.id == id)
    }

    /// Sidebar Places: the server's list, or `/api/config`'s on older servers.
    pub fn places(&self) -> Vec<Link> {
        match &self.links {
            Some(l) => l.clone(),
            None => self
                .server_config
                .as_ref()
                .map(|c| {
                    c.links
                        .iter()
                        .map(|l| Link { id: String::new(), name: l.name.clone(), url: l.url.clone(), view: l.view.clone(), description: (!l.description.is_empty()).then(|| l.description.clone()) })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The voice channel we're in (or joining).
    pub fn my_voice_channel(&self) -> Option<&Channel> {
        self.voice_wanted.as_deref().or(self.voice_channel.as_deref()).and_then(|id| self.channel(id))
    }

    /// Opus bitrate for the call: the channel's, else our own setting.
    pub fn voice_bitrate(&self) -> u32 {
        self.my_voice_channel().and_then(|c| c.bitrate).unwrap_or(self.config.bitrate)
    }

    /// Another of our devices is in our call, unmuted: we're talking through
    /// that one, so this one doesn't say "You're muted" (OBS links are always muted).
    pub fn live_elsewhere(&self) -> bool {
        let Some(channel) = self.voice_channel.as_deref() else { return false };
        self.sessions.values().any(|s| {
            s.uid == self.config.uid && Some(&s.sid) != self.sid.as_ref() && !s.is_obs() && s.voice.as_ref().is_some_and(|v| v.channel_id == channel && !v.muted && !v.deafened)
        })
    }

    /// Our newest message in a channel (Up-arrow edits it).
    pub fn last_own_message(&self, channel_id: &str) -> Option<&Message> {
        self.history.get(channel_id)?.messages.iter().rev().find(|m| m.uid == self.config.uid && !m.pending && !m.body.is_empty())
    }

    pub fn channel(&self, id: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn first_text_channel(&self) -> Option<&Channel> {
        self.channels.iter().filter(|c| c.is_text()).min_by_key(|c| c.position)
    }

    pub fn is_unread(&self, c: &Channel) -> bool {
        if !c.is_chat() {
            return false;
        }
        let Some(last) = c.last_message_at else { return false };
        if c.id == self.config.channel && self.window_focused {
            return false;
        }
        last > self.config.last_read.get(&c.id).copied().unwrap_or(0)
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toasts.push_back((text.into(), Instant::now()));
        while self.toasts.len() > 3 {
            self.toasts.pop_front();
        }
    }

    pub fn mark_read(&mut self, channel_id: &str, ts: Option<i64>) {
        self.mentions.remove(channel_id);
        let latest = ts.or_else(|| self.channel(channel_id).and_then(|c| c.last_message_at));
        let Some(latest) = latest else { return };
        if self.config.last_read.get(channel_id).copied().unwrap_or(0) >= latest {
            return;
        }
        self.config.last_read.insert(channel_id.to_string(), latest);
        self.config_dirty = true;
    }

    /// A device that has never read anything (a newcomer, or you on a new
    /// computer) starts caught up: every channel read up to its newest
    /// message (empty ones at 1, so anything new counts), not every channel
    /// unread since the beginning. Returns whether it did.
    pub fn start_caught_up(&mut self) -> bool {
        if !self.config.last_read.is_empty() || self.channels.is_empty() {
            return false;
        }
        for c in self.channels.iter().filter(|c| c.is_chat()) {
            self.config.last_read.insert(c.id.clone(), c.last_message_at.filter(|t| *t > 0).unwrap_or(1));
        }
        self.config_dirty = true;
        true
    }

    /// What `unread.counts` asks about: every channel with something unread (≤ 50).
    pub fn unread_since(&self) -> HashMap<String, i64> {
        self.channels.iter().filter(|c| self.is_unread(c)).take(50).map(|c| (c.id.clone(), self.config.last_read.get(&c.id).copied().unwrap_or(0))).collect()
    }

    /// Mentions that came while you were away (`unread.counts` › `mentions`):
    /// a channel's badge is the larger of that and what arrived live. Older
    /// servers don't count them (None): nothing changes. The channel you're
    /// looking at gets none, like a live mention there.
    pub fn seed_mentions(&mut self, counts: &HashMap<String, UnreadCount>) -> Vec<(String, u32)> {
        let mut seeded = Vec::new();
        for (id, c) in counts {
            let Some(n) = c.mentions.filter(|n| *n > 0) else { continue };
            if !self.channel(id).is_some_and(|ch| self.is_unread(ch)) || n <= self.mentions.get(id).copied().unwrap_or(0) {
                continue;
            }
            self.mentions.insert(id.clone(), n);
            seeded.push((id.clone(), n));
        }
        seeded.sort();
        seeded
    }

    /// How many new messages in a channel are for you (Catch up's "N for you").
    pub fn mentions_for_you(&self, channel_id: &str) -> u32 {
        let counted = self.unread_counts.as_ref().and_then(|c| c.get(channel_id)).and_then(|u| u.mentions).unwrap_or(0);
        counted.max(self.mentions.get(channel_id).copied().unwrap_or(0))
    }

    /// Catch up › Mark all read: every channel read up to its newest message.
    pub fn mark_all_read(&mut self) {
        let latest: Vec<(String, i64)> = self.channels.iter().filter(|c| c.is_chat()).filter_map(|c| c.last_message_at.map(|t| (c.id.clone(), t))).collect();
        for (id, t) in latest {
            let read = self.config.last_read.entry(id).or_insert(0);
            if *read < t {
                *read = t;
                self.config_dirty = true;
            }
        }
        self.mentions.clear();
    }

    /// In a call: keep `config.last_call` up to date (at most a write a minute),
    /// so the next launch can offer it again. Returns whether it changed.
    pub fn note_call(&mut self, now: i64) -> bool {
        let Some(channel) = self.voice_channel.clone() else { return false };
        let fresh = self.config.last_call.as_ref().is_some_and(|c| c.channel == channel && c.server == self.config.server && (0..60_000).contains(&(now - c.at)));
        if fresh {
            return false;
        }
        self.config.last_call = Some(crate::config::LastCall { server: self.config.server.clone(), channel, at: now });
        self.config_dirty = true;
        true
    }

    /// Left the call yourself (or signed out, or another server): nothing to offer next time.
    pub fn forget_call(&mut self) {
        if self.config.last_call.take().is_some() {
            self.config_dirty = true;
        }
    }

    /// A quiet line in a channel for this session (the same `key` replaces an older one; the last 20 stay).
    pub fn add_local_note(&mut self, note: LocalNote) {
        self.local_notes.retain(|n| n.key != note.key);
        self.local_notes.push(note);
        let extra = self.local_notes.len().saturating_sub(20);
        self.local_notes.drain(..extra);
    }

    /// Does this message @-mention me (or @everyone / @here)?
    pub fn mentions_me(&self, msg: &Message) -> bool {
        if msg.uid == self.config.uid {
            return false;
        }
        // A reply to me counts as a mention.
        if msg.reply.as_ref().is_some_and(|r| r.uid == self.config.uid) {
            return true;
        }
        if msg.body.is_empty() || crate::vault::is_note(&msg.body) {
            return false;
        }
        // The one rule every client and the server use (web/js/mentions.js):
        // longest names first, so "@Sam Lee" never pings Sam.
        let mut people: Vec<(&str, &str)> = self.users.values().map(|u| (u.id.as_str(), u.name.as_str())).collect();
        if !self.users.contains_key(&self.config.uid) {
            people.push((self.config.uid.as_str(), self.config.name.as_str()));
        }
        let (uids, everyone) = find_mentions(&msg.body, people);
        everyone || uids.contains(&self.config.uid)
    }

    /// The "Choose a password" prompt, and why (PROTOCOL.md › Identity): with
    /// `ready.weakPin`, for `ready.weakReason` (an older server doesn't say: an
    /// old PIN if that's what you signed in with, else a guessable password),
    /// unless Later put that reason off on this device. Each reason is put off
    /// on its own, so a reset's temporary password still asks after a PIN's
    /// prompt was put off.
    pub fn password_prompt(&self) -> Option<WeakReason> {
        if !self.weak_pin || self.needs_pin {
            return None;
        }
        let reason = self.weak_reason.unwrap_or(if self.secret_kind == "pin" { WeakReason::Pin } else { WeakReason::Guessable });
        (!self.config.password_later.iter().any(|r| r == reason.as_str())).then_some(reason)
    }

    /// Are we the owner or a mod (delete anyone's messages, ban, cancel any hang…)?
    pub fn am_moderator(&self) -> bool {
        self.users.get(&self.config.uid).is_some_and(|u| u.role == "owner" || u.role == "mod")
    }

    /// Are we the owner (reset the main link, make mods)?
    pub fn am_owner(&self) -> bool {
        self.users.get(&self.config.uid).is_some_and(|u| u.role == "owner")
    }

    pub fn user_name(&self, uid: &str) -> String {
        self.users.get(uid).map(|u| u.name.clone()).unwrap_or_else(|| "Unknown".into())
    }

    /// Who shows under a voice channel: everyone in it but OBS links, which
    /// listen without showing up (no row, no chimes; the call still connects
    /// to them, and the call report names them). A–Z.
    pub fn voice_members(&self, channel_id: &str) -> Vec<&Session> {
        let mut members: Vec<&Session> = self.sessions.values().filter(|s| !s.is_obs() && s.voice.as_ref().is_some_and(|v| v.channel_id == channel_id)).collect();
        members.sort_by_cached_key(|s| self.user_name(&s.uid).to_lowercase());
        members
    }

    /// Whose OBS link is still listening in a voice channel they aren't in
    /// themselves (it may for 5 minutes after they leave; PROTOCOL.md › Voice ›
    /// OBS links in a call): shown greyed out under the channel, "<name>'s OBS
    /// is still listening", so nobody's heard without knowing. Not a call
    /// member: no volume, no ring, not counted. Their uids, A–Z by name, once
    /// each however many OBS links they have there.
    pub fn obs_listening(&self, channel_id: &str) -> Vec<&str> {
        let in_channel = |s: &Session| s.voice.as_ref().is_some_and(|v| v.channel_id == channel_id);
        let mut uids: Vec<&str> = self
            .sessions
            .values()
            .filter(|s| s.is_obs() && in_channel(s))
            .map(|s| s.uid.as_str())
            .filter(|uid| !self.sessions.values().any(|s| !s.is_obs() && s.uid == *uid && in_channel(s)))
            .collect();
        uids.sort_by_cached_key(|uid| (self.user_name(uid).to_lowercase(), uid.to_string()));
        uids.dedup();
        uids
    }

    pub fn emojis_by_name(&self) -> HashMap<String, Emoji> {
        self.emojis.values().map(|e| (e.name.to_lowercase(), e.clone())).collect()
    }

    pub fn typers(&self, channel_id: &str) -> Vec<String> {
        let now = Instant::now();
        let mut v: Vec<String> = self
            .typing
            .get(channel_id)
            .map(|m| m.iter().filter(|(uid, until)| **until > now && **uid != self.config.uid).map(|(uid, _)| self.user_name(uid)).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
}

/// Click-to-update, as the UI sees it.
#[derive(Clone, Debug, Default)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(crate::update::Available),
    Downloading { version: String, done: u64, total: u64 },
    /// Swapped in; restart to run it.
    Installed(String),
    Failed(String),
}

#[cfg(test)]
mod unread_tests {
    use super::*;

    fn model(channels: &[(&str, &str, Option<i64>)]) -> Model {
        let mut m = Model::new(Config { uid: "me".into(), name: "Sam".into(), channel: "general".into(), ..Config::default() });
        m.channels = channels
            .iter()
            .enumerate()
            .map(|(i, (id, kind, last))| serde_json::from_value(serde_json::json!({ "id": id, "name": id, "kind": kind, "position": i, "lastMessageAt": last })).unwrap())
            .collect();
        m
    }

    /// PROTOCOL.md › Since you left: a device that has never read anything starts caught up.
    #[test]
    fn a_new_device_starts_caught_up() {
        let mut m = model(&[("general", "text", Some(500)), ("games", "text", None), ("lounge", "voice", Some(9)), ("p_me", "personal", Some(70))]);
        assert!(m.start_caught_up());
        assert_eq!(m.config.last_read.get("general"), Some(&500));
        assert_eq!(m.config.last_read.get("games"), Some(&1), "an empty channel: anything new counts");
        assert_eq!(m.config.last_read.get("p_me"), Some(&70));
        assert!(!m.config.last_read.contains_key("lounge"), "voice channels aren't read");
        assert!(m.config_dirty);
        assert!(m.channels.iter().all(|c| !m.is_unread(c)), "nothing unread");
        // A device that has read something keeps what it knows.
        let mut m = model(&[("general", "text", Some(500)), ("games", "text", Some(800))]);
        m.config.last_read.insert("general".into(), 100);
        assert!(!m.start_caught_up());
        assert_eq!(m.config.last_read.get("games"), None);
        assert!(m.is_unread(m.channel("games").unwrap()));
    }

    /// `unread.counts` › `mentions` seeds the badges (larger of it and the live count);
    /// older servers (None) change nothing; the channel you're looking at gets none.
    #[test]
    fn counts_seed_mention_badges() {
        let mut m = model(&[("general", "text", Some(900)), ("games", "text", Some(900)), ("plans", "text", Some(900)), ("music", "text", Some(900))]);
        for id in ["general", "games", "plans", "music"] {
            m.config.last_read.insert(id.into(), 100);
        }
        m.mentions.insert("plans".into(), 5);
        let count = |n: u32, mentions: Option<u32>| UnreadCount { n, first: None, mentions };
        let counts: HashMap<String, UnreadCount> =
            [("general", count(3, Some(2))), ("games", count(4, Some(2))), ("plans", count(9, Some(1))), ("music", count(2, None))].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        m.unread_counts = Some(counts.clone());
        assert_eq!(m.seed_mentions(&counts), vec![("games".to_string(), 2)]);
        assert_eq!(m.mentions.get("general"), None, "you're looking at it");
        assert_eq!(m.mentions.get("games"), Some(&2));
        assert_eq!(m.mentions.get("plans"), Some(&5), "a bigger live count stays");
        assert_eq!(m.mentions.get("music"), None, "an older server: unknown, not 0");
        assert_eq!(m.mentions_for_you("plans"), 5);
        assert_eq!(m.mentions_for_you("general"), 2);
        assert_eq!(m.mentions_for_you("music"), 0);
        // Mark all read: everything read, no badges.
        m.mark_all_read();
        assert!(m.mentions.is_empty());
        assert!(m.channels.iter().all(|c| m.config.last_read.get(&c.id) == Some(&900)));
        assert!(m.unread_since().is_empty());
    }

    #[test]
    fn local_notes_replace_by_key_and_keep_twenty() {
        let mut m = model(&[("general", "text", Some(1))]);
        let note = |key: &str, ts: i64| LocalNote { key: key.into(), channel_id: "general".into(), ts, icon: "user-plus", bold: "Theo".into(), color: None, text: " joined".into() };
        m.add_local_note(note("join:theo", 1));
        m.add_local_note(note("join:theo", 2));
        assert_eq!(m.local_notes.len(), 1);
        assert_eq!(m.local_notes[0].ts, 2);
        for i in 0..30 {
            m.add_local_note(note(&format!("k{i}"), i));
        }
        assert_eq!(m.local_notes.len(), 20);
        assert_eq!(m.local_notes[0].key, "k10");
    }
}

#[cfg(test)]
mod draw_tests {
    use super::*;
    use crate::protocol::Stroke;

    /// `draw.wipe {l}` empties that layer only; my wiped strokes leave the undo list.
    #[test]
    fn a_wipe_empties_one_layer() {
        let stroke = |id: &str, uid: &str, l: u8| Stroke { id: id.into(), uid: uid.into(), c: "#000000".into(), w: 14.0, p: vec![1.0, 2.0], l, ..Default::default() };
        let mut d = DrawModel { strokes: vec![stroke("a", "me", 0), stroke("b", "me", 2), stroke("c", "them", 2), stroke("d", "me", 1), stroke("e", "them", 0)], mine: vec!["a".into(), "b".into(), "d".into()], version: 7, loaded: true, ..Default::default() };
        d.live.insert("x:live".into(), LiveStroke { sid: "x".into(), c: "#000000".into(), w: 14.0, e: false, l: 2, p: vec![1.0, 2.0], at: Instant::now() });
        let msg = serde_json::from_str::<ServerMsg>(r#"{"t":"draw.wipe","l":2,"by":"them"}"#).unwrap();
        let ServerMsg::DrawWipe { l, .. } = msg else { panic!("draw.wipe") };
        assert_eq!(d.wipe(l), 2);
        assert_eq!(d.strokes.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["a", "d", "e"], "the other layers keep their strokes, in order");
        assert_eq!(d.mine, ["a", "d"], "undo can't bring a wiped stroke back");
        assert_eq!(d.version, 8, "the board redraws");
        assert_eq!(d.live.len(), 1, "a stroke still being drawn stays");
        // Live pieces are capped: per stroke, and in how many strokes.
        for _ in 0..100 {
            d.add_live("x".into(), "live", "#000000".into(), 14.0, false, 2, vec![1.0; 400]);
        }
        assert_eq!(d.live["x:live"].p.len(), crate::drawing::MAX_NUMBERS);
        for k in 0..200 {
            d.add_live("y".into(), &format!("s{k}"), "#000000".into(), 14.0, false, 0, vec![1.0, 2.0]);
        }
        assert_eq!(d.live.len(), MAX_LIVE);
        // The bottom layer (where every stroke from before layers is).
        assert_eq!(d.wipe(0), 2);
        assert_eq!(d.strokes.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["d"]);
        assert_eq!(d.wipe(0), 0, "an empty layer stays empty");
    }
}

#[cfg(test)]
mod password_tests {
    use super::*;

    /// The prompt (PROTOCOL.md › Identity): only with `ready.weakPin`, never
    /// for an account with no password at all (it has its own bar), in the
    /// words `ready.weakReason` picks, and Later puts off one reason at a time.
    #[test]
    fn the_password_prompt() {
        let mut m = Model::new(Config::default());
        assert_eq!(m.password_prompt(), None, "nothing weak");
        m.weak_pin = true;
        // An older server: no weakReason, so what you signed in with decides.
        m.secret_kind = "pin".into();
        assert_eq!(m.password_prompt(), Some(WeakReason::Pin));
        m.secret_kind = "password".into();
        assert_eq!(m.password_prompt(), Some(WeakReason::Guessable));
        // The server says why.
        m.weak_reason = Some(WeakReason::Temporary);
        assert_eq!(m.password_prompt(), Some(WeakReason::Temporary));
        m.secret_kind = "pin".into();
        m.weak_reason = Some(WeakReason::Pin);
        assert_eq!(m.password_prompt(), Some(WeakReason::Pin));
        // Later on the PIN's prompt: not again on this device…
        m.config.password_later.push("pin".into());
        assert_eq!(m.password_prompt(), None, "put off on this device");
        // …but a reset's temporary password still asks, once.
        m.secret_kind = "password".into();
        m.weak_reason = Some(WeakReason::Temporary);
        assert_eq!(m.password_prompt(), Some(WeakReason::Temporary), "a temporary password asks even after the PIN's Later");
        m.config.password_later.push("temporary".into());
        assert_eq!(m.password_prompt(), None);
        m.weak_reason = Some(WeakReason::Guessable);
        assert_eq!(m.password_prompt(), Some(WeakReason::Guessable));
        m.needs_pin = true;
        assert_eq!(m.password_prompt(), None, "no password at all: the other bar");
    }

    #[test]
    fn weak_reasons_on_the_wire() {
        for r in [WeakReason::Pin, WeakReason::Temporary, WeakReason::Guessable] {
            assert_eq!(WeakReason::from_wire(r.as_str()), Some(r));
        }
        assert_eq!(WeakReason::from_wire("something-new"), None);
    }
}

#[cfg(test)]
mod voice_tests {
    use super::*;
    use serde_json::json;

    /// An OBS link listens without showing up (the web's sidebar.js): it isn't
    /// listed under the channel, though it's still a session in it.
    #[test]
    fn obs_links_are_left_out_of_a_voice_channel() {
        let mut m = Model::new(Config::default());
        for (id, name) in [("u1", "Rosa"), ("u2", "ben")] {
            m.users.insert(id.into(), serde_json::from_value(json!({ "id": id, "name": name, "color": "#5b6cff" })).unwrap());
        }
        let session = |sid: &str, uid: &str, client: &str, channel: Option<&str>| -> Session {
            serde_json::from_value(json!({ "sid": sid, "uid": uid, "client": client, "voice": channel.map(|c| json!({ "channelId": c })) })).unwrap()
        };
        for s in [
            session("s1", "u1", "web", Some("lounge")),
            session("s2", "u2", "desktop", Some("lounge")),
            session("o1", "u1", "obs", Some("lounge")),
            session("s3", "u2", "web", Some("games")),
            session("s4", "u1", "desktop", None),
        ] {
            m.sessions.insert(s.sid.clone(), s);
        }
        let sids = |list: Vec<&Session>| list.iter().map(|s| s.sid.clone()).collect::<Vec<_>>();
        assert_eq!(sids(m.voice_members("lounge")), ["s2", "s1"], "A–Z by name, the OBS link left out");
        assert_eq!(sids(m.voice_members("games")), ["s3"]);
        assert!(m.voice_members("quiet").is_empty());
        assert!(m.sessions["o1"].is_obs(), "still a session in the call");
        assert!(m.obs_listening("lounge").is_empty(), "Rosa is in the call herself: her OBS link stays invisible");
    }

    /// While only someone's OBS link is in a channel, they show greyed out
    /// there ("Rosa's OBS is still listening"); once they're back, nothing.
    #[test]
    fn an_obs_link_without_its_account_shows_greyed_out() {
        let mut m = Model::new(Config::default());
        for (id, name) in [("u1", "Rosa"), ("u2", "ben"), ("u3", "Ada")] {
            m.users.insert(id.into(), serde_json::from_value(json!({ "id": id, "name": name, "color": "#5b6cff" })).unwrap());
        }
        let session = |sid: &str, uid: &str, client: &str, channel: Option<&str>| -> Session {
            serde_json::from_value(json!({ "sid": sid, "uid": uid, "client": client, "voice": channel.map(|c| json!({ "channelId": c, "muted": true })) })).unwrap()
        };
        for s in [
            session("s2", "u2", "desktop", Some("lounge")),
            // Rosa left the call (her app is in chat): her OBS link listens on, for now.
            session("s1", "u1", "web", None),
            session("o1", "u1", "obs", Some("lounge")),
            session("o1b", "u1", "obs", Some("lounge")),
            // Ada moved to another channel; her OBS link stayed in Lounge.
            session("s3", "u3", "desktop", Some("games")),
            session("o3", "u3", "obs", Some("lounge")),
        ] {
            m.sessions.insert(s.sid.clone(), s);
        }
        assert_eq!(m.obs_listening("lounge"), ["u3", "u1"], "A–Z by name, once each (Rosa has two OBS links there)");
        assert!(m.obs_listening("games").is_empty(), "Ada herself is in Games");
        let sids = |list: Vec<&Session>| list.iter().map(|s| s.sid.clone()).collect::<Vec<_>>();
        assert_eq!(sids(m.voice_members("lounge")), ["s2"], "not call members");
        // Rosa comes back: her OBS link is invisible again.
        m.sessions.get_mut("s1").unwrap().voice = Some(serde_json::from_value(json!({ "channelId": "lounge" })).unwrap());
        assert_eq!(m.obs_listening("lounge"), ["u3"]);
        // The server takes Ada's OBS link out 5 minutes after she left.
        m.sessions.get_mut("o3").unwrap().voice = None;
        assert!(m.obs_listening("lounge").is_empty());
    }

    /// PROTOCOL.md › Voice › Transmit gate: no "You're muted" while another of
    /// our devices is live in the call (we're talking through that one).
    #[test]
    fn live_on_another_device() {
        let mut m = Model::new(Config { uid: "me".into(), ..Config::default() });
        m.sid = Some("here".into());
        m.voice_channel = Some("lounge".into());
        let session = |sid: &str, uid: &str, client: &str, voice: serde_json::Value| -> Session {
            serde_json::from_value(json!({ "sid": sid, "uid": uid, "client": client, "voice": voice })).unwrap()
        };
        let live = |m: &mut Model, s: Session| {
            m.sessions.clear();
            m.sessions.insert("here".into(), session("here", "me", "desktop", json!({ "channelId": "lounge", "muted": true })));
            m.sessions.insert(s.sid.clone(), s);
            m.live_elsewhere()
        };
        assert!(live(&mut m, session("phone", "me", "ios", json!({ "channelId": "lounge" }))), "my phone, unmuted in this call");
        assert!(!live(&mut m, session("phone", "me", "ios", json!({ "channelId": "lounge", "muted": true }))), "muted there too");
        assert!(!live(&mut m, session("phone", "me", "ios", json!({ "channelId": "lounge", "deafened": true }))), "deafened there");
        assert!(!live(&mut m, session("phone", "me", "ios", json!({ "channelId": "games" }))), "another call");
        assert!(!live(&mut m, session("phone", "me", "ios", serde_json::Value::Null)), "not in voice");
        assert!(!live(&mut m, session("ben", "u2", "web", json!({ "channelId": "lounge" }))), "someone else");
        assert!(!live(&mut m, session("o1", "me", "obs", json!({ "channelId": "lounge" }))), "an OBS link");
        m.voice_channel = None;
        assert!(!live(&mut m, session("phone", "me", "ios", json!({ "channelId": "lounge" }))), "not in a call here");
    }
}
