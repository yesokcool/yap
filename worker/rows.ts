// SQLite rows → the wire shapes in protocol.ts, and the column lists the
// Hub's queries share.
import type { Attachment } from './attachment.ts';
import type { Away, Channel, ChannelKind, Gif, ImageRef, Message, Role, Roll, Session, User } from './protocol.ts';

export const MEMBER_ROLES = "('owner', 'mod', 'member')";
/** The message columns clients see (MessageRow). */
export const MESSAGE_COLUMNS = 'id, channel_id, user_id, body, ts, gif, image, edited_at, reply_to, pinned_at, extra';

export type ChannelRow = { id: string; name: string; kind: ChannelKind; position: number; last_ts: number | null; bitrate?: number | null };
export type UserRow = { id: string; name: string; color: string; last_seen: number; role: Role; avatar: string | null; look: string | null; away?: string | null; chime?: string | null };
export type MessageRow = { id: string; channel_id: string; user_id: string; body: string; ts: number; gif: string | null; image?: string | null; edited_at?: number | null; reply_to?: string | null; pinned_at?: number | null; pinned_by?: string | null; extra?: string | null };
/** messages.extra: what a message is besides text (votes live in poll_votes). */
export type MessageExtra = { poll?: { options: string[] }; roll?: Roll };

export const toChannel = (r: ChannelRow): Channel => ({
  id: r.id, name: r.name, kind: r.kind, position: r.position, lastMessageAt: r.last_ts ?? null,
  ...(r.kind === 'voice' ? { bitrate: r.bitrate ?? null } : {}),
});
// lastSeen is always 0: Yap has no presence (PROTOCOL.md › Presence).
export const toUser = (r: UserRow): User => {
  const user: User = { id: r.id, name: r.name, color: r.color, lastSeen: 0, role: r.role ?? 'member', avatar: r.avatar ?? null, look: r.look ?? null };
  const away = r.away ? (JSON.parse(r.away) as Away) : null;
  if (away && (away.until === null || away.until > Date.now())) user.away = away;
  if (r.chime) user.chime = r.chime;
  return user;
};
export const toMessage = (r: MessageRow): Message => {
  const message: Message = { id: r.id, channelId: r.channel_id, uid: r.user_id, body: r.body, ts: r.ts };
  if (r.gif) message.gif = JSON.parse(r.gif) as Gif;
  if (r.image) message.image = JSON.parse(r.image) as ImageRef;
  if (r.edited_at) message.editedAt = r.edited_at;
  if (r.pinned_at) message.pinnedAt = r.pinned_at;
  if (r.extra) {
    const extra = JSON.parse(r.extra) as MessageExtra;
    if (extra.poll) message.poll = { options: extra.poll.options, votes: extra.poll.options.map(() => []) };
    if (extra.roll) message.roll = extra.roll;
  }
  return message;
};

/** Voice resume fields for `voice` broadcasts and Session.voice (absent for clients that don't resume). */
export const voiceResume = (a: Attachment): { resume?: true; prev?: string } => ({ ...(a.vr ? { resume: true } : {}), ...(a.vp ? { prev: a.vp } : {}) });
export const toSession = (a: Attachment): Session => ({
  sid: a.sid,
  uid: a.uid,
  client: a.client,
  voice: a.vc ? { channelId: a.vc, muted: a.muted, deafened: a.deafened, ...(a.screen ? { screen: true } : {}), ...voiceResume(a) } : null,
  view: a.view ?? null,
  ...(a.watching ? { watching: true } : {}),
});
