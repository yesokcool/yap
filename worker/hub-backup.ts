// Backups and restores (PROTOCOL.md › Backups): the full export the
// owner's tools page through, and rebuilding an empty server from one.
import { toMessage, type MessageRow } from './rows';
import { MIGRATIONS, eraseDeletedPosts } from './schema';
import type { HubHost } from './hub-host';
import { armHangs } from './hub-extras';

/**
 * Everything, including archived channels, deleted messages and deleted
 * emojis — this is the backup. Messages are paged by ts.
 */
export async function exportPage(hub: HubHost, afterTs: number, limit: number) {
  const messages = hub.sql
    .exec<MessageRow & { deleted_at: number | null }>(
      'SELECT id, channel_id, user_id, body, ts, gif, image, edited_at, deleted_at, reply_to, pinned_at, pinned_by, extra FROM messages WHERE ts > ? ORDER BY ts LIMIT ?',
      afterTs, limit,
    )
    .toArray()
    .map((r) => ({ ...toMessage(r), deletedAt: r.deleted_at, ...(r.reply_to ? { replyTo: r.reply_to } : {}), ...(r.pinned_by ? { pinnedBy: r.pinned_by } : {}), ...(r.extra ? { extra: r.extra } : {}) }));
  const first = afterTs <= 0;
  const all = (sql: string) => (first ? hub.sql.exec(sql).toArray() : undefined);
  return {
    schema: Number(hub.sql.exec<{ value: string }>("SELECT value FROM meta WHERE key = 'schema'").one().value),
    channels: all('SELECT * FROM channels ORDER BY kind, position'),
    aliases: all('SELECT * FROM user_aliases ORDER BY created_at'),
    users: all('SELECT * FROM users ORDER BY created_at'),
    emojis: all('SELECT * FROM emojis ORDER BY created_at'),
    // Everything else, so a backup can rebuild the server on its own (tools/restore.mjs).
    reactions: all('SELECT * FROM reactions ORDER BY ts'),
    edits: all('SELECT * FROM message_edits ORDER BY edited_at'),
    strokes: all('SELECT * FROM strokes ORDER BY ts'),
    drawings: all('SELECT * FROM drawings ORDER BY ts'),
    pollVotes: all('SELECT * FROM poll_votes ORDER BY ts'),
    sounds: all('SELECT * FROM sounds ORDER BY created_at'),
    hangs: all('SELECT * FROM hangs ORDER BY created_at'),
    invites: all('SELECT * FROM invites ORDER BY created_at'),
    links: all('SELECT * FROM links ORDER BY position'),
    modLog: all('SELECT * FROM mod_log ORDER BY id'),
    devices: all('SELECT * FROM devices ORDER BY created_at'),
    meta: all("SELECT * FROM meta WHERE key IN ('canvas')"),
    messages,
    next: messages.length === limit ? messages[messages.length - 1].ts : null,
  };
}

/**
 * Disaster recovery: rebuild this server from a backup file, in three
 * steps (tools/restore.mjs drives them). Only ever into an EMPTY server
 * (no users, no messages), so it can't overwrite anyone's real data.
 *   begin    — every table but messages (replaces the seeded channels/links)
 *   messages — a page of messages, as exported
 *   finish   — done; everyone reconnects
 */
export async function importBackup(hub: HubHost, part: { phase: string; data?: Record<string, unknown>; messages?: Record<string, unknown>[] }): Promise<{ ok: boolean; message?: string; count?: number }> {
  const importing = hub.sql.exec("SELECT 1 FROM meta WHERE key = 'importing'").toArray().length > 0;
  const users = hub.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM users').one().n;
  const messages = hub.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM messages').one().n;
  const occupied = !importing && (users > 0 || messages > 0);
  if (occupied && (part.phase === 'check' || part.phase === 'begin')) {
    return { ok: false, message: 'This server already has people or messages. Restore only into a fresh, empty server.' };
  }
  if (part.phase === 'check') return { ok: true };
  if (part.phase === 'begin') {
    const data = part.data ?? {};
    if (Number(data.schema) > MIGRATIONS.length) return { ok: false, message: `Backup is schema ${data.schema}; this server only knows ${MIGRATIONS.length}. Deploy newer code first.` };
    hub.sql.exec('DELETE FROM channels');
    hub.sql.exec('DELETE FROM links');
    let count = 0;
    for (const [key, table] of [['channels', 'channels'], ['aliases', 'user_aliases'], ['users', 'users'], ['emojis', 'emojis'], ['reactions', 'reactions'], ['edits', 'message_edits'], ['strokes', 'strokes'], ['drawings', 'drawings'], ['pollVotes', 'poll_votes'], ['sounds', 'sounds'], ['hangs', 'hangs'], ['invites', 'invites'], ['links', 'links'], ['modLog', 'mod_log'], ['devices', 'devices'], ['meta', 'meta']] as const) {
      count += insertRows(hub, table, (data[key] as Record<string, unknown>[] | undefined) ?? []);
    }
    hub.sql.exec("INSERT INTO meta (key, value) VALUES ('importing', '1') ON CONFLICT(key) DO UPDATE SET value = '1'");
    hub.aliases = null;
    return { ok: true, count };
  }
  if (!importing) return { ok: false, message: 'Send phase "begin" first.' };
  if (part.phase === 'messages') {
    const rows = (part.messages ?? []).map((m) => ({
      id: m.id, channel_id: m.channelId, user_id: m.uid, body: m.body ?? '', ts: m.ts,
      gif: m.gif ? JSON.stringify(m.gif) : null, image: m.image ? JSON.stringify(m.image) : null, edited_at: m.editedAt ?? null, deleted_at: m.deletedAt ?? null, reply_to: m.replyTo ?? null,
      pinned_at: m.pinnedAt ?? null, pinned_by: m.pinnedBy ?? null, extra: m.extra ?? null,
    }));
    const count = insertRows(hub, 'messages', rows);
    // Deleted posts stay erased, even from a backup taken before they were (schema < 15).
    eraseDeletedPosts(hub.sql);
    return { ok: true, count };
  }
  if (part.phase === 'finish') {
    eraseDeletedPosts(hub.sql);
    hub.sql.exec("DELETE FROM meta WHERE key = 'importing'");
    hub.lastTs = 0;
    hub.aliases = null;
    await armHangs(hub);
    hub.ctx.waitUntil(hub.emptyTrash());
    hub.reconnectEveryone();
    return { ok: true };
  }
  return { ok: false, message: 'Unknown phase.' };
}

/** INSERT OR IGNORE rows, keeping only columns this table really has. */
function insertRows(hub: HubHost, table: string, rows: Record<string, unknown>[]): number {
  if (!rows.length) return 0;
  const columns = new Set(hub.sql.exec<{ name: string }>(`SELECT name FROM pragma_table_info('${table}')`).toArray().map((c) => c.name));
  let n = 0;
  for (const row of rows) {
    const keys = Object.keys(row).filter((k) => columns.has(k));
    if (!keys.length) continue;
    hub.sql.exec(`INSERT OR IGNORE INTO ${table} (${keys.join(', ')}) VALUES (${keys.map(() => '?').join(', ')})`, ...keys.map((k) => row[k] as SqlStorageValue));
    n++;
  }
  return n;
}
