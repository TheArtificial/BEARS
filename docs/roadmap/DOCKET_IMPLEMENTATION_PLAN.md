# Docket Implementation Plan

> **Note (2026-06).** Docket (ADR-0034) is part of the current direction and stays canonical for tasks/jobs in Den Postgres. Where this doc says Bear memory/runtime is "still Letta-backed", that is superseded: memory is per-Bear SQLite ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)) and the runtime is the in-process Den loop. See [Den runtime](../architecture/den-runtime.md) ([runtime plan](DEN_RUNTIME_PLAN.md)). Session task-list checkout/sync semantics are specified in [ADR-0045](../decisions/adr-0045-session-task-lists-and-docket-checkout.md).

Docket is the Den control-plane subsystem for work management: the system of record for all tasks and the orchestrator for jobs. Its canonical model is specified in [ADR-0034: Jobs and Tasks Work-Management Model](../decisions/adr-0034-jobs-and-tasks-work-management.md). This document plans how that model is realized in the Den Rust source tree and how the bear/Den/Docket separation is enforced in code.

For the storage boundary rationale (memory is bear-canonical SQLite; tasks/jobs are Docket-canonical Postgres), see ADR-0034 and the scope amendment in [ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md). For the relationship between session-visible task lists and Docket task hierarchies, see [ADR-0045](../decisions/adr-0045-session-task-lists-and-docket-checkout.md).

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

> **Status (2026-07): relational Docket vertical slice landed.** `den-docket` is the `den-core`-only Docket crate with a `DocketService` (`PgDocketService`) public face and crate-internal `db.rs`. New Docket jobs/tasks are stored in ADR-0034 relational Postgres tables: `bear_jobs`, `bear_tasks`, `bear_job_runs`, task run state, job/task criteria, criteria state, and job/task events. Service APIs cover job/task CRUD, hierarchy reads, criteria evaluation, execution/run state, task-list checkout/sync projection, and the minimal `TaskDispatcher` seam for future `work` stance execution. Legacy `bear_work_plans` service/DB shims have been retired from `den-docket`; legacy checkout sources now return no relational task-list projection instead of reading or writing the old activity-board tables. The table may still exist for archived historical data until a dedicated data-retention migration removes or archives it.

- Create `core/docket/`, absorbing and evolving the current `core/work_plans.rs`:
  - `db.rs` — Postgres access, **internal** (`pub(crate)` at most; not exported past the module).
  - `model.rs` — `bear_jobs`, `bear_tasks`, `bear_job_runs`, `bear_task_run_state`, `bear_job_criteria`, `bear_job_criteria_state`, events.
  - `runs.rs`, `events.rs`, `criteria.rs` — domain logic.
  - `service.rs` — a `DocketService` trait that is the **only** public face of the module.
- Bear-facing tools in `core/tools/` call `docket::DocketService`, never `docket::db`. Enforce with module privacy now; a lint/import-restriction check can be added later.
- Migrate `bear_work_plans` + JSONB `items` → the ADR-0034 relational schema; rename `bear_work_plan_events` → `bear_job_events`; retire `den.work_plan.*` tools in favor of `den.job.*` / `den.task.*`.

## Implementation ordering

The implementation should not jump directly from the current legacy activity board to the full relational Docket schema. Do the task-list compatibility/projection slice first so models and UI stop learning overloaded “plan” terminology while storage still remains honest about the legacy shape.

### Phase 0 — Task-list language over current activity-board storage

**Goal:** Make the current visible session work surface model-facing as a **task list**, without yet claiming it is the relational Docket jobs/tasks model.

| Task | Done when |
| --- | --- |
| Rename model-facing provider names | Descriptors advertise `list_task_lists`, `get_task_list_status`, `update_task_list`, and `request_task_list_handoff`; legacy aliases (`list_plans`, `get_plan_status`, `update_plan`, `request_work_handoff`) remain accepted at routing boundaries. |
| Update prompt/tool guidance | Model environment clearly distinguishes session task lists, Docket jobs/tasks, workplan artifacts, and roadmaps. |
| Keep internal canonical names honest | Existing `den.work_plan.*`, `bear_work_plans`, and `WorkPlan*` names may remain internally until the relational Docket schema lands. |
| Preserve ACP plan UI compatibility | ACP clients may still receive `plan` updates, but Den/armature copy should present them as task-list/focus-list updates where possible. |

**Exit gate:** Pair/ACP can list/update a visible task list using task-list provider names; old provider aliases still work; no relational Docket migration required.

### Phase 1 — Task-list projection and source metadata

**Goal:** Introduce an explicit projection layer over the current `bear_work_plans.items` JSONB shape.

| Task | Done when |
| --- | --- |
| Add task-list projection types | Code has `TaskListProjection`, `TaskListItem`, `TaskListSourceRef`, and `TaskListSyncState` (or equivalent) wrapping legacy rows. |
| Represent local-only vs backed items | Items can say `source_ref.kind = "local"` or `"docket_task"` even if only local/legacy sources are supported initially. |
| Preserve sync metadata | Projection includes `sync.state` such as `local_only`, `checked_out`, `dirty`, `synced`, `conflict`, or `review_required`. |
| Update Den tool payloads | `list_task_lists` / `get_task_list_status` / `update_task_list` return task-list-shaped payloads while compatibility fields remain available. Passive prompt payloads are scoped to the current session/conversation and stance; explicit reads can still list visible task lists. |

**Exit gate:** Model-facing tools and ACP/BearWire plan UI consume task-list projection payloads, not raw `bear_work_plans.items` semantics.

### Phase 2 — Checkout/sync service seams

**Goal:** Define the operations that make session task lists effective working projections, before backing them with the full Docket relational schema.

| Task | Done when |
| --- | --- |
| Add checkout seam | `checkout_task_list` service/API shape can create or refresh a session task list from a local checklist, legacy activity board, roadmap section, or future Docket job/task subtree. |
| Add sync seam | `sync_task_list` service/API shape can apply authorized item changes to backing records or surface conflicts/review requirements. |
| Add handoff/review seam | `request_task_list_handoff` can request review/promotion of local-only items or unsynced edits into durable Docket work. |
| Define conflict behavior | Stale backed items surface `sync.state = "conflict"` rather than silently overwriting Docket or session state. |

**Exit gate:** Interfaces exist and are testable against local/legacy task-list state, even if Docket-backed checkout is still stubbed.

### Phase 3 — Relational Docket schema

**Goal:** Land ADR-0034’s durable Docket model.

| Task | Done when |
| --- | --- |
| Add Docket schema | `bear_jobs`, `bear_tasks`, `bear_job_runs`, `bear_task_run_state`, `bear_job_criteria`, `bear_job_criteria_state`, and events exist in the Docket namespace. `bear_tasks.completion_criteria` gives each task concrete done-condition strings. |
| Add Docket service methods | `DocketService` exposes job/task CRUD, task hierarchy reads, checkout inputs, sync targets, run state, and criteria state through a public trait/API. Task create/update validates non-empty completion criteria. |
| Keep table access internal | Bear-facing tools call `DocketService`, not Docket DB modules. |
| Preserve execution invariant | Docket dispatches ready tasks to runtime-owned `TaskDispatcher`; it never executes task bodies itself. |

**Exit gate:** Docket jobs/tasks can be created/read/updated through the service API with tests; no runtime dispatch requirement yet.

### Phase 4 — Legacy activity-board migration

**Goal:** Migrate current `bear_work_plans` activity board data and tools into the task-list/Docket model without false naming.

| Task | Done when |
| --- | --- |
| Backfill legacy rows | Existing `bear_work_plans` become session task lists, Docket jobs/tasks, or archived legacy projections according to migration rules. |
| Retire legacy provider names from advertisement | Model-facing descriptors advertise task-list/Docket names; old `*_plan*` names remain aliases only where needed for compatibility. |
| Rename events carefully | `bear_work_plan_events` are either migrated to Docket events or retained as archived legacy activity events. |
| Update UI and operator copy | “Plan” copy remains only for plan mode/workplan artifacts/roadmaps; visible session checklists use task-list language. |

**Exit gate:** New turns no longer create raw legacy work-plan rows except via compatibility shims.

### Phase 5 — Runtime dispatch and operator UX

**Goal:** Use Docket jobs/tasks for durable execution while keeping session task lists as the working view.

| Task | Done when |
| --- | --- |
| Implement `TaskDispatcher` integration | Docket can dispatch ready tasks to `work` runtime through the runtime-owned dispatch trait. |
| Checkout Docket tasks into sessions | `pair`/`work` can check out a Docket job/task subtree into a session task list. |
| Sync task-list changes to Docket | Authorized completion, edits, new subtasks, blockers, and evidence update Docket task/run state. |
| Operator UI reflects both views | Operators can see Docket job/task state and current session task-list projections without confusing their ownership. |

**Exit gate:** A Bear can check out a Docket parent task’s children, work them in-session, and sync authorized changes back to Docket.

### Session task-list checkout and sync

A session **task list** is the Bear/human working view used by `pair` or `work` stance during a session/run. It is not merely scratch state and not automatically a Docket task table row. It can contain:

- **local-only task-list items** for session focus, investigation, or emerging work not yet promoted to Docket;
- **Docket-backed task-list items** checked out from a Docket job/task subtree, commonly the children of a parent Docket task.

The intended flow is effectiveness-first:

1. `pair` or `work` checks out a Docket job/task subtree into a session task list.
2. The Bear works the task list in the session, updating statuses, editing task text, splitting items, adding subtasks, recording blockers, and attaching evidence.
3. Docket-backed items preserve `source_ref` / sync metadata so authorized changes can sync back to Docket.
4. Local-only items remain local until explicitly synced, handed off, promoted, or discarded.
5. Conflicts surface in the task list rather than being silently overwritten.

Docket remains canonical for durable jobs, task identity, task hierarchy, run state, criteria, and audit. Session task lists are the working projection and sync surface. This boundary exists to preserve source-of-truth, concurrency, audit, and dispatch semantics — not to prevent Bears from effectively working Docket tasks through a task list.

Session task lists are session- and stance-local by default. Passive prompt context should include only the current session/conversation's task list for the current stance. Cross-stance visibility belongs to explicit read tools (`list_task_lists`, Docket job/task tools) or to Docket-backed/promoted work, not automatic prompt injection.

Target Docket service capabilities should therefore include:

- checkout: create/refresh a session task list from a Docket job, parent task's children, roadmap section, or local checklist;
- sync: apply authorized task-list changes back to linked Docket tasks/jobs;
- handoff/review: request review or promotion of local-only task-list items or unsynced changes;
- conflict detection: identify stale task-list projections when Docket changed since checkout.

### Storage namespacing

Docket uses the **shared Den Postgres connection pool** but a **distinct schema namespace** (e.g. a `docket` schema). Shared pool avoids pointless connection overhead; distinct schema makes the control-plane region legible and isolates Docket migrations. A separate pool was considered and rejected as overkill.

### Dispatch-direction seam

The one runtime touch-point between Docket and the bear is task dispatch. The dependency direction is deliberately **runtime-owns-the-trait, Docket-calls-out**:

- The bear runtime side owns a `dispatch` trait (e.g. `TaskDispatcher`).
- Docket emits "task T is ready to dispatch" and invokes the dispatcher; it never imports or holds an executor and never runs a task `body` itself.
- Pair/work session task lists may project and update Docket-backed tasks, but execution still flows through the Bear runtime via this dispatcher seam.
- This direction-of-dependency mechanically prevents Docket from executing task content, enforcing the ADR-0034 execution invariant at compile time within the module structure.

### Symmetric treatment for bear memory

Bear memory already has the symmetric shape this seam assumes: `core/memory/` holds the per-Bear SQLite store (`store/`) and curation (`curation.rs`). Giving it a `MemoryStore` trait as its public face — so the runtime depends on the `DocketService` and `MemoryStore` *traits*, not on either subsystem's internals — is available now and is a prerequisite for the `den-memory` crate in [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md).

## Crate boundary (separate effort)

Promoting these module seams to compile-time **crate** boundaries — turning the single `den` crate into a Cargo workspace, motivated by build and test time (and used as a Rust-idiom refactor) — is tracked separately in [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md). It depends on the trait seams above (`DocketService`, `TaskDispatcher`) being in place, but is otherwise out of scope for Docket. Note that across crate boundaries the `TaskDispatcher` trait is defined by `den-docket` (the caller) and implemented by `den-runtime`, so Docket calls out without depending on the runtime crate — the same dependency direction described in the dispatch-direction seam, now enforced at the crate level.

## Relationship to existing plans

- Supersedes the schema/CRUD/handoff portions of [`TASK_SYSTEM_IMPLEMENTATION_PLAN.md`](TASK_SYSTEM_IMPLEMENTATION_PLAN.md) (phases 1–4); its runtime-dispatch and operator/UX phases (5–6) remain valid, read through ADR-0034, ADR-0045, and this plan.
- [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md) consumes this plan's `DocketService`/`TaskDispatcher` trait seams and promotes them (and the `MemoryStore` seam) to Cargo workspace crate boundaries, motivated by build/test time.
- Session task-list checkout/sync semantics are canonicalized in [ADR-0045](../decisions/adr-0045-session-task-lists-and-docket-checkout.md) and should guide future model-facing `task_list` tool naming.
- The MemFS intent/approved-task pipeline remains the unattended, `review`-gated path and is out of Docket's scope.
