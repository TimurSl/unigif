// Unigif service worker. It does one job: receive the Android share target (`POST /share`).
//
// A share carries files and/or a link and arrives as a form POST that the page cannot see, so the
// worker parks it in Cache Storage and redirects to `/share-received`. The app picks it up from there
// once it is signed in (see src/lib/share.ts), uploads it, and lands in Unsorted. Nothing else is
// intercepted or cached: the API, media and app files always come from the network.

const STASH = 'unigif-share';

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (event) => event.waitUntil(self.clients.claim()));

self.addEventListener('fetch', (event) => {
  const { request } = event;
  if (request.method !== 'POST') return;
  const url = new URL(request.url);
  if (url.origin === self.location.origin && url.pathname === '/share') {
    event.respondWith(receive(request));
  }
});

async function receive(request) {
  try {
    const form = await request.formData();
    const cache = await caches.open(STASH);
    const stamp = Date.now();
    let n = 0;
    for (const value of form.getAll('file')) {
      if (typeof value === 'string') continue;
      await cache.put(
        `/share-stash/${stamp}-${n++}-file`,
        new Response(value, {
          headers: {
            'Content-Type': value.type || 'application/octet-stream',
            'X-Filename': encodeURIComponent(value.name || 'shared'),
          },
        }),
      );
    }
    const note = {};
    for (const key of ['title', 'text', 'url']) {
      const v = form.get(key);
      if (typeof v === 'string' && v) note[key] = v;
    }
    if (Object.keys(note).length) {
      await cache.put(
        `/share-stash/${stamp}-note`,
        new Response(JSON.stringify(note), { headers: { 'Content-Type': 'application/json' } }),
      );
    }
  } catch (err) {
    // Fall through to the app: it shows nothing to import rather than an error page.
    console.error('unigif: could not receive the shared content', err);
  }
  return Response.redirect('/share-received', 303);
}
