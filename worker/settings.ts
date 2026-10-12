// This server's own settings: wrangler.jsonc "vars" (and the optional BOARD
// service binding). All optional; SELF_HOSTING.md lists them. Read loosely
// here, not through `Env`, so the types `wrangler types` writes for any
// config (with or without these vars, as literal strings) still fit.

/**
 * Where Yap's source code lives (set when it's published). Yap is MIT-licensed;
 * a server running its own copy can point SOURCE_URL at that code instead.
 */
export const UPSTREAM_SOURCE = '';

export interface Settings {
  /** The source code people are running (the Source link): SOURCE_URL, else upstream. */
  sourceUrl?: string;
  /** Where people reach this server, e.g. https://chat.example.com (push links, Web Push). */
  publicUrl?: string;
  /** "host:port" of a Minecraft server whose status shows in the sidebar, and what to call it. */
  minecraftServer?: string;
  minecraftName?: string;
  /** Another Worker shown inside Yap at /board (see worker/board.ts). */
  board?: Fetcher;
  /** A regex of the other root paths the board's page uses (its APIs, images). */
  boardPaths?: string;
  /** The origin the board Worker is asked on (default: this server's). */
  boardOrigin?: string;
  /** What the board's sidebar link is called (default: "Board"). */
  boardName?: string;
}

export function settings(env: Env): Settings {
  const e = env as unknown as Record<string, unknown>;
  const str = (k: string) => (typeof e[k] === 'string' && (e[k] as string).trim() ? (e[k] as string).trim() : undefined);
  const board = e.BOARD as Fetcher | undefined;
  return {
    sourceUrl: str('SOURCE_URL') ?? (UPSTREAM_SOURCE || undefined),
    publicUrl: str('PUBLIC_URL')?.replace(/\/+$/, ''),
    minecraftServer: str('MINECRAFT_SERVER'),
    minecraftName: str('MINECRAFT_NAME'),
    board: typeof board?.fetch === 'function' ? board : undefined,
    boardPaths: str('BOARD_PATHS'),
    boardOrigin: str('BOARD_ORIGIN'),
    boardName: str('BOARD_NAME'),
  };
}
