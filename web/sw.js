// Keeps the page's own files so Flowie opens without the server; the outline itself is in OPFS.
// Network first (a new build is used as soon as it is there), the cache when offline. The sync
// API (/v1/) is never cached.
const CACHE = "flowie-shell";

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

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
