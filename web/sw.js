// Keeps the page's own files so Flowie opens without the server; the outline itself is in OPFS.
// The build (vite.config.ts) fills in FILES, every file it wrote, and VERSION, a hash of them:
// a new build caches its files in a new cache when installed and drops the old one. Network
// first (a new build is used as soon as it is there), the cache when offline. The sync API
// (/v1/) is never cached.
const FILES = self.__FLOWIE_FILES__;
const CACHE = `flowie-${self.__FLOWIE_VERSION__}`;

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.addAll(FILES))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((key) => key.startsWith("flowie-") && key !== CACHE).map((key) => caches.delete(key))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (event.request.method !== "GET" || url.origin !== location.origin || url.pathname.startsWith("/v1/")) return;
  event.respondWith(
    fetch(event.request)
      .then((response) => {
        if (response.ok) {
          const copy = response.clone();
          event.waitUntil(caches.open(CACHE).then((cache) => cache.put(event.request, copy)));
        }
        return response;
      })
      .catch(async () => (await caches.match(event.request, { ignoreSearch: true })) ?? Response.error()),
  );
});
