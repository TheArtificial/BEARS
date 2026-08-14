# ADR: Jobs and Tasks Work-Management Model

**Status:** Proposed
**Date:** 2026-06-07
**Deciders:** Hans

> **Amended by [ADR-0045](adr-0045-session-task-lists-and-docket-checkout.md).** Docket remains canonical for durable jobs/tasks, but session task lists are the Bear/human working projection. A task-list item may be local-only or backed by a Docket task, and authorized task-list changes may sync back to Docket. Read this ADR's "session-bound tasks" language through that checkout/projection model rather than as a claim that every session task-list item is itself a Docket task.

> **Amended by [ADR-0056](adr-0056-docket-driven-turn-routing.md).** Tasks gain routing/placement metadata (`routing_strategy`, plus advisory `expected_context_size` and `result_rollup_policy`); result rollups are recorded as append-only, run-scoped `bear_task_events` entries read latest-per-child by the parent. This ADR's invariants stand unchanged: task rows still hold no status, result, or model name; execution profiles are resolved by the ADR-0033 model-tasks layer at dispatch and recorded on routing decisions and runs.

> **2026-07-14 amendment — journals, notebooks, and checkpoints.** Docket entries provide the durable semantic record for task outcomes and selectively shared job knowledge. Every terminal task settlement requires an `outcome` journal entry; the applicable output contract determines whether that outcome also requires independently inspectable evidence. Runtime checkpoints remain tactical continuation control under ADR-0050, not automatic history, but may deliberately append a durable entry when one is warranted.
>
> **2026-08-10 amendment — Pair settlement is independent of commit delivery.** In `pair`, a user or model may declare a task terminal under its normal authority and summary requirements. Docket records that declared outcome atomically; `commit_policy` schedules runtime-owned commit delivery (`none`, `per_task`, or `per_job`) after settlement and is not a prerequisite for it. Delivery success, failure, retry, and any stronger publication requirement are separate evidence/state. An explicit Work or release policy may gate *job delivery* on finalized output, but it must not suppress or undo an individual task's factual terminal outcome.

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

- [ADR-0006 (Bear work surfaces)](adr-0006-bear-work-surfaces.md): jobs assign zero or more work surfaces. Each assignment states whether mutation is `required`, `optional`, or `forbidden`; the work surface and its typed adapter define how any resulting output is materialized and observed.
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
3. **One canonical Docket concept of "task."** Every durable Docket task has exactly one owner: either a session for lightweight stance-owned work or a Job for durable, dispatchable work. A task cannot be unowned or jointly session- and Job-owned. Per [ADR-0045](adr-0045-session-task-lists-and-docket-checkout.md), session task-list items are a working projection: they may be local-only or Docket-backed, and only Docket-backed items are canonical Docket tasks.
4. **Tasks are bead-like.** Each task carries a self-contained `body` (its prompt/goal) executable in roughly one turn. More complex work is expressed as child tasks. Hierarchy is a simple tree; there are no n:n dependency edges.
5. **Tasks and runs hold execution state; jobs project it.** A task's identity is stable and owned by exactly one session or Job. Status, results, and per-run telemetry live against a run, not on the task row. The operational job status is a derived projection of run, task, criterion, and explicit lifecycle evidence; it is not an independently authoritative persisted attribute.
6. **Decomposition is live and audited.** Tasks added during execution are reflected in the report immediately and recorded with who added them, when, and in which run.
7. **Storage is Den control-plane.** Jobs/tasks orchestration records live in Den Postgres, not per-Bear SQLite. The Bear *uses* Den's job-management platform to organize its work, the way a person uses a project tracker; the platform is infrastructure the Bear plugs into, not part of the Bear's cognition. This narrows [ADR-0031](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), which keeps SQLite canonical for Bear *memory*. See the **execution invariant** below.
8. **Completion requires an accountable outcome and recorded output evidence where required, not a worker report alone.** Every terminal task settlement has a durable `outcome` journal entry. When the task's output contract requires evidence, Docket may settle it as complete only after recording the declared or observed durable run output and required validation evidence. Worker prose, including a claimed commit hash, is a summary and never sufficient completion evidence. This records provenance and a reviewable result; it does not certify that the output is technically correct. A task may explicitly require stronger surface observation, publication, artifact finalization, or a successful check.
9. **Live work runs are the sole execution evidence.** Focus is navigation state, not execution. A task is in progress only while it has a live work run; task definitions retain durable outcome states rather than a mutable `in_progress` state. A job projects running from live work runs and completion from required task outcomes. Historical job-run state cannot override those projections.

### Output evidence and settlement

Docket owns settlement policy and evidence persistence. A work surface owns the mechanics of materializing an output and may contribute observations about it. Jobs assign work surfaces through `job_work_surface_assignments`; an assignment's mutation policy has these semantics:

| Policy | Meaning |
| --- | --- |
| `required` | The job is expected to leave a durable mutation on this surface. This is the default when the singular creation shorthand is used. |
| `optional` | The surface may be mutated when warranted, but a no-change or report-only outcome is valid. Any mutation that does occur still requires the surface's applicable evidence and validation. |
| `forbidden` | The surface is context only. Mutable dispatch and mutation capabilities for that assignment are rejected. |

Mutation policy expresses expectation and authorization, not a second model-authored output taxonomy. The surface declares the typed output kinds it supports and may name a default. Docket resolves the applicable output contract from assigned surfaces, mutation policies, publication policy, task acceptance criteria, and observed authorized effects. The model does not separately classify every task and declare an output requirement.

A `required` assignment normally makes corresponding durable surface evidence mandatory before the job can settle successfully. It does not mean every individual task must independently mutate that surface: with per-job publication, intermediate tasks can settle with accountable outcomes while final job settlement proves the required surface output. An `optional` or `forbidden` assignment does not create a fake output obligation.

The initial closed vocabulary is intentionally small:

```rust
enum WorkOutputKind {
    GitCommit,
    GitPatch,
    FileBundle,
    Report,
}
```

A run records a typed primary output and validation evidence. The output record includes its available stable identity: for example, a declared Git commit OID or a finalized artifact digest. When validation names an identity, it must name the same output identity, so the record cannot accidentally attribute a check to another checkout or deliverable. A work surface may additionally observe existence, reachability, publication, or artifact finalization when that is feasible.

These records make the work reviewable and give Docket a credible basis to distinguish a result from a bare narrative. They are **not** a universal proof that the output is correct, complete, or fit for its intended purpose. Stronger requirements—such as a successful command, a finalized artifact, or publication/reachability observation—are explicit task or surface policy. A missing or failed *required* observation/check leaves the task blocked with a structured reason while preserving the candidate output and raw run evidence; an optional unavailable verifier does not turn ordinary work completion into a false failure.

On terminal execution, Docket persists the task's `outcome` journal entry and its required evidence before recomputing task and job projections from current durable facts. It must not infer completion from a narrative result alone. The contract may permit evidence to be optional for an explicitly coordination-shaped task, but that does not waive the terminal outcome: it must still state the disposition and what actually occurred. This allows legitimate decomposition, inspection, and no-change outcomes without incentivizing fake commits or artifacts.

For a Pair-owned task, terminal settlement is the user- or model-declared factual outcome. It always appends that canonical `outcome` entry and links it to the task; a job's `commit_policy` does not make finalized output a prerequisite. When a policy applies, the runtime evaluates delivery after settlement: `per_task` after a terminal task with relevant managed-surface changes, `per_job` at the job's delivery boundary, and `none` not at all. It must not create an empty commit for a no-change, investigation, or surface-free task. Delivery attempts and failures remain visible, retryable delivery evidence and do not reopen or invalidate the Pair task. A Work or release contract may separately hold a job's delivery projection pending required output, validation, publication, or artifact finalization.

### Docket journals and notebooks

Docket stores append-only semantic entries rather than treating raw run/session logs as the durable work record. A **task journal** is the task-centric accountable record: it establishes what a task accomplished, learned, chose, or encountered. A **job notebook** is the job-centric, selectively shared knowledge pool: it captures findings and decisions that may matter to other work within the same job. Neither is a mirror of automatic run logs.

The same entry model serves both views. An entry has a task-journal or job-notebook scope and may link to its originating task, run, evidence, and related tasks. A task-journal entry can be promoted to the job notebook without duplicating its content. Job-notebook publication does not promote knowledge into Bear memory; it remains Docket-scoped work context unless separately promoted through the normal memory process.

The initial vocabulary is deliberately small:

| Kind | Meaning |
| --- | --- |
| `outcome` | Accountable terminal statement of what happened to a task. |
| `finding` | A fact learned through inspection, experiment, or validation; it may be confirmed or provisional. |
| `decision` | A deliberate choice, with rationale and material consequences where applicable. |
| `obstacle` | Something that impeded progress. It does not itself assert that the task lifecycle is `blocked`. |
| `follow_up` | Linked work that must continue elsewhere in the same job; it does not imply transfer to another agent. |
| `milestone` | A meaningful nonterminal phase boundary; used sparingly. |
| `question` | A request for user direction; writable only in `pair` stance. |

`outcome` carries a typed disposition: `completed`, `no_change`, `delegated`, `blocked`, `failed`, or `cancelled`. A task's terminal lifecycle state must not contradict its outcome disposition: in particular, a `blocked` or `failed` outcome cannot be presented as task `done`. `delegated` settles a task only when its actual responsibility was coordination or decomposition; it does not settle a parent that remains responsible for the delegated substantive result.

Every terminal task state requires exactly one current `outcome` entry associated with that task. If work is reopened, the prior entry remains append-only history and a later terminal settlement appends a new outcome. Other entry kinds are intentional rather than routine: do not create `progress`, `status`, `comment`, or `checkpoint` entries merely to narrate activity.

An `outcome` always requires a concise semantic summary. Evidence references are mandatory when the task's output contract requires them, and otherwise optional. Valid evidence includes typed work outputs, artifacts, workspace revisions/diffs, command or check results, file references, child-task references, and other durable Docket records. Raw logs may be linked as supporting evidence but cannot alone establish that the task's intended result was achieved.

Workers receive bounded, relevant notebook context rather than the entire chronological entry stream: pinned job decisions, explicit follow-ups, and manually tagged entries relevant to the task are the initial selection mechanism. This is intentionally simple until observation justifies a richer retrieval system.
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
- commit_policy       commit_policy nullable   -- none | per_task | per_job; publication timing, not mutation expectation
- cancellation_requested_at timestamptz nullable -- explicit user lifecycle intent
- paused_at           timestamptz nullable     -- explicit user lifecycle intent
- archived_at         timestamptz nullable     -- explicit user lifecycle intent
- visibility          work_plan_visibility     -- reuse existing enum
- current_run_id      uuid nullable FK         -- convenience pointer; non-authoritative
- created_at, updated_at
```

#### `work_surfaces` and typed adapters — managed work targets and context

```text
work_surfaces
- id                  uuid PK
- kind                work_surface_kind   -- git_workspace | ...
- name, description
- created_by_user_id  uuid FK
- created_at, updated_at

git_work_surface_details
- work_surface_id     uuid PK/FK -> work_surfaces
- upstream_url
- default_ref
- sandbox_image
- encrypted_credentials
- adapter-specific configuration

job_work_surface_assignments
- job_id              uuid FK -> bear_jobs
- work_surface_id     uuid FK -> work_surfaces
- mutation_policy     mutation_policy -- required | optional | forbidden
- PRIMARY KEY (job_id, work_surface_id)
```

`work_surfaces` is the sole generic registry. Kind-specific fields live in strongly typed adapter tables rather than nullable generic columns or untyped configuration blobs. The assignment table is the sole canonical job-to-surface relationship. A singular `work_surface_id` accepted by a creation API is convenience syntax for one `required` assignment, not a second persisted relationship.

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
- session_anchor_id uuid nullable FK -> acp_sessions  -- exactly one task owner: Job or session
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
- state         run_state     -- dispatched | running | paused | stalled | completed | failed | cancelled
- started_at, finished_at
- outcome       jsonb nullable
```

Every job has **at least one** run. A oneshot job has exactly one. Recurrence (deferred) means the scheduler creates additional runs against the *same* job; no schema change is required.

A run becomes **`stalled`** when Den can no longer confirm continuation health or tool progress, without evidence that the requested work reached a terminal outcome. For example, a continuation that stops responding while waiting on a legitimately long-running tool call is stalled, not failed. Preserve its last progress/evidence and diagnostic. A stalled run is non-terminal for job intent: an operator may wait or resume it when supported, cancel it, or resolve it as failed. It must not be silently converted to `failed` merely to release an active-run slot.

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

#### `bear_docket_entries` — durable task journals and job notebooks (activity domain)

```text
bear_docket_entries
- id                uuid PK
- job_id            uuid FK -> bear_jobs
- task_id           uuid nullable FK -> bear_tasks
- run_id            uuid nullable FK -> bear_job_runs
- scope             docket_entry_scope -- task_journal | job_notebook
- kind              docket_entry_kind  -- outcome | finding | decision | obstacle
                                      --  | follow_up | milestone | question
- summary           text
- body              text nullable
- disposition       outcome_disposition nullable
- evidence_refs     jsonb not null default '[]'
- related_task_ids  jsonb not null default '[]'
- tags              jsonb not null default '[]'
- by_role           bear_agent_role
- by_agent_id       text nullable
- by_user_id        uuid nullable
- created_at        timestamptz
```

`summary` is required for every entry. `disposition` is required only for `outcome`; it is null for other kinds. `question` creation is authorized only in `pair` stance. `task_id` is required for task-journal entries and optional for job-notebook entries. An entry can be visible in both views by promotion/reference without copying text. Insertions emit the corresponding task/job audit event, but the entry row—not opaque event payload—is the canonical semantic record.

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

The user-facing **status report** and operational **job status** are projections, not separately mutable job fields. The status projection derives from persisted lifecycle intent, task/criterion state, and work-run evidence using one shared normalization path for APIs, conversation, operator UI, and logs:

1. `completed` when all required criteria are met or waived for the resolved run and no explicit cancellation intent applies.
2. `cancelled` when an operator has explicitly requested cancellation; a failed or stalled run alone never cancels a job.
3. `running` when a relevant run is dispatched, running, or paused with an active continuation.
4. `stalled` when an unresolved relevant run is stalled and no later run has resolved its task/criterion obligations.
5. `blocked` when required work is explicitly blocked (including a handoff) and no active or stalled run takes precedence.
6. `ready` when runnable work remains without an active, stalled, or blocked condition.
7. `draft` when the job has no runnable task tree or is otherwise incomplete for dispatch.

A terminal failed run is evidence, not a separate job status: it leaves the job `ready` when remediation remains runnable, otherwise `blocked` with its failure evidence. Implementations may materialize this projection for query performance, but it is a rebuildable cache and never an authority. The report remains an auditable rendering of `bear_job_events` plus the same normalized run/task/criterion evidence; it must not semantically disagree with the status projection.

Runtime checkpoints from ADR-0050 are tactical continuation-focus evaluations, not `bear_task_events`, `bear_job_events`, or automatic journal history. They may reference Docket run/task ids to assess what changed, what remains, and the smallest credible next action. A checkpoint may deliberately append or promote a journal/notebook entry only when it produces a durable `finding`, `decision`, `obstacle`, `follow_up`, `milestone`, or `outcome`. Checkpoint text is never progress, task output, or completion evidence by itself, and writing an entry does not itself earn continuation budget. Durable task progress, outcomes, criteria evaluation, and report-visible history still require the explicit Docket records defined here.

### Decomposition, template vs. run scope, and learning

Tasks created during planning by a human or `pair` default to `scope = template` — they are durable parts of the job definition. Tasks created by `work` mid-execution default to `scope = run` — they are run-local improvisation, not assumed wise. A run-scoped task may be **promoted** to `template` when observation justifies it (e.g. `work` recreates an equivalent child in most runs). Conversely, a chronically trivial or skipped template task may be **pruned**.

This promotion/pruning loop is the explicit mechanism for learning job structure over time without trusting the executing model's one-shot judgment. It is enabled by stable task identity plus the run dimension, both present from day one.

### Tool surface

Evolves the `den.work_plan.*` tools.

- **Job management (`pair` / `chat` / UI):** `den.job.create` (goal, criteria, initial task tree, `work_surface_id` shorthand or typed `work_surface_assignments`, commit policy), `den.job.get` (job + task tree + assignments + derived operational status + rendered report), `den.job.list`. Callers use the assignment list only for multiple surfaces or a non-default mutation policy. Lifecycle mutations record explicit pause, cancellation, archive, or supersession intent; they do not write a job status.
- **Task tree (all roles):** `den.task.create` (body, kind, parent_task_id, and one owner), `den.task.update` (status/result via run context), `den.task.list`. Pair never supplies a raw `session_anchor_id`: for a jobless task, Den derives the session owner from authenticated request context. A `job_id` instead creates a Job-owned task. Callers cannot create an unowned task or supply both owners.
- **Execution (`work`):** receives one Job dispatch from Den. The Job Run retains one sandbox/workspace/session while `work` advances the task tree (one `in_progress` task at a time), with Job goal and acceptance criteria injected; it may call `den.task.create` to add run-scoped children. Dispatch/watchdog paths record run progress and mark an unconfirmed continuation `stalled`, preserving evidence for an operator decision rather than fabricating a terminal outcome.

The `den.work_plan.request_handoff` tool is retired — job creation *is* the handoff from planning to durable work.

### Scope of v1

Built now, because identity and the run dimension are ruinous to retrofit:

- All tables above, including `bear_job_runs`, `bear_task_run_state`, `run_id` on events, and the `template | run` scope flag.
- Stable, canonical task identity owned by the job.
- One run per oneshot job.

Deferred — **only the scheduler behavior**, not the data model:

- Cron/trigger wiring, recurrence policy, and event-triggered runs. When added, recurrence is "the scheduler creates more runs against the same job" with no migration, no backfill, and full retroactive observability.

### Tests, commits, and documentation

These behaviors are **not** modeled as task-type enums. They belong to assigned work-surface instructions (per ADR-0006 anchors and the orientation recommendations), mutation policy, acceptance criteria, and publication policy. When an assigned surface resolves to a Git adapter, Den injects that surface's Git/test/doc conventions into applicable task dispatches. `required`, `optional`, and `forbidden` say whether mutation is expected, permitted, or prohibited; they do not themselves define a validation command. This keeps the task schema surface-agnostic while making code conventions mandatory context where they apply.

## Consequences

### Positive

- One persisted, hierarchical task concept usable with or without a job.
- Stable task identity + run-scoped state gives cross-run observability (chronic vs. anomalous effort) from the first recurring run, with no retrofit.
- Acceptance criteria provide enforceable hard gates owned by the job system, not the `review` agent.
- Live, audited decomposition with explicit authorship and a structure-learning loop.
- Recurrence becomes a behavioral feature, not a data-model migration.

### Negative / costs

- A task has no intrinsic status; every status read requires a run context (`current_run_id` resolves the common case).
- Operational job status is a derived query across lifecycle intent, runs, tasks, and criteria. It needs a single tested normalization path; any stored projection is disposable cache.
- `current_run_id` is denormalized convenience state that must be kept correct under run lifecycle transitions; it is not status authority.
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
