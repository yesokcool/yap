// Draw: the shared easel, its layers, the drawings on the bulletin board and
// their earlier versions (PROTOCOL.md › Draw layers, › Draw drawings and the board).
//
// A drawing is a page of strokes: its id is the strokes' `canvas`, and meta
// 'canvas' says which one is on the easel. Strokes are never erased (undo,
// wipe and going back only mark them deleted), so a version is just the ids
// of the strokes that were live then, and going back flips deleted marks.
import type { Attachment } from './attachment';
import { ClientError, cleanStroke } from './validate';
import { LIMITS, type ClientMessage, type DrawPage, type DrawVersion, type SavedDrawing, type ServerMessage, type Stroke } from './protocol';
import type { HubHost } from './hub-host';

/** While people draw, the page is kept as a version at most this often (before the stroke that comes after the wait). */
const AUTO_VERSION_MS = 3 * 60_000;
const BOARD_LIMIT = 200;
const DOWN_LIMIT = 100;
const HISTORY_LIMIT = 50;

type PageRow = {
  id: number; created_at: number; pinned_at: number | null; down_at: number | null; down_by: string | null;
  drawing: string | null; saved_at: number | null; saved_by: string | null; artists: string | null; changed_at: number | null; media_id: string | null;
};
type Why = DrawVersion['why'];

/** The drawing on the easel. It has a row (v21's fillDrawPages, a new drawing, and stateMessage make sure). */
export function easelId(hub: HubHost): number {
  const row = hub.sql.exec<{ value: string }>("SELECT value FROM meta WHERE key = 'canvas'").toArray()[0];
  return row ? Number(row.value) : 1;
}

function pageExists(hub: HubHost, id: number): boolean {
  return Number.isInteger(id) && hub.sql.exec('SELECT 1 FROM draw_pages WHERE id = ?', id).toArray().length > 0;
}

function hasInk(hub: HubHost, page: number): boolean {
  return hub.sql.exec('SELECT 1 FROM strokes WHERE canvas = ? AND deleted_at IS NULL LIMIT 1', page).toArray().length > 0;
}

/** The page's live strokes, oldest first. */
function strokesOf(hub: HubHost, page: number, ids?: string): Stroke[] {
  const rows = ids === undefined
    ? hub.sql.exec<{ id: string; user_id: string; data: string }>('SELECT id, user_id, data FROM strokes WHERE canvas = ? AND deleted_at IS NULL ORDER BY ts, id', page)
    : hub.sql.exec<{ id: string; user_id: string; data: string }>('SELECT id, user_id, data FROM strokes WHERE canvas = ? AND id IN (SELECT value FROM json_each(?)) ORDER BY ts, id', page, ids);
  return rows.toArray().map((r) => ({ id: r.id, uid: hub.canonical(r.user_id), ...(JSON.parse(r.data) as Omit<Stroke, 'id' | 'uid'>) }));
}

function liveIds(hub: HubHost, page: number): string[] {
  return hub.sql.exec<{ id: string }>('SELECT id FROM strokes WHERE canvas = ? AND deleted_at IS NULL ORDER BY ts, id', page).toArray().map((r) => r.id);
}

const PAGE_SELECT = 'SELECT p.*, d.media_id FROM draw_pages p LEFT JOIN drawings d ON d.id = p.drawing';

function toPage(hub: HubHost, r: PageRow, easel: number): DrawPage {
  const page: DrawPage = { id: r.id, createdAt: r.created_at };
  if (r.media_id) Object.assign(page, { media: r.media_id, savedAt: r.saved_at ?? undefined, savedBy: r.saved_by ? hub.canonical(r.saved_by) : undefined });
  if (r.pinned_at) page.pinnedAt = r.pinned_at;
  if (r.down_at) Object.assign(page, { downAt: r.down_at, downBy: r.down_by ? hub.canonical(r.down_by) : undefined });
  if (r.artists) page.artists = [...new Set((JSON.parse(r.artists) as string[]).map((u) => hub.canonical(u)))];
  if (r.id === easel) page.dirty = dirty(hub, r);
  return page;
}

/** Changed since its picture was saved (a new or undone stroke, a wipe, going back), or never saved. */
function dirty(hub: HubHost, r: PageRow): boolean {
  if (r.saved_at == null) return hasInk(hub, r.id);
  if ((r.changed_at ?? 0) > r.saved_at) return true;
  return hub.sql.exec('SELECT 1 FROM strokes WHERE canvas = ?1 AND (ts > ?2 OR deleted_at > ?2) LIMIT 1', r.id, r.saved_at).toArray().length > 0;
}

/** What's pinned (newest first), what was taken down, and the drawing on the easel. */
export function board(hub: HubHost, easel = easelId(hub)): DrawPage[] {
  const rows = [
    ...hub.sql.exec<PageRow>(`${PAGE_SELECT} WHERE p.pinned_at IS NOT NULL AND p.down_at IS NULL ORDER BY p.pinned_at DESC LIMIT ?`, BOARD_LIMIT).toArray(),
    ...hub.sql.exec<PageRow>(`${PAGE_SELECT} WHERE p.down_at IS NOT NULL ORDER BY p.down_at DESC LIMIT ?`, DOWN_LIMIT).toArray(),
  ];
  if (!rows.some((r) => r.id === easel)) rows.unshift(...hub.sql.exec<PageRow>(`${PAGE_SELECT} WHERE p.id = ?`, easel).toArray());
  return rows.map((r) => toPage(hub, r, easel));
}

/**
 * The pictures Space's Gallery hangs (and older apps' gallery): the latest
 * picture of each drawing on the board, and pictures from before drawings
 * were kept (no page), newest first.
 */
export function gallery(hub: HubHost): SavedDrawing[] {
  return hub.sql
    .exec<{ id: string; media_id: string; user_id: string; ts: number; page: number | null }>(
      `SELECT id, media_id, user_id, ts, page FROM drawings
       WHERE page IS NULL OR id IN (SELECT drawing FROM draw_pages WHERE drawing IS NOT NULL AND pinned_at IS NOT NULL AND down_at IS NULL)
       ORDER BY ts DESC LIMIT 60`,
    )
    .toArray()
    .map((r) => ({ id: r.id, media: r.media_id, uid: hub.canonical(r.user_id), ts: r.ts, ...(r.page != null ? { page: r.page } : {}) }));
}

function stateMessage(hub: HubHost, extra: { by?: string; why?: 'open' | 'new' | 'restore'; at?: number } = {}): ServerMessage {
  const canvas = easelId(hub);
  hub.sql.exec('INSERT OR IGNORE INTO draw_pages (id, created_at) VALUES (?, ?)', canvas, Date.now());
  return { t: 'draw.state', canvas, strokes: strokesOf(hub, canvas), gallery: gallery(hub), board: board(hub, canvas), ...extra };
}

export function onDrawState(hub: HubHost, ws: WebSocket) {
  hub.send(ws, stateMessage(hub));
}

function sendBoard(hub: HubHost) {
  const canvas = easelId(hub);
  hub.relayToView('draw', null, { t: 'draw.board', canvas, board: board(hub, canvas) });
}

/**
 * Keep the page as it is now as a version, unless it's blank or exactly the
 * latest version already (a save then just marks that one as saved).
 */
function keepVersion(hub: HubHost, page: number, why: Why, uid: string | null, drawing?: string) {
  const ids = JSON.stringify(liveIds(hub, page));
  if (ids === '[]') return;
  const last = hub.sql.exec<{ id: string; stroke_ids: string }>('SELECT id, stroke_ids FROM draw_versions WHERE page = ? ORDER BY ts DESC, rowid DESC LIMIT 1', page).toArray()[0];
  if (last?.stroke_ids === ids) {
    if (drawing) hub.sql.exec("UPDATE draw_versions SET why = 'save', user_id = ?, drawing = ? WHERE id = ?", uid, drawing, last.id);
    return;
  }
  hub.sql.exec('INSERT INTO draw_versions (id, page, ts, user_id, why, stroke_ids, drawing) VALUES (?, ?, ?, ?, ?, ?, ?)', crypto.randomUUID(), page, Date.now(), uid, why, ids, drawing ?? null);
}

/** The page the client says it's drawing on (`pg`), or the easel's. */
function targetPage(hub: HubHost, pg: unknown): number {
  const easel = easelId(hub);
  if (pg === undefined || pg === null) return easel;
  const page = Number(pg);
  if (page !== easel && !pageExists(hub, page)) throw new ClientError('stale_page', "That drawing isn't there any more.");
  return page;
}

/** In-progress stroke pieces: relayed live to people on the board, never stored (nor read: these come ~25 a second). */
export function onDrawSeg(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.seg' }>) {
  const s = cleanStroke(msg, 400);
  if (!s) return;
  const pg = Number(msg.pg);
  hub.relayToView('draw', ws, { t: 'draw.seg', sid: me.sid, sk: s.id, c: s.c, w: s.w, e: s.e, p: s.p, ...(s.l ? { l: s.l } : {}), ...(Number.isInteger(pg) ? { pg } : {}) });
}

/**
 * A finished stroke, kept on the drawing it was drawn on: one that was still
 * being drawn when the easel changed stays on the drawing it started on.
 */
export function onDrawStroke(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.stroke' }>) {
  const s = cleanStroke(msg, 8000);
  if (!s) throw new ClientError('bad_stroke', 'That stroke looks broken.');
  const page = targetPage(hub, msg.pg);
  // While people draw: the page as it was before this stroke, now and then (History).
  const last = hub.sql.exec<{ ts: number }>('SELECT ts FROM draw_versions WHERE page = ? ORDER BY ts DESC LIMIT 1', page).toArray()[0];
  if (!last || Date.now() - last.ts >= AUTO_VERSION_MS) keepVersion(hub, page, 'auto', null);
  hub.sql.exec(
    'INSERT OR IGNORE INTO strokes (id, canvas, user_id, data, ts) VALUES (?, ?, ?, ?, ?)',
    s.id, page, me.uid, JSON.stringify({ c: s.c, w: s.w, e: s.e, p: s.p, ...(s.k ? { k: s.k } : {}), ...(s.text ? { text: s.text } : {}), ...(s.l ? { l: s.l } : {}) }), Date.now(),
  );
  hub.relayToView('draw', ws, { t: 'draw.stroke', stroke: { ...s, uid: me.uid }, pg: page });
}

export function onDrawUndo(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.undo' }>) {
  const row = hub.sql.exec<{ user_id: string }>('SELECT user_id FROM strokes WHERE id = ? AND deleted_at IS NULL', String(msg.id)).toArray()[0];
  if (!row) return;
  if (hub.canonical(row.user_id) !== me.uid && !hub.isMod(me.uid)) throw new ClientError('forbidden', 'You can only undo your own strokes.');
  hub.sql.exec('UPDATE strokes SET deleted_at = ? WHERE id = ?', Date.now(), String(msg.id));
  hub.relayToView('draw', null, { t: 'draw.undo', id: String(msg.id) });
}

/** Refuse when the easel isn't showing what the sender saw (`from`): someone else just changed it. */
function checkFrom(hub: HubHost, from: unknown): number {
  const easel = easelId(hub);
  if (from !== undefined && from !== null && Number(from) !== easel) throw new ClientError('stale_page', 'Someone just put another drawing on the easel. Have a look first.');
  return easel;
}

/**
 * Put another drawing on the easel for everyone. The one leaving is kept as a
 * version, and goes on the board if it has ink and was never pinned: nothing
 * drawn is ever lost to a new page (a taken-down one stays down).
 */
function moveEasel(hub: HubHost, me: Attachment, from: number, to: number, why: 'open' | 'new') {
  keepVersion(hub, from, 'leave', me.uid);
  if (hasInk(hub, from)) hub.sql.exec('UPDATE draw_pages SET pinned_at = ? WHERE id = ? AND pinned_at IS NULL', Date.now(), from);
  hub.sql.exec("INSERT INTO meta (key, value) VALUES ('canvas', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", String(to));
  hub.relayToView('draw', null, stateMessage(hub, { by: me.uid, why }));
}

/** A new blank drawing on the easel (nothing happens when the easel is blank already). Also what draw.clear does now. */
export function onDrawNew(hub: HubHost, ws: WebSocket, me: Attachment, msg: { from?: unknown }) {
  const from = checkFrom(hub, msg.from);
  if (!hasInk(hub, from)) return hub.send(ws, stateMessage(hub));
  const newest = hub.sql.exec<{ n: number | null }>('SELECT MAX(n) AS n FROM (SELECT MAX(id) AS n FROM draw_pages UNION ALL SELECT MAX(canvas) FROM strokes)').one().n ?? 0;
  const id = Math.max(newest, from) + 1;
  hub.sql.exec('INSERT INTO draw_pages (id, created_at, created_by) VALUES (?, ?, ?)', id, Date.now(), me.uid);
  moveEasel(hub, me, from, id, 'new');
}

export function onDrawOpen(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.open' }>) {
  const to = Number(msg.page);
  if (!pageExists(hub, to)) throw new ClientError('no_page', "That drawing isn't there any more.");
  const from = checkFrom(hub, msg.from);
  if (to === from) return hub.send(ws, stateMessage(hub));
  moveEasel(hub, me, from, to, 'open');
}

/** Take a drawing down from the board, or put it back where it was. */
export function onDrawPin(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.pin' }>) {
  const page = Number(msg.page);
  if (!pageExists(hub, page)) throw new ClientError('no_page', "That drawing isn't there any more.");
  if (msg.on === true) hub.sql.exec('UPDATE draw_pages SET pinned_at = COALESCE(pinned_at, ?), down_at = NULL, down_by = NULL WHERE id = ?', Date.now(), page);
  else hub.sql.exec('UPDATE draw_pages SET down_at = ?, down_by = ? WHERE id = ? AND pinned_at IS NOT NULL AND down_at IS NULL', Date.now(), me.uid, page);
  sendBoard(hub);
}

/** The easel drawing's versions, newest first; the newest is marked `now` when nothing changed since. */
export function onDrawHistory(hub: HubHost, ws: WebSocket) {
  const page = easelId(hub);
  const rows = hub.sql
    .exec<{ id: string; ts: number; user_id: string | null; why: Why; stroke_ids: string; media_id: string | null }>(
      `SELECT v.id, v.ts, v.user_id, v.why, v.stroke_ids, d.media_id FROM draw_versions v LEFT JOIN drawings d ON d.id = v.drawing
       WHERE v.page = ? ORDER BY v.ts DESC, v.rowid DESC LIMIT ?`,
      page, HISTORY_LIMIT,
    )
    .toArray();
  const now = JSON.stringify(liveIds(hub, page));
  const versions: DrawVersion[] = rows.map((r, i) => ({
    id: r.id, ts: r.ts, why: r.why, n: (JSON.parse(r.stroke_ids) as string[]).length,
    ...(r.user_id ? { uid: hub.canonical(r.user_id) } : {}), ...(r.media_id ? { media: r.media_id } : {}), ...(i === 0 && r.stroke_ids === now ? { now: true } : {}),
  }));
  hub.send(ws, { t: 'draw.history', page, versions });
}

function versionRow(hub: HubHost, id: unknown) {
  const v = hub.sql.exec<{ page: number; ts: number; stroke_ids: string }>('SELECT page, ts, stroke_ids FROM draw_versions WHERE id = ?', String(id)).toArray()[0];
  if (!v) throw new ClientError('no_version', "That version isn't there any more.");
  return v;
}

export function onDrawVersion(hub: HubHost, ws: WebSocket, msg: Extract<ClientMessage, { t: 'draw.version' }>) {
  const v = versionRow(hub, msg.id);
  hub.send(ws, { t: 'draw.version', id: String(msg.id), page: v.page, ts: v.ts, strokes: strokesOf(hub, v.page, v.stroke_ids) });
}

/**
 * Take the drawing on the easel back to a version, for everyone. What's there
 * now is kept as a version first, so going back can be undone the same way.
 */
export function onDrawRestore(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.restore' }>) {
  const v = versionRow(hub, msg.id);
  const page = easelId(hub);
  if (v.page !== page) throw new ClientError('stale_page', "That drawing isn't on the easel any more.");
  keepVersion(hub, page, 'restore', me.uid);
  const now = Date.now();
  hub.sql.exec('UPDATE strokes SET deleted_at = ? WHERE canvas = ? AND deleted_at IS NULL AND id NOT IN (SELECT value FROM json_each(?))', now, page, v.stroke_ids);
  hub.sql.exec('UPDATE strokes SET deleted_at = NULL WHERE canvas = ? AND deleted_at IS NOT NULL AND id IN (SELECT value FROM json_each(?))', page, v.stroke_ids);
  hub.sql.exec('UPDATE draw_pages SET changed_at = ? WHERE id = ?', now, page);
  hub.relayToView('draw', null, stateMessage(hub, { by: me.uid, why: 'restore', at: v.ts }));
}

/**
 * Empty one layer of the drawing on the easel (PROTOCOL.md › Draw layers), or
 * every layer (`all`); the strokes stay in the database, like an undo, and the
 * page is kept as a version first. Each layer goes out as its own draw.wipe
 * (apps from before `all` know only those).
 */
export function onDrawWipe(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.wipe' }>) {
  const all = msg.all === true;
  const l = all ? 0 : Number(msg.l);
  if (!Number.isInteger(l) || l < 0 || l >= LIMITS.drawLayers) throw new ClientError('bad_layer', 'There is no such layer.');
  const page = checkFrom(hub, msg.pg);
  keepVersion(hub, page, 'wipe', me.uid);
  const now = Date.now();
  for (const layer of all ? Array.from({ length: LIMITS.drawLayers }, (_, i) => i) : [l]) {
    hub.sql.exec(
      "UPDATE strokes SET deleted_at = ? WHERE canvas = ? AND deleted_at IS NULL AND COALESCE(json_extract(data, '$.l'), 0) = ?",
      now, page, layer,
    );
    hub.relayToView('draw', null, { t: 'draw.wipe', l: layer, by: me.uid, pg: page, ...(all ? { all } : {}) });
  }
}

/**
 * Worker RPC after a picture of a drawing (PNG or GIF) was stored in Media:
 * it becomes that drawing's picture (`page`, else the one on the easel), the
 * drawing goes on the board (`pin` false: a taken-down one stays down), and
 * the page is kept as a version.
 */
export async function addDrawing(hub: HubHost, input: { sid: string; ut: string; media: string; page?: string | null; pin?: string | null }): Promise<{ ok: boolean; drawing?: SavedDrawing; message?: string }> {
  const ws = hub.uploader(input.sid, input.ut);
  if (!ws) return { ok: false, message: 'Connect first.' };
  const me = ws.deserializeAttachment() as Attachment;
  if (!hub.allow(me.sid, 'save', 6, 1 / 10)) return { ok: false, message: 'Saving too fast.' };
  const page = input.page ? Number(input.page) : easelId(hub);
  if (!pageExists(hub, page)) return { ok: false, message: "That drawing isn't there any more." };
  const pin = input.pin !== '0';
  const drawing: SavedDrawing = { id: crypto.randomUUID(), media: input.media, uid: me.uid, ts: Date.now(), page };
  hub.sql.exec('INSERT INTO drawings (id, media_id, user_id, ts, page) VALUES (?, ?, ?, ?, ?)', drawing.id, drawing.media, drawing.uid, drawing.ts, page);
  const artists = [...new Set(hub.sql.exec<{ user_id: string }>('SELECT DISTINCT user_id FROM strokes WHERE canvas = ? AND deleted_at IS NULL', page).toArray().map((r) => hub.canonical(r.user_id)))];
  hub.sql.exec(
    `UPDATE draw_pages SET drawing = ?1, saved_at = ?2, saved_by = ?3, artists = ?4,
       pinned_at = CASE WHEN ?5 THEN COALESCE(pinned_at, ?2) ELSE pinned_at END,
       down_at = CASE WHEN ?5 THEN NULL ELSE down_at END, down_by = CASE WHEN ?5 THEN NULL ELSE down_by END
     WHERE id = ?6`,
    drawing.id, drawing.ts, me.uid, artists.length ? JSON.stringify(artists) : null, pin ? 1 : 0, page,
  );
  keepVersion(hub, page, 'save', me.uid, drawing.id);
  const onBoard = hub.sql.exec('SELECT 1 FROM draw_pages WHERE id = ? AND pinned_at IS NOT NULL AND down_at IS NULL', page).toArray().length > 0;
  if (onBoard) hub.broadcast({ t: 'drawing', drawing });
  sendBoard(hub);
  return { ok: true, drawing };
}
