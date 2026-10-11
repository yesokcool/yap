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
 * Who may connect: true for the main invite key, the invite's id for a live
 * personal invite (yi_…), null otherwise. Personal invites are checked with
 * the Hub and remembered for a minute per Worker instance.
 */
const inviteCache = new Map<string, { id: string | null; until: number }>();
export async function keyAccess(env: Env, key: string | null): Promise<true | string | null> {
  if (keyMatches(env, key)) return true;
  if (!key || !key.startsWith('yi_')) return null;
  const now = Date.now();
  const hit = inviteCache.get(key);
  if (hit && hit.until > now) return hit.id;
  const id = await hub(env).checkInvite(key);
  if (inviteCache.size > 500) inviteCache.clear();
  inviteCache.set(key, { id, until: now + (id ? 60_000 : 10_000) });
  return id;
}

function keyMatches(env: Env, key: string | null): boolean {
  if (!env.YAP_KEY) return true;
  return Boolean(key) && sameSecret(key!, env.YAP_KEY);
}
