# Memory Write Topology Plan

## Status

**Complete** (2026-07-31). Implements the [ADR-0031 write-topology amendment](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md#write-topology-amendment-2026-07-30-one-owning-process-per-bear-database): one owning process per Bear database, with a single in-process write path.

- **Phase 1 landed** (commit `eaaf897a`): one `MemoryStoreManager` built at server startup; clones threaded through web/API state, the web chat runtime, reflection worker loops, provision/recall functions, and tool backends (`DenToolContext`). Production `::new` sites reduced from ~40 to 4 — startup plus the sanctioned short-lived CLIs (`reindex`, `import-legacy-memory`, `seed`), each commented.
- **Phases 2–3 landed** (commit `81fbad49`): `scripts/check-memory-write-topology.sh` enforces the allowlist in `lint.sh` and CI (`den-clippy.yml`); invariant noted in `AGENTS.md`; `import-legacy-memory` warns loudly on WAL/SHM sidecars suggesting a live runtime; worker co-location documented in [infrastructure-and-ops](../guides/infrastructure-and-ops.md).

## Current state (audited 2026-07-30)

The amendment's process-level claim already holds: reflection lanes (`memory_curate`, `recall_index`, `archive_harvest`, `context_compact`) run as **Tokio tasks inside the `den` binary** (spawned in `services/den/src/lib.rs`, gated by `RUN_WORKERS`), claiming Postgres queue rows via `FOR UPDATE SKIP LOCKED` in `den-runtime/src/reflection/conductor.rs`. There is no separate curation process.

The instance-level claim does **not** hold: `MemoryStoreManager` (`den-memory/src/manager.rs`) correctly opens each Bear database with `max_connections(1)` + WAL + `busy_timeout`, but many call sites construct **their own manager instance**, each owning an independent 1-connection pool to the same file:

- reflection worker loops (`conductor.rs` — fresh `MemoryStoreManager::new` per loop body),
- runtime tool layer (`services/den/src/core/tools/memory_read.rs`),
- den-web handlers (`bear/memory.rs`, `profile.rs`, `management.rs`),
- den-service (`bears/provision.rs`, `recall/query.rs`, `recall/reconcile.rs`),
- den-bearwire (`events.rs`).

`DenState` already carries a `memory_stores: MemoryStoreManager` field (`den-service/src/state.rs`), so the shared instance exists — it just isn't threaded everywhere. Today, in-process serialization silently relies on WAL locking + `busy_timeout`, which the amendment demotes to a defensive backstop.

CLI status: `den reindex` reads SQLite only (writes go to Postgres/Qdrant) — safe against a live runtime. `den import-legacy-memory` writes SQLite and is already guarded against Bears with existing memory, but has no live-runtime guard.

## Phases

### Phase 1 — One manager instance per process

Thread the `DenState` (or app-context) `MemoryStoreManager` through every call site instead of constructing new instances:

- reflection conductor loops take the shared manager as a parameter,
- tool backends (`memory_read.rs` and peers) resolve it from runtime context,
- den-web / den-service / den-bearwire call sites take it from `DenState`.

Endpoint: `MemoryStoreManager::new` is called exactly once in production code (process startup) plus `test_support.rs`. Because the manager keys pools by `bear_id`, sharing one instance yields a true single pool — and single connection — per Bear per process.

### Phase 2 — Enforce the invariant

- Make ad hoc construction hard: restrict `MemoryStoreManager::new` visibility (e.g. constructor gated behind the startup/state module, or `#[doc(hidden)]` + lint note in `AGENTS.md`), so new call sites fail review structurally rather than by convention.
- Add a CI grep/clippy-style check: no `MemoryStoreManager::new` outside the sanctioned modules.

### Phase 3 — Deployment and CLI guardrails

- **Deployment note:** `RUN_WORKERS` must be enabled in the same process that serves turns for a Bear's databases. Running a second `den` process with workers against the same data directory is unsupported; document in the self-hosting/compose docs.
- **`den import-legacy-memory`:** document (and where cheap, detect — e.g. a WAL-file heuristic or advisory lock probe) that the target Bear's runtime must be stopped. Refuse or warn loudly on suspected live databases.
- Keep WAL + `busy_timeout` settings unchanged — they remain the backstop for operator error.

## Verification

- Unit/integration test: concurrent writes from a worker task and a tool call share one pool (observable via pool identity or SQLite `busy` counters staying at zero under contention).
- CI check from Phase 2 passing.
- Existing DB-backed tests in `services/den/tests/` unaffected (`SQLX_OFFLINE` conventions per repo).

## Non-goals

- No change to WAL/synchronous/busy_timeout pragmas.
- No cross-process coordination mechanism (locks, lease files) beyond documentation and cheap CLI detection — cross-process writing is defined as a bug, not a mode to support.
