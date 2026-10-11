// Who may use this server: the invite key (the main one or a personal invite)
// and where a request carries it. The admin API has its own key (admin.ts).
import { sameSecret } from './encoding';
import { hub } from './objects';

/**
 * The invite key, from (best first) the X-Yap-Key header, the WebSocket
 * subprotocol "key.<k>", or the ?key= query. Keep the query: the web client
 * (settings.js › socketUrl) and desktop 0.8 put the key in /ws's address when
 * it has characters a subprotocol can't carry (anything but letters, digits,
 * - and _). HTTP calls from the shipped clients all use the header.
 */
export function inviteKey(request: Request, url: URL): string | null {
  const header = request.headers.get('X-Yap-Key');
  if (header) return header;
  const proto = (request.headers.get('Sec-WebSocket-Protocol') ?? '').split(',').map((p) => p.trim()).find((p) => p.startsWith('key.'));
  if (proto) return proto.slice(4);
  return url.searchParams.get('key');
}

/**
 * The main key as this Worker instance last heard it from the Hub (the owner
 * can reset it: PROTOCOL.md › Resetting the main link; YAP_KEY is only the
 * first one), remembered for a minute. A key that doesn't match asks the Hub
 * again, so a link that was just reset works at once; keys that still don't
 * match are remembered as wrong for 10 s, so they don't each cost a request
 * to the Hub. An old link keeps working here for up to a minute after a
 * reset; the socket itself is checked by the Hub against the current key.
 */
let mainCache: { key: string | null; until: number } | null = null;
const wrongKeys = new Map<string, number>();

async function mainKey(env: Env, fresh: boolean): Promise<string | null> {
  const now = Date.now();
  if (!fresh && mainCache && mainCache.until > now) return mainCache.key;
  const key = await hub(env).mainKey();
  mainCache = { key, until: now + 60_000 };
  return key;
}

/** Forget what this instance remembers about the main key (the admin API just changed it). */
export function forgetMainKey() {
  mainCache = null;
  wrongKeys.clear();
}

/**
 * Who may connect: true for the main invite key, the invite's id for a live
 * personal invite (yi_…), null otherwise. Personal invites are checked with
 * the Hub and remembered for a minute per Worker instance. `exact` asks the
 * Hub for the main key every time (/api/check, which says whether a link works).
 */
const inviteCache = new Map<string, { id: string | null; until: number }>();
export async function keyAccess(env: Env, key: string | null, exact = false): Promise<true | string | null> {
  if (!env.YAP_KEY) return true; // an open server (local dev only)
  if (!key || key.length > 512) return null;
  const now = Date.now();
  if (sameSecret(key, (await mainKey(env, exact)) ?? '')) return true;
  if (key.startsWith('yi_')) {
    const hit = inviteCache.get(key);
    if (hit && hit.until > now) return hit.id;
    const id = await hub(env).checkInvite(key);
    if (inviteCache.size > 500) inviteCache.clear();
    inviteCache.set(key, { id, until: now + (id ? 60_000 : 10_000) });
    return id;
  }
  if ((wrongKeys.get(key) ?? 0) > now && !exact) return null;
  // Not the main key as last heard: it may have just been reset. Ask the Hub.
  if (!exact && sameSecret(key, (await mainKey(env, true)) ?? '')) return true;
  if (wrongKeys.size > 500) wrongKeys.clear();
  wrongKeys.set(key, now + 10_000);
  return null;
}
