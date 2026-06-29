# Bear Den Stack — Coolify Deployment Guide

Deploy Bear Den on Coolify from the repository root [`docker-compose.yaml`](../../../docker-compose.yaml). This is the supported path for operators: one Compose resource, one shared network, and service names that resolve internally as `bears-*`.

The default stack runs the **Den-native agent loop** (`AGENT_RUNTIME=native`): inference via Bifrost, Bear memory in per-Bear SQLite on a mounted volume. See [den-native-runtime.md](../../architecture/den-native-runtime.md) for the architecture and [den-deploy.md](../den-deploy.md) for single-image env details.

## What You Deploy

Use the root [`docker-compose.yaml`](../../../docker-compose.yaml). It starts:

| Service | Purpose |
| ------- | ------- |
| `bears-preflight-config` | One-shot deploy preflight: required env shape and secrets |
| `bears-preflight-den-db` | One-shot Den Postgres connectivity check (when `bundled` profile provides Postgres) |
| `bears-bifrost` | Model gateway on port `8080` |
| `bears-den` | Den web UI on port `3000` and Den API/BearWire on port `3001` |

Optional compose profiles:

| Profile | Service | Purpose |
| ------- | ------- | ------- |
| `bundled` | `bears-postgres` | Local/dev Den Postgres when you do not use a managed database |
| `volume-backup` | `bears-den-sqlite-data-backup` | S3-compatible archive of per-Bear SQLite data |

**Database:** set **`DATABASE_URL`** for `bears-den` (managed Postgres is preferred in Coolify). For local compose only, `COMPOSE_PROFILES=bundled` starts `bears-postgres` with dev defaults.

**Bear memory:** `bears-den` mounts `bears-den-sqlite-data` at `BEAR_SQLITE_DATA_DIR` (default `/var/lib/den/bear-sqlite`). Image upgrades keep Bear memory when this volume is attached.

## Requirements

- Coolify v4+
- One Postgres database for Den (`DATABASE_URL`)
- An OpenAI API key (for Bifrost)
- Public access for Den web and Den API. The API can be a subdomain such as `api.bears.[domain]`, a separate hostname, or a published port on the web host.

## 1. Create The Database

Create Den Postgres in Coolify first. Copy the database's **Postgres URL (internal)** — you will paste it into `DATABASE_URL` on the Compose resource.

For local/devcontainer runs, the `bundled` compose profile provides `bears-postgres` for Den. Production should prefer managed databases.

## 2. Create The Compose Resource

In Coolify:

1. Go to **Add Resource**.
2. Choose **Docker Compose**.
3. Select this repository.
4. Set **Build Pack** to `Docker Compose`.
5. Set **Base Directory** to `.`.
6. Set **Compose File** to `docker-compose.yaml`.

## 3. Configure Domains

In the Compose resource general configuration:

1. Set the primary web domain for `bears-den` with port suffix `:3000`.
2. Set public access for the Den API with port suffix `:3001`. A subdomain like `api.bears.[domain]` is the recommended convention, but not a requirement; you can also use another hostname or a published port on the web host.
3. Under **Build**, enable **Preserve Repository During Deployment**.
4. Save.

Den web is the browser-facing UI. Den API is the bearer-token machine-client surface and hosts BearWire for local armatures such as `bear-armature`.

## 4. Connect The Network

In the Compose resource advanced settings:

1. Open **Docker Compose**.
2. Enable **Connect To Predefined Network**.
3. Save.

This keeps the `bears-*` service names stable for internal URLs such as `http://bears-bifrost:8080` and `http://bears-den:3001`.

## 5. Set Environment Variables

Set these on the Compose resource:

| Variable | Value |
| -------- | ----- |
| `AGENT_RUNTIME` | `native` (default in compose) |
| `JWT_SECRET` | Random secret string |
| `OPENAI_API_KEY` | Your OpenAI API key |
| `DATABASE_URL` | Den Postgres **Postgres URL (internal)** from Coolify |
| `DEN_WEB_ORIGIN` | Public Den web origin, e.g. `https://bears.[domain]`; compose derives `WEB_SERVER_URL` from this |
| `DEN_API_ORIGIN` | Public Den API origin, e.g. `https://api.bears.[domain]` or `https://bears.[domain]:3001`; compose derives `API_SERVER_URL` from this |
| `BEAR_SQLITE_DATA_DIR` | Leave default `/var/lib/den/bear-sqlite` unless you customize the volume mount |

Optional:

| Variable | Value |
| -------- | ----- |
| `DEN_IMAGE` | Optional local tag assigned to the compose-built Den image |
| `CARGO_BUILD_JOBS` | Den Docker build parallelism; keep low on small deploy hosts |
| `BIFROST_APP_PORT` | Bifrost listen port. Use distinct values per environment on shared networks, for example prod `8080`, test `8081` |
| `BIFROST_ORIGIN` | Canonical internal Bifrost origin; compose derives `BIFROST_BASE_URL`, `BIFROST_MANAGEMENT_URL`, and `LLM_API_URL` from this. Defaults to `http://bears-bifrost:${BIFROST_APP_PORT}` |
| `RUN_WEB` / `RUN_API` / `RUN_WORKERS` | Service toggles inside the Den container (compose defaults web+API on) |

You usually do not need to set internal service URLs. The compose file already defaults to:

| Variable | Default |
| -------- | ------- |
| `BIFROST_ORIGIN` | `http://bears-bifrost:${BIFROST_APP_PORT}` |
| `LLM_API_URL` | `${BIFROST_ORIGIN}/v1` |
| `BIFROST_BASE_URL` | `${BIFROST_ORIGIN}` |
| `BIFROST_MANAGEMENT_URL` | `${BIFROST_ORIGIN}/api` |
| `BEAR_SQLITE_DATA_DIR` | `/var/lib/den/bear-sqlite` |

## 6. Deploy

For the initial deploy, click **Deploy** in Coolify.

For ongoing `main` updates, prefer the GitHub-coordinated deployment flow:

1. In Coolify, disable automatic deploys on Git push for this Compose resource if they are enabled.
2. Keep the repository secrets `COOLIFY_WEBHOOK` and `COOLIFY_TOKEN` configured in GitHub.
3. Let `.github/workflows/coolify-deploy.yml` trigger the Coolify deploy webhook.

That workflow waits for the Den image workflow when a push changes `services/den/**` or related stack files, then triggers Coolify only after the required GHCR image is available. This avoids Coolify pulling stale `latest` image tags before GitHub has finished rebuilding them.

If deploy preflight fails, check the missing environment variable in the logs first. The compose file intentionally defaults required secrets and database URLs to `SETME` so bad deploys fail early. Preflight services (`bears-preflight-config`, `bears-preflight-den-db`) must complete successfully before `bears-bifrost` and `bears-den` start.

## Verification

From Coolify's terminal for a service on the same network:

| Check | Command |
| ----- | ------- |
| Bifrost | `curl http://bears-bifrost:8080/health` |
| Den web | Open the public Den URL |
| Den API | `curl ${API_SERVER_URL}/healthcheck` |
| Den readiness | `curl ${API_SERVER_URL}/health/ready` |
| Stack status | Open `${WEB_SERVER_URL}/status` or `curl ${WEB_SERVER_URL}/status.json` |
| BearWire auth check | `curl -i ${API_SERVER_URL}/bearwire/v1/sessions/smoke-session/events?bear_slug=test-bear` should return `401` without a bearer token |

End-to-end check: create or open a bear in Den, go to its chat page, and send a message.

## Troubleshooting

- If Den cannot start, confirm `DATABASE_URL`, `JWT_SECRET`, `WEB_SERVER_URL`, `API_SERVER_URL`, and `LLM_API_URL`.
- If chat does not stream, confirm `bears-den` can reach `${BIFROST_ORIGIN}/v1` and `OPENAI_API_KEY` is set for Bifrost.
- If Bifrost is unhealthy, confirm `OPENAI_API_KEY` and `services/bifrost/config.json`.
- If Bear memory does not persist across redeploys, confirm the `bears-den-sqlite-data` volume is attached and `BEAR_SQLITE_DATA_DIR` matches the mount path.

## Optional Backups

The root compose file includes an optional volume-backup sidecar behind the `volume-backup` profile:

| Service | Volume | When needed |
| ------- | ------ | ----------- |
| `bears-den-sqlite-data-backup` | `bears-den-sqlite-data` | Native runtime — per-Bear SQLite memory |

Enable only after the base stack is healthy:

```bash
COMPOSE_PROFILES=volume-backup docker compose --profile volume-backup up -d
```

Provide the `SCALEWAY_*` backup variables (and optional `DEN_SQLITE_VOLUME_BACKUP_CRON`) in `.env`. Den does not run SQLite backups in-process; the sidecar archives the mounted volume for operator restore.

## Appendix: Legacy Letta stack

The pre-native Letta/Codepool/MemFS/Redis compose profile is **not** part of the default root `docker-compose.yaml`. Operators still on that path should consult archived notes under [`docs/archive/letta/`](../../archive/letta/) and set `AGENT_RUNTIME=letta` only in a custom compose overlay — not as the primary deployment documented here.

---

Last updated: 2026-06-11
