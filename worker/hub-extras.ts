// Extras (PROTOCOL.md › Extras): the soundboard and uploaded chimes, hang
// reminders (and the alarm that starts them), and When's good.
import type { Attachment } from './attachment';
import { CUSTOM_EMOJI_RE, ClientError, cleanName, isEmoji } from './validate';
import { LIMITS, type ClientMessage, type Hang, type Sound } from './protocol';
import type { HubHost } from './hub-host';
import { deliver, pushTargets } from './hub-pushes';

/** When's good slots are half hours since the epoch. Kept: start of last week (UTC Monday) → two weeks ahead. */
const SLOT_MS = 1_800_000;

function whenWindow(now = Date.now()): [number, number] {
  const day = Math.floor(now / 86_400_000);
  const monday = day - ((day + 3) % 7); // 1970-01-01 was a Thursday
  return [((monday - 7) * 86_400_000) / SLOT_MS, ((monday + 21) * 86_400_000) / SLOT_MS];
}

export function sounds(hub: HubHost): Sound[] {
  // Board sounds, plus the chimes people actually use.
  return hub.sql
    .exec<{ id: string; kind: 'board' | 'chime'; name: string; emoji: string | null; media_id: string; gain: number; ms: number; user_id: string; created_at: number }>(
      `SELECT id, kind, name, emoji, media_id, gain, ms, user_id, created_at FROM sounds
       WHERE deleted_at IS NULL AND (kind = 'board' OR 's:' || id IN (SELECT chime FROM users WHERE chime IS NOT NULL))
       ORDER BY created_at`,
    )
    .toArray()
    .map((r) => ({ id: r.id, kind: r.kind, name: r.name, emoji: r.emoji, media: r.media_id, gain: r.gain, ms: r.ms, uid: hub.canonical(r.user_id), createdAt: r.created_at }));
}

/** Worker RPC after a clip was checked and stored (POST /api/sound). */
export async function addSound(hub: HubHost, input: { sid: string; ut: string; kind: string; name: string; emoji: string; media: string; gain: number; ms: number }): Promise<{ ok: boolean; sound?: Sound; message?: string }> {
  const ws = hub.uploader(input.sid, input.ut);
  if (!ws) return { ok: false, message: 'Connect first.' };
  const me = ws.deserializeAttachment() as Attachment;
  const kind = input.kind === 'chime' ? 'chime' : 'board';
  const name = kind === 'chime' ? (hub.user(me.uid)?.name ?? 'chime') : cleanName(input.name, 24);
  if (!name) return { ok: false, message: 'Give it a name (up to 24 characters).' };
  const emoji = input.emoji && isEmoji(input.emoji) && !CUSTOM_EMOJI_RE.test(input.emoji) ? input.emoji : null;
  if (kind === 'board') {
    const { n } = hub.sql.exec<{ n: number }>("SELECT COUNT(*) AS n FROM sounds WHERE kind = 'board' AND deleted_at IS NULL").one();
    if (n >= LIMITS.sounds) return { ok: false, message: `The soundboard is full (${LIMITS.sounds}). Delete one first.` };
  }
  const sound: Sound = { id: crypto.randomUUID(), kind, name, emoji, media: input.media, gain: input.gain, ms: input.ms, uid: me.uid, createdAt: Date.now() };
  hub.sql.exec(
    'INSERT INTO sounds (id, kind, name, emoji, media_id, gain, ms, user_id, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)',
    sound.id, kind, name, emoji, sound.media, sound.gain, sound.ms, me.uid, sound.createdAt,
  );
  hub.broadcast({ t: 'sound.added', sound });
  if (kind === 'chime') {
    hub.sql.exec('UPDATE users SET chime = ? WHERE id = ?', `s:${sound.id}`, me.uid);
    hub.broadcast({ t: 'user', user: hub.user(me.uid)! });
  }
  return { ok: true, sound };
}

export function onSoundPlay(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'sound.play' }>) {
  if (!me.vc) throw new ClientError('not_in_voice', 'Join a voice channel to play sounds.');
  const sound = hub.sql.exec<{ id: string }>("SELECT id FROM sounds WHERE id = ? AND kind = 'board' AND deleted_at IS NULL", String(msg.id)).toArray()[0];
  if (!sound) throw new ClientError('no_sound', 'That sound is gone.');
  if (!hub.allow(me.uid, 'sound', 2, 1 / 3) || !hub.allow(`vc:${me.vc}`, 'sound', 4, 0.4)) throw new ClientError('slow_down', 'Easy on the soundboard.');
  hub.sendWhere({ t: 'sound', id: sound.id, sid: me.sid, uid: me.uid }, (a) => a.vc === me.vc);
}

export function onSoundDelete(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'sound.delete' }>) {
  const row = hub.sql.exec<{ user_id: string }>('SELECT user_id FROM sounds WHERE id = ? AND deleted_at IS NULL', String(msg.id)).toArray()[0];
  if (!row) throw new ClientError('no_sound', 'That sound is gone.');
  if (hub.canonical(row.user_id) !== me.uid && !hub.isMod(me.uid)) throw new ClientError('forbidden', 'Only whoever added it (or a mod) can delete it.');
  hub.sql.exec('UPDATE sounds SET deleted_at = ? WHERE id = ?', Date.now(), String(msg.id));
  if (hub.canonical(row.user_id) !== me.uid) hub.log(me.uid, 'sound.delete', hub.canonical(row.user_id), null);
  hub.broadcast({ t: 'sound.deleted', id: String(msg.id) });
}

// ── Hang reminders (a When's good slot turned into a plan) ─────────────

export function hangs(hub: HubHost): Hang[] {
  return hub.sql
    .exec<{ id: string; at: number; title: string; channel_id: string | null; user_id: string; created_at: number }>(
      'SELECT id, at, title, channel_id, user_id, created_at FROM hangs WHERE cancelled_at IS NULL AND at > ? ORDER BY at LIMIT 10',
      Date.now() - 2 * 3_600_000,
    )
    .toArray()
    .map((r) => ({ id: r.id, at: r.at, title: r.title, channelId: r.channel_id, uid: hub.canonical(r.user_id), createdAt: r.created_at }));
}

export function onHangSet(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'hang.set' }>) {
  const at = Number(msg.at);
  const now = Date.now();
  const test = hub.env.TEST_HANGS === '1';
  if (!Number.isInteger(at) || (!test && at % SLOT_MS !== 0) || at < now + (test ? 1000 : 5 * 60_000) || at > now + 21 * 86_400_000) {
    throw new ClientError('bad_hang', 'Pick a half-hour slot between now and three weeks out.');
  }
  const title = cleanName(msg.title, LIMITS.hangTitleChars);
  if (!title) throw new ClientError('bad_hang', 'Give it a name (up to 80 characters).');
  const channelId = msg.channelId ? String(msg.channelId) : null;
  if (channelId && hub.channel(channelId)?.kind !== 'voice') throw new ClientError('no_channel', 'Pick a voice channel.');
  const upcoming = hub.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM hangs WHERE cancelled_at IS NULL AND at > ?', now).one().n;
  if (upcoming >= LIMITS.hangsUpcoming) throw new ClientError('too_many', `There are already ${LIMITS.hangsUpcoming} hangs planned.`);
  const hang: Hang = { id: crypto.randomUUID(), at, title, channelId, uid: me.uid, createdAt: now };
  hub.sql.exec('INSERT INTO hangs (id, at, title, channel_id, user_id, created_at) VALUES (?, ?, ?, ?, ?, ?)', hang.id, at, title, channelId, me.uid, now);
  hub.broadcast({ t: 'hang', hang });
  hub.ctx.waitUntil(armHangs(hub));
}

export function onHangCancel(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'hang.cancel' }>) {
  const row = hub.sql.exec<{ user_id: string }>('SELECT user_id FROM hangs WHERE id = ? AND cancelled_at IS NULL', String(msg.id)).toArray()[0];
  if (!row) throw new ClientError('no_hang', 'That hang is gone.');
  if (hub.canonical(row.user_id) !== me.uid && !hub.isMod(me.uid)) throw new ClientError('forbidden', 'Only whoever planned it (or a mod) can cancel it.');
  hub.sql.exec('UPDATE hangs SET cancelled_at = ? WHERE id = ?', Date.now(), String(msg.id));
  hub.broadcast({ t: 'hang.cancelled', id: String(msg.id) });
  hub.ctx.waitUntil(armHangs(hub));
}

/** Wake up when the next hang starts (one Durable Object alarm). */
export async function armHangs(hub: HubHost) {
  const next = hub.sql.exec<{ at: number | null }>('SELECT MIN(at) AS at FROM hangs WHERE cancelled_at IS NULL AND notified_at IS NULL').one().at;
  if (next === null) await hub.ctx.storage.deleteAlarm();
  else await hub.ctx.storage.setAlarm(Math.max(next, Date.now() + 1000));
}

/** Hangs starting now: tell everyone connected, and push once to whoever said they're free. */
export async function startDueHangs(hub: HubHost) {
  const now = Date.now();
  const due = hub.sql
    .exec<{ id: string; at: number; title: string; channel_id: string | null; user_id: string }>(
      'SELECT id, at, title, channel_id, user_id FROM hangs WHERE cancelled_at IS NULL AND notified_at IS NULL AND at <= ? ORDER BY at',
      now + 1000,
    )
    .toArray();
  for (const h of due) {
    hub.sql.exec('UPDATE hangs SET notified_at = ? WHERE id = ?', now, h.id);
    // Hours late (the object was asleep through a deploy?): skip the push, it would only confuse.
    if (now - h.at > 2 * 3_600_000) continue;
    hub.broadcast({ t: 'hang.now', id: h.id });
    const slot = Math.floor(h.at / SLOT_MS);
    const free = new Set(whenPeople(hub).filter((p) => p.slots.includes(slot)).map((p) => p.uid));
    free.add(hub.canonical(h.user_id));
    const where = h.channel_id ? hub.channel(h.channel_id)?.name : null;
    const targets = pushTargets(hub);
    const everyone = free.size <= 1;
    for (const t of targets) {
      if (!everyone && !free.has(t.id)) continue;
      deliver(hub, t, { title: `🗓 ${h.title} is starting${where ? ` · ${where}` : ''}`, message: 'Hop on Yap!', tags: ['calendar'], priority: 4, tag: `hang-${h.id}` });
    }
  }
  await armHangs(hub);
}

// ── When's good (a weekly when2meet) ───────────────────────────────────

function whenPeople(hub: HubHost): { uid: string; slots: number[] }[] {
  const [from, to] = whenWindow();
  const people: { uid: string; slots: number[] }[] = [];
  for (const r of hub.sql.exec<{ user_id: string; slots: string }>('SELECT user_id, slots FROM when_slots').toArray()) {
    const slots = (JSON.parse(r.slots) as number[]).filter((s) => s >= from && s < to);
    // Weekly clear: anything from before last week is gone for good.
    if (!slots.length) hub.sql.exec('DELETE FROM when_slots WHERE user_id = ?', r.user_id);
    else people.push({ uid: hub.canonical(r.user_id), slots });
  }
  return people;
}

export function onWhenGet(hub: HubHost, ws: WebSocket) {
  hub.send(ws, { t: 'when.all', people: whenPeople(hub) });
}

export function onWhenSet(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'when.set' }>) {
  if (!Array.isArray(msg.slots)) throw new ClientError('bad_slots', 'slots must be a list.');
  const [from, to] = whenWindow();
  const slots = [...new Set(msg.slots.filter((s) => Number.isInteger(s) && s >= from && s < to))].sort((a, b) => a - b);
  if (slots.length > LIMITS.whenSlots) throw new ClientError('too_many', 'That’s a lot of free time.');
  if (slots.length) {
    hub.sql.exec(
      'INSERT INTO when_slots (user_id, slots, updated_at) VALUES (?, ?, ?) ON CONFLICT(user_id) DO UPDATE SET slots = excluded.slots, updated_at = excluded.updated_at',
      me.uid, JSON.stringify(slots), Date.now(),
    );
  } else hub.sql.exec('DELETE FROM when_slots WHERE user_id = ?', me.uid);
  hub.broadcast({ t: 'when', uid: me.uid, slots });
}
