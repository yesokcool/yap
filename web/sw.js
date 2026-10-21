// Yap's service worker. It only shows browser pushes (pings, @mentions,
// join requests) while Yap is closed or in the background: no caching, no
// offline mode, no fetch handling.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));

self.addEventListener('push', (event) => {
  let data = {};
  try {
    data = event.data?.json() ?? {};
  } catch {}
  event.waitUntil((async () => {
    const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    if (windows.some((w) => w.focused)) return; // you're looking at Yap right now
    await self.registration.showNotification(data.title || 'Yap', {
      body: data.body || '',
      tag: data.tag || 'yap',
      renotify: true,
      icon: '/icon-192.png',
      badge: '/icon-192.png',
      // A push about a message (a mention, a reply, "all" mode) opens that message.
      data: typeof data.channelId === 'string' ? { channelId: data.channelId, messageId: typeof data.messageId === 'string' ? data.messageId : null } : {},
    });
  })());
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  const { channelId, messageId } = event.notification.data ?? {};
  event.waitUntil((async () => {
    const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    const open = windows.find((w) => new URL(w.url).pathname === '/');
    if (open) {
      // Yap is open: it goes to the message itself (no reload).
      if (channelId) open.postMessage({ t: 'yap.open', channelId, messageId });
      return open.focus();
    }
    // Closed: start it at the message (#channel=…&message=…, read once at boot).
    const hash = channelId ? `#${new URLSearchParams({ channel: channelId, ...(messageId ? { message: messageId } : {}) })}` : '';
    return self.clients.openWindow(`/${hash}`);
  })());
});
