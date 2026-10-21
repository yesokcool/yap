// The page's state, its socket, and small read-only questions about them.
// A leaf: nothing here imports from js/app/, so every part can use it while
// the others are still loading.

import { settings } from '../settings.js';

export const $ = (id) => document.getElementById(id);
export const GROUP_MS = 7 * 60_000; // consecutive messages within this window share a header
export const TYPING_MS = 6000;
export const TYPING_SEND_EVERY = 3500;
export const coarsePointer = matchMedia('(pointer: coarse)').matches;
/** Touch screens: nothing hovers, so a tap shows a message's actions (CSS uses the same query). */
export const noHover = matchMedia('(hover: none)');

export const state = {
  sid: null,
  status: 'connecting', // connecting | online | offline
  channels: new Map(),
  users: new Map(),
  sessions: new Map(),
  /** id -> custom emoji */
  emojis: new Map(),
  /** uid -> user waiting in the waiting room */
  waiting: new Map(),
  /** channelId -> unread @mentions of me */
  mentions: new Map(),
  /** channelId -> what you'd typed there and not sent (the open channel's is the message box itself) */
  drafts: new Map(),
  /** Someone wrote below while you're scrolled up in the open channel (the "↓ New messages" pill) */
  newBelow: false,
  /** A message to open once connected ({ channelId, id }): a tapped browser push (sw.js) */
  openMessage: null,
  view: 'chat', // chat | board
  config: null,
  /** channelId -> { messages, hasMore, loading, mode: 'latest' | 'older' } */
  history: new Map(),
  /** nonce -> optimistic message, shown dimmed until the server echoes it */
  pending: new Map(),
  /** Pictures being prepared and uploaded right now (they aren't in the outbox until they're up). */
  sendingPictures: 0,
  /**
   * Every `send` the server hasn't answered yet, in the order they were made
   * (chat.js › Sending): { nonce, frame, draft?, tries, out } — `out` = its
   * place in the order things went on the wire (0 = not sent yet).
   */
  outbox: [],
  /** channelId -> Map(uid -> expiresAt) */
  typing: new Map(),
  voiceChannel: null, // server-confirmed voice channel
  voiceWanted: null, // the one we're joining / in; survives reconnects
  /** The call this tab was in before the page reloaded: { channelId, at } → "You were in …" [Rejoin] (voice-ui.js). */
  rejoin: null,
  /** This session's voice resume key (ready.resumeKey). */
  resumeKey: null,
  /** Our call while the socket is down: { sid, key, at, channelId } (PROTOCOL.md › Voice › Keeping calls across reconnects). */
  resume: null,
  /** sid → timers of people in our call whose socket dropped: we keep their connection a while (shown "reconnecting…"). */
  ghosts: new Map(),
  /** uid → a leave chime held back 5 s: if they're straight back, neither chime plays. */
  leaveChimes: new Map(),
  speaking: new Set(),
  peerStates: new Map(),
  voiceStats: null,
  /** This web client's version (changelog.json › current), once loaded: the call report says it. */
  appVersion: null,
  awaitingChannel: null,
  micTest: false,
  failures: 0,
  /** This device's link is wrong or old (it connects with its device token until `ready.key` brings the new one). */
  keyWrong: false,
  /** The owner asked to reset the main link: show the new one when `key` comes back. */
  resettingMain: false,
  /** Encrypted Personal space: the server's wrap, and this device's key once unlocked. */
  vault: { wrap: null, key: null, keyCt: null },
  /** Replying to: { id, uid, channelId } */
  replying: null,
  /** Search: the last query and its results; Pins: the open list's channel. */
  search: { q: '', results: [], pending: false },
  pinsChannel: null,
  /** Pinned messages seen in the Pins list (may not be loaded in the channel). */
  pinnedLoose: new Map(),
  /** Long messages the reader expanded ("Show more"). */
  expanded: new Set(),
  /** People sheet: whose controls are open. */
  personOpen: null,
  /** encrypted body → {text, gif?, image?} | {error} | 'pending' */
  notes: new Map(),
  /** encrypted picture id → blob: URL | 'pending' | 'error' */
  sealedImages: new Map(),
  /** Soundboard clips and uploaded chimes, by id. */
  sounds: new Map(),
  /** Planned hangs, by id; `hangNow` = one starting right now. */
  hangs: new Map(),
  hangNow: null,
  /** channelId → what you'd read up to when you opened it ("New since…" divider). */
  since: new Map(),
  /** "Ada is away: …" lines under your own messages (this session only). */
  awayNotes: [],
  /** Quiet lines in a channel, this session only ("Cleo joined — invited by Ada"): { key, channelId, ts, html }. */
  localNotes: [],
  /** A word in the tab's title while it's in the background ("You're in!"). */
  titleNote: null,
};

export let conn = null;
/** start() opens the socket; everything else only reads `conn`. */
export function setConn(c) {
  conn = c;
}

export function firstTextChannel() {
  return [...state.channels.values()].filter((c) => c.kind === 'text').sort((a, b) => a.position - b.position)[0];
}

export function emojiList() {
  return [...state.emojis.values()].sort((a, b) => a.name.localeCompare(b.name));
}

export function emojisByName() {
  return new Map([...state.emojis.values()].map((e) => [e.name.toLowerCase(), e]));
}

export function findMessage(id) {
  for (const h of state.history.values()) {
    const m = h.messages.find((x) => x.id === id);
    if (m) return m;
  }
  return null;
}

export const myRole = () => state.users.get(settings.uid)?.role ?? 'member';
export const isMod = () => ['owner', 'mod'].includes(myRole());

export function memberNames() {
  return [...state.users.values()].map((u) => u.name);
}

/** Someone's away message, if it's still on. */
export const awayOf = (user) => (user?.away && (user.away.until === null || user.away.until > Date.now()) ? user.away : null);

export const MAX_BODY = 40_000;
/** The server's limit on one frame, in characters of JSON (bigger ones are refused as bad_frame). */
export const MAX_FRAME = 384 * 1024;
export const LONG_CHARS = 3000;
export const LONG_LINES = 30;
export const isPersonalChannel = (id) => state.channels.get(id)?.kind === 'personal';

export const personalId = () => `p_${settings.uid}`;
/** In your Personal space, with encryption turned on. */
export const sealedHere = () => settings.channel === personalId() && Boolean(state.vault.wrap);

export const MAX_PICTURE = 1_900_000;
export const MAX_SIDE_PX = 360;
