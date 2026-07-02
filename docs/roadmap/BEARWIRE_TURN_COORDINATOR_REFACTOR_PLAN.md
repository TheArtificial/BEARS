# BearWire turn coordinator refactor plan

Status: implemented  
Date: 2026-07-02  
Last updated: 2026-07-02  
Related ADR: [ADR-0048: Core turn/client-obligation coordinator](../decisions/adr-0048-core-turn-client-obligation-coordinator.md)

## Goal

Restore a consolidated, protocol-neutral Den turn/obligation state machine after the ACP-to-edge migration. BearWire remains the Den ↔ armature wire, and ACP remains edge-only, but model-continuation decisions move behind a core coordinator that is also usable by channels such as web chat, Slack, and future macOS surfaces.

## Problem statement

Recent BearWire permission/tool failures show that continuation decisions are currently fragmented across:

- `den-runtime` stream state;
- BearWire run/obligation persistence;
- BearWire `client.permission.result` and `client.tool.result` handlers;
- armature permission UI and auto-approval;
- local tool execution callbacks;
- streamed BearWire event ordering.

This permits illegal states:

- permission approval can continue the model before a tool result exists;
- one tool result can continue the model while sibling tool obligations from the same model step remain open;
- stale `run.paused` events can appear after permission/tool settlement;
- local tool errors can be mixed into run infrastructure failure behavior;
- repeated auto-approval can cause loops until the runtime hits max steps.

## Architectural target

```text
Den runtime core
  owns turn/run state, step state, obligations, settlement, continuation barriers

Surface projections
  BearWire for trusted armatures
  web chat / Slack / macOS channel actions for human input, approval, resource binding, and channel-local interactions

Armature / channel adapters
  own protocol projection, permission UI, channel UI, local tool execution, local caches
```

## Core invariants

1. **Only the coordinator may start model continuation.** BearWire RPC methods record inputs and receive coordinator outcomes.
2. **Continuation requires a closed step barrier.** A model continuation may start only when the current model step has no open client obligations.
3. **Permission approval is not tool execution.** For armature-local tools, approval transitions to local tool dispatch, not model continuation.
4. **A model step is a batch.** Multiple tool calls from one model step settle together; continuation happens once per step.
5. **Actionable waits are obligations.** `client.waiting` must carry `obligation_id` and `expected_client_method`; `run.paused` is non-actionable status.
6. **Tool errors are normal tool results.** File-not-found and similar local failures settle tool-result obligations and are shown to the model.
7. **Durable IDs fence results.** Results are scoped by run/session/obligation/tool/permission, and the target state adds `turn_step_id`.
8. **Events follow persistence.** Den must persist obligations before streaming answerable wait events.

## Target model

### Tables / persisted concepts

```text
turn_runs
  run_id
  session_id
  state

turn_steps
  id
  run_id
  step_index
  state
  provider_response_id
  opened_at
  closed_at

turn_obligations
  id
  run_id
  turn_step_id
  session_id
  kind
  expected_responder_action
  responder_ref_id
  tool_call_id
  permission_id
  state
  request_payload
  result_payload

turn_obligation_results
  id
  run_id
  turn_step_id
  obligation_kind
  obligation_id
  result_hash
  payload_json
```

### Run states

```text
accepted
running
waiting_for_client
waiting_for_tool_result
waiting_for_permission
continuing
completed
failed
cancelled
```

### Step states

```text
streaming_model
waiting_for_client
ready_to_continue
continued
failed
cancelled
```

### Obligation states

```text
requested
waiting_for_client
result_received
continued
failed
cancelled
```

The obligation `kind` and `expected_responder_action` values carry the semantic meaning (`permission_decision`, `tool_result`, `human_input`, `resource_binding`, `handoff_decision`) while the state remains protocol-neutral lifecycle state.

## Coordinator API shape

Introduce a core module, likely in `den-runtime`, such as:

```text
den_runtime::turn_coordinator
```

or:

```text
den_runtime::client_obligations
```

Initial API sketch:

```rust
pub enum ClientResultInput {
    Permission {
        run_id: RunId,
        session_id: ClientSessionId,
        obligation_id: ObligationId,
        permission_id: PermissionId,
        decision: PermissionDecision,
        reason: Option<String>,
    },
    ToolResult {
        run_id: RunId,
        session_id: ClientSessionId,
        obligation_id: Option<ObligationId>,
        tool_call_id: ToolCallId,
        status: ToolResultStatus,
        content: String,
        structured_content: serde_json::Value,
    },
}

pub enum CoordinatorOutcome {
    WaitingForMoreClientResults {
        run_state: RunState,
        open_obligations: Vec<ObligationSummary>,
    },
    DispatchLocalTool(LocalToolRequest),
    ContinueModel(ContinueModelRequest),
    IgnoredLateResult(LateResultReason),
    Failed(CoordinatorFailure),
}
```

BearWire handlers become thin adapters:

```rust
let outcome = coordinator.record_client_result(input).await?;
project_outcome_to_bearwire_response(outcome)
```

## Phased implementation

### Phase 0: containment and documentation

Status: implemented (2026-07-02).

- Document ADR-0048 and this plan.
- Treat `client.waiting` as canonical actionable wait.
- Keep `run.paused` non-actionable in armatures.
- Ensure permission approval for armature-local tools returns `local_tool_request`, not model continuation.
- Ensure `RunPaused` terminates the current runtime stream segment so the same provider response cannot keep emitting new waits after pausing.

Done when:

- Current loop/failure class is contained.
- Docs state the correct ownership and invariants.

### Phase 1: add open-obligation barrier

Status: implemented baseline (2026-07-02).

Add repository helpers:

```rust
open_obligations_for_run(run_id)
open_obligations_for_step(run_id, turn_step_id) // after step ids exist
has_open_obligations_for_run(run_id)
```

Patch `client.tool.result`:

1. record tool result;
2. mark obligation result received;
3. check sibling open obligations;
4. if any remain, return `WaitingForMoreClientResults` and do not continue;
5. if none remain, continue once.

This phase may use run-level barriers before `turn_step_id` exists.

Done when:

- One model step with two armature-local tool calls does not continue after the first result.
- Continuation starts only after both results are recorded.

### Phase 2: extract coordinator facade

Status: implemented (2026-07-02). `den_runtime::client_obligation_coordinator` owns client result recording, duplicate/conflict detection, late-result handling, permission-to-local-tool transition, tool-result settlement, and continuation-readiness decisions. BearWire performs transport/auth parsing and projects coordinator outcomes into BearWire responses, events, and continuation tasks.

Create the coordinator module and move continuation decision logic out of BearWire handlers.

Move or wrap logic for:

- duplicate/conflicting client results;
- permission result normalization;
- permission-to-local-tool transition;
- tool-result settlement;
- continuation readiness;
- late terminal run handling.

BearWire should only:

- authenticate/authorize;
- parse BearWire params;
- call coordinator;
- project coordinator outcome into BearWire response/events.

Done when:

- `client.permission.result` and `client.tool.result` no longer directly call `spawn_continuation_task` except through coordinator outcome handling.
- Coordinator tests cover local approvals, Den-hosted approvals, denial, tool error result, duplicate result, and late result.

### Phase 3: add run-step identity

Status: implemented (2026-07-02). `turn_steps` and nullable `turn_step_id` columns are available; all new core wait helpers assign obligations to an active turn step, client results record that step id, stale/wrong-step results are rejected, and coordinator barriers use step-level checks when present with run-level fallback for older rows.

Add schema:

```text
turn_steps
turn_obligations.turn_step_id
turn_obligation_results.turn_step_id
```

Migration strategy:

- Add nullable `turn_step_id` first.
- New runs write step rows and obligation step ids.
- Existing rows remain valid via run-level compatibility fallback.
- Later make `turn_step_id` required for new active runs.

Done when:

- Every new client obligation belongs to a `run_step`.
- Continuation barriers check step-level obligations, not all run-level obligations.

### Phase 4: transactional obligation/event outbox

Status: implemented (2026-07-02). `den_runtime::turn_waits` owns transactional wait persistence. Runtime tool-call waits update run state, ensure/create the turn step, upsert the client obligation, and append the BearWire event in one transaction. `client.waiting` events are emitted only from persisted obligation data and include validated `obligation_id`, `expected_client_method`, `permission_id`, `tool_call_id`, and `turn_step_id`. Non-BearWire surfaces can use the generic surface-obligation transaction helper without creating another state machine.

Create a single operation for answerable waits:

```text
BEGIN
  create/update run step
  create/update client obligation
  append client.waiting BearWire event
  update run/step state
COMMIT
```

Done when:

- Den cannot stream `client.waiting` unless the corresponding obligation exists in the same committed transaction.
- Tests assert event payload IDs match persisted obligations.

### Phase 5: make `run.paused` non-actionable and remove compatibility paths

Status: implemented for active armature flow (2026-07-02). The armature treats `run.paused` as status-only and ignores legacy `tool_call.blocked` / `permission.requested` as actionable permission UI. `client.waiting` is the only active permission wait path.

- Stop using `tool_call.blocked` for active permission flow.
- Keep `tool_call.blocked` only as legacy/replay compatibility if still needed.
- Armatures render permission UI only for `client.waiting` with valid `obligation_id` and `expected_client_method`.
- Remove direct handling of `run.paused` as tool activity.

Done when:

- No active armature permission path depends on `run.paused` or `tool_call.blocked`.

### Phase 6: neutralize coordinator API and tighten typed state

The coordinator must be deeper than BearWire. Keep BearWire as one projection over the coordinator, not the owner or vocabulary source for the state machine.

#### Phase 6A: neutralize Rust-facing coordinator API over existing tables

Status: implemented (2026-07-02).

- Rename Rust-facing concepts away from BearWire where they are core turn semantics:
  - Core Rust-facing obligation rows now use neutral `TurnObligationRow` / `TurnObligationState` naming.
  - `ExpectedClientMethod` should become a protocol-neutral expected responder/action type where possible.
- Deployed BearWire-prefixed turn tables are renamed forward to neutral `turn_*` tables.
- New not-yet-deployed fields/tables should use neutral names such as `turn_steps` and `turn_step_id`.

Done when:

- BearWire handlers consume coordinator outcomes and projection types, not raw storage rows where avoidable.
- Core coordinator APIs can be called by non-BearWire surfaces without leaking BearWire method names.

#### Phase 6B: add non-BearWire obligation kinds

Status: implemented (2026-07-02). Core turn obligations now use the neutral kind/action set: `tool_result`, `permission_decision`, `human_input`, `resource_binding`, and `handoff_decision`. Existing tool/permission paths no longer store legacy `tool_call` / `permission` kind values, and a generic `create_turn_obligation_for_step` helper can create non-BearWire/channel obligations with a neutral `responder_ref_id`.

Extend the core obligation model beyond current tool/permission pressure:

- `ToolResult`
- `PermissionDecision`
- `HumanInput`
- `ResourceBinding`
- `HandoffDecision`

Done when:

- The coordinator can represent a turn waiting on human/channel input, not only armature-local tools.
- Obligation kinds describe what the turn needs, not which wire method will answer it.

#### Phase 6C: add surface projection contracts

Status: implemented (2026-07-02). `den_runtime::surface_projection` defines neutral surface kinds and action projections for BearWire armatures, web chat, Slack, and macOS app surfaces. Unsupported obligation/surface pairs are explicit rather than silently inventing a state machine.

Define how core obligations project to each surface:

- BearWire armature projection:
  - `ToolResult` → `client.tool.result`
  - `PermissionDecision` → `client.permission.result`
  - actionable wait event → `client.waiting`
- Web chat projection:
  - `HumanInput` → chat reply/action
  - `PermissionDecision` → web approve/deny action
- Slack projection:
  - `HumanInput` → thread reply
  - `PermissionDecision` → button/action callback
- macOS app projection:
  - may act as channel, armature, or both depending on capabilities.

Done when:

- Adding a channel means writing a projection for supported obligation kinds, not creating another turn state machine.

#### Phase 6D: persistence rename/migration

Status: implemented (2026-07-02). Core turn state uses neutral persistence names: `turn_runs`, `turn_steps`, `turn_obligations`, and `turn_obligation_results`. `bearwire_events` remains BearWire-specific because it is the wire event log.

Completed renames:

- `bearwire_runs` → `turn_runs`
- `bearwire_run_obligations` → `turn_obligations`
- `bearwire_client_results` → `turn_obligation_results`

Done when:

- DB names match ownership.
- Schema regression verifies neutral `turn_*` tables/columns and preserves only intentional BearWire wire tables.

#### Phase 6E: typed IDs and states

Status: implemented (2026-07-02). Core turn coordination now has typed ID wrappers for run/session/tool/permission/responder/action/step/obligation ids, typed parsers for turn run/step/obligation states including neutral `waiting_for_client`, and neutral typed obligation kind/action parsing. Repository rows still carry storage strings at the DB boundary but expose typed parse helpers; coordinator logic uses typed action/state helpers where practical.

Replace stringly internal control state with typed values:

- `RunState`
- `TurnStepState`
- `TurnObligationState`
- `TurnObligationKind`
- expected responder/action enum
- typed IDs for run, turn step, obligation, tool call, permission, session, channel action.

Done when:

- Illegal state transitions are compile-time-visible or centralized in the coordinator.
- BearWire methods do not compare raw method/status strings outside boundary parsing.
- Non-BearWire surfaces can satisfy obligations through typed coordinator inputs.

## Required tests

### Coordinator unit/integration tests

1. Permission approval for armature-local tool returns `DispatchLocalTool`, not `ContinueModel`.
2. Tool result after local dispatch returns `WaitingForMoreClientResults` if sibling obligation remains.
3. Tool result after final sibling obligation returns `ContinueModel` once.
4. Den-hosted approved tool can execute/continue through coordinator.
5. Permission denial becomes a model-visible tool result or terminal coordinator outcome as designed.
6. Tool execution error settles obligation and may continue model after barrier closes.
7. Duplicate identical results are idempotent.
8. Duplicate conflicting results fail clearly.
9. Late result after terminal run is ignored with structured reason.
10. Stale result with wrong step id is rejected or ignored.

### BearWire/armature tests

1. `client.waiting` without `obligation_id` is not rendered as permission UI.
2. `client.waiting` with wrong `expected_client_method` is not rendered as permission UI.
3. `run.paused` does not trigger permission UI or auto-approval.
4. Permission auto-approval for local tool yields local tool execution, then tool result, then one continuation.
5. File-not-found from local tool is posted as `client.tool.result(status=error)`.

### Stream tests

1. `RunPaused` terminates the current runtime stream segment.
2. Queued `client.waiting` events are emitted only after obligation persistence.
3. A provider response with multiple tool calls produces multiple obligations before continuation.

## Migration constraints

- Keep BearWire external compatibility during rollout.
- Do not reintroduce ACP types into Den core.
- Do not move local tool execution into Den; Den coordinates, armature executes.
- Avoid broad rewrites before Phase 1 barrier tests are in place.
- Prefer additive schema changes and compatibility fallbacks.

## Implementation status summary

Implemented in the 2026-07-02 refactor series:

- Core turn state persists under neutral `turn_*` names; `bearwire_events` remains the BearWire wire event log.
- `den_runtime::client_obligation_coordinator` owns client result idempotency, conflict detection, late-result handling, step barriers, local-tool dispatch decisions, and continuation readiness.
- `den_runtime::turn_waits` owns transactional wait persistence for BearWire tool waits and generic non-BearWire surface obligations.
- New active waits create/update a `turn_step`, attach obligations/results to `turn_step_id`, and reject stale/wrong-step results.
- BearWire handlers are thin adapters that authenticate/parse, call the coordinator, then project outcomes to BearWire responses/events.
- Non-BearWire surfaces can persist and project core obligations such as web-chat `human_input` without creating a separate state machine.
- Tests cover multi-tool barriers, tool errors as results, local/Den-hosted approval paths, denial, duplicates, late results, wrong-step results, transactional `client.waiting` IDs, non-BearWire surface projection, neutral schema names, typed states/ids, and full `den-bearwire` behavior.

## Success criteria

- No code path can continue the model while the current step has open client obligations.
- Permission approval for armature-local tools cannot directly continue the model.
- Multiple tool calls in one model step continue the model exactly once after all results settle.
- Tool execution errors are visible to the model as tool results.
- BearWire RPC methods are thin protocol adapters over the coordinator.
- ACP remains edge-only; core state machine is protocol-neutral.
