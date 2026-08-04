# Docket Turn Routing — Implementation Plan

## Status

Not started. Implements [ADR-0056 — Docket-driven turn routing](../decisions/adr-0056-docket-driven-turn-routing.md) (2026-07-20 revision).

**Scope stance:** autonomous execution **leads** (Phases 1–2); `pair` session visibility, task-tree mutation, and run steering are still **v1** (Phases 1 and 3). The ACP one-continuous-session illusion, armature-attached dispatch, and the ADR-0053 delegation broker are explicitly post-v1 (Phase 4).

Companion plans:

- [DOCKET_IMPLEMENTATION_PLAN.md](DOCKET_IMPLEMENTATION_PLAN.md) — the relational Docket substrate this plan routes over.
- [STANCE_SCOPED_DELEGATED_RUNS_IMPLEMENTATION_PLAN.md](STANCE_SCOPED_DELEGATED_RUNS_IMPLEMENTATION_PLAN.md) — the broker that eventually generalizes the `delegated` lane.
- [AGENT_LOOP_CONTROL_IMPLEMENTATION_PLAN.md](AGENT_LOOP_CONTROL_IMPLEMENTATION_PLAN.md) — continuation gate, focused Job, budgets; producers of `continuation` turn intents and escalation signals.
- [PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md](PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md) — shared turn/work status payload, task-list/Docket detail views, approval queue. Phase 3 here **consumes and extends** these surfaces; it must not add a parallel status shape or a second approval queue.
- [PHASE1_OPERATOR_CONSOLE_PLAN.md](PHASE1_OPERATOR_CONSOLE_PLAN.md) — operator pull-visibility of active/blocked/failed work; the console links to the surfaces this plan adds rather than duplicating them.

Depends on:

- [ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md) Docket schema (live: `bear_jobs`, `bear_tasks`, `bear_job_runs`, `bear_task_run_state`, `bear_task_events`).
- [ADR-0033](../decisions/adr-0033-model-tasks-layer.md) model-tasks layer (profile resolution lands in Phase 2 of this plan; only the `ModelRequestProfile` concept is required, not the full taxonomy).
- [ADR-0043](../decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md) — the router is core and protocol-neutral; replayable tool-activity transcripts are the substrate for the log-review APIs.
- [ADR-0045](../decisions/adr-0045-session-task-lists-and-docket-checkout.md) checkout/sync surface (live: `checkout_task_list`, `sync_task_list`, `TaskListSyncState`) — reused, not replaced, for `pair` tree mutation.

## Goal

A Docket job runs to completion unattended: the work-run dispatcher feeds turn intents to one router, each task lands in a policy-chosen conversation on a policy-chosen (cheap-by-default) model, children report back as strict rollups, and every placement is recorded. Meanwhile any number of `pair`/GUI sessions hold independent cursors over the same tree — browsing status, transcripts, and routing decisions live, and editing the tree — without ever contending with the run.

## Current-state anchors (2026-07-20)

What this plan builds on, by file:

- `den-docket/src/model.rs` — `DocketTaskRow` already has `parent_task_id`, `sibling_order`, `kind`, `difficulty`, `effort_hint`; **no** `routing_strategy`.
- `den-docket/src/work_runs.rs` — full work-run lifecycle (`Queued → … → Succeeded/Failed/TimedOut`), `attempt` counter, per-job serialization, `result_summary`/`result_refs`. This is the dispatcher the router adopts as its first client.
- `den-docket/src/service.rs` + `db.rs` — `execute_job` selects the next task and emits `focus_selected`; becomes a router call that records a RoutingDecision.
- `den-runtime/src/runtime/focus_context.rs` — `RuntimeFocusContext` resolution from `docket_execution_sessions`; the durable half becomes ConversationBinding (its `ponytail` note anticipates this).
- `den-bearwire/src/methods/run.rs` — `ResolvedRunModel`/`ResolvedRunModelSource` (conversation/stance-scoped); Phase 2 adds a task-profile source.
- `den-bearwire/src/methods/conversation.rs` / `session.rs` — `⌖` focus title projection, `clear_focus_for_mode_change`; become cursor projections in Phase 3.
- `den-service/src/client_sessions.rs` — one session ↔ one conversation binding; unchanged for chat clients in v1 (the illusion is deferred).

## Core invariants

1. **One placement mechanism.** Every routed turn — dispatch, continuation, rollup, or user — enters the router as a turn intent and leaves a persisted RoutingDecision. The dispatcher has no private placement path.
2. **Run state owns execution position.** At most one `in_progress` task per run; siblings serialize by `sibling_order`; no intra-job fan-out. Cursors are per-session viewports with zero authority and zero contention; cursor-on-terminal-task is a normal state.
3. **No model identifiers on task rows.** Tasks carry advisory descriptors only; profiles are resolved at dispatch by the model-tasks layer and recorded on the decision and run. Symbolic model refs only.
4. **Rollups are append-only, run-scoped task events.** Task rows hold no results (ADR-0034). Parents read latest-per-child. Under autonomy the rollup is the child's entire product: schema-validated, criteria-referenced.
5. **Router is core and protocol-neutral** (ADR-0043). No `acp_*` vocabulary; edges project.
6. **Stale state cannot force continuation.** Router resolution is `turn intent × task state × run state × conversation state × policy`; update the [state machine inventory](../architecture/den-state-machine-inventory.md) and add stale-cursor/stale-projection tests (ADR-0045 obligation).
7. **Placement never changes sequencing.** Scoping a transcript does not license concurrent execution.
8. **Execution surface is explicit, never inferred from prose.** From `pair`, the default is the session's attached work surface via its own armature; sandbox dispatch is a deliberate, user-visible choice (elicit when ambiguous); from `chat`/UI, dispatch defaults to sandbox. The resolved surface is recorded on every routing decision.
9. **Steering is run control plus tree mutation, under bear rights.** Jobs introduce no new ACL and no parallel command channel: `pause`/`resume`/`stop` plus audited Docket edits, picked up at turn/task boundaries; editing the in-progress task requires pause; `stop` is terminal for the run but the job stays resumable.
10. **Every attempt leaves a durable account.** Persist the turn/attempt envelope before model invocation, append replayable model and tool activity as it occurs, and terminalize it with a typed normalized outcome plus evidence. Provider silence, disconnects, watchdog expiry, and process loss must produce a synthetic terminal event if no model terminal event exists; partial activity remains readable after failure.
11. **The supervisor, not the model, owns disposition.** Model text or a model-requested stop may report blocked work and evidence, but cannot terminalize an autonomous task or run. The runtime validates completion against task criteria and chooses retry, profile escalation, handoff, pause, or typed terminal failure under explicit bounded policy.
12. **One failure truth, multiple projections.** Conversation history, task/run views, notifications, and forensic logs render the same normalized outcome/evidence record. Concise surfaces may summarize it, but must not independently infer a different cause or hide the last successful activity, the failing boundary, retry disposition, or recovery action.

## V1 delivery plan

Implement v1 as nine independently shippable increments. The first four increments are the **failure-prevention vertical slice**: they must land before broadening autonomous routing. This intentionally pulls the Phase 3 failure projection forward; waiting for all cursor/browsing work would leave autonomous runs unaccountable while the router is being built.

The estimates are elapsed engineer effort, not calendar promises. They assume the existing Docket, replay transcript, work-run, and shared turn/work status substrates can be extended rather than replaced. Total: **24–36 engineer-weeks**; the failure-prevention slice: **8–12 engineer-weeks**.

### Increment 0 — Contract and state inventory (2–3 engineer-weeks; cumulative 2–3)

1. Trace one successful, model-blocked, watchdog-expired, provider-disconnected, cancelled, and process-orphaned turn through work-run state, transcript persistence, task events, BearWire, and the run page.
2. Define one strongly typed normalized attempt/outcome/evidence contract. Include attempt identity, routing decision, lifecycle timestamps, outcome/cause/code, last successful activity, failing boundary, criteria evidence, profile, retry disposition, recovery action, and synthetic-terminal provenance.
3. Specify legal attempt, task-run, work-run, and job-run transitions, including which component owns each transition and the transaction boundary between them.
4. Update the state-machine inventory before implementation. Resolve whether the canonical replay stream can accept incremental events; do not introduce a second raw-log format.
5. Write executable DB/state-machine fixtures for the six traced scenarios. Initially they may fail, but they become the acceptance corpus for Increments 1–3.

**Exit gate:** schema/API review agrees on one canonical outcome shape and explicit transition ownership; no UI or worker independently invents failure causes.

### Increment 1 — Durable attempt ledger (2–3 engineer-weeks; cumulative 4–6)

1. Add routing-decision and durable turn-attempt migrations plus typed repository APIs.
2. Create the attempt envelope before provider/model invocation in the same durable dispatch path that identifies the work run and task.
3. Append assistant, provider, and tool activity to the canonical replay stream as it occurs; preserve ordering and idempotency keys.
4. Add compare-and-set/idempotent terminalization for normal completion and typed failure, including synthetic terminal events when no provider terminal event exists.
5. Wire existing work-run execution through the ledger without changing routing policy yet.

**Runnable check:** inject a watchdog timeout immediately after a recorded tool request and assert that the attempt, partial transcript, typed terminal outcome, and evidence all survive a process restart.

**Exit gate:** every old dispatch path either creates a durable attempt before invocation or is removed; no autonomous model invocation is unaccounted for.

### Increment 2 — Supervisor-owned disposition and recovery (2–3 engineer-weeks; cumulative 6–9)

1. Put autonomous completion behind a supervisor gate that validates task criteria. Model text and model-requested stop are evidence only; they cannot terminalize a task or run.
2. Implement explicit bounded policy outcomes: retry, profile escalation request, handoff, pause, typed terminal failure, or completion. Persist the selected disposition on the normalized outcome.
3. Ensure retries re-enter the router/dispatch boundary and create a new attempt; never overwrite the failed attempt.
4. Add an idempotent orphan sweeper using leases/heartbeats or an equivalent positive liveness rule. It must not close a genuinely live attempt.
5. Atomically record the failure rollup and task/run transition before retry or user attention routing.

**Runnable checks:** model-authored “cannot continue” leaves work under supervisor control; retry budget exhaustion creates one terminal failure; two sweepers racing close an orphan exactly once; a live attempt is not swept.

**Exit gate:** no model-originated field directly controls terminal autonomous state, and every non-completion has a durable bounded disposition.

### Increment 3 — Shared failure projection (2–3 engineer-weeks; cumulative 8–12)

1. Extend the existing shared turn/work status payload with the normalized outcome/evidence contract; do not create a run-page-only DTO.
2. Render the failed attempt in conversation sequence, including preserved partial activity and a concise cause/disposition summary.
3. Render the same canonical record on the run page: headline by default, narrative detail, and forensic raw-event drill-down.
4. Add recovery actions (retry/resume, handoff, or inspect) from the persisted disposition rather than client-side inference.
5. Add a semantic parity test that feeds one normalized failure to conversation and run projections and compares cause, code, disposition, evidence refs, and recovery action.

**Runnable check:** replay a `5bb511df`-style watchdog timeout after a tool request. Both surfaces show what succeeded last, where activity stopped, why Den terminalized it, whether it will retry/escalate, and what the user can do; forensic view shows the underlying events.

**Exit gate — failure-prevention slice complete:** failed turns cannot disappear, models cannot abandon autonomous work, and the run page explains the same failure recorded in conversation history and forensic logs.

### Increment 4 — Router foundation and dispatcher adoption (4–5 engineer-weeks; cumulative 12–17)

1. Land the remaining Phase 0 task-definition, binding, cursor, paused-state, and rollup-event migrations and neutral core types.
2. Implement the pure deterministic router and exhaustive placement-table tests.
3. Persist every routing decision and convert dispatch/continuation entry points to `TurnIntent`; delete private placement paths.
4. Add scoped conversation creation and binding reuse. Keep task sequencing unchanged and serialized.
5. Make the work-run dispatcher the first router client, retaining compatibility projections only where an existing client still consumes them.

**Exit gate:** every dispatched or continued turn has exactly one persisted routing decision and one pre-created attempt; existing successful dispatch behavior remains green.

### Increment 5 — Unattended completion, rollups, and run control (4–6 engineer-weeks; cumulative 16–23)

1. Implement structured latest-per-child rollups with criteria evidence; parent prompts consume rollups, never child transcripts.
2. Implement next-actionable-task selection, criteria-gated job completion, and blocked-task handoff.
3. Add durable pause/resume/stop and boundary pickup for tree edits; reject edits to an in-progress task unless paused.
4. Add attention routing through the existing handoff and approval surfaces, with deep links and chat answers that resume work.
5. Exercise failure retry/escalation through the same completion loop rather than a side channel.

**Runnable check:** a mixed inline/scoped three-level job runs unattended to completion; pause/add sibling/resume dispatches the new task; blocking notifies the user and a chat answer resumes the run.

**Exit gate:** Phase 1 acceptance is green, including all Increment 1–3 failure scenarios.

### Increment 6 — Model economy (2–3 engineer-weeks; cumulative 18–26)

1. Implement the minimal symbolic `ModelRequestProfile` resolver adjacent to the model registry.
2. Resolve profiles at dispatch from task descriptors, stance, model library, attempt, and prior typed outcome; never persist model identifiers on task rows.
3. Add the bounded escalation ladder and record profile provenance, cost, and latency on decisions/runs.
4. Verify that supervisor escalation selects the next allowed profile and cannot loop beyond policy limits.

**Exit gate:** Phase 2 acceptance is green; tier-1 failure produces a recorded tier-2 decision and bounded terminal behavior at the top tier.

### Increment 7 — Cursor, browsing, and live observation (3–5 engineer-weeks; cumulative 21–31)

1. Implement cursor lifecycle and replace focus-title behavior with compatibility-safe cursor projections.
2. Add task-tree, task-conversation, paged transcript, routing-decision, and job-event APIs over existing canonical records.
3. Add headline/narrative/forensic server-side projections; clients do not filter raw events to derive meaning.
4. Add stale cursor behavior and tests for completed, blocked, and deleted tasks.
5. Show live autonomous position without granting cursors execution authority.

**Exit gate:** a `pair` session can inspect completed work and watch a live run without changing sequencing; golden-trace compatibility remains green.

### Increment 8 — Steering, mutation, and v1 hardening (3–5 engineer-weeks; cumulative 24–36)

1. Route interactive `user` and `continuation` intents through the same router.
2. Reuse checkout/sync for tree mutation and add audited routing-strategy edits.
3. Expose run controls and immediate pending acknowledgements on `pair`, chat, and UI surfaces.
4. Persist the elicited execution-surface choice and provide one-action recovery from failed/stopped runs.
5. Run restart, race, stale-state, authorization, migration, and compatibility suites; update operator docs and the state-machine inventory.
6. Remove superseded focus/private-placement code and vestigial data shapes before declaring v1 complete.

**Exit gate — v1 complete:** all Phase 0–3 acceptance criteria in this document pass on the same build, migrations work from the current production schema snapshot, and no post-v1 Phase 4 capability is required for the advertised behavior.

### Sequencing and parallel work

- Increments 0–3 are a single dependency chain. UI work in Increment 3 may begin once the outcome contract is fixed, but it cannot ship against fabricated client-side outcomes.
- After Increment 1, one stream may implement supervisor/recovery while another builds shared projections, using the same fixtures; integration remains gated on Increment 2 dispositions.
- After Increment 4 fixes router and binding contracts, rollup/run-control work (Increment 5) and profile resolution (Increment 6) can proceed in parallel.
- Cursor APIs in Increment 7 may begin after the Phase 0 cursor schema lands, but interactive routing and mutation wait for Increment 4 so no second placement mechanism appears.
- Keep changes reviewable: prefer one migration/type PR, one repository/state-machine PR, and one vertical behavior PR per increment. Do not hold the failure slice in a long-lived branch.

### V1 definition of done

- All model invocations have a durable pre-invocation attempt and incremental replay trail.
- All terminal states are typed, evidence-bearing, idempotent, and supervisor-owned.
- Conversation, run, notification, and forensic surfaces consume one canonical outcome without semantic drift.
- Every turn has one routing decision; every retry is a new attempt through the same router.
- A representative nested job completes unattended with bounded escalation, rollups, controls, attention routing, and restart recovery.
- Multiple sessions can browse and steer through cursors without controlling execution position or violating sequencing.
- DB migrations, SQLx offline metadata, unit/integration/golden-trace tests, clippy, and state-machine documentation are green.
- Superseded pre-release paths and shapes are removed rather than deprecated.

## Phase 0 — Schema and the focus split

Foundations only; no behavior change to dispatch yet.

1. **Migrations (Den Postgres):**
   - `bear_tasks`: add `routing_strategy` enum (`inline | scoped | delegated | auto`, default `auto`), nullable `expected_context_size`, nullable `result_rollup_policy`. Extend `DocketTaskDefinitionPatch` and task create/update tool paths.
   - `docket_conversation_bindings`: `task_id`, `preferred_conversation_id`, `created_at`, `updated_at`; plus per-run refs (`run_id → conversation_id`) — either columns on run records or a small history table. Backfill from `docket_execution_sessions`.
   - `docket_cursors`: `client_session_id` PK, `job_id`, `task_id?`, `updated_at`. Volatile semantics; safe to truncate.
   - `docket_routing_decisions`: decision object per ADR-0056 (turn_source, target, conversation strategy/id, routing_strategy, execution surface, resolved_profile?, attempt, cursor_before/after, reason, timestamps, `job_id`/`run_id` FKs for listing).
   - Durable turn-attempt records keyed to routing decision/work run, created before invocation, with lifecycle timestamps, typed normalized outcome (`completed | blocked | failed | timed_out | cancelled`), cause/code, retry disposition, and evidence refs. Reuse the canonical replay/tool-activity event stream for incremental content; do not add a second raw-log format.
   - `WorkRunState`: add a `Paused` variant (job runs already have one) so `pause` is durable run state; define legal transitions (`Running ↔ Paused`, `Paused → Cancelled`).
   - `bear_task_events`: new event type `rollup_recorded` (payload: summary, criteria evidence refs, `run_id`).
2. **Types:** `TurnIntent` (`source: user | continuation | dispatch | rollup`), `RoutingDecision`, `ConversationBinding`, `DocketCursor` in `den-docket`/`den-core` (neutral names, no wire types).
3. **State inventory** entry for cursor/binding/decision axes; tests that a cursor pointing at a completed/deleted task cannot initiate execution.

Acceptance: migrations apply cleanly; existing dispatch and focus behavior unchanged; `cargo sqlx prepare` offline data updated; clippy gate green.

## Phase 1 — Router core and autonomous job completion (leading)

1. **Router in core** (`den-runtime`/`den-docket` seam): `route_turn(TurnIntent) -> RoutingDecision`, with the deterministic placement policy from ADR-0056 (bound conversation → reuse; trivial sibling → inline; `scoped`/`delegated` → scoped conversation; non-trivial child → scoped; else inline). Persist every decision.
2. **Dispatcher as first client.** Rework `execute_job` next-task selection and the work-run worker so each dispatch is a `dispatch` turn intent through the router. `focus_selected` events remain but reference the decision id.
3. **Scoped conversation creation.** `create_scoped_conversation(task_id, parent_conversation_id)` in core conversation persistence; bind via `docket_conversation_bindings`. Work-run checkout consumes the routed conversation instead of implicitly minting one.
4. **Job-completion loop.** After a task's work run reaches terminal state: emit a `rollup` intent → rollup event recorded → router selects the next actionable task (`sibling_order`, skipping terminal/blocked) → next work run enqueued. Job completes when no actionable tasks remain and ADR-0034 criteria gates pass; blocked tasks surface per ADR-0026 handoff.
5. **Rollup generation.** Structured, schema-validated model task (cheap tier): summary + criteria evidence, sourced from `result_summary`/`result_refs` and the child transcript. Parent dispatch prompts include latest-per-child rollups, never child transcripts.
6. **Failure and abandonment path.** Incrementally persist replayable assistant/tool activity against the pre-created attempt. A model may report `blocked` with evidence, but cannot declare an autonomous task/run finished or abandoned; the supervisor checks criteria and applies bounded retry/profile escalation, handoff, pause, or typed terminal failure. `Failed`/`TimedOut` — including watchdog expiry after a tool request — atomically terminalizes the attempt with a normalized outcome/evidence record and records the failure rollup before retry re-enters the router. If the provider/runtime emitted no terminal event, recovery writes a synthetic one without discarding partial activity; an idempotent sweeper closes attempts left open by process loss.
7. **Run control and boundary pickup.** `pauseRun`/`resumeRun`/`stopRun` over the work-run/job-run lifecycle: `pause` is durable run state (the pausing client may disconnect); `stop` is terminal for the run, job resumable via a new run. Tree edits made mid-run land in Docket immediately and are picked up at the next turn/task boundary; editing the in-progress task is rejected unless the run is paused. Steering authorization = bear rights.
8. **Attention routing (push).** Autonomy's failure mode is silent stalling. The durable side already has owners — ADR-0026 handoff records and the approval queue in [PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md](PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md) — so the new work is only the push hop: deliver blocked / decision-needed / completed / failed events to the user's active surface (chat message via the Bear, client notification, console badge) with a deep link to the job. Answering from chat — updating the blocked `decision` task or approving — resumes the run without opening a console.

Acceptance: a seeded 3-level job (parent + investigation child + execution children, mixed `inline`/`scoped`) runs unattended to `completed`; every turn has a RoutingDecision row and a durable attempt record; the parent's final context contains rollups only; pause → add a sibling task → resume dispatches the new sibling; a run that blocks mid-job produces a user-visible notification whose answer (from chat) resumes it unattended; a model-authored “cannot continue” does not terminalize work; a watchdog timeout after recorded assistant/tool activity preserves that activity, adds a synthetic typed terminal event, and follows bounded retry/escalation policy; restart recovery closes an orphaned attempt exactly once; DB tests cover next-task selection, binding reuse, rollup latest-per-child reads, in-progress-edit rejection, and idempotent attempt terminalization.

## Phase 2 — Model economy

1. **Profile resolution.** Minimal ADR-0033 slice: `resolve_execution_profile(task descriptors × stance × Bear model library × registry) -> ModelRequestProfile` (symbolic `model_ref`, effort, limits). Home: a new `model_tasks` module adjacent to the registry — not in Docket, not in feature code.
2. **Dispatch consumes profiles.** Extend `ResolvedRunModelSource` with a `TaskProfile` variant; work-run dispatch resolves via the profile path, conversation/stance resolution remains the fallback and the `pair` default.
3. **Escalation ladder.** Cheap-by-default: first attempt uses the lowest tier `difficulty` allows; attempt N+1 (after typed failure: task-gate rejection loop, ko threshold, `Failed`/`TimedOut`) resolves one tier higher, bounded by the model library. Record profile + attempt on the run and the decision.
4. **Observability.** Cost/latency metadata keyed by task class and profile (ADR-0033); decision records make spend attributable per task. Feed ADR-0034 chronic-vs-anomalous-effort views; leave ADR-0051 assessment-driven tuning as a hook, not a deliverable.

Acceptance: a task that fails at tier 1 re-dispatches at tier 2 with both decisions recorded; no model name appears in any `bear_tasks` row; profile provenance visible via `listRoutingDecisions`.

## Phase 3 — `pair` visibility and task-tree mutation (completes v1)

1. **Cursor lifecycle.** BearWire methods for `getCursor`/`setCursor`/`listCursors`; cursor per client session; many per tree. Replace the `⌖` focus-title path and `clear_focus_for_mode_change` with cursor projections/behavior (migration step 2 of ADR-0056). `conversation_has_active_focus` derives from bindings + run state.
2. **Browsing APIs.** `listTaskTree(job_id)` (full hierarchy — the flat `den.acp_plan_projection.v1` stays for legacy clients), `getTaskConversations(task_id)` (preferred + per-run history), `getConversationTranscript` paged over replay-state transcripts (ADR-0043/0048), `listRoutingDecisions`, `streamJobEvents(job_id)` projecting `bear_job_events` + decisions + rollups over BearWire. These extend the shared turn/work status payload from [PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md](PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md) — no parallel status shape.
3. **Interactive routing and steering.** `user` and `continuation` intents from `pair` sessions enter the same router (direct placement verbs only — no negotiation vocabulary, no `previewTurnRoute`). Interactive forks — reopen a completed task, choose an execution surface — go through the single elicitation tool. From `pair`, execution defaults to the session's attached work surface via its own armature; sandbox dispatch is an explicit, elicited choice. Watching a live run (transcript + event stream) and steering it (run control + tree mutation) work from `pair`, `chat`, and UI alike, authorized by bear rights.
4. **Tree mutation from `pair`.** Reuse ADR-0045 checkout/sync for item edits; expose `routing_strategy` as an editable definition field (`setRoutingStrategy` → task-update path, audited via `task_updated` events). Live decomposition (`child_added`) already flows through Docket events into the stream.
5. **Staleness UX.** Acting on a cursor at a completed/blocked/deleted task degrades per the ADR table (reopen/inspect/navigate), with tests.
6. **Projection altitudes.** Transcript and event-stream APIs take a detail level — `headline` (task transitions + rollups), `narrative` (assistant text + key results), `forensic` (full tool traces) — mapping ADR-0053 visibility metadata to rendering tiers. The chat "running log" projection defaults to narrative; console/tree views default to headline with drill-down. Clients never re-derive altitude by filtering raw events themselves.
7. **Steering feedback.** Run-control and mid-run tree edits acknowledge immediately with a pending state on the event stream ("pausing at next boundary", "edit queued — applies at next task"), so boundary pickup never reads as ignored input. Model-facing run-control tools carry guardrails: ambiguous utterances ("hold on") elicit rather than `stopRun`; `pause` is the safe default mapping.
8. **Recovery affordances.** A failed run (including the armature-disconnect timeout path) and a stopped run both offer one-action resume: a new run seeded from task states and the failure rollup. The failure notification carries the resume action — coming back to a run that failed only because a laptop closed must cost one tap, not a re-dispatch ritual.
9. **Elicitation memory.** The execution-surface choice is recorded on the job at first dispatch (alongside `commit_policy`) and reused on re-dispatch; editable later, never re-asked per dispatch.
10. **Failure explanation.** The shared turn/work status payload includes the normalized outcome and evidence: plain-language cause, stable code, last successful activity, failing boundary, attempt/profile, retry or escalation disposition, and available recovery action. Conversation history shows the failed attempt in sequence; the run page shows the same headline with narrative and forensic drill-down. Raw events remain available but are not the UI's only explanation.

Acceptance: while a seeded job executes autonomously, a `pair` session can (a) hold a cursor on a done child and read its transcript and rollup, (b) watch live position from run state, (c) add a child task and flip a sibling to `scoped`, all without perturbing the run; stale-cursor actions degrade safely; pause/edit commands show a pending acknowledgment before their boundary; a timed-out armature run resumes in one action; the surface question is asked at most once per job; for the same injected watchdog failure, conversation history and the run page both show the preserved partial attempt and semantically identical cause/disposition/evidence, while forensic view exposes the underlying events; state inventory updated.

## Phase 4 — Post-v1 (deferred, in likely order)

1. **Armature-attached dispatch**: unattended `work` runs executing on a user's online armature session with a matching work surface — no new stance (`work` × background × armature; even sandboxed work executes through a Den-provisioned armature). Semantics fixed by ADR-0056: acts on the live working tree with a non-blocking dirty-tree warning at dispatch; the armature's local permission profile stays authoritative for commands (an action that would interactively prompt blocks the run) while job `commit_policy`/autonomy governs writes; armature disconnect auto-pauses the run, and a bounded timeout converts the pause to a failed run with a failure rollup.
2. **Delegation broker integration** (ADR-0053): `delegated` routes through the broker; work-run lane becomes one brokered backend; capability minting decoupled from placement.
3. **ACP illusion projection**: multi-conversation routed sessions behind one apparent ACP session; adapter-only work over the Phase 1 router.
4. **`auto` promotion heuristics**: promote inline→scoped from observed complexity (checkpoint/ko/context-pressure signals), replacing the trivial rule table.
5. **Run-record unification**: fold turn runs / job runs / work runs toward one shape (ADR-0056 open question 4).
6. **Intra-job fan-out**: only after ADR-0034 revisits its deferral; the router and bindings are already shaped for it.

## Testing and delivery notes

- DB-backed tests for router policy, bindings, rollup reads, and cursor staleness (Docket integration-test conventions; keep `sqlx` offline data current).
- Router unit tests are pure: `TurnIntent × fixture state -> RoutingDecision` with no I/O, so the placement table is exhaustively testable.
- Golden-trace parity (ADR-0043) must stay green through Phase 3's focus-title/cursor swap; no wire-visible regression for existing clients.
- Den is pre-release: completed slices activate by default once tested; no long observe-only windows. Phases land in order — each is independently shippable, and Phase 1 alone already delivers unattended job completion.
