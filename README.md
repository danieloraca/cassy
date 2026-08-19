# cassy

A small Rust chat backend intended to run on a Raspberry Pi. The first version
uses Axum, WebSockets, and SQLite in WAL mode.

## Run with Docker

```sh
docker compose up --build -d
docker compose logs -f
```

The browser chat and HTTP API are available at `http://localhost:8944`.

Open that address in two browser windows, choose the same conversation and
different names, and messages will arrive live over WebSocket.

## Try it

```sh
curl http://localhost:8944/health

curl -X POST http://localhost:8944/api/messages \
  -H 'content-type: application/json' \
  -d '{"conversation_id":"demo","sender_id":"daniel","body":"hello"}'

curl http://localhost:8944/api/conversations/demo/messages
```

WebSocket clients can subscribe to `ws://localhost:8944/ws/demo`.

## PWA and Web Push

The browser client registers a service worker and can be installed as a PWA.
Before the first Compose start, generate the VAPID private key used to sign Web
Push requests:

```sh
mkdir -p .secrets
openssl ecparam -name prime256v1 -genkey -noout \
  -out .secrets/vapid_private.pem
chmod 600 .secrets/vapid_private.pem
```

Open Cassy and select **Notifications** to subscribe the current browser and
conversation. Browsers allow service workers and Web Push on `localhost`; when
the service runs on the Pi, put it behind HTTPS. On iPhone and iPad, add Cassy
to the Home Screen before enabling notifications.

The current profile ID is local to each browser installation. It demonstrates
persistent identity, but it is not secure authentication yet.
# cassy
