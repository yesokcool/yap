/**
 * Optional: a board, i.e. another Worker bound as `BOARD` (a service binding),
 * shown inside Yap at /board so it can share live pointers (web/js/cursors.js).
 * Its page is served at /board; `BOARD_PATHS` (a regex) lists the other root
 * paths that page uses (its APIs, images), which are passed through as they are.
 * `BOARD_ORIGIN` is the origin the board Worker sees (default: this one).
 */
import { settings } from './settings';

let boardPaths: { source: string; re: RegExp | null } | null = null;
function isBoardPath(env: Env, path: string): boolean {
  if (path === '/board' || path === '/board/') return true;
  const source = settings(env).boardPaths ?? '';
  if (boardPaths?.source !== source) {
    let re: RegExp | null = null;
    try {
      re = source ? new RegExp(source) : null;
    } catch {
      console.warn('BOARD_PATHS is not a valid regex');
    }
    boardPaths = { source, re };
  }
  return boardPaths.re?.test(path) ?? false;
}

/** The board's page and the paths it uses, from the BOARD Worker; null for every other path. */
export async function boardRoute(request: Request, url: URL, env: Env): Promise<Response | null> {
  const board = settings(env).board;
  if (board && isBoardPath(env, url.pathname)) {
    const target = new URL(url.pathname.startsWith('/board') ? '/' : url.pathname, settings(env).boardOrigin ?? url.origin);
    target.search = url.search;
    const res = await board.fetch(new Request(target, request));
    return url.pathname.startsWith('/board') ? lockDownBoardPage(res) : res;
  }
  if (url.pathname === '/board' || url.pathname === '/board/') return new Response('There is no board on this server.', { status: 404 });
  return null;
}

/**
 * The board page runs on Yap's origin (that's what lets Yap draw pointers on
 * it), so a script injected into the board could read Yap's sign-in token.
 * Lock it down: only the board's own inline scripts (by hash) may run — no
 * javascript: links, no injected handlers, no third-party scripts.
 */
async function lockDownBoardPage(res: Response): Promise<Response> {
  if (!(res.headers.get('Content-Type') ?? '').includes('text/html')) return res;
  const html = await res.text();
  const hashes: string[] = [];
  for (const m of html.matchAll(/<script(?![^>]*\bsrc=)[^>]*>([\s\S]*?)<\/script>/gi)) {
    const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(m[1]));
    hashes.push(`'sha256-${btoa(String.fromCharCode(...new Uint8Array(digest)))}'`);
  }
  const headers = new Headers(res.headers);
  headers.set('Content-Security-Policy', [
    "default-src 'self'",
    `script-src 'self' ${hashes.join(' ')}`,
    "style-src 'self' 'unsafe-inline' https://fonts.googleapis.com",
    "font-src 'self' https://fonts.gstatic.com",
    "img-src 'self' data: https:",
    "connect-src 'self'",
    "frame-ancestors 'self'",
    "base-uri 'none'",
    "object-src 'none'",
    "form-action 'self'",
  ].join('; '));
  headers.set('X-Content-Type-Options', 'nosniff');
  headers.set('Referrer-Policy', 'no-referrer');
  headers.delete('Content-Length');
  return new Response(html, { status: res.status, headers });
}
