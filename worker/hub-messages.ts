// Messages: history, sending, editing and deleting, replies, reactions,
// polls, search, pins, the media shelf, catch-up counts, and sealing old
// Personal-space notes.
import { findMentions } from '../web/js/mentions.js';
import type { Attachment } from './attachment';
import { MESSAGE_COLUMNS, toMessage, type MessageExtra, type MessageRow } from './rows';
import { cleanPoll, doodleId, rollDice } from './extras';
import {
  ClientError,
  ENCRYPTED_NOTE_RE,
  MEDIA_ID_RE,
  bodyLimit,
  clamp,
  cleanGif,
  cleanImage,
  isEmoji,
  isPersonal,
  personalId,
  tooLong,
} from './validate';
import { LIMITS, type ClientMessage, type ImageRef, type Message, type Reaction, type ReplyRef, type ServerMessage, type User } from './protocol';
import type { HubHost } from './hub-host';
import { awayReplies, backFromAway, pushForMessage } from './hub-pushes';

export function onHistory(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'history' }>) {
  // Archived channels keep their messages (backups), but nobody can page
  // them; a Personal space only by its owner.
  if (!hub.channelFor(me, msg.channelId)) throw new ClientError('no_channel', 'That channel does not exist.');
  const limit = clamp(Number(msg.limit) || LIMITS.historyPage, 1, LIMITS.historyMax);
  const before = Number.isFinite(msg.before) ? Number(msg.before) : Number.MAX_SAFE_INTEGER;
  const beforeId = typeof msg.beforeId === 'string' ? msg.beforeId : '￿';
  // (ts, id) < (before, beforeId) as a row value: the index then starts right at
  // `before`, so a page far back reads one page of rows, not every newer one.
  const rows = hub.sql
    .exec<MessageRow>(
      `SELECT ${MESSAGE_COLUMNS} FROM messages
       WHERE channel_id = ? AND deleted_at IS NULL AND (ts, id) < (?, ?)
       ORDER BY ts DESC, id DESC LIMIT ?`,
      msg.channelId, before, beforeId, limit + 1,
    )
    .toArray();
  // Long messages: keep a page well under the WebSocket frame limit.
  let size = 0;
  let fit = 0;
  for (const r of rows.slice(0, limit)) {
    size += r.body.length * 2 + (r.gif?.length ?? 0) + 200;
    if (fit && size > LIMITS.historyChars) break;
    fit++;
  }
  const hasMore = rows.length > fit;
  hub.send(ws, { t: 'history', channelId: msg.channelId, messages: present(hub, rows.slice(0, fit).reverse()), hasMore });
}

export function onSend(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'send' }>) {
  const channel = hub.channelFor(me, msg.channelId);
  if (!channel || channel.kind === 'voice') throw new ClientError('no_channel', 'That channel does not exist.');
  // The nonce only matters to the sender (to swap its optimistic copy, and to send again).
  const nonce = typeof msg.nonce === 'string' ? msg.nonce.slice(0, 64) : undefined;
  let body = typeof msg.body === 'string' ? msg.body.replace(/\s+$/, '') : '';
  if (nonce && answerRepeat(hub, ws, me, channel.id, nonce, body)) return;
  const personal = channel.kind === 'personal';
  const gif = msg.gif === undefined || msg.gif === null ? undefined : cleanGif(msg.gif);
  const image = msg.image === undefined || msg.image === null ? undefined : cleanImage(msg.image);
  if (msg.image && !image) throw new ClientError('bad_image', 'That picture looks broken.');
  const extra: MessageExtra = {};
  if (msg.poll !== undefined && msg.poll !== null) {
    if (personal) throw new ClientError('bad_poll', 'Polls are for channels.');
    extra.poll = cleanPoll(msg.poll);
    if (!body.trim() || body.length > LIMITS.pollQuestionChars) throw new ClientError('bad_poll', `Ask a question (up to ${LIMITS.pollQuestionChars} characters).`);
  } else if (!gif && !image) {
    const dice = rollDice(body); // the server rolls, so nobody can fake a result
    if (dice) ({ body, roll: extra.roll } = dice);
  }
  let replyTo: string | null = null;
  if (msg.replyTo !== undefined && msg.replyTo !== null) {
    // A reply answers a message in the same channel; in your Personal space it
    // may point anywhere you can read (Save to Personal: a bookmark).
    const original = hub.sql
      .exec<{ id: string; channel_id: string }>('SELECT id, channel_id FROM messages WHERE id = ? AND deleted_at IS NULL', String(msg.replyTo))
      .toArray()[0];
    const reachable = original && (personal ? hub.readableChannels(me).includes(original.channel_id) : original.channel_id === channel.id);
    if (!reachable) throw new ClientError('no_message', 'The message you replied to is gone.');
    replyTo = original.id;
  }
  if (!body.trim() && !gif && !image && !(personal && replyTo)) throw new ClientError('empty', 'Message is empty.');
  if (body.length > bodyLimit(channel, body)) throw tooLong(channel, body);
  hub.limit(me.uid, 'send', 10, 1); // only real messages count toward the spam limit (per account: more connections don't help)
  const message: Message = { id: crypto.randomUUID(), channelId: channel.id, uid: me.uid, body, ts: hub.nextTs() };
  if (gif) message.gif = gif;
  if (image) message.image = image;
  if (replyTo) message.reply = replyRef(hub, replyTo);
  if (extra.poll) message.poll = { options: extra.poll.options, votes: extra.poll.options.map(() => []) };
  if (extra.roll) message.roll = extra.roll;
  hub.sql.exec(
    'INSERT INTO messages (id, channel_id, user_id, body, ts, gif, image, reply_to, extra, nonce) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
    message.id, message.channelId, message.uid, message.body, message.ts, gif ? JSON.stringify(gif) : null, image ? JSON.stringify(image) : null, replyTo,
    extra.poll || extra.roll ? JSON.stringify(extra) : null, nonce || null,
  );
  hub.send(ws, { t: 'message', message, nonce });
  if (personal) return hub.toOwner(channel.id, { t: 'message', message }, ws); // no pushes either
  hub.broadcast({ t: 'message', message }, ws);
  const mentions = message.body ? findMentions(message.body, hub.users()) : { uids: [], everyone: false };
  // A reply mentions the person it answers.
  if (message.reply && message.reply.uid !== me.uid && !mentions.uids.includes(message.reply.uid)) mentions.uids.push(message.reply.uid);
  pushForMessage(hub, me, channel.name, message, mentions);
  awayReplies(hub, ws, me, mentions.uids);
  backFromAway(hub, me.uid);
}

/** How long a send may come again (a reconnect resends what had no answer yet) and still be the same message. */
const REPEAT_MS = 24 * 3_600_000;

/**
 * A send that already made a message: the socket went before its echo arrived,
 * and the client sent the same frame (same nonce) again after `ready`
 * (PROTOCOL.md › Identity). Answers it with that message and stores and tells
 * nobody anything new. False when it's a new send.
 */
function answerRepeat(hub: HubHost, ws: WebSocket, me: Attachment, channelId: string, nonce: string, body: string): boolean {
  // (No ts or ORDER BY in the query: either one makes SQLite walk the channel's index instead of the nonce's.)
  const since = Date.now() - REPEAT_MS;
  const sent = hub.sql
    .exec<MessageRow & { deleted_at: number | null }>(
      `SELECT ${MESSAGE_COLUMNS}, deleted_at FROM messages WHERE user_id = ? AND nonce = ? AND channel_id = ? LIMIT 20`,
      me.uid, nonce, channelId,
    )
    .toArray()
    .filter((r) => r.ts > since)
    .sort((x, y) => y.ts - x.ts)[0];
  if (!sent) return false;
  // Deleted meanwhile (by a mod, say): sending it again mustn't bring it back.
  if (sent.deleted_at) throw new ClientError('no_message', 'That message was deleted.');
  // Other words under the same nonce are a new message from a client that reuses nonces (a roll's text is the server's).
  if (sent.body !== body && !sent.extra?.includes('"roll"')) return false;
  hub.send(ws, { t: 'message', message: present(hub, [sent])[0], nonce });
  return true;
}

// ── Reactions ──────────────────────────────────────────────────────────

export function onReact(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'react' }>) {
  const doodle = typeof msg.emoji === 'string' ? doodleId(msg.emoji) : null;
  if (!doodle && !isEmoji(msg.emoji)) throw new ClientError('bad_emoji', "That isn't an emoji.");
  const row = hub.sql.exec<{ channel_id: string }>('SELECT channel_id FROM messages WHERE id = ? AND deleted_at IS NULL', String(msg.id)).toArray()[0];
  if (!row) throw new ClientError('no_message', 'Message not found.');
  if (isPersonal(row.channel_id) && row.channel_id !== personalId(me.uid)) throw new ClientError('no_message', 'Message not found.');
  const id = String(msg.id);
  const removed: string[] = [];
  const remove = (emoji: string) => {
    // Also remove reactions made from accounts merged into this one.
    for (const uid of hub.identitiesOf(me.uid)) hub.sql.exec('DELETE FROM reactions WHERE message_id = ? AND user_id = ? AND emoji = ?', id, uid, emoji);
    removed.push(emoji);
  };
  if (msg.on) {
    if (doodle) {
      // One doodle each per message: a new one replaces yours.
      const mine = hub.sql
        .exec<{ emoji: string }>(`SELECT emoji FROM reactions WHERE message_id = ? AND user_id IN (${hub.identitiesOf(me.uid).map(() => '?').join(', ')}) AND emoji LIKE '<d:%'`, id, ...hub.identitiesOf(me.uid))
        .toArray();
      for (const r of mine) if (r.emoji !== msg.emoji) remove(r.emoji);
    }
    const distinct = hub.sql.exec<{ n: number }>('SELECT COUNT(DISTINCT emoji) AS n FROM reactions WHERE message_id = ? AND emoji != ?', id, msg.emoji).one().n;
    if (distinct >= 20) throw new ClientError('too_many', 'That message has plenty of reactions already.');
    hub.sql.exec('INSERT OR IGNORE INTO reactions (message_id, user_id, emoji, ts) VALUES (?, ?, ?, ?)', id, me.uid, msg.emoji, Date.now());
  } else remove(msg.emoji);
  for (const emoji of removed) {
    if (emoji !== msg.emoji) hub.toAudience(row.channel_id, { t: 'reaction', messageId: id, channelId: row.channel_id, emoji, uid: me.uid, on: false });
  }
  hub.toAudience(row.channel_id, { t: 'reaction', messageId: id, channelId: row.channel_id, emoji: msg.emoji, uid: me.uid, on: Boolean(msg.on) });
  // A doodle nobody shows any more is erased (like a deleted post's picture).
  const gone = removed.map(doodleId).filter((d): d is string => !!d);
  for (const d of gone) hub.sql.exec('INSERT OR IGNORE INTO media_trash (id) VALUES (?)', d);
  if (gone.length) hub.ctx.waitUntil(hub.emptyTrash());
}

// ── Polls ──────────────────────────────────────────────────────────────

export function onPollVote(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'poll.vote' }>) {
  const row = hub.sql
    .exec<{ channel_id: string; extra: string | null }>('SELECT channel_id, extra FROM messages WHERE id = ? AND deleted_at IS NULL', String(msg.id))
    .toArray()[0];
  const poll = row?.extra ? (JSON.parse(row.extra) as MessageExtra).poll : undefined;
  if (!row || !poll || !hub.readableChannels(me).includes(row.channel_id)) throw new ClientError('no_message', 'That poll is gone.');
  const id = String(msg.id);
  const option = msg.option === null ? null : Number(msg.option);
  if (option !== null && !(Number.isInteger(option) && option >= 0 && option < poll.options.length)) throw new ClientError('bad_poll', 'No such option.');
  for (const uid of hub.identitiesOf(me.uid)) hub.sql.exec('DELETE FROM poll_votes WHERE message_id = ? AND user_id = ?', id, uid);
  if (option !== null) hub.sql.exec('INSERT INTO poll_votes (message_id, user_id, option, ts) VALUES (?, ?, ?, ?)', id, me.uid, option, Date.now());
  hub.broadcast({ t: 'poll', id, channelId: row.channel_id, votes: pollVotes(hub, id, poll.options.length) });
}

/** Reactions for some messages, grouped by emoji in first-used order. */
function reactionsOf(hub: HubHost, ids: string[]): Map<string, Reaction[]> {
  const out = new Map<string, Reaction[]>();
  if (!ids.length) return out;
  const rows = hub.sql
    .exec<{ message_id: string; user_id: string; emoji: string }>(
      `SELECT message_id, user_id, emoji FROM reactions WHERE message_id IN (${ids.map(() => '?').join(', ')}) ORDER BY ts, rowid`,
      ...ids,
    )
    .toArray();
  for (const r of rows) {
    const list = out.get(r.message_id) ?? [];
    let entry = list.find((x) => x.emoji === r.emoji);
    if (!entry) list.push((entry = { emoji: r.emoji, uids: [] }));
    const uid = hub.canonical(r.user_id);
    if (!entry.uids.includes(uid)) entry.uids.push(uid);
    out.set(r.message_id, list);
  }
  return out;
}

/** Edit your own message. The previous text goes to message_edits first. */
export function onEdit(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'edit' }>) {
  const row = hub.sql.exec<{ user_id: string; channel_id: string; body: string; gif: string | null; image: string | null; extra: string | null }>(
    'SELECT user_id, channel_id, body, gif, image, extra FROM messages WHERE id = ? AND deleted_at IS NULL', String(msg.id),
  ).toArray()[0];
  if (!row) throw new ClientError('no_message', 'Message not found.');
  if (hub.canonical(row.user_id) !== me.uid) throw new ClientError('forbidden', 'You can only edit your own messages.');
  if (row.extra?.includes('"roll"')) throw new ClientError('forbidden', 'A roll is a roll — it can’t be edited.');
  const channel = hub.channelFor(me, row.channel_id);
  if (!channel) throw new ClientError('no_channel', 'That channel does not exist.');
  const body = typeof msg.body === 'string' ? msg.body.replace(/\s+$/, '') : '';
  if (!body.trim() && !row.gif && !row.image) throw new ClientError('empty', 'Message is empty. Delete it instead.');
  if (body.length > bodyLimit(channel, body)) throw tooLong(channel, body);
  if (body === row.body) return;
  const now = Date.now();
  hub.sql.exec('INSERT INTO message_edits (message_id, body, edited_at) VALUES (?, ?, ?)', String(msg.id), row.body, now);
  hub.sql.exec('UPDATE messages SET body = ?, edited_at = ? WHERE id = ?', body, now, String(msg.id));
  hub.toAudience(row.channel_id, { t: 'edited', id: String(msg.id), channelId: row.channel_id, body, editedAt: now });
}

export function onDelete(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'delete' }>) {
  const row = hub.sql.exec<{ user_id: string; channel_id: string; image: string | null }>(
    'SELECT user_id, channel_id, image FROM messages WHERE id = ? AND deleted_at IS NULL', String(msg.id),
  ).toArray()[0];
  if (!row) throw new ClientError('no_message', 'Message not found.');
  // A Personal space is its owner's alone — mods included.
  if (isPersonal(row.channel_id) && row.channel_id !== personalId(me.uid)) throw new ClientError('no_message', 'Message not found.');
  if (hub.canonical(row.user_id) !== me.uid && !hub.isMod(me.uid)) throw new ClientError('forbidden', 'You can only delete your own messages.');
  if (hub.canonical(row.user_id) !== me.uid) hub.log(me.uid, 'message.delete', hub.canonical(row.user_id), null);
  // Erased, not hidden ("dont save deleted posts"): the text, GIF,
  // picture, reactions and earlier versions go. The empty row stays so the
  // id is never reused and replies can say "deleted".
  const id = String(msg.id);
  const trash = new Set<string>();
  if (row.image) trash.add((JSON.parse(row.image) as ImageRef).id);
  // Encrypted pictures: only their owner knows the ids (they're inside the note).
  if (row.channel_id === personalId(me.uid) && Array.isArray(msg.blobs)) {
    for (const b of msg.blobs.slice(0, 10)) if (typeof b === 'string' && MEDIA_ID_RE.test(b)) trash.add(b);
  }
  for (const r of hub.sql.exec<{ emoji: string }>("SELECT emoji FROM reactions WHERE message_id = ? AND emoji LIKE '<d:%'", id).toArray()) {
    const d = doodleId(r.emoji);
    if (d) trash.add(d);
  }
  hub.sql.exec("UPDATE messages SET deleted_at = ?, body = '', gif = NULL, image = NULL, pinned_at = NULL, pinned_by = NULL, extra = NULL WHERE id = ?", Date.now(), id);
  hub.sql.exec('DELETE FROM message_edits WHERE message_id = ?', id);
  hub.sql.exec('DELETE FROM reactions WHERE message_id = ?', id);
  hub.sql.exec('DELETE FROM poll_votes WHERE message_id = ?', id);
  for (const t of trash) hub.sql.exec('INSERT OR IGNORE INTO media_trash (id) VALUES (?)', t);
  hub.toAudience(row.channel_id, { t: 'deleted', id, channelId: row.channel_id });
  if (trash.size) hub.ctx.waitUntil(hub.emptyTrash());
}

/** What a reply shows of the message it answers. */
function replyRef(hub: HubHost, id: string): ReplyRef {
  const r = hub.sql
    .exec<{ user_id: string; channel_id: string; body: string; gif: string | null; image: string | null; deleted_at: number | null }>(
      'SELECT user_id, channel_id, body, gif, image, deleted_at FROM messages WHERE id = ?', id,
    )
    .toArray()[0];
  if (!r || r.deleted_at) return { id, uid: r ? hub.canonical(r.user_id) : '', ...(r ? { channelId: r.channel_id } : {}), body: '', deleted: true };
  const ref: ReplyRef = { id, uid: hub.canonical(r.user_id), channelId: r.channel_id, body: r.body.slice(0, 200) };
  if (r.gif) ref.gif = true;
  if (r.image) ref.image = true;
  return ref;
}

// ── Sealing old Personal-space notes (when encryption is turned on) ─────

export function onPersonalPlain(hub: HubHost, ws: WebSocket, me: Attachment) {
  const notes = hub.sql
    .exec<MessageRow>(
      `SELECT id, channel_id, user_id, body, ts, gif, image FROM messages
       WHERE channel_id = ? AND deleted_at IS NULL AND body NOT LIKE 'e1:%' ORDER BY ts LIMIT 2000`,
      personalId(me.uid),
    )
    .toArray()
    .map((r) => {
      const m = toMessage(r);
      return { id: m.id, body: m.body, ...(m.gif ? { gif: m.gif } : {}), ...(m.image ? { image: m.image } : {}) };
    });
  hub.send(ws, { t: 'personal.plain', notes });
}

/**
 * Swap plain notes for their sealed versions. The plain text (and earlier
 * plain versions) is erased; a picture moved into the note goes to the trash.
 */
export function onPersonalSeal(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'personal.seal' }>) {
  if (!Array.isArray(msg.notes) || msg.notes.length > 50) throw new ClientError('bad_seal', 'Up to 50 notes at a time.');
  const channel = hub.personalChannel(me.uid);
  let trashed = false;
  for (const n of msg.notes) {
    const id = String(n?.id ?? '');
    const body = String(n?.body ?? '');
    if (!ENCRYPTED_NOTE_RE.test(body) || body.length > bodyLimit(channel, body)) throw new ClientError('bad_seal', 'That doesn’t look sealed.');
    const row = hub.sql
      .exec<{ body: string; image: string | null; edited_at: number | null }>(
        'SELECT body, image, edited_at FROM messages WHERE id = ? AND channel_id = ? AND deleted_at IS NULL', id, channel.id,
      )
      .toArray()[0];
    if (!row || ENCRYPTED_NOTE_RE.test(row.body)) continue;
    if (row.image) {
      hub.sql.exec('INSERT OR IGNORE INTO media_trash (id) VALUES (?)', (JSON.parse(row.image) as ImageRef).id);
      trashed = true;
    }
    hub.sql.exec('UPDATE messages SET body = ?, gif = NULL, image = NULL WHERE id = ?', body, id);
    hub.sql.exec('DELETE FROM message_edits WHERE message_id = ?', id);
    hub.toOwner(channel.id, { t: 'edited', id, channelId: channel.id, body, editedAt: row.edited_at ?? 0, sealed: true });
  }
  if (trashed) hub.ctx.waitUntil(hub.emptyTrash());
}

/** Full messages as clients see them: canonical author, reply, reactions, poll votes. */
function present(hub: HubHost, rows: MessageRow[]): Message[] {
  const reactions = reactionsOf(hub, rows.map((r) => r.id));
  return rows.map((r) => {
    const m: Message = { ...toMessage(r), uid: hub.canonical(r.user_id) };
    if (r.reply_to) m.reply = replyRef(hub, r.reply_to);
    const list = reactions.get(r.id);
    if (list) m.reactions = list;
    if (m.poll) m.poll = { ...m.poll, votes: pollVotes(hub, r.id, m.poll.options.length) };
    return m;
  });
}

/** votes[i] = uids who picked option i (canonical, oldest vote first). */
function pollVotes(hub: HubHost, id: string, options: number): string[][] {
  const votes: string[][] = Array.from({ length: options }, () => []);
  for (const r of hub.sql.exec<{ user_id: string; option: number }>('SELECT user_id, option FROM poll_votes WHERE message_id = ? ORDER BY ts, rowid', id).toArray()) {
    const uid = hub.canonical(r.user_id);
    if (votes[r.option] && !votes.some((v) => v.includes(uid))) votes[r.option].push(uid);
  }
  return votes;
}

/**
 * Plain-text search, newest first. Words must all appear; `from:Name` and
 * `has:image|gif|link` narrow it. A LIKE scan is fine at friend-group size.
 */
export function onSearch(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'search' }>) {
  const q = String(msg.q ?? '').slice(0, 200);
  const where: string[] = ['deleted_at IS NULL', "body NOT LIKE 'e1:%'"];
  const args: (string | number)[] = [];
  const readable = hub.readableChannels(me);
  const scope = typeof msg.channelId === 'string' && msg.channelId ? [msg.channelId].filter((c) => readable.includes(c)) : readable;
  if (!scope.length) throw new ClientError('no_channel', 'That channel does not exist.');
  where.push(`channel_id IN (${scope.map(() => '?').join(',')})`);
  args.push(...scope);
  let words = 0;
  for (const token of q.match(/(?:[^\s"]+|"[^"]*")+/g) ?? []) {
    const [, key, value] = token.match(/^(from|has):(.+)$/i) ?? [];
    if (key?.toLowerCase() === 'from') {
      const name = value.replace(/^"|"$/g, '');
      const ids = hub.users().filter((u) => u.name.toLowerCase().startsWith(name.toLowerCase())).map((u) => u.id);
      const all = [...ids, ...hub.sql.exec<{ alias: string }>('SELECT alias FROM user_aliases').toArray().filter((a) => ids.includes(hub.canonical(a.alias))).map((a) => a.alias)];
      if (!all.length) return hub.send(ws, { t: 'search.results', q, channelId: msg.channelId, messages: [] });
      where.push(`user_id IN (${all.map(() => '?').join(',')})`);
      args.push(...all);
    } else if (key?.toLowerCase() === 'has') {
      const what = value.toLowerCase();
      if (what.startsWith('image') || what.startsWith('picture')) where.push('image IS NOT NULL');
      else if (what.startsWith('gif')) where.push('gif IS NOT NULL');
      else if (what.startsWith('link')) where.push("body LIKE '%http%'");
    } else {
      const word = token.replace(/^"|"$/g, '').toLowerCase();
      if (!word) continue;
      where.push("lower(body) LIKE ? ESCAPE '\\'");
      args.push(`%${word.replace(/[\\%_]/g, (c) => `\\${c}`)}%`);
      words++;
    }
  }
  if (where.length === 3 && !words) return hub.send(ws, { t: 'search.results', q, channelId: msg.channelId, messages: [] });
  const rows = hub.sql
    .exec<MessageRow>(
      `SELECT ${MESSAGE_COLUMNS} FROM messages
       WHERE ${where.join(' AND ')} ORDER BY ts DESC LIMIT 50`,
      ...args,
    )
    .toArray();
  hub.send(ws, { t: 'search.results', q, channelId: msg.channelId, messages: present(hub, rows) });
}

export function onPin(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'pin' }>) {
  const row = hub.sql
    .exec<{ channel_id: string; pinned_at: number | null }>('SELECT channel_id, pinned_at FROM messages WHERE id = ? AND deleted_at IS NULL', String(msg.id))
    .toArray()[0];
  if (!row || !hub.readableChannels(me).includes(row.channel_id)) throw new ClientError('no_message', 'Message not found.');
  const on = Boolean(msg.on);
  if (on === Boolean(row.pinned_at)) return;
  if (on) {
    const { n } = hub.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM messages WHERE channel_id = ? AND pinned_at IS NOT NULL', row.channel_id).one();
    if (n >= 50) throw new ClientError('too_many', 'This channel has 50 pins. Unpin one first.');
  }
  const at = on ? Date.now() : null;
  hub.sql.exec('UPDATE messages SET pinned_at = ?, pinned_by = ? WHERE id = ?', at, on ? me.uid : null, String(msg.id));
  hub.toAudience(row.channel_id, { t: 'pinned', id: String(msg.id), channelId: row.channel_id, pinnedAt: at, by: me.uid });
}

export function onPins(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'pins' }>) {
  const channelId = String(msg.channelId ?? '');
  if (!hub.readableChannels(me).includes(channelId)) throw new ClientError('no_channel', 'That channel does not exist.');
  const rows = hub.sql
    .exec<MessageRow>(
      `SELECT ${MESSAGE_COLUMNS} FROM messages
       WHERE channel_id = ? AND deleted_at IS NULL AND pinned_at IS NOT NULL ORDER BY pinned_at DESC LIMIT 50`,
      channelId,
    )
    .toArray();
  hub.send(ws, { t: 'pins', channelId, messages: present(hub, rows) });
}

// ── Media shelf and catching up ────────────────────────────────────────

export function onShelf(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'shelf' }>) {
  const channelId = String(msg.channelId ?? '');
  if (!hub.readableChannels(me).includes(channelId)) throw new ClientError('no_channel', 'That channel does not exist.');
  const before = Number.isFinite(msg.before) ? Number(msg.before) : Number.MAX_SAFE_INTEGER;
  const rows = hub.sql
    .exec<MessageRow>(
      `SELECT ${MESSAGE_COLUMNS} FROM messages
       WHERE channel_id = ? AND deleted_at IS NULL AND (image IS NOT NULL OR gif IS NOT NULL) AND ts < ?
       ORDER BY ts DESC LIMIT 61`,
      channelId, before,
    )
    .toArray();
  hub.send(ws, { t: 'shelf', channelId, items: present(hub, rows.slice(0, 60)), hasMore: rows.length > 60 });
}

export function onUnreadCounts(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'unread.counts' }>) {
  const readable = new Set(hub.readableChannels(me));
  const counts: Extract<ServerMessage, { t: 'unread.counts' }>['counts'] = {};
  const mine = hub.identitiesOf(me.uid);
  const notMine = `user_id NOT IN (${mine.map(() => '?').join(', ')})`;
  let members: User[] | null = null;
  for (const [channelId, since] of Object.entries(msg.since ?? {}).slice(0, 50)) {
    const ts = Number(since);
    if (!readable.has(channelId) || !Number.isFinite(ts) || isPersonal(channelId)) continue;
    const { n } = hub.sql.exec<{ n: number }>(`SELECT COUNT(*) AS n FROM (SELECT 1 FROM messages WHERE channel_id = ? AND ts > ? AND deleted_at IS NULL AND ${notMine} LIMIT 999)`, channelId, ts, ...mine).one();
    const first = n
      ? hub.sql
        .exec<{ id: string; ts: number; user_id: string; body: string }>(`SELECT id, ts, user_id, body FROM messages WHERE channel_id = ? AND ts > ? AND deleted_at IS NULL AND ${notMine} ORDER BY ts LIMIT 1`, channelId, ts, ...mine)
        .toArray()[0]
      : undefined;
    let mentions = 0;
    if (n) {
      // Only messages that could mention you are read: an @ in the text, or a reply.
      const rows = hub.sql
        .exec<{ body: string; reply_uid: string | null }>(
          `SELECT m.body, r.user_id AS reply_uid FROM messages m LEFT JOIN messages r ON r.id = m.reply_to
           WHERE m.channel_id = ? AND m.ts > ? AND m.deleted_at IS NULL AND m.user_id NOT IN (${mine.map(() => '?').join(', ')})
           AND (instr(m.body, '@') > 0 OR m.reply_to IS NOT NULL) ORDER BY m.ts LIMIT 999`,
          channelId, ts, ...mine,
        )
        .toArray();
      for (const row of rows) {
        if (row.reply_uid && mine.includes(row.reply_uid)) mentions++;
        else if (row.body.includes('@')) {
          const found = findMentions(row.body, (members ??= hub.users()));
          if (found.everyone || found.uids.includes(me.uid)) mentions++;
        }
      }
    }
    counts[channelId] = {
      n,
      first: first ? { id: first.id, ts: first.ts, uid: hub.canonical(first.user_id), body: first.body.slice(0, 200) } : null,
      mentions: Math.min(mentions, n),
    };
  }
  hub.send(ws, { t: 'unread.counts', counts });
}
