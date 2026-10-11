// Pushes and pings: notification settings and quiet hours, ntfy and browser
// pushes, @mention and "all" pushes, pings, and away-message replies.
import { NOTIFY_MODES, newTopic, publish } from './notify';
import { plainText } from '../web/js/format.js';
import { sendWebPush } from './webpush';
import type { Attachment } from './attachment';
import { MEMBER_ROLES } from './rows';
import { cleanQuiet, isQuietNow } from './extras';
import { ClientError, cleanName } from './validate';
import type { ClientMessage, Message, NotifyMode, QuietHours } from './protocol';
import type { HubHost } from './hub-host';

/** People you just mentioned who are away: tell you (only you), once per person per 10 min. */
export function awayReplies(hub: HubHost, ws: WebSocket, me: Attachment, uids: string[]) {
  for (const uid of uids) {
    if (uid === me.uid) continue;
    const away = hub.user(uid)?.away;
    if (!away || !hub.allow(`${me.uid}>${uid}`, 'away', 1, 1 / 600)) continue;
    hub.send(ws, { t: 'away', uid, text: away.text, until: away.until });
  }
}

/** Posting 2+ minutes after going away means you're back. */
export function backFromAway(hub: HubHost, uid: string) {
  const away = hub.user(uid)?.away;
  if (!away || Date.now() - away.since < 120_000) return;
  hub.sql.exec('UPDATE users SET away = NULL WHERE id = ?', uid);
  hub.broadcast({ t: 'user', user: hub.user(uid)! });
}

// ── Phone pushes (ntfy) and pings ──────────────────────────────────────

export function notifySettings(hub: HubHost, uid: string): { mode: NotifyMode; topic: string | null; quiet: QuietHours | null } {
  const row = hub.sql.exec<{ ntfy_mode: NotifyMode; ntfy_topic: string | null; quiet: string | null }>('SELECT ntfy_mode, ntfy_topic, quiet FROM users WHERE id = ?', uid).toArray()[0];
  return { mode: row?.ntfy_mode ?? 'off', topic: row?.ntfy_topic ?? null, quiet: row?.quiet ? (JSON.parse(row.quiet) as QuietHours) : null };
}

export function onQuiet(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'notify.quiet' }>) {
  const quiet = msg.quiet === null ? null : cleanQuiet(msg.quiet);
  hub.sql.exec('UPDATE users SET quiet = ? WHERE id = ?', quiet ? JSON.stringify(quiet) : null, me.uid);
  // Every one of your devices shows the same settings.
  for (const other of hub.sockets()) {
    if ((other.deserializeAttachment() as Attachment).uid === me.uid) hub.send(other, { t: 'notify', ...notifySettings(hub, me.uid) });
  }
}

export function onNotifySettings(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'notify.settings' }>) {
  const mode = NOTIFY_MODES.includes(msg.mode) ? msg.mode : 'off';
  const current = notifySettings(hub, me.uid);
  // Keep the same topic across off/on so a phone stays subscribed.
  const topic = current.topic ?? (mode === 'off' ? null : newTopic());
  hub.sql.exec('UPDATE users SET ntfy_mode = ?, ntfy_topic = ? WHERE id = ?', mode, topic, me.uid);
  hub.send(ws, { t: 'notify', mode, topic, quiet: notifySettings(hub, me.uid).quiet });
}

/** Users who'd get a push: members with pushes on and somewhere to send them (ntfy and/or browsers). */
export function pushTargets(hub: HubHost): { id: string; name: string; topic: string | null; mode: NotifyMode }[] {
  return hub.sql
    .exec<{ id: string; name: string; ntfy_topic: string | null; ntfy_mode: NotifyMode }>(
      `SELECT id, name, ntfy_topic, ntfy_mode FROM users
       WHERE role IN ${MEMBER_ROLES} AND ntfy_mode != 'off'
       AND (ntfy_topic IS NOT NULL OR id IN (SELECT user_id FROM push_subs))
       AND id NOT IN (SELECT alias FROM user_aliases)`,
    )
    .toArray()
    .map((r) => ({ id: r.id, name: r.name, topic: r.ntfy_topic, mode: r.ntfy_mode }));
}

/**
 * One push to one person: their ntfy topic and every browser they subscribed.
 * Held back during their quiet hours — except sign-in alerts, and mentions
 * if they let those through.
 */
export function deliver(hub: HubHost, target: { id: string; topic: string | null }, push: { title: string; message: string; tags?: string[]; priority?: 1 | 2 | 3 | 4 | 5; tag?: string }, kind: 'normal' | 'mention' | 'security' = 'normal') {
  if (kind !== 'security') {
    const quiet = notifySettings(hub, target.id).quiet;
    if (quiet && isQuietNow(quiet) && !(kind === 'mention' && quiet.mentions)) return;
  }
  if (target.topic) hub.ctx.waitUntil(publish(hub.env, { topic: target.topic, title: push.title, message: push.message, tags: push.tags, priority: push.priority, click: hub.publicUrl() }));
  const jwk = hub.env.VAPID_PRIVATE_JWK;
  if (!jwk) return;
  const subs = hub.sql.exec<{ endpoint: string; p256dh: string; auth: string }>('SELECT endpoint, p256dh, auth FROM push_subs WHERE user_id = ?', target.id).toArray();
  for (const sub of subs) {
    hub.ctx.waitUntil((async () => {
      try {
        const status = await sendWebPush(sub, { title: push.title, body: push.message, tag: push.tag ?? 'yap' }, jwk, hub.publicUrl());
        // Gone (unsubscribed, browser reset): forget it.
        if (status === 404 || status === 410) hub.sql.exec('DELETE FROM push_subs WHERE endpoint = ?', sub.endpoint);
        else if (status >= 400) console.warn('web push failed', status, new URL(sub.endpoint).host);
      } catch (err) {
        console.warn('web push error', err);
      }
    })());
  }
}

export function onPushSubscribe(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'push.subscribe' }>) {
  const endpoint = String(msg.endpoint ?? '');
  const p256dh = String(msg.keys?.p256dh ?? '');
  const auth = String(msg.keys?.auth ?? '');
  let host = '';
  try {
    const url = new URL(endpoint);
    // https only (plain http just for a test push service on this machine).
    host = url.protocol === 'https:' || (url.protocol === 'http:' && url.hostname === '127.0.0.1') ? url.host : '';
  } catch {}
  if (!host || endpoint.length > 600 || !/^[A-Za-z0-9_-]{80,100}$/.test(p256dh) || !/^[A-Za-z0-9_-]{16,32}$/.test(auth)) {
    throw new ClientError('bad_subscription', 'That push subscription looks broken.');
  }
  // Pages send their subscription again on every connect; only a new one is "turning browser pushes on".
  const known = hub.sql.exec('SELECT 1 FROM push_subs WHERE endpoint = ? AND user_id = ?', endpoint, me.uid).toArray().length > 0;
  hub.sql.exec(
    'INSERT INTO push_subs (endpoint, user_id, p256dh, auth, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(endpoint) DO UPDATE SET user_id = excluded.user_id, p256dh = excluded.p256dh, auth = excluded.auth',
    endpoint, me.uid, p256dh, auth, Date.now(),
  );
  // Turning on browser pushes with pushes off means "pings and mentions".
  // (A re-sent subscription leaves Off alone: it means Off.)
  if (!known && notifySettings(hub, me.uid).mode === 'off') hub.sql.exec("UPDATE users SET ntfy_mode = 'pings' WHERE id = ?", me.uid);
  hub.send(ws, { t: 'notify', ...notifySettings(hub, me.uid) });
}

function onlineUids(hub: HubHost): Set<string> {
  return new Set(hub.sockets().map((ws) => (ws.deserializeAttachment() as Attachment).uid));
}

export function onPing(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'ping' }>) {
  const text = cleanName(msg.text ?? '', 140);
  const from = hub.user(me.uid)!;
  hub.broadcast({ t: 'ping', from: me.uid, text });
  for (const t of pushTargets(hub)) {
    if (t.id === me.uid) continue;
    deliver(hub, t, { title: `📣 ${from.name} pinged everyone`, message: text || 'Hop on Yap!', tags: ['loudspeaker'], priority: 4, tag: 'ping' }, 'mention');
  }
}

/**
 * @mentions and @everyone push to people with pushes on; "all" mode also
 * pushes every message to people who aren't connected (at most one per 20 s).
 */
export function pushForMessage(hub: HubHost, me: Attachment, channelName: string, message: Message, mentions: { uids: string[]; everyone: boolean }) {
  const targets = pushTargets(hub);
  if (!targets.length) return;
  const author = hub.user(me.uid)!;
  // The same plain text as the apps' own notifications: no formatting marks, spoilers hidden, :emoji: names.
  const text = plainText(message.poll ? `📊 ${message.body}` : message.body || (message.image ? '📷 A picture' : message.gif ? `GIF: ${message.gif.title}` : ''));
  const online = onlineUids(hub);
  const now = Date.now();
  for (const t of targets) {
    if (t.id === me.uid) continue;
    const mentioned = mentions.everyone || mentions.uids.includes(t.id);
    if (mentioned) {
      deliver(hub, t, { title: `${author.name} mentioned ${mentions.uids.includes(t.id) ? 'you' : 'everyone'} in ${channelName}`, message: text, tags: ['speech_balloon'], priority: 4, tag: `mention-${message.channelId}` }, 'mention');
    } else if (t.mode === 'all' && !online.has(t.id) && now - (hub.lastPush.get(t.id) ?? 0) > 20_000) {
      hub.lastPush.set(t.id, now);
      deliver(hub, t, { title: `${author.name} in ${channelName}`, message: text, priority: 3, tag: `msg-${message.channelId}` });
    }
  }
}
