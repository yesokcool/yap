// The extras' pure rules (PROTOCOL.md › Extras): validation, dice, quiet
// hours, clip loudness. No Durable Object state here, so they're unit-tested
// directly (test/extras.test.mjs).
// (`.ts` on the imports so Node can load this file straight for the tests.)
import { LIMITS, type Away, type QuietHours, type Roll } from './protocol.ts';
import { ClientError, cleanName } from './validate.ts';

/** A doodle reaction (`<d:mediaId>`, a tiny drawing from POST /api/doodle): its media id. */
export const doodleId = (emoji: string) => /^<d:([a-f0-9]{32})>$/.exec(emoji)?.[1] ?? null;

/** Pushes show spoilers as blocks, never their text. */
export const hideSpoilers = (text: string) => text.replace(/\|\|((?:(?!\|\|)[^\n])+)\|\|/g, '▒▒▒');

/** Quiet hours: minutes after midnight in a real IANA time zone; a snooze within a week. */
export function cleanQuiet(value: unknown): QuietHours {
  const v = (typeof value === 'object' && value !== null ? value : {}) as Record<string, unknown>;
  const minute = (n: unknown) => (Number.isInteger(n) && (n as number) >= 0 && (n as number) < 1440 ? (n as number) : -1);
  const from = minute(v.from);
  const to = minute(v.to);
  const tz = typeof v.tz === 'string' && v.tz.length <= 64 ? v.tz : '';
  let zoneOk = false;
  try {
    zoneOk = !!tz && !!new Intl.DateTimeFormat('en-US', { timeZone: tz }).format(0);
  } catch {}
  if (from < 0 || to < 0 || !zoneOk) throw new ClientError('bad_quiet', 'Quiet hours need a start, an end and a time zone.');
  const now = Date.now();
  const until = v.until === null || v.until === undefined ? null : Number(v.until);
  if (until !== null && !(until > now && until <= now + 7 * 86_400_000)) throw new ClientError('bad_quiet', 'Snooze for at most a week.');
  return { from, to, tz, mentions: v.mentions === true, until };
}

/** Is it quiet time for these settings right now? (`from` = `to` means no daily window.) */
export function isQuietNow(q: QuietHours, now = Date.now()): boolean {
  if (q.until && now < q.until) return true;
  if (q.from === q.to) return false;
  let minute: number;
  try {
    const parts = new Intl.DateTimeFormat('en-GB', { timeZone: q.tz, hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).formatToParts(now);
    minute = Number(parts.find((p) => p.type === 'hour')?.value) * 60 + Number(parts.find((p) => p.type === 'minute')?.value);
  } catch {
    return false;
  }
  return q.from < q.to ? minute >= q.from && minute < q.to : minute >= q.from || minute < q.to;
}

/** An away message: 1–80 chars, ending within a week (or "until I'm back"). */
export function cleanAway(value: unknown): Away {
  const v = (typeof value === 'object' && value !== null ? value : {}) as Record<string, unknown>;
  const text = cleanName(v.text, LIMITS.awayChars);
  if (!text) throw new ClientError('bad_away', 'Say what you’re up to (up to 80 characters).');
  const now = Date.now();
  const until = v.until === null || v.until === undefined ? null : Number(v.until);
  if (until !== null && !(until > now && until <= now + 7 * 86_400_000)) throw new ClientError('bad_away', 'Pick an end time within the next week.');
  return { text, until, since: now };
}

/** 2–10 distinct options, 1–80 chars each. */
export function cleanPoll(value: unknown): { options: string[] } {
  const raw = (value as { options?: unknown } | null)?.options;
  const options = Array.isArray(raw) ? raw.map((o) => cleanName(o, LIMITS.pollOptionChars)) : [];
  const distinct = new Set(options.map((o) => o.toLowerCase()));
  if (options.length < 2 || options.length > LIMITS.pollOptions || options.some((o) => !o) || distinct.size !== options.length) {
    throw new ClientError('bad_poll', `A poll needs 2–${LIMITS.pollOptions} different options.`);
  }
  return { options };
}

/** A fair random integer in 1..n. */
function roll1(n: number): number {
  const limit = Math.floor(0x1_0000_0000 / n) * n;
  for (;;) {
    const r = crypto.getRandomValues(new Uint32Array(1))[0];
    if (r < limit) return (r % n) + 1;
  }
}

/** `/roll [spec]` or `/flip` → the server's result (PROTOCOL.md › Extras › Polls, /roll and /flip); null if it's not a command. */
export function rollDice(body: string): { body: string; roll: Roll } | null {
  const m = /^\/(roll|flip)(?:\s+(\S+))?\s*$/i.exec(body.trim());
  if (!m) return null;
  const bad = () => new ClientError('bad_roll', 'Try /roll, /roll 20, /roll 2d6, /roll 1d20+3 or /flip.');
  if (m[1].toLowerCase() === 'flip') {
    if (m[2]) throw bad();
    const r = roll1(2) - 1;
    return { body: r === 0 ? '🪙 Heads' : '🪙 Tails', roll: { kind: 'flip', spec: 'coin', rolls: [r], total: r } };
  }
  const spec = (m[2] ?? '100').toLowerCase();
  if (/^\d{1,7}$/.test(spec)) {
    const n = Number(spec);
    if (n < 2 || n > 1_000_000) throw bad();
    const r = roll1(n);
    return { body: `🎲 1–${n}: ${r}`, roll: { kind: 'roll', spec: String(n), rolls: [r], total: r } };
  }
  const d = /^(\d{1,2})?d(\d{1,4})(?:([+-])(\d{1,4}))?$/.exec(spec);
  if (!d) throw bad();
  const count = Number(d[1] ?? 1);
  const sides = Number(d[2]);
  const bonus = d[3] ? Number(d[4]) * (d[3] === '-' ? -1 : 1) : 0;
  if (count < 1 || count > 20 || sides < 2 || sides > 1000 || Math.abs(bonus) > 1000) throw bad();
  const rolls = Array.from({ length: count }, () => roll1(sides));
  const total = rolls.reduce((a, b) => a + b, 0) + bonus;
  const name = `${count}d${sides}${bonus ? (bonus > 0 ? `+${bonus}` : bonus) : ''}`;
  const sum = `${rolls.join(' + ')}${bonus ? ` ${bonus > 0 ? '+' : '−'} ${Math.abs(bonus)}` : ''}`;
  return { body: count === 1 && !bonus ? `🎲 ${name}: ${total}` : `🎲 ${name}: ${sum} = ${total}`, roll: { kind: 'roll', spec: name, rolls, total } };
}

/**
 * A soundboard clip: WAV, PCM 16-bit mono, 8–48 kHz, up to `maxMs`. Returns
 * its length and a loudness cap — `gain = min(1, 0.25 / rms, 0.71 / peak)`,
 * about −12 dBFS RMS and −3 dBFS peak, never louder than it came in — or why
 * it's refused. Clients play it at most at `gain` (PROTOCOL.md › Soundboard).
 */
export function checkWav(b: Uint8Array, maxMs: number): { ms: number; gain: number } | string {
  const view = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const ascii = (at: number, n: number) => String.fromCharCode(...b.subarray(at, at + n));
  if (b.length < 44 || ascii(0, 4) !== 'RIFF' || ascii(8, 4) !== 'WAVE') return 'Send a WAV file.';
  let fmt: { format: number; channels: number; rate: number; bits: number } | null = null;
  let data: { start: number; size: number } | null = null;
  for (let at = 12; at + 8 <= b.length;) {
    const id = ascii(at, 4);
    const size = view.getUint32(at + 4, true);
    const body = at + 8;
    if (id === 'fmt ' && body + 16 <= b.length) {
      fmt = { format: view.getUint16(body, true), channels: view.getUint16(body + 2, true), rate: view.getUint32(body + 4, true), bits: view.getUint16(body + 14, true) };
    } else if (id === 'data') {
      data = { start: body, size: Math.min(size, b.length - body) };
      break;
    }
    at = body + size + (size & 1);
  }
  if (!fmt || !data) return 'That WAV file looks broken.';
  if (fmt.format !== 1 || fmt.channels !== 1 || fmt.bits !== 16 || fmt.rate < 8000 || fmt.rate > 48000) return 'Use 16-bit mono PCM WAV (8–48 kHz).';
  const samples = Math.floor(data.size / 2);
  const ms = Math.round((samples / fmt.rate) * 1000);
  if (ms < 50 || ms > maxMs + 50) return `Clips can be up to ${maxMs / 1000} s long.`;
  let peak = 0;
  let sum = 0;
  for (let i = 0; i < samples; i++) {
    const v = view.getInt16(data.start + i * 2, true) / 32768;
    peak = Math.max(peak, Math.abs(v));
    sum += v * v;
  }
  if (peak < 1e-3) return 'That clip is silent.';
  const rms = Math.sqrt(sum / samples);
  return { ms, gain: Math.round(Math.min(1, 0.25 / rms, 0.71 / peak) * 1000) / 1000 };
}
