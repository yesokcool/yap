// Watch together (PROTOCOL.md › Watch together): one shared YouTube player.
import type { Attachment } from './attachment';
import { ClientError, YOUTUBE_ID_RE } from './validate';
import type { ClientMessage, WatchState } from './protocol';
import type { HubHost } from './hub-host';

export function watchState(hub: HubHost): WatchState {
  const raw = hub.sql.exec<{ value: string }>("SELECT value FROM meta WHERE key = 'watch'").toArray()[0]?.value;
  const empty: WatchState = { videoId: null, title: null, playing: false, position: 0, at: Date.now(), by: null, queue: [] };
  return raw ? { ...empty, ...(JSON.parse(raw) as WatchState) } : empty;
}

function saveWatch(hub: HubHost, state: WatchState) {
  hub.sql.exec("INSERT INTO meta (key, value) VALUES ('watch', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", JSON.stringify(state));
  hub.broadcast({ t: 'watch.state', state, now: Date.now() });
}

/** Where the video is right now (seconds), given the stored state. */
function watchPosition(s: WatchState, now = Date.now()) {
  return s.playing ? s.position + (now - s.at) / 1000 : s.position;
}

/** A video's title, from YouTube's public oEmbed (no key; the Worker asks, not the browser). */
async function youtubeTitle(id: string): Promise<string | null> {
  try {
    const res = await fetch(`https://www.youtube.com/oembed?format=json&url=${encodeURIComponent(`https://www.youtube.com/watch?v=${id}`)}`);
    if (!res.ok) return null;
    const body = (await res.json()) as { title?: string };
    return typeof body.title === 'string' ? body.title.slice(0, 200) : null;
  } catch {
    return null;
  }
}

export function setWatching(hub: HubHost, ws: WebSocket, me: Attachment, on: boolean) {
  if (Boolean(me.watching) === on || me.listen) return;
  const updated: Attachment = { ...me, watching: on };
  ws.serializeAttachment(updated);
  hub.showSession(ws, me, updated);
  hub.broadcast({ t: 'watching', sid: me.sid, on });
  hub.hideSession(ws, me, updated);
  if (on) hub.send(ws, { t: 'watch.state', state: watchState(hub), now: Date.now() });
}

// Durable Objects let other messages run while a fetch() is out, so look
// titles up *before* reading the state — read-await-write would let two
// quick changes overwrite each other (a queued video vanishing).
export async function onWatchSet(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'watch.set' }>) {
  if (msg.videoId !== undefined) {
    const id = String(msg.videoId);
    if (!YOUTUBE_ID_RE.test(id)) throw new ClientError('bad_video', 'That doesn’t look like a YouTube video.');
    const title = await youtubeTitle(id);
    const s = watchState(hub);
    Object.assign(s, { videoId: id, title, playing: true, position: 0, at: Date.now(), by: me.uid });
    return saveWatch(hub, s);
  }
  const s = watchState(hub);
  const now = Date.now();
  if (!s.videoId) throw new ClientError('no_video', 'Nothing is playing.');
  const position = msg.position !== undefined ? Number(msg.position) : watchPosition(s, now);
  if (!Number.isFinite(position) || position < 0 || position > 24 * 3600) throw new ClientError('bad_position', 'Bad position.');
  Object.assign(s, { position, at: now, playing: msg.playing !== undefined ? Boolean(msg.playing) : s.playing, by: me.uid });
  saveWatch(hub, s);
}

export async function onWatchQueue(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'watch.queue' }>) {
  if (msg.add !== undefined) {
    const id = String(msg.add);
    if (!YOUTUBE_ID_RE.test(id)) throw new ClientError('bad_video', 'That doesn’t look like a YouTube video.');
    const title = await youtubeTitle(id);
    const s = watchState(hub); // fresh, after the await
    // Nothing on: play it now. Decided after the await, so of two adds at once one plays and one waits.
    if (!s.videoId) {
      Object.assign(s, { videoId: id, title, playing: true, position: 0, at: Date.now(), by: me.uid });
      return saveWatch(hub, s);
    }
    if (s.queue.length >= 20) throw new ClientError('too_many', 'The queue is full (20).');
    s.queue.push({ id, title, by: me.uid });
    return saveWatch(hub, s);
  }
  const s = watchState(hub);
  if (msg.remove !== undefined) {
    const i = Number(msg.remove);
    if (!Number.isInteger(i) || i < 0 || i >= s.queue.length) throw new ClientError('bad_index', 'No such queue item.');
    s.queue.splice(i, 1);
  }
  saveWatch(hub, s);
}

/** The video ended. Several clients report it at once; only the first one counts. */
export function onWatchNext(hub: HubHost, me: Attachment, msg: Extract<ClientMessage, { t: 'watch.next' }>) {
  const s = watchState(hub);
  if (!s.videoId || s.videoId !== String(msg.expect)) return;
  const next = s.queue.shift();
  Object.assign(s, next
    ? { videoId: next.id, title: next.title, playing: true, position: 0, at: Date.now(), by: next.by }
    : { videoId: null, title: null, playing: false, position: 0, at: Date.now(), by: me.uid });
  saveWatch(hub, s);
}
