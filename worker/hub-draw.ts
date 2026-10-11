// Draw: the shared drawing board, its layers and the gallery (PROTOCOL.md › Draw layers).
import type { Attachment } from './attachment';
import { ClientError, cleanStroke } from './validate';
import { LIMITS, type ClientMessage, type SavedDrawing, type Stroke } from './protocol';
import type { HubHost } from './hub-host';

function canvasId(hub: HubHost): number {
  const row = hub.sql.exec<{ value: string }>("SELECT value FROM meta WHERE key = 'canvas'").toArray()[0];
  return row ? Number(row.value) : 1;
}

export function onDrawState(hub: HubHost, ws: WebSocket) {
  const canvas = canvasId(hub);
  const strokes = hub.sql
    .exec<{ id: string; user_id: string; data: string }>('SELECT id, user_id, data FROM strokes WHERE canvas = ? AND deleted_at IS NULL ORDER BY ts', canvas)
    .toArray()
    .map((r) => ({ id: r.id, uid: hub.canonical(r.user_id), ...(JSON.parse(r.data) as Omit<Stroke, 'id' | 'uid'>) }));
  hub.send(ws, { t: 'draw.state', canvas, strokes, gallery: gallery(hub) });
}

export function gallery(hub: HubHost): SavedDrawing[] {
  return hub.sql
    .exec<{ id: string; media_id: string; user_id: string; ts: number }>('SELECT id, media_id, user_id, ts FROM drawings ORDER BY ts DESC LIMIT 60')
    .toArray()
    .map((r) => ({ id: r.id, media: r.media_id, uid: hub.canonical(r.user_id), ts: r.ts }));
}

/** In-progress stroke pieces: relayed live to people on the board, never stored. */
export function onDrawSeg(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.seg' }>) {
  const s = cleanStroke(msg, 400);
  if (!s) return;
  hub.relayToView('draw', ws, { t: 'draw.seg', sid: me.sid, sk: s.id, c: s.c, w: s.w, e: s.e, p: s.p, ...(s.l ? { l: s.l } : {}) });
}

export function onDrawStroke(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.stroke' }>) {
  const s = cleanStroke(msg, 8000);
  if (!s) throw new ClientError('bad_stroke', 'That stroke looks broken.');
  hub.sql.exec(
    'INSERT OR IGNORE INTO strokes (id, canvas, user_id, data, ts) VALUES (?, ?, ?, ?, ?)',
    s.id, canvasId(hub), me.uid, JSON.stringify({ c: s.c, w: s.w, e: s.e, p: s.p, ...(s.k ? { k: s.k } : {}), ...(s.text ? { text: s.text } : {}), ...(s.l ? { l: s.l } : {}) }), Date.now(),
  );
  hub.relayToView('draw', ws, { t: 'draw.stroke', stroke: { ...s, uid: me.uid } });
}

export function onDrawUndo(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.undo' }>) {
  const row = hub.sql.exec<{ user_id: string }>('SELECT user_id FROM strokes WHERE id = ? AND deleted_at IS NULL', String(msg.id)).toArray()[0];
  if (!row) return;
  if (hub.canonical(row.user_id) !== me.uid && !hub.isMod(me.uid)) throw new ClientError('forbidden', 'You can only undo your own strokes.');
  hub.sql.exec('UPDATE strokes SET deleted_at = ? WHERE id = ?', Date.now(), String(msg.id));
  hub.relayToView('draw', null, { t: 'draw.undo', id: String(msg.id) });
}

/** New blank canvas. The old strokes stay in the database under the old canvas id. */
export function onDrawClear(hub: HubHost, me: Attachment) {
  const canvas = canvasId(hub) + 1;
  hub.sql.exec("INSERT INTO meta (key, value) VALUES ('canvas', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", String(canvas));
  hub.relayToView('draw', null, { t: 'draw.clear', canvas, by: me.uid });
}

/** Empty one layer of this page (PROTOCOL.md › Draw layers); its strokes stay in the database, like an undo. */
export function onDrawWipe(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'draw.wipe' }>) {
  const l = Number(msg.l);
  if (!Number.isInteger(l) || l < 0 || l >= LIMITS.drawLayers) throw new ClientError('bad_layer', 'There is no such layer.');
  hub.sql.exec(
    "UPDATE strokes SET deleted_at = ? WHERE canvas = ? AND deleted_at IS NULL AND COALESCE(json_extract(data, '$.l'), 0) = ?",
    Date.now(), canvasId(hub), l,
  );
  hub.relayToView('draw', null, { t: 'draw.wipe', l, by: me.uid });
}

/** Worker RPC after a drawing (PNG or GIF) was stored in Media: it joins the gallery. */
export async function addDrawing(hub: HubHost, input: { sid: string; ut: string; media: string }): Promise<{ ok: boolean; drawing?: SavedDrawing; message?: string }> {
  const ws = hub.uploader(input.sid, input.ut);
  if (!ws) return { ok: false, message: 'Connect first.' };
  const me = ws.deserializeAttachment() as Attachment;
  if (!hub.allow(me.sid, 'save', 6, 1 / 10)) return { ok: false, message: 'Saving too fast.' };
  const drawing: SavedDrawing = { id: crypto.randomUUID(), media: input.media, uid: me.uid, ts: Date.now() };
  hub.sql.exec('INSERT INTO drawings (id, media_id, user_id, ts) VALUES (?, ?, ?, ?)', drawing.id, drawing.media, drawing.uid, drawing.ts);
  hub.broadcast({ t: 'drawing', drawing });
  return { ok: true, drawing };
}
