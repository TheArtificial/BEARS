# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.92.0

FROM rust:${RUST_VERSION}-alpine AS build
ARG APP_NAME=den
ARG GIT_SHA=
ARG DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres?sslmode=disable
ARG SQLX_OFFLINE=false
ARG CARGO_BUILD_JOBS=2
ENV GIT_SHA=${GIT_SHA}
WORKDIR /app
RUN apk add --no-cache clang lld musl-dev git
COPY . .
# BuildKit cache mounts persist Cargo state across deployments:
#   registry/git — avoid re-downloading crate sources.
#   target       — preserve compiled artifacts. External dependencies are reused
#                  across builds and only recompile when Cargo.lock changes;
#                  workspace crates recompile each build because COPY rewrites
#                  source mtimes (Cargo's freshness is mtime-based on stable).
RUN --mount=type=cache,target=/usr/local/cargo/registry/,id=den-cargo-registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git/db,id=den-cargo-git,sharing=locked \
    --mount=type=cache,target=/app/target,id=den-build-target,sharing=locked \
    export DATABASE_URL="${DATABASE_URL}" SQLX_OFFLINE="${SQLX_OFFLINE}" && \
    echo "DATABASE_URL=$DATABASE_URL" && echo "SQLX_OFFLINE=$SQLX_OFFLINE" && echo "CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS" && \
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS}" cargo build --locked --release --features=production --message-format=short && \
    cp "./target/release/${APP_NAME}" /tmp/server

FROM alpine:3.18 AS final

ARG UID=10001
RUN apk add --no-cache su-exec wget git \
    && adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    appuser

COPY --from=build /tmp/server /bin/server
COPY docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod +x /docker-entrypoint.sh

# Runtime env (set in Coolify / compose — not baked into the image):
#   AGENT_RUNTIME — native (default) or letta (legacy external runtime).
#   BEAR_SQLITE_DATA_DIR — per-Bear SQLite store for native runtime (default /var/lib/den/bear-sqlite).
#     Mount a persistent volume at this path in production; entrypoint chowns it for appuser.
#   LLM_API_URL — Bifrost OpenAI-compatible gateway (required for native runtime).
# Legacy letta runtime only: LETTA_BASE_URL, CODEPOOL_BASE_URL, LETTA_MEMFS_SERVICE_URL, LETTA_PG_URI.
# See services/den/COOLIFY_DEPLOY.md.

EXPOSE 3000

ENTRYPOINT ["/docker-entrypoint.sh"]
CMD ["/bin/server"]
