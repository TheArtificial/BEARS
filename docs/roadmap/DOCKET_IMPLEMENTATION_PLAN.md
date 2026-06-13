# Docket Implementation Plan

> **Note (2026-06).** Docket (ADR-0034) is part of the current direction and stays canonical for tasks/jobs in Den Postgres. Where this doc says Bear memory/runtime is "still Letta-backed", that is superseded: memory is per-Bear SQLite ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)) and the runtime is the Den-native loop. See [Den-Native Runtime](../architecture/den-native-runtime.md) ([migration plan](DEN_NATIVE_RUNTIME_PLAN.md)).

Docket is the Den control-plane subsystem for work management: the system of record for all tasks and the orchestrator for jobs. Its canonical model is specified in [ADR-0034: Jobs and Tasks Work-Management Model](../decisions/adr-0034-jobs-and-tasks-work-management.md). This document plans how that model is realized in the Den Rust source tree and how the bear/Den/Docket separation is enforced in code.

For the storage boundary rationale (memory is bear-canonical SQLite; tasks/jobs are Docket-canonical Postgres), see ADR-0034 and the scope amendment in [ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md).

## Subsystem shape

Den is the container. Both the bear runtime and Docket live inside Den. The separation is three subsystems with enforced seams, not three processes:

1. **Docket** — Postgres-canonical task/job orchestration. Owns its tables, migrations, and a service API.
2. **Bear memory** — SQLite-canonical cognition (per ADR-0031). **Native today**: implemented as per-Bear SQLite under `core/memory/` (`store/`, `curation.rs`); no longer Letta-backed.
3. **Bear runtime** — role harnesses (`role_runtime`, `pair_turn`, the `work` dispatch loop) that *consume* Docket and bear memory through service interfaces.

Two invariants the source tree must make hard to violate:

- Docket code never reaches into bear-memory storage, and vice versa (no shared DB handle, no cross-module direct table access).
- Execution flows through the bear role runtime, never Docket executing task bodies directly (the ADR-0034 execution invariant).

## Module boundary (now)

Land Docket as a self-contained module within the existing `den` crate, with a trait seam designed so a later crate split is a mechanical move rather than a redesign. The crate split itself is a separate effort tracked in [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md); this plan only needs to land the module and its `DocketService`/`TaskDispatcher` trait seams.

> **Status (2026-06): minimal module + crate landed ("Level 1, honest naming").** `core/docket/` exists with a `DocketService` (`PgDocketService`) public face, an internal `db.rs`, and `model.rs`; it was promoted to the `den-docket` crate (`den-core`-only) per the crate-split plan. **Deliberately deferred to the relational realization below:** the `bear_jobs`/`bear_tasks`/`bear_job_runs`/criteria schema, `runs.rs`/`events.rs`/`criteria.rs`, the `bear_work_plans → bear_jobs` migration, and `den.work_plan.* → den.job.*/den.task.*`. The module still wraps the **legacy `bear_work_plans` activity board** and keeps those honest type names rather than minting `Job`/`Task` structs over the JSONB shape (which would mislabel the structure). `TaskDispatcher` is deferred to the `den-runtime` extraction (defined in its consumer per the dispatch-direction seam). The remaining bullets in this section describe the **target** shape.

- Create `core/docket/`, absorbing and evolving the current `core/work_plans.rs`:
  - `db.rs` — Postgres access, **internal** (`pub(crate)` at most; not exported past the module).
  - `model.rs` — `bear_jobs`, `bear_tasks`, `bear_job_runs`, `bear_task_run_state`, `bear_job_criteria`, `bear_job_criteria_state`, events.
  - `runs.rs`, `events.rs`, `criteria.rs` — domain logic.
  - `service.rs` — a `DocketService` trait that is the **only** public face of the module.
- Bear-facing tools in `core/tools/` call `docket::DocketService`, never `docket::db`. Enforce with module privacy now; a lint/import-restriction check can be added later.
- Migrate `bear_work_plans` + JSONB `items` → the ADR-0034 relational schema; rename `bear_work_plan_events` → `bear_job_events`; retire `den.work_plan.*` tools in favor of `den.job.*` / `den.task.*`.

### Storage namespacing

Docket uses the **shared Den Postgres connection pool** but a **distinct schema namespace** (e.g. a `docket` schema). Shared pool avoids pointless connection overhead; distinct schema makes the control-plane region legible and isolates Docket migrations. A separate pool was considered and rejected as overkill.

### Dispatch-direction seam

The one runtime touch-point between Docket and the bear is task dispatch. The dependency direction is deliberately **runtime-owns-the-trait, Docket-calls-out**:

- The bear runtime side owns a `dispatch` trait (e.g. `TaskDispatcher`).
- Docket emits "task T is ready to dispatch" and invokes the dispatcher; it never imports or holds an executor and never runs a task `body` itself.
- This direction-of-dependency mechanically prevents Docket from executing task content, enforcing the ADR-0034 execution invariant at compile time within the module structure.

### Symmetric treatment for bear memory

Bear memory already has the symmetric shape this seam assumes: `core/memory/` holds the per-Bear SQLite store (`store/`) and curation (`curation.rs`). Giving it a `MemoryStore` trait as its public face — so the runtime depends on the `DocketService` and `MemoryStore` *traits*, not on either subsystem's internals — is available now and is a prerequisite for the `den-memory` crate in [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md).

## Crate boundary (separate effort)

Promoting these module seams to compile-time **crate** boundaries — turning the single `den` crate into a Cargo workspace, motivated by build and test time (and used as a Rust-idiom refactor) — is tracked separately in [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md). It depends on the trait seams above (`DocketService`, `TaskDispatcher`) being in place, but is otherwise out of scope for Docket. Note that across crate boundaries the `TaskDispatcher` trait is defined by `den-docket` (the caller) and implemented by `den-runtime`, so Docket calls out without depending on the runtime crate — the same dependency direction described in the dispatch-direction seam, now enforced at the crate level.

## Relationship to existing plans

- Supersedes the schema/CRUD/handoff portions of [`TASK_SYSTEM_IMPLEMENTATION_PLAN.md`](TASK_SYSTEM_IMPLEMENTATION_PLAN.md) (phases 1–4); its runtime-dispatch and operator/UX phases (5–6) remain valid, read through ADR-0034 and this plan.
- [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md) consumes this plan's `DocketService`/`TaskDispatcher` trait seams and promotes them (and the `MemoryStore` seam) to Cargo workspace crate boundaries, motivated by build/test time.
- The MemFS intent/approved-task pipeline remains the unattended, `review`-gated path and is out of Docket's scope.
