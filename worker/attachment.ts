// What the Hub keeps about each socket, in the socket itself (the hibernation
// API's serialized attachment), and what that state lets a session see and do.

/**
 * Per-socket state. Lives in the socket's serialized attachment (not in
 * memory) so it survives the Durable Object hibernating between messages.
 * Max 2 KB — keep it small; names/colors live in the users table instead.
 */
export interface Attachment {
  sid: string;
  hello: boolean;
  uid: string;
  client: string;
  vc: string | null;
  muted: boolean;
  deafened: boolean;
  /** Said hello but waiting to be let in (not a member yet). */
  pending?: boolean;
  view?: string | null;
  /** SHA-256 of this device's token, for sign-out. */
  tokenHash?: string;
  ip?: string;
  /** Position in the Space view. */
  sx?: number;
  sy?: number;
  sd?: number;
  /** Moved since the last saved Space position. */
  sdirty?: boolean;
  /** Personal invite this socket connected with (set by the Worker). */
  invite?: string;
  /**
   * Came with a wrong or old invite key (PROTOCOL.md › Resetting the main
   * link): until its hello, all it may send is `hello { token }`. `since` =
   * when it connected (one that never says hello is closed after 20 s).
   */
  keyless?: boolean;
  since?: number;
  /** Listen-only (an OBS link): may only join voice and listen. */
  listen?: boolean;
  /** The connection's User-Agent, cut short (for naming new devices). */
  ua?: string;
  /** Sharing their screen in their voice channel. */
  screen?: boolean;
  /** In the Watch together audience. */
  watching?: boolean;
  /**
   * Upload token: only this connection knows it (sent in its own `ready`).
   * Session ids are visible to every member, so uploads need both.
   */
  ut?: string;
  /** This session's voice resume key (`ready.resumeKey`): proves a later session is the same person's. */
  rk?: string;
  /** In voice with a client that keeps calls across reconnects (`voice.join {resume: true}`). */
  vr?: boolean;
  /** The session this voice session continues (a resumed call), until it leaves voice. */
  vp?: string;
}

/** All an OBS (listen-only) session may do. */
export const LISTEN_ALLOWED = new Set(['hello', 'voice.join', 'voice.leave', 'voice.state', 'signal']);

/** Only sessions doing something shared (voice, a shared view) are visible: no presence. */
export const visible = (a: Attachment) => Boolean(a.vc || a.view || a.watching);
