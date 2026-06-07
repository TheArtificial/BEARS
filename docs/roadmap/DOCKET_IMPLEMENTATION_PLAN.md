# Docket Implementation Plan

Docket is the Den control-plane subsystem for work management: the system of record for all tasks and the orchestrator for jobs. Its canonical model is specified in [ADR-0034: Jobs and Tasks Work-Management Model](../decisions/adr-0034-jobs-and-tasks-work-management.md). This document plans how that model is realized in the Den Rust source tree and how the bear/Den/Docket separation is enforced in code.

For the storage boundary rationale (memory is bear-canonical SQLite; tasks/jobs are Docket-canonical Postgres), see ADR-0034 and the scope amendment in [ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md).

## Subsystem shape

Den is the container. Both the bear runtime and Docket live inside Den. The separation is three subsystems with enforced seams, not three processes:

1. **Docket** — Postgres-canonical task/job orchestration. Owns its tables, migrations, and a service API.
2. **Bear memory** — SQLite-canonical cognition (per ADR-0031; not yet implemented, still Letta-backed).
3. **Bear runtime** — role harnesses (`role_runtime`, `pair_turn`, the `work` dispatch loop) that *consume* Docket and bear memory through service interfaces.

Two invariants the source tree must make hard to violate:

- Docket code never reaches into bear-memory storage, and vice versa (no shared DB handle, no cross-module direct table access).
- Execution flows through the bear role runtime, never Docket executing task bodies directly (the ADR-0034 execution invariant).

## Phase 1 (now): module boundary — "Option A"

Land Docket as a self-contained module within the existing `den` crate, with a trait seam designed so a later crate split is a mechanical move rather than a redesign.

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

### Symmetric treatment for bear memory (when SQLite lands)

When the ADR-0031 SQLite memory store is implemented, give it the symmetric shape: `core/bear_memory/` with its own SQLite handle (internal) and a `MemoryStore` trait as its public face. The runtime depends on the `DocketService` and `MemoryStore` *traits*, not on either subsystem's internals.

## Phase 2 (future): crate boundary — "Option B"

Promote the subsystem seams to compile-time crate boundaries once both sides of the protected boundary exist (i.e. after the SQLite bear-memory store is built). Splitting only Docket today would leave the most important boundary — memory — unmodeled, so the crate split is intentionally deferred until both halves can be isolated together.

Target workspace shape (illustrative, names TBD):

- `den-docket` — Postgres-canonical jobs/tasks; depends on no bear-memory crate.
- `den-bear-memory` — SQLite-canonical memory; depends on no Docket crate.
- `den-runtime` — role harnesses; depends on the `DocketService` and `MemoryStore` traits.
- `den` (binary/control plane) — wiring.

The compile-time guarantee: Docket literally cannot import bear-memory types because it does not depend on that crate, and vice versa. Because Phase 1 already routes all cross-subsystem access through traits, this phase is a move-and-rename, not a redesign.

### Trigger for Phase 2

Revisit the crate split when **both** are true:

- the SQLite bear-memory store (ADR-0031) is implemented, and
- the trait seams from Phase 1 have stabilized in practice.

## Relationship to existing plans

- Supersedes the schema/CRUD/handoff portions of [`TASK_SYSTEM_IMPLEMENTATION_PLAN.md`](TASK_SYSTEM_IMPLEMENTATION_PLAN.md) (phases 1–4); its runtime-dispatch and operator/UX phases (5–6) remain valid, read through ADR-0034 and this plan.
- The MemFS intent/approved-task pipeline remains the unattended, `review`-gated path and is out of Docket's scope.
