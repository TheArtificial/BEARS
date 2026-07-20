# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.92.0

FROM rust:${RUST_VERSION}-alpine AS build
ARG APP_NAME=den
ARG DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres?sslmode=disable
ARG SQLX_OFFLINE=false
ARG CARGO_BUILD_JOBS=2
WORKDIR /app
RUN apk add --no-cache clang lld musl-dev git

# Pinned nightly toolchain, used ONLY to enable `-Z checksum-freshness`, which
# makes Cargo decide rebuilds from file content hashes instead of mtimes. Docker
# `COPY` rewrites every file's mtime on each build, so on stable Cargo recompiles
# the entire workspace on every deploy. With checksum freshness, unchanged crates
# are reused. Tracking issue: https://github.com/rust-lang/cargo/issues/14136
#
# REVERT TO STABLE when the feature stabilizes: set RUST_NIGHTLY to an empty
# string and bump RUST_VERSION to the stable release that ships it. The build
# then runs on stable with no `-Z` flag and keeps working — it just loses the
# optimization until the stabilized config form (`[unstable]`/`[build]`
# checksum-freshness) is wired in. Pinned by date for reproducibility; bump
# freely, any recent nightly carries the feature.
#
# Caveat: files read by build scripts (e.g. minijinja-embed template embedding)
# still use mtimes even with checksum-freshness, so template-embedding edge
# crates may still recompile; the leaf crates get the benefit.
ARG RUST_NIGHTLY=nightly-2026-06-01
RUN if [ -n "${RUST_NIGHTLY}" ]; then \
        rustup toolchain install "${RUST_NIGHTLY}" --profile minimal && \
        rustup default "${RUST_NIGHTLY}"; \
    fi

COPY . .
# BuildKit cache mounts persist Cargo state across deployments:
#   registry/git — avoid re-downloading crate sources.
#   target       — preserve compiled artifacts. External dependencies recompile
#                  only when Cargo.lock changes; with `-Z checksum-freshness`
#                  (above) unchanged workspace crates are reused across builds
#                  instead of recompiling on every COPY-rewritten mtime.
RUN --mount=type=cache,target=/usr/local/cargo/registry/,id=den-cargo-registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git/db,id=den-cargo-git,sharing=locked \
    --mount=type=cache,target=/app/target,id=den-build-target,sharing=locked \
    export DATABASE_URL="${DATABASE_URL}" SQLX_OFFLINE="${SQLX_OFFLINE}" && \
    if [ -n "${RUST_NIGHTLY}" ]; then FRESHNESS="-Z checksum-freshness"; else FRESHNESS=""; fi && \
    echo "DATABASE_URL=$DATABASE_URL" && echo "SQLX_OFFLINE=$SQLX_OFFLINE" && echo "CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS" && echo "FRESHNESS=${FRESHNESS:-<stable: none>}" && \
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS}" cargo build ${FRESHNESS} --locked --release --features=production --message-format=short && \
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

# Sandbox provider variant (compose service `bears-sandbox-provider`): the
# same server binary plus a docker CLI for driving the `bears-sandbox-engine`
# dind daemon over DOCKER_HOST (TLS TCP). No docker daemon or socket in this
# image, so the appuser drop needs no socket-group handling.
# Build with `--target sandbox-provider`.
FROM final AS sandbox-provider
RUN apk add --no-cache docker-cli

# The provider offers one-click sandbox image builds. Bake the immutable build
# context into this image: PaaS deployments may replace or hide repository
# checkouts, making runtime bind mounts unreliable. `sandbox_packaging` and
# `bear_armature` are BuildKit named contexts supplied by Compose.
COPY --from=sandbox_packaging / /opt/bears/sandbox-build/packaging/sandbox-image
COPY --from=bear_armature / /opt/bears/sandbox-build/tools/bear-armature
COPY --from=build /app/Cargo.toml /opt/bears/sandbox-build/services/den/Cargo.toml
COPY --from=build /app/crates/bearwire-protocol /opt/bears/sandbox-build/services/den/crates/bearwire-protocol

# Keep the plain server image as the default build target (the stage above
# would otherwise become it, being last).
FROM final
