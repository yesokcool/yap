// ── Browser pushes (Web Push via /sw.js) ─────────────────────────────────────
// The service worker shows pings, @mentions and join requests while Yap is
// closed or in the background; the page then leaves those to it.

import { loadConfig } from '../config.js';
import { unb64u } from '../vault.js';
import { conn, state } from './state.js';

const pushSupported = () => 'serviceWorker' in navigator && 'PushManager' in window && typeof Notification !== 'undefined';

export async function currentPushSubscription() {
  if (!pushSupported()) return null;
  const reg = await navigator.serviceWorker.getRegistration('/');
  return (await reg?.pushManager.getSubscription()) ?? null;
}


export async function enableBrowserPush() {
  if (!pushSupported()) throw new Error('This browser can’t do pushes. On iPhone, add Yap to your Home Screen and open it from there.');
  const config = await loadConfig();
  if (!config.push?.publicKey) throw new Error('Browser pushes aren’t set up on this server yet.');
  if ((await Notification.requestPermission()) !== 'granted') throw new Error('Notifications are blocked for this site in your browser settings.');
  const reg = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
  await navigator.serviceWorker.ready;
  const sub = (await reg.pushManager.getSubscription()) ?? (await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: unb64u(config.push.publicKey) }));
  sendPushSubscription(sub);
  state.browserPush = true;
}

export async function disableBrowserPush() {
  const sub = await currentPushSubscription();
  if (sub) {
    conn?.send({ t: 'push.unsubscribe', endpoint: sub.endpoint });
    await sub.unsubscribe();
  }
  state.browserPush = false;
}

function sendPushSubscription(sub) {
  const { endpoint, keys } = sub.toJSON();
  conn?.send({ t: 'push.subscribe', endpoint, keys });
}

/** After every (re)connect: keep the server's copy of this browser's subscription fresh. */
export async function syncBrowserPush() {
  const sub = await currentPushSubscription().catch(() => null);
  state.browserPush = Boolean(sub);
  if (sub) sendPushSubscription(sub);
}
