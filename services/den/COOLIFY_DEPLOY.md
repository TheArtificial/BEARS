# Den — Coolify deployment guide

**Stack context:** Den is the BEARS **control plane and native runtime** (Rust / Axum): provisioning, users↔bears membership, web/API surfaces, BearWire/ACP sessions, per-Bear SQLite memory, and Den-native agent turns through Bifrost. It sits alongside **Bifrost**, **Postgres**, optional **Garage** artifacts, and optional **Outline** (Cabinet). For architecture, see [`docs/architecture/den-native-runtime.md`](../../docs/architecture/den-native-runtime.md).

## Overview

- **One image, one binary** — built from [`Dockerfile`](Dockerfile) in this directory. Runtime behavior is controlled with **environment variables** (`RUN_WEB`, `RUN_API`, `RUN_WORKERS`, ports, `DATABASE_URL`, …). Deeper reference: [`docs/deploy.md`](docs/deploy.md) and [`docs/infrastructure-and-ops.md`](docs/infrastructure-and-ops.md).
- **PostgreSQL is mandatory** — Den exits on startup if it cannot use `DATABASE_URL`. The **database must exist** (empty is fine). In the compose deployment, a one-off `bears-den-migrate` job runs embedded SQLx migrations from [`migrations/`](migrations/) before the long-running `bears-den` service is allowed to start. By default migrations are **strict** (see `SQLX_MIGRATE_IGNORE_MISSING` in [`.env.example`](.env.example) / [`docs/deploy.md`](docs/deploy.md)—leave it unset in production).
- **SQLx at image build time** — the `Dockerfile` runs `cargo build` with compile-time SQLx checks. Coolify’s build environment must supply a **`DATABASE_URL` build argument** that resolves **from the build machine** (often the same Postgres you use at runtime, reachable on the Docker build network). See **Build-time database** below.

## Prerequisites

- Coolify v4+
- A **PostgreSQL** instance (Coolify managed database, external managed Postgres, or another service on a shared Docker network).
- For the recommended GitHub-built image path, GHCR pull access configured on the Coolify host when packages are private. Git repository access remains necessary for the root Compose resource because it supplies stack configuration.
- **Bifrost** reachable from Den for model calls (`LLM_API_URL`, defaulting to the compose service URL in the root stack).

---

## Option A: Build from Git — Dockerfile build pack (legacy/direct path)

### 1. Database (before first deploy)

1. Provision **PostgreSQL** (Coolify **Add Resource** → **Database** → PostgreSQL, or attach an existing instance).
2. Create an **empty database** (or pick an existing one) and a role with permission to create tables and run DDL — the deploy-time migration job applies schema before `bears-den` is switched over.

### 2. Create the Den resource

1. Open your Coolify **project** → **Add New Resource**.
2. Connect **this** repository (public or private, per your hosting setup).
3. Choose the **Dockerfile** build pack only when intentionally using this direct source-build path. The recommended root-stack path is [Option B](#option-b-github-built-image-deployment-recommended).

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

The build phase completes before either `bears-den-migrate` or `bears-den` starts, because both services use the same built Den image tag. The migration job then runs `den migrate`; the app service starts with `den serve` after that job exits successfully.

Optional: pin **`RUST_VERSION`** in the `Dockerfile` or override it via build args if your Coolify setup supports passing additional `ARG` values.

### 5. Runtime environment variables

In the resource → **Environment Variables** / **Production Variables**, set at least:

| Variable | Notes |
| -------- | ----- |
| `DATABASE_URL` | **Required.** The database Den serves at runtime; the deploy-time migration job and startup schema guard both use this URL (connection string as accepted by SQLx / `tokio-postgres`). |
| `DB_MAX_CONNECTIONS` | Optional SQLx pool size for `DATABASE_URL` (default **5**). |
| `DB_ACQUIRE_TIMEOUT_SECS` | Optional SQLx pool acquire timeout for `DATABASE_URL` (default **3**). |
| `DB_IDLE_TIMEOUT_SECS` | Optional SQLx idle connection timeout for `DATABASE_URL` (default **600**; set **0** to disable). |
| `SQLX_MIGRATE_IGNORE_MISSING` | Optional migration recovery switch for `DATABASE_URL`; leave **false** in normal deployments. |
| `JWT_SECRET` | **Required for release images** (Dockerfile builds with `--features production`). Use a long random value. Also required whenever `RUN_API=true` in dev builds so OAuth access tokens can be signed (HS256). |
| `RUN_WEB` | `true` to serve the web UI (recommended first smoke). |
| `RUN_API` | `true` for the standalone API listener. In the root BEARS Compose stack this defaults to `true` so BearWire is available. |
| `RUN_WORKERS` | `true` when you want in-process workers enabled. |
| `PORT` | Web listen port inside the container (default **3000**). |
| `API_PORT` | API listen port when `RUN_API=true` (default **3001**). |
| `BEAR_SQLITE_DATA_DIR` | **Required for native runtime persistence.** Absolute path inside the container where Den stores per-Bear SQLite files (default **`/var/lib/den/bear-sqlite`**). Mount a **persistent volume** at this path so Bear memory survives image upgrades and container recreation. Den does **not** run backups for this store — use volume snapshots or the optional `bears-den-sqlite-data-backup` sidecar in root [`docker-compose.yaml`](../../docker-compose.yaml) (`volume-backup` profile). |

Strongly recommended for production:

| Variable | Notes |
| -------- | ----- |
| `DEN_WEB_ORIGIN` | Public origin of the web app (**no** trailing slash), for example `https://den.example.com`; compose derives `WEB_SERVER_URL` from this. |
| `DEN_API_ORIGIN` | Public origin of the API when `RUN_API=true`; for armatures this can be a subdomain such as `https://api.bears.[domain]`, another hostname, or a published port such as `https://bears.[domain]:3001`; compose derives `API_SERVER_URL` from this. |
| `SESSION_COOKIE_DOMAIN` | Cookie `Domain` when sessions must span subdomains; omit for host-only cookies. |
| `DEN_GIT_SHA_OVERRIDE` | Optional runtime-only commit identifier for `/version` and `/status.json`. Recommended when your Coolify build omits `GIT_SHA` build args to preserve Docker cache reuse. If your Coolify setup exposes a commit variable at container runtime, map it here. |
| `DEN_BUILT_AT_OVERRIDE` | Optional runtime-only timestamp for `/version` and `/status.json` (for example an RFC 3339 deploy timestamp). Use this if you want the status page to show deploy-time metadata instead of the compile-time timestamp from the crate build script. |

Integrations (set when you wire the rest of the stack):

| Variable | Notes |
| -------- | ----- |
| `BIFROST_APP_PORT` | Bifrost listen port. Use distinct values per environment on shared networks, for example prod `8080`, test `8081`. |
| `BIFROST_ORIGIN` | Canonical internal Bifrost origin. Root compose derives `BIFROST_BASE_URL`, `BIFROST_MANAGEMENT_URL`, and `LLM_API_URL` from this. Defaults to `http://bears-bifrost:${BIFROST_APP_PORT}`. |
| `RUN_API` | When enabled, mounts the API and BearWire under `/bearwire`. |

Mail, OAuth, and other keys are documented in [`.env.example`](.env.example) and [`docs/deploy.md`](docs/deploy.md).

**Migrations:** Compose runs embedded SQL from [`migrations/`](migrations/) in the one-off `bears-den-migrate` job before starting `bears-den`. The long-running service uses `den serve`, which refuses to boot if the database schema version recorded in `_sqlx_migrations` is newer than the binary's embedded migrator. By default, SQLx does **not** ignore migration files missing from the binary; do not set `SQLX_MIGRATE_IGNORE_MISSING` in production unless you are following a documented recovery procedure for a legacy `_sqlx_migrations` table.

**Policy:** Keep migrations reversible and rollout-safe. See [`migrations/README.md`](migrations/README.md) for the expand-contract and `*_down.sql` policy that goes with this deploy flow.

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

Use **Deploy** / **Redeploy** on the resource. Watch **Build logs** for compile failures, then the `bears-den-migrate` logs for migration failures, then `bears-den` application logs for runtime config errors (missing `DATABASE_URL`, unreachable Bifrost, etc.).

### 11. Networking with Bifrost

- If Den and Bifrost are **different** Coolify resources, attach them to a **shared Docker network** (Coolify’s “connect to predefined network” / equivalent) so internal DNS names resolve.
- Set `BIFROST_ORIGIN` to Bifrost's internal origin, for example `http://bears-bifrost:8080`; compose derives `LLM_API_URL` as `${BIFROST_ORIGIN}/v1`.
- Operator-facing Bifrost governance/metadata integration is configured via env keys documented in [`.env.example`](.env.example); align hostnames with your Bifrost service name inside Coolify.

---

## Option B: GitHub-built image deployment (recommended)

GitHub Actions is the build authority for the root Compose stack's Den-derived services. Coolify pulls the published images; it does not compile Den during deployment.

On every relevant source change, `.github/workflows/den-image.yml` builds with **`SQLX_OFFLINE=true`** against the committed [`.sqlx/`](.sqlx/) metadata and publishes both:

- `ghcr.io/<owner>/den` for `bears-den` and `bears-den-migrate`;
- `ghcr.io/<owner>/den-sandbox-provider` for `bears-sandbox-provider`.

Each image receives an immutable `sha-<commit>` tag. Pushes to `test` also update `:testing`; pushes to `main` update `:latest`. After both images are available, `.github/workflows/coolify-deploy.yml` calls the environment-specific Coolify webhook.

Set the matching image variables on each Coolify resource and disable Coolify Git-push auto-deploys. The Compose file sets `pull_policy: always` for these services so a webhook deployment pulls the new lane digest.

If the GHCR packages are **private**, authenticate Docker on the Coolify server so it can pull the image. SSH in and run as root:
   ```
   echo "<GITHUB_PAT>" | docker login ghcr.io -u <GITHUB_USER> --password-stdin
   ```
The PAT needs the `read:packages` scope. This must be run as root (Coolify's Docker daemon uses `/root/.docker/config.json`).

### Keeping `.sqlx/` up to date

When you add or change SQLx queries, run `cargo sqlx prepare` locally against a database with current migrations applied, then commit the updated `.sqlx/` directory. The CI build will fail if the metadata is stale.

When these images are used in the compose stack, `bears-den-migrate` still applies migrations first and `bears-den` then starts in `serve` mode.

---

## Build caching (GitHub Actions)

GitHub Actions builds Den images and owns the relevant cache. The [`Dockerfile`](Dockerfile) uses three BuildKit cache mounts:

- `/usr/local/cargo/registry` + `/usr/local/cargo/git` — downloaded crate sources.
- `/app/target` — compiled artifacts.

**External dependencies** are not re-downloaded or recompiled unless `Cargo.lock` changes — resolved by the `/app/target` mount exported through the Actions and registry BuildKit caches.

**Workspace crates** (`den-core`, `den-web`, …) are kept incremental with Cargo's `-Z checksum-freshness`, which decides freshness from file **content hashes** instead of mtimes. Without it, Docker's `COPY` stamps a fresh mtime on every file each build and Cargo recompiles the entire workspace every deploy. The feature is still [unstable](https://github.com/rust-lang/cargo/issues/14136), so the build stage installs a **pinned nightly toolchain** (`RUST_NIGHTLY` in the [`Dockerfile`](Dockerfile)) purely to enable it; the runtime image is unaffected.

> **Reverting to stable:** when `checksum-freshness` stabilizes, set `RUST_NIGHTLY=` (empty) and bump `RUST_VERSION` to the stable release that ships it. The build keeps working on stable with no `-Z` flag — it just loses the optimization until the stabilized config form is wired in.
>
> **Caveat:** files read by build scripts (e.g. `minijinja-embed` template embedding) still use mtimes even under `checksum-freshness`, so template-embedding edge crates (`den-web`, `den-http`, `den-api`) may still recompile when any file changes; the leaf crates get the full benefit.

> **Deploy behavior:** Coolify does not build Den. The base Compose file uses `pull_policy: always` for the CI-published Den services, so every webhook deploy resolves the current `testing` or `latest` digest. Local source builds use [`docker-compose.dev.yaml`](../../docker-compose.dev.yaml), which restores the build sections and disables registry pulls.

---

## Verify (without the shell)

After deploy:

1. Open the **Logs** tab on `bears-den-migrate` and confirm the migration job exited successfully.
2. Open the **Logs** tab on `bears-den` and confirm the process started without configuration errors.
3. If you assigned a public domain in Coolify, open **`https://<your-host>/healthcheck`** in a browser — you should see a short **OK**-style response for the web server.
4. Optionally open **`/health/ready`** — expect success only when the database is reachable.
5. For the full operator experience, load the **web root** `/` and complete any first-run or sign-in flows your deployment enables.

---

## Troubleshooting

| Symptom | What to check in Coolify |
| ------- | ------------------------ |
| **Build fails** during `cargo build` / SQLx | **`DATABASE_URL` build arg** reachable from the build server for compile-time checks; repo includes committed [`.sqlx/`](.sqlx/) if you use offline builds. |
| **Build killed / exit 255 with no compiler error** | Likely OOM during the Rust link step. Lower `CARGO_BUILD_JOBS`, add swap/RAM to the deploy host, or temporarily deploy a pinned versioned image from CI. |
| **Whole workspace recompiles on every deploy** | Check the `build` log for `FRESHNESS=-Z checksum-freshness` and that the pinned nightly installed. If `RUST_NIGHTLY` is empty (reverted to stable) this is expected. See [Build caching](#build-caching-what-is-and-isnt-cached). |
| **Migration job fails** | Check `bears-den-migrate` logs for DDL permissions, broken migration SQL, or a startup schema version mismatch. The old `bears-den` container should remain the last successful runtime until the new app service is started. |
| **Container exits immediately** | **Logs** — missing or invalid `DATABASE_URL`, or a startup schema version mismatch indicating the database is newer than this binary. |
| **Running but `/health/ready` is 503** | Database credentials or network from the Den container to Postgres; if the process exits instead, check logs for migration failures. |
| **Sessions, redirects, or ACP adapter URL wrong** | `WEB_SERVER_URL` / `API_SERVER_URL` and (if used) `SESSION_COOKIE_DOMAIN` must match the URLs users and adapters actually use. For ACP, `API_SERVER_URL` should be the public API origin, whether that is `https://api.bears.[domain]`, another hostname, or a host+port URL. |

---

## Reference

- Example env keys: [`.env.example`](.env.example)
- Deploy and SQLx notes: [`docs/deploy.md`](docs/deploy.md)
- Ports, health endpoints, toggles: [`docs/infrastructure-and-ops.md`](docs/infrastructure-and-ops.md)
- Stack placement: [`docs/deployment/DEPLOYMENT.md`](../docs/deployment/DEPLOYMENT.md)
- Den-native architecture: [`docs/architecture/den-native-runtime.md`](../../docs/architecture/den-native-runtime.md)
