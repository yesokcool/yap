import { PROTOCOL_VERSION } from './protocol';
import { ping, type MinecraftStatus } from './minecraft';
import { vapidPublicKey } from './webpush';
import { ntfyServer } from './notify';
import { settings } from './settings';
import { inviteKey, keyAccess } from './access';
import { adminOk, adminRoute } from './admin';
import { boardRoute } from './board';
import { iceServers } from './ice';
import { hub } from './objects';
import { serveBlob, uploadBlob, uploadDoodle, uploadEmoji, uploadImage, uploadSound } from './uploads';

export { Hub } from './hub';
export { Media } from './media';

let minecraftCache: MinecraftStatus | null = null;

/** Optional: a Minecraft server's status in the sidebar (`MINECRAFT_SERVER` = "host:port"). */
function minecraftServer(env: Env): { name: string; host: string; port: number } | null {
  const s = settings(env);
  const m = /^([^:\s/]+):(\d{1,5})$/.exec(s.minecraftServer ?? '');
  return m ? { name: s.minecraftName ?? 'Minecraft', host: m[1], port: Number(m[2]) } : null;
}

export default {
  async fetch(request, env): Promise<Response> {
    const url = new URL(request.url);
    const key = inviteKey(request, url);

    // API, socket and board page are capped per IP. Images (/emoji, /media,
    // and the paths a board lists in BOARD_PATHS) are not: one board view can
    // load 100+ images, and browsers cache them.
    const ip = request.headers.get('CF-Connecting-IP');
    const limited = url.pathname === '/ws' || url.pathname.startsWith('/api/') || url.pathname.startsWith('/board');
    // Admin tools (backup, restore) hold the admin key and may need many calls.
    if (limited && ip && env.PER_IP && !adminOk(env, request)) {
      const { success } = await env.PER_IP.limit({ key: ip });
      if (!success) return new Response('Slow down.', { status: 429, headers: { 'Retry-After': '10' } });
    }

    const board = await boardRoute(request, url, env);
    if (board) return board;
    if (url.pathname.startsWith('/api/admin/')) {
      const admin = await adminRoute(request, url, env);
      if (admin) return admin;
    }

    switch (url.pathname) {
      case '/ws': {
        if (request.headers.get('Upgrade')?.toLowerCase() !== 'websocket') {
          return new Response('Expected a WebSocket upgrade.', { status: 426 });
        }
        const access = await keyAccess(env, key);
        if (!access) return new Response('Invalid invite key.', { status: 401 });
        // Tell the Hub which personal invite this was (never trust the client's copy).
        const headers = new Headers(request.headers);
        headers.delete('X-Yap-Invite');
        if (access !== true) headers.set('X-Yap-Invite', access);
        // One server for now. Multiple servers later = one Hub per name.
        return hub(env).fetch(new Request(request, { headers }));
      }

      case '/api/emoji': {
        if (request.method !== 'POST') return new Response('POST an image.', { status: 405 });
        if (!(await keyAccess(env, key))) return new Response('Invalid invite key.', { status: 401 });
        return uploadEmoji(request, url, env);
      }

      case '/api/avatar':
      case '/api/drawing':
      case '/api/image': {
        if (request.method !== 'POST') return new Response('POST an image.', { status: 405 });
        if (!(await keyAccess(env, key))) return new Response('Invalid invite key.', { status: 401 });
        return uploadImage(request, url, env);
      }

      case '/api/blob': {
        if (request.method !== 'POST') return new Response('POST the encrypted bytes.', { status: 405 });
        if (!(await keyAccess(env, key))) return new Response('Invalid invite key.', { status: 401 });
        return uploadBlob(request, url, env);
      }

      case '/api/sound': {
        if (request.method !== 'POST') return new Response('POST a WAV.', { status: 405 });
        if (!(await keyAccess(env, key))) return new Response('Invalid invite key.', { status: 401 });
        return uploadSound(request, url, env);
      }

      case '/api/doodle': {
        if (request.method !== 'POST') return new Response('POST an image.', { status: 405 });
        if (!(await keyAccess(env, key))) return new Response('Invalid invite key.', { status: 401 });
        return uploadDoodle(request, url, env);
      }

      // Lets clients tell "wrong key" apart from "server down" before
      // opening the socket (a browser can't read a failed upgrade's status).
      case '/api/check':
        return Response.json({
          ok: Boolean(await keyAccess(env, key)),
          keyRequired: Boolean(env.YAP_KEY),
          protocol: PROTOCOL_VERSION,
        });

      // Client-side config for invited users. The KLIPY key must be used from
      // each user's own browser/app (KLIPY's terms forbid proxying requests),
      // so it's handed out here rather than baked into the public page.
      case '/api/config': {
        if (!(await keyAccess(env, key))) return new Response('Invalid invite key.', { status: 401 });
        const mc = minecraftServer(env);
        return Response.json(
          {
            gifs: env.KLIPY_KEY ? { provider: 'klipy', appKey: env.KLIPY_KEY } : null,
            links: await hub(env).links(),
            ...(mc ? { minecraft: { name: mc.name, address: `${mc.host}:${mc.port}` } } : {}),
            push: env.VAPID_PRIVATE_JWK ? { publicKey: vapidPublicKey(JSON.parse(env.VAPID_PRIVATE_JWK)) } : null,
            // Where phone pushes come from: people subscribe to their topic there.
            ntfy: { server: ntfyServer(env) },
            source: settings(env).sourceUrl ?? null,
          },
          { headers: { 'Cache-Control': 'no-store' } },
        );
      }

      // Is the Minecraft server up? Cached ~30 s per Worker instance.
      case '/api/minecraft': {
        if (!(await keyAccess(env, key))) return new Response('Invalid invite key.', { status: 401 });
        const mc = minecraftServer(env);
        if (!mc) return new Response('No Minecraft server is set up.', { status: 404 });
        if (!minecraftCache || Date.now() - minecraftCache.checkedAt > 30_000) {
          minecraftCache = await ping(mc.host, mc.port);
        }
        return Response.json({ name: mc.name, ...minecraftCache }, { headers: { 'Cache-Control': 'no-store' } });
      }

      case '/api/ice': {
        if (!(await keyAccess(env, key))) return new Response('Invalid invite key.', { status: 401 });
        return Response.json({ iceServers: await iceServers(env) }, { headers: { 'Cache-Control': 'no-store' } });
      }
    }

    // /emoji/<id> and /media/<id> are the same content-addressed store.
    const blob = url.pathname.match(/^\/(?:emoji|media)\/([a-f0-9]{32})$/);
    if (blob) return serveBlob(request, blob[1], env, url.pathname.startsWith('/emoji/'));
    const asset = await env.ASSETS.fetch(request);
    // A mistyped address in a browser: a page with a way back (web/404.html), not a blank one.
    if (asset.status === 404 && request.method === 'GET' && (request.headers.get('Accept') ?? '').includes('text/html')) {
      const page = await env.ASSETS.fetch(new Request(new URL('/404', url)));
      if (page.ok) return new Response(page.body, { status: 404, headers: page.headers });
    }
    return asset;
  },
} satisfies ExportedHandler<Env>;
