// Running the server: channels, custom emojis, letting people in, bans and
// roles, the mod log, personal invites, Places links, voice quality, storage,
// and the admin API's merge/role/stats calls.
import { tokenHash } from './auth';
import type { Attachment } from './attachment';
import { b64urlEncode, sameSecret } from './encoding';
import { settings } from './settings';
import { ClientError, EMOJI_NAME_RE, cleanName } from './validate';
import { LIMITS, type ClientMessage, type Emoji, type Invite, type JoinedUser, type Link, type ModLogEntry, type Role, type User } from './protocol';
import type { HubHost } from './hub-host';

/** Databases on the Free plan cap out around here; warn owner/mods at 80%. */
export const DB_LIMIT_BYTES = 1024 ** 3;

const BITRATES = new Set([32000, 64000, 96000, 128000, 192000, 256000, 384000, 510000]);

export type AddEmojiResult = { ok: true; emoji: Emoji } | { ok: false; status: number; code: string; message: string };

export function onChannelCreate(hub: HubHost, actor: string, msg: Extract<ClientMessage, { t: 'channel.create' }>) {
  if (msg.kind !== 'text' && msg.kind !== 'voice') throw new ClientError('bad_kind', 'kind must be text or voice.');
  let name = cleanName(msg.name, LIMITS.channelNameChars);
  // Names are shown as typed (no "#" or slugging any more).
  if (!name) throw new ClientError('bad_name', 'Channel needs a name.');
  const { next } = hub.sql.exec<{ next: number }>(
    'SELECT COALESCE(MAX(position), -1) + 1 AS next FROM channels WHERE kind = ?', msg.kind,
  ).one();
  const id = crypto.randomUUID().replace(/-/g, '').slice(0, 12);
  hub.sql.exec(
    'INSERT INTO channels (id, name, kind, position, created_at) VALUES (?, ?, ?, ?, ?)',
    id, name, msg.kind, next, Date.now(),
  );
  hub.log(actor, 'channel.create', null, `${msg.kind} channel "${name}"`);
  hub.broadcast({ t: 'channel', channel: hub.channel(id)! });
}

/** Admin API: add channels by name, in order, skipping ones that exist. */
export async function addChannels(hub: HubHost, names: string[], kind: 'text' | 'voice') {
  const added: string[] = [];
  for (const raw of names) {
    const name = cleanName(raw, LIMITS.channelNameChars);
    if (!name || hub.sql.exec('SELECT 1 FROM channels WHERE lower(name) = lower(?) AND kind = ? AND archived_at IS NULL', name, kind).toArray().length) continue;
    onChannelCreate(hub, 'admin', { t: 'channel.create', name, kind });
    added.push(name);
  }
  return { ok: true, added };
}

/** Admin API: archive channels by name (messages are kept; nobody sees the channel). */
export async function archiveChannels(hub: HubHost, names: string[], kind: 'text' | 'voice') {
  const archived: string[] = [];
  for (const name of names) {
    const row = hub.sql
      .exec<{ id: string; name: string }>('SELECT id, name FROM channels WHERE lower(name) = lower(?) AND kind = ? AND archived_at IS NULL', name, kind)
      .toArray()[0];
    if (!row) continue;
    onChannelArchive(hub, 'admin', { t: 'channel.archive', channelId: row.id });
    archived.push(row.name);
  }
  return { ok: true, archived };
}

/** Admin API: bring archived channels back by name; their messages were kept, so history returns too. */
export async function unarchiveChannels(hub: HubHost, names: string[], kind: 'text' | 'voice') {
  const restored: string[] = [];
  for (const name of names) {
    const live = hub.sql.exec('SELECT 1 FROM channels WHERE lower(name) = lower(?) AND kind = ? AND archived_at IS NULL', name, kind).toArray().length;
    const row = hub.sql
      .exec<{ id: string; name: string }>('SELECT id, name FROM channels WHERE lower(name) = lower(?) AND kind = ? AND archived_at IS NOT NULL ORDER BY archived_at DESC', name, kind)
      .toArray()[0];
    if (!row || live) continue;
    hub.sql.exec('UPDATE channels SET archived_at = NULL WHERE id = ?', row.id);
    hub.log('admin', 'channel.unarchive', null, `${kind} channel "${row.name}"`);
    hub.broadcast({ t: 'channel', channel: hub.channel(row.id)! });
    restored.push(row.name);
  }
  return { ok: true, restored };
}

export function onChannelArchive(hub: HubHost, actor: string, msg: Extract<ClientMessage, { t: 'channel.archive' }>) {
  const channel = hub.channel(msg.channelId);
  if (!channel) throw new ClientError('no_channel', 'That channel does not exist.');
  if (channel.kind === 'text') {
    const { n } = hub.sql.exec<{ n: number }>(
      "SELECT COUNT(*) AS n FROM channels WHERE kind = 'text' AND archived_at IS NULL",
    ).one();
    if (n <= 1) throw new ClientError('last_channel', 'Keep at least one text channel.');
  }
  // Archive, never delete: messages stay in the database.
  hub.sql.exec('UPDATE channels SET archived_at = ? WHERE id = ?', Date.now(), channel.id);
  hub.log(actor, 'channel.archive', null, `"${channel.name}"`);
  for (const ws of hub.sockets()) {
    const a = ws.deserializeAttachment() as Attachment;
    if (a.vc === channel.id) hub.setVoice(ws, a, null, a.muted, a.deafened);
  }
  hub.broadcast({ t: 'channel.archived', channelId: channel.id });
}

export function onEmojiDelete(hub: HubHost, actor: string, msg: Extract<ClientMessage, { t: 'emoji.delete' }>) {
  const id = String(msg.id);
  const row = hub.sql.exec<{ name: string }>('SELECT name FROM emojis WHERE id = ? AND deleted_at IS NULL', id).toArray()[0];
  if (!row) throw new ClientError('no_emoji', 'Emoji not found.');
  hub.log(actor, 'emoji.delete', null, `:${row.name}:`);
  // Soft delete: old messages keep showing it (their tokens carry the id).
  hub.sql.exec('UPDATE emojis SET deleted_at = ? WHERE id = ?', Date.now(), id);
  hub.broadcast({ t: 'emoji.deleted', id });
}

export function onEmojiRename(hub: HubHost, msg: Extract<ClientMessage, { t: 'emoji.rename' }>) {
  const id = String(msg.id);
  const name = String(msg.name ?? '');
  const row = hub.sql.exec<{ mime: string; user_id: string; created_at: number }>(
    'SELECT mime, user_id, created_at FROM emojis WHERE id = ? AND deleted_at IS NULL', id,
  ).toArray()[0];
  if (!row) throw new ClientError('no_emoji', 'Emoji not found.');
  if (!EMOJI_NAME_RE.test(name)) throw new ClientError('bad_name', 'Emoji names are 2–32 letters, numbers or underscores.');
  const taken = hub.sql
    .exec('SELECT id FROM emojis WHERE lower(name) = lower(?) AND deleted_at IS NULL AND id != ?', name, id)
    .toArray()[0];
  if (taken) throw new ClientError('name_taken', `:${name}: is already taken.`);
  hub.sql.exec('UPDATE emojis SET name = ? WHERE id = ?', name, id);
  // Old messages keep their <:oldname:id> tokens; the id still resolves.
  hub.broadcast({ t: 'emoji', emoji: { id, name, mime: row.mime, uid: hub.canonical(row.user_id), createdAt: row.created_at } });
}

// ── Moderation ─────────────────────────────────────────────────────────

/** The owner and mods let someone in (or turn them away). */
export function onApprove(hub: HubHost, me: Attachment, uid: unknown, approved: boolean) {
  if (!hub.isMod(me.uid)) throw new ClientError('forbidden', 'Only the owner and mods can let people in.');
  const target = hub.user(String(uid));
  if (!target || target.role !== 'pending') throw new ClientError('no_request', 'That join request is gone.');
  const newcomer = approved ? newcomerOf(hub, target) : undefined; // (read while they're still waiting)
  hub.sql.exec('UPDATE users SET role = ? WHERE id = ?', approved ? 'member' : 'banned', target.id);
  for (const ws of hub.ctx.getWebSockets()) {
    const a = ws.deserializeAttachment() as Attachment;
    if (!a.pending || a.uid !== target.id) continue;
    if (approved) {
      hub.admit(ws, a);
    } else {
      hub.send(ws, { t: 'error', code: 'denied', message: 'Nobody let you in this time.' });
      hub.closeQuietly(ws, 4003, 'denied');
    }
  }
  hub.log(me.uid, approved ? 'member.approve' : 'member.deny', target.id, null);
  hub.broadcast({ t: 'join.resolved', uid: target.id, approved, by: me.uid, ...(newcomer ? { user: newcomer } : {}) });
}

/**
 * Who was just let in, for join.resolved: members never saw the request, and
 * still say "Cleo joined — invited by Ada".
 */
function newcomerOf(hub: HubHost, target: User): JoinedUser {
  const waiting = hub.pendingUsers().find((u) => u.id === target.id);
  return { id: target.id, name: target.name, color: target.color, ...(waiting?.invitedBy ? { invitedBy: waiting.invitedBy } : {}) };
}

/** Owner and mods can remove someone: their sessions close and they can't come back. */
export function onBan(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'member.ban' }>) {
  if (!hub.isMod(me.uid)) throw new ClientError('forbidden', 'Only the owner and mods can ban.');
  const target = hub.user(hub.canonical(String(msg.uid)));
  if (!target || target.role === 'banned') throw new ClientError('no_user', 'No such member.');
  if (target.role === 'owner' || target.id === me.uid) throw new ClientError('forbidden', "Can't ban the owner or yourself.");
  hub.sql.exec("UPDATE users SET role = 'banned' WHERE id = ?", target.id);
  hub.log(me.uid, 'member.ban', target.id, null);
  for (const ws of hub.ctx.getWebSockets()) {
    const a = ws.deserializeAttachment() as Attachment;
    if (a.uid !== target.id) continue;
    hub.send(ws, { t: 'error', code: 'banned', message: "You've been removed from this server." });
    hub.kick(ws, 4003, 'banned');
  }
  hub.broadcast({ t: 'user', user: hub.user(target.id)! });
}

export function onRole(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'member.role' }>) {
  if (hub.user(me.uid)?.role !== 'owner') throw new ClientError('forbidden', 'Only the owner can make mods.');
  if (msg.role !== 'mod' && msg.role !== 'member') throw new ClientError('bad_role', 'Role must be mod or member.');
  const target = hub.user(hub.canonical(String(msg.uid)));
  if (!target || (target.role !== 'member' && target.role !== 'mod')) throw new ClientError('no_user', 'No such member.');
  hub.sql.exec('UPDATE users SET role = ? WHERE id = ?', msg.role, target.id);
  hub.log(me.uid, 'member.role', target.id, msg.role);
  hub.broadcast({ t: 'user', user: hub.user(target.id)! });
  if (msg.role === 'mod') showWaitingRoom(hub, target.id);
}

/** Someone just became a mod (or owner): the people already waiting, as if they'd just knocked. */
function showWaitingRoom(hub: HubHost, uid: string) {
  const waiting = hub.pendingUsers();
  if (!waiting.length) return;
  for (const ws of hub.sockets()) {
    const a = ws.deserializeAttachment() as Attachment;
    if (a.uid !== uid || a.listen) continue;
    for (const user of [...waiting].reverse()) hub.send(ws, { t: 'join.request', user });
  }
}

/** Become owner by proving you hold the server's admin key. */
export function onClaimOwner(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'owner.claim' }>) {
  if (!sameSecret(String(msg.key ?? ''), hub.env.ADMIN_KEY ?? '')) throw new ClientError('bad_key', "That isn't the admin key.");
  const wasMod = hub.isMod(me.uid);
  hub.sql.exec("UPDATE users SET role = 'owner' WHERE id = ?", me.uid);
  hub.log(me.uid, 'owner.claim', me.uid, null);
  hub.broadcast({ t: 'user', user: hub.user(me.uid)! });
  if (!wasMod) showWaitingRoom(hub, me.uid);
}

export function modLog(hub: HubHost): ModLogEntry[] {
  return hub.sql
    .exec<{ ts: number; actor: string | null; action: string; target: string | null; detail: string | null }>(
      'SELECT ts, actor, action, target, detail FROM mod_log ORDER BY id DESC LIMIT 200',
    )
    .toArray()
    .map((r) => ({ ts: r.ts, actor: r.actor && r.actor !== 'admin' ? hub.canonical(r.actor) : r.actor, action: r.action, target: r.target ? hub.canonical(r.target) : null, detail: r.detail }));
}

// ── Personal invite links ──────────────────────────────────────────────
// Anyone here can make one (attributed to them). Newcomers still wait in
// the waiting room; the request says whose link they used. Only a hash of
// the key is stored, so the link is shown once, when it's made.

export function invites(hub: HubHost, uid: string): Invite[] {
  const all = hub.isMod(uid);
  return hub.sql
    .exec<{ id: string; label: string; created_by: string; created_at: number; uses: number; last_used_at: number | null; revoked_at: number | null }>(
      `SELECT id, label, created_by, created_at, uses, last_used_at, revoked_at FROM invites
       ${all ? '' : 'WHERE created_by = ?'} ORDER BY created_at DESC LIMIT 100`,
      ...(all ? [] : [uid]),
    )
    .toArray()
    .map((r) => ({ id: r.id, label: r.label, by: hub.canonical(r.created_by), createdAt: r.created_at, uses: r.uses, lastUsedAt: r.last_used_at, revoked: r.revoked_at !== null }));
}

export async function onInviteCreate(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'invite.create' }>) {
  const label = cleanName(msg.label ?? '', 40) || 'Invite';
  const key = `yi_${b64urlEncode(crypto.getRandomValues(new Uint8Array(24)))}`;
  const id = crypto.randomUUID().replace(/-/g, '').slice(0, 12);
  const now = Date.now();
  hub.sql.exec(
    'INSERT INTO invites (id, key_hash, label, created_by, created_at) VALUES (?, ?, ?, ?, ?)',
    id, await tokenHash(key), label, me.uid, now,
  );
  hub.log(me.uid, 'invite.create', null, label);
  hub.send(ws, { t: 'invite.created', key, invite: { id, label, by: me.uid, createdAt: now, uses: 0, lastUsedAt: null, revoked: false } });
}

export function onInviteRevoke(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'invite.revoke' }>) {
  const row = hub.sql.exec<{ created_by: string; label: string }>('SELECT created_by, label FROM invites WHERE id = ? AND revoked_at IS NULL', String(msg.id)).toArray()[0];
  if (!row) throw new ClientError('no_invite', 'That invite is already gone.');
  if (hub.canonical(row.created_by) !== me.uid && !hub.isMod(me.uid)) throw new ClientError('forbidden', 'Only whoever made an invite (or a mod) can revoke it.');
  hub.sql.exec('UPDATE invites SET revoked_at = ? WHERE id = ?', Date.now(), String(msg.id));
  hub.log(me.uid, 'invite.revoke', null, row.label);
  hub.send(ws, { t: 'invites', invites: invites(hub, me.uid) });
}

/** Worker RPC: is this a live personal invite key? Returns its id. */
export async function checkInvite(hub: HubHost, key: string): Promise<string | null> {
  if (!/^yi_[A-Za-z0-9_-]{32}$/.test(key)) return null;
  const row = hub.sql.exec<{ id: string }>('SELECT id FROM invites WHERE key_hash = ? AND revoked_at IS NULL', await tokenHash(key)).toArray()[0];
  return row?.id ?? null;
}

// ── Sidebar links ──────────────────────────────────────────────────────

/**
 * The Places links. A board link only shows when this server has a board
 * (settings › BOARD); BOARD_NAME renames it, and with no address of its own
 * it opens this server's /board.
 */
export function links(hub: HubHost): Link[] {
  const { board, boardName } = settings(hub.env);
  return hub.sql
    .exec<{ id: string; name: string; url: string; view: string | null; description: string | null }>(
      'SELECT id, name, url, view, description FROM links WHERE archived_at IS NULL ORDER BY position, created_at',
    )
    .toArray()
    .filter((r) => r.view !== 'board' || board)
    .map((r) => (r.view === 'board' ? { ...r, name: boardName ?? r.name, url: r.url || `${hub.publicUrl()}/board` } : r))
    .map((r) => ({ id: r.id, name: r.name, url: r.url, ...(r.view ? { view: r.view } : {}), ...(r.description ? { description: r.description } : {}) }));
}

export function onLinkAdd(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'link.add' }>) {
  const name = cleanName(msg.name ?? '', 32);
  if (!name) throw new ClientError('bad_name', 'Give the link a name.');
  const url = String(msg.url ?? '').trim();
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new ClientError('bad_url', 'That isn’t a web address.');
  }
  if (parsed.protocol !== 'https:' || url.length > 300) throw new ClientError('bad_url', 'Links must start with https:// (and be under 300 characters).');
  if (links(hub).length >= 20) throw new ClientError('too_many', 'That’s plenty of links. Remove one first.');
  const { next } = hub.sql.exec<{ next: number }>('SELECT COALESCE(MAX(position), -1) + 1 AS next FROM links').one();
  hub.sql.exec(
    'INSERT INTO links (id, name, url, position, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?)',
    crypto.randomUUID().replace(/-/g, '').slice(0, 12), name, parsed.href, next, me.uid, Date.now(),
  );
  hub.log(me.uid, 'link.add', null, `${name} → ${parsed.host}`);
  hub.broadcast({ t: 'links', links: links(hub) });
}

export function onLinkRemove(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'link.remove' }>) {
  const row = hub.sql.exec<{ name: string }>('SELECT name FROM links WHERE id = ? AND archived_at IS NULL', String(msg.id)).toArray()[0];
  if (!row) throw new ClientError('no_link', 'That link is already gone.');
  hub.sql.exec('UPDATE links SET archived_at = ? WHERE id = ?', Date.now(), String(msg.id));
  hub.log(me.uid, 'link.remove', null, row.name);
  hub.broadcast({ t: 'links', links: links(hub) });
}

// ── Voice channel quality ──────────────────────────────────────────────

export function onChannelBitrate(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'channel.bitrate' }>) {
  const channel = hub.channel(msg.channelId);
  if (!channel || channel.kind !== 'voice') throw new ClientError('no_channel', 'That voice channel does not exist.');
  const bitrate = msg.bitrate === null ? null : Number(msg.bitrate);
  if (bitrate !== null && !BITRATES.has(bitrate)) throw new ClientError('bad_bitrate', 'Pick one of the listed qualities.');
  if ((channel.bitrate ?? null) === bitrate) return;
  hub.sql.exec('UPDATE channels SET bitrate = ? WHERE id = ?', bitrate, channel.id);
  hub.log(me.uid, 'channel.bitrate', null, `"${channel.name}" → ${bitrate ? `${bitrate / 1000} kbps` : 'personal settings'}`);
  hub.broadcast({ t: 'channel', channel: hub.channel(channel.id)! });
}

// ── Storage ────────────────────────────────────────────────────────────

async function storage(hub: HubHost): Promise<{ chatBytes: number; mediaBytes: number }> {
  const chatBytes = hub.ctx.storage.sql.databaseSize;
  let mediaBytes = 0;
  try {
    mediaBytes = (await hub.env.MEDIA.get(hub.env.MEDIA.idFromName('main')).stats()).databaseBytes;
  } catch {}
  return { chatBytes, mediaBytes };
}

export async function onStats(hub: HubHost, ws: WebSocket, me: Attachment) {
  if (!hub.isMod(me.uid)) throw new ClientError('forbidden', 'Only the owner and mods can see storage.');
  hub.send(ws, { t: 'stats', ...(await storage(hub)), limitBytes: DB_LIMIT_BYTES });
}

/**
 * Register an uploaded emoji. The image is already stored in Media.
 * Uploaded by a live session (`sid`) or by the admin API (`asAdmin`).
 */
export async function addEmoji(hub: HubHost, input: { id: string; name: string; mime: string; sid?: string; ut?: string; asAdmin?: boolean }): Promise<AddEmojiResult> {
  let uid = 'admin';
  if (!input.asAdmin) {
    const uploader = hub.uploader(input.sid, input.ut);
    if (!uploader) return { ok: false, status: 401, code: 'no_session', message: 'Connect first, then upload.' };
    uid = (uploader.deserializeAttachment() as Attachment).uid;
    if (!hub.allow(input.sid!, 'emoji', 30, 0.5)) return { ok: false, status: 429, code: 'slow_down', message: 'Too many uploads at once — give it a minute.' };
  }
  if (!EMOJI_NAME_RE.test(input.name)) {
    return { ok: false, status: 400, code: 'bad_name', message: 'Emoji names are 2–32 letters, numbers or underscores.' };
  }
  const taken = hub.sql
    .exec<{ id: string }>('SELECT id FROM emojis WHERE lower(name) = lower(?) AND deleted_at IS NULL', input.name)
    .toArray()[0];
  if (taken && taken.id !== input.id) {
    return { ok: false, status: 409, code: 'name_taken', message: `:${input.name}: is already taken.` };
  }
  const existing = hub.sql
    .exec<{ name: string; deleted_at: number | null }>('SELECT name, deleted_at FROM emojis WHERE id = ?', input.id)
    .toArray()[0];
  if (existing && existing.deleted_at === null) {
    return { ok: false, status: 409, code: 'duplicate', message: `That image is already :${existing.name}:.` };
  }
  const now = Date.now();
  if (existing) {
    // Same image uploaded again after a delete: bring it back under the new name.
    hub.sql.exec('UPDATE emojis SET name = ?, user_id = ?, created_at = ?, deleted_at = NULL WHERE id = ?', input.name, uid, now, input.id);
  } else {
    hub.sql.exec(
      'INSERT INTO emojis (id, name, mime, user_id, created_at) VALUES (?, ?, ?, ?, ?)',
      input.id, input.name, input.mime, uid, now,
    );
  }
  const emoji: Emoji = { id: input.id, name: input.name, mime: input.mime, uid, createdAt: now };
  hub.broadcast({ t: 'emoji', emoji });
  return { ok: true, emoji };
}

/**
 * Merge a duplicate account into the real one: `from` is read as `into`
 * everywhere (messages, emojis, deletes), `from` drops off the member list,
 * and its devices adopt `into` on their next connect. Reversible: unmerge.
 */
export async function mergeUsers(hub: HubHost, from: string, into: string) {
  into = hub.canonical(into);
  if (from === into) return { ok: false, message: 'Same user.' };
  if (!hub.user(from) || !hub.user(into)) return { ok: false, message: 'Unknown user id.' };
  if (hub.aliasMap().has(from)) return { ok: false, message: `${from} is already merged.` };
  const now = Date.now();
  hub.sql.exec('INSERT INTO user_aliases (alias, uid, created_at) VALUES (?, ?, ?)', from, into, now);
  hub.sql.exec('UPDATE user_aliases SET uid = ? WHERE uid = ?', into, from); // chains collapse
  hub.aliases = null;
  hub.log('admin', 'member.merge', into, from);
  hub.reconnectEveryone();
  return { ok: true, from: hub.user(from), into: hub.user(into) };
}

/** Admin API: set anyone's role (e.g. make someone the owner). */
export async function setRole(hub: HubHost, uid: string, role: Role) {
  const target = hub.user(hub.canonical(uid));
  if (!target) return { ok: false, message: 'Unknown user id.' };
  const newcomer = target.role === 'pending' && role !== 'banned' ? newcomerOf(hub, target) : undefined;
  hub.sql.exec('UPDATE users SET role = ? WHERE id = ?', role, target.id);
  hub.log('admin', 'member.role', target.id, role);
  for (const ws of hub.ctx.getWebSockets()) {
    const a = ws.deserializeAttachment() as Attachment;
    if (a.uid !== target.id) continue;
    if (role === 'banned') {
      hub.send(ws, { t: 'error', code: 'banned', message: "You've been removed from this server." });
      hub.kick(ws, 4003, 'banned');
    } else if (a.pending) hub.admit(ws, a); // approved from the admin API
  }
  if (newcomer) hub.broadcast({ t: 'join.resolved', uid: target.id, approved: true, by: 'admin', user: newcomer });
  hub.broadcast({ t: 'user', user: hub.user(target.id)! });
  if ((role === 'mod' || role === 'owner') && target.role !== 'mod' && target.role !== 'owner') showWaitingRoom(hub, target.id);
  return { ok: true, user: hub.user(target.id) };
}

export async function unmergeUser(hub: HubHost, alias: string) {
  hub.sql.exec('DELETE FROM user_aliases WHERE alias = ?', alias);
  hub.aliases = null;
  hub.log('admin', 'member.unmerge', null, alias);
  hub.reconnectEveryone();
  return { ok: true };
}

export async function stats(hub: HubHost) {
  const count = (table: string) => hub.sql.exec<{ n: number }>(`SELECT COUNT(*) AS n FROM ${table}`).one().n;
  return {
    databaseBytes: hub.ctx.storage.sql.databaseSize,
    messages: count('messages'),
    users: count('users'),
    channels: count('channels'),
    emojis: count('emojis'),
    sessions: hub.sockets().length,
    pending: hub.pendingUsers().length,
  };
}
