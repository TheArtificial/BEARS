# Quick start (local development)

Local development runs Den in-process with Bifrost for inference and per-Bear SQLite for memory. See [den-runtime.md](../architecture/den-runtime.md) for the runtime model.

## Run the app

1. Copy [`.env.example`](../.env.example) to `.env` (or set env another way) and set **`DATABASE_URL`** to a PostgreSQL database that exists on your machine or network (empty database is fine).
2. Set **`AGENT_RUNTIME=native`** (default) and **`LLM_API_URL`** to your Bifrost OpenAI-compatible endpoint (for example `http://localhost:8080/v1` or `http://bears-bifrost:8080/v1` inside the dev stack).
3. Set **`BEAR_SQLITE_DATA_DIR`** to a writable directory for per-Bear SQLite files (for example `./data/bear-sqlite` locally, or `/var/lib/den/bear-sqlite` in Docker).
4. Enable at least one service, for example **`RUN_WEB=true`** (and optionally `RUN_API`, `RUN_WORKERS`).
5. Run from `services/den/`:

   ```bash
   cargo run
   ```

   The app applies SQLx migrations from [`migrations/`](../migrations/) automatically. A migration seeds a **bootstrap operator** on empty databases: username **`admin`**, password **`Never deploy with default passwords.`** (see [`migrations/README.md`](../migrations/README.md) § *Default operator account*). Replace that password before any real deployment.

   When you add new migration files, use `sqlx migrate add` / `sqlx migrate run` from the `services/den/` directory as described in [sqlx-patterns.md](sqlx-patterns.md).

   **Static assets (`src/web/assets/`):** In a **debug** `cargo run`, `memory-serve` registers routes when the binary is **compiled** and reads file bytes from **disk** at request time using those recorded paths. If you add or change files under `src/web/assets/` (for example Deep Chat under `assets/deep-chat/`), run a **fresh build** and **restart** the `den` process; a long-lived or stale process can otherwise return **404** for `/assets/...` even though the files exist in the tree. Release builds embed assets in the binary instead.

You can use the devcontainer in this repo instead of a manual local Postgres setup if that matches your workflow. The devcontainer attaches to the Docker stack network and exports defaults for `DATABASE_URL` and `LLM_API_URL` so Den can reach `bears-postgres` and `bears-bifrost`.

## Docker stack (recommended for integration)

From the repo root, start the native compose stack (Bifrost + Den; optional `bundled` Postgres):

```bash
docker compose --profile bundled up -d
```

Operational scripts (see also [AGENTS.md](../../AGENTS.md)):

```bash
./scripts/smoke.sh              # HTTP smoke tests against the running stack
./scripts/restart.sh bears-den  # Recreate Den after code/image changes
./scripts/logs.sh bears-den     # Tail Den logs
```

For a full build-from-source smoke pass:

```bash
./scripts/smoke-stack.sh
```

## Development and smoke seeds

Schema migrations stay environment-agnostic. Disposable development and smoke-test data is seeded separately with:

```bash
cargo run -- seed --profile smoke
```

The initial `smoke` profile is idempotent and creates/reuses:

| Fixture | Value |
|---------|-------|
| Username | `alice` |
| Password | `Never deploy seed passwords.` |
| Bear slug | `test-bear` |

The profile also verifies Alice's email and grants her membership on `test-bear`, so `/bear/test-bear` can be used by smoke tests and manual UI checks. `minimal` currently aliases `smoke`.

In the repo devcontainer, `/workspace/scripts/devcontainer-start.sh` builds local Bifrost and Den images, starts bundled Postgres when configured, runs `/workspace/scripts/seed-dev.sh smoke`, then attempts to start the rest of the stack with those local images. Startup builds, seeding, and full-stack startup are non-fatal: the container remains usable if any step fails. Check `.devcontainer/logs/startup.status` and `.devcontainer/logs/startup.log` for details, then rerun manually with:

```bash
./scripts/seed-dev.sh smoke
```

## Development-only link prefix

Without `--features production`, [`src/config.rs`](../src/config.rs) sets `URL_PREFIX` to `https://redirectmeto.com/http://localhost:3000/`. Generated absolute links (email verification, telemetry) therefore go through the third-party redirect service [redirectmeto.com](https://redirectmeto.com) before hitting your local app. Edit `URL_PREFIX` in that file if you prefer plain `http://localhost:…`, a tunnel URL, or another approach.

## Templates

In **development**, MiniJinja loads files from **`TEMPLATES_DIR`** (default `src/web/templates`). In **`--features production`** / release Docker builds, templates are **embedded** at compile time—plan on **rebuilding** the binary when HTML changes in production.

## API and JWT secret

If you enable **`RUN_API=true`**, set **`JWT_SECRET`** to a long random value (OAuth access tokens are HS256-signed). Release and Docker images are built with **`--features production`**, which also requires **`JWT_SECRET`** at runtime.

## Fresh database and SQLx offline

The schema is applied automatically on startup from `migrations/`. For **`SQLX_OFFLINE`** / CI builds, run `cargo sqlx prepare` against a database that has run those migrations at least once and commit [`.sqlx/`](../.sqlx/). See [sqlx-patterns.md](sqlx-patterns.md).

**Strict migrations:** By default, SQLx does not ignore migration files missing from the repo. If integration tests or a disposable database fail with a migration history mismatch, fix the database or set **`SQLX_MIGRATE_IGNORE_MISSING=true`** only as a documented recovery step—not for routine production deploys.

## Mail

`MAILGUN_API_KEY` and `MAILGUN_DOMAIN` default to empty; set them (or swap the mail implementation) before relying on outbound email.

## Shutdown

**Ctrl+C** is honored on all platforms; **SIGTERM** triggers graceful shutdown on **Unix** only.
