# ADR: Jobs and Tasks Work-Management Model

**Status:** Proposed
**Date:** 2026-06-07
**Deciders:** Hans

> **Amended by [ADR-0045](adr-0045-session-task-lists-and-docket-checkout.md).** Docket remains canonical for durable jobs/tasks, but session task lists are the Bear/human working projection. A task-list item may be local-only or backed by a Docket task, and authorized task-list changes may sync back to Docket. Read this ADR's "session-bound tasks" language through that checkout/projection model rather than as a claim that every session task-list item is itself a Docket task.

> **Amended by [ADR-0056](adr-0056-docket-driven-turn-routing.md).** Tasks gain routing/placement metadata (`routing_strategy`, plus advisory `expected_context_size` and `result_rollup_policy`); result rollups are recorded as append-only, run-scoped `bear_task_events` entries read latest-per-child by the parent. This ADR's invariants stand unchanged: task rows still hold no status, result, or model name; execution profiles are resolved by the ADR-0033 model-tasks layer at dispatch and recorded on routing decisions and runs.

## Context

BEARS already has several overlapping work-management surfaces:

- `bear_work_plans` + `bear_work_plan_events` — a live activity board with a JSONB `items` array, owned per role, enforcing one `in_progress` item (see [`TASK_SYSTEM_IMPLEMENTATION_PLAN.md`](../roadmap/TASK_SYSTEM_IMPLEMENTATION_PLAN.md)).
- A MemFS file-based task pipeline — intent (`chat/tasks/`, `pair/tasks/`) → approved task (`core/tasks/`) → run result (`work/results/`), mediated by `review`.
- ACP `pair` Ask/Plan/Write modes and plan-mode artifacts (see [`planning.md`](../architecture/planning.md)).

This works but leaves the unit of work under-specified. We want a first-class, persisted **work-management model** usable by both `pair` (planning and delegating) and `work` (executing), with:

- a named container ("job") carrying a goal and acceptance criteria,
- a hierarchical, self-contained unit of work ("task") aligned with the "bead" model from Steve Yegge's *beads* (each task is executable in roughly one turn and carries its own prompt/body),
- a status report publishable to users,
- live, audited modification of the task tree during execution (including decomposition by `work`),
- and observability strong enough to detect chronic vs. anomalous effort on a stable task identity over time.

This ADR also resolves how the new model relates to and supersedes parts of the existing work-plan and task documents, and how it sits beside adjacent ADRs.

### Relationship to adjacent decisions

- [ADR-0006 (Bear work surfaces)](adr-0006-bear-work-surfaces.md): jobs attach to a work surface. The normalized `bear_work_surfaces` entity does not yet exist, so jobs carry a nullable `work_surface_ref` slug until it does.
- [ADR-0026 (Work handoff and human escalation)](adr-0026-work-handoff-and-human-escalation.md): a job that blocks for human input opens a `work_handoffs` record. Handoff is a **peer** concept, not owned by the job.
- [ADR-0027 (Workflow state ontology)](adr-0027-workflow-state-ontology.md): jobs span the **workplan** domain (goal, acceptance criteria, task definitions) and the **activity** domain (run status, report). Task execution state is the **execution** domain. These remain distinct; this ADR labels each table accordingly.
- [ADR-0031 (SQLite-first canonical store)](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md): adopts that ADR's append-only `task_events` shape. This ADR implements it in **Den Postgres**, not SQLite. The event taxonomy here is the authoritative one for jobs/tasks.
- [ADR-0033 (Model tasks layer)](adr-0033-model-tasks-layer.md): task `difficulty`/`effort_hint` are advisory inputs to model/effort selection. Model names are **not** stored on tasks; the model-tasks policy layer maps difficulty → model + effort.
- [ADR-0023 (Task focus supervisor)](adr-0023-task-focus-supervisor.md): superseded/re-homed by [ADR-0035](adr-0035-den-native-in-process-agent-runtime.md) + [ADR-0039](adr-0039-trust-profiles-and-governance.md). The supervisor is **not** orthogonal to this model: a job's acceptance criteria (`bear_job_criteria`) are the on-task **definition of done** it enforces, and continuation bias is the run's **governance** (ADR-0039), not the trust profile. The ephemeral focus record stays ephemeral, but is a projection over Docket run/task/criteria state plus governance.

## Decision

BEARS adopts a two-level durable work-management model — **jobs** and **tasks** — that evolves `bear_work_plans` into a relational structure, with a **run** entity present from day one so execution state and observability are run-scoped.

### Core principles

1. **Jobs are human-initiated.** Only humans create jobs, via `pair`, `chat`, or the UI directly. Agents do not self-originate jobs.
2. **The `review` agent role is not a required gate.** Quality gating of a job (acceptance-criteria satisfaction, pre-allocation review for decisions) is managed by the job system itself. The Reflection/`review` conductor may log a completed job to Cabinet, but the `review` *agent* need not be invoked to run a job.
3. **One canonical Docket concept of "task."** Durable Docket tasks are persisted whether or not they belong to a job. Per [ADR-0045](adr-0045-session-task-lists-and-docket-checkout.md), session task-list items are a working projection: they may be local-only or Docket-backed, and only Docket-backed items are canonical Docket tasks.
4. **Tasks are bead-like.** Each task carries a self-contained `body` (its prompt/goal) executable in roughly one turn. More complex work is expressed as child tasks. Hierarchy is a simple tree; there are no n:n dependency edges.
5. **Tasks are definitions; runs hold execution state.** A task's identity is stable and owned by the job. Status, results, and per-run telemetry live against a run, not on the task row.
6. **Decomposition is live and audited.** Tasks added during execution are reflected in the report immediately and recorded with who added them, when, and in which run.
7. **Storage is Den control-plane.** Jobs/tasks orchestration records live in Den Postgres, not per-Bear SQLite. The Bear *uses* Den's job-management platform to organize its work, the way a person uses a project tracker; the platform is infrastructure the Bear plugs into, not part of the Bear's cognition. This narrows [ADR-0031](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), which keeps SQLite canonical for Bear *memory*. See the **execution invariant** below.

### Execution invariant

This control-plane placement holds **only** because Job execution stays inside the Bear. Den dispatches a Job to the Bear's own role runtime (e.g. `work`) **running with the Bear's scoped memory context**. One Job Run owns one sandbox/workspace/session and advances the Job's task tree with at most one task `in_progress` at a time. Den schedules, gates (acceptance criteria), and records; it does **not** execute task `body` content via generic, non-Bear subagents. If Den ever executed task bodies outside the Bear's memory context, the task tree would be Bear cognition smuggled into the control plane, and the storage decision in principle 7 (and the ADR-0031 amendment it rests on) would no longer be justified.

### Docket, `pair`, and the bear/Den boundary

The job-management platform described by this ADR is named **Docket**. Docket is a Den control-plane subsystem: it is the system of record for all tasks and the orchestrator for jobs. The bear/Den boundary that [ADR-0031](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md) protects is drawn at **memory** (bear-canonical, SQLite), **not** at tasks. Tasks and jobs are Docket-canonical (Postgres). There is no bear-local task store and no sync seam.

This matters because `pair` is a Den-hosted runtime (it is already API-direct to Den for memory, situation, and search tools). `pair` using Docket for tasks is therefore *not* a new boundary crossing — it is the same Den surface `pair` already uses. The clean boundary is `pair` ↔ bear *memory*, which stays in SQLite and is untouched by task work.

Three usage patterns all resolve to the same single store, with no data crossing a store boundary:

1. **In-session focus.** `pair` maintains a session task list to stay focused mid-conversation. Items may be local-only or Docket-backed; local-only items are not Docket tasks until promoted/synced.
2. **ACP task-list projection.** The `pair` harness renders task-list state as ACP plan/task-list entries. This is a projection for the client, not a separate source of truth.
3. **Checking out Docket work.** `pair` or `work` can check out a Docket job/task subtree into the session task list. Docket-backed items preserve their Docket identity and can sync authorized changes back to Docket.

#### Session task lists vs. Docket ownership

Per [ADR-0045](adr-0045-session-task-lists-and-docket-checkout.md), a session task list is a working view. It can contain:

- **local-only task-list items**: owned by the session task list; useful for focus, exploration, or work not yet promoted to Docket;
- **Docket-backed task-list items**: projections of durable Docket tasks, typically checked out from a job/task subtree.

Docket tables remain canonical only for Docket-backed items. Local-only task-list items should preserve enough source/sync metadata to be promoted, handed off, or discarded deliberately.

### Schema

All tables are in Den Postgres. Domain labels per ADR-0027 are noted.

#### `bear_jobs` — container (workplan + activity domains)

```text
bear_jobs
- id                  uuid PK
- bear_id             uuid FK
- created_by_user_id  uuid NOT NULL FK        -- humans only
- created_by_role     bear_agent_role          -- channel the human used (pair | chat | ui)
- goal                text NOT NULL            -- workplan: intent
- work_surface_ref    text nullable            -- slug; FK once ADR-0006 lands
- commit_policy       commit_policy nullable   -- none | per_task | per_job | propose_only
- status              job_status               -- activity: draft | ready | running | blocked | completed | cancelled
- visibility          work_plan_visibility     -- reuse existing enum
- current_run_id      uuid nullable FK         -- pointer to the active/latest run
- created_at, updated_at
```

#### `bear_job_criteria` — acceptance criteria (workplan domain)

```text
bear_job_criteria
- id            uuid PK
- job_id        uuid FK
- kind          criterion_kind   -- narrative | command | check_ref
- description   text NOT NULL    -- always human-readable
- spec          jsonb nullable   -- command: {cmd, expect_exit_0}; check_ref: reference
```

- `narrative`: judged by an agent or human.
- `command` (hard gate): a command Den can run; e.g. `cargo test --lib` must exit 0.
- `check_ref`: points at external state (CI status, review approval).

A job **cannot** reach `completed` while any `command` criterion is `unmet` in the active run. The acceptance criteria are injected into `work` dispatch context as the success contract. Per-criterion evaluation is run-scoped (see `bear_job_criteria_state`).

#### `bear_tasks` — bead definitions (execution domain; pure definition)

```text
bear_tasks
- id                uuid PK
- bear_id           uuid FK
- job_id            uuid nullable FK -> bear_jobs
- session_anchor_id uuid nullable FK -> acp_sessions  -- lightweight use (no job)
- parent_task_id    uuid nullable self-ref
- sibling_order     int NOT NULL DEFAULT 0            -- strict sequential for v1
- kind              task_kind          -- execution | investigation | decision
- scope             task_scope         -- template | run  (see decomposition)
- title             text NOT NULL
- body              text NOT NULL      -- self-contained prompt
- completion_criteria jsonb NOT NULL   -- array of concrete done-condition strings
- difficulty        task_difficulty nullable  -- trivial | moderate | hard | unknown (advisory)
- effort_hint       effort_level nullable     -- low | medium | high (advisory)
- assigned_to_role  bear_agent_role nullable
- created_by_role   bear_agent_role
- created_at, updated_at
```

The task row holds **no status or result** — those are run-scoped. It does hold concrete `completion_criteria`: a lightweight array of done-condition strings that gives the model a stopping target for task execution. `task.kind` does double duty: pre-allocation review can scan for `decision` tasks needing human pre-decisions before dispatch, and a `decision` task moving to `blocked` is the structured, API-legible "stuck, needs input" signal that opens a `work_handoffs` record (ADR-0026).

#### `bear_job_runs` — execution log (activity domain)

```text
bear_job_runs
- id            uuid PK
- job_id        uuid FK
- trigger       run_trigger   -- manual | scheduled | event
- schedule_ref  text nullable -- the schedule that spawned it, when recurring
- state         run_state     -- dispatched | running | paused | completed | failed | cancelled
- started_at, finished_at
- outcome       jsonb nullable
```

Every job has **at least one** run. A oneshot job has exactly one. Recurrence (deferred) means the scheduler creates additional runs against the *same* job; no schema change is required.

#### `bear_task_run_state` — per-run task execution state (execution domain)

```text
bear_task_run_state
- run_id          uuid FK -> bear_job_runs
- task_id         uuid FK -> bear_tasks
- status          task_status   -- pending | in_progress | done | blocked | cancelled
- result_refs     jsonb nullable -- {commit_sha, pr_url, file_paths, ...}
- result_summary  text nullable
- started_at, finished_at, updated_at
- PRIMARY KEY (run_id, task_id)
```

At most one task per run may be `in_progress` (the evolved form of the current one-`in_progress`-item rule).

#### `bear_job_criteria_state` — per-run criterion evaluation (activity domain)

```text
bear_job_criteria_state
- run_id        uuid FK
- criterion_id  uuid FK -> bear_job_criteria
- status        criterion_status -- unmet | met | waived
- evaluated_at  timestamptz nullable
- evidence      jsonb nullable
- PRIMARY KEY (run_id, criterion_id)
```

#### `bear_task_events` — canonical task history (ADR-0031 shape)

```text
bear_task_events
- id          uuid PK
- task_id     uuid FK
- run_id      uuid nullable FK     -- non-null for execution events; null for template edits
- event_type  task_event_type      -- created | claimed | started | progressed
                                    --  | blocked | completed | cancelled | child_added
- by_role     bear_agent_role
- by_agent_id text nullable
- by_user_id  uuid nullable
- payload     jsonb
- created_at
```

`child_added` records live decomposition: the parent task, the new child's id, `by_role`/`by_agent_id`/`by_user_id`, and the run. It bubbles up to a `task_added` job event. Execution events carry `run_id`; definitional edits (reordering, human refinement between runs) carry a null `run_id`.

#### `bear_job_events` — report + audit stream (activity domain)

```text
bear_job_events
- id          uuid PK
- job_id      uuid FK
- run_id      uuid nullable FK
- event_type  job_event_type   -- job_created | task_added | task_updated | criterion_evaluated
                                --  | job_blocked | job_completed | job_cancelled
                                --  | handoff_requested | note_added | run_started | run_finished
- task_id     uuid nullable
- by_role     bear_agent_role
- by_agent_id text nullable
- by_user_id  uuid nullable
- payload     jsonb
- created_at
```

The user-facing **status report** is a projection of `bear_job_events`, not a separate mutable text field. This keeps the report auditable and consistent with the event history.

Runtime checkpoints from ADR-0050 are not `bear_task_events` or `bear_job_events`. They may reference Docket run/task ids for loop-control continuity, but durable task progress, blockers, completion, criteria evaluation, and report-visible history still require explicit task/job events produced through the task-management path.

### Decomposition, template vs. run scope, and learning

Tasks created during planning by a human or `pair` default to `scope = template` — they are durable parts of the job definition. Tasks created by `work` mid-execution default to `scope = run` — they are run-local improvisation, not assumed wise. A run-scoped task may be **promoted** to `template` when observation justifies it (e.g. `work` recreates an equivalent child in most runs). Conversely, a chronically trivial or skipped template task may be **pruned**.

This promotion/pruning loop is the explicit mechanism for learning job structure over time without trusting the executing model's one-shot judgment. It is enabled by stable task identity plus the run dimension, both present from day one.

### Tool surface

Evolves the `den.work_plan.*` tools.

- **Job management (`pair` / `chat` / UI):** `den.job.create` (goal, criteria, initial task tree, work_surface_ref, commit_policy), `den.job.get` (job + task tree + rendered report), `den.job.list`.
- **Task tree (all roles):** `den.task.create` (body, kind, parent_task_id, job_id or session_anchor_id), `den.task.update` (status/result via run context), `den.task.list`.
- **Execution (`work`):** receives one Job dispatch from Den. The Job Run retains one sandbox/workspace/session while `work` advances the task tree (one `in_progress` task at a time), with Job goal and acceptance criteria injected; it may call `den.task.create` to add run-scoped children.

The `den.work_plan.request_handoff` tool is retired — job creation *is* the handoff from planning to durable work.

### Scope of v1

Built now, because identity and the run dimension are ruinous to retrofit:

- All tables above, including `bear_job_runs`, `bear_task_run_state`, `run_id` on events, and the `template | run` scope flag.
- Stable, canonical task identity owned by the job.
- One run per oneshot job.

Deferred — **only the scheduler behavior**, not the data model:

- Cron/trigger wiring, recurrence policy, and event-triggered runs. When added, recurrence is "the scheduler creates more runs against the same job" with no migration, no backfill, and full retroactive observability.

### Tests, commits, and documentation

These behaviors are **not** modeled as task-type enums. They belong to work-surface instructions (per ADR-0006 anchors and the orientation recommendations) referenced through acceptance criteria and `commit_policy`. When a job's `work_surface_ref` resolves to a git surface, Den injects that surface's git/test/doc conventions into each task dispatch. This keeps the task schema surface-agnostic (research, design, and ops work surfaces are not git-centric) while making code conventions mandatory context where they apply.

## Consequences

### Positive

- One persisted, hierarchical task concept usable with or without a job.
- Stable task identity + run-scoped state gives cross-run observability (chronic vs. anomalous effort) from the first recurring run, with no retrofit.
- Acceptance criteria provide enforceable hard gates owned by the job system, not the `review` agent.
- Live, audited decomposition with explicit authorship and a structure-learning loop.
- Recurrence becomes a behavioral feature, not a data-model migration.

### Negative / costs

- A task has no intrinsic status; every status read requires a run context (`current_run_id` resolves the common case).
- `current_run_id` is denormalized state that must be kept correct under run lifecycle transitions.
- More tables than the JSONB `items` approach; more join surface for reads.

### Migration and supersession

- `bear_work_plans.items` (JSONB) → `bear_tasks` rows.
- `bear_work_plan_events` → `bear_job_events`.
- `den.work_plan.*` tools → `den.job.*` + `den.task.*`; `request_handoff` retired.
- [`TASK_SYSTEM_IMPLEMENTATION_PLAN.md`](../roadmap/TASK_SYSTEM_IMPLEMENTATION_PLAN.md) phases 1–4 are superseded by this ADR; phases 5–6 (runtime dispatch, operator/chat UX) remain valid with updated schema references.
- The MemFS intent/approved-task pipeline remains valid for unattended, `review`-gated recurring/observation work that has no human-initiated job container. Jobs are the path for human-initiated, goal-directed work.

## Open questions

- Concurrency within a sibling set is sequential by `sibling_order` for v1; job-level concurrency only. Intra-job fan-out (subagents per sibling) is a later revisit.
- Exact projection rules for rendering `bear_job_events` into a user-facing report (verbosity, redaction by visibility) need a follow-on spec.
- Promotion/pruning policy (thresholds, who decides) is described conceptually here; the concrete heuristic belongs with the observability work.
