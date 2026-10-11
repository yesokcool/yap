//! What the UI (and headless mode) asks the core to do, and what the core
//! reports back.

use super::*;

/// What Clear does once the page is safely in the gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterSave {
    /// A fresh page for everyone (`draw.clear`).
    Clear,
    /// Empty this layer for everyone (`draw.wipe`).
    Wipe(u8),
}

pub enum Cmd {
    OpenChannel(String),
    LoadOlder(String),
    /// Send text to the current channel (custom-emoji encoding already applied),
    /// optionally as a reply. Sealed first in an encrypted Personal space.
    Send { body: String, reply_to: Option<String> },
    Delete(String),
    Typing,
    CreateChannel { name: String, kind: String },
    ArchiveChannel(String),
    JoinVoice(String),
    LeaveVoice,
    ToggleMute,
    ToggleDeafen,
    /// Profile edit (already saved in model.config by the UI).
    Profile { name: bool, color: bool },
    SetVolume { uid: String },
    /// Audio settings changed in model.config: re-apply (restart mic, renegotiate).
    AudioSettings { restart_input: bool, restart_output: bool, renegotiate: bool },
    MicTest(bool),
    /// Settings › Voice › Hear yourself (headless: also save what came out of the codec here).
    HearYourself(Option<String>),
    /// Internal: Hear yourself finished playing.
    HearDone,
    /// Internal: reopen the mic and/or speakers (a stream error, a stall, a device change).
    AudioRestart { input: bool, output: bool },
    /// Tests: drop the socket now (it reconnects as after any blip).
    DropSocket,
    /// Ask `/api/ice` whether there's a relay (Settings › Voice's "Hide my IP address").
    CheckRelay,
    DeleteEmoji(String),
    UploadEmoji { name: String, bytes: Vec<u8> },
    /// Server/key changed (welcome screen): reconnect from scratch.
    Reconnect,
    /// Sign in (or sign up) on the current server with name + password.
    SignIn(Auth),
    /// Set or change our password (`current`: the password or old PIN, when there is one).
    SetPassword { password: String, current: Option<String> },
    SignOut,
    /// Owner/mod: give someone a temporary password.
    ResetPassword { uid: String },
    /// Settings › Account: list your devices / sign one out / sign out all the others.
    Devices,
    RevokeDevice(String),
    SignOutOthers,
    /// Owner/mods: ban someone (kicks every session).
    Ban { uid: String },
    /// Owner: make someone a mod, or a member again.
    SetRole { uid: String, role: String },
    /// The window came back to the front: skip any reconnect backoff.
    Wake,
    /// Let someone in from the waiting room (or turn them away).
    Approve { uid: String, approved: bool },
    /// Send a KLIPY GIF to the current channel, with an optional caption.
    SendGif { gif: crate::protocol::Gif, caption: String, reply_to: Option<String> },
    /// Send a picture that was staged (and shrunk in the background) with a caption.
    SendStagedPicture { slot: crate::picture::PrepSlot, caption: String, reply_to: Option<String> },
    /// Edit one of our messages (custom-emoji encoding already applied).
    Edit { id: String, body: String },
    /// Send a picture (raw file bytes; shrunk and re-encoded as needed) with a caption.
    SendPicture { bytes: Vec<u8>, caption: String },
    /// Personal invite links.
    CreateInvite(String),
    ListInvites,
    RevokeInvite(String),
    /// Owner only: reset the main invite link (`invite.resetMain`).
    ResetMainLink,
    /// Sidebar Places.
    AddLink { name: String, url: String },
    RemoveLink(String),
    /// Voice quality for everyone in a channel (None = each person's own).
    SetChannelBitrate { channel_id: String, bitrate: Option<u32> },
    /// Ask for the moderation log.
    ModLog,
    /// Internal: a picture finished uploading (`image` None = sealed into `body`).
    PictureUploaded { channel: String, body: String, image: Option<ImageRef>, reply_to: Option<String> },
    /// Encrypted Personal space: set up, unlock, change the passcode, start over, lock.
    Vault(VaultOp),
    /// Search one channel or everywhere (None); an empty query clears it.
    Search { q: String, channel: Option<String> },
    /// Pin or unpin a message / ask for a channel's pinned messages.
    Pin { id: String, on: bool },
    Pins(String),
    /// Open the channel and scroll to the message, loading older pages to find it.
    JumpTo { channel: String, id: String },
    /// Show a shared view (Draw, Space) instead of the chat, or go back (None).
    SetView(Option<View>),
    /// Drawing board: a live piece of my stroke (on layer `l`) / the finished
    /// stroke / undo / clear / empty one layer for everyone.
    DrawSeg { sk: String, c: String, w: f32, e: bool, p: Vec<f32>, l: u8 },
    DrawStroke(crate::protocol::Stroke),
    DrawUndo,
    DrawClear,
    DrawWipe(u8),
    /// Keep the board in the gallery: the 640 × 400 PNG, or the wiggle as a
    /// looping GIF (`gif`; one frame with `wiggle` off). `download`: also into
    /// Downloads; `share`: also post it in the chat; `then`: a fresh page, or
    /// one layer emptied, for everyone (only once the archive is safely uploaded).
    SaveDrawing { strokes: Vec<crate::protocol::Stroke>, gif: bool, wiggle: bool, download: bool, share: bool, then: Option<AfterSave> },
    /// Space: where I am / an emote over my head / my character (None = my picture).
    SpaceMove { x: f32, y: f32, d: f32 },
    SpaceEmote(String),
    SetLook(Option<String>),
    /// A Space mini-game update (`space.game`).
    SpaceGame { g: String, d: serde_json::Value },
    /// Space voice: per-session volume factors (distance or room).
    Proximity(Vec<(String, f32)>),
    /// Internal: a vault operation finished.
    VaultDone(Result<VaultResult, String>),
    /// Internal: earlier notes, sealed — hand them back (`personal.seal`), in
    /// batches that fit a frame; `failed`: notes that couldn't be sealed.
    SealBatches { batches: Vec<Vec<(String, String)>>, failed: usize },
    /// Add or remove our reaction on a message.
    React { id: String, emoji: String, on: bool },
    /// 📣 Ping everyone (optional note).
    Ping(String),
    // ── Extras (v0.36) ──
    /// A poll in the current channel (`/poll Q | A | B`).
    SendPoll { question: String, options: Vec<String> },
    /// Vote for an option (None = take your vote back).
    PollVote { id: String, option: Option<usize> },
    /// Soundboard: play a clip to your voice channel / delete one.
    PlaySound(String),
    DeleteSound(String),
    /// Hear a clip or chime here only (Settings, the upload dialog): a sound
    /// id, `chime:<built-in>`, or raw WAV bytes.
    PreviewSound(Preview),
    /// Upload a clip (`kind` "board" or "chime"): WAV bytes, ready to go.
    UploadSound { kind: String, name: String, emoji: Option<String>, wav: Vec<u8> },
    /// The media shelf: the first page, or the next older one (`more`).
    Shelf { channel: String, more: bool },
    /// Catch up: ask what's new in every unread channel.
    UnreadCounts,
    /// Catch up › Mark all read.
    MarkAllRead,
    /// Quiet hours (None = off).
    SetQuiet(Option<QuietHours>),
    /// Plan a hang / cancel one.
    SetHang { at: i64, title: String, channel: Option<String> },
    CancelHang(String),
    /// Your away message (None = back) / your join chime (None = default).
    SetAway(Option<(String, Option<i64>)>),
    SetChime(Option<String>),
    /// Open the web app signed in (a `device.handoff` pass) at a place;
    /// `video`: queue this YouTube id in Watch together first.
    OpenWeb { place: Option<String>, video: Option<String> },
    /// Bookmark a message into the Personal space (with an optional comment).
    SaveToPersonal { id: String, comment: String },
    /// React with a doodle: upload the image (GIF/PNG), then react `<d:ID>`.
    Doodle { message: String, bytes: Vec<u8> },
    /// Save a picture to Downloads (a sealed one decrypted with `key`).
    SavePicture { url: String, key: Option<crate::vault::Key> },
    /// Look for a newer release (`manual`: say "up to date" too).
    CheckUpdate { manual: bool },
    /// Download, verify and swap in the available update.
    InstallUpdate,
    /// Internal: results of background HTTP calls.
    ServerConfig(crate::http::ServerConfig),
    UpdateChecked { result: Result<Option<crate::update::Available>, String>, manual: bool },
    UpdateInstalled(Result<String, String>),
    Minecraft(crate::http::McStatus),
    /// Internal: stop audio if we're not in a call (after the leave blip).
    StopAudioIfIdle,
    // ── Screen sharing ──
    /// Share Screen: send `source` to whoever in the call asks for it.
    ShareScreen { source: crate::screen::capture::Source, quality: crate::screen::Quality, sound: bool },
    StopSharing,
    /// While sharing: Smooth / Sharp / Max, the computer's sound, or something else to share.
    ShareQuality(crate::screen::Quality),
    ShareSound(bool),
    ShareSource(crate::screen::capture::Source),
    /// Watch someone's screen (their sid; joins their voice channel if need be) / stop.
    WatchScreen(String),
    StopWatching,
    /// The share's sound volume for someone changed (already in config.screen_volumes).
    ScreenVolume { uid: String },
    Quit,
}

/// What to preview in Settings / the upload dialog.
#[derive(Clone, Debug)]
pub enum Preview {
    Sound(String),
    Chime(String),
    Wav(std::sync::Arc<Vec<u8>>),
}

#[derive(Clone, Debug)]
pub enum VaultOp {
    Setup { pass: String },
    Unlock { pass: String },
    Change { old: String, new: String },
    /// Forgot it: a new key (old notes stay locked).
    Reset { pass: String },
    Lock,
}

pub enum VaultResult {
    /// A new or re-wrapped key to send (`vault.set`) and use; `seal`: a new
    /// key, so seal the notes written before (`personal.plain`).
    Set { wrap: crate::vault::Wrap, key: crate::vault::Key, toast: &'static str, seal: bool },
    /// Unlocked with the passcode.
    Unlocked { key: crate::vault::Key },
}

/// Things worth logging in headless mode.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields are printed via Debug in headless mode
pub enum CoreEvent {
    Status(Status, String),
    Ready { sid: String },
    Message { id: String, channel: String, from: String, body: String, gif: bool, image: bool, reply: Option<String> },
    /// The Personal space key: "set", "unlocked", "locked", or why it failed.
    Vault(String),
    /// Earlier notes handed back sealed (`personal.seal`).
    Sealed(usize),
    /// Your account signed in somewhere new (the device's label).
    SigninAlert(String),
    /// Search results arrived (message ids, newest first).
    SearchResults(Vec<String>),
    Pinned { id: String, on: bool },
    Pins { channel: String, count: usize },
    /// A jump found its message after loading `pages` older pages (255 = gave up).
    Jumped { id: String, pages: u8 },
    Draw(String),
    Space(String),
    /// Watch together: "state <title>" / "state none", "watching <sid> on|off".
    Watch(String),
    /// Someone started or stopped sharing their screen in voice.
    Screen { sid: String, on: bool },
    Edited { id: String, body: String },
    Links(Vec<String>),
    Invite { label: String, link: String },
    Mention { from: String, body: String },
    Reaction { message_id: String, emoji: String, from: String, on: bool },
    Ping { from: String, text: String },
    Deleted { id: String },
    VoiceJoined { channel: String },
    Token(String),
    /// `used_by`: others who signed up with the same personal link (`invite.usedBy`).
    JoinRequest { uid: String, name: String, invited_by: Option<String>, used_by: Vec<String> },
    VoiceLeft,
    PeerState { sid: String, state: String },
    Error { code: String, message: String },
    /// The v0.36 extras, for headless logs: (tag, text), e.g. ("POLL", "…").
    Extra(&'static str, String),
}
