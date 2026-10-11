// The ICE servers voice calls use (GET /api/ice; PROTOCOL.md › Voice).

const STUN: RTCIceServerLike = { urls: ['stun:stun.cloudflare.com:3478', 'stun:stun.l.google.com:19302'] };

interface RTCIceServerLike {
  urls: string | string[];
  username?: string;
  credential?: string;
}

/**
 * STUN always; TURN (relay for strict NATs, e.g. phones on cellular) when one
 * is configured:
 * - a relay with a fixed login (TURN_URLS + TURN_USERNAME + TURN_CREDENTIAL),
 *   e.g. a free plan with no card on file — when its free quota runs out it
 *   simply stops relaying, and calls fall back to direct connections; or
 * - a Cloudflare Realtime TURN key (short-lived credentials; needs billing).
 */
export async function iceServers(env: Env): Promise<RTCIceServerLike[]> {
  if (env.TURN_URLS && env.TURN_USERNAME && env.TURN_CREDENTIAL) {
    const urls = env.TURN_URLS.split(',').map((u) => u.trim()).filter((u) => /^turns?:/.test(u));
    if (urls.length) return [STUN, { urls, username: env.TURN_USERNAME, credential: env.TURN_CREDENTIAL }];
  }
  if (!env.TURN_KEY_ID || !env.TURN_KEY_API_TOKEN) return [STUN];
  try {
    const res = await fetch(
      `https://rtc.live.cloudflare.com/v1/turn/keys/${env.TURN_KEY_ID}/credentials/generate-ice-servers`,
      {
        method: 'POST',
        headers: { Authorization: `Bearer ${env.TURN_KEY_API_TOKEN}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ ttl: 86400 }),
      },
    );
    if (!res.ok) throw new Error(`TURN credentials: HTTP ${res.status}`);
    const body = (await res.json()) as { iceServers: RTCIceServerLike | RTCIceServerLike[] };
    const servers = Array.isArray(body.iceServers) ? body.iceServers : [body.iceServers];
    // Port 53 URLs stall in browsers; Cloudflare's docs say to drop them.
    return servers.map((s) => ({
      ...s,
      urls: (Array.isArray(s.urls) ? s.urls : [s.urls]).filter((u) => !/:53(\?|$)/.test(u)),
    }));
  } catch (err) {
    console.error('TURN unavailable, falling back to STUN only', err);
    return [STUN];
  }
}
