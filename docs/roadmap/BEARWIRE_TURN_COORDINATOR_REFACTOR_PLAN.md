# BearWire turn coordinator refactor plan

Status: draft  
Date: 2026-07-02  
Related ADR: [ADR-0048: Core turn/client-obligation coordinator](../decisions/adr-0048-core-turn-client-obligation-coordinator.md)

## Goal

Restore a consolidated, protocol-neutral Den turn/client-obligation state machine after the ACP-to-edge migration. BearWire remains the Den ↔ armature wire, and ACP remains edge-only, but model-continuation decisions move behind a core coordinator.

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
  owns run state, step state, client obligations, settlement, continuation barriers

BearWire
  owns wire methods/events: client.waiting, client.permission.result, client.tool.result

Armature / ACP adapter
  owns ACP projection, permission UI, local tool execution, local permission cache
```

## Core invariants

1. **Only the coordinator may start model continuation.** BearWire RPC methods record inputs and receive coordinator outcomes.
2. **Continuation requires a closed step barrier.** A model continuation may start only when the current model step has no open client obligations.
3. **Permission approval is not tool execution.** For armature-local tools, approval transitions to local tool dispatch, not model continuation.
4. **A model step is a batch.** Multiple tool calls from one model step settle together; continuation happens once per step.
5. **Actionable waits are obligations.** `client.waiting` must carry `obligation_id` and `expected_client_method`; `run.paused` is non-actionable status.
6. **Tool errors are normal tool results.** File-not-found and similar local failures settle tool-result obligations and are shown to the model.
7. **Durable IDs fence results.** Results are scoped by run/session/obligation/tool/permission, and the target state adds `run_step_id`.
8. **Events follow persistence.** Den must persist obligations before streaming answerable wait events.

## Target model

### Tables / persisted concepts

```text
bearwire_runs
  run_id
  session_id
  state

bearwire_run_steps
  id
  run_id
  step_index
  state
  provider_response_id
  opened_at
  closed_at

bearwire_run_obligations
  id
  run_id
  step_id
  session_id
  kind
  expected_client_method
  tool_call_id
  permission_id
  state
  request_payload
  result_payload

bearwire_client_results
  id
  run_id
  step_id
  obligation_id
  kind
  target_id
  payload_hash
  payload
```

### Run states

```text
accepted
running_model
waiting_for_client
continuing_model
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
waiting_for_permission
permission_granted
permission_denied
waiting_for_tool_result
tool_result_received
continued
failed
cancelled
```

The existing `requested`, `waiting_for_client`, `result_received`, and `continued` values can remain during migration, but the coordinator should expose semantic states internally.

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

Status: partially in progress.

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
open_obligations_for_step(run_id, step_id) // after step ids exist
has_open_obligations_for_run(run_id)
```

Patch `client.tool.result`:

1. record tool result;
2. mark obligation result received;
3. check sibling open obligations;
4. if any remain, return `WaitingForMoreClientResults` and do not continue;
5. if none remain, continue once.

This phase may use run-level barriers before `step_id` exists.

Done when:

- One model step with two armature-local tool calls does not continue after the first result.
- Continuation starts only after both results are recorded.

### Phase 2: extract coordinator facade

Status: partially implemented (2026-07-02). `den_runtime::client_obligation_coordinator` now owns the tool-result and permission-result outcome decisions for the current run-level barrier. BearWire still performs transport/auth/result projection and starts continuation tasks from coordinator outcomes.

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

Status: additive schema baseline added (2026-07-02). `bearwire_run_steps` and nullable `step_id` columns are available; follow-up work must populate step ids for new runs and switch barriers from run-level to step-level.

Add schema:

```text
bearwire_run_steps
bearwire_run_obligations.step_id
bearwire_client_results.step_id
```

Migration strategy:

- Add nullable `step_id` first.
- New runs write step rows and obligation step ids.
- Existing rows remain valid via run-level compatibility fallback.
- Later make `step_id` required for new active runs.

Done when:

- Every new client obligation belongs to a `run_step`.
- Continuation barriers check step-level obligations, not all run-level obligations.

### Phase 4: transactional obligation/event outbox

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

- Stop using `tool_call.blocked` for active permission flow.
- Keep `tool_call.blocked` only as legacy/replay compatibility if still needed.
- Armatures render permission UI only for `client.waiting` with valid `obligation_id` and `expected_client_method`.
- Remove direct handling of `run.paused` as tool activity.

Done when:

- No active armature permission path depends on `run.paused` or `tool_call.blocked`.

### Phase 6: tighten types and state enums

Replace stringly internal control state with typed values:

- `RunState`
- `RunStepState`
- `ClientObligationState`
- `ExpectedClientMethod`
- `ClientObligationKind`
- typed IDs for run, step, obligation, tool call, permission, session.

Done when:

- Illegal state transitions are compile-time-visible or centralized in the coordinator.
- BearWire methods do not compare raw method/status strings outside boundary parsing.

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

## Success criteria

- No code path can continue the model while the current step has open client obligations.
- Permission approval for armature-local tools cannot directly continue the model.
- Multiple tool calls in one model step continue the model exactly once after all results settle.
- Tool execution errors are visible to the model as tool results.
- BearWire RPC methods are thin protocol adapters over the coordinator.
- ACP remains edge-only; core state machine is protocol-neutral.
