# Trestle

This is a starter repository for Rust web applications. It is meant to be useful as boilerplate for coding agents and seeks to provide strong patterns to guide them toward a maintainable, efficient result.

As such, it is opinionated:

- URLs for both APIs and page requests are routed to straightforward [Axum](https://github.com/tokio-rs/axum) handlers.
- HTML responses are generated with [MiniJinja](https://docs.rs/minijinja/) templates.
- Data is stored in Postgresql (bring your own), managed with [SQLx](https://github.com/launchbadge/sqlx) (including migrations).
- An OAuth provider and very minimal user management is included as example code. (With emails sent via [Mailgun](https://www.mailgun.com/))
- Simple in-process worker management is stubbed in.
- Deployment is simple with Docker,

You don't need to be familiar with any of this to get started, but would benefit from having enough technical understanding to make sense of what they are.

Note that the name "newapp" is used in a few places. You or our agent should see [`docs/rename-from-starter.md`](docs/rename-from-starter.md) for details, and can use **`./scripts/verify-rename.sh --strict`** to check.

---

## Key Documentation

| File | |
|------|-|
| `README.md` (this file) | Scope, conventions, how services are toggled |
| `AGENTS.md` | Where agents start; links into `docs/` |
| [`docs/development-principles.md`](docs/development-principles.md) | Development principles (dependencies, frontend minimalism, etc.); fill in as your team agrees |
| `docs/` | Axum, SQLx, MiniJinja, deploy, auth/OAuth provider notes |
| [`.env.example`](.env.example) | Sample **runtime** env for a hello-world deploy |
| [`deploy/docker-build.env.example`](deploy/docker-build.env.example) | Sample **`DATABASE_URL` for `docker build`** (SQLx) |

---

## Quickstart

Use the devcontainer or local `.env` (see [`.env.example`](.env.example)) with `DATABASE_URL`, set `RUN_WEB=true` (and optionally `RUN_API`, `RUN_WORKERS`), then `cargo run`. `RUN_WORKERS=true` now starts the in-process memory-curate background worker. For deploys, compose now runs `den migrate` before `den serve`; locally, `cargo run` still applies migrations as part of the default startup path, and you use `sqlx migrate add` / `sqlx migrate run` when authoring new SQL migrations. Set **`JWT_SECRET`** when `RUN_API=true` or when using **`--features production`** (Docker release builds).

**Development-only link prefix:** Without `--features production`, [`src/config.rs`](src/config.rs) sets `URL_PREFIX` to `https://redirectmeto.com/http://localhost:3000/`. Generated absolute links (email verification, telemetry) therefore go through the third-party redirect service [redirectmeto.com](https://redirectmeto.com) before hitting your local app. Edit `URL_PREFIX` in that file if you prefer plain `http://localhost:…`, a tunnel URL, or another approach.

**Templates:** in **development**, MiniJinja loads files from `TEMPLATES_DIR` (default `src/web/templates`). In **`--features production`** / release Docker builds, templates are **embedded** at compile time—plan on **rebuilding** the binary when HTML changes in production.

**Fresh database:** an empty database is enough — the first `cargo run` or container start applies everything under `migrations/` (see [`migrations/README.md`](migrations/README.md), including default operator **`admin`**). For `SQLX_OFFLINE` / CI builds, run `cargo sqlx prepare` against a database that has seen those migrations at least once and commit `.sqlx/`.

**Mail:** `MAILGUN_API_KEY` and `MAILGUN_DOMAIN` default to empty; set them (or swap the mail implementation) before relying on outbound email.

**Den-native chat:** signed-in users chat with a Bear via **`GET /bear/{slug}`** and `POST /v1/chat/send` using session cookies. Den runs the native agent loop against Bifrost (`LLM_API_URL`) and per-Bear SQLite memory. Deep Chat is vendored under `src/web/assets/deep-chat/` (see [`docs/frontend-development.md`](docs/frontend-development.md#bear-chat-deep-chat)). See [`.env.example`](.env.example).

**ACP prompt memory status (June 2026):** persisted prompt-memory blocks are now Den-owned for ACP runtime prompt assembly, mutation/admin surfaces, and status inspection. Normal ACP runtime assembly uses persisted prompt-memory selection rather than synthetic fallback blocks; ACP inspection is available through session/status surfaces and `GET /bears/{slug}/sessions/{session_id}/prompt-memory`.

**Shutdown:** **Ctrl+C** is honored on all platforms; **SIGTERM** triggers graceful shutdown on **Unix** only.

---

## Sandbox provider (RUN_SANDBOX)

The `RUN_SANDBOX` service hosts isolated workspaces where headless
bear-armatures execute Docket work autonomously (`work` stance). It stands
alone — no Postgres — and may run on a different host from the Den instance
using it; Den talks to it over `/sandbox/v1` (static bearer token,
`SANDBOX_SERVICE_TOKEN`), and in-sandbox armatures dial back to Den over
BearWire.

Recommended dev setup (bare-metal provider, host docker):

1. Build the sandbox images: `scripts/build-sandbox-image.sh all` (base
   `bears/sandbox:latest` plus `bears/sandbox-{rust,node,godot}:latest`
   toolchain variants; see `packaging/sandbox-image/Dockerfile*`) — or use
   the `/admin/sandbox` build buttons once everything is running.
2. Roots and the image catalog are **Den-managed** (no config file): work
   surfaces at `/work/surfaces` (git upstream + optional encrypted
   credential, assigned to bears) and the catalog at `/admin/sandbox`
   (migration seeds the four `bears/sandbox*` entries). Den pushes both to
   the provider (`PUT /sandbox/v1/managed-config`) at worker startup, after
   every UI mutation, and on a 5-minute reconcile; the provider persists
   them under its workspaces dir. Git-backed surfaces are pristine
   server-managed bare clones, synced fetch/fast-forward-only before each
   provisioning; sandboxes get an ephemeral local clone at the requested
   ref. Dispatch selects images by **catalog name** only — raw image
   references never travel from Den.
3. Run the provider:
   `RUN_SANDBOX=true SANDBOX_SERVICE_TOKEN=devtoken cargo run -- serve`
4. Point the Den workers at it (in the Den process env):
   `SANDBOX_SERVER_URL=http://localhost:3002 SANDBOX_SERVER_TOKEN=devtoken`
   plus `SANDBOX_CALLBACK_API_URL` set to the Den API URL as reachable from
   inside containers (e.g. `http://host.docker.internal:3001`; must be
   plain http in the default restricted network mode).
5. Create a Docket job with tasks assigned to `work` — conversationally via
   `create_job`, or at `/work/new` — and dispatch with the `dispatch_work`
   tool or the job page's dispatch form (root + image selects). Watch
   progress and results at `/work`.

Publishing: with job `commit_policy` `per_task` or `per_job`, each successful
run's commits are pushed **host-side with the root's credentials** to the
job's upstream work branch (caller-specified; generated `den/job-<short-id>`
by default; the default ref is refused). Later runs of the same job provision
from that branch, so tasks build on each other. With `none`, no source changes
are expected and runs are not published.

Network: sandboxes default to `restricted` egress — a per-sandbox internal
network whose only way out is a socat relay to the Den callback endpoint, so
task code cannot reach anything but Den (`WORK_SANDBOX_NETWORK=open` opts
out; the run's `sandbox_strength` states the actual mode).

Compose alternative (part of the default stack): two services —
`bears-sandbox-provider` (this binary plus a docker CLI, built from the
`sandbox-provider` Dockerfile target) and `bears-sandbox-engine` (a
dedicated dind Docker Engine that hosts the sandbox containers, relays, and
networks). Nothing touches the host docker daemon, and `docker compose down
-v` removes every trace. The workers are wired to it by default
(`SANDBOX_SERVER_URL=http://bears-sandbox-provider:3002` and a matched
token default — override `SANDBOX_SERVER_TOKEN` in production). Catalog images must
exist in the **engine's** store: reference the GHCR-published
`ghcr.io/<owner>/bears-sandbox*` images (built by the sandbox-images
workflow), or build them into the engine with
`docker compose --profile sandbox-build run --rm bears-sandbox-images`.

## License

This project is licensed under the [MIT license](LICENSE.md).
