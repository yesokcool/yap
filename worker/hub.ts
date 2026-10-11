import { DurableObject } from 'cloudflare:workers';
import { BUILD } from './build';
import { LISTEN_ALLOWED, visible, type Attachment } from './attachment';
import { MEMBER_ROLES, toChannel, toSession, toUser, voiceResume, type ChannelRow, type UserRow } from './rows';
import { migrate } from './schema';
import { b64urlEncode, hex } from './encoding';
import { settings } from './settings';
import { cleanAway } from './extras';
import {
  COLOR_RE,
  CUSTOM_EMOJI_RE,
  ClientError,
  ID_RE,
  MEDIA_ID_RE,
  SID_RE,
  cleanAnchor,
  cleanName,
  isEmoji,
  isPersonal,
  personalId,
} from './validate';
import {
  CHIMES,
  LIMITS,
  PROTOCOL_VERSION,
  SPACE_EMOTES,
  type Channel,
  type ClientMessage,
  type Emoji,
  type Link,
  type Sound,
  type Role,
  type SavedDrawing,
  type ServerMessage,
  type Session,
  type User,
} from './protocol';
import { devicesOf, onHandoff, onHello, onObsCreate, onObsRevoke, onResetPin, onRevokeDevice, onSetPin, onSignOut, onSignOutOthers, onVaultSet, resetPin, vaultOf } from './hub-accounts';
import { DB_LIMIT_BYTES, addChannels, addEmoji, archiveChannels, checkInvite, invites, links, mergeUsers, modLog, onApprove, onBan, onChannelArchive, onChannelBitrate, onChannelCreate, onClaimOwner, onEmojiDelete, onEmojiRename, onInviteCreate, onInviteRevoke, onLinkAdd, onLinkRemove, onRole, onStats, setRole, stats, unarchiveChannels, unmergeUser, type AddEmojiResult } from './hub-admin';
import { exportPage, importBackup } from './hub-backup';
import { addDrawing, gallery, onDrawClear, onDrawSeg, onDrawState, onDrawStroke, onDrawUndo, onDrawWipe } from './hub-draw';
import { addSound, hangs, onHangCancel, onHangSet, onSoundDelete, onSoundPlay, onWhenGet, onWhenSet, sounds, startDueHangs } from './hub-extras';
import { onDelete, onEdit, onHistory, onPersonalPlain, onPersonalSeal, onPin, onPins, onPollVote, onReact, onSearch, onSend, onShelf, onUnreadCounts } from './hub-messages';
import { notifySettings, onNotifySettings, onPing, onPushSubscribe, onQuiet } from './hub-pushes';
import { onSpaceMove, onSpaceState, saveSpot } from './hub-space';
import { onWatchNext, onWatchQueue, onWatchSet, setWatching, watchState } from './hub-watch';

const VIEWS = new Set(['board', 'draw', 'space']);

export class Hub extends DurableObject<Env> {
  // The feature modules (hub-*.ts) reach these through HubHost (hub-host.ts).
  declare readonly env: Env;
  declare readonly ctx: DurableObjectState<{}>;
  sql: SqlStorage;
  /** Last message timestamp handed out; message ts is strictly increasing. */
  lastTs = 0;
  /** alias uid -> canonical uid. Cache of user_aliases; null = reload. */
  aliases: Map<string, string> | null = null;
  /**
   * Token buckets for rate limits, keyed `${who}:${kind}`, in memory only.
   * `who` is a session id (dropped when that session closes, in drop()), or a
   * uid, `ip:<address>`, `vc:<channel>` or `<uid>>` + `<uid>` (away replies): those
   * stay until the object hibernates or restarts, a handful at a friend group's size.
   */
  private buckets = new Map<string, { at: number; n: number }>();
  /**
   * PIN_PEPPER must be set: hashing without it would silently create hashes
   * that stop verifying once it is set. Fail closed instead.
   */
  get pepper(): string {
    if (!this.env.PIN_PEPPER) {
      console.error('PIN_PEPPER secret is missing — refusing to hash/verify PINs');
      throw new ClientError('unavailable', 'Accounts are temporarily unavailable. Tell the server owner (PIN_PEPPER missing).');
    }
    return this.env.PIN_PEPPER;
  }
  /** Last time a "someone wants to join" push went out (one per 2 min at most). */
  lastJoinPush = 0;
  /** Last "while you were away" push per user, so busy chats don't buzz a phone nonstop. */
  lastPush = new Map<string, number>();
  /** The address people last connected on (cached; kept in meta as 'origin'). */
  private origin = '';
  /** HMAC key for voice resume keys (its secret lives in meta, so keys survive restarts and deploys). */
  private resumeHmac: Promise<CryptoKey> | null = null;

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    // Heartbeats are answered by the runtime without waking the object.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping', 'pong'));
    ctx.blockConcurrencyWhile(async () => {
      migrate(this.sql);
      this.announceBuild();
      // After the migrations: a brand-new database has no media_trash before them.
      ctx.waitUntil(this.emptyTrash());
    });
  }

  /**
   * Erase pictures of deleted posts from the Media store, unless something
   * still uses the same bytes (ids are content hashes: the same picture posted
   * twice, an avatar, an emoji, a drawing).
   */
  async emptyTrash() {
    const ids = this.sql.exec<{ id: string }>('SELECT id FROM media_trash LIMIT 200').toArray().map((r) => r.id);
    if (!ids.length) return;
    const media = this.env.MEDIA.get(this.env.MEDIA.idFromName('main'));
    for (const id of ids) {
      const used = this.sql.exec<{ n: number }>(
        `SELECT (SELECT COUNT(*) FROM messages WHERE deleted_at IS NULL AND json_extract(image, '$.id') = ?1)
              + (SELECT COUNT(*) FROM users WHERE avatar = ?1)
              + (SELECT COUNT(*) FROM emojis WHERE id = ?1)
              + (SELECT COUNT(*) FROM drawings WHERE media_id = ?1)
              + (SELECT COUNT(*) FROM sounds WHERE media_id = ?1)
              + (SELECT COUNT(*) FROM reactions WHERE emoji = '<d:' || ?1 || '>') AS n`, id,
      ).one().n;
      try {
        if (!used && id && MEDIA_ID_RE.test(id)) await media.remove(id);
        this.sql.exec('DELETE FROM media_trash WHERE id = ?', id);
      } catch {} // try again next time
    }
  }

  /**
   * First start of a new deploy: tell any page still connected which web
   * build is live now (PROTOCOL.md › Updates). Deploys usually drop every
   * socket and `ready` carries the build; this covers sockets that survive.
   */
  private announceBuild() {
    const stamp = `${BUILD.app}:${BUILD.style}`;
    const row = this.sql.exec<{ value: string }>("SELECT value FROM meta WHERE key = 'build'").toArray()[0];
    if (row?.value === stamp) return;
    this.sql.exec("INSERT INTO meta (key, value) VALUES ('build', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", stamp);
    if (row) for (const ws of this.ctx.getWebSockets()) this.send(ws, { t: 'build', build: BUILD });
  }

  async fetch(request: Request): Promise<Response> {
    const { 0: client, 1: server } = new WebSocketPair();
    this.ctx.acceptWebSocket(server);
    const attachment: Attachment = {
      sid: crypto.randomUUID(),
      hello: false,
      uid: '',
      client: '',
      vc: null,
      muted: false,
      deafened: false,
      ip: request.headers.get('CF-Connecting-IP') ?? '',
      ua: (request.headers.get('User-Agent') ?? '').slice(0, 300),
    };
    // Set by the Worker (after it checked the key), never by the client.
    const invite = request.headers.get('X-Yap-Invite');
    if (invite && ID_RE.test(invite)) attachment.invite = invite;
    server.serializeAttachment(attachment);
    this.rememberOrigin(request.url);
    // Clients may pass the invite key as a subprotocol ("yap", "key.<k>") to
    // keep it out of URLs; we must echo the protocol we accept.
    const offered = (request.headers.get('Sec-WebSocket-Protocol') ?? '').split(',').map((p) => p.trim());
    const headers = offered.includes('yap') ? { 'Sec-WebSocket-Protocol': 'yap' } : undefined;
    return new Response(null, { status: 101, webSocket: client, headers });
  }

  // ── WebSocket lifecycle ────────────────────────────────────────────────

  async webSocketMessage(ws: WebSocket, raw: string | ArrayBuffer) {
    if (typeof raw !== 'string' || raw.length > LIMITS.frameBytes) {
      return this.send(ws, { t: 'error', code: 'bad_frame', message: `That's too much to send at once (the limit is ${LIMITS.frameBytes / 1024} KB).` });
    }
    let msg: ClientMessage;
    try {
      msg = JSON.parse(raw);
    } catch {
      return this.send(ws, { t: 'error', code: 'bad_json', message: 'Frame is not valid JSON.' });
    }
    // "null", "42", "[]" parse too; only an object can say what it is (and null would throw below).
    if (typeof msg !== 'object' || msg === null || Array.isArray(msg)) {
      return this.send(ws, { t: 'error', code: 'bad_json', message: 'Each frame is a JSON object with a "t".' });
    }
    const me = ws.deserializeAttachment() as Attachment;
    try {
      if (me.pending) throw new ClientError('pending', 'Waiting for someone to let you in.');
      if (me.listen && !LISTEN_ALLOWED.has(msg.t)) throw new ClientError('forbidden', 'This is a listen-only (OBS) link.');
      if (msg.t === 'hello') return await onHello(this, ws, me, msg);
      if (!me.hello) throw new ClientError('no_hello', 'Send hello first.');
      switch (msg.t) {
        case 'history': return onHistory(this, ws, me, msg);
        case 'send': return onSend(this, ws, me, msg);
        case 'delete': return onDelete(this, me, msg);
        case 'when.get': return onWhenGet(this, ws);
        case 'search':
          this.limit(me.sid, 'search', 6, 0.5);
          return onSearch(this, ws, me, msg);
        case 'pin':
          this.limit(me.sid, 'pin', 10, 0.5);
          return onPin(this, me, msg);
        case 'pins': return onPins(this, ws, me, msg);
        case 'poll.vote':
          this.limit(me.sid, 'vote', 10, 1);
          return onPollVote(this, me, msg);
        case 'sound.play': return onSoundPlay(this, ws, me, msg);
        case 'sound.delete': return onSoundDelete(this, me, msg);
        case 'shelf':
          this.limit(me.sid, 'shelf', 6, 0.5);
          return onShelf(this, ws, me, msg);
        case 'unread.counts':
          this.limit(me.sid, 'counts', 4, 0.2);
          return onUnreadCounts(this, ws, me, msg);
        case 'notify.quiet': return onQuiet(this, ws, me, msg);
        case 'hang.set':
          this.limit(me.uid, 'hang', 5, 0.05);
          return onHangSet(this, me, msg);
        case 'hang.cancel': return onHangCancel(this, me, msg);
        case 'space.game': {
          if (!this.allow(me.sid, 'game', 30, 15)) return;
          const g = String(msg.g ?? '');
          const d = JSON.stringify(msg.d ?? null);
          if (!/^[a-z]{1,16}$/.test(g) || d.length > 512) throw new ClientError('bad_game', 'Game updates are small.');
          return this.relayToView('space', ws, { t: 'space.game', sid: me.sid, g, d: JSON.parse(d) });
        }
        case 'personal.plain': return onPersonalPlain(this, ws, me);
        case 'personal.seal':
          this.limit(me.uid, 'seal', 40, 2);
          return onPersonalSeal(this, me, msg);
        case 'when.set':
          this.limit(me.uid, 'when', 20, 1);
          return onWhenSet(this, me, msg);
        case 'edit':
          this.limit(me.sid, 'edit', 10, 0.5);
          return onEdit(this, me, msg);
        case 'invite.create':
          this.limit(me.uid, 'invite', 5, 0.05);
          return await onInviteCreate(this, ws, me, msg);
        case 'invite.list': return this.send(ws, { t: 'invites', invites: invites(this, me.uid) });
        case 'invite.revoke': return onInviteRevoke(this, ws, me, msg);
        case 'link.add':
          this.limit(me.sid, 'link', 5, 0.1);
          return onLinkAdd(this, me, msg);
        case 'link.remove': return onLinkRemove(this, me, msg);
        case 'channel.bitrate':
          this.limit(me.sid, 'bitrate', 5, 0.5);
          return onChannelBitrate(this, me, msg);
        case 'modlog': return this.send(ws, { t: 'modlog', entries: modLog(this) });
        case 'obs.create':
          this.limit(me.uid, 'obs', 5, 0.05);
          return await onObsCreate(this, ws, me);
        case 'obs.revoke': return onObsRevoke(this, ws, me);
        case 'vault.set':
          this.limit(me.uid, 'vault', 5, 0.05);
          return onVaultSet(this, me, msg);
        case 'device.handoff':
          this.limit(me.uid, 'handoff', 5, 0.05);
          return await onHandoff(this, ws, me);
        case 'push.subscribe':
          this.limit(me.sid, 'pushsub', 5, 0.1);
          return onPushSubscribe(this, ws, me, msg);
        case 'push.unsubscribe':
          this.sql.exec('DELETE FROM push_subs WHERE endpoint = ? AND user_id = ?', String(msg.endpoint), me.uid);
          return;
        case 'stats': return await onStats(this, ws, me);
        case 'typing':
          if (!this.allow(me.sid, 'typing', 3, 0.5)) return;
          return this.onTyping(ws, me, msg);
        case 'profile': return this.onProfile(me, msg);
        case 'channel.create':
          this.limit(me.sid, 'create', 5, 0.1);
          return onChannelCreate(this, me.uid, msg);
        case 'channel.archive': return onChannelArchive(this, me.uid, msg);
        case 'voice.join': return await this.onVoiceJoin(ws, me, msg);
        case 'voice.leave': this.setVoice(ws, me, null, me.muted, me.deafened); return;
        case 'voice.state':
          // Only inside voice: outside, it would announce a session nobody can otherwise see (no presence).
          if (me.vc) this.setVoice(ws, me, me.vc, !!msg.muted, !!msg.deafened, !!msg.screen && !me.listen);
          return;
        case 'watch.join': return setWatching(this, ws, me, true);
        case 'watch.leave': return setWatching(this, ws, me, false);
        case 'watch.get': return this.send(ws, { t: 'watch.state', state: watchState(this), now: Date.now() });
        case 'watch.set':
          this.limit(me.sid, 'watch', 20, 2);
          return await onWatchSet(this, me, msg);
        case 'watch.queue':
          this.limit(me.sid, 'watch', 20, 2);
          return await onWatchQueue(this, me, msg);
        case 'watch.next': return onWatchNext(this, me, msg);
        case 'emoji.delete': return onEmojiDelete(this, me.uid, msg);
        case 'emoji.rename':
          this.limit(me.sid, 'rename', 10, 0.2);
          return onEmojiRename(this, msg);
        case 'member.approve': return onApprove(this, me, msg.uid, true);
        case 'member.deny': return onApprove(this, me, msg.uid, false);
        case 'member.ban': return onBan(this, me, msg);
        case 'member.role': return onRole(this, me, msg);
        case 'owner.claim': return onClaimOwner(this, me, msg);
        case 'account.pin': return await onSetPin(this, ws, me, msg);
        case 'account.signout': return onSignOut(this, ws, me);
        case 'account.devices': return this.send(ws, { t: 'devices', devices: devicesOf(this, me) });
        case 'account.revoke': return onRevokeDevice(this, ws, me, String(msg.id ?? ''));
        case 'account.signoutOthers': return onSignOutOthers(this, ws, me);
        case 'member.resetPin': return await onResetPin(this, ws, me, msg);
        case 'view':
          if (!this.allow(me.sid, 'view', 10, 1)) return;
          return this.onView(ws, me, msg);
        case 'cursor':
          if (!this.allow(me.sid, 'cursor', 40, 25)) return;
          return this.onCursor(ws, me, msg);
        case 'notify.settings': return onNotifySettings(this, ws, me, msg);
        case 'ping':
          this.limit(me.uid, 'ping', 2, 1 / 30);
          return onPing(this, me, msg);
        case 'draw.state': return onDrawState(this, ws);
        case 'draw.gallery': return this.send(ws, { t: 'draw.gallery', gallery: gallery(this) });
        case 'draw.seg':
          if (!this.allow(me.sid, 'seg', 80, 40)) return;
          return onDrawSeg(this, ws, me, msg);
        case 'draw.stroke':
          this.limit(me.sid, 'stroke', 40, 8);
          return onDrawStroke(this, ws, me, msg);
        case 'draw.undo': return onDrawUndo(this, me, msg);
        case 'draw.clear':
          this.limit(me.sid, 'clear', 3, 1 / 20);
          return onDrawClear(this, me);
        case 'draw.wipe':
          this.limit(me.sid, 'wipe', 5, 1 / 10);
          return onDrawWipe(this, me, msg);
        case 'space.state': return onSpaceState(this, ws);
        case 'react':
          this.limit(me.sid, 'react', 20, 4);
          return onReact(this, me, msg);
        case 'space.emote':
          if (!this.allow(me.sid, 'emote', 4, 1)) return;
          if (!SPACE_EMOTES.includes(msg.e)) throw new ClientError('bad_emoji', 'Pick one of the emotes.');
          return this.relayToView('space', null, { t: 'space.emote', sid: me.sid, e: msg.e });
        case 'space.move':
          if (!this.allow(me.sid, 'move', 40, 20)) return;
          return onSpaceMove(this, ws, me, msg);
        case 'signal':
          // Joining a call is a burst (an offer or answer plus every ICE candidate, to each
          // of up to ~7 peers), so the allowance is generous; past it, quietly dropped.
          if (!this.allow(me.sid, 'signal', 200, 20)) return;
          return this.onSignal(me, msg);
        default: throw new ClientError('unknown_type', `Unknown message type "${(msg as { t: string }).t}".`);
      }
    } catch (err) {
      if (err instanceof ClientError) {
        this.send(ws, { t: 'error', code: err.code, message: err.message, re: msg.t });
      } else {
        console.error('handler failed', msg.t, err);
        this.send(ws, { t: 'error', code: 'internal', message: 'Server error.', re: msg.t });
      }
    }
  }

  async webSocketClose(ws: WebSocket, code: number) {
    this.drop(ws);
    try {
      ws.close(code === 1005 || code === 1006 ? 1000 : code, 'bye');
    } catch {
      // Already closed (newer runtimes auto-reply to close frames).
    }
  }

  async webSocketError(ws: WebSocket) {
    this.drop(ws);
  }

  private drop(ws: WebSocket) {
    const me = ws.deserializeAttachment() as Attachment;
    for (const key of this.buckets.keys()) if (key.startsWith(`${me.sid}:`)) this.buckets.delete(key);
    if (!me.hello) return;
    // Mark it gone first so broadcasts during teardown skip this socket.
    ws.serializeAttachment({ ...me, hello: false });
    // `lost`: the socket went without a voice.leave, so the call may come back (PROTOCOL.md › Voice › Keeping calls across reconnects).
    if (me.vc) this.broadcast({ t: 'voice', sid: me.sid, channelId: null, muted: me.muted, deafened: me.deafened, lost: true });
    if (me.sdirty) saveSpot(this, me.uid, me.sx!, me.sy ?? 0, me.sd ?? 0);
    if (visible(me)) this.broadcast({ t: 'session.leave', sid: me.sid });
    this.sql.exec('UPDATE users SET last_seen = ? WHERE id = ?', Date.now(), me.uid);
  }

  /** Full welcome: `ready` to this socket, join announced to everyone else. */
  admit(ws: WebSocket, me: Attachment, token?: string) {
    const ut = me.listen ? undefined : hex(crypto.getRandomValues(new Uint8Array(16)));
    const { hasPin, ...secret } = this.secretInfo(me.uid);
    const updated: Attachment = { ...me, hello: true, pending: false, view: null, ut };
    ws.serializeAttachment(updated);
    this.send(ws, {
      t: 'ready',
      v: PROTOCOL_VERSION,
      sid: me.sid,
      uid: me.uid,
      token,
      needsPin: !hasPin,
      notify: notifySettings(this, me.uid),
      channels: me.listen ? this.channels() : [this.personalChannel(me.uid), ...this.channels()],
      users: this.users(),
      sessions: [...this.sessions().filter((s) => s.voice || s.view || s.watching), toSession(updated)],
      emojis: this.emojis(),
      pending: this.pendingUsers(),
      build: BUILD,
      ...(this.env.YAP_KEY && !me.listen ? { key: this.env.YAP_KEY } : {}),
      links: links(this),
      sounds: sounds(this),
      hangs: hangs(this),
      ...(this.isMod(me.uid) && this.ctx.storage.sql.databaseSize > DB_LIMIT_BYTES * 0.8 ? { storageWarning: true } : {}),
      ...(ut ? { uploadToken: ut } : {}),
      ...(me.rk ? { resumeKey: me.rk } : {}),
      ...(me.listen ? {} : { vault: vaultOf(this, me.uid), ...secret }),
    });
    // No presence: connecting tells nobody (sessions appear when they join
    // voice or a shared view). A brand-new member is still announced.
    if (this.isNewMember(me.uid)) this.broadcast({ t: 'user', user: this.user(me.uid)! }, ws);
  }

  /** Whether you have a PIN, whether it's guessable (known from your last sign-in) and PIN vs password. One read. */
  private secretInfo(uid: string): { hasPin: boolean; weakPin?: boolean; secretKind?: 'pin' | 'password' } {
    const row = this.sql.exec<{ weak_secret: number | null; secret_kind: string | null; pin_hash: string | null }>('SELECT weak_secret, secret_kind, pin_hash FROM users WHERE id = ?', uid).toArray()[0];
    if (!row?.pin_hash) return { hasPin: false };
    return { hasPin: true, ...(row.weak_secret ? { weakPin: true } : {}), secretKind: row.secret_kind === 'password' ? 'password' : 'pin' };
  }

  private isNewMember(uid: string): boolean {
    const row = this.sql.exec<{ created_at: number }>('SELECT created_at FROM users WHERE id = ?', uid).toArray()[0];
    return !!row && Date.now() - row.created_at < 7 * 86_400_000 && !this.sql.exec('SELECT 1 FROM messages WHERE user_id = ? LIMIT 1', uid).toArray().length;
  }

  /** A session going from hidden to visible (or back) is announced/withdrawn around the change. */
  showSession(ws: WebSocket, before: Attachment, after: Attachment) {
    if (!visible(before) && visible(after)) {
      this.broadcast({ t: 'session.join', session: toSession({ ...after, vc: null, view: null }) }, ws);
    }
  }

  hideSession(ws: WebSocket, before: Attachment, after: Attachment) {
    if (visible(before) && !visible(after)) this.broadcast({ t: 'session.leave', sid: after.sid }, ws);
  }

  // ── Voice ──────────────────────────────────────────────────────────────

  private async onVoiceJoin(ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'voice.join' }>) {
    if (me.listen) msg = { ...msg, muted: true, deafened: false };
    const channel = this.channel(msg.channelId);
    if (!channel || channel.kind !== 'voice') throw new ClientError('no_channel', 'That voice channel does not exist.');
    // Keeping a call across a reconnect: `prev` is the session this one
    // replaces, `proof` that session's resume key. The key is an HMAC of
    // (sid, uid), so it proves `prev` was *this account's* session — even
    // after a deploy, when the server remembers nothing about it.
    let prev: string | undefined;
    if (typeof msg.prev === 'string' && typeof msg.proof === 'string' && SID_RE.test(msg.prev) && msg.prev !== me.sid) {
      const ok = await this.checkResumeKey(msg.prev, me.uid, msg.proof);
      me = ws.deserializeAttachment() as Attachment; // anything can have happened during the await
      if (!me.hello) return;
      if (ok) prev = msg.prev;
    }
    if (me.vc === channel.id) return;
    // Switching channels is leave + join, so peers in the old channel hang up.
    if (me.vc) me = this.setVoice(ws, me, null, me.muted, me.deafened);
    me = { ...me, vr: msg.resume === true || undefined, vp: prev };
    this.setVoice(ws, me, channel.id, !!msg.muted, !!msg.deafened, !!msg.screen && !me.listen);
    // The old socket may still be open here (a half-dead connection the client
    // already gave up on). It goes now, *after* the resume was announced, so
    // clients that resumed have nothing left to hang up.
    if (prev) {
      const old = this.sockets().find((s) => (s.deserializeAttachment() as Attachment).sid === prev);
      if (old) {
        this.drop(old);
        try {
          old.close(4000, 'resumed elsewhere');
        } catch {}
      }
    }
  }

  setVoice(ws: WebSocket, me: Attachment, vc: string | null, muted: boolean, deafened: boolean, screen = false): Attachment {
    // Screen sharing only makes sense inside a voice channel.
    const sharing = Boolean(vc && screen);
    const updated: Attachment = { ...me, vc, muted, deafened, screen: sharing, ...(vc ? {} : { vr: undefined, vp: undefined }) };
    ws.serializeAttachment(updated);
    this.showSession(ws, me, updated);
    if (me.vc !== vc || me.muted !== muted || me.deafened !== deafened || Boolean(me.screen) !== sharing) {
      this.broadcast({ t: 'voice', sid: me.sid, channelId: vc, muted, deafened, ...(sharing ? { screen: true } : {}), ...voiceResume(updated) });
    }
    this.hideSession(ws, me, updated);
    return updated;
  }

  /** The secret behind resume keys: random, made once, kept in meta (never exported). */
  private resumeCrypto(): Promise<CryptoKey> {
    if (!this.resumeHmac) {
      const read = () => this.sql.exec<{ value: string }>("SELECT value FROM meta WHERE key = 'resume_secret'").toArray()[0]?.value;
      if (!read()) this.sql.exec("INSERT OR IGNORE INTO meta (key, value) VALUES ('resume_secret', ?)", b64urlEncode(crypto.getRandomValues(new Uint8Array(32))));
      const secret = Uint8Array.from(atob(read()!.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
      this.resumeHmac = crypto.subtle.importKey('raw', secret, { name: 'HMAC', hash: 'SHA-256' }, false, ['sign', 'verify']);
    }
    return this.resumeHmac;
  }

  /** `ready.resumeKey` for a session: HMAC-SHA256(secret, "voice-resume:<sid>:<uid>"), base64url. */
  async resumeKey(sid: string, uid: string): Promise<string> {
    const mac = await crypto.subtle.sign('HMAC', await this.resumeCrypto(), new TextEncoder().encode(`voice-resume:${sid}:${uid}`));
    return b64urlEncode(new Uint8Array(mac));
  }

  private async checkResumeKey(sid: string, uid: string, proof: string): Promise<boolean> {
    if (!/^[A-Za-z0-9_-]{43}$/.test(proof)) return false;
    const mac = Uint8Array.from(atob(proof.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
    return crypto.subtle.verify('HMAC', await this.resumeCrypto(), mac, new TextEncoder().encode(`voice-resume:${sid}:${uid}`));
  }

  /** WebRTC set-up between two people in the same voice channel, or with someone sharing their screen. */
  private onSignal(me: Attachment, msg: Extract<ClientMessage, { t: 'signal' }>) {
    const target = this.sockets().find((ws) => (ws.deserializeAttachment() as Attachment).sid === msg.to);
    const them = target?.deserializeAttachment() as Attachment | undefined;
    if (!target || !them || !((me.vc && them.vc === me.vc) || them.screen || me.screen)) throw new ClientError('no_peer', 'That session is gone.');
    this.send(target, { t: 'signal', from: me.sid, data: msg.data });
  }

  // ── Shared views (the board) ───────────────────────────────────────────

  private onView(ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'view' }>) {
    const view = msg.view === null ? null : VIEWS.has(String(msg.view)) ? String(msg.view) : null;
    if ((me.view ?? null) === view) return;
    // Leaving Space: remember exactly where you stood (if it changed).
    if (me.view === 'space' && me.sdirty) saveSpot(this, me.uid, me.sx!, me.sy ?? 0, me.sd ?? 0);
    const updated: Attachment = { ...me, view, sdirty: me.view === 'space' ? false : me.sdirty };
    ws.serializeAttachment(updated);
    this.showSession(ws, me, updated);
    this.broadcast({ t: 'view', sid: me.sid, view });
    this.hideSession(ws, me, updated);
  }

  /** Pointer positions, relayed only to others on the same view. Never stored. */
  private onCursor(ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'cursor' }>) {
    const view = String(msg.view);
    if (!VIEWS.has(view)) return;
    this.relayToView(view, ws, { t: 'cursor', sid: me.sid, view, a: cleanAnchor(msg.a) });
  }

  // ── Typing and profiles ────────────────────────────────────────────────

  private onTyping(ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'typing' }>) {
    if (typeof msg.channelId !== 'string' || isPersonal(msg.channelId)) return;
    this.broadcast({ t: 'typing', channelId: msg.channelId, uid: me.uid }, ws);
  }

  private onProfile(me: Attachment, msg: Extract<ClientMessage, { t: 'profile' }>) {
    const current = this.user(me.uid)!;
    const name = msg.name === undefined ? current.name : cleanName(msg.name, LIMITS.nameChars);
    if (!name) throw new ClientError('bad_name', 'Pick a name.');
    if (this.nameTaken(name, me.uid)) throw new ClientError('name_taken', `Someone here is already called ${name}.`);
    const color = msg.color !== undefined && COLOR_RE.test(msg.color) ? msg.color.toLowerCase() : current.color;
    const avatar = msg.avatar === null ? null : current.avatar; // set via /api/avatar; null removes
    let look = current.look;
    if (msg.look === null) look = null;
    else if (msg.look !== undefined) {
      if (!isEmoji(msg.look) || CUSTOM_EMOJI_RE.test(msg.look)) throw new ClientError('bad_emoji', 'Pick an emoji for your character.');
      look = msg.look;
    }
    // Every field is checked before anything is saved: one bad field changes nothing.
    const sets = ['name = ?', 'color = ?', 'avatar = ?', 'look = ?'];
    const values: SqlStorageValue[] = [name, color, avatar, look];
    if (msg.away !== undefined) {
      sets.push('away = ?');
      values.push(msg.away === null ? null : JSON.stringify(cleanAway(msg.away)));
    }
    if (msg.chime !== undefined) {
      sets.push('chime = ?');
      values.push(msg.chime === null ? null : this.cleanChime(me.uid, msg.chime));
    }
    this.sql.exec(`UPDATE users SET ${sets.join(', ')} WHERE id = ?`, ...values, me.uid);
    this.broadcast({ t: 'user', user: this.user(me.uid)! });
  }

  /** A built-in chime id, or `s:<id>` of a chime you uploaded. */
  private cleanChime(uid: string, value: unknown): string {
    if (typeof value === 'string' && (CHIMES as readonly string[]).includes(value)) return value;
    const id = typeof value === 'string' && value.startsWith('s:') ? value.slice(2) : '';
    const sound = this.sql.exec<{ user_id: string }>("SELECT user_id FROM sounds WHERE id = ? AND kind = 'chime' AND deleted_at IS NULL", id).toArray()[0];
    if (!sound || this.canonical(sound.user_id) !== uid) throw new ClientError('bad_chime', 'Pick one of the chimes, or upload your own.');
    return `s:${id}`;
  }

  /** Called by the Worker after an avatar image was stored in Media. */
  async setAvatar(input: { sid: string; ut: string; media: string }): Promise<{ ok: boolean; message?: string }> {
    const ws = this.uploader(input.sid, input.ut);
    if (!ws) return { ok: false, message: 'Connect first.' };
    const me = ws.deserializeAttachment() as Attachment;
    if (!this.allow(me.sid, 'avatar', 5, 1 / 30)) return { ok: false, message: 'Changing too fast.' };
    this.sql.exec('UPDATE users SET avatar = ? WHERE id = ?', input.media, me.uid);
    this.broadcast({ t: 'user', user: this.user(me.uid)! });
    return { ok: true };
  }

  // ── Uploads (the Worker asks before it stores anything) ────────────────

  /** The live session behind an upload: its id AND its private upload token must match. */
  uploader(sid: unknown, ut: unknown): WebSocket | undefined {
    if (typeof sid !== 'string' || typeof ut !== 'string' || ut.length !== 32) return undefined;
    return this.sockets().find((s) => {
      const a = s.deserializeAttachment() as Attachment;
      return a.sid === sid && a.ut === ut;
    });
  }

  /** Worker RPC: is this media id a custom emoji (live or deleted)? Those are cached for a year. */
  async isEmoji(id: string): Promise<boolean> {
    return this.sql.exec('SELECT 1 FROM emojis WHERE id = ?', id).toArray().length > 0;
  }

  /**
   * Worker RPC, before it stores an emoji: is this a signed-in member's
   * session? Spends nothing (a picker uploads many at once; addEmoji has its
   * own limit).
   */
  async isUploader(sid: string, ut: string): Promise<boolean> {
    return Boolean(this.uploader(sid, ut));
  }

  /** Worker RPC, before it stores any upload: is this a signed-in member's session, under the upload rate? */
  async canUpload(sid: string, ut: string): Promise<boolean> {
    if (!this.uploader(sid, ut)) return false;
    return this.allow(sid, 'upload', 10, 1 / 6);
  }

  // ── RPC (called by the Worker, not over the socket) ────────────────────

  // Worker RPC that lives in the feature modules (hub-*.ts): one line each,
  // so the Worker's stub keeps calling these on Hub.
  async resetPin(uid: string) {
    return resetPin(this, uid);
  }

  async addDrawing(input: { sid: string; ut: string; media: string }): Promise<{ ok: boolean; drawing?: SavedDrawing; message?: string }> {
    return addDrawing(this, input);
  }

  async addSound(input: { sid: string; ut: string; kind: string; name: string; emoji: string; media: string; gain: number; ms: number }): Promise<{ ok: boolean; sound?: Sound; message?: string }> {
    return addSound(this, input);
  }

  /** The Durable Object alarm: hangs starting now (hub-extras.ts). */
  async alarm() {
    return startDueHangs(this);
  }

  async addChannels(names: string[], kind: 'text' | 'voice') {
    return addChannels(this, names, kind);
  }

  async archiveChannels(names: string[], kind: 'text' | 'voice') {
    return archiveChannels(this, names, kind);
  }

  async unarchiveChannels(names: string[], kind: 'text' | 'voice') {
    return unarchiveChannels(this, names, kind);
  }

  async checkInvite(key: string): Promise<string | null> {
    return checkInvite(this, key);
  }

  links(): Link[] {
    return links(this);
  }

  async addEmoji(input: { id: string; name: string; mime: string; sid?: string; ut?: string; asAdmin?: boolean }): Promise<AddEmojiResult> {
    return addEmoji(this, input);
  }

  async exportPage(afterTs: number, limit: number) {
    return exportPage(this, afterTs, limit);
  }

  async importBackup(part: { phase: string; data?: Record<string, unknown>; messages?: Record<string, unknown>[] }): Promise<{ ok: boolean; message?: string; count?: number }> {
    return importBackup(this, part);
  }

  async mergeUsers(from: string, into: string) {
    return mergeUsers(this, from, into);
  }

  async setRole(uid: string, role: Role) {
    return setRole(this, uid, role);
  }

  async unmergeUser(alias: string) {
    return unmergeUser(this, alias);
  }

  async stats() {
    return stats(this);
  }

  /** Everyone reconnects and gets a fresh `ready` (cheap; clients handle it). */
  reconnectEveryone() {
    for (const ws of this.ctx.getWebSockets()) {
      try {
        ws.close(1012, 'refresh');
      } catch {}
    }
  }

  /**
   * Point-in-time recovery, step 1: schedule a restore of this database to a
   * moment in the last 30 days (or to a bookmark, e.g. an earlier `undo`).
   * Returns the bookmark that undoes it. Step 2 is `restart()`.
   */
  async prepareRestore(to: { time?: number; bookmark?: string }): Promise<{ bookmark: string; undo: string }> {
    const bookmark = to.bookmark ?? (await this.ctx.storage.getBookmarkForTime(to.time ?? Date.now()));
    const undo = await this.ctx.storage.onNextSessionRestoreBookmark(bookmark);
    return { bookmark, undo };
  }

  /** Restart the object (applies a scheduled restore; clients reconnect). */
  async restart(): Promise<void> {
    this.ctx.abort('restoring from backup');
  }

  // ── Rate limits ────────────────────────────────────────────────────────

  /** Token bucket: `cap` burst, refilled at `perSec`. */
  allow(sid: string, kind: string, cap: number, perSec: number): boolean {
    const key = `${sid}:${kind}`;
    const now = Date.now();
    const b = this.buckets.get(key) ?? { at: now, n: cap };
    b.n = Math.min(cap, b.n + ((now - b.at) / 1000) * perSec);
    b.at = now;
    const ok = b.n >= 1;
    if (ok) b.n -= 1;
    this.buckets.set(key, b);
    return ok;
  }

  /** A bucket as it was last left (allow() refills it on use), without spending from it. */
  peek(who: string, kind: string): { at: number; n: number } | undefined {
    return this.buckets.get(`${who}:${kind}`);
  }

  limit(sid: string, kind: string, cap: number, perSec: number) {
    if (!this.allow(sid, kind, cap, perSec)) throw new ClientError('slow_down', 'Slow down a little.');
  }

  /**
   * Sign a connection out now (banned, signed out, a device revoked): it
   * leaves voice and any shared view for everyone first, then closes.
   */
  kick(ws: WebSocket, code: number, reason: string) {
    this.drop(ws);
    this.closeQuietly(ws, code, reason);
  }

  closeQuietly(ws: WebSocket, code: number, reason: string) {
    try {
      ws.close(code, reason);
    } catch {}
  }

  // ── Queries ────────────────────────────────────────────────────────────

  private channels(): Channel[] {
    return this.sql
      .exec<ChannelRow>(
        `SELECT c.id, c.name, c.kind, c.position, c.bitrate,
                (SELECT MAX(ts) FROM messages m WHERE m.channel_id = c.id AND m.deleted_at IS NULL) AS last_ts
         FROM channels c WHERE c.archived_at IS NULL ORDER BY c.kind, c.position`,
      )
      .toArray()
      .map(toChannel);
  }

  channel(id: unknown): Channel | undefined {
    if (typeof id !== 'string') return undefined;
    const row = this.sql
      .exec<ChannelRow>(
        `SELECT c.id, c.name, c.kind, c.position, c.bitrate,
                (SELECT MAX(ts) FROM messages m WHERE m.channel_id = c.id AND m.deleted_at IS NULL) AS last_ts
         FROM channels c WHERE c.id = ? AND c.archived_at IS NULL`,
        id,
      )
      .toArray()[0];
    return row && toChannel(row);
  }

  private emojis(): Emoji[] {
    return this.sql
      .exec<{ id: string; name: string; mime: string; user_id: string; created_at: number }>(
        'SELECT id, name, mime, user_id, created_at FROM emojis WHERE deleted_at IS NULL ORDER BY created_at',
      )
      .toArray()
      .map((r) => ({ id: r.id, name: r.name, mime: r.mime, uid: r.user_id === 'admin' ? 'admin' : this.canonical(r.user_id), createdAt: r.created_at }));
  }

  users(): User[] {
    return this.sql
      .exec<UserRow>(
        `SELECT id, name, color, last_seen, role, avatar, look, away, chime FROM users
         WHERE role IN ${MEMBER_ROLES} AND id NOT IN (SELECT alias FROM user_aliases)
         ORDER BY last_seen DESC LIMIT 500`,
      )
      .toArray()
      .map(toUser);
  }

  pendingUsers(): User[] {
    const rows = this.sql
      .exec<UserRow & { invite_id: string | null; invite_label: string | null; inviter: string | null }>(
        `SELECT u.id, u.name, u.color, u.last_seen, u.role, u.avatar, u.look, u.invite_id, i.label AS invite_label, c.name AS inviter
         FROM users u LEFT JOIN invites i ON i.id = u.invite_id LEFT JOIN users c ON c.id = i.created_by
         WHERE u.role = 'pending' ORDER BY u.created_at DESC LIMIT 50`,
      )
      .toArray();
    // Personal links can be passed on: say who else came in with the same one
    // (any role, oldest first), so whoever lets them in can check with the inviter.
    const sameLink = new Map<string, { id: string; name: string }[]>();
    const links = [...new Set(rows.map((r) => r.invite_id).filter((id): id is string => Boolean(id)))];
    if (links.length) {
      const used = this.sql
        .exec<{ id: string; name: string; invite_id: string }>(
          `SELECT id, name, invite_id FROM users WHERE invite_id IN (${links.map(() => '?').join(', ')})
           AND id NOT IN (SELECT alias FROM user_aliases) ORDER BY created_at`,
          ...links,
        )
        .toArray();
      for (const u of used) sameLink.set(u.invite_id, [...(sameLink.get(u.invite_id) ?? []), u]);
    }
    return rows.map((r) => {
      const user = toUser(r);
      if (r.inviter) {
        user.invitedBy = r.invite_label ? `${r.inviter} (${r.invite_label})` : r.inviter;
        const usedBy = (sameLink.get(r.invite_id!) ?? []).filter((u) => u.id !== r.id).slice(0, 10).map((u) => u.name);
        user.invite = { by: r.inviter, label: r.invite_label ?? '', usedBy };
      }
      return user;
    });
  }

  hasMembers(): boolean {
    return this.sql.exec(`SELECT 1 FROM users WHERE role IN ${MEMBER_ROLES} LIMIT 1`).toArray().length > 0;
  }

  isMod(uid: string): boolean {
    const role = this.user(uid)?.role;
    return role === 'owner' || role === 'mod';
  }

  aliasMap(): Map<string, string> {
    this.aliases ??= new Map(
      this.sql.exec<{ alias: string; uid: string }>('SELECT alias, uid FROM user_aliases').toArray().map((r) => [r.alias, r.uid]),
    );
    return this.aliases;
  }

  canonical(uid: string): string {
    return this.aliasMap().get(uid) ?? uid;
  }

  /** This uid plus every uid merged into it. */
  identitiesOf(uid: string): string[] {
    return [uid, ...[...this.aliasMap()].filter(([, to]) => to === uid).map(([alias]) => alias)];
  }

  /** Is `name` (case-insensitive) used by any live account other than `uid`? */
  nameTaken(name: string, uid: string): boolean {
    return this.sql
      .exec(
        `SELECT id FROM users WHERE lower(name) = lower(?) AND id != ? AND role != 'banned'
         AND id NOT IN (SELECT alias FROM user_aliases)`,
        name, uid,
      )
      .toArray().length > 0;
  }

  user(id: string): User | undefined {
    const row = this.sql.exec<UserRow>('SELECT id, name, color, last_seen, role, avatar, look, away, chime FROM users WHERE id = ?', id).toArray()[0];
    return row && toUser(row);
  }

  private sessions(): Session[] {
    return this.sockets().map((ws) => toSession(ws.deserializeAttachment() as Attachment));
  }

  /** Open sockets that have said hello. */
  sockets(): WebSocket[] {
    return this.ctx.getWebSockets().filter((ws) => {
      if (ws.readyState !== WebSocket.OPEN) return false;
      return (ws.deserializeAttachment() as Attachment | null)?.hello === true;
    });
  }

  /** Channels this person can read: live ones and their own Personal space. */
  readableChannels(me: Attachment): string[] {
    return [personalId(me.uid), ...this.channels().filter((c) => c.kind === 'text').map((c) => c.id)];
  }

  /**
   * Workers freeze Date.now() within an event, so two quick sends can share
   * a millisecond. Bumping keeps ts unique, so ordering and paging by ts is
   * exactly send order.
   */
  nextTs(): number {
    if (!this.lastTs) {
      this.lastTs = this.sql.exec<{ ts: number | null }>('SELECT MAX(ts) AS ts FROM messages').one().ts ?? 0;
    }
    this.lastTs = Math.max(Date.now(), this.lastTs + 1);
    return this.lastTs;
  }

  // ── Moderation log ─────────────────────────────────────────────────────

  /** Who did what to whom. Never message text. Anyone can read it. */
  log(actor: string | null, action: string, target: string | null, detail: string | null) {
    this.sql.exec('INSERT INTO mod_log (ts, actor, action, target, detail) VALUES (?, ?, ?, ?, ?)', Date.now(), actor, action, target, detail);
  }

  // ── This server's address ──────────────────────────────────────────────

  /** Where people reach this server (push links, Web Push): PUBLIC_URL, else the address the last connection came in on. */
  publicUrl(): string {
    return settings(this.env).publicUrl ?? (this.origin || this.sql.exec<{ value: string }>("SELECT value FROM meta WHERE key = 'origin'").toArray()[0]?.value) ?? '';
  }

  private rememberOrigin(url: string) {
    if (settings(this.env).publicUrl) return;
    const origin = new URL(url).origin;
    if (origin === this.origin) return;
    this.origin = origin;
    this.sql.exec("INSERT INTO meta (key, value) VALUES ('origin', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", origin);
  }

  // ── Sending ────────────────────────────────────────────────────────────

  send(ws: WebSocket, msg: ServerMessage) {
    try {
      ws.send(JSON.stringify(msg));
    } catch {
      // Socket died mid-send; its close handler cleans up.
    }
  }

  broadcast(msg: ServerMessage, except?: WebSocket) {
    this.sendWhere(msg, undefined, except);
  }

  /** One message to every signed-in connection `to` picks (all of them without it), serialized once. */
  sendWhere(msg: ServerMessage, to?: (a: Attachment) => boolean, except?: WebSocket | null) {
    const frame = JSON.stringify(msg);
    for (const ws of this.sockets()) {
      if (ws === except || (to && !to(ws.deserializeAttachment() as Attachment))) continue;
      try {
        ws.send(frame);
      } catch {} // socket died mid-send; its close handler cleans up
    }
  }

  /** Send to everyone currently on `view` (except `except`). */
  relayToView(view: string, except: WebSocket | null, msg: ServerMessage) {
    this.sendWhere(msg, (a) => a.view === view, except);
  }

  // ── Personal spaces ────────────────────────────────────────────────────
  // Everyone has one: channel id `p_<uid>`, no row in `channels`. Its
  // messages go only to its owner's own connections (never broadcast,
  // never pushed), and only the owner can page, edit, delete or react.

  personalChannel(uid: string): Channel {
    const id = personalId(uid);
    const last = this.sql.exec<{ ts: number | null }>('SELECT MAX(ts) AS ts FROM messages WHERE channel_id = ? AND deleted_at IS NULL', id).one().ts;
    return { id, name: 'Personal space', kind: 'personal', position: -1, lastMessageAt: last ?? null };
  }

  /** A channel this person may use: a live channel, or their own Personal space. */
  channelFor(me: Attachment, id: unknown): Channel | undefined {
    if (isPersonal(id)) return id === personalId(me.uid) ? this.personalChannel(me.uid) : undefined;
    return this.channel(id);
  }

  /** Send to everyone with access: the owner's connections for a Personal space, else everyone. */
  toAudience(channelId: string, msg: ServerMessage, except?: WebSocket) {
    if (isPersonal(channelId)) this.toOwner(channelId, msg, except);
    else this.broadcast(msg, except);
  }

  toOwner(channelId: string, msg: ServerMessage, except?: WebSocket) {
    const owner = channelId.slice(2);
    this.sendWhere(msg, (a) => !a.listen && a.uid === owner, except);
  }
}
