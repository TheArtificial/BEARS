# Agent Loop Control Implementation Plan

## Status

Planned. Implements [ADR-0050 — Agent Loop Control, Adaptive Budgets, and Runtime Checkpoints](../decisions/adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md), and depends on the task-list/Docket boundaries in [ADR-0045](../decisions/adr-0045-session-task-lists-and-docket-checkout.md) and [ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md).

> **Companion plan (2026-07-06):** [AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md](AGENT_LOOP_CONTROL_GROUNDING_AND_TUNING_PLAN.md) delivers the ADR-0050 amendment — surface-declared grounding probes (§7c), context/token budget as a loop dimension (§11), and an advisory-first rollout with a persisted replayable ledger and offline tuning harness. That plan reframes this plan's Phase 11 rollout as advisory-first and adds grounding as the arbiter for the "meaningful mutation" judgment in Phases 3–5. Land the companion plan's ledger + advisory foundation early; it is the measurement loop the rest of this plan is tuned against.

## Goal

Give Den a typed **agent loop control** layer that governs tool-using turns across model calls and tool results: when to continue, checkpoint, retry, warn, stop, escalate thinking effort, or require task-state reconciliation.

The implementation must make Bear runs more efficient and auditable without letting runtime checkpoint prose become task state, memory, or ordinary conversation history.

## Scope

In scope:

- progressive agent loop control levels (`light`, `standard`, `careful`, `strict`),
- governance as the continuation-pressure input (`interactive`, `grace`, `autonomous_continuation`, `observational`, `frozen`),
- focused Job as the Docket objective input for long-running continuation,
- model default control levels,
- Bear-level and stance-level overrides mirroring model selection,
- task/run escalation for risk, difficulty, or governance,
- profile-owned budgets, ko/failure thresholds, checkpoint triggers, and task-gate behavior,
- optional checkpoint-turn thinking-level escalation,
- structured checkpoint artifacts/events,
- audit-oriented checkpoint retention for `work` runs,
- explicit projection/replay/history boundaries.

Out of scope:

- replacing Docket task/job state with checkpoint artifacts,
- using checkpoint prose to satisfy task gates,
- free-form transcript parsing to detect loops,
- hardcoded per-model runtime behavior,
- verbose chain-of-thought capture,
- provider reasoning-stream persistence as conversation history.

## Core invariants

1. **Runtime controls continuation; task tools control task state.** Checkpoints may identify that task state should change, but only `update_task_list`, `sync_task_list`, `checkout_task_list`, `request_task_list_handoff`, or Docket service APIs may mutate session task-list/Docket state.
2. **Checkpoints are structured artifacts, not informal prose.** A checkpoint report should arrive as a runtime-owned `checkpoint` tool call whose arguments are parsed into typed fields. Assistant prose/JSON fallback is degraded and never the primary control path.
3. **Checkpoint artifacts can be auditable without becoming Docket events.** `work` runs may retain checkpoint artifacts as run audit evidence, but Docket `bear_task_events`/`bear_job_events` remain the report-visible source of task progress, blockers, completion, and criteria evaluation.
4. **Thinking effort is a quality knob, not a safety boundary.** Budget, ko, task-gate, and stop enforcement remain runtime-authoritative even if a provider ignores thinking-level metadata.
5. **Runtime code consumes resolved typed profiles.** Model names and provider quirks belong in registry/configuration; the runtime must not scatter model-name `match` arms.

## Conceptual model

```mermaid
flowchart TD
    A[Model registry default] --> R[Resolve agent loop control]
    B[Bear override] --> R
    C[Stance override] --> R
    D[Task/run escalation] --> R

    R --> P[Resolved control profile]
    P --> L[Loop budget/ko/failure state]
    L --> T{Trigger?}

    T -- continue --> M[Next model/tool step]
    T -- checkpoint --> Ck[Checkpoint request]
    Ck --> CT[Optional checkpoint thinking effort]
    Ck --> CR[Checkpoint tool report]

    CR --> A1[Run audit artifact]
    CR --> D1{Task state change needed?}
    D1 -- yes --> Tool[Task-management tool required]
    D1 -- no --> M
    Tool --> State[Session task-list/Docket state]
```

## Phase 0 — Baseline inventory

**Goal:** identify current seams before changing behavior.

| Task | Done when |
| --- | --- |
| Inventory budget state | Existing `TurnBudgetPolicy`/ledger structures and enforcement points are documented. |
| Inventory model selection | The current model resolution precedence for system/Bear/stance/conversation is documented. |
| Inventory request metadata | The Bifrost/provider request path that can carry thinking/reasoning effort metadata is identified. |
| Inventory task gates | Current session task-list continuation gates and task-gate rejection handling are documented. |
| Inventory persistence | Conversation history, hidden model-visible records, BearWire events, Docket events, and run telemetry stores are distinguished. |

**Exit gate:** a short implementation note identifies exact files/modules for type definitions, resolver, request metadata, loop state, checkpoint artifacts, and projections.

## Phase 1 — Agent loop control types and profiles

**Goal:** introduce typed policy without changing runtime behavior.

Suggested types:

```rust
pub enum AgentLoopControlLevel {
    Light,
    Standard,
    Careful,
    Strict,
}

pub enum AgentLoopControlSource {
    ModelDefault,
    BearOverride,
    StanceOverride,
    TaskEscalation,
    SystemDefault,
}

pub struct ResolvedAgentLoopControl {
    pub level: AgentLoopControlLevel,
    pub source: AgentLoopControlSource,
    pub profile: AgentLoopControlProfile,
}

pub struct AgentLoopControlProfile {
    pub budget: TurnBudgetPolicy,
    pub ko: KoPolicy,
    pub checkpoints: CheckpointPolicy,
    pub task_gate: TaskGatePolicy,
    pub thinking: CheckpointThinkingPolicy,
}
```

| Task | Done when |
| --- | --- |
| Add level enum | Control levels are serialized with stable snake_case names. |
| Add policy structs | Budget, ko, checkpoint, task-gate, and thinking policy have typed structs. |
| Add default profiles | `light`, `standard`, `careful`, and `strict` resolve to deterministic profiles. |
| Add monotonic ordering | Runtime can compare/escalate levels without string matching. |
| Add unit tests | Default profile thresholds and ordering are covered. |

**Exit gate:** profiles are typed and testable, but not yet enforced.

## Phase 2 — Model defaults and Bear/stance overrides

**Goal:** resolve control level the same way model selection is resolved.

Resolution order:

1. model registry default,
2. Bear-level override,
3. stance-level override,
4. task/run escalation.

Task/run escalation may raise intensity, but should not silently downgrade operator-configured Bear/stance policy.

| Task | Done when |
| --- | --- |
| Extend model metadata | Model registry can expose default `agent_loop_control_level`. |
| Add Bear override storage | Bear config can set a default loop control level. |
| Add stance override storage | Stance/profile config can override for `pair`, `chat`, `work`, etc. |
| Implement resolver | Runtime receives `ResolvedAgentLoopControl`, including source and profile. |
| Add diagnostics | Run logs/progress include level, source, and profile summary. |
| Add tests | Model default, Bear override, stance override, unknown model fallback, and escalation precedence are covered. |

**Exit gate:** runs can resolve and report a control profile without behavior changes.

## Phase 2a — Governance and focused Job inputs

**Goal:** make the loop controller consume explicit supervision and objective inputs before enforcement uses them.

Suggested shape:

```rust
pub enum Governance {
    Interactive,
    Grace,
    AutonomousContinuation,
    Observational,
    Frozen,
}

pub struct LoopControlContext {
    pub governance: Governance,
    pub focused_job_id: Option<JobId>,
}
```

Do **not** introduce a generic `FocusTarget` enum yet. The only supported durable focus object is a Docket Job.

Focused Job ownership is conversation-scoped. The durable source of truth is the conversation's current focused Job; live sessions project that state into their UI, and each run receives a snapshot in `LoopControlContext`. Work runs should also record the focused Job they were launched under for auditability, but they do not own focus. Governance is resolved per run and is not the same durable state as the focused Job.

Minimal first shape:

```rust
pub struct ConversationFocusState {
    pub focused_job_id: Option<JobId>,
}
```

If debugging needs it, add timestamps/source later; do not build a broader focus subsystem up front.

| Task | Done when |
| --- | --- |
| Rename docs/projections toward governance | New runtime prose says **governance** for the supervision axis, while preserving existing `Mode` code/API compatibility where already decided by ADR-0039. |
| Persist conversation focus | The conversation can durably store `focused_job_id: Option<JobId>` as the source of truth for focus across turns and reconnects. |
| Project focus to sessions | Live sessions derive their displayed Focused state and title from conversation focus. |
| Snapshot focus into runs | Each run receives governance and the current focused Job as `LoopControlContext`; work runs persist the launched-under Job for audit. |
| Add focused Job to run context | Runtime context can carry `focused_job_id: Option<JobId>` independently from governance. |
| Enforce `work` designation | A `work` run must have a focused Job before model-driving continuation begins; missing Job is a hard rejection before model invocation. |
| Allow explicit `pair` focus | `pair` can designate a focused Job through Bear conversation or a client command without changing trust stance. |
| Derive task focus | Given governance + focused Job + Docket/task-list state, runtime derives the next logical incomplete/unblocked task as ephemeral task focus. |
| Add diagnostics | Run diagnostics include governance, focused Job id if any, and derived task-focus refs without leaking hidden task-gate internals. |
| Add tests | Normal `pair` has no focused Job; focused `pair` keeps `pair` trust stance; `work` without focused Job is rejected; `work` with focused Job derives a next task. |

**Exit gate:** loop control has explicit governance and focused-Job inputs, but no generic focus-target abstraction.

## Phase 2b — Client projection for Focus

**Goal:** let UI-providing clients expose focused-Job state without pretending it is a user-selectable approval preset.

UI-providing clients such as ACP should project focused-Job behavior as a special permissions/presentation state. The command and feature are called **Focus**; the visible permission/mode label while active is **Focused**. Focused is **not UI-selectable** as an ordinary permissions mode. It can be entered through Bear conversation or slash commands after Den designates a Job.

Command shape:

```text
/focus [job]
```

`[job]` resolution:

- exact Job ID: focus that Job and begin execution immediately;
- other text: search existing Jobs; if exactly one high-confidence match exists, focus it and begin execution immediately; otherwise show possible matches and ask the user to select one or begin defining a new Job;
- empty: show recent Jobs and ask the user to select one or begin defining a new Job.

Selection is mediated through one model-visible elicitation tool. The model should not know whether the client rendered a native picker or sent numbered text options. Clients without elicitation UI present numbered options and ask the user to reply with a number; the runtime normalizes the result as the same elicitation response.

When focused:

- the client permission/mode label is **Focused**;
- the session title reflects focus;
- the conversation title is updated to `[Job name] - [current task]` with simple fallbacks such as `[Job name]`, `[Job name] - selecting next task`, `[Job name] - blocked`, or `[Job name] - complete`;
- changing the client mode away from Focused clears conversation focus and stops focused continuation.

| Task | Done when |
| --- | --- |
| Define Focus projection | BearWire/ACP can project **Focused** as a special non-selectable state tied to conversation `focused_job_id`. |
| Keep Den authoritative | Clients display Focus and may provide commands to request it, but Den validates/designates/clears the focused Job. |
| Add slash-command hook | ACP/client command plumbing can handle `/focus [job]` with exact-id, search, ambiguous-selection, empty-recent, and define-new paths. |
| Add elicitation path | Job selection uses one model-visible elicitation tool; client adapters choose native UI or numbered text fallback without exposing that choice to the model. |
| Clear on mode change | If a client changes mode away from **Focused**, Den clears the conversation focused Job. |
| Update focused titles | Focused conversations/sessions update title as `[Job name] - [current task]` with blocked/complete/selection fallbacks. |
| Prevent permission laundering | Focus does not grant tools, approvals, memory access, or outbound auth beyond the effective policy from trust stance + governance + armature. |
| Add projection tests | Focused appears when a focused Job is active, cannot be selected directly from normal permission UI, clears when Den clears focus, and clears when the client mode changes. |

**Exit gate:** ACP-style clients can show/request Focus, but cannot select it as an ordinary permissions mode or use it to alter trust boundaries.

## Phase 3 — Budget/ko/failure integration

**Goal:** initialize loop budgets from the resolved control profile.

| Task | Done when |
| --- | --- |
| Wire profile into turn state | Turn state owns the resolved profile and budget ledger. |
| Replace hardcoded thresholds | Existing budgets/ko/failure limits come from the resolved profile. |
| Preserve global fuses | Wall-clock, total tool calls, repeated failures, and emergency hard steps remain turn-global. |
| Preserve verification windows | Read/search replenishment after meaningful mutation remains small and verification-oriented. |
| Add tests | Budget initialization, ko cutoff, failure cutoff, and verification replenishment use resolved profile values. |

**Exit gate:** existing budget behavior is policy-driven, with no checkpoint enforcement yet.

## Phase 4 — Checkpoint trigger state

**Goal:** track loop-health signals needed for runtime checkpoints.

Suggested state:

```rust
pub struct CheckpointState {
    pub read_search_since_mutation: u32,
    pub consecutive_failures: u32,
    pub same_signature_repeat_count: u32,
    pub last_checkpoint_at_step: Option<u32>,
    pub pending_checkpoint: Option<CheckpointRequest>,
}

pub enum CheckpointReason {
    OverExploration,
    ConsecutiveFailure,
    SameSignatureNearKo,
    TaskGateRejection,
    LowBudget,
    PreRiskMutation,
}
```

| Task | Done when |
| --- | --- |
| Classify tool calls | Read/search, mutative, execute, destructive, and no-op/failure outcomes are typed. |
| Track exploration since mutation | Read/search counter increments and resets only on meaningful mutation. |
| Track failure and repeat signals | Consecutive failures and repeated signatures feed checkpoint state. |
| Track task-gate rejection | Gate rejection state can request a checkpoint before stronger stop behavior. |
| Add observe-only diagnostics | Runtime can emit `runtime_checkpoint_would_trigger` without forcing behavior. |
| Add tests | Each trigger reason fires under profile-specific thresholds. |

**Exit gate:** checkpoint triggers are observable and tested but may still run in observe-only mode.

## Phase 5 — Structured checkpoint request and checkpoint tool protocol

**Goal:** implement Option B as a typed runtime-owned `checkpoint` tool call rather than unstructured assistant prose or assistant text JSON.

A checkpoint request should include enough context to make the response auditable:

```rust
pub struct RuntimeCheckpointRequest {
    pub checkpoint_id: CheckpointId,
    pub run_id: String,
    pub reason: CheckpointReason,
    pub control_level: AgentLoopControlLevel,
    pub active_objective: Option<String>,
    pub task_context: Option<CheckpointTaskContext>,
    pub evidence_refs: Vec<CheckpointEvidenceRef>,
    pub required_fields: Vec<CheckpointField>,
}
```

The model-facing response should be a `checkpoint(...)` tool call. Suggested tool argument schema:

```rust
pub struct RuntimeCheckpointResponse {
    pub checkpoint_id: CheckpointId,
    pub active_objective: String,
    pub summary: Option<String>, // short prose synthesis for audit
    pub more_exploration_justified: bool,
    pub next_action: CheckpointNextAction,
    pub task_state_change_needed: Option<TaskStateChangeIntent>,
    pub evidence_refs: Vec<CheckpointEvidenceRef>,
    pub confidence: Option<CheckpointConfidence>,
}
```

Do not force learned facts and uncertainty into required JSON arrays. Keep model reasoning/prose separate; structure only the decision fields and a short audit summary.

`CheckpointNextAction` should be a typed enum, for example:

- `call_tool`,
- `edit`,
- `validate`,
- `update_task_list`,
- `sync_task_list`,
- `request_handoff`,
- `final_if_gate_allows`,
- `stop_blocked`.

`TaskStateChangeIntent` is **intent only**. It does not mutate task state.

| Task | Done when |
| --- | --- |
| Define request/report DTOs | `RuntimeCheckpointRequest` and `RuntimeCheckpointResponse` are typed and serializable; the response DTO is the `checkpoint` tool argument shape. |
| Add checkpoint ids | Every request/report pair has a stable id scoped to run/turn. |
| Add model instruction fragment | Runtime asks the model to call the `checkpoint` tool, not to answer with JSON text. |
| Parse response at boundary | Tool arguments are parsed once at the runtime boundary into typed structs. Assistant-text JSON is degraded fallback only. |
| Validate required fields | Missing/invalid checkpoint tool fields produce a typed recovery nudge/advisory result; hard failure is reserved for emergency fuses or unrecoverable protocol loops. |
| Add tests | Valid, missing-field, invalid-next-action, stale-checkpoint-id, and degraded assistant-text fallback cases are covered. |

**Exit gate:** the runtime can request and parse a structured checkpoint tool report, but it still does not mutate task state from it.

## Phase 6 — Checkpoint artifact retention and audit policy

**Goal:** make checkpoints useful for `work` run audit without polluting conversation history or Docket task events.

Introduce a run-scoped checkpoint artifact store or event stream. Preferred shape:

```text
bear_run_checkpoints
- id uuid primary key
- run_id text not null
- turn_id text nullable
- checkpoint_id text not null
- reason text not null
- control_level text not null
- request jsonb not null
- response jsonb nullable
- validation_status text not null -- requested | valid | invalid | superseded
- visibility text not null -- audit_only | live_ephemeral | model_visible_hidden
- replay_policy text not null -- none | summary_once | until_superseded
- related_task_list_id text nullable
- related_task_item_id text nullable
- related_docket_task_id uuid nullable
- created_at timestamptz not null
```

Retention rules:

- `pair` default: live/debug telemetry or short retention unless needed for recovery.
- `work` default: audit-retained run artifact.
- Docket report-visible history still requires Docket/task events.
- Model replay uses only explicit replay policies, never raw checkpoint prose by default.

| Task | Done when |
| --- | --- |
| Add checkpoint artifact schema/service | Runtime can persist request/response artifacts by run id. |
| Keep artifact outside Docket events | Checkpoints do not appear as `bear_task_events` or `bear_job_events`. |
| Add `work` audit retention | `work` runs retain valid checkpoint artifacts with task refs where available. |
| Add pair/chat retention policy | Interactive stances avoid durable clutter unless recovery policy requires it. |
| Add read API for audits | Operator tooling can inspect checkpoint artifacts for a run/job. |
| Add tests | Checkpoints are retained for `work`, not conversation history, not task events, and not model replay unless policy says so. |

**Exit gate:** structured checkpoints are auditable for `work` while preserving Docket/task boundaries.

## Phase 7 — Checkpoint enforcement in the loop

**Goal:** make checkpoint triggers affect continuation.

When a trigger fires:

1. runtime creates `RuntimeCheckpointRequest`,
2. optional checkpoint thinking policy is resolved,
3. next model inference should call the runtime-owned `checkpoint` tool before more exploration/risky action,
4. runtime validates the tool arguments and records an advisory/audit artifact,
5. runtime continues according to deterministic loop signals plus advisory `next_action`/task-gate state.

| Task | Done when |
| --- | --- |
| Enforce over-exploration checkpoints | Read/search threshold forces checkpoint before more read/search. |
| Enforce failure checkpoints | Consecutive failures force checkpoint before retry. |
| Enforce same-signature checkpoint | Near-ko repeated signature forces different action or checkpoint. |
| Enforce task-gate checkpoint | First/repeated gate rejection can require checkpoint before stronger gate behavior. |
| Enforce pre-risk checkpoint | `careful`/`strict` can require checkpoint before broad/destructive actions. |
| Reset checkpoint observation window | A valid or degraded checkpoint report clears only checkpoint-trigger counters, giving the model a bounded fresh read/search or recovery window without resetting authoritative budgets/ko/fuses. |
| Add loop tests | Simulated turns prove checkpoint tool calls are handled internally, invalid checkpoint reports degrade without killing the run, valid reports can require task-tool follow-through, and checkpoint reports open a fresh checkpoint-observation window. |

**Exit gate:** checkpoints are part of runtime continuation, not merely diagnostics, and a checkpoint report prevents immediate re-triggering of the same checkpoint while preserving budget/ko authority.

## Phase 8 — Optional checkpoint thinking-level escalation

**Goal:** pair checkpoint turns with higher reasoning effort when policy/provider support allows.

| Task | Done when |
| --- | --- |
| Add thinking metadata type | Provider request metadata can carry low/medium/high or provider-equivalent effort. |
| Add capability detection | Model/provider support is known or safely treated as best-effort. |
| Apply only on checkpoint/pre-risk turns | Escalation is bounded to the inference that needs synthesis unless profile says otherwise. |
| Emit diagnostics | Run diagnostics include applied/skipped thinking override and reason. |
| Add tests | Supported provider receives metadata; unsupported provider degrades without failure; enforcement remains independent. |

**Exit gate:** checkpoint turns can request elevated thinking without changing budget/ko/task-gate authority.

## Phase 9 — Task-list/Docket integration

**Goal:** weave checkpoints, focused Job, and task management together without conflicting with history-visible task records.

Focused Job behavior:

- `focused_job_id` identifies the Docket Job that should remain centered across model/tool steps;
- `work` dispatch requires a focused Job before autonomous continuation begins;
- `pair` may designate a focused Job explicitly through conversation or client command;
- focused Job does not itself mark tasks complete, blocked, cancelled, or waived;
- Jobs are mutatable by default, so an executing agent may update the task tree through task/Docket tools;
- a mutable focused Job with no tasks means the next work is defining the executable task tree, not declaring the Job complete;
- a static/frozen Job with no tasks, or only blocked/non-actionable tasks, is an exception that should be flagged before execution continues;
- task focus is derived from Docket/task-list state and remains ephemeral.

Task focus derivation:

- any `in_progress` task wins before pending work is considered;
- if multiple tasks are `in_progress`, choose the first in depth-first task-tree order and emit a diagnostic;
- otherwise choose the first actionable `pending` task in depth-first task-tree order;
- siblings are ordered by `sibling_order`, then creation time, then task id;
- parent tasks are actionable even when they have children unless their own state says otherwise.

Checkpoint requests should include focused Job and active task context when available:

```json
{
  "focused_job_id": "...",
  "task_list_id": "...",
  "task_list_version": 12,
  "active_item_id": "...",
  "active_item_title": "...",
  "source_ref": {
    "kind": "docket_task",
    "job_id": "...",
    "task_id": "..."
  }
}
```

Required behavior:

- focused Job is runtime objective context, not Docket task state;
- checkpoint `task_state_change_needed` is advisory intent only;
- task-state changes require `update_task_list`, `sync_task_list`, or `request_task_list_handoff`;
- Docket-backed changes follow source/sync policy;
- checkpoint artifacts may reference Docket task ids for audit, but do not become Docket events;
- continuation gate evaluates task-list/Docket state, not checkpoint text.

| Task | Done when |
| --- | --- |
| Attach active task context | Checkpoint requests include current task-list/Docket refs when available. |
| Validate task-state intent | Checkpoint tool report can recommend update/sync/handoff but cannot mutate state. |
| Require tool call for state changes | Runtime requires the appropriate task-management tool when task-state change is needed. |
| Preserve gate semantics | Checkpoint tool report alone cannot satisfy completion/blocker/non-applicable state. |
| Add Docket audit correlation | `work` checkpoint artifacts can be queried by run/job/task refs. |
| Add tests | Checkpoint saying “done” does not complete task; `update_task_list` with evidence does. |

**Exit gate:** checkpoints improve `work` auditability while Docket remains canonical for task history.

## Phase 10 — Visibility, BearWire, and operator UX

**Goal:** expose useful runtime legibility without confusing users or models.

Runtime/BearWire events:

- `run.progress kind=agent_loop_control_resolved`,
- `run.progress kind=runtime_checkpoint_required`,
- `run.progress kind=checkpoint_response_recorded`,
- `run.progress kind=checkpoint_thinking_override_applied`,
- `run.progress kind=checkpoint_invalid`.

Visibility defaults:

| Artifact | Pair/chat | Work |
| --- | --- | --- |
| Control level resolution | diagnostic/live progress | diagnostic/live progress |
| Checkpoint request | live progress | live progress + audit artifact |
| Checkpoint tool report | hidden/ephemeral unless useful | audit artifact, optionally operator-visible |
| Provider reasoning stream | live UI only | live UI only unless separate debug retention |
| Task progress | task-list/Docket tools only | task-list/Docket tools only |

| Task | Done when |
| --- | --- |
| Add BearWire projections | Runtime checkpoint events project as typed `run.progress` events. |
| Add operator read path | Work run checkpoint audit can be inspected from admin/operator surfaces. |
| Keep conversation history clean | Checkpoint artifacts do not appear as ordinary user-visible assistant messages. |
| Add replay rules | Only normalized model-visible hidden continuity notes replay, not raw provider reasoning or checkpoint prose by default. |

**Exit gate:** humans can understand why a run checkpointed, but task history and transcript remain clean.

## Phase 11 — Rollout

Recommended rollout flags:

```text
BEARS_AGENT_LOOP_CONTROL=off|observe|enforce
BEARS_AGENT_LOOP_CHECKPOINTS=off|on
BEARS_CHECKPOINT_THINKING=off|on
BEARS_CHECKPOINT_AUDIT=off|work|all
```

Rollout order:

1. **Observe:** resolve profiles and emit diagnostics; no enforcement.
2. **Checkpoint observe:** trigger would-checkpoint events and validate policy thresholds.
3. **Checkpoint enforce for failures/over-exploration:** enable `standard` for `pair`/`chat` and `work`.
4. **Checkpoint audit for `work`:** retain structured checkpoint request/response artifacts by run/job/task refs.
5. **Thinking escalation:** enable checkpoint/pre-risk thinking overrides for supported models.
6. **Strict/careful pre-risk:** enable broader `careful`/`strict` behaviors for high-risk workflows.

## Validation matrix

| Area | Required tests |
| --- | --- |
| Resolver | model default, Bear override, stance override, task escalation, source attribution |
| Profiles | level ordering, thresholds, checkpoint thinking defaults |
| Budgets | wall-clock/tool/failure/ko limits from profile, global fuses preserved |
| Checkpoint triggers | over-exploration, failure, same-signature, task-gate, low-budget, pre-risk |
| Structured response | valid response, missing field, invalid next action, stale checkpoint id |
| Audit artifacts | `work` retention, pair/chat non-retention/default retention, query by run/job/task refs |
| Task boundary | checkpoint cannot complete/block/cancel/waive task; task tools can with evidence |
| Thinking escalation | supported/unsupported providers, bounded to checkpoint turn, diagnostics emitted |
| Projection | BearWire progress events, no ordinary conversation history pollution, no Docket event pollution |

## First implementation slice

The safest first slice is:

1. add typed control levels/profiles;
2. add resolver with model default + Bear/stance override support;
3. emit `agent_loop_control_resolved` diagnostics;
4. add checkpoint request/response DTOs behind an observe-only flag;
5. add checkpoint artifact schema/service but retain only in `work` observe mode;
6. add tests proving checkpoint artifacts are not conversation history, not model replay, and not Docket events.

This slice creates the typed foundation and audit model before enforcement changes model behavior.
