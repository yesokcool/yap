// Phone/desktop pushes through ntfy (https://ntfy.sh, or a self-hosted
// server via NTFY_SERVER). Each person gets their own unguessable topic and
// subscribes to it in the ntfy app. Pushes are fire-and-forget. (Who a
// message mentions: web/js/mentions.js, shared with the web client.)

import type { NotifyMode } from './protocol';

export const NOTIFY_MODES: NotifyMode[] = ['off', 'pings', 'all'];

export function newTopic(): string {
  const alphabet = 'abcdefghijkmnpqrstuvwxyz23456789';
  const bytes = crypto.getRandomValues(new Uint8Array(24));
  return `yap-${[...bytes].map((b) => alphabet[b % alphabet.length]).join('')}`;
}

export interface Push {
  topic: string;
  title: string;
  message: string;
  tags?: string[];
  priority?: 1 | 2 | 3 | 4 | 5;
  /** Opened when the push is tapped: this server's address. */
  click?: string;
}

/** The ntfy server pushes go through (and where people subscribe): NTFY_SERVER, else ntfy.sh. */
export const ntfyServer = (env: Env) => (env.NTFY_SERVER || 'https://ntfy.sh').replace(/\/+$/, '');

export async function publish(env: Env, push: Push): Promise<void> {
  const server = ntfyServer(env);
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (env.NTFY_TOKEN) headers.Authorization = `Bearer ${env.NTFY_TOKEN}`;
  try {
    const res = await fetch(`${server}/`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        topic: push.topic,
        title: push.title.slice(0, 120),
        message: push.message.slice(0, 400) || ' ',
        tags: push.tags,
        priority: push.priority ?? 3,
        ...(push.click ? { click: push.click } : {}),
      }),
    });
    if (!res.ok) console.warn('ntfy publish failed', res.status, await res.text());
  } catch (err) {
    console.warn('ntfy publish error', err);
  }
}
