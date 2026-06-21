# Den — Coolify deployment guide

**Stack context:** Den is the BEARS **control plane** (Rust / Axum): provisioning, **users↔bears** membership, routing, and related HTTP surfaces. It sits alongside **Letta**, **Bifrost**, and optional **Outline** (Cabinet) per [DEPLOYMENT.md](../docs/deployment/DEPLOYMENT.md) and [PLAN.md](../docs/planning/PLAN.md). For architecture, see [DEN_ARCHITECTURE.md](../docs/architecture/DEN_ARCHITECTURE.md).

## Overview

- **One image, one binary** — built from [`Dockerfile`](Dockerfile) in this directory. Runtime behavior is controlled with **environment variables** (`RUN_WEB`, `RUN_API`, `RUN_WORKERS`, ports, `DATABASE_URL`, …). Deeper reference: [`docs/deploy.md`](docs/deploy.md) and [`docs/infrastructure-and-ops.md`](docs/infrastructure-and-ops.md).
- **PostgreSQL is mandatory** — Den exits on startup if it cannot use `DATABASE_URL`. The **database must exist** (empty is fine); on each start Den runs **embedded SQLx migrations** from [`migrations/`](migrations/) against that URL before serving traffic, so routine deploys do not need a separate migration job. By default migrations are **strict** (see `SQLX_MIGRATE_IGNORE_MISSING` in [`.env.example`](.env.example) / [`docs/deploy.md`](docs/deploy.md)—leave it unset in production).
- **SQLx at image build time** — the `Dockerfile` runs `cargo build` with compile-time SQLx checks. Coolify’s build environment must supply a **`DATABASE_URL` build argument** that resolves **from the build machine** (often the same Postgres you use at runtime, reachable on the Docker build network). See **Build-time database** below.

## Prerequisites

- Coolify v4+
- A **PostgreSQL** instance (Coolify managed database, external managed Postgres, or another service on a shared Docker network).
- **Git** access to this monorepo if you use the **Dockerfile** build pack (recommended for GitOps).
- **Letta** (and **Bifrost**) when you enable bear provisioning and chat proxying — set `LETTA_BASE_URL` (and `LETTA_API_KEY` when Letta enforces auth). Cross-service hostnames follow your Coolify stack naming (for example the internal hostname shown on the Letta resource).

---

## Option A: Build from Git — Dockerfile build pack

### 1. Database (before first deploy)

1. Provision **PostgreSQL** (Coolify **Add Resource** → **Database** → PostgreSQL, or attach an existing instance).
2. Create an **empty database** (or pick an existing one) and a role with permission to create tables and run DDL — Den applies schema automatically on startup.

### 2. Create the Den resource

1. Open your Coolify **project** → **Add New Resource**.
2. Connect **this** repository (public or private, per your hosting setup).
3. Choose the **Dockerfile** build pack (not “Docker Image” alone — you want Coolify to **build** from the repo).

### 3. Point Coolify at `services/den/`

In the Dockerfile deployment settings, set:

| Field | Value |
| ----- | ----- |
| **Branch** | Your production branch (for example `main`). |
| **Base Directory** | `services/den` |
| **Dockerfile** | `Dockerfile` (path is relative to the base directory). |

Coolify should clone the repo and run `docker build` with context rooted at `services/den/`.

### 4. Build arguments (required for the current `Dockerfile`)

Open **Build Arguments** / **Docker Build Args** (wording varies by Coolify version) and add:

| Name | Purpose |
| ---- | ------- |
| `DATABASE_URL` | Used at **image build** for SQLx when **`SQLX_OFFLINE` is unset or `false`**. Must reach PostgreSQL from the build environment (disposable compile-only DB is fine). The Dockerfile defaults to a dummy URL when you use offline mode instead. |
| `SQLX_OFFLINE` | Set to **`true`** to compile against committed [`.sqlx/`](.sqlx/) query metadata (no live Postgres during `cargo build`). The image build copies `.sqlx/` from the Git checkout into the build context. Regenerate metadata with `cargo sqlx prepare` when queries change. |
| `SOURCE_DATE_EPOCH` (optional) | Unix timestamp (seconds) used as **`GET /version`** → `built_at_utc` when set at image build time; otherwise the build uses the real clock when `build.rs` runs. Useful for reproducible builds. |

If you want `/version` and `/status.json` to report deploy metadata **without** changing Docker build args on every push, prefer **runtime** environment variables instead of build args. Den checks `DEN_GIT_SHA_OVERRIDE` first, then `GIT_SHA`, then `SOURCE_COMMIT`, and falls back to the compile-time `GIT_SHA` baked by `build.rs`. It also checks `DEN_BUILT_AT_OVERRIDE` first and otherwise falls back to the compile-time `DEN_BUILT_AT_UTC` from `build.rs`.

If you omit `SQLX_OFFLINE=true`, the build needs a reachable Postgres so SQLx can verify queries against a database that has applied the current migrations (same as before). Offline builds are the usual **CI / air-gapped** approach (see [`docs/deploy.md`](docs/deploy.md)).

At **container start**, Den connects using the **runtime** `DATABASE_URL` and applies any pending migrations there automatically.

Optional: pin **`RUST_VERSION`** in the `Dockerfile` or override it via build args if your Coolify setup supports passing additional `ARG` values.

### 5. Runtime environment variables

In the resource → **Environment Variables** / **Production Variables**, set at least:

| Variable | Notes |
| -------- | ----- |
| `DATABASE_URL` | **Required.** The database Den serves at runtime; migrations run against this URL on startup (connection string as accepted by SQLx / `tokio-postgres`). |
| `DB_MAX_CONNECTIONS` | Optional SQLx pool size for `DATABASE_URL` (default **5**). |
| `DB_ACQUIRE_TIMEOUT_SECS` | Optional SQLx pool acquire timeout for `DATABASE_URL` (default **3**). |
| `DB_IDLE_TIMEOUT_SECS` | Optional SQLx idle connection timeout for `DATABASE_URL` (default **600**; set **0** to disable). |
| `SQLX_MIGRATE_IGNORE_MISSING` | Optional migration recovery switch for `DATABASE_URL`; leave **false** in normal deployments. |
| `JWT_SECRET` | **Required for release images** (Dockerfile builds with `--features production`). Use a long random value. Also required whenever `RUN_API=true` in dev builds so OAuth access tokens can be signed (HS256). |
| `RUN_WEB` | `true` to serve the web UI (recommended first smoke). |
| `RUN_API` | `true` for the standalone API listener. In the root BEARS Compose stack this defaults to `true` so the ACP gateway is available. |
| `RUN_WORKERS` | `true` when you want in-process workers enabled. |
| `PORT` | Web listen port inside the container (default **3000**). |
| `API_PORT` | API listen port when `RUN_API=true` (default **3001**). |
| `AGENT_RUNTIME` | `native` (in-process loop + per-Bear SQLite) or `letta` (legacy). Root compose defaults to **`native`**. |
| `BEAR_SQLITE_DATA_DIR` | **Required for native runtime persistence.** Absolute path inside the container where Den stores per-Bear SQLite files (default **`/var/lib/den/bear-sqlite`**). Mount a **persistent volume** at this path so Bear memory survives image upgrades and container recreation. Den does **not** run backups for this store — use volume snapshots or the optional `bears-den-sqlite-data-backup` sidecar in root [`docker-compose.yaml`](../../docker-compose.yaml) (`volume-backup` profile). |

Strongly recommended for production:

| Variable | Notes |
| -------- | ----- |
| `WEB_SERVER_URL` | Public origin of the web app (**no** trailing slash), for example `https://den.example.com`. |
| `API_SERVER_URL` | Public origin of the API when `RUN_API=true`; for BEARS ACP this can be a subdomain such as `https://api.bears.[domain]`, another hostname, or a published port such as `https://bears.[domain]:3001`. |
| `SESSION_COOKIE_DOMAIN` | Cookie `Domain` when sessions must span subdomains; omit for host-only cookies. |
| `DEN_GIT_SHA_OVERRIDE` | Optional runtime-only commit identifier for `/version` and `/status.json`. Recommended when your Coolify build omits `GIT_SHA` build args to preserve Docker cache reuse. If your Coolify setup exposes a commit variable at container runtime, map it here. |
| `DEN_BUILT_AT_OVERRIDE` | Optional runtime-only timestamp for `/version` and `/status.json` (for example an RFC 3339 deploy timestamp). Use this if you want the status page to show deploy-time metadata instead of the compile-time timestamp from the crate build script. |

Integrations (set when you wire the rest of the stack):

| Variable | Notes |
| -------- | ----- |
| `LETTA_BASE_URL` | Internal base URL for Letta (no trailing slash). **Production** images default to **`http://bears-letta:8283`** when unset (override for local dev; see `services/den/.env.example`). |
| `LETTA_API_KEY` | Bearer token when Letta is configured with `LETTA_SERVER_PASS` / API auth. |
| `CODEPOOL_BASE_URL` | When `RUN_WEB=true`, must be non-empty. **Production** images default to **`http://bears-codepool:3030`** when unset. **Codepool** harness (repository root `services/codepool/`). |
| `CODEPOOL_INTERNAL_TOKEN` | Optional shared secret; Den sends `Authorization: Bearer …` to Codepool (must match the pool service). |
| `ACP_GATEWAY_ENABLED` | Enables the API-only ACP gateway on `/acp/*`; requires `RUN_API=true` and `LETTA_BASE_URL`. ACP routes to the Bear's API-direct `pair` role, not Codepool. Root BEARS Compose defaults this to `true`. |
| `LETTA_MEMFS_SERVICE_URL` | Optional; same **MemFS Manager** base URL as Letta (no trailing slash), e.g. **`http://bears-memfs-manager:8285`**. When set, **bear details** shows **Private memory (git)** — latest commit on the agent’s context repo. **Production** images do not default this; root [`docker-compose.yaml`](../../docker-compose.yaml) sets it for `bears-den` when you use the full stack. |

Mail, OAuth, and other keys are documented in [`.env.example`](.env.example) and [`docs/deploy.md`](docs/deploy.md).

**Migrations:** Den applies embedded SQL from [`migrations/`](migrations/) on startup. By default, SQLx does **not** ignore migration files missing from the binary; do not set `SQLX_MIGRATE_IGNORE_MISSING` in production unless you are following a documented recovery procedure for a legacy `_sqlx_migrations` table.

**Sessions:** Login sessions use `tower-sessions` with the Postgres store; the session cookie carries an opaque id and data lives in Postgres. Optional signed/encrypted cookies (`with_signed` / `with_private`) are not configured in this repo—no extra session signing env var is required today.

### 6. Ports

- **Web only (`RUN_WEB=true`, `RUN_API=false`):** expose internal port **3000** and map it to HTTPS / your Coolify domain as usual.
- **Web + API:** add a **second published port** in Coolify for **3001** (or change `API_PORT` and expose the matching port). Route whatever public API origin you choose to that port, e.g. `api.bears.[domain]` -> `bears-den:3001` or a published port on the web host. The runtime image listens on whichever ports you configure via `PORT` / `API_PORT`.

The `Dockerfile` only declares `EXPOSE 3000`; publishing the API port is done in Coolify’s **Ports** / **Networking** UI when you enable the API.

### 7. Health checks (Coolify)

Prefer **HTTP** health checks so you do not need shell inside the image:

| Mode | Path | Expected |
| ---- | ---- | -------- |
| Liveness (web) | `GET /healthcheck` on the **web** port | Plain-text response containing **OK** |
| Readiness (web) | `GET /health/ready` on the **web** port | **200** when Postgres is reachable; **503** when not |
| API enabled | Same paths on the **API** port | **API OK** text on `/healthcheck` when the API server is enabled |

Use **readiness** on `/health/ready` if you want Coolify to wait for database connectivity before sending traffic.

Suggested intervals match your other BEARS services (for example 30s interval, generous start period on cold Rust startup).

### 8. Persistent storage (native runtime)

When `AGENT_RUNTIME=native`, Den keeps **bear-canonical memory** (role-local notes, proposals, promotions, curation) in one SQLite file per Bear under `BEAR_SQLITE_DATA_DIR`. This is **separate from** `DATABASE_URL` (Den Postgres control-plane).

- **Docker Compose (repo root):** `bears-den` mounts the named volume `bears-den-sqlite-data` at `/var/lib/den/bear-sqlite` and sets `BEAR_SQLITE_DATA_DIR` accordingly.
- **Coolify Docker Image resource:** add a **persistent storage** volume mounted at `/var/lib/den/bear-sqlite` and set `BEAR_SQLITE_DATA_DIR=/var/lib/den/bear-sqlite`. The image entrypoint (`docker-entrypoint.sh`) creates the directory and assigns ownership to the `appuser` runtime user before starting `/bin/server`.
- **Backups:** Den has no built-in SQLite backup job. Enable operator backups (volume snapshots, or `bears-den-sqlite-data-backup` with `COMPOSE_PROFILES=volume-backup` and `SCALEWAY_*` credentials). SQLite uses WAL mode — prefer quiesced copies or full-volume archives over copying a single `.sqlite` file while Den is writing.

### 9. Restart policy

Set restart policy to **unless stopped** (or your platform equivalent) so Den recovers after host reboots.

### 10. Deploy

Use **Deploy** / **Redeploy** on the resource. Watch **Build logs** for compile failures and **Application logs** for runtime config errors (missing `DATABASE_URL`, unreachable Letta, etc.).

### 11. Networking with Letta and Bifrost

- If Den and Letta are **different** Coolify resources, attach them to a **shared Docker network** (Coolify’s “connect to predefined network” / equivalent) so internal DNS names resolve.
- Set `LETTA_BASE_URL` to Letta’s **internal** URL (scheme + host + port, no path suffix).
- Operator-facing Bifrost integration (when present in your build) is configured via env keys documented in [`.env.example`](.env.example); align hostnames with your Bifrost service name inside Coolify.

---

## Option B: Versioned image publish from CI

The normal compose deployment builds Den from source on the deploy host. GitHub Actions only publishes a Den image when `services/den/Cargo.toml` changes and the Den crate `package.version` is different from the previous commit.

The workflow:

- Builds with **`SQLX_OFFLINE=true`** against the committed [`.sqlx/`](.sqlx/) metadata (no database needed at build time).
- Tags images as **`ghcr.io/<owner>/den:<version>`**, **`ghcr.io/<owner>/den:v<version>`**, and the commit SHA. The default branch also gets `latest`.
- Uses GitHub Actions layer cache (`type=gha`) so unchanged layers are reused across builds.

Use this path for pinned releases or external consumers. The root `docker-compose.yaml` does not depend on these images for normal deployment.

If you do deploy one of these images directly and the GHCR package is **private**, authenticate Docker on the Coolify server so it can pull the image. SSH in and run as root:
   ```
   echo "<GITHUB_PAT>" | docker login ghcr.io -u <GITHUB_USER> --password-stdin
   ```
The PAT needs the `read:packages` scope. This must be run as root (Coolify's Docker daemon uses `/root/.docker/config.json`).

### Keeping `.sqlx/` up to date

When you add or change SQLx queries, run `cargo sqlx prepare` locally against a database with current migrations applied, then commit the updated `.sqlx/` directory. The CI build will fail if the metadata is stale.

New versions still apply migrations automatically on first container start against the configured `DATABASE_URL`.

---

## Build caching (what is and isn't cached)

Coolify builds Den from source on every deploy. The [`Dockerfile`](Dockerfile) uses three BuildKit cache mounts that persist on the deploy host across deployments (until `docker builder prune`):

- `/usr/local/cargo/registry` + `/usr/local/cargo/git` — downloaded crate sources.
- `/app/target` — compiled artifacts.

**What this buys you:** external dependencies are not re-downloaded or recompiled unless `Cargo.lock` changes. This was the main pain point and it is resolved by the `/app/target` mount.

**What still recompiles:** the workspace crates (`den-core`, `den-web`, …) rebuild on every deploy. Cargo decides freshness from file **mtimes**, and Docker's `COPY` stamps a fresh mtime on every file each build, so Cargo treats all workspace sources as changed. The content-hash–based fix for this (`cargo`'s `checksum-freshness`) is still [unstable](https://github.com/rust-lang/cargo/issues/14136) and requires a nightly toolchain, so it is not used here while the build pins stable `RUST_VERSION`.

> **Avoiding the double build:** Coolify runs `docker compose build` then `docker compose up -d`, from two different directories. `bears-den` deliberately does **not** set `pull_policy: build` — that flag would force the `up` step to recompile the image a second time even though `build` already produced `bears-den:local`. Without it, `up` reuses the freshly built image. The `build` phase always runs first, so the reused image is always current. (The cache mounts are also shared across passes by `id=`, so even a forced second build reused dependencies — but the workspace would still recompile, which is what removing the flag avoids.)

---

## Verify (without the shell)

After deploy:

1. Open the **Logs** tab on the Den resource and confirm the process started without configuration errors.
2. If you assigned a public domain in Coolify, open **`https://<your-host>/healthcheck`** in a browser — you should see a short **OK**-style response for the web server.
3. Optionally open **`/health/ready`** — expect success only when the database is reachable.
4. For the full operator experience, load the **web root** `/` and complete any first-run or sign-in flows your deployment enables.

---

## Troubleshooting

| Symptom | What to check in Coolify |
| ------- | ------------------------ |
| **Build fails** during `cargo build` / SQLx | **`DATABASE_URL` build arg** reachable from the build server for compile-time checks; repo includes committed [`.sqlx/`](.sqlx/) if you use offline builds. |
| **Build killed / exit 255 with no compiler error** | Likely OOM during the Rust link step. Lower `CARGO_BUILD_JOBS`, add swap/RAM to the deploy host, or temporarily deploy a pinned versioned image from CI. |
| **Whole workspace recompiles on every deploy** | Expected on stable Rust — see [Build caching](#build-caching-what-is-and-isnt-cached). External dependencies stay cached via the `/app/target` mount; only workspace crates rebuild, because Cargo freshness is mtime-based and `COPY` rewrites mtimes. |
| **Container exits immediately** | **Logs** — missing or invalid `DATABASE_URL`, or a **migration error** (DDL permissions, broken migration, incompatible existing schema). |
| **Running but `/health/ready` is 503** | Database credentials or network from the Den container to Postgres; if the process exits instead, check logs for migration failures. |
| **Letta provisioning fails** | `LETTA_BASE_URL` scheme/host/port; shared network with Letta; `LETTA_API_KEY` matches Letta’s server password / auth configuration. |
| **Sessions, redirects, or ACP adapter URL wrong** | `WEB_SERVER_URL` / `API_SERVER_URL` and (if used) `SESSION_COOKIE_DOMAIN` must match the URLs users and adapters actually use. For ACP, `API_SERVER_URL` should be the public API origin, whether that is `https://api.bears.[domain]`, another hostname, or a host+port URL. |

---

## Reference

- Example env keys: [`.env.example`](.env.example)
- Deploy and SQLx notes: [`docs/deploy.md`](docs/deploy.md)
- Ports, health endpoints, toggles: [`docs/infrastructure-and-ops.md`](docs/infrastructure-and-ops.md)
- Stack placement: [`docs/deployment/DEPLOYMENT.md`](../docs/deployment/DEPLOYMENT.md)
- Den + Letta architecture: [`docs/architecture/DEN_ARCHITECTURE.md`](../docs/architecture/DEN_ARCHITECTURE.md)
