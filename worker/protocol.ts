// Yap wire protocol, version 1. PROTOCOL.md is the human-readable spec that
// every client (web, desktop, SwiftUI) implements; keep the two in sync.
//
// Transport: one WebSocket at /ws, the invite key in its subprotocol
// ("yap", "key.<key>"; or /ws?key=<key> for a key that can't be one). Every frame is a JSON
// object with a "t" (type) field, except the bare-text heartbeat
// "ping" -> "pong", which the runtime answers without waking the server.

export const PROTOCOL_VERSION = 1;

/** Emotes you can pop over your head in Space (keys 1–6). */
export const SPACE_EMOTES = ['👋', '😂', '❤️', '👍', '🎉', '💤'];

/** 'personal' = your own Personal space (only ever sent to you; id `p_<your uid>`). */
export type ChannelKind = 'text' | 'voice' | 'personal';

export interface Channel {
  id: string;
  name: string;
  kind: ChannelKind;
  position: number;
  /** Newest message time (ms since epoch); null when empty. Drives unread dots. */
  lastMessageAt: number | null;
  /** Voice channels: Opus bitrate everyone in it uses (bits/s); null = each person's own setting. */
  bitrate?: number | null;
}

/**
 * Your Personal space key, wrapped with your Personal space passcode
 * (PBKDF2-SHA256 → AES-GCM; see PROTOCOL.md › Encrypted Personal space).
 * The server stores it but can't open it.
 */
export interface VaultWrap {
  v: 1;
  iter: number;
  salt: string; // base64url, 16 bytes
  iv: string; // base64url, 12 bytes
  ct: string; // base64url, 48 bytes (the 32-byte key + GCM tag)
}

/** A personal invite link (the key itself is only shown once, when it's made). */
export interface Invite {
  id: string;
  label: string;
  /** uid of whoever made it. */
  by: string;
  createdAt: number;
  uses: number;
  lastUsedAt: number | null;
  revoked: boolean;
}

/** A sidebar Place: an outside link, or (`view`) something that opens inside Yap. */
export interface Link {
  id: string;
  name: string;
  url: string;
  view?: string;
  description?: string;
}

/** One line of the moderation log. `actor` is a uid, or "admin" for the admin API. */
export interface ModLogEntry {
  ts: number;
  actor: string | null;
  action: string;
  target: string | null;
  detail: string | null;
}

/** pending = waiting to be let in; banned = turned away / removed. */
export type Role = 'owner' | 'mod' | 'member' | 'pending' | 'banned';

/** Why ready.weakPin is true (it picks the "Choose a password" prompt's words): an old PIN, a temporary password from a reset, or a guessable password. */
export type WeakReason = 'pin' | 'temporary' | 'guessable';

/** Someone just let in from the waiting room (join.resolved.user). */
export type JoinedUser = Pick<User, 'id' | 'name' | 'color' | 'invitedBy'>;

export interface User {
  id: string;
  name: string;
  /** "#rrggbb" */
  color: string;
  lastSeen: number;
  role: Role;
  /** Media id of their avatar image (served at /media/<id>), or null. */
  avatar: string | null;
  /** Their character in the Space view: an emoji, or null for their picture/initial. */
  look: string | null;
  /** Waiting-room entries only: who invited them, e.g. "Ada (for Cleo)". */
  invitedBy?: string;
  /**
   * Waiting-room entries only, when they came with a personal invite link:
   * its maker's name, its label, and who else signed up with the same link
   * before (names, oldest first, ≤ 10) — links can be passed on.
   */
  invite?: { by: string; label: string; usedBy: string[] };
  /** An away message they set (AIM style); absent when they're not away. */
  away?: Away;
  /** Their join chime: a built-in id (CHIMES) or `s:<soundId>`; absent = "default". */
  chime?: string;
}

export interface Away {
  text: string;
  /** Ends at this time (ms); null = until they're back (their next message). */
  until: number | null;
  since: number;
}

/** Built-in join chimes, synthesized by every client (PROTOCOL.md › Extras › Join chimes). */
export const CHIMES = ['default', 'ding', 'chord', 'tada', 'bubble', 'knock', 'arcade', 'none'] as const;

/** A soundboard clip or an uploaded join chime (WAV at /media/<media>). */
export interface Sound {
  id: string;
  kind: 'board' | 'chime';
  name: string;
  emoji: string | null;
  media: string;
  /** Play at most this loud (measured at upload: ≈ −12 dBFS RMS, −3 dBFS peak). */
  gain: number;
  ms: number;
  uid: string;
  createdAt: number;
}

/** A plan made from a When's good slot: one push when it starts. */
export interface Hang {
  id: string;
  at: number;
  title: string;
  /** Voice channel to meet in, or null. */
  channelId: string | null;
  uid: string;
  createdAt: number;
}

/** Quiet hours for pushes (minutes after midnight in `tz`; the window may wrap). */
export interface QuietHours {
  from: number;
  to: number;
  tz: string;
  /** Let @mentions and replies through. */
  mentions: boolean;
  /** Snooze every push until this time (ms). */
  until?: number | null;
}

export interface Poll {
  options: string[];
  /** votes[i] = uids who picked option i. */
  votes: string[][];
}

/** A server-rolled /roll or /flip. */
export interface Roll {
  kind: 'roll' | 'flip';
  spec: string;
  rolls: number[];
  total: number;
}

/** Reactions on a message: each emoji (a character, or a custom `<:name:id>` token) and who added it. */
export interface Reaction {
  emoji: string;
  uids: string[];
}

export type NotifyMode = 'off' | 'pings' | 'all';

/** One finished pen stroke on the shared drawing board. Coordinates are 0..10000 (x then y, flattened). */
export interface Stroke {
  id: string;
  uid: string;
  /** "#rrggbb" */
  c: string;
  /** width in board units (board is 10000 wide) */
  w: number;
  /** eraser */
  e: boolean;
  p: number[];
  /** Kind: absent = freehand (smoothed); "poly" = straight segments (lines, boxes); "text" = `text` at p[0],p[1], size from w. */
  k?: 'poly' | 'text';
  text?: string;
  /** Layer 0–2 (absent = 0, the bottom); see PROTOCOL.md › Draw layers. */
  l?: number;
}

/** A picture in a message (stored in the Media object, served at /media/<id>). */
export interface ImageRef {
  id: string;
  width: number;
  height: number;
}

export interface SavedDrawing {
  id: string;
  media: string;
  uid: string;
  ts: number;
}

/** Where a pointer is on a shared view, anchored to content so it lines up across layouts. */
export interface CursorAnchor {
  /** card / lane = an element by id (x, y = fraction of its box); page = the whole page */
  k: 'card' | 'lane' | 'page';
  id?: string;
  x: number;
  y: number;
}

export interface VoiceState {
  channelId: string;
  /** Sharing their screen with the channel (video goes peer to peer to whoever asks). */
  screen?: boolean;
  muted: boolean;
  deafened: boolean;
  /** This session keeps calls across reconnects (PROTOCOL.md › Voice › Keeping calls across reconnects). */
  resume?: true;
  /** It continues the call of this earlier session (same person, checked by the server). */
  prev?: string;
}

/** One live connection. A user can have several (phone + desktop). */
export interface Session {
  sid: string;
  uid: string;
  client: string;
  voice: VoiceState | null;
  /** Shared view this session is looking at (e.g. "board"), or null for chat. */
  view: string | null;
  /** In the Watch together audience (even while browsing elsewhere in the app). */
  watching?: boolean;
}

/**
 * A GIF picked from KLIPY. URLs are KLIPY's own (static*.klipy.com), used
 * exactly as their API returned them — their terms require it.
 */
export interface Gif {
  provider: 'klipy';
  slug: string;
  title: string;
  width: number;
  height: number;
  mp4?: string;
  webm?: string;
  webp?: string;
  gif?: string;
  /** First-frame JPEG, for posters and reduced motion. */
  still?: string;
}

/** A custom emoji. `id` is the SHA-256 (first 32 hex chars) of the image bytes. */
export interface Emoji {
  id: string;
  name: string;
  mime: string;
  uid: string;
  createdAt: number;
}

export interface Message {
  id: string;
  channelId: string;
  uid: string;
  /** May be empty when the message is just a GIF. */
  body: string;
  ts: number;
  gif?: Gif;
  image?: ImageRef;
  reactions?: Reaction[];
  /** Set once the author has edited it (earlier versions are kept server-side until it's deleted). */
  editedAt?: number;
  /** The message this one answers (filled in by the server). */
  reply?: ReplyRef;
  /** Pinned to the channel (by anyone) at this time. */
  pinnedAt?: number;
  poll?: Poll;
  roll?: Roll;
}

/** Watch together: one shared YouTube player for the whole server. */
export interface WatchState {
  videoId: string | null;
  title: string | null;
  playing: boolean;
  /** Seconds into the video at server time `at`. */
  position: number;
  at: number;
  by: string | null;
  queue: { id: string; title: string | null; by: string }[];
}

/** One of your signed-in devices (Settings › Account). */
export interface Device {
  /** First 16 hex chars of the token's hash — enough to pick it, useless to sign in with. */
  id: string;
  client: string;
  /** "Chrome on Windows", "Yap for iPhone"… */
  label: string;
  createdAt: number;
  lastSeen: number;
  /** null = full account; 'listen' = an OBS link. */
  scope: string | null;
  /** The device asking. */
  current?: true;
}

/** A quoted message: who wrote it and how it started. */
export interface ReplyRef {
  id: string;
  uid: string;
  /** Where the original lives (differs from the reply's channel for Save to Personal). */
  channelId?: string;
  /** First 200 characters ('' once deleted). */
  body: string;
  gif?: true;
  image?: true;
  deleted?: true;
}

export type ClientMessage =
  /**
   * One of (checked in this order):
   *  - `token`: a device token from an earlier sign-in (normal reconnect);
   *  - `login`: { name, pin } — sign in on a new device (`pin`: the password, or an old 6-digit PIN);
   *  - `signup`: { name, pin, color } — new account (`pin`: a password, 8–128 characters; then the waiting room).
   * (The pre-PIN uid-only hello is gone: `pin_required`.)
   * `ready` / `pending` carry a fresh `token` whenever one was issued — store it.
   */
  | {
      t: 'hello';
      v: number;
      token?: string;
      login?: { name: string; pin: string };
      signup?: { name: string; pin: string; color?: string };
      client?: string;
    }
  /** `pin`: the new password (8–128 characters, else `bad_pin`; guessable → `weak_pin`); `current`: your password or old PIN. */
  | { t: 'account.pin'; pin: string; current?: string }
  | { t: 'account.signout' }
  /** Your signed-in devices → `devices`. */
  | { t: 'account.devices' }
  /** Sign out one of your other devices (id from `devices`) → `devices`. */
  | { t: 'account.revoke'; id: string }
  /** Sign out every device but this one, OBS links included → `devices`. */
  | { t: 'account.signoutOthers' }
  /** Owner/mod: a temporary password for someone → `pin.reset`. */
  | { t: 'member.resetPin'; uid: string }
  | { t: 'history'; channelId: string; before?: number; beforeId?: string; limit?: number }
  | { t: 'send'; channelId: string; body: string; nonce?: string; gif?: Gif; image?: ImageRef; replyTo?: string; poll?: { options: string[] } }
  /** Vote in a poll (option index), or take your vote back (null) → `poll`. */
  | { t: 'poll.vote'; id: string; option: number | null }
  /** Soundboard: play a clip to your voice channel → `sound`; delete one (uploader, owner/mods) → `sound.deleted`. */
  | { t: 'sound.play'; id: string }
  | { t: 'sound.delete'; id: string }
  /** Pictures and GIFs in a channel, newest first → `shelf`. */
  | { t: 'shelf'; channelId: string; before?: number }
  /** How much is new since your last-read times → `unread.counts`. */
  | { t: 'unread.counts'; since: Record<string, number> }
  /** Quiet hours for pushes → `notify`. */
  | { t: 'notify.quiet'; quiet: QuietHours | null }
  /** A plan from When's good (one push when it starts) / cancel it. */
  | { t: 'hang.set'; at: number; title: string; channelId?: string | null }
  | { t: 'hang.cancel'; id: string }
  | { t: 'delete'; id: string; blobs?: string[] }
  /** Edit your own message (text only; a GIF stays). */
  | { t: 'edit'; id: string; body: string }
  /** Personal invite links: make one (the key comes back once), list them, revoke one. */
  | { t: 'invite.create'; label: string }
  | { t: 'invite.list' }
  | { t: 'invite.revoke'; id: string }
  /** Sidebar Places: add an https link, or remove one. */
  | { t: 'link.add'; name: string; url: string }
  | { t: 'link.remove'; id: string }
  /** Voice channel quality for everyone in it (bits/s), or null for personal settings. */
  | { t: 'channel.bitrate'; channelId: string; bitrate: number | null }
  /** Read the moderation log (anyone) / storage use (owner and mods). */
  | { t: 'modlog' }
  | { t: 'stats' }
  /** OBS links: a listen-only device token for a Browser Source / turn all of yours off. */
  | { t: 'obs.create' }
  | { t: 'obs.revoke' }
  /** Browser pushes (Web Push): this browser's subscription, or forget it. */
  | { t: 'push.subscribe'; endpoint: string; keys: { p256dh: string; auth: string } }
  | { t: 'push.unsubscribe'; endpoint: string }
  /** A one-time sign-in pass (15 min) for a new home-screen app or device: hello with it as `token`. */
  | { t: 'device.handoff' }
  /** Store (or replace) your wrapped Personal space key; null removes it (back to unencrypted). */
  | { t: 'vault.set'; wrap: VaultWrap | null }
  /** Encrypting your Personal space: your notes that are still plain → `personal.plain`. */
  | { t: 'personal.plain' }
  /** …then hand them back sealed ("e1:…"), ≤ 50 at a time. Their GIF/picture moves inside the note. */
  | { t: 'personal.seal'; notes: { id: string; body: string }[] }
  /** Search messages → `search.results`. `q`: words (all must match), `from:Name`, `has:image|gif|link`. */
  | { t: 'search'; q: string; channelId?: string }
  /** Pin / unpin a message (anyone; ≤ 50 pins per channel) → `pinned` to everyone who can see it. */
  | { t: 'pin'; id: string; on: boolean }
  /** A channel's pinned messages → `pins`. */
  | { t: 'pins'; channelId: string }
  /** Space mini-games: a small state update relayed to everyone on Space (not stored). */
  | { t: 'space.game'; g: string; d: unknown }
  /** When's good: everyone's availability → `when.all`. */
  | { t: 'when.get' }
  /** Your whole availability, as half-hour slots since the epoch (replaces it). */
  | { t: 'when.set'; slots: number[] }
  | { t: 'typing'; channelId: string }
  | { t: 'profile'; name?: string; color?: string; avatar?: null; look?: string | null; away?: { text: string; until: number | null } | null; chime?: string | null }
  /** Add (`on: true`) or remove your reaction. */
  | { t: 'react'; id: string; emoji: string; on: boolean }
  | { t: 'channel.create'; name: string; kind: ChannelKind }
  | { t: 'channel.archive'; channelId: string }
  /**
   * `resume`: you keep calls alive across reconnects. `prev` + `proof`: this
   * join continues session `prev`'s call; `proof` = that session's
   * `ready.resumeKey`. `screen`: you're still sharing (a resumed call).
   */
  | { t: 'voice.join'; channelId: string; muted?: boolean; deafened?: boolean; resume?: boolean; prev?: string; proof?: string; screen?: boolean }
  | { t: 'voice.leave' }
  | { t: 'voice.state'; muted: boolean; deafened: boolean; screen?: boolean }
  /** Watch together (YouTube): join/leave the audience (you stay in it while browsing the app). */
  | { t: 'watch.join' }
  | { t: 'watch.leave' }
  | { t: 'watch.get' }
  /** Change what's playing: a new video (starts at 0, playing), play/pause, or seek. */
  | { t: 'watch.set'; videoId?: string; playing?: boolean; position?: number }
  /** Queue: add a video id, or remove the item at an index (≤ 20 queued). */
  | { t: 'watch.queue'; add?: string; remove?: number }
  /** The video ended: go to the next in the queue (only if `expect` is still what's playing). */
  | { t: 'watch.next'; expect: string }
  | { t: 'emoji.delete'; id: string }
  | { t: 'emoji.rename'; id: string; name: string }
  | { t: 'member.approve'; uid: string }
  | { t: 'member.deny'; uid: string }
  | { t: 'member.ban'; uid: string }
  | { t: 'member.role'; uid: string; role: 'mod' | 'member' }
  | { t: 'owner.claim'; key: string }
  | { t: 'view'; view: string | null }
  | { t: 'cursor'; view: string; a: CursorAnchor | null }
  /** Phone pushes: pick a mode; the reply carries your private ntfy topic. */
  | { t: 'notify.settings'; mode: NotifyMode }
  /** 📣 Everyone online hears a sound; everyone with pushes on gets one. */
  | { t: 'ping'; text?: string }
  | { t: 'draw.state' }
  /** Just the saved drawings (newest first, ≤ 60) — e.g. for the Space art gallery. */
  | { t: 'draw.gallery' }
  | { t: 'draw.seg'; sk: string; c: string; w: number; e: boolean; p: number[]; l?: number }
  | { t: 'draw.stroke'; sk: string; c: string; w: number; e: boolean; p: number[]; k?: 'poly' | 'text'; text?: string; l?: number }
  | { t: 'draw.undo'; id: string }
  | { t: 'draw.clear' }
  /** Empty one layer of the page for everyone. */
  | { t: 'draw.wipe'; l: number }
  | { t: 'space.state' }
  | { t: 'space.move'; x: number; y: number; d: number }
  /** A quick emote over your head in Space (one of SPACE_EMOTES). */
  | { t: 'space.emote'; e: string }
  /** WebRTC signaling, relayed verbatim to session `to`. See PROTOCOL.md for `data`. */
  | { t: 'signal'; to: string; data: unknown };

export type ServerMessage =
  | { t: 'build'; build: { app: string; style: string } }
  /**
   * `uid` is your canonical id — adopt it if it differs from what you sent (merged accounts).
   * `key`: the server's main invite key, so members who came in through a
   * personal invite keep working if that invite is revoked later.
   * `storageWarning` (owner/mods): a database is over 80% full.
   */
  | { t: 'ready'; v: number; sid: string; uid: string; token?: string; needsPin: boolean; notify: { mode: NotifyMode; topic: string | null; quiet?: QuietHours | null }; channels: Channel[]; users: User[]; sessions: Session[]; emojis: Emoji[]; pending: User[]; build?: { app: string; style: string }; key?: string; links?: Link[]; storageWarning?: boolean; uploadToken?: string; resumeKey?: string; vault?: VaultWrap | null; weakPin?: boolean; weakReason?: WeakReason; secretKind?: 'pin' | 'password'; sounds?: Sound[]; hangs?: Hang[] }
  | { t: 'poll'; id: string; channelId: string; votes: string[][] }
  /** Play this clip (someone in your voice channel pressed it). */
  | { t: 'sound'; id: string; sid: string; uid: string }
  | { t: 'sound.added'; sound: Sound }
  | { t: 'sound.deleted'; id: string }
  /** Only to you: someone you mentioned / replied to is away. */
  | { t: 'away'; uid: string; text: string; until: number | null }
  | { t: 'shelf'; channelId: string; items: Message[]; hasMore: boolean }
  /** `mentions`: how many of the `n` mention you (@you, @everyone/@here, or a reply to you). Older servers leave it out. */
  | { t: 'unread.counts'; counts: Record<string, { n: number; first: { id: string; ts: number; uid: string; body: string } | null; mentions: number }> }
  | { t: 'hang'; hang: Hang }
  | { t: 'hang.cancelled'; id: string }
  /** A hang is starting now. */
  | { t: 'hang.now'; id: string }
  | { t: 'invite.created'; invite: Invite; key: string }
  | { t: 'invites'; invites: Invite[] }
  | { t: 'links'; links: Link[] }
  | { t: 'modlog'; entries: ModLogEntry[] }
  | { t: 'stats'; chatBytes: number; mediaBytes: number; limitBytes: number }
  | { t: 'obs.created'; token: string }
  | { t: 'obs.revoked'; count: number }
  | { t: 'device.handoff'; token: string }
  | { t: 'vault'; wrap: VaultWrap | null }
  | { t: 'when.all'; people: { uid: string; slots: number[] }[] }
  /** Newest first, ≤ 50. Your encrypted Personal-space notes can't be searched here (search them on the device). */
  | { t: 'search.results'; q: string; channelId?: string; messages: Message[] }
  | { t: 'pinned'; id: string; channelId: string; pinnedAt: number | null; by: string }
  | { t: 'pins'; channelId: string; messages: Message[] }
  | { t: 'space.game'; sid: string; g: string; d: unknown }
  | { t: 'personal.plain'; notes: { id: string; body: string; gif?: Gif; image?: ImageRef }[] }
  | { t: 'when'; uid: string; slots: number[] }
  /** Instead of `ready` for a newcomer: wait until the owner or a mod lets you in (then `ready` arrives). */
  | { t: 'pending'; message: string; uid: string; token?: string }
  /** Password set/changed (`kind` is always "password" now); `token` is never sent any more. */
  | { t: 'account'; hasPin: boolean; token?: string; kind?: 'pin' | 'password' }
  | { t: 'devices'; devices: Device[] }
  /** Someone signed in to your account on a new device (sent to your other devices). */
  | { t: 'signin.alert'; device: Device }
  /** To the owner/mod who reset someone's password: the temporary one to pass on ("maple-river-stone-42"). */
  | { t: 'pin.reset'; uid: string; pin: string }
  /** Someone's waiting to be let in (owner and mods only). */
  | { t: 'join.request'; user: User }
  /** `user` (when let in): who joined, for everyone who never saw the request ("Cleo joined — invited by Ada"). */
  | { t: 'join.resolved'; uid: string; approved: boolean; by: string; user?: JoinedUser }
  | { t: 'view'; sid: string; view: string | null }
  | { t: 'cursor'; sid: string; view: string; a: CursorAnchor | null }
  | { t: 'notify'; mode: NotifyMode; topic: string | null; quiet?: QuietHours | null }
  | { t: 'ping'; from: string; text: string }
  | { t: 'draw.state'; canvas: number; strokes: Stroke[]; gallery: SavedDrawing[] }
  | { t: 'draw.gallery'; gallery: SavedDrawing[] }
  | { t: 'draw.seg'; sid: string; sk: string; c: string; w: number; e: boolean; p: number[]; l?: number }
  | { t: 'draw.stroke'; stroke: Stroke }
  | { t: 'draw.undo'; id: string }
  | { t: 'draw.clear'; canvas: number; by: string }
  | { t: 'draw.wipe'; l: number; by: string }
  | { t: 'drawing'; drawing: SavedDrawing }
  /** `me` = where you were last time (null the first time). */
  | { t: 'space.state'; players: { sid: string; x: number; y: number; d: number }[]; me: { x: number; y: number; d: number } | null }
  | { t: 'reaction'; messageId: string; channelId: string; emoji: string; uid: string; on: boolean }
  | { t: 'space.pos'; sid: string; x: number; y: number; d: number }
  | { t: 'space.emote'; sid: string; e: string }
  | { t: 'error'; code: string; message: string; re?: string }
  | { t: 'history'; channelId: string; messages: Message[]; hasMore: boolean }
  | { t: 'message'; message: Message; nonce?: string }
  | { t: 'deleted'; id: string; channelId: string }
  /** `sealed`: a Personal-space note was encrypted in place — drop its `gif`/`image` (now inside the note). */
  | { t: 'edited'; id: string; channelId: string; body: string; editedAt: number; sealed?: true }
  | { t: 'typing'; channelId: string; uid: string }
  | { t: 'user'; user: User }
  | { t: 'session.join'; session: Session }
  | { t: 'session.leave'; sid: string }
  | { t: 'channel'; channel: Channel }
  | { t: 'channel.archived'; channelId: string }
  /** `lost`: left because its socket closed (it may come back as a `prev`); `resume`/`prev`: see VoiceState. */
  | { t: 'voice'; sid: string; channelId: string | null; muted: boolean; deafened: boolean; screen?: boolean; resume?: true; prev?: string; lost?: true }
  /** What's on in Watch together. `now` = server time when sent (to sync clocks). */
  | { t: 'watch.state'; state: WatchState; now: number }
  /** Someone joined or left the Watch audience. */
  | { t: 'watching'; sid: string; on: boolean }
  | { t: 'emoji'; emoji: Emoji }
  | { t: 'emoji.deleted'; id: string }
  | { t: 'signal'; from: string; data: unknown };

export const LIMITS = {
  frameBytes: 384 * 1024,
  bodyChars: 40_000,
  /** A history page stops here even if it has fewer than `limit` messages. */
  historyChars: 512 * 1024,
  /** When's good: most slots one person can mark. */
  whenSlots: 1000,
  nameChars: 32,
  channelNameChars: 32,
  historyPage: 50,
  historyMax: 100,
  emojiBytes: 512 * 1024,
  /** Soundboard / chime clips (WAV PCM16 mono). */
  soundBytes: 480 * 1024,
  soundMs: 5000,
  chimeMs: 2000,
  sounds: 60,
  /** Doodle reactions. */
  doodleBytes: 64 * 1024,
  doodlePx: 256,
  pollOptions: 10,
  pollOptionChars: 80,
  pollQuestionChars: 300,
  awayChars: 80,
  hangTitleChars: 80,
  hangsUpcoming: 5,
  /** Draw: layers on the page (0 = bottom). */
  drawLayers: 3,
} as const;
