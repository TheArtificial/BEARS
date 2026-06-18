# syntax=docker/dockerfile:1

# Multi-stage build with cargo-chef so dependency compilation is cached in a
# Docker layer that only invalidates when Cargo.lock / Cargo.toml / build.rs
# dependency metadata changes — not when templates or other application sources
# change. GitHub Actions imports/exports buildx cache; this structure makes
# dependency layer reuse visible as a BuildKit `CACHED` step instead of running
# `cargo chef cook` on every build and merely reporting Cargo artifacts as
# `Fresh`.

ARG RUST_VERSION=1.92.0

FROM rust:${RUST_VERSION}-alpine AS rust-base
WORKDIR /app
RUN apk add --no-cache clang lld musl-dev git

FROM rust-base AS chef-base
RUN cargo install cargo-chef --version 0.1.71 --locked

FROM chef-base AS planner
# Workspace-aware recipe: `cargo chef prepare` runs `cargo metadata`, which needs
# every workspace member's Cargo.toml present, so copy the whole context (minus
# .dockerignore'd target/.git/etc.). `prepare` is cheap and this layer is not
# reused across source edits anyway; the expensive dependency compile lives in
# the `cacher` stage and is gated only by recipe.json, so build-time caching is
# unaffected. (Per docs/roadmap/DEN_CRATE_SPLIT_PLAN.md the crate split made the
# previous single-crate selective COPY brittle.)
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef-base AS cacher
COPY --from=planner /app/recipe.json recipe.json
COPY .cargo ./.cargo
COPY Cargo.toml Cargo.lock build.rs ./
# Keep dependency build artifacts in the committed layer at /app/target. Do not
# put CARGO_TARGET_DIR behind a cache mount here: doing so makes CI rerun
# `cargo chef cook` and print many `Fresh` lines instead of reusing this step as
# a normal Docker layer. Registry/git cache mounts still avoid re-downloading
# crates when this layer genuinely has to be rebuilt.
RUN --mount=type=cache,target=/usr/local/cargo/registry/,id=den-cargo-registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git/db,id=den-cargo-git,sharing=locked \
    cargo chef cook --release --recipe-path recipe.json

FROM rust-base AS build
ARG APP_NAME=den
ARG GIT_SHA=
ARG DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres?sslmode=disable
ARG SQLX_OFFLINE=false
ARG CARGO_BUILD_JOBS=2
ENV GIT_SHA=${GIT_SHA}
COPY . .
# Keep only Cargo registry/git caches in the final application build. Do not
# mount or seed /app/target here: target fingerprints/artifacts can survive
# source-only Docker rebuilds and produce stale deployed binaries. This makes
# the final Den build slower than a target-seeded build, but deterministic: the
# release binary is compiled from the source copied in this Docker layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry/,id=den-cargo-registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git/db,id=den-cargo-git,sharing=locked \
    export DATABASE_URL="${DATABASE_URL}" SQLX_OFFLINE="${SQLX_OFFLINE}" && \
    echo "DATABASE_URL=$DATABASE_URL" && echo "SQLX_OFFLINE=$SQLX_OFFLINE" && echo "CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS" && \
    CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS}" cargo build --locked --release --features=production --message-format=short && \
    cp "./target/release/${APP_NAME}" /bin/server

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

COPY --from=build /bin/server /bin/
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
