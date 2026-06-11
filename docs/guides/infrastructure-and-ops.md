# Infrastructure and operations

## Processes

**This project** builds as **one Rust binary** (crate name defaults to **`newapp`**). At runtime you enable:

- **Web** — `RUN_WEB=true` (port from `PORT`, default `3000`)
- **API** — `RUN_API=true` (port from `API_PORT`, default `3001`); in the Bear Den stack this hosts the API-only ACP gateway and must have a public origin reachable by adapters (for example `api.bears.[domain]`, another hostname, or a published port on the web host)
- **Workers** — `RUN_WORKERS=true` (background tasks in the same process)

Legacy `SERVER_MODE=web|api|both` may still be parsed for migration; prefer the `RUN_*` flags (see `src/config.rs`).

You can run any combination (e.g. web + workers only). If nothing is enabled, the process will warn and do little useful work.

## Configuration

- **`DATABASE_URL`** — PostgreSQL (required for normal operation).
- **Service toggles** — `RUN_WEB`, `RUN_API`, `RUN_WORKERS`, `ACP_GATEWAY_ENABLED`.
- **Templates / assets** — paths and production embedding follow `Config` and feature flags (`production`).

Other variables (mail, OAuth, optional integrations) are defined on `Config` as needed for your deployment.

## Database

- **PostgreSQL** with migrations under `migrations/`.
- **SQLx** with compile-time checked queries; CI/production builds typically use offline data (`.sqlx/`). See [sqlx-patterns.md](sqlx-patterns.md).

Session storage uses the same database (tower-sessions SQLx store); session migrations run at startup.

## Deployment

- **Docker** — root `Dockerfile` produces one image; set env in the orchestrator (same as local). See [den-deploy.md](den-deploy.md).

## Logging

Structured logging via **tracing** with default filters wired in [`src/lib.rs`](../src/lib.rs) (`run()`, crate prefix `newapp`). Override with **`RUST_LOG`** when debugging.

## Health checks

| Service | Liveness | Readiness (PostgreSQL `SELECT 1`) |
|---------|-----------|-----------------------------------|
| Web (`RUN_WEB`) | `GET /healthcheck` → `OK` | `GET /health/ready` → `OK` or **503** |
| API (`RUN_API`) | `GET /healthcheck` → `API OK` | `GET /health/ready` → `OK` or **503** |

When `ACP_GATEWAY_ENABLED=true`, the API also serves `POST /acp/bears/{slug}/sessions/{session_id}/prompt` on the API port. Assign the API port a public origin reachable by adapters (for example `api.bears.[domain]`, another hostname, or a published host+port URL) and set `API_SERVER_URL` to that origin.

**Build identity:** `GET /version` (web and API) returns JSON with `built_at_utc` (RFC 3339 UTC) from when the build script last ran, plus `git_sha` when the image was built with `GIT_SHA`. Set `SOURCE_DATE_EPOCH` during the image build if you need a deterministic timestamp (reproducible builds).

### Bear Den stack status (web)

For a **single watch point** across the native stack (Den Postgres, Bifrost health/metadata, low-cost env validation aligned with `services/preflight`, and optional **GHCR** comparison), use:

- `GET /status` — human-readable HTML (stack checks + deployed vs registry hints when configured).
- `GET /status.json` — JSON for scripts and monitors (**503** when any health check is in the `fail` state; `warn` and `skipped` do not fail the HTTP status).

With **`AGENT_RUNTIME=native`** (default), Letta/Codepool/MemFS probes are reported as **skipped** — they are not part of the default compose stack.

Default runtime probes include:

| Check | What it validates |
| ----- | ----------------- |
| Den PostgreSQL | `SELECT 1` against `DATABASE_URL` |
| Bifrost | `GET /health` and metadata URL from `BIFROST_BASE_URL` / `BIFROST_METADATA_URL` |
| Config shape | `JWT_SECRET` when required, `DATABASE_URL` host/scheme, `WEB_SERVER_URL`, `LLM_API_URL` shape, `OPENAI_API_KEY` presence (warn if empty) |

Optional **`GITHUB_PACKAGES_TOKEN`** (PAT with `read:packages`), **`GHCR_PACKAGES_OWNER`** (GitHub org or user that owns the images), and **`GHCR_PACKAGES_OWNER_KIND`** (`org` or `user`) populate GHCR tag / updated-at columns for Den image drift checks.

When derived recall is configured (`QDRANT_URL` and embedding settings per [ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)), operators may add external monitors for Qdrant reachability; `/status` does not yet include a first-class Qdrant probe in the default native path.

This page is **not** a substitute for **`GET /healthcheck`** (process liveness) or **`GET /health/ready`** (Den-only DB readiness).

## Workers

When `RUN_WORKERS=true`, long-running and periodic tasks run in-process. See [`src/lib.rs`](../src/lib.rs) for the worker slot; this slim starter keeps workers idle until shutdown.

## Graceful shutdown

On **Unix**, the process handles **SIGTERM** and **Ctrl+C**. On other platforms, **Ctrl+C** only.
