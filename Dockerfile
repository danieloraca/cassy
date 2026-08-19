FROM rust:1-bookworm AS builder

RUN apt-get update \
    && apt-get install --yes --no-install-recommends libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY static ./static
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked \
    && cp /app/target/release/cassy /tmp/cassy

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 cassy \
    && mkdir -p /data \
    && chown cassy:cassy /data

COPY --from=builder /tmp/cassy /usr/local/bin/cassy

USER cassy
ENV PORT=8944 \
    DATABASE_PATH=/data/cassy.sqlite3 \
    VAPID_PRIVATE_KEY_PATH=/run/secrets/vapid_private \
    VAPID_SUBJECT=mailto:admin@cassy.local \
    RUST_LOG=cassy=info,tower_http=info

EXPOSE 8944
VOLUME ["/data"]

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:8944/health >/dev/null || exit 1

CMD ["cassy"]
