// Accounts: hello (token, sign-in, sign-up), devices, lockouts, passwords
// (and old PINs), sign-out, one-time sign-in passes, OBS links, and the
// encrypted Personal space's key.
import { PIN_RE, deviceLabel, hashPin, needsRehash, newToken, passwordRule, tempPassword, tokenHash, verifyPin, weakPassword } from './auth';
import type { Attachment } from './attachment';
import { COLOR_RE, ClientError, cleanName, cleanVaultWrap, personalId } from './validate';
import { LIMITS, PROTOCOL_VERSION, type ClientMessage, type Device, type Role, type ServerMessage, type VaultWrap } from './protocol';
import type { HubHost } from './hub-host';
import { deliver, pushTargets } from './hub-pushes';

/** How long a one-time sign-in pass (device.handoff) stays good. */
const HANDOFF_MS = 15 * 60_000;

/**
 * users.weak_secret: 0 = fine, 1 = should be replaced (an old PIN, or a
 * password the rules now call guessable), 2 = a temporary password from a
 * reset, until its owner chooses their own. Anything but 0 → ready.weakPin.
 */
export const WEAK = { no: 0, yes: 1, temporary: 2 } as const;

/** A new password (sign-up, Change password): bad_pin or weak_pin, or fine. */
function checkNewPassword(secret: string, name: string) {
  const rule = passwordRule(secret);
  if (rule) throw new ClientError('bad_pin', rule);
  const weak = weakPassword(secret, name);
  if (weak) throw new ClientError('weak_pin', weak);
}

export async function onHello(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'hello' }>) {
  if (me.hello) throw new ClientError('already_hello', 'Already said hello.');
  if (msg.v !== PROTOCOL_VERSION) throw new ClientError('bad_version', `Server speaks protocol v${PROTOCOL_VERSION}.`);
  const client = cleanName(msg.client, 16) || 'unknown';
  const now = Date.now();
  let uid: string;
  let token: string | undefined; // only set when a new device token is issued

  if (typeof msg.token === 'string' && msg.token) {
    // ── Returning device ──
    const hash = await tokenHash(msg.token);
    const device = hub.sql
      .exec<{ user_id: string; scope: string | null; created_at: number; client: string | null; label: string | null }>(
        'SELECT user_id, scope, created_at, client, label FROM devices WHERE token_hash = ?', hash,
      )
      .toArray()[0];
    if (!device) throw new ClientError('bad_token', 'This device was signed out. Sign in with your name and password.');
    uid = hub.canonical(device.user_id);
    if (device.scope === 'handoff') {
      // A one-time pass (e.g. a new home-screen app): spend it, hand out a real token.
      hub.sql.exec('DELETE FROM devices WHERE token_hash = ?', hash);
      if (now - device.created_at > HANDOFF_MS) throw new ClientError('bad_token', 'That sign-in link expired. Sign in with your name and password.');
      ({ token, me } = await issueToken(hub, me, uid, client));
    }
    const listen = device.scope === 'listen';
    if (device.scope !== 'handoff') {
      // The label is the one Yap gave the device (nobody can rename one). When the device now says
      // it's something else (an iPad app that used to send "ios"), a default label follows it.
      const nowClient = listen ? 'obs' : client;
      const was = device.client ?? '';
      const isDefault = device.label === null || device.label === deviceLabel(me.ua ?? '', was) || device.label === deviceLabel('', was);
      const label = device.label === null || (was !== nowClient && isDefault) ? (listen ? 'OBS link' : deviceLabel(me.ua ?? '', client)) : device.label;
      hub.sql.exec('UPDATE devices SET last_seen = ?, client = ?, label = ? WHERE token_hash = ?', now, nowClient, label, hash);
      me = { ...me, tokenHash: hash, ...(listen ? { listen: true } : {}) };
    }
  } else if (msg.login) {
    // ── Sign in on a new device ──
    const name = cleanName(msg.login.name, LIMITS.nameChars);
    const pin = String(msg.login.pin ?? '');
    checkLockout(hub, name, me.ip ?? '');
    const row = hub.sql
      .exec<{ id: string; pin_hash: string | null; role: Role; secret_kind: string | null; weak_secret: number | null }>(
        `SELECT id, pin_hash, role, secret_kind, weak_secret FROM users WHERE lower(name) = lower(?) AND role != 'banned'
         AND id NOT IN (SELECT alias FROM user_aliases)`,
        name,
      )
      .toArray()[0];
    if (row && !row.pin_hash) {
      throw new ClientError('no_pin', `${name} hasn't set a password yet. Use a device that's already signed in to set one (Settings › Account), or ask the owner or a mod to reset it.`);
    }
    if (!row || !(await verifyPin(pin, row.pin_hash, hub.pepper))) {
      recordFailure(hub, name, me.ip ?? '');
      throw new ClientError('bad_login', 'Wrong name or password.');
    }
    hub.sql.exec('DELETE FROM login_failures WHERE name = ?', name.toLowerCase());
    if (needsRehash(row.pin_hash)) hub.sql.exec('UPDATE users SET pin_hash = ? WHERE id = ?', await hashPin(pin, hub.pepper), row.id);
    uid = row.id;
    // The only moment the server sees the secret: remember whether it should be replaced, so every
    // device can ask once (ready.weakPin). An old PIN always should; a temporary password stays
    // temporary until its owner chooses one; any other password is checked against today's rules.
    const weak = row.secret_kind !== 'password' ? WEAK.yes : row.weak_secret === WEAK.temporary ? WEAK.temporary : weakPassword(pin, name) ? WEAK.yes : WEAK.no;
    hub.sql.exec('UPDATE users SET weak_secret = ? WHERE id = ?', weak, uid);
    ({ token, me } = await issueToken(hub, me, uid, client));
    alertSignIn(hub, uid, me.tokenHash!);
  } else if (msg.signup) {
    // ── New account ──
    const name = cleanName(msg.signup.name, LIMITS.nameChars);
    if (!name) throw new ClientError('bad_name', 'Pick a name.');
    const secret = String(msg.signup.pin ?? '');
    const rule = passwordRule(secret);
    if (rule) throw new ClientError('bad_pin', rule);
    if (hub.nameTaken(name, '')) throw new ClientError('name_taken', `Someone here is already called ${name}. If that's you, choose "I have an account" and sign in with your password.`);
    checkNewPassword(secret, name);
    if (hub.hasMembers() && hub.pendingUsers().length >= 20) throw new ClientError('busy', 'Too many people are waiting to join right now. Try again later.');
    limitSignups(hub, me);
    uid = crypto.randomUUID().replace(/-/g, '');
    const color = COLOR_RE.test(msg.signup.color ?? '') ? msg.signup.color!.toLowerCase() : '#5b8def';
    // The very first person on a brand-new server becomes its owner.
    const role: Role = hub.hasMembers() ? 'pending' : 'owner';
    hub.sql.exec(
      "INSERT INTO users (id, name, color, created_at, last_seen, role, pin_hash, secret_kind, weak_secret) VALUES (?, ?, ?, ?, ?, ?, ?, 'password', 0)",
      uid, name, color, now, now, role, await hashPin(secret, hub.pepper),
    );
    if (me.invite) {
      hub.sql.exec('UPDATE users SET invite_id = ? WHERE id = ?', me.invite, uid);
      hub.sql.exec('UPDATE invites SET uses = uses + 1, last_used_at = ? WHERE id = ?', now, me.invite);
    }
    ({ token, me } = await issueToken(hub, me, uid, client));
  } else {
    // No credentials (no token, login or signup), like a device from before
    // PINs that only knows its uid: uids are visible to members, so they
    // prove nothing. Every client sends people to sign-in on this code.
    throw new ClientError('pin_required', 'Sign in with your name and password. No password yet? Ask the owner or a mod to reset it for you.');
  }

  const user = hub.user(uid);
  if (!user || user.role === 'banned') throw new ClientError('banned', "You can't join this server.");
  hub.sql.exec('UPDATE users SET last_seen = ? WHERE id = ?', now, uid);
  me = { ...me, rk: await hub.resumeKey(me.sid, uid) };
  if (user.role === 'pending') {
    ws.serializeAttachment({ ...me, uid, client, pending: true });
    hub.send(ws, { t: 'pending', message: WAITING, uid, token });
    // Only the owner and mods let people in, so only they hear about it.
    const waiting = hub.pendingUsers().find((u) => u.id === uid) ?? user;
    toMods(hub, { t: 'join.request', user: waiting });
    // The owner and mods with pushes on get one (deliver() keeps each one's quiet hours), at most
    // once every 2 minutes, so a leaked link can't make phones buzz nonstop.
    const targets = msg.signup ? pushTargets(hub).filter((p) => hub.isMod(p.id)) : [];
    if (targets.length && Date.now() - hub.lastJoinPush > 120_000) {
      hub.lastJoinPush = Date.now();
      for (const t of targets) {
        deliver(hub, t, {
          title: `🚪 ${user.name} is waiting to join`,
          message: `${waiting.invitedBy ? `Invited by ${waiting.invitedBy}. ` : ''}Open Yap to let them in or turn them away.`,
          tags: ['door'], priority: 4, tag: 'join',
        });
      }
    }
    return;
  }
  // OBS links connect as "obs": apps leave them out of the channel list and chimes.
  hub.admit(ws, { ...me, uid, client: me.listen ? 'obs' : client }, token);
}

async function issueToken(hub: HubHost, me: Attachment, uid: string, client: string) {
  const token = newToken();
  const hash = await tokenHash(token);
  const now = Date.now();
  hub.sql.exec('INSERT INTO devices (token_hash, user_id, client, created_at, last_seen, label) VALUES (?, ?, ?, ?, ?, ?)', hash, uid, client, now, now, deviceLabel(me.ua ?? '', client));
  return { token, me: { ...me, tokenHash: hash } };
}

// ── Your devices (Settings › Account) ──────────────────────────────────

export function devicesOf(hub: HubHost, me: Attachment): Device[] {
  return hub.sql
    .exec<{ token_hash: string; client: string; label: string | null; created_at: number; last_seen: number; scope: string | null }>(
      "SELECT token_hash, client, label, created_at, last_seen, scope FROM devices WHERE user_id = ? AND (scope IS NULL OR scope != 'handoff') ORDER BY last_seen DESC",
      me.uid,
    )
    .toArray()
    .map((r) => ({
      id: r.token_hash.slice(0, 16),
      client: r.client,
      label: r.label ?? (r.scope === 'listen' ? 'OBS link' : deviceLabel('', r.client)),
      createdAt: r.created_at,
      lastSeen: r.last_seen,
      scope: r.scope,
      ...(r.token_hash === me.tokenHash ? { current: true as const } : {}),
    }));
}

/** Close every live socket signed in with one of these device tokens. */
function kickDevices(hub: HubHost, hashes: Set<string>) {
  for (const ws of hub.ctx.getWebSockets()) {
    const a = ws.deserializeAttachment() as Attachment | null;
    if (a?.tokenHash && hashes.has(a.tokenHash)) hub.kick(ws, 4001, 'signed out');
  }
}

export function onRevokeDevice(hub: HubHost, ws: WebSocket, me: Attachment, id: string) {
  if (!/^[0-9a-f]{16}$/.test(id)) throw new ClientError('no_device', 'No such device.');
  const rows = hub.sql
    .exec<{ token_hash: string }>('SELECT token_hash FROM devices WHERE user_id = ? AND substr(token_hash, 1, 16) = ?', me.uid, id)
    .toArray()
    .filter((r) => r.token_hash !== me.tokenHash);
  if (!rows.length) throw new ClientError('no_device', 'No such device (to sign out this one, use Sign out).');
  for (const r of rows) hub.sql.exec('DELETE FROM devices WHERE token_hash = ?', r.token_hash);
  kickDevices(hub, new Set(rows.map((r) => r.token_hash)));
  hub.send(ws, { t: 'devices', devices: devicesOf(hub, me) });
}

/** "Not you?": every other device (OBS links too) is signed out at once. */
export function onSignOutOthers(hub: HubHost, ws: WebSocket, me: Attachment) {
  const rows = hub.sql
    .exec<{ token_hash: string }>('SELECT token_hash FROM devices WHERE user_id = ? AND token_hash != ?', me.uid, me.tokenHash ?? '')
    .toArray();
  for (const r of rows) hub.sql.exec('DELETE FROM devices WHERE token_hash = ?', r.token_hash);
  kickDevices(hub, new Set(rows.map((r) => r.token_hash)));
  hub.send(ws, { t: 'devices', devices: devicesOf(hub, me) });
}

/** A new device signed in with your name + password: tell your other devices (and your phone). */
function alertSignIn(hub: HubHost, uid: string, hash: string) {
  const row = hub.sql
    .exec<{ client: string; label: string | null; created_at: number; last_seen: number }>('SELECT client, label, created_at, last_seen FROM devices WHERE token_hash = ?', hash)
    .toArray()[0];
  if (!row) return;
  const device: Device = { id: hash.slice(0, 16), client: row.client, label: row.label ?? row.client, createdAt: row.created_at, lastSeen: row.last_seen, scope: null };
  for (const ws of hub.sockets()) {
    const a = ws.deserializeAttachment() as Attachment;
    if (a.uid === uid && a.tokenHash !== hash && !a.listen) hub.send(ws, { t: 'signin.alert', device });
  }
  const target = pushTargets(hub).find((t) => t.id === uid);
  if (target) {
    deliver(hub, target, { title: '🔐 New sign-in to your Yap account', message: `${device.label}, just now. Not you? Open Yap › Settings › Account › Sign out everywhere else, then change your password.`, tags: ['lock'], priority: 4, tag: `signin-${device.id}` }, 'security');
  }
}

/** Lockout: 5 free tries per name, then waits that double (1 min → … → 24 h); plus 20 misses/hour per IP. */
function checkLockout(hub: HubHost, name: string, ip: string) {
  const row = hub.sql.exec<{ locked_until: number }>('SELECT locked_until FROM login_failures WHERE name = ?', name.toLowerCase()).toArray()[0];
  const wait = (row?.locked_until ?? 0) - Date.now();
  if (wait > 0) throw new ClientError('locked', `Too many wrong tries. Try again in ${Math.ceil(wait / 60_000)} min.`);
  const b = hub.peek(`ip:${ip || 'unknown'}`, 'login');
  if (b && b.n < 1 && Date.now() - b.at < 3_600_000) throw new ClientError('locked', 'Too many wrong tries from here. Try again later.');
}

function recordFailure(hub: HubHost, name: string, ip: string) {
  const key = name.toLowerCase();
  const row = hub.sql.exec<{ failures: number }>('SELECT failures FROM login_failures WHERE name = ?', key).toArray()[0];
  const failures = (row?.failures ?? 0) + 1;
  const lockMs = failures < 5 ? 0 : Math.min(86_400_000, 60_000 * 2 ** (failures - 5));
  hub.sql.exec(
    `INSERT INTO login_failures (name, failures, locked_until) VALUES (?, ?, ?)
     ON CONFLICT(name) DO UPDATE SET failures = excluded.failures, locked_until = excluded.locked_until`,
    key, failures, Date.now() + lockMs,
  );
  hub.allow(`ip:${ip || 'unknown'}`, 'login', 20, 20 / 3600);
}

/** At most ~20 new accounts per IP per hour (local dev/tests exempt). */
function limitSignups(hub: HubHost, me: Attachment) {
  const ip = me.ip ?? '';
  if (!ip || ip === '127.0.0.1' || ip === '::1') return;
  if (!hub.allow(`ip:${ip}`, 'signup', 20, 20 / 3600)) throw new ClientError('slow_down', 'Too many new accounts from here. Try again later.');
}

/**
 * Set or change your password. Changing needs the current one (your password,
 * or your old PIN). The new one is always a password: 8–128 characters, not
 * guessable.
 */
export async function onSetPin(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'account.pin' }>) {
  const pin = String(msg.pin ?? '');
  checkNewPassword(pin, hub.user(me.uid)?.name ?? '');
  hub.limit(me.sid, 'pin', 5, 1 / 60);
  const row = hub.sql.exec<{ pin_hash: string | null; secret_kind: string | null }>('SELECT pin_hash, secret_kind FROM users WHERE id = ?', me.uid).toArray()[0];
  if (row?.pin_hash && !(await verifyPin(String(msg.current ?? ''), row.pin_hash, hub.pepper))) {
    throw new ClientError('bad_login', row.secret_kind === 'password' ? 'Your current password is wrong.' : 'Your current password or PIN is wrong.');
  }
  hub.sql.exec("UPDATE users SET pin_hash = ?, secret_kind = 'password', weak_secret = ? WHERE id = ?", await hashPin(pin, hub.pepper), WEAK.no, me.uid);
  // (`account.token` stays in the protocol, but every signed-in connection
  // already has a device token, so there's never a new one to hand out.)
  hub.send(ws, { t: 'account', hasPin: true, kind: 'password' });
}

export function onSignOut(hub: HubHost, ws: WebSocket, me: Attachment) {
  if (me.tokenHash) hub.sql.exec('DELETE FROM devices WHERE token_hash = ?', me.tokenHash);
  hub.kick(ws, 4001, 'signed out');
}

/** Owner/mod: give someone a temporary password (they forgot theirs, or never set one). */
export async function onResetPin(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'member.resetPin' }>) {
  if (!hub.isMod(me.uid)) throw new ClientError('forbidden', 'Only the owner and mods can reset passwords.');
  const target = hub.user(hub.canonical(String(msg.uid)));
  if (!target || target.role === 'banned' || target.role === 'pending') throw new ClientError('no_user', 'No such member.');
  if (target.role === 'owner' && target.id !== me.uid) throw new ClientError('forbidden', "Can't reset the owner's password.");
  const pin = await setTemporary(hub, target);
  hub.log(me.uid, 'member.resetPin', target.id, null);
  hub.send(ws, { t: 'pin.reset', uid: target.id, pin });
}

/** Admin API twin of member.resetPin. */
export async function resetPin(hub: HubHost, uid: string) {
  const target = hub.user(hub.canonical(uid));
  if (!target) return { ok: false, message: 'Unknown user id.' };
  const pin = await setTemporary(hub, target);
  hub.log('admin', 'member.resetPin', target.id, null);
  return { ok: true, name: target.name, pin };
}

/** A temporary password replaces whatever they had (and clears any lockout); it asks to be replaced after sign-in. */
async function setTemporary(hub: HubHost, target: { id: string; name: string }) {
  const password = tempPassword();
  hub.sql.exec("UPDATE users SET pin_hash = ?, secret_kind = 'password', weak_secret = ? WHERE id = ?", await hashPin(password, hub.pepper), WEAK.temporary, target.id);
  hub.sql.exec('DELETE FROM login_failures WHERE name = ?', target.name.toLowerCase());
  return password;
}

/**
 * Tests only (`TEST_PINS=1`, never on a real server): give an account an old
 * 6-digit PIN, like the ones made before 2026-10-11, since sign-up won't make
 * them any more. Admin API: POST /api/admin/test-pin?uid=&pin=.
 */
export async function setTestPin(hub: HubHost, uid: string, pin: string) {
  if (hub.env.TEST_PINS !== '1') return { ok: false, message: 'Not on this server.' };
  const target = hub.user(hub.canonical(uid));
  if (!target) return { ok: false, message: 'Unknown user id.' };
  if (!PIN_RE.test(pin)) return { ok: false, message: 'A PIN is 6 digits.' };
  hub.sql.exec("UPDATE users SET pin_hash = ?, secret_kind = 'pin', weak_secret = ? WHERE id = ?", await hashPin(pin, hub.pepper), WEAK.yes, target.id);
  hub.sql.exec('DELETE FROM login_failures WHERE name = ?', target.name.toLowerCase());
  return { ok: true, name: target.name };
}

/** One message to the owner's and mods' connections (join requests). */
export function toMods(hub: HubHost, msg: ServerMessage) {
  const mods = new Set(
    hub.sql.exec<{ id: string }>("SELECT id FROM users WHERE role IN ('owner', 'mod')").toArray().map((r) => r.id),
  );
  hub.sendWhere(msg, (a) => !a.listen && mods.has(a.uid));
}

/** What a newcomer in the waiting room reads (pending.message). */
export const WAITING = 'Waiting for the owner or a mod to let you in.';

// ── Sign-in passes for a new home-screen app or device ────────────────
// A device row with scope 'handoff': good once, for 15 minutes. Expired
// ones are cleaned up whenever a new one is made.

export async function onHandoff(hub: HubHost, ws: WebSocket, me: Attachment) {
  const now = Date.now();
  hub.sql.exec("DELETE FROM devices WHERE scope = 'handoff' AND created_at < ?", now - HANDOFF_MS);
  const token = newToken();
  hub.sql.exec(
    "INSERT INTO devices (token_hash, user_id, client, created_at, last_seen, scope) VALUES (?, ?, 'handoff', ?, ?, 'handoff')",
    await tokenHash(token), me.uid, now, now,
  );
  hub.send(ws, { t: 'device.handoff', token });
}

// ── OBS links (listen-only devices) ────────────────────────────────────

export async function onObsCreate(hub: HubHost, ws: WebSocket, me: Attachment) {
  const token = newToken();
  const now = Date.now();
  hub.sql.exec(
    "INSERT INTO devices (token_hash, user_id, client, created_at, last_seen, scope) VALUES (?, ?, 'obs', ?, ?, 'listen')",
    await tokenHash(token), me.uid, now, now,
  );
  hub.log(me.uid, 'obs.create', null, null);
  hub.send(ws, { t: 'obs.created', token });
}

export function onObsRevoke(hub: HubHost, ws: WebSocket, me: Attachment) {
  const hashes = hub.sql
    .exec<{ token_hash: string }>("SELECT token_hash FROM devices WHERE user_id = ? AND scope = 'listen'", me.uid)
    .toArray()
    .map((r) => r.token_hash);
  hub.sql.exec("DELETE FROM devices WHERE user_id = ? AND scope = 'listen'", me.uid);
  for (const other of hub.ctx.getWebSockets()) {
    const a = other.deserializeAttachment() as Attachment;
    if (a.listen && a.tokenHash && hashes.includes(a.tokenHash)) hub.kick(other, 4003, 'revoked');
  }
  if (hashes.length) hub.log(me.uid, 'obs.revoke', null, String(hashes.length));
  hub.send(ws, { t: 'obs.revoked', count: hashes.length });
}

/** Your wrapped space key, or null (Personal space not encrypted). */
export function vaultOf(hub: HubHost, uid: string): VaultWrap | null {
  const raw = hub.sql.exec<{ space_wrap: string | null }>('SELECT space_wrap FROM users WHERE id = ?', uid).toArray()[0]?.space_wrap;
  return raw ? (JSON.parse(raw) as VaultWrap) : null;
}

/**
 * Store (or replace / remove) your wrapped Personal space key. The browser
 * did the wrapping; we only check its shape. All your devices hear about it.
 */
export function onVaultSet(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'vault.set' }>) {
  const wrap = msg.wrap === null ? null : cleanVaultWrap(msg.wrap);
  if (msg.wrap !== null && !wrap) throw new ClientError('bad_vault', 'That key looks broken.');
  hub.sql.exec('UPDATE users SET space_wrap = ? WHERE id = ?', wrap ? JSON.stringify(wrap) : null, me.uid);
  hub.toOwner(personalId(me.uid), { t: 'vault', wrap });
}
