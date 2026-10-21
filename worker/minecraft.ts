import { connect } from 'cloudflare:sockets';

/**
 * Minecraft "Server List Ping" (the same status query the game's server list
 * uses) over a raw TCP socket. Read-only: handshake → status request → JSON.
 * Only ever pointed at our own configured server (no user-supplied hosts).
 */

export interface MinecraftStatus {
  online: boolean;
  address: string;
  checkedAt: number;
  ms?: number;
  version?: string;
  motd?: string;
  players?: { online: number; max: number; names: string[] };
  error?: string;
}

const varint = (n: number): number[] => {
  const out: number[] = [];
  do {
    let b = n & 0x7f;
    n >>>= 7;
    if (n) b |= 0x80;
    out.push(b);
  } while (n);
  return out;
};

const packet = (id: number, payload: number[]) => {
  const body = [...varint(id), ...payload];
  return new Uint8Array([...varint(body.length), ...body]);
};

function readVarint(buf: Uint8Array, offset: number): [value: number, next: number] | null {
  let value = 0;
  for (let i = 0; i < 5; i++) {
    if (offset + i >= buf.length) return null;
    const b = buf[offset + i];
    value |= (b & 0x7f) << (7 * i);
    if (!(b & 0x80)) return [value, offset + i + 1];
  }
  throw new Error('bad varint');
}

/** Chat component (string or {text, extra}) → plain text, minus § colour codes. */
function plain(component: unknown): string {
  if (typeof component === 'string') return component.replace(/§./g, '');
  if (!component || typeof component !== 'object') return '';
  const c = component as { text?: unknown; extra?: unknown[] };
  return plain(c.text ?? '') + (Array.isArray(c.extra) ? c.extra.map(plain).join('') : '');
}

export async function ping(host: string, port: number, timeoutMs = 4000): Promise<MinecraftStatus> {
  const address = `${host}:${port}`;
  const started = Date.now();
  const socket = connect({ hostname: host, port });
  let timer: number | undefined;
  try {
    const query = (async () => {
      const writer = socket.writable.getWriter();
      const hostBytes = [...new TextEncoder().encode(host)];
      await writer.write(packet(0x00, [...varint(767), ...varint(hostBytes.length), ...hostBytes, (port >> 8) & 0xff, port & 0xff, ...varint(1)]));
      await writer.write(packet(0x00, []));
      const reader = socket.readable.getReader();
      let buf = new Uint8Array(0);
      for (;;) {
        const header = readVarint(buf, 0);
        if (header && buf.length >= header[1] + header[0]) {
          const body = buf.slice(header[1], header[1] + header[0]);
          const id = readVarint(body, 0)!;
          const len = readVarint(body, id[1])!;
          return JSON.parse(new TextDecoder().decode(body.slice(len[1], len[1] + len[0])));
        }
        const { value, done } = await reader.read();
        if (done) throw new Error('connection closed');
        const merged = new Uint8Array(buf.length + value.length);
        merged.set(buf);
        merged.set(value, buf.length);
        buf = merged;
        if (buf.length > 1 << 20) throw new Error('response too large');
      }
    })();
    // Once the timeout wins, closing the socket fails the read: nobody is listening for that any more.
    query.catch(() => {});
    const timeout = new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('timed out')), timeoutMs) as unknown as number;
    });
    const status = (await Promise.race([query, timeout])) as {
      version?: { name?: string };
      players?: { online?: number; max?: number; sample?: { name?: string }[] };
      description?: unknown;
    };
    return {
      online: true,
      address,
      checkedAt: Date.now(),
      ms: Date.now() - started,
      version: status.version?.name,
      motd: plain(status.description).trim(),
      players: {
        online: status.players?.online ?? 0,
        max: status.players?.max ?? 0,
        names: (status.players?.sample ?? []).map((p) => String(p.name ?? '')).filter(Boolean).slice(0, 20),
      },
    };
  } catch (err) {
    return { online: false, address, checkedAt: Date.now(), error: err instanceof Error ? err.message : String(err) };
  } finally {
    if (timer !== undefined) clearTimeout(timer);
    socket.close().catch(() => {});
  }
}
