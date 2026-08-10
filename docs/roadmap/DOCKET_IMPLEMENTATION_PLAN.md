# Docket Implementation Plan

> **Note (2026-07).** Docket (ADR-0034) is part of the current direction and stays canonical for tasks/jobs in Den Postgres. Where this doc says Bear memory/runtime is "still Letta-backed", that is superseded: memory is per-Bear SQLite ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)) and the runtime is the in-process Den loop. See [Den runtime](../architecture/den-runtime.md) ([runtime plan](DEN_RUNTIME_PLAN.md)). Conversation-linked Docket objectives are the current target for task-list orientation; older session-checkout language from [ADR-0045](../decisions/adr-0045-session-task-lists-and-docket-checkout.md) should be read through that ownership model.

Docket is the Den control-plane subsystem for work management: the system of record for all tasks and the orchestrator for jobs. Its canonical model is specified in [ADR-0034: Jobs and Tasks Work-Management Model](../decisions/adr-0034-jobs-and-tasks-work-management.md). This document plans how that model is realized in the Den Rust source tree and how the bear/Den/Docket separation is enforced in code.

For the storage boundary rationale (memory is bear-canonical SQLite; structured work state is Docket-canonical Postgres), see ADR-0034 and the scope amendment in [ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md). ADR-0045 remains useful background on checkout/sync projection, but the active direction is to make conversation-linked Docket objectives the normal owner for mutable task lists rather than creating session-owned task containers.

## Subsystem shape

Den is the container. Both the bear runtime and Docket live inside Den. The separation is three subsystems with enforced seams, not three processes:

1. **Docket** — Postgres-canonical task/job orchestration. Owns its tables, migrations, and a service API.
2. **Bear memory** — SQLite-canonical cognition (per ADR-0031). **Native today**: implemented as per-Bear SQLite under `core/memory/` (`store/`, `curation.rs`); no longer Letta-backed.
3. **Bear runtime** — role harnesses (`role_runtime`, `pair_turn`, the `work` dispatch loop) that *consume* Docket and bear memory through service interfaces.

Two invariants the source tree must make hard to violate:

- Docket code never reaches into bear-memory storage, and vice versa (no shared DB handle, no cross-module direct table access).
- Execution flows through the bear role runtime, never Docket executing task bodies directly (the ADR-0034 execution invariant).

## Module boundary (now)

Docket is now a self-contained `den-docket` crate with a service seam consumed by Den tools/runtime. The original module-split work has landed; this section remains as the status record for the Docket boundary and the remaining runtime-dispatch/operator UX work.

> **Status (2026-07): relational Docket vertical slice landed and legacy compatibility paths retired.** `den-docket` is the `den-core`-only Docket crate with a `DocketService` (`PgDocketService`) public face and crate-internal `db.rs`. New Docket jobs/tasks are stored in ADR-0034 relational Postgres tables: `bear_jobs`, `bear_tasks`, `bear_job_runs`, task run state, job/task criteria, criteria state, and job/task events. Service APIs cover job/task CRUD, hierarchy reads, criteria evaluation, execution/run state, Docket-backed task-list checkout/sync projection, and the minimal `TaskDispatcher` seam for future `work` stance execution. Legacy `bear_work_plans` service/DB/model shims have been retired from active runtime crates, and legacy task-list provider aliases (`list_plans`, `get_plan_status`, `update_plan`, `request_work_handoff`) no longer resolve. Historical `bear_work_plans` references are limited to migrations/archive/docs; old data is handled by the destructive retirement migration or retained only as archived historical data in already-migrated deployments.

- Current crate layout:
  - `services/den/crates/den-docket/src/db.rs` — crate-internal Postgres access.
  - `services/den/crates/den-docket/src/model.rs` — Docket jobs/tasks/runs/criteria/events plus task-list projection types.
  - `services/den/crates/den-docket/src/service.rs` — `DocketService` / `PgDocketService`, the public persistence/orchestration face.
  - `services/den/crates/den-docket/src/dispatcher.rs` — `TaskDispatcher` seam for future `work` runtime dispatch.
- Bear-facing tools call `DocketService`, not Docket DB modules.
- Legacy `bear_work_plans`/`bear_work_plan_events` compatibility code and old provider aliases have been retired from active runtime paths; remaining table references are migrations/archive/docs only.

## Implementation ordering

Phases 0–4 are complete/retired in the current runtime. They remain below as the implementation record and as a guardrail for terminology. Phase 5 is the remaining Docket-related work: runtime dispatch through `work` plus operator UX.

### Phase 0 — Task-list language over current activity-board storage — done

**Goal:** Make the current visible session work surface model-facing as a **task list**, without yet claiming it is the relational Docket jobs/tasks model.

| Task | Done when |
| --- | --- |
| Rename model-facing provider names | Descriptors advertise `list_task_lists`, `get_task_list_status`, `update_task_list`, and `request_task_list_handoff`; legacy aliases (`list_plans`, `get_plan_status`, `update_plan`, `request_work_handoff`) are retired and do not resolve. |
| Update prompt/tool guidance | Model environment clearly distinguishes session task lists, Docket jobs/tasks, workplan artifacts, and roadmaps. |
| Keep internal canonical names honest | Active runtime crates use Docket/task-list names; `bear_work_plans` remains only in migrations/archive/docs. |
| Preserve ACP plan UI compatibility | ACP clients may still receive `plan` updates, but Den/armature copy should present them as task-list/focus-list updates where possible. |

**Exit gate:** Pair/ACP can list Docket-backed/session task-list context using task-list provider names; old provider aliases no longer work; relational Docket is canonical.

### Phase 1 — Task-list projection and source metadata — done

**Goal:** Maintain explicit task-list projection shapes over canonical Docket jobs/tasks. Historical `bear_work_plans.items` projection support is retired from active runtime paths.

| Task | Done when |
| --- | --- |
| Add task-list projection types | Code has `TaskListProjection`, `TaskListItem`, `TaskListSourceRef`, and `TaskListSyncState` wrapping Docket-backed or local projection payloads without legacy row models. |
| Represent local-only vs backed items | Items can say `source_ref.kind = "local"` or `"docket_task"`; local-only items remain projections until explicit sync/promotion semantics apply. |
| Preserve sync metadata | Projection includes `sync.state` such as `local_only`, `checked_out`, `dirty`, `synced`, `conflict`, or `review_required`. |
| Update Den tool payloads | `list_task_lists` / `get_task_list_status` expose task-list-shaped context without legacy row models; `update_task_list` no longer writes session-local legacy state and points callers to Docket checkout/sync/task tools. Passive prompt payloads are scoped to the current session/conversation and stance. |

**Exit gate:** Model-facing tools and ACP/BearWire plan UI consume task-list projection payloads, not raw `bear_work_plans.items` semantics.

### Phase 2 — Checkout/sync service seams — done

**Goal:** Define the operations that make session task lists effective working projections, before backing them with the full Docket relational schema.

| Task | Done when |
| --- | --- |
| Add checkout seam | `checkout_task_list` service/API shape can create or refresh a session task-list projection from a Docket job/task subtree. |
| Add sync seam | `sync_task_list` service/API shape can apply authorized item changes to backing records or surface conflicts/review requirements. |
| Add handoff/review seam | `request_task_list_handoff` remains the reviewed-promotion/reconciliation boundary; durable work is still created or updated through canonical Docket job/task APIs. |
| Define conflict behavior | Stale backed items surface `sync.state = "conflict"` rather than silently overwriting Docket or session state. |

**Exit gate:** Interfaces exist and are testable against Docket-backed task-list state.

### Phase 3 — Relational Docket schema — done

**Goal:** Land ADR-0034’s durable Docket model.

| Task | Done when |
| --- | --- |
| Add Docket schema | `bear_jobs`, `bear_tasks`, `bear_job_runs`, `bear_task_run_state`, `bear_job_criteria`, `bear_job_criteria_state`, and events exist in the Docket namespace. `bear_tasks.completion_criteria` gives each task concrete done-condition strings. |
| Add Docket service methods | `DocketService` exposes job/task CRUD, task hierarchy reads, checkout inputs, sync targets, run state, and criteria state through a public trait/API. Task create/update validates non-empty completion criteria. |
| Keep table access internal | Bear-facing tools call `DocketService`, not Docket DB modules. |
| Preserve execution invariant | Docket dispatches ready tasks to runtime-owned `TaskDispatcher`; it never executes task bodies itself. |

**Exit gate:** Docket jobs/tasks can be created/read/updated through the service API with tests; no runtime dispatch requirement yet.

### Phase 4 — Legacy activity-board migration — done

**Goal:** Retire `bear_work_plans` activity-board data and tools in favor of canonical task-list/Docket concepts without false naming.

| Task | Done when |
| --- | --- |
| Backfill/archive legacy rows | Existing `bear_work_plans` are handled by migration/archive policy; active runtime no longer reads or writes them. |
| Retire legacy provider names from advertisement | Model-facing descriptors advertise task-list/Docket names; old `*_plan*` provider aliases are not advertised and do not resolve. |
| Rename events carefully | `bear_work_plan_events` are either migrated to Docket events or retained as archived legacy activity events. |
| Update UI and operator copy | “Plan” copy remains only for plan mode/workplan artifacts/roadmaps; visible session checklists use task-list language. |

**Exit gate:** New turns do not create, read, or update raw legacy work-plan rows; compatibility code is removed from active runtime paths.

### Phase 5 — Conversation objectives, runtime dispatch, settlement, and operator UX — in progress

**Goal:** Use Docket for structured work state while keeping the conversation task list as the Bear/human working view, with accountable terminal outcomes and one consistent diagnostic story across model, conversation, run logs, and web UI.

> **Status (2026-08):** Runtime dispatch and the first settlement/journal vertical slices have landed. Docket now exposes durable task journals and job notebooks, typed terminal outcome dispositions, idempotent notebook promotion, and bounded notebook context at worker checkout. Dispatch supports an isolated sandbox or the current attached workspace; Pair contextually defaults to its sole attached workspace and otherwise uses or explicitly requests a sandbox. Dispatch is a coordination capability and is not exposed to the Work stance. Model-facing instructions for these workflows are compiled from concise coordination and execution prompt fragments rather than hardcoded Rust prose. The job UI shows notebook entries and append-only settlement history. Remaining work is primarily conversation-objective ownership/projection, output-contract resolution and enforcement, stalled-run/projection recovery, database-backed UAT, and end-to-end model/browser review.

| Task | Status | Done when |
| --- | --- | --- |
| Implement conversation-linked objectives | Remaining | A conversation that enters task orientation has one mutable Docket-backed objective representing the structured work state for that conversation. |
| Project active task context | Remaining | Runtime orientation projects the active top-level task/subtree from the conversation objective instead of owning separate session task state. |
| Implement `TaskDispatcher` integration | Landed | Docket dispatches ready durable tasks to `work` through the runtime-owned dispatch seam; worker checkout receives typed task context. Coordinating stances may select an isolated sandbox or the current attached workspace. Pair defaults to local only with exactly one attachment, defaults to sandbox with none, and requires an explicit target when attachment selection is ambiguous; Work cannot dispatch recursively because the tool is absent from that stance. |
| Add accountable terminal settlement | Landed; database UAT pending | Terminal updates create durable outcomes, retries are idempotent, reopen/resettle preserves history, and lifecycle-compatible typed dispositions are enforced. |
| Add journals and selectively shared notebook context | Landed; database UAT pending | Models can append/list task-journal and job-notebook entries, promote an entry by reference idempotently, and workers receive deterministic bounded notebook context. |
| Resolve and enforce output contracts | Remaining | Docket derives evidence requirements from assigned surfaces, mutation/publication policy, task criteria, and observed effects; required evidence is validated without inventing a separate model-authored output taxonomy. |
| Sync task-list changes through Docket | Partial | Authorized completion, edits, new subtasks, blockers, journal entries, and evidence update the conversation objective's Docket task tree. |
| Derive operational job status | Remaining | `job.status` is computed from explicit lifecycle intent plus canonical run, task, criterion, and settlement evidence through one shared normalizer. APIs, conversation status, operator UI, and logs show that same projection; no independently authoritative persisted job-status field remains. |
| Support stalled work runs | Remaining | Continuation loss or missing tool-progress confirmation records the run as `stalled`, retains last evidence and diagnostic, and does not manufacture `failed`/`cancelled`. Operators can wait/resume when supported, cancel/end, or resolve as failed; the job projection reports unresolved stalled work. |
| Test projection recovery | Remaining | Tests cover status precedence, stalled-run resolution, retry/reopen settlement journeys, and rebuilding projections from persisted evidence so partial writes or restarts cannot leave status inconsistent with outcomes. |
| Review end-to-end model experience | In progress | Fresh and existing sessions discover the same compiled guidance; completion, malformed evidence, retry, reopen, promotion, dispatch context, and recovery journeys produce actionable behavior without inviting invented evidence. Critical findings block completion. |
| Review Docket web UI end to end | In progress | Operators can inspect notebook and settlement history with matching normalized status/evidence across conversation and run/log views; loading, empty, error, permission, responsive, keyboard, and accessibility states are exercised in a running browser. Critical findings block completion. |
| Keep model guidance in context compilation | Ongoing invariant | Model-facing prompt text lives in registered prompt fragments rendered by context compilation. Rust carries typed context and structural tool schemas, not embedded behavioral prompt prose. |

**Exit gate:** A Bear can evolve a conversation-linked Docket objective across turns, work or dispatch its active task/subtree, and recover runtime/UI projections from canonical Docket facts. Terminal outcomes are accountable and idempotent, required output evidence is enforced by resolved policy, and unresolved continuation loss is visible as stalled rather than converted into an invented outcome. PostgreSQL-backed journey tests and end-to-end model/browser reviews have no unresolved critical findings.

### Conversation-linked task-list objectives

A conversation **task list** is the Bear/human working view for structured work in a conversation. It is Docket-backed once task orientation is invoked; the session/runtime projects it, but does not own it. The target invariant is:

- at most one mutable conversation-linked objective per conversation;
- top-level tasks under that objective represent the apparent "projects" or "jobs" that occur during the conversation;
- nested subtasks represent the current working decomposition;
- the active task/subtree is projection state over the Docket objective, not a separate source of truth;
- sessions, reconnects, and adapter bindings do not create separate objectives.

The intended flow is effectiveness-first:

1. Normal chat has no Docket objective.
2. When a conversation enters task orientation, Docket creates or reuses that conversation's mutable objective.
3. The Bear works the task list by updating statuses, editing task text, splitting items, adding subtasks, recording blockers, and attaching evidence.
4. Runtime and UI surfaces project the active top-level task/subtree as the current focus.
5. Conflicts surface in the task list rather than being silently overwritten.

Docket remains canonical for structured work state, task identity, task hierarchy, run state, criteria, evidence, and audit. Conversation task lists are the working projection and mutation surface. This boundary exists to preserve source-of-truth, recovery, audit, and dispatch semantics — not to prevent Bears from effectively working through a lightweight task list.

Conversation-linked objectives are conversation-local by default. Passive prompt context should include only the current conversation's active task/subtree for the current stance. Cross-conversation visibility belongs to explicit read tools (`list_task_lists`, Docket job/task tools) or to durable/promoted jobs, not automatic prompt injection.

Promotion to durable Jobs is future work. The likely shape is promoting a top-level task/subtree out of the conversation objective into a durable Docket Job while leaving a source link or transferred stub behind. Do not build that general promotion graph until a current workflow needs it.

Target Docket service capabilities should therefore include:

- get-or-create: create/reuse the conversation-linked objective when task orientation starts;
- project: return the active task/subtree and one-level-at-a-time views for runtime/UI;
- mutate: apply authorized task-list changes to the conversation objective's task tree;
- handoff/review: request review or future promotion of a top-level task/subtree;
- conflict detection: identify stale projections when Docket changed since the caller's view was produced.

### Storage namespacing

Docket uses the **shared Den Postgres connection pool** but a **distinct schema namespace** (e.g. a `docket` schema). Shared pool avoids pointless connection overhead; distinct schema makes the control-plane region legible and isolates Docket migrations. A separate pool was considered and rejected as overkill.

### Dispatch-direction seam

The one runtime touch-point between Docket and the bear is task dispatch. The dependency direction is deliberately **runtime-owns-the-trait, Docket-calls-out**:

- The bear runtime side owns a `dispatch` trait (e.g. `TaskDispatcher`).
- Docket emits "task T is ready to dispatch" and invokes the dispatcher; it never imports or holds an executor and never runs a task `body` itself.
- Pair/work task-list projections may update conversation-objective-backed or durable-job-backed Docket tasks, but execution still flows through the Bear runtime via this dispatcher seam.
- This direction-of-dependency mechanically prevents Docket from executing task content, enforcing the ADR-0034 execution invariant at compile time within the module structure.

### Symmetric treatment for bear memory

Bear memory already has the symmetric shape this seam assumes: `core/memory/` holds the per-Bear SQLite store (`store/`) and curation (`curation.rs`). Giving it a `MemoryStore` trait as its public face — so the runtime depends on the `DocketService` and `MemoryStore` *traits*, not on either subsystem's internals — is available now and is a prerequisite for the `den-memory` crate in [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md).

## Crate boundary (separate effort)

Promoting these module seams to compile-time **crate** boundaries — turning the single `den` crate into a Cargo workspace, motivated by build and test time (and used as a Rust-idiom refactor) — is tracked separately in [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md). It depends on the trait seams above (`DocketService`, `TaskDispatcher`) being in place, but is otherwise out of scope for Docket. Note that across crate boundaries the `TaskDispatcher` trait is defined by `den-docket` (the caller) and implemented by `den-runtime`, so Docket calls out without depending on the runtime crate — the same dependency direction described in the dispatch-direction seam, now enforced at the crate level.

## Relationship to existing plans

- Supersedes the schema/CRUD/handoff portions of [`TASK_SYSTEM_IMPLEMENTATION_PLAN.md`](TASK_SYSTEM_IMPLEMENTATION_PLAN.md) (phases 1–4); its runtime-dispatch and operator/UX phases (5–6) remain valid, read through ADR-0034, ADR-0045, and this plan.
- [`DEN_CRATE_SPLIT_PLAN.md`](DEN_CRATE_SPLIT_PLAN.md) consumes this plan's `DocketService`/`TaskDispatcher` trait seams and promotes them (and the `MemoryStore` seam) to Cargo workspace crate boundaries, motivated by build/test time.
- Session task-list checkout/sync semantics are canonicalized in [ADR-0045](../decisions/adr-0045-session-task-lists-and-docket-checkout.md) and should guide future model-facing `task_list` tool naming.
