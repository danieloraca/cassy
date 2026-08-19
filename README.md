# cassy

A small Rust chat backend intended to run on a Raspberry Pi. The first version
uses Axum, WebSockets, and SQLite in WAL mode.

## Run with Docker

```sh
docker compose up --build -d
docker compose logs -f
```

The HTTP server is available at `http://localhost:8944`.

## Try it

```sh
curl http://localhost:8944/health

curl -X POST http://localhost:8944/api/messages \
  -H 'content-type: application/json' \
  -d '{"conversation_id":"demo","sender_id":"daniel","body":"hello"}'

curl http://localhost:8944/api/conversations/demo/messages
```

WebSocket clients can subscribe to `ws://localhost:8944/ws/demo`.
# cassy
