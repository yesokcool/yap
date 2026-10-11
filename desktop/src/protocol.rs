//! Yap wire protocol v1 (see ../PROTOCOL.md and ../worker/protocol.ts).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    /// "text" | "voice" | "personal"
    pub kind: String,
    #[serde(default)]
    pub position: i64,
    #[serde(rename = "lastMessageAt", default)]
    pub last_message_at: Option<i64>,
    /// Voice channels: the Opus bitrate everyone in it uses; None = each person's own.
    #[serde(default)]
    pub bitrate: Option<u32>,
}

impl Channel {
    pub fn is_text(&self) -> bool {
        self.kind == "text"
    }
    /// Your own Personal space (`p_<uid>`): a text channel only you see.
    pub fn is_personal(&self) -> bool {
        self.kind == "personal"
    }
    /// Somewhere you can chat: a text channel or your Personal space.
    pub fn is_chat(&self) -> bool {
        self.is_text() || self.is_personal()
    }
    pub fn is_voice(&self) -> bool {
        self.kind == "voice"
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub color: String,
    #[serde(rename = "lastSeen", default)]
    pub last_seen: i64,
    /// owner | mod | member | pending | banned
    #[serde(default = "member_role")]
    pub role: String,
    /// Media id of their picture (served at /media/<id>).
    #[serde(default)]
    pub avatar: Option<String>,
    /// Their Space-view character (an emoji).
    #[serde(default)]
    pub look: Option<String>,
    /// Waiting-room entries: who invited them, e.g. "Ada (for Ben)".
    #[serde(rename = "invitedBy", default, skip_serializing_if = "Option::is_none")]
    pub invited_by: Option<String>,
    /// Waiting-room entries who came with a personal invite link: whose link,
    /// its label, and who else signed up with the same link before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<InviteUse>,
    /// An away message (AIM style) while it's on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub away: Option<Away>,
    /// Their join chime: a built-in id or `s:<soundId>`; None = "default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chime: Option<String>,
}

/// A waiting-room entry's personal invite link (PROTOCOL.md › Identity ›
/// Waiting room): its maker's name, its label, and the names of the other
/// accounts that signed up with the same link before (oldest first, ≤ 10).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct InviteUse {
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub label: String,
    #[serde(rename = "usedBy", default)]
    pub used_by: Vec<String>,
}

impl InviteUse {
    /// "This link was also used by Noor — check with Priya." (None when nobody else used it.)
    pub fn warning(&self) -> Option<String> {
        (!self.used_by.is_empty()).then(|| format!("This link was also used by {} — check with {}.", name_list(&self.used_by), self.by))
    }
}

/// Names as a sentence, like the web's `nameList`: "Noor", "Noor and Iris",
/// "Noor, Iris and Lena", "Noor, Iris and 2 others".
pub fn name_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} others", rest.len()),
    }
}

/// An away message: `until` = ms, or None ("until I'm back").
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Away {
    pub text: String,
    #[serde(default)]
    pub until: Option<i64>,
    #[serde(default)]
    pub since: i64,
}

impl Away {
    /// Still on at `now` (ms)? (The server drops expired ones, but clients hide them once past.)
    pub fn on(&self, now: i64) -> bool {
        self.until.is_none_or(|u| u > now)
    }
}

/// A soundboard clip or an uploaded join chime (WAV at /media/<media>).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Sound {
    pub id: String,
    /// "board" | "chime"
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub emoji: Option<String>,
    pub media: String,
    /// Play at most this loud (measured at upload).
    #[serde(default = "one")]
    pub gain: f32,
    #[serde(default)]
    pub ms: u32,
    #[serde(default)]
    pub uid: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: i64,
}

fn one() -> f32 {
    1.0
}

/// A plan made for a time: one push when it starts.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Hang {
    pub id: String,
    pub at: i64,
    pub title: String,
    #[serde(rename = "channelId", default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub uid: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: i64,
}

/// Quiet hours for pushes (minutes after midnight in `tz`; may wrap past midnight).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct QuietHours {
    pub from: u32,
    pub to: u32,
    #[serde(default)]
    pub tz: String,
    /// Let @mentions and replies through.
    #[serde(default)]
    pub mentions: bool,
    /// Snooze everything until then (ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
}

/// Your push settings (`ready.notify`, `notify`).
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
pub struct NotifySettings {
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub quiet: Option<QuietHours>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct Poll {
    pub options: Vec<String>,
    /// votes[i] = uids who picked option i.
    #[serde(default)]
    pub votes: Vec<Vec<String>>,
}

/// A server-rolled /roll or /flip.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct Roll {
    /// "roll" | "flip"
    pub kind: String,
    #[serde(default)]
    pub spec: String,
    #[serde(default)]
    pub rolls: Vec<i64>,
    #[serde(default)]
    pub total: i64,
}

/// `unread.counts`: what's new in one channel since you last read it.
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
pub struct UnreadCount {
    #[serde(default)]
    pub n: u32,
    #[serde(default)]
    pub first: Option<UnreadFirst>,
    /// How many of the `n` mention you (@you, @everyone/@here, or a reply to
    /// you). None: an older server that doesn't count them (unknown, not 0).
    #[serde(default)]
    pub mentions: Option<u32>,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
pub struct UnreadFirst {
    pub id: String,
    #[serde(default)]
    pub ts: i64,
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub body: String,
}

fn member_role() -> String {
    "member".into()
}

/// One of your signed-in devices.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub label: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: i64,
    #[serde(rename = "lastSeen", default)]
    pub last_seen: i64,
    /// None = a full account; "listen" = an OBS link.
    #[serde(default)]
    pub scope: Option<String>,
    /// The device asking.
    #[serde(default)]
    pub current: bool,
}

/// A note written before encryption was on (`personal.plain`).
#[derive(Deserialize, Clone, Debug)]
pub struct PlainNote {
    pub id: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub gif: Option<Gif>,
    #[serde(default)]
    pub image: Option<ImageRef>,
}

/// The quoted message a reply answers (filled in by the server).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct ReplyRef {
    pub id: String,
    #[serde(default)]
    pub uid: String,
    /// Where the original lives (differs for Save to Personal).
    #[serde(rename = "channelId", default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// The original's first 200 characters ("" once it's deleted).
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub gif: bool,
    #[serde(default)]
    pub image: bool,
    #[serde(default)]
    pub deleted: bool,
}

/// A picture in a message (served at /media/<id>).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct ImageRef {
    pub id: String,
    pub width: f32,
    pub height: f32,
}

/// A personal invite link (the key itself only arrives once, in `invite.created`).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Invite {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub by: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: i64,
    #[serde(default)]
    pub uses: u32,
    #[serde(rename = "lastUsedAt", default)]
    pub last_used_at: Option<i64>,
    #[serde(default)]
    pub revoked: bool,
}

/// A sidebar Place: an outside link, or (`view`) something that opens inside Yap.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Link {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// One line of the moderation log. `actor` is a uid, or "admin".
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct ModLogEntry {
    pub ts: i64,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

impl Session {
    /// A listen-only OBS Browser Source (shown, but not "online").
    pub fn is_obs(&self) -> bool {
        self.client == "obs"
    }
}

impl User {
    /// Shown in the member list (not waiting, not banned).
    pub fn is_member(&self) -> bool {
        self.role != "pending" && self.role != "banned"
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct VoiceState {
    #[serde(rename = "channelId")]
    pub channel_id: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub deafened: bool,
    /// Sharing their screen in this channel (watched on the web for now).
    #[serde(default)]
    pub screen: bool,
    /// Their app keeps calls across reconnects.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resume: bool,
    /// The session this one continues (a resumed call), until it leaves voice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Session {
    pub sid: String,
    pub uid: String,
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub voice: Option<VoiceState>,
    /// A shared view it's on ("board", "draw", "space"), if any.
    #[serde(default)]
    pub view: Option<String>,
    /// In the Watch together audience.
    #[serde(default)]
    pub watching: bool,
}

/// Watch together: the shared YouTube player (only the bits we show).
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct WatchState {
    #[serde(rename = "videoId")]
    pub video_id: Option<String>,
    pub title: Option<String>,
    pub playing: bool,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct Gif {
    pub provider: String,
    pub slug: String,
    #[serde(default)]
    pub title: String,
    pub width: f32,
    pub height: f32,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mp4: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub webm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub webp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub gif: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub still: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Emoji {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub uid: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: i64,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    #[serde(rename = "channelId")]
    pub channel_id: String,
    pub uid: String,
    #[serde(default)]
    pub body: String,
    pub ts: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gif: Option<Gif>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
    /// Set once the author has edited it.
    #[serde(rename = "editedAt", default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
    /// The message this one answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<ReplyRef>,
    /// When it was pinned (None = not pinned).
    #[serde(rename = "pinnedAt", default, skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<Poll>,
    /// A /roll or /flip the server rolled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roll: Option<Roll>,
    /// Local only: what our optimistic copy answers (re-sent on reconnect).
    #[serde(skip)]
    pub reply_to: Option<String>,
    /// Local only: our optimistic copy, not yet echoed by the server.
    #[serde(skip)]
    pub pending: bool,
}

/// One emoji on a message (a character or a `<:name:id>` token) and who added it.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Reaction {
    pub emoji: String,
    #[serde(default)]
    pub uids: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "t")]
pub enum ServerMsg {
    #[serde(rename = "ready")]
    Ready {
        sid: String,
        /// Canonical uid (differs from ours after an account merge).
        #[serde(default)]
        uid: Option<String>,
        /// A fresh device token, when one was issued (store it).
        #[serde(default)]
        token: Option<String>,
        /// An account with no password at all (signed in with an old device token): ask it to set one.
        #[serde(rename = "needsPin", default)]
        needs_pin: bool,
        channels: Vec<Channel>,
        users: Vec<User>,
        sessions: Vec<Session>,
        #[serde(default)]
        emojis: Vec<Emoji>,
        /// People waiting to be let in (waiting room).
        #[serde(default)]
        pending: Vec<User>,
        /// The server's main invite key (keep it: personal invites can be revoked).
        #[serde(default)]
        key: Option<String>,
        /// Sidebar Places.
        #[serde(default)]
        links: Option<Vec<Link>>,
        /// Private to this connection: every upload sends it as `ut`.
        #[serde(rename = "uploadToken", default)]
        upload_token: Option<String>,
        /// Your encrypted Personal space's wrapped key (None = not encrypted).
        #[serde(default)]
        vault: Option<crate::vault::Wrap>,
        /// You last signed in with an old PIN, a guessable password or a reset's
        /// temporary one: ask once per device for a password (PROTOCOL.md › Identity).
        #[serde(rename = "weakPin", default)]
        weak_pin: bool,
        /// Why `weakPin` (only sent with it): "pin", "temporary" or "guessable".
        /// It picks the prompt's words; older servers leave it out.
        #[serde(rename = "weakReason", default)]
        weak_reason: Option<String>,
        /// "pin" or "password".
        #[serde(rename = "secretKind", default)]
        secret_kind: Option<String>,
        /// Push settings (with quiet hours).
        #[serde(default)]
        notify: Option<NotifySettings>,
        /// Soundboard clips and the chimes people use.
        #[serde(default)]
        sounds: Vec<Sound>,
        /// Upcoming hangs.
        #[serde(default)]
        hangs: Vec<Hang>,
        /// This session's voice resume key (PROTOCOL.md › Voice › Keeping calls across reconnects).
        #[serde(rename = "resumeKey", default)]
        resume_key: Option<String>,
    },
    /// Your signed-in devices (`account.devices`, `account.revoke`, `account.signoutOthers`).
    #[serde(rename = "devices")]
    Devices { devices: Vec<Device> },
    /// Your account just signed in on a new device.
    #[serde(rename = "signin.alert")]
    SigninAlert { device: Device },
    /// Your Personal space's wrapped key changed (here or on another device).
    #[serde(rename = "vault")]
    Vault {
        #[serde(default)]
        wrap: Option<crate::vault::Wrap>,
    },
    #[serde(rename = "edited")]
    Edited {
        id: String,
        #[serde(rename = "channelId")]
        channel_id: String,
        body: String,
        /// 0 = never edited (a note sealed in place).
        #[serde(rename = "editedAt", default)]
        edited_at: i64,
        /// A Personal-space note was encrypted in place: its gif/image are inside now.
        #[serde(default)]
        sealed: bool,
    },
    /// Your Personal-space notes that aren't encrypted yet (to seal them).
    #[serde(rename = "personal.plain")]
    PersonalPlain {
        #[serde(default)]
        notes: Vec<PlainNote>,
    },
    #[serde(rename = "invite.created")]
    InviteCreated { invite: Invite, key: String },
    #[serde(rename = "invites")]
    Invites { invites: Vec<Invite> },
    /// The owner reset the main invite link: this is the new main key (save it
    /// as your link; PROTOCOL.md › HTTP › Resetting the main link).
    #[serde(rename = "key")]
    Key {
        #[serde(default)]
        key: String,
    },
    #[serde(rename = "links")]
    Links { links: Vec<Link> },
    #[serde(rename = "modlog")]
    ModLog { entries: Vec<ModLogEntry> },
    /// Instead of `ready` for a newcomer: wait until a member lets you in.
    #[serde(rename = "pending")]
    Pending {
        #[serde(default)]
        message: String,
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        uid: Option<String>,
    },
    #[serde(rename = "reaction")]
    Reaction {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "channelId")]
        channel_id: String,
        emoji: String,
        uid: String,
        on: bool,
    },
    /// 📣 Someone pinged everyone.
    #[serde(rename = "ping")]
    Ping {
        from: String,
        #[serde(default)]
        text: String,
    },
    /// Password set/changed (`token` when this device didn't have one yet; never sent any more).
    #[serde(rename = "account")]
    Account {
        #[serde(rename = "hasPin", default)]
        has_pin: bool,
        #[serde(default)]
        token: Option<String>,
        /// "pin" or "password".
        #[serde(default)]
        kind: Option<String>,
    },
    /// Owner/mod reset someone's password: hand this temporary one over (`pin` is the wire's name).
    #[serde(rename = "pin.reset")]
    PinReset { uid: String, pin: String },
    #[serde(rename = "join.request")]
    JoinRequest { user: User },
    #[serde(rename = "join.resolved")]
    JoinResolved {
        uid: String,
        approved: bool,
        #[serde(default)]
        by: String,
        /// Who was let in (`{ id, name, color, invitedBy? }`), sent to everyone
        /// when someone is let in: members never saw the request. Older servers
        /// leave it out.
        #[serde(default)]
        user: Option<User>,
    },
    #[serde(rename = "error")]
    Error {
        code: String,
        #[serde(default)]
        message: String,
        #[serde(default)]
        re: Option<String>,
    },
    #[serde(rename = "history")]
    History {
        #[serde(rename = "channelId")]
        channel_id: String,
        messages: Vec<Message>,
        #[serde(rename = "hasMore")]
        has_more: bool,
    },
    #[serde(rename = "message")]
    Message {
        message: Message,
        #[serde(default)]
        nonce: Option<String>,
    },
    #[serde(rename = "deleted")]
    Deleted {
        id: String,
        #[serde(rename = "channelId")]
        channel_id: String,
    },
    #[serde(rename = "typing")]
    Typing {
        #[serde(rename = "channelId")]
        channel_id: String,
        uid: String,
    },
    #[serde(rename = "user")]
    User { user: User },
    #[serde(rename = "session.join")]
    SessionJoin { session: Session },
    #[serde(rename = "session.leave")]
    SessionLeave { sid: String },
    #[serde(rename = "channel")]
    Channel { channel: Channel },
    #[serde(rename = "channel.archived")]
    ChannelArchived {
        #[serde(rename = "channelId")]
        channel_id: String,
    },
    #[serde(rename = "voice")]
    Voice {
        sid: String,
        #[serde(rename = "channelId")]
        channel_id: Option<String>,
        #[serde(default)]
        muted: bool,
        #[serde(default)]
        deafened: bool,
        #[serde(default)]
        screen: bool,
        /// Keeps calls across reconnects.
        #[serde(default)]
        resume: bool,
        /// Continues this earlier session's call.
        #[serde(default)]
        prev: Option<String>,
        /// Left because its socket closed (it may come back as a `prev`).
        #[serde(default)]
        lost: bool,
    },
    #[serde(rename = "watch.state")]
    WatchState {
        #[serde(default)]
        state: WatchState,
    },
    #[serde(rename = "watching")]
    Watching { sid: String, on: bool },
    #[serde(rename = "emoji")]
    Emoji { emoji: Emoji },
    #[serde(rename = "emoji.deleted")]
    EmojiDeleted { id: String },
    #[serde(rename = "signal")]
    Signal { from: String, data: Value },
    #[serde(rename = "search.results")]
    SearchResults {
        #[serde(default)]
        q: String,
        #[serde(rename = "channelId", default)]
        channel_id: Option<String>,
        #[serde(default)]
        messages: Vec<Message>,
    },
    /// A message was pinned (`pinned_at` set) or unpinned.
    #[serde(rename = "pinned")]
    Pinned {
        id: String,
        #[serde(rename = "channelId")]
        channel_id: String,
        #[serde(rename = "pinnedAt", default)]
        pinned_at: Option<i64>,
        #[serde(default)]
        by: String,
    },
    #[serde(rename = "pins")]
    Pins {
        #[serde(rename = "channelId")]
        channel_id: String,
        #[serde(default)]
        messages: Vec<Message>,
    },
    /// A session switched shared views.
    #[serde(rename = "view")]
    View { sid: String, #[serde(default)] view: Option<String> },
    #[serde(rename = "draw.state")]
    DrawState {
        #[serde(default)]
        strokes: Vec<Stroke>,
        #[serde(default)]
        gallery: Vec<SavedDrawing>,
    },
    #[serde(rename = "draw.seg")]
    DrawSeg { sid: String, sk: String, #[serde(default)] c: String, #[serde(default)] w: f32, #[serde(default)] e: bool, #[serde(default)] p: Vec<f32>, #[serde(default)] l: u8 },
    #[serde(rename = "draw.stroke")]
    DrawStroke { stroke: Stroke },
    #[serde(rename = "draw.undo")]
    DrawUndo { id: String },
    #[serde(rename = "draw.clear")]
    DrawClear { #[serde(default)] by: String },
    /// One layer of the page emptied for everyone (PROTOCOL.md › Draw layers).
    #[serde(rename = "draw.wipe")]
    DrawWipe { l: u8, #[serde(default)] by: String },
    #[serde(rename = "drawing")]
    Drawing { drawing: SavedDrawing },
    /// Just the gallery (`draw.gallery`, for Space's Gallery): newest first, ≤ 60.
    #[serde(rename = "draw.gallery")]
    DrawGallery {
        #[serde(default)]
        gallery: Vec<SavedDrawing>,
    },
    #[serde(rename = "space.state")]
    SpaceState {
        #[serde(default)]
        players: Vec<SpacePos>,
        #[serde(default)]
        me: Option<SpacePos>,
    },
    #[serde(rename = "space.pos")]
    SpacePos(SpacePos),
    #[serde(rename = "space.emote")]
    SpaceEmote { sid: String, e: String },
    /// A Space mini-game update from someone else (`ball`, `tag`, `sync`).
    #[serde(rename = "space.game")]
    SpaceGame {
        sid: String,
        g: String,
        #[serde(default)]
        d: Value,
    },
    // ── Extras (v0.36) ──
    #[serde(rename = "poll")]
    Poll {
        id: String,
        #[serde(rename = "channelId")]
        channel_id: String,
        #[serde(default)]
        votes: Vec<Vec<String>>,
    },
    /// Someone in your voice channel played a soundboard clip.
    #[serde(rename = "sound")]
    Sound {
        id: String,
        #[serde(default)]
        uid: String,
    },
    #[serde(rename = "sound.added")]
    SoundAdded { sound: Sound },
    #[serde(rename = "sound.deleted")]
    SoundDeleted { id: String },
    /// Only to you: someone you mentioned or replied to is away.
    #[serde(rename = "away")]
    Away {
        uid: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        until: Option<i64>,
    },
    #[serde(rename = "shelf")]
    Shelf {
        #[serde(rename = "channelId")]
        channel_id: String,
        #[serde(default)]
        items: Vec<Message>,
        #[serde(rename = "hasMore", default)]
        has_more: bool,
    },
    #[serde(rename = "unread.counts")]
    UnreadCounts {
        #[serde(default)]
        counts: std::collections::HashMap<String, UnreadCount>,
    },
    #[serde(rename = "hang")]
    Hang { hang: Hang },
    #[serde(rename = "hang.cancelled")]
    HangCancelled { id: String },
    #[serde(rename = "hang.now")]
    HangNow { id: String },
    /// Your push settings changed (quiet hours…).
    #[serde(rename = "notify")]
    Notify {
        #[serde(default)]
        quiet: Option<QuietHours>,
    },
    /// A one-time sign-in pass, to open the web app signed in.
    #[serde(rename = "device.handoff")]
    DeviceHandoff { token: String },
    #[serde(other)]
    Unknown,
}

/// A finished stroke on the drawing board (board units: 10000 × 6250).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Default)]
pub struct Stroke {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub uid: String,
    /// "#rrggbb"
    #[serde(default)]
    pub c: String,
    #[serde(default)]
    pub w: f32,
    /// Eraser.
    #[serde(default)]
    pub e: bool,
    #[serde(default)]
    pub p: Vec<f32>,
    /// None = freehand (smoothed); "poly" = straight segments; "text" = `text` at p[0], p[1].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Layer: 0 = bottom (the default, and every stroke from before layers), 1 = middle, 2 = top.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub l: u8,
}

fn is_zero(v: &u8) -> bool {
    *v == 0
}

/// A drawing saved to the gallery (`/media/<media>`).
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct SavedDrawing {
    pub id: String,
    pub media: String,
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub ts: i64,
}

/// Where someone stands in Space (world 2400 × 1600; `d` = facing, radians).
#[derive(Deserialize, Clone, Debug, PartialEq, Default)]
pub struct SpacePos {
    #[serde(default)]
    pub sid: String,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub d: f32,
}

/// Client -> server frames, built as JSON values (they're small and one-off).
pub mod out {
    use super::*;

    pub fn hello_token(token: &str) -> Value {
        json!({ "t": "hello", "v": PROTOCOL_VERSION, "token": token, "client": "desktop" })
    }
    pub fn hello_login(name: &str, pin: &str) -> Value {
        json!({ "t": "hello", "v": PROTOCOL_VERSION, "login": { "name": name, "pin": pin }, "client": "desktop" })
    }
    pub fn hello_signup(name: &str, pin: &str, color: &str) -> Value {
        json!({ "t": "hello", "v": PROTOCOL_VERSION, "signup": { "name": name, "pin": pin, "color": color }, "client": "desktop" })
    }
    pub fn account_pin(pin: &str, current: Option<&str>) -> Value {
        match current {
            Some(c) => json!({ "t": "account.pin", "pin": pin, "current": c }),
            None => json!({ "t": "account.pin", "pin": pin }),
        }
    }
    pub fn account_devices() -> Value {
        json!({ "t": "account.devices" })
    }
    pub fn account_revoke(id: &str) -> Value {
        json!({ "t": "account.revoke", "id": id })
    }
    pub fn account_signout_others() -> Value {
        json!({ "t": "account.signoutOthers" })
    }
    pub fn account_signout() -> Value {
        json!({ "t": "account.signout" })
    }
    pub fn react(id: &str, emoji: &str, on: bool) -> Value {
        json!({ "t": "react", "id": id, "emoji": emoji, "on": on })
    }
    pub fn ping(text: &str) -> Value {
        if text.is_empty() { json!({ "t": "ping" }) } else { json!({ "t": "ping", "text": text }) }
    }
    pub fn member_ban(uid: &str) -> Value {
        json!({"t": "member.ban", "uid": uid})
    }
    pub fn member_role(uid: &str, role: &str) -> Value {
        json!({"t": "member.role", "uid": uid, "role": role})
    }
    pub fn member_reset_pin(uid: &str) -> Value {
        json!({ "t": "member.resetPin", "uid": uid })
    }
    pub fn history(channel_id: &str, before: Option<(i64, &str)>, limit: u32) -> Value {
        match before {
            Some((ts, id)) => json!({ "t": "history", "channelId": channel_id, "before": ts, "beforeId": id, "limit": limit }),
            None => json!({ "t": "history", "channelId": channel_id, "limit": limit }),
        }
    }
    pub fn send(channel_id: &str, body: &str, nonce: &str, gif: Option<&Gif>, reply_to: Option<&str>) -> Value {
        let mut v = json!({ "t": "send", "channelId": channel_id, "body": body, "nonce": nonce });
        if let Some(g) = gif {
            v["gif"] = serde_json::to_value(g).unwrap_or(Value::Null);
        }
        if let Some(r) = reply_to {
            v["replyTo"] = json!(r);
        }
        v
    }
    pub fn send_image(channel_id: &str, body: &str, nonce: &str, image: &ImageRef, reply_to: Option<&str>) -> Value {
        let mut v = json!({ "t": "send", "channelId": channel_id, "body": body, "nonce": nonce, "image": { "id": image.id, "width": image.width.round() as u32, "height": image.height.round() as u32 } });
        if let Some(r) = reply_to {
            v["replyTo"] = json!(r);
        }
        v
    }
    /// A poll: the question as the body, 2–10 options.
    pub fn send_poll(channel_id: &str, question: &str, nonce: &str, options: &[String]) -> Value {
        json!({ "t": "send", "channelId": channel_id, "body": question, "nonce": nonce, "poll": { "options": options } })
    }
    pub fn poll_vote(id: &str, option: Option<usize>) -> Value {
        json!({ "t": "poll.vote", "id": id, "option": option })
    }
    pub fn sound_play(id: &str) -> Value {
        json!({ "t": "sound.play", "id": id })
    }
    pub fn sound_delete(id: &str) -> Value {
        json!({ "t": "sound.delete", "id": id })
    }
    pub fn shelf(channel_id: &str, before: Option<i64>) -> Value {
        match before {
            Some(b) => json!({ "t": "shelf", "channelId": channel_id, "before": b }),
            None => json!({ "t": "shelf", "channelId": channel_id }),
        }
    }
    pub fn unread_counts(since: &std::collections::HashMap<String, i64>) -> Value {
        json!({ "t": "unread.counts", "since": since })
    }
    pub fn notify_quiet(quiet: Option<&QuietHours>) -> Value {
        json!({ "t": "notify.quiet", "quiet": quiet })
    }
    pub fn hang_set(at: i64, title: &str, channel_id: Option<&str>) -> Value {
        match channel_id {
            Some(c) => json!({ "t": "hang.set", "at": at, "title": title, "channelId": c }),
            None => json!({ "t": "hang.set", "at": at, "title": title }),
        }
    }
    pub fn hang_cancel(id: &str) -> Value {
        json!({ "t": "hang.cancel", "id": id })
    }
    /// Your away message (None = back).
    pub fn profile_away(away: Option<(&str, Option<i64>)>) -> Value {
        match away {
            Some((text, until)) => json!({ "t": "profile", "away": { "text": text, "until": until } }),
            None => json!({ "t": "profile", "away": null }),
        }
    }
    /// Your join chime: a built-in id or `s:<soundId>` (None = default).
    pub fn profile_chime(chime: Option<&str>) -> Value {
        json!({ "t": "profile", "chime": chime })
    }
    pub fn device_handoff() -> Value {
        json!({ "t": "device.handoff" })
    }
    pub fn watch_queue(add: &str) -> Value {
        json!({ "t": "watch.queue", "add": add })
    }
    pub fn vault_set(wrap: Option<&crate::vault::Wrap>) -> Value {
        json!({ "t": "vault.set", "wrap": wrap })
    }
    pub fn search(q: &str, channel_id: Option<&str>) -> Value {
        match channel_id {
            Some(c) => json!({ "t": "search", "q": q, "channelId": c }),
            None => json!({ "t": "search", "q": q }),
        }
    }
    pub fn pin(id: &str, on: bool) -> Value {
        json!({ "t": "pin", "id": id, "on": on })
    }
    pub fn pins(channel_id: &str) -> Value {
        json!({ "t": "pins", "channelId": channel_id })
    }
    pub fn view(view: Option<&str>) -> Value {
        json!({ "t": "view", "view": view })
    }
    pub fn draw_state() -> Value {
        json!({ "t": "draw.state" })
    }
    pub fn draw_gallery() -> Value {
        json!({ "t": "draw.gallery" })
    }
    /// `l`: the layer, written only when it isn't the bottom one.
    pub fn draw_seg(sk: &str, c: &str, w: f32, e: bool, p: &[f32], l: u8) -> Value {
        let mut v = json!({ "t": "draw.seg", "sk": sk, "c": c, "w": w, "e": e, "p": p });
        if l != 0 {
            v["l"] = json!(l);
        }
        v
    }
    pub fn draw_stroke(s: &Stroke) -> Value {
        let mut v = json!({ "t": "draw.stroke", "sk": s.id, "c": s.c, "w": s.w, "e": s.e, "p": s.p });
        if let Some(k) = &s.k {
            v["k"] = json!(k);
        }
        if let Some(t) = &s.text {
            v["text"] = json!(t);
        }
        if s.l != 0 {
            v["l"] = json!(s.l);
        }
        v
    }
    /// Empty one layer of the page for everyone.
    pub fn draw_wipe(l: u8) -> Value {
        json!({ "t": "draw.wipe", "l": l })
    }
    pub fn draw_undo(id: &str) -> Value {
        json!({ "t": "draw.undo", "id": id })
    }
    pub fn draw_clear() -> Value {
        json!({ "t": "draw.clear" })
    }
    pub fn space_state() -> Value {
        json!({ "t": "space.state" })
    }
    pub fn space_move(x: f32, y: f32, d: f32) -> Value {
        json!({ "t": "space.move", "x": x.round(), "y": y.round(), "d": (d * 100.0).round() / 100.0 })
    }
    pub fn space_game(g: &str, d: &Value) -> Value {
        json!({ "t": "space.game", "g": g, "d": d })
    }
    pub fn space_emote(e: &str) -> Value {
        json!({ "t": "space.emote", "e": e })
    }
    pub fn profile_look(look: Option<&str>) -> Value {
        json!({ "t": "profile", "look": look })
    }
    pub fn personal_plain() -> Value {
        json!({ "t": "personal.plain" })
    }
    /// Sealed bodies for notes from `personal.plain` (≤ 50 per message).
    pub fn personal_seal(notes: &[(String, String)]) -> Value {
        let notes: Vec<Value> = notes.iter().map(|(id, body)| json!({ "id": id, "body": body })).collect();
        json!({ "t": "personal.seal", "notes": notes })
    }
    pub fn delete_with_blobs(id: &str, blobs: &[String]) -> Value {
        json!({ "t": "delete", "id": id, "blobs": blobs })
    }
    pub fn edit(id: &str, body: &str) -> Value {
        json!({ "t": "edit", "id": id, "body": body })
    }
    pub fn invite_create(label: &str) -> Value {
        json!({ "t": "invite.create", "label": label })
    }
    pub fn invite_list() -> Value {
        json!({ "t": "invite.list" })
    }
    pub fn invite_revoke(id: &str) -> Value {
        json!({ "t": "invite.revoke", "id": id })
    }
    /// Owner only: a new main invite link (everyone connected gets `key`).
    pub fn invite_reset_main() -> Value {
        json!({ "t": "invite.resetMain" })
    }
    pub fn link_add(name: &str, url: &str) -> Value {
        json!({ "t": "link.add", "name": name, "url": url })
    }
    pub fn link_remove(id: &str) -> Value {
        json!({ "t": "link.remove", "id": id })
    }
    pub fn channel_bitrate(channel_id: &str, bitrate: Option<u32>) -> Value {
        json!({ "t": "channel.bitrate", "channelId": channel_id, "bitrate": bitrate })
    }
    pub fn modlog() -> Value {
        json!({ "t": "modlog" })
    }
    pub fn delete(id: &str) -> Value {
        json!({ "t": "delete", "id": id })
    }
    pub fn typing(channel_id: &str) -> Value {
        json!({ "t": "typing", "channelId": channel_id })
    }
    pub fn profile(name: Option<&str>, color: Option<&str>) -> Value {
        let mut v = json!({ "t": "profile" });
        if let Some(n) = name {
            v["name"] = json!(n);
        }
        if let Some(c) = color {
            v["color"] = json!(c);
        }
        v
    }
    pub fn channel_create(name: &str, kind: &str) -> Value {
        json!({ "t": "channel.create", "name": name, "kind": kind })
    }
    pub fn channel_archive(channel_id: &str) -> Value {
        json!({ "t": "channel.archive", "channelId": channel_id })
    }
    /// `resume: true` always (we keep calls across reconnects); `prev` = (our
    /// old sid, its resume key) when this join continues that session's call.
    /// `prev`/`proof`: this join continues our call from before a reconnect;
    /// `screen` then keeps our screen share going.
    pub fn voice_join(channel_id: &str, muted: bool, deafened: bool, prev: Option<(&str, &str)>, screen: bool) -> Value {
        let mut v = json!({ "t": "voice.join", "channelId": channel_id, "muted": muted, "deafened": deafened, "resume": true });
        if let Some((sid, proof)) = prev {
            v["prev"] = json!(sid);
            v["proof"] = json!(proof);
            if screen {
                v["screen"] = json!(true);
            }
        }
        v
    }
    pub fn voice_leave() -> Value {
        json!({ "t": "voice.leave" })
    }
    pub fn voice_state(muted: bool, deafened: bool, screen: bool) -> Value {
        json!({ "t": "voice.state", "muted": muted, "deafened": deafened, "screen": screen })
    }
    /// Ask a sharer for their screen (`want`), or say we're done with it.
    pub fn signal_screen(to: &str, want: bool) -> Value {
        json!({ "t": "signal", "to": to, "data": { "screen": if want { "want" } else { "stop" } } })
    }
    pub fn watch_get() -> Value {
        json!({ "t": "watch.get" })
    }
    pub fn signal(to: &str, data: &SignalData) -> Value {
        json!({ "t": "signal", "to": to, "data": data })
    }
    pub fn emoji_delete(id: &str) -> Value {
        json!({ "t": "emoji.delete", "id": id })
    }
    pub fn member_approve(uid: &str, approved: bool) -> Value {
        json!({ "t": if approved { "member.approve" } else { "member.deny" }, "uid": uid })
    }
}

/// `signal.data`: exactly one of `sdp` or `candidate`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SignalData {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sdp: Option<SdpJson>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub candidate: Option<CandidateJson>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SdpJson {
    #[serde(rename = "type")]
    pub kind: String,
    pub sdp: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CandidateJson {
    pub candidate: String,
    #[serde(rename = "sdpMid", default)]
    pub sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex", default)]
    pub sdp_mline_index: Option<u16>,
    #[serde(rename = "usernameFragment", default, skip_serializing_if = "Option::is_none")]
    pub username_fragment: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ready_and_voice() {
        let ready = r#"{"t":"ready","v":1,"sid":"s1","channels":[{"id":"general","name":"general","kind":"text","position":0,"lastMessageAt":null}],"users":[],"sessions":[{"sid":"s1","uid":"u","client":"web","voice":null}],"emojis":[]}"#;
        match serde_json::from_str::<ServerMsg>(ready).unwrap() {
            ServerMsg::Ready { sid, channels, .. } => {
                assert_eq!(sid, "s1");
                assert_eq!(channels[0].kind, "text");
            }
            other => panic!("{other:?}"),
        }
        let screen = r#"{"t":"voice","sid":"s1","channelId":"v","muted":false,"deafened":false,"screen":true}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(screen).unwrap(), ServerMsg::Voice { screen: true, .. }));
        let watch = r#"{"t":"watch.state","state":{"videoId":"dQw4w9WgXcQ","title":"Never","playing":true,"position":12.5,"at":1,"by":"u","queue":[{"id":"x","title":null,"by":"u"}]},"now":2}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(watch).unwrap(), ServerMsg::WatchState { state: WatchState { playing: true, .. } }));
        let empty = r#"{"t":"watch.state","state":{"videoId":null,"title":null,"playing":false,"position":0,"at":0,"by":null,"queue":[]},"now":2}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(empty).unwrap(), ServerMsg::WatchState { state: WatchState { video_id: None, .. } }));
        let watching = r#"{"t":"watching","sid":"s1","on":true}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(watching).unwrap(), ServerMsg::Watching { on: true, .. }));
        let s: Session = serde_json::from_str(r#"{"sid":"s","uid":"u","client":"web","voice":{"channelId":"v","muted":false,"deafened":false,"screen":true},"view":null,"watching":true}"#).unwrap();
        assert!(s.watching && s.voice.unwrap().screen);
        let voice = r#"{"t":"voice","sid":"s1","channelId":null,"muted":false,"deafened":false}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(voice).unwrap(), ServerMsg::Voice { channel_id: None, .. }));
        let unknown = r#"{"t":"something.new","x":1}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(unknown).unwrap(), ServerMsg::Unknown));
    }

    #[test]
    fn parses_voice_resume_fields() {
        let ready = r#"{"t":"ready","v":1,"sid":"s2","channels":[],"users":[],"sessions":[{"sid":"s2","uid":"u","client":"desktop","voice":{"channelId":"v","muted":false,"deafened":false,"resume":true,"prev":"s1"}}],"emojis":[],"resumeKey":"k"}"#;
        match serde_json::from_str::<ServerMsg>(ready).unwrap() {
            ServerMsg::Ready { resume_key, sessions, .. } => {
                assert_eq!(resume_key.as_deref(), Some("k"));
                let v = sessions[0].voice.clone().unwrap();
                assert!(v.resume);
                assert_eq!(v.prev.as_deref(), Some("s1"));
            }
            other => panic!("{other:?}"),
        }
        let lost = r#"{"t":"voice","sid":"s1","channelId":null,"muted":false,"deafened":false,"lost":true}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(lost).unwrap(), ServerMsg::Voice { lost: true, resume: false, .. }));
        let resumed = r#"{"t":"voice","sid":"s2","channelId":"v","muted":false,"deafened":false,"resume":true,"prev":"s1"}"#;
        match serde_json::from_str::<ServerMsg>(resumed).unwrap() {
            ServerMsg::Voice { resume, prev, lost, .. } => assert!(resume && prev.as_deref() == Some("s1") && !lost),
            other => panic!("{other:?}"),
        }
        // An older server: none of it.
        let old = r#"{"t":"voice","sid":"s1","channelId":"v","muted":false,"deafened":false}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(old).unwrap(), ServerMsg::Voice { resume: false, prev: None, lost: false, .. }));
        let join = out::voice_join("v", false, true, None, true);
        assert_eq!(join, serde_json::json!({ "t": "voice.join", "channelId": "v", "muted": false, "deafened": true, "resume": true }), "screen only with prev");
        let again = out::voice_join("v", false, false, Some(("s1", "proof")), false);
        assert_eq!(again["prev"], "s1");
        assert_eq!(again["proof"], "proof");
        assert!(again.get("screen").is_none());
        // A share kept across a reconnect says so (PROTOCOL.md › Keeping calls across reconnects).
        assert_eq!(out::voice_join("v", false, false, Some(("s1", "proof")), true)["screen"], true);
        assert_eq!(out::voice_state(true, false, true), serde_json::json!({ "t": "voice.state", "muted": true, "deafened": false, "screen": true }));
        assert_eq!(out::signal_screen("s2", true), serde_json::json!({ "t": "signal", "to": "s2", "data": { "screen": "want" } }));
        assert_eq!(out::signal_screen("s2", false)["data"]["screen"], "stop");
        // What we'd ever serialize of a VoiceState leaves the new fields out when unset.
        let vs = VoiceState { channel_id: "v".into(), muted: false, deafened: false, screen: false, resume: false, prev: None };
        assert!(!serde_json::to_string(&vs).unwrap().contains("resume"));
    }

    /// Fields the app doesn't read (`ready.v`, `sound.sid`) still parse: they're just ignored.
    #[test]
    fn ignores_fields_it_doesnt_read() {
        let sound = r#"{"t":"sound","id":"k1","sid":"s1","uid":"u1"}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(sound).unwrap(), ServerMsg::Sound { id, uid } if id == "k1" && uid == "u1"));
    }

    #[test]
    fn parses_new_shapes() {
        let ready = r#"{"t":"ready","v":1,"sid":"s1","channels":[{"id":"v","name":"Lounge","kind":"voice","position":1,"lastMessageAt":null,"bitrate":256000}],"users":[],"sessions":[{"sid":"o","uid":"u","client":"obs","voice":null}],"emojis":[],"key":"main","links":[{"id":"l1","name":"Board","url":"https://x/board","view":"board"}]}"#;
        match serde_json::from_str::<ServerMsg>(ready).unwrap() {
            ServerMsg::Ready { channels, sessions, key, links, .. } => {
                assert_eq!(channels[0].bitrate, Some(256000));
                assert!(sessions[0].is_obs());
                assert_eq!(key.as_deref(), Some("main"));
                assert_eq!(links.unwrap()[0].view.as_deref(), Some("board"));
            }
            other => panic!("{other:?}"),
        }
        let msg = r#"{"t":"message","message":{"id":"m","channelId":"c","uid":"u","body":"hi","ts":1,"image":{"id":"abc","width":640,"height":480},"editedAt":5}}"#;
        match serde_json::from_str::<ServerMsg>(msg).unwrap() {
            ServerMsg::Message { message, .. } => {
                assert_eq!(message.image.unwrap().width, 640.0);
                assert_eq!(message.edited_at, Some(5));
            }
            other => panic!("{other:?}"),
        }
        let reply = r#"{"t":"message","message":{"id":"m2","channelId":"c","uid":"u","body":"yes","ts":2,"reply":{"id":"m","uid":"v","body":"are you coming?"}}}"#;
        match serde_json::from_str::<ServerMsg>(reply).unwrap() {
            ServerMsg::Message { message, .. } => assert_eq!(message.reply.unwrap().body, "are you coming?"),
            other => panic!("{other:?}"),
        }
        let vault = r#"{"t":"vault","wrap":{"v":1,"iter":600000,"salt":"AAAA","iv":"BBBB","ct":"CCCC"}}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(vault).unwrap(), ServerMsg::Vault { wrap: Some(_) }));
        let edited = r#"{"t":"edited","id":"m","channelId":"c","body":"new","editedAt":9}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(edited).unwrap(), ServerMsg::Edited { edited_at: 9, sealed: false, .. }));
        let sealed = r#"{"t":"edited","id":"m","channelId":"c","body":"e1:x","editedAt":0,"sealed":true}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(sealed).unwrap(), ServerMsg::Edited { edited_at: 0, sealed: true, .. }));
        let devices = r#"{"t":"devices","devices":[{"id":"0123456789abcdef","client":"desktop","label":"Desktop app (Mac)","createdAt":1,"lastSeen":2,"scope":null,"current":true},{"id":"fedcba9876543210","client":"web","label":"Chrome on Windows","createdAt":1,"lastSeen":2,"scope":"listen"}]}"#;
        match serde_json::from_str::<ServerMsg>(devices).unwrap() {
            ServerMsg::Devices { devices } => assert!(devices[0].current && !devices[1].current && devices[1].scope.as_deref() == Some("listen")),
            _ => panic!("devices"),
        }
        let alert = r#"{"t":"signin.alert","device":{"id":"0123456789abcdef","client":"web","label":"Safari on iPhone","createdAt":1,"lastSeen":2,"scope":null}}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(alert).unwrap(), ServerMsg::SigninAlert { .. }));
        let res = r#"{"t":"search.results","q":"pizza","messages":[{"id":"m","channelId":"c","uid":"u","body":"pizza?","ts":5,"pinnedAt":9}]}"#;
        match serde_json::from_str::<ServerMsg>(res).unwrap() {
            ServerMsg::SearchResults { q, channel_id: None, messages } => assert!(q == "pizza" && messages[0].pinned_at == Some(9)),
            _ => panic!("search.results"),
        }
        let unpin = r#"{"t":"pinned","id":"m","channelId":"c","pinnedAt":null,"by":"u"}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(unpin).unwrap(), ServerMsg::Pinned { pinned_at: None, .. }));
        let st = r##"{"t":"draw.state","canvas":3,"strokes":[{"id":"s","uid":"u","c":"#000000","w":32,"e":false,"p":[1,2,3,4]},{"id":"t","uid":"u","c":"#ff0000","w":14,"e":false,"p":[5,6],"k":"text","text":"hi"}],"gallery":[{"id":"g","media":"0123","uid":"u","ts":1}]}"##;
        match serde_json::from_str::<ServerMsg>(st).unwrap() {
            ServerMsg::DrawState { strokes, gallery } => assert!(strokes.len() == 2 && strokes[1].text.as_deref() == Some("hi") && gallery.len() == 1),
            _ => panic!("draw.state"),
        }
        let gal = r#"{"t":"draw.gallery","gallery":[{"id":"g","media":"0123456789abcdef","uid":"u","ts":1}]}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(gal).unwrap(), ServerMsg::DrawGallery { gallery } if gallery.len() == 1));
        let pos = r#"{"t":"space.pos","sid":"s","x":10.5,"y":20,"d":1.57}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(pos).unwrap(), ServerMsg::SpacePos(SpacePos { x: 10.5, .. })));
        let sst = r#"{"t":"space.state","players":[{"sid":"a","x":1,"y":2,"d":0}],"me":null}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(sst).unwrap(), ServerMsg::SpaceState { me: None, .. }));
        let stroke = Stroke { id: "s1".into(), c: "#000000".into(), w: 32.0, p: vec![1.0, 2.0], k: Some("poly".into()), ..Default::default() };
        let v = out::draw_stroke(&stroke);
        assert!(v["sk"] == "s1" && v["k"] == "poly" && v.get("text").is_none() && v.get("uid").is_none());
        assert!(v.get("l").is_none(), "the bottom layer isn't written");
        let plain = r#"{"t":"personal.plain","notes":[{"id":"a","body":"hi"},{"id":"b","body":"","image":{"id":"0123","width":10,"height":20}}]}"#;
        match serde_json::from_str::<ServerMsg>(plain).unwrap() {
            ServerMsg::PersonalPlain { notes } => assert!(notes.len() == 2 && notes[1].image.is_some()),
            _ => panic!("personal.plain"),
        }
        let inv = r#"{"t":"invite.created","invite":{"id":"i","label":"Ben","by":"u","createdAt":1,"uses":0,"lastUsedAt":null,"revoked":false},"key":"yi_abc"}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(inv).unwrap(), ServerMsg::InviteCreated { .. }));
        // The main link was reset: the new main key, to everyone connected.
        let key = r#"{"t":"key","key":"k2_new-Main"}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(key).unwrap(), ServerMsg::Key { key } if key == "k2_new-Main"));
        assert_eq!(out::invite_reset_main(), serde_json::json!({ "t": "invite.resetMain" }));
        let user = r##"{"id":"u","name":"Cleo","color":"#fff","lastSeen":0,"role":"pending","avatar":null,"look":null,"invitedBy":"Ada (for Cleo)"}"##;
        let cleo = serde_json::from_str::<User>(user).unwrap();
        assert_eq!(cleo.invited_by.as_deref(), Some("Ada (for Cleo)"));
        assert_eq!(cleo.invite, None, "older servers send no invite");
        let v = out::channel_bitrate("v", None);
        assert!(v["bitrate"].is_null());
        let v = out::send_image("c", "", "n", &ImageRef { id: "i".into(), width: 10.0, height: 20.0 }, Some("m1"));
        assert_eq!(v["replyTo"], "m1");
        assert_eq!(v["image"]["height"], 20);
    }

    /// A waiting-room entry's `invite: { by, label, usedBy }` (PROTOCOL.md › Identity › Waiting room).
    #[test]
    fn invite_used_by() {
        let pending = r##"{"t":"join.request","user":{"id":"o","name":"Omar","color":"#fff","lastSeen":0,"role":"pending","avatar":null,"look":null,"invitedBy":"Priya (for Noor)","invite":{"by":"Priya","label":"Noor","usedBy":["Noor","Theo","Lena"]}}}"##;
        let ServerMsg::JoinRequest { user } = serde_json::from_str::<ServerMsg>(pending).unwrap() else { panic!("join.request") };
        let inv = user.invite.clone().unwrap();
        assert_eq!((inv.by.as_str(), inv.label.as_str(), inv.used_by.len()), ("Priya", "Noor", 3));
        assert_eq!(inv.warning().as_deref(), Some("This link was also used by Noor, Theo and Lena — check with Priya."));
        // Nobody else used it: nothing to say.
        let first: User = serde_json::from_str(r##"{"id":"n","name":"Noor","color":"#fff","role":"pending","invite":{"by":"Priya","label":"Noor","usedBy":[]}}"##).unwrap();
        assert_eq!(first.invite.unwrap().warning(), None);
        // No usedBy at all reads as empty.
        let bare: User = serde_json::from_str(r##"{"id":"n","name":"Noor","color":"#fff","invite":{"by":"Priya","label":"Noor"}}"##).unwrap();
        assert!(bare.invite.unwrap().used_by.is_empty());
        let names = |n: &[&str]| name_list(&n.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(names(&["Noor"]), "Noor");
        assert_eq!(names(&["Noor", "Iris"]), "Noor and Iris");
        assert_eq!(names(&["A", "B", "C", "D", "E"]), "A, B and 3 others");
    }

    /// `unread.counts` with and without `mentions` (older servers: unknown, not 0).
    #[test]
    fn unread_counts_mentions() {
        let msg = r#"{"t":"unread.counts","counts":{"general":{"n":4,"first":{"id":"m","ts":5,"uid":"u","body":"hi"},"mentions":2},"games":{"n":1,"first":null}}}"#;
        let ServerMsg::UnreadCounts { counts } = serde_json::from_str::<ServerMsg>(msg).unwrap() else { panic!("unread.counts") };
        assert_eq!(counts["general"].mentions, Some(2));
        assert_eq!(counts["games"].mentions, None);
        assert_eq!(counts["games"].n, 1);
    }

    /// Layers (PROTOCOL.md › Draw layers): absent = 0, 0 isn't written, 2 is kept.
    #[test]
    fn stroke_layers_round_trip() {
        let old = r##"{"id":"s","uid":"u","c":"#000000","w":32,"e":false,"p":[1,2,3,4]}"##;
        let s: Stroke = serde_json::from_str(old).unwrap();
        assert_eq!(s.l, 0, "strokes from before layers are on the bottom one");
        let back = serde_json::to_value(&s).unwrap();
        assert!(back.get("l").is_none(), "0 isn't written");
        let top: Stroke = serde_json::from_str(r##"{"id":"t","uid":"u","c":"#ff0000","w":14,"e":true,"p":[5,6],"l":2}"##).unwrap();
        assert_eq!(top.l, 2);
        assert_eq!(serde_json::to_value(&top).unwrap()["l"], 2);
        assert_eq!(serde_json::from_value::<Stroke>(serde_json::to_value(&top).unwrap()).unwrap(), top);
        assert_eq!(out::draw_stroke(&top)["l"], 2);
        // Live pieces: sent with l only off the bottom layer; heard with or without it.
        assert!(out::draw_seg("a", "#000000", 14.0, false, &[1.0, 2.0], 0).get("l").is_none());
        assert_eq!(out::draw_seg("a", "#000000", 14.0, false, &[1.0, 2.0], 2)["l"], 2);
        let seg = r##"{"t":"draw.seg","sid":"x","sk":"a","c":"#000000","w":14,"e":false,"p":[1,2],"l":1}"##;
        assert!(matches!(serde_json::from_str::<ServerMsg>(seg).unwrap(), ServerMsg::DrawSeg { l: 1, .. }));
        let seg0 = r##"{"t":"draw.seg","sid":"x","sk":"a","c":"#000000","w":14,"e":false,"p":[1,2]}"##;
        assert!(matches!(serde_json::from_str::<ServerMsg>(seg0).unwrap(), ServerMsg::DrawSeg { l: 0, .. }));
        // The wipe, both ways.
        assert_eq!(out::draw_wipe(1), serde_json::json!({ "t": "draw.wipe", "l": 1 }));
        match serde_json::from_str::<ServerMsg>(r#"{"t":"draw.wipe","l":2,"by":"u"}"#).unwrap() {
            ServerMsg::DrawWipe { l, by } => assert!(l == 2 && by == "u"),
            _ => panic!("draw.wipe"),
        }
    }

    /// PROTOCOL.md › Identity: why the "Choose a password" prompt asks
    /// (`ready.weakReason`); an older server doesn't say.
    #[test]
    fn parses_weak_reason() {
        let ready = r#"{"t":"ready","v":1,"sid":"s","channels":[],"users":[],"sessions":[],"emojis":[],"weakPin":true,"weakReason":"temporary","secretKind":"password"}"#;
        match serde_json::from_str::<ServerMsg>(ready).unwrap() {
            ServerMsg::Ready { weak_pin, weak_reason, secret_kind, .. } => {
                assert!(weak_pin);
                assert_eq!(weak_reason.as_deref(), Some("temporary"));
                assert_eq!(secret_kind.as_deref(), Some("password"));
            }
            other => panic!("{other:?}"),
        }
        let older = r#"{"t":"ready","v":1,"sid":"s","channels":[],"users":[],"sessions":[],"emojis":[],"weakPin":true,"secretKind":"pin"}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(older).unwrap(), ServerMsg::Ready { weak_reason: None, .. }));
    }

    /// PROTOCOL.md › Identity › Waiting room: who was let in
    /// (`join.resolved.user`), for members who never saw the request; not
    /// sent when someone is turned away, nor by an older server.
    #[test]
    fn parses_who_joined() {
        let joined = r##"{"t":"join.resolved","uid":"c","approved":true,"by":"a","user":{"id":"c","name":"Cleo","color":"#30a46c","invitedBy":"Ada (for Cleo)"}}"##;
        match serde_json::from_str::<ServerMsg>(joined).unwrap() {
            ServerMsg::JoinResolved { uid, approved, by, user } => {
                assert!(approved && uid == "c" && by == "a");
                let user = user.unwrap();
                assert_eq!((user.id.as_str(), user.name.as_str(), user.color.as_str()), ("c", "Cleo", "#30a46c"));
                assert_eq!(user.invited_by.as_deref(), Some("Ada (for Cleo)"));
            }
            other => panic!("{other:?}"),
        }
        let turned_away = r#"{"t":"join.resolved","uid":"c","approved":false,"by":"a"}"#;
        assert!(matches!(serde_json::from_str::<ServerMsg>(turned_away).unwrap(), ServerMsg::JoinResolved { approved: false, user: None, .. }));
    }

    #[test]
    fn signal_roundtrip() {
        let s = r#"{"candidate":{"candidate":"candidate:1 1 udp 1 1.2.3.4 5 typ host","sdpMid":"0","sdpMLineIndex":0,"usernameFragment":"ab"}}"#;
        let d: SignalData = serde_json::from_str(s).unwrap();
        assert_eq!(d.candidate.as_ref().unwrap().sdp_mline_index, Some(0));
        let back = serde_json::to_string(&d).unwrap();
        assert!(back.contains("\"sdpMLineIndex\":0"));
        assert!(!back.contains("sdp\":"));
    }
}
