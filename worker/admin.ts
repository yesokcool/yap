// The admin API (/api/admin/*, PROTOCOL.md › Backups): what the server
// owner's tools call (tools/backup.mjs, restore.mjs, import-emojis.mjs).
// Every route needs the admin key.
import { checkWav } from './extras';
import { sameSecret } from './encoding';
import { hub, media } from './objects';
import { forgetMainKey } from './access';
import { MAX_PICTURE, contentId, fail, uploadEmoji } from './uploads';
import { sniffImage } from './pictures';

/** Admin endpoints take `Authorization: Bearer <ADMIN_KEY>`; disabled if unset. */
export function adminOk(env: Env, request: Request): boolean {
  const presented = request.headers.get('Authorization')?.replace(/^Bearer\s+/i, '') ?? '';
  return Boolean(presented) && sameSecret(presented, env.ADMIN_KEY ?? '');
}

/** An /api/admin/* request; null when no admin route has that path. */
export async function adminRoute(request: Request, url: URL, env: Env): Promise<Response | null> {
  switch (url.pathname) {
    case '/api/admin/stats':
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      return Response.json({ hub: await hub(env).stats(), media: await media(env).stats() });

    // Backup: GET pages until `next` is null. Media blobs: /api/admin/media, then /api/admin/media/<id>.
    case '/api/admin/export': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      const after = Number(url.searchParams.get('after') ?? 0) || 0;
      return Response.json(await hub(env).exportPage(after, 5000));
    }

    // Bulk-add emojis without a live session: POST raw image bytes.
    case '/api/admin/emoji': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST an image.', { status: 405 });
      return uploadEmoji(request, url, env, true);
    }

    case '/api/admin/merge-users': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST ?from=<uid>&into=<uid>.', { status: 405 });
      const result = await hub(env).mergeUsers(url.searchParams.get('from') ?? '', url.searchParams.get('into') ?? '');
      return Response.json(result, { status: result.ok ? 200 : 400 });
    }

    // Add text channels by name (skips names that already exist): ?names=a,b,c
    // Archive by name instead (messages kept, hidden for everyone): ?archive=a,b,c
    // Bring archived ones back (with all their messages): ?unarchive=a,b,c
    case '/api/admin/channels': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST ?names=a,b,c[&kind=voice] or ?archive=a,b,c.', { status: 405 });
      const list = (key: string) => (url.searchParams.get(key) ?? '').split(',').map((n) => n.trim()).filter(Boolean).slice(0, 20);
      const kind = url.searchParams.get('kind') === 'voice' ? 'voice' : 'text';
      if (url.searchParams.has('archive')) return Response.json(await hub(env).archiveChannels(list('archive'), kind));
      if (url.searchParams.has('unarchive')) return Response.json(await hub(env).unarchiveChannels(list('unarchive'), kind));
      return Response.json(await hub(env).addChannels(list('names'), kind));
    }

    case '/api/admin/role': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST ?uid=<uid>&role=owner|mod|member|banned.', { status: 405 });
      const role = url.searchParams.get('role') ?? '';
      if (!['owner', 'mod', 'member', 'banned'].includes(role)) return new Response('Bad role.', { status: 400 });
      const result = await hub(env).setRole(url.searchParams.get('uid') ?? '', role as 'owner');
      return Response.json(result, { status: result.ok ? 200 : 400 });
    }

    // A forgotten password, or an account that never set one: returns a temporary password.
    case '/api/admin/reset-pin': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST ?uid=<uid>.', { status: 405 });
      const result = await hub(env).resetPin(url.searchParams.get('uid') ?? '');
      return Response.json(result, { status: result.ok ? 200 : 400 });
    }

    // The main invite link: POST = a new one (like Settings › Invite › Reset main link…),
    // DELETE = back to the YAP_KEY secret. Everyone connected gets the new key.
    case '/api/admin/main-key': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST' && request.method !== 'DELETE') return new Response('POST for a new main link, DELETE to go back to YAP_KEY.', { status: 405 });
      const result = await hub(env).resetMainKey(request.method === 'DELETE');
      forgetMainKey();
      return Response.json(result, { status: result.ok ? 200 : 400 });
    }

    // Tests only: an account with an old 6-digit PIN (sign-up makes passwords). Off unless TEST_PINS=1.
    case '/api/admin/test-pin': {
      if (env.TEST_PINS !== '1') return null;
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST ?uid=<uid>&pin=<6 digits>.', { status: 405 });
      const result = await hub(env).setTestPin(url.searchParams.get('uid') ?? '', url.searchParams.get('pin') ?? '');
      return Response.json(result, { status: result.ok ? 200 : 400 });
    }

    case '/api/admin/unmerge-user': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST ?alias=<uid>.', { status: 405 });
      return Response.json(await hub(env).unmergeUser(url.searchParams.get('alias') ?? ''));
    }

    case '/api/admin/media':
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      return Response.json(await media(env).list());

    // Disaster recovery (tools/restore.mjs): put an image back, then rebuild
    // the chat database in phases. Import only works into an empty server.
    case '/api/admin/media-put': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST image bytes.', { status: 405 });
      // Anything the backup has: images, clips, and sealed (encrypted) pictures.
      const data = await request.arrayBuffer();
      if (data.byteLength > MAX_PICTURE + 28) return fail(413, 'too_big', 'Too big.');
      const bytes = new Uint8Array(data);
      const mime = sniffImage(bytes) ?? (typeof checkWav(bytes, 60_000) === 'string' ? 'application/octet-stream' : 'audio/wav');
      const id = await contentId(data);
      await media(env).put(id, mime, data);
      return Response.json({ ok: true, id, mime });
    }
    case '/api/admin/import': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST a phase.', { status: 405 });
      const result = await hub(env).importBackup(await request.json());
      return Response.json(result, { status: result.ok ? 200 : 409 });
    }

    // Point-in-time recovery (last 30 days). ?to=<ISO time> or ?bookmark=<undo bookmark>.
    case '/api/admin/restore': {
      if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
      if (request.method !== 'POST') return new Response('POST to restore.', { status: 405 });
      const to = url.searchParams.get('to');
      const bookmark = url.searchParams.get('bookmark') ?? undefined;
      const time = to ? Date.parse(to) : undefined;
      if (!bookmark && !Number.isFinite(time)) return new Response('Pass ?to=<ISO time> or ?bookmark=.', { status: 400 });
      const result = await hub(env).prepareRestore({ time, bookmark });
      try {
        await hub(env).restart();
      } catch {
        // Expected: aborting the object fails the in-flight call.
      }
      return Response.json({ restored: true, ...result, note: 'Keep `undo` — POST ?bookmark=<undo> reverses this.' });
    }
  }

  // One stored blob for a backup, never cached anywhere (it may be erased tomorrow).
  const adminBlob = url.pathname.match(/^\/api\/admin\/media\/([a-f0-9]{32})$/);
  if (adminBlob) {
    if (!adminOk(env, request)) return new Response('Admin key required.', { status: 401 });
    const found = await media(env).get(adminBlob[1]);
    if (!found) return new Response('Gone.', { status: 404 });
    return new Response(found.data, { headers: { 'Content-Type': found.mime, 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' } });
  }
  return null;
}
