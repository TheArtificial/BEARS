# Den state machine inventory

**Status:** Living architecture reference.

This document inventories the state dimensions that apply to a Den conversation, session, turn, run, and Docket-focused unit of work. It is intentionally a matrix of orthogonal axes, not a proposal for one giant enum.

Use this document when adding or changing runtime state, mode labels, continuation policy, approvals, task focus, model selection, or client projection. If a new state dimension can affect what the model may do, whether a turn may stop, what a client shows, or how a run resumes, update this inventory in the same change.

## Maintenance rule

State changes are architecture changes. A PR or plan that adds, removes, renames, or changes semantics for any state axis below should also update:

- this inventory;
- the owning architecture document or ADR;
- relevant roadmap/implementation plans;
- at least one executable invariant, table-driven test, replay assertion, or projection test when the state affects runtime behavior.

Prompt text and UI labels are never authority by themselves. Canonical state must remain typed, owned, replayable where it affects runtime behavior, and derivable into prompt/UI projections.

## State-machine shape

Den should keep a broad inventory of mostly orthogonal product axes:

```text
TurnStateInventory =
  BearIdentity
× UserMembership
× ConversationState
× SessionBinding
× TrustProfile
× Governance
× FocusState
× WorkflowState
× PermissionPolicy
× TurnRunLifecycle
× ObligationSet
× LoopControlState
× ModelSelection
× RecoveryState
```

That inventory is not the runtime authority surface. The implemented authority
surface is intentionally smaller:

```text
RuntimeAuthorityState =
  TurnAuthority
× ResolvedFocus
× DocketTaskState
× TurnRunState
× ObligationSet
× LoopControlState
```

The three authority owners that must stay singular are:

```text
MutationAuthority = TurnAuthority
FocusDrivenContinuation = ResolvedFocus × DocketTaskState
ActiveObligations = TurnRunState × ObligationSet
TerminalOutcome = TurnRunState × ObligationSet × TurnStepState × BearWireTerminalEvent
```

`TurnAuthority` is the compiled permission surface for the turn: stance plus
mode/plan policy determine tool routing, prompt authority blocks, and client
permission projection. Governance is a separate run-supervision context and is
not a mutation-permission input; prompt or client labels cannot feed authority
back into either seam. `ResolvedFocus` is the only
focus input that may let completion policy force continuation for unfinished
focused work. `TurnRunState` owns lifecycle and active wait reason. Terminal
closure is committed through one atomic finish operation that transitions the
run, settles obligations and active steps, and appends exactly one run terminal
BearWire event before exposing the terminal state.

The remaining axes feed compilation, execution, or projection. They are not peer
authority owners unless a typed implementation seam consumes them into one of
the authority owners above:

```text
EffectivePolicyProjection = TurnAuthority × Armature × RunAuthContext

RunSupervisionProjection = Governance × HumanPresence

CompletionProjection = ResolvedFocus × DocketTaskState × Governance × BudgetState × ObligationSet

DerivedViews = ModelRequest × PromptContext × MemoryProjection × CompactionProjection × SurfaceProjection
```

`DerivedViews` are explicitly non-authoritative. They may cache or render canonical
state, but cannot expand mutation permission, manufacture focus/obligations, choose
a terminal outcome, or override current-turn ordering.

Do not let a derived view become a peer source of truth. If two axes disagree,
resolve the conflict into canonical state before prompt rendering or client
projection.

Exit gate for the reduced authority model:

- mutation authority has one owner: `TurnAuthority`;
- focus-driven continuation has one owner: active `ResolvedFocus` plus current
  Docket task state;
- active waits have one owner: `TurnRunState` plus `ObligationSet`;
- projections, caches, labels, model choice, prompt text, and compaction state
  cannot expand authority or manufacture focus/work.

## Ownership summary

| Axis | Primary owner | Scope/lifetime | Notes |
| --- | --- | --- | --- |
| Bear identity/profile binding | Den registry/control plane | durable Bear | Identifies the Bear and stance template, not the current run's supervision. |
| User membership/access | Den identity/RBAC | durable + request-scoped | Gates which conversations/surfaces a user can access. |
| Conversation | Den Postgres | durable user-visible chat container | Owns transcript, archive state, title, model selection, and conversation-scoped focus. |
| Session/client binding | adapter/BearWire/web edge + Den session store | live client binding | Projects conversation/runtime state to a connected client; not the conversation. |
| Trust profile | Bear profile registry | per turn/template | `chat`, `pair`, `curate`, `work`, `watch`; memory/tool/default trust contract. |
| Governance | runtime/workspace session | run-scoped mutable timeline | `interactive`, `grace`, `autonomous_continuation`, `observational`, `frozen`. |
| Focused Job | Docket + conversation focus resolver | conversation-scoped durable objective | Only active focus may drive focused completion behavior. |
| Task focus | runtime derivation | ephemeral per run/turn | Next actionable Docket/task-list item derived from active focus and task state. |
| Workflow state | current-turn state compiler | current turn | Inputs compiled into `TurnAuthority`; derived operational focus is advisory. |
| Permission policy | Den policy resolver + descriptors | current turn/tool call | Resolves Ask/Plan/Write, tool classes, approvals, and armature routes before `TurnAuthority`/routing consume them. |
| Turn/run lifecycle | runtime turn controller + atomic run finisher | run/turn | Accepted/running/waiting states use ordinary transitions. Completed/failed/cancelled state, obligation/step settlement, and the terminal BearWire event are one atomic finish operation. |
| Obligations | obligation coordinator | per tool/permission/human wait | Client/tool/approval waits; blocks only while open. |
| Loop control/budgets | agent loop controller | run/turn | Budgets, checkpoints, KO/failure signals, context pressure. |
| Recovery/error | runtime recovery logic | run/turn/session | Retry, resume, terminal outcome, late-result handling. |
| Derived views: model request, prompt/context, memory, compaction, UI/surface caches | projection/assembler owners | request/turn/session bounded | Non-authoritative views only; cannot create permissions, focus, obligations, lifecycle, or completion. |
| Work surface/sandbox | Docket/work execution | job/run/workspace | Execution materialization; not conversation focus by itself. |

## Axis inventory

### Bear identity and involved users

Representative state:

- Bear id and slug;
- bound trust-profile/stance config;
- involved users and membership roles;
- source user/session ids;
- compiled prompt/config binding;
- per-Bear memory-store binding.

Invariants:

- Bear identity is durable and not changed by governance, focus, or client reconnects.
- User access is resolved structurally, not by parsing rendered labels.
- Trust-profile changes apply to new turns/templates; they do not silently launder memory/tool scope mid-run.

### Conversation

Representative state:

- canonical conversation id;
- pending/external client conversation id during materialization;
- title and title timestamp;
- archived vs active;
- transcript and message persistence;
- conversation-scoped focused Job id, when active;
- selected model state;
- compaction and prompt-memory state;
- review/reflection candidate state.

Invariants:

- A conversation is the durable user-visible chat container. A session is only a live binding to it.
- Archived conversations should not accept ordinary new turns unless explicitly restored or unarchived.
- Title updates are non-blocking structured updates; they must not create model-visible obligations or force continuation.
- Conversation focus must be cleared or restored through canonical focus state, not stale prompt text or session cache.

### Session and client binding

Representative state:

- client session id;
- client kind/channel;
- conversation id and resolved conversation id;
- current client mode/projection;
- cwd and workspace roots;
- armature availability;
- advertised client tools and direct tool support;
- connected/disconnected state;
- active turn registration.

Common client modes:

- `ask`;
- `plan`;
- `write`;
- Focused as a projection when conversation focus is active, not an ordinary permission preset.

Invariants:

- Client mode is not canonical focus. Changing client mode away from Focused clears conversation focus.
- Session-local projections can seed prompts/tools only when validated against current conversation/session state.
- A stale session cache must never resurrect cleared conversation focus.

### Trust profile

Values:

- `chat`;
- `pair`;
- `curate`;
- `work`;
- `watch`.

Owned semantics:

- memory read/write scope;
- default tool roster;
- approval/autonomy defaults;
- cross-role visibility;
- default prompt slice.

Invariants:

- Governance does not change trust profile.
- A `pair` run that continues autonomously does not become `work`.
- A `work` run under observation does not gain `pair` memory scope.

### Governance

Values:

- `interactive` — human present, live collaboration;
- `grace` — client recently disconnected, bounded continuation/cleanup;
- `autonomous_continuation` — human absent, runtime drives approved/focused work;
- `observational` — human present read-only inspection;
- `frozen` — panic, handoff, or cancelled checkpoint awaiting disposition.

Expected transitions:

```text
interactive -> grace                    client disconnected
grace -> autonomous_continuation         grace timeout
autonomous_continuation -> observational inspection opened
observational -> autonomous_continuation inspection released
any active -> frozen                     panic, handoff, cancellation
frozen -> interactive                    explicit resume/new workspace session
```

Invariants:

- Governance changes continuation pressure, not memory or trust scope.
- `observational` cannot own mutation or continuation.
- `frozen` cannot continue the model/tool loop.
- Autonomous continuation without an explicit focused objective is invalid for `work` and suspicious for interactive profiles.

### Focus, Docket, and task orientation

Representative state:

- conversation-focused Docket Job id;
- active Docket execution session;
- session task-list projection;
- task orientation (`freeform`, task-oriented, Job-focused);
- Job and task status;
- next actionable task;
- handoff/sync state.

Job statuses:

- `draft`, `ready`, `running`, `blocked`, `completed`, `cancelled`, `archived`.

Task statuses:

- `pending`, `in_progress`, `done`, `blocked`, `cancelled`.

Focus sources should be treated as:

| Source | Completion authority? | Notes |
| --- | --- | --- |
| Active conversation/Docket focus | yes | May force continuation while incomplete actionable work remains. |
| Current-turn explicit focus selection | yes after persisted/resolved | Must be typed and current, not prompt-only. |
| Runtime/session cached task list | no by default | Prompt/tool seed only unless validated as current active focus. |
| None | no | Completion must be allowed when no obligations/budgets block. |

Critical invariant:

```text
FocusState=None => completion policy must not return Continue(FocusedWorkRemains)
```

Do not let task orientation or stale activity projection force continuation. Only active focused work may do that.

### Current-turn workflow state

Canonical domains:

1. `workplan` — proposal and approval lane;
2. `activity` — live tactical progress lane;
3. `memory` — durable semantic capture lane;
4. `execution` — current-turn capability/effect lane.

Representative fields:

- `state_authority`;
- `state_version`;
- `permission_mode`;
- `tool_classes`;
- `workplan.state`;
- `workplan.approval_status`;
- `activity.status`;
- `activity.current_item`;
- `execution.execution_unlocked`;
- `memory.active_plan_write_allowed`.

Derived `operational_focus` values may include:

- `clarify`;
- `plan`;
- `execute`;
- `await_approval`;
- `summarize`;
- `recover_context`;
- `handoff`.

Invariants:

- Current-turn workflow state contributes typed inputs to `TurnAuthority`; it is
  not a separate mutation authority after compilation.
- `operational_focus` is advisory.
- If execution is locked, no derived focus may imply mutation is allowed.
- Prior-turn state and prompt reminders must not override current-turn authority.

### Workplan and plan-mode gate

Representative states:

- absent/inactive;
- `active` / drafting;
- `submitted` / awaiting approval;
- `approved`;
- `rejected` / cancelled.

Derived policy shape:

| Client mode / plan state | Effective behavior |
| --- | --- |
| Ask, no plan | read-only |
| Plan, no plan | read-only |
| Write, no plan | write-capable subject to policy/descriptors |
| active/submitted plan | read-only until approved |
| approved plan | execution may unlock subject to policy/descriptors |
| rejected plan | read-only/cancelled |

Invariants:

- Workplan approval is not the same as individual tool approval.
- Submitted/drafting workplans do not unlock mutation.
- Rejected workplans cannot preserve write capability through stale mode state.

### Permissions, approvals, and tool routing

Representative state:

- permission mode: Ask/Plan/Write;
- allowed and denied tool classes;
- tool enablement: read-only vs all enabled classes;
- approval-required policy;
- active approval ids;
- approval decisions and scopes;
- tool execution route: Den server, adapter-local, unsupported.

Tool classes:

- `read_only`;
- `workspace_mutation`;
- `execution`;
- `browser`.

Invariants:

- Tool availability comes from resolved policy and descriptors, not prompt labels.
- Execution ownership is resolved once by the descriptor-owned `ToolExecutionOwner` resolver; projection, initial streams, continuations, recovery, and armatures consume that result rather than matching tool names independently.
- Adapter-local tools that require a client response must create obligations.
- Den-server tools must not wait for client tool results; Den-owned approval requests may wait only on a typed permission obligation.
- Unsupported tools fail or stop explicitly; they do not wait forever.

### Tool calls and obligations

Turn phases:

- `Created`;
- `Streaming`;
- `WaitingForObligations`;
- `ContinuingAfterTool`;
- `Cancelling`;
- `Terminal`.

Obligation kinds:

- `ToolResult`;
- `PermissionDecision`;
- `HumanInput`;
- `ResourceBinding`;
- `HandoffDecision`.

Obligation states:

- requested/waiting;
- result received;
- continued;
- failed;
- timed out;
- cancelled;
- late ignored.

Invariants:

- Open obligations block turn completion/continuation decisions; settled obligations do not.
- Terminal turns cannot be reopened by late client/tool/permission results.
- Waiting states require a matching open obligation.

### Turn and run lifecycle

Representative states:

- `Accepted`;
- `Running`;
- `WaitingForClient`;
- `Continuing`;
- `Completed`;
- `Failed`;
- `Cancelled`.

Terminal outcomes/reasons include:

- ok/end-turn/stream-complete;
- failed/stream-error/tool-timeout/unsupported-tool;
- cancelled;
- orphaned-requires-approval;
- recovered;
- needs-new-session.

Invariants:

- At most one active turn per runtime ownership key.
- Terminal run states must not have open obligations or active turn steps.
- A terminal run state must have a matching durable `run.completed`, `run.failed`, or `run.cancelled` event committed in the same transaction.
- Ordinary `transition_run` calls are nonterminal-only; terminal fixtures and production outcomes use the atomic finisher.
- `WaitingForClient` requires at least one open client obligation.
- The specific blocking reason (`tool_result`, `permission_decision`, human/resource/handoff, or `multiple`) is derived from open obligations and is not duplicated in run state.

### Completion and continuation

Completion decisions:

- complete;
- continue.

Representative complete reasons:

- no active focused task;
- focused work complete finalization drain;
- focused work complete or terminally blocked;
- repeated terminal objection.

Representative continue reasons:

- focused work remains;
- runtime limit is not focused completion.

Invariants:

- No active focus means normal final answers may complete the turn when no obligations block.
- Focused incomplete actionable work may force continuation only while focus is active.
- Runtime limits, budget pressure, progress reports, tool failure, assistant text, and stream EOF are not run completion.
- ACP ends a turn only after a durable run terminal event; tool-level terminal updates are not run terminal events.
- Den-owned tool requests continue draining the runtime stream; armature-owned requests and typed approval waits form client-wait boundaries.
- Repeated task-gate rejection should stop with a human-review blocker instead of infinite continuation.

### Loop control, budgets, and checkpoints

Control levels:

- `light`;
- `standard`;
- `careful`;
- `strict`.

Budget dimensions:

- wall-clock;
- total tool calls;
- tool-class calls;
- context/token budget;
- consecutive failures;
- repeated same tool/signature;
- emergency hard steps;
- post-mutation verification window.

Tool budget classes:

- `read`, `search`, `fetch`, `execute`, `write`, `destructive`, `other`.

Invariants:

- Loop control may checkpoint, warn, stop, or nudge; it does not mutate task state by itself.
- Checkpoint prose cannot satisfy Docket/task-list gates.
- Budget exhaustion can stop a run, but it is not proof that focused work is complete.

### Error and recovery

Representative state:

- consecutive tool failures;
- same-signature KO counts;
- stale approval recovery;
- interrupted run recovery attempts;
- retry final delivery;
- overflow/context recovery;
- orphaned approval detection;
- unsupported tool handling;
- late result ignored counts.

Invariants:

- Recovery may add context or retry delivery, but it must not mutate focus, task, or permission state except through explicit typed transitions.
- Needs-new-session and frozen states are explicit terminal/recovery outcomes, not hidden active waits.

### Model selection

Representative state:

- selection mode: auto or explicit;
- requested model;
- selected model;
- effective model;
- model options/capability profile;
- per-model loop-control default.

Invariants:

- Model selection affects provider request and model capability defaults.
- Model selection must not directly change focus, governance, or permission mode.
- Mid-active-turn model changes apply at a clear turn boundary unless explicitly designed otherwise.

### Prompt, context, memory projection, and compaction

Representative state:

- compiled prompt/config hash;
- projected key memory;
- prompt-memory blocks;
- transcript window;
- compaction state and diagnostic;
- context budget report;
- runtime supplements and reminders.

Invariants:

- Prompt/context projection is not authority.
- Compaction must preserve unresolved obligations and active focus accurately, but must not create either.
- Old prompt text cannot preserve focus, permission, or governance after canonical state changes.

### Work surface, sandbox, and external execution

Representative state:

- work surface ref;
- work branch and commit policy;
- workspace/sandbox/session id;
- work run state;
- upstream auth context;
- egress policy;
- checkpoint/publish branch.

Work run states:

- `queued`, `claimed`, `provisioning`, `running`, `reporting`, `succeeded`, `blocked`, `failed`, `cancelled`, `timed_out`.

Invariants:

- Work surface state is not conversation focus by itself.
- A run may have a workspace without the current conversation being focused on that Job.
- Outbound auth follows `RunAuthContext`, never client labels or trust-profile names alone.

## High-value validity matrices

### Trust profile × governance × focused Job

| Trust profile | Governance | Focused Job requirement | Notes |
| --- | --- | --- | --- |
| `chat` | `interactive` | none | ordinary chat |
| `pair` | `interactive` | none unless explicitly focused | normal pair collaboration |
| `pair` | `autonomous_continuation` | explicit focused objective required | no silent profile flip to `work` |
| `work` | `autonomous_continuation` | required | normal work execution |
| `watch` | `observational` | optional | inspect/observe only |
| any | `frozen` | none active | no continuation/mutation |

Invalid or suspicious combinations should be rejected before prompt assembly or surfaced as explicit diagnostics.

### Focus source × completion policy

| Focus source | May pass focused task list to completion policy? | May force continuation? |
| --- | --- | --- |
| active durable conversation/Docket focus | yes | yes |
| explicit current-turn focus selection after resolution | yes | yes |
| stale session cache / cached task list only | no | no |
| none | no | no |

### Run state × obligations

| Run state | Required obligation condition |
| --- | --- |
| Running | no blocking awaited obligation |
| WaitingForClient | at least one open client obligation; blocking reason derived from obligation responder actions |
| Completed/Failed/Cancelled | no open obligations or active steps; exactly one matching terminal BearWire event committed atomically; late results ignored |

## Test obligations

Add or update tests whenever behavior crosses axes. Keep these seam checks permanently mapped to the reduced authority model:

| Seam | Current executable check |
| --- | --- |
| submitted/drafting plan cannot unlock mutation | `den-core` `submitted_plan_keeps_write_tools_locked`; `turn_authority_is_single_derived_permission_surface` |
| prompt/client projection labels cannot expand authority | `den-core` `turn_authority_ignores_client_policy_projection_labels`; `turn_authority_has_no_prompt_or_compaction_authority_input` |
| model choice is not an authority source | `den-core` `turn_authority_has_no_model_choice_authority_input`; loop-control tests may still cover model-default budget/checkpoint behavior |
| stale cached task list cannot manufacture focus/continuation | `den-runtime` `no_focus_allows_final_even_with_cached_task_list`; `final_gate_ignores_and_clears_cache_without_durable_focus` |
| terminal transitions atomically own run state, obligation/step closure, and terminal event | `den-bearwire` completion/cancellation/expiry/failure persistence tests; `den-runtime` `terminal_turn_run_cannot_be_reopened_or_overwritten`; `client_obligation_coordinator_contract` late-result tests |
| WorkRun owns sandbox root materialization | `den-docket` `effective_work_run_root_prefers_trimmed_request_then_job_default` plus provisioning's `WorkRunRow.root_name` check |

Baseline scenarios to preserve:

1. no focus allows final answers, even with a stale cached task list;
2. changing mode away from Focused clears focus before completion policy runs;
3. active durable focus with incomplete actionable work may force continuation;
4. completed or terminally blocked focused work allows finalization;
5. `work` + autonomous continuation without focused Job is invalid;
6. submitted/drafting plan keeps mutation tools locked;
7. terminal run with open obligation is invalid;
8. late tool/permission result after terminal run is ignored;
9. model selection changes do not mutate focus/governance/permissions;
10. prompt/compaction projection cannot create focus or obligations;
11. Den-owned tool requests do not stop initial or continuation stream consumption unless a typed approval obligation waits on the client;
12. terminal run state, settled obligations/steps, and one terminal BearWire event commit or roll back together.

Prefer pure derivation/validation tests first, then narrow persistence/replay tests at state seams.

## Related documents

- [Den runtime](den-runtime.md)
- [Workflow state overview](workflow-state-overview.md)
- [Interactive stances and role axes](interactive-stances-and-role-axes.md)
- [ACP runtime contract](acp-runtime-contract.md)
- [Non-blocking structured updates](non-blocking-structured-updates.md)
- [Tasks and autonomy](tasks-and-autonomy.md)
- [ADR-0027: Workflow-state ontology](../decisions/adr-0027-workflow-state-ontology.md)
- [ADR-0039: Trust profiles and governance states](../decisions/adr-0039-trust-profiles-and-governance.md)
- [ADR-0045: Session task lists as Docket checkouts and working projections](../decisions/adr-0045-session-task-lists-and-docket-checkout.md)
- [ADR-0048: Core turn/client-obligation coordinator](../decisions/adr-0048-core-turn-client-obligation-coordinator.md)
- [ADR-0050: Agent Loop Control, Adaptive Budgets, and Runtime Checkpoints](../decisions/adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md)
