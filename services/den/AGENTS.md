# AGENTS.md

How to orient in **Den**: this is the Rust service for Bear identity/session state, Den-hosted memory and work tools, BearWire, web/API surfaces, canonical conversation persistence, and the Den-native agent runtime.

Den is no longer just a generic Axum starter or a Letta/Codepool orchestration shim. The active Pair/ACP path is:

```text
ACP client / armature
        │
        ▼
bear-armature
        │ BearWire v1
        ▼
den-bearwire
        │
        ▼
Den-native Pair runtime
```

Key crates and areas:

- `crates/den-bearwire/` — BearWire RPC/SSE edge for armatures.
- `crates/den-runtime/` — native runtime, agent loop, conversation persistence, BearWire event projection, memory/runtime helpers.
- `crates/den-core/` — descriptor-owned Den tools, tool constants, dispatch, policy/context types.
- `src/core/tools/` — concrete Den tool context wiring for builtin Den-hosted tools.
- `migrations/` — Postgres schema.
- `src/lib.rs` / `src/main.rs` — binary composition and service startup.

For repository-wide project rules, Bear concepts, worktree safety, and stack commands, also read [`../../AGENTS.md`](../../AGENTS.md).

## BearWire / ACP runtime rules

- BearWire is the canonical Den ↔ armature wire. Prefer `den-bearwire` for new armature-facing behavior.
- Do not reintroduce adapter-SSE or legacy `/acp/**` hot-path behavior when BearWire can handle the flow.
- ACP/Zed is an armature, not a generic channel. It owns local filesystem/git/terminal/MCP execution and permission UI.
- Channels such as Slack, WhatsApp, web chat, and macOS app chat should be implemented as channel adapters, not as ACP armatures. See [`../../docs/roadmap/DEN_CHANNELS_IMPLEMENTATION_PLAN.md`](../../docs/roadmap/DEN_CHANNELS_IMPLEMENTATION_PLAN.md).

## Tool surfaces and routing

Den-native Pair sessions expose a stable mixed tool surface:

- Den-hosted tools:
  - `session_info`
  - `memory_write_entry`
  - `memory_status`
  - `memory_browse`
  - `memory_read`
  - `memory_search`
  - `memory_request_review`
  - `web_fetch`
  - `web_search`
  - `list_task_lists`
  - `get_task_list_status`
  - `update_task_list`
  - `request_task_list_handoff`
  - `set_conversation_title`
- Armature-local/client tools:
  - `fs_*`
  - `git_*`
  - `terminal_run_command`
  - `process_run`
  - forwarded MCP tools.

Rules:

- Do not use prompt heuristics to hide or reveal Pair tools turn-by-turn. This caused ACP sessions to lose filesystem capabilities after meta/capability questions.
- Route by descriptor ownership, not by ad hoc tool-name matches:
  - Den-hosted tools execute in Den through the Den tool dispatcher/invoker.
  - Armature-local and forwarded MCP tools are emitted to the armature/client.
- If the model sees a Den-hosted tool such as `list_task_lists`, Den must be able to execute it server-side. It must not reach `bear-armature` as an unsupported local tool request.
- Keep model-facing names descriptor-owned and concise; do not advertise legacy `den_*`, `situation_get`, `memory_tree`, `list_plans`, `get_plan_status`, `update_plan`, `request_work_handoff`, or implementation-branded names.

## Conversation history

- Canonical conversation persistence is the source of truth for transcript replay.
- Native runtime history loading should use shared transcript projection helpers, not raw `message_type` string checks.
- Keep model transcript replay and user-visible history as separate projections.
- For BearWire multi-turn fixes, test both:
  1. current turn persistence for future history;
  2. next-turn LLM request includes prior user and assistant messages exactly once.

## Verifying Rust changes (agents + dev containers)

**`cargo` is available** in typical dev containers and CI images that include the Rust toolchain. After editing this crate, run checks from the repository root with `--manifest-path services/den/Cargo.toml`, or from `services/den/` directly, for example:

- `cargo build` or `cargo check` — compile the library + binary; for host-side commands, prefix with `SQLX_OFFLINE=true` (details below).
- `cargo test` — unit tests; integration tests that need Postgres require `DATABASE_URL` and applied migrations (see [`docs/quickstart.md`](docs/quickstart.md)).
- `cargo clippy --all-targets` — Clippy is not suppressed at the crate root; the heaviest legacy bundle remains scoped on [`src/api/oauth/mod.rs`](src/api/oauth/mod.rs). Fix warnings in code you touch and shrink those module-level allows over time.

Do not assume the environment is “simulated only”: prefer running focused `cargo` checks yourself to catch compile errors before handing work back.

Useful focused checks:

> **SQLx offline builds:** Normal focused Rust checks must use the checked-in SQLx query metadata rather than trying to resolve a development database host. Prefix host-side commands with `SQLX_OFFLINE=true`, for example `SQLX_OFFLINE=true cargo check --manifest-path services/den/Cargo.toml -p den-web`. The Docker smoke-stack build already enables this. Do not replace SQLx compile-time macros with runtime queries to work around an unavailable database; refresh `.sqlx` with `cargo sqlx prepare --workspace` from an environment with the migrated database when queries or schema change.

```bash
SQLX_OFFLINE=true cargo test --manifest-path services/den/Cargo.toml -p den-bearwire bearwire_
cargo test --manifest-path services/den/Cargo.toml -p den-runtime pair_
cargo test --manifest-path services/den/Cargo.toml -p den-runtime den_tools_route_server_side_but_client_tools_do_not
cargo test --manifest-path services/den/Cargo.toml -p den-bearwire bearwire_
```

**Docker build:** For release/deploy-impacting changes, do not treat the change as complete until a `docker build` of [`Dockerfile`](Dockerfile) from `services/den/` succeeds. For narrow Rust/runtime changes, run the most specific cargo tests first and explicitly state if Docker was not run. Release images use `--features production`, Alpine/musl, and SQLx at build time in ways a local glibc `cargo check` does not fully replicate. When Docker is unavailable locally, say so explicitly (build-time env: [`docs/deploy.md`](docs/deploy.md), [`COOLIFY_DEPLOY.md`](COOLIFY_DEPLOY.md)).

## Start here

1. [`docs/README.md`](docs/README.md) — documentation index.
2. [`docs/concepts-overview.md`](docs/concepts-overview.md) — repository layout and where things live in code.
3. [`docs/quickstart.md`](docs/quickstart.md) — local development (env, migrations, `cargo run`).
4. [`docs/axum-in-this-repo.md`](docs/axum-in-this-repo.md) — how Axum routers, state, and layers map to `src/web` and `src/api`.
5. [`docs/development-principles.md`](docs/development-principles.md) — development principles (dependencies, frontend minimalism); populate for your product.
6. Implementation patterns under [`docs/`](docs/): SQLx, MiniJinja contexts, Axum handlers, infrastructure, frontend, deploy.

## Database migrations (SQLx)

- **Never edit** an existing file under `migrations/` that has already been applied anywhere: SQLx checksums the file content in `_sqlx_migrations`. **Add a new** `*_up.sql` for fixes or new columns (see [`migrations/README.md`](migrations/README.md)).
- New migrations should follow the reversible / expand-contract deployment policy documented in [`migrations/README.md`](migrations/README.md). In short: add a matching `.down.sql` for each new `.up.sql` unless explicitly justified, prefer backward-compatible expand steps first, and defer destructive contract changes until a later deploy.
- Den startup now rejects databases whose successful SQLx version is newer than the binary's embedded migrator. Keep deploy docs and migration reviews aligned with that guard; details live in [`migrations/README.md`](migrations/README.md) and [`COOLIFY_DEPLOY.md`](COOLIFY_DEPLOY.md).
- If checksum drift already happened, follow **Repairing checksum mismatch** in that README (`sqlx migrate info`, then align `checksum` with the canonical file).

## Working on features

- **BearWire / armatures** — `crates/den-bearwire/`, `crates/den-runtime/src/runtime/bearwire_projection/`, and `tools/bear-armature/` at the repo root.
- **Native runtime / agent loop** — `crates/den-runtime/src/agent_loop/`, `crates/den-runtime/src/native_runtime/`.
- **Den-hosted tools** — descriptors and dispatch in `crates/den-core/src/tools/`; concrete service wiring in `src/core/tools/`.
- **Conversation persistence/history** — `crates/den-runtime/src/conversation/`, `crates/den-runtime/src/native_runtime/turn.rs`, BearWire history/replay in `crates/den-bearwire/`.
- **HTTP web UI** — `src/web/`, templates under `src/web/templates/`. CSS: follow [`docs/frontend-development.md`](docs/frontend-development.md): no authored `<style>` blocks or inline layout/theme in templates; standalone pages still use `/assets/css/style.css` and scoped rules in `src/web/assets/css/specifics.css`.
- **HTTP API / OAuth provider** — `src/api/`.
- **Config** — `src/config.rs`, plus env and ops notes in [`docs/deploy.md`](docs/deploy.md), [`docs/infrastructure-and-ops.md`](docs/infrastructure-and-ops.md), and [`.env.example`](.env.example).
- **Entrypoint / workers** — [`src/lib.rs`](src/lib.rs) (`run()`), thin [`src/main.rs`](src/main.rs).

## After substantial changes

- If project focus shifts, suggest updates to [`docs/concepts-overview.md`](docs/concepts-overview.md) and any affected run/deploy docs under [`docs/`](docs/); update the root [`README.md`](README.md) only if you still use it as the primary human-facing overview.
- If you add a repeatable workflow, document it in `tasks.md` at the repo root (create if missing).

## Patterns (read when touching that layer)

| Topic | Doc |
|--------|-----|
| Development principles | [`docs/development-principles.md`](docs/development-principles.md) |
| SQLx macros & `cargo sqlx prepare` | [`docs/sqlx-patterns.md`](docs/sqlx-patterns.md) |
| `minijinja::context!` | [`docs/minijinja-context-patterns.md`](docs/minijinja-context-patterns.md) |
| Axum in this repo (routers, state, layers) | [`docs/axum-in-this-repo.md`](docs/axum-in-this-repo.md) |
| Axum routes & extractors (`{id}` not `:id`) | [`docs/axum-handler-patterns.md`](docs/axum-handler-patterns.md) |
| Services, deploy, ops | [`docs/infrastructure-and-ops.md`](docs/infrastructure-and-ops.md) |
| Local quickstart (`cargo run`, dev quirks) | [`docs/quickstart.md`](docs/quickstart.md) |
| Deploy notes | [`docs/deploy.md`](docs/deploy.md) |
| Frontend / templates | [`docs/frontend-development.md`](docs/frontend-development.md) |
| MiniJinja template limits (vs full Jinja2) | [`docs/minijinja-template-limitations.md`](docs/minijinja-template-limitations.md) |

## Planning docs (BEARS)

Use monorepo roadmap docs under [`../../docs/roadmap/`](../../docs/roadmap/) for active implementation plans.

Especially relevant for Den work:

- [`../../docs/roadmap/BEARWIRE_ARMATURE_WIRE_IMPLEMENTATION_PLAN.md`](../../docs/roadmap/BEARWIRE_ARMATURE_WIRE_IMPLEMENTATION_PLAN.md)
- [`../../docs/roadmap/DEN_CHANNELS_IMPLEMENTATION_PLAN.md`](../../docs/roadmap/DEN_CHANNELS_IMPLEMENTATION_PLAN.md)
- [`../../docs/roadmap/DEN_NATIVE_RUNTIME_PLAN.md`](../../docs/roadmap/DEN_NATIVE_RUNTIME_PLAN.md)
- [`../../docs/roadmap/DEN_CONTEXT_COMPACTION_IMPLEMENTATION_PLAN.md`](../../docs/roadmap/DEN_CONTEXT_COMPACTION_IMPLEMENTATION_PLAN.md)

Do not duplicate roadmap markdown under `services/den/plans/`.
