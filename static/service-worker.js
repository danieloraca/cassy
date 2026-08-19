const CACHE_NAME = "cassy-v1";
const APP_SHELL = ["/", "/app.css", "/app.js", "/manifest.webmanifest", "/icon.svg"];

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(CACHE_NAME).then((cache) => cache.addAll(APP_SHELL)));
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((key) => key !== CACHE_NAME).map((key) => caches.delete(key))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  if (event.request.method !== "GET") return;
  const url = new URL(event.request.url);
  if (url.origin !== self.location.origin || url.pathname.startsWith("/api/")) return;

  event.respondWith(
    fetch(event.request)
      .then((response) => {
        const copy = response.clone();
        caches.open(CACHE_NAME).then((cache) => cache.put(event.request, copy));
        return response;
      })
      .catch(() => caches.match(event.request)),
  );
});

self.addEventListener("push", (event) => {
  const payload = event.data?.json() || {
    title: "Cassy",
    body: "You have a new message",
    conversation_id: "demo",
  };

  event.waitUntil(
    self.registration.showNotification(payload.title || "Cassy", {
      body: payload.body || "You have a new message",
      icon: "/icon.svg",
      badge: "/icon.svg",
      tag: `conversation-${payload.conversation_id}`,
      data: { conversation_id: payload.conversation_id },
    }),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const conversation = encodeURIComponent(event.notification.data?.conversation_id || "demo");
  const target = `/?conversation=${conversation}`;

  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((clients) => {
      const existing = clients.find((client) => "focus" in client);
      return existing
        ? existing.navigate(target).then((client) => client?.focus())
        : self.clients.openWindow(target);
    }),
  );
});
