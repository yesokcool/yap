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
      data: { url: '/' },
    });
  })());
});

self.addEventListener('notificationclick', (event) => {
  event.notification.close();
  event.waitUntil((async () => {
    const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
    const open = windows.find((w) => new URL(w.url).pathname === '/');
    return open ? open.focus() : self.clients.openWindow('/');
  })());
});
