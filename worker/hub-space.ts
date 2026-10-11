// Space (PROTOCOL.md › Space): who stands where, and the spot each person
// comes back to. Emotes and games are relayed by hub.ts's dispatch.
import type { Attachment } from './attachment';
import { clamp } from './validate';
import type { ClientMessage } from './protocol';
import type { HubHost } from './hub-host';

const WORLD = { w: 2400, h: 1600 }; // Space view size, in world units

export function onSpaceState(hub: HubHost, ws: WebSocket) {
  const me = ws.deserializeAttachment() as Attachment;
  const players = hub.sockets()
    .map((s) => s.deserializeAttachment() as Attachment)
    .filter((a) => a.view === 'space' && a.sx !== undefined)
    .map((a) => ({ sid: a.sid, x: a.sx!, y: a.sy!, d: a.sd ?? 0 }));
  const saved = hub.sql
    .exec<{ space_x: number | null; space_y: number | null; space_d: number | null }>('SELECT space_x, space_y, space_d FROM users WHERE id = ?', me.uid)
    .toArray()[0];
  const spot = saved?.space_x != null ? { x: saved.space_x, y: saved.space_y ?? 0, d: saved.space_d ?? 0 } : null;
  hub.send(ws, { t: 'space.state', players, me: spot });
}

export function onSpaceMove(hub: HubHost, ws: WebSocket, me: Attachment, msg: Extract<ClientMessage, { t: 'space.move' }>) {
  const x = Math.round(clamp(Number(msg.x) || 0, 0, WORLD.w));
  const y = Math.round(clamp(Number(msg.y) || 0, 0, WORLD.h));
  const d = Math.round((Number.isFinite(Number(msg.d)) ? Number(msg.d) % (2 * Math.PI) : 0) * 100) / 100;
  // Remember where you are: on leaving Space or disconnecting, and at most
  // every 30 s while walking (row writes are a free-plan budget).
  const save = hub.allow(me.uid, 'spacesave', 1, 1 / 30);
  if (save) saveSpot(hub, me.uid, x, y, d);
  ws.serializeAttachment({ ...me, sx: x, sy: y, sd: d, sdirty: !save });
  hub.relayToView('space', ws, { t: 'space.pos', sid: me.sid, x, y, d });
}

export function saveSpot(hub: HubHost, uid: string, x: number, y: number, d: number) {
  hub.sql.exec('UPDATE users SET space_x = ?, space_y = ?, space_d = ? WHERE id = ?', x, y, d, uid);
}
