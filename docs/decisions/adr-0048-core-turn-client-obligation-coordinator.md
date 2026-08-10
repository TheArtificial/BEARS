# ADR-0048: Core turn/client-obligation coordinator

**Status:** Proposed  
**Date:** 2026-07-02  
**Deciders:** Hans

**Related:**

- [ADR-0034: BearWire as the Den ↔ armature wire](adr-0034-bearwire-as-den-armature-wire.md)
- [ADR-0043: ACP Is an Edge Adapter; the Den Runtime Is Protocol-Agnostic](adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)
- [ADR-0044: Runtime stream state machines must make progress explicit](adr-0044-runtime-stream-wake-invariant.md)
- [BearWire JSON specification](../architecture/bearwire-json-spec.md)
- [BearWire turn coordinator refactor plan](../roadmap/BEARWIRE_TURN_COORDINATOR_REFACTOR_PLAN.md)
- [Armature tool-obligation leasing implementation plan](../roadmap/ARMATURE_TOOL_OBLIGATION_LEASING_PLAN.md)
- [Den state machine inventory](../architecture/den-state-machine-inventory.md)

> **State-inventory maintenance.** This ADR owns turn phases, run lifecycle waits, obligation kinds/states, and late-result handling in the Den state machine inventory. Changes to any wait/settlement/continuation transition must update that inventory and add/update coordinator tests for legal and illegal transitions.

## Context

ADR-0043 correctly moved ACP-specific semantics to the edge: ACP framing, Zed permission UI, local editor tool execution, and ACP projection belong in the armature/adapter, not in Den core.

Recent BearWire permission/tool failures exposed a second boundary that must not move to the edge: **turn and client-obligation coordination**. This coordination is not ACP-specific. It is core Den runtime semantics.

Observed failure modes included:

- permission approval for an armature-local tool caused model continuation before a tool result existed;
- a single model step produced multiple client tool waits, and individual results could independently start continuations;
- stale `run.paused` events appeared after permission/tool settlement;
- tool execution errors such as `Resource not found` were mixed into confusing run failure/loop behavior;
- repeated permission auto-approval and continuation caused the runtime to hit `native agent loop reached max steps`.

These failures show that BearWire RPC handlers, runtime stream projection, obligation rows, and armature permission handling were collectively acting as an implicit state machine. The implicit state machine was fragmented and therefore permitted illegal transitions.

## Decision

Den will restore a protocol-neutral **core turn/client-obligation coordinator** below BearWire and below every channel/armature projection. BearWire remains the Den ↔ armature wire, and ACP remains an edge/client protocol, but neither BearWire nor the armature owns model-continuation decisions. The same coordinator must also serve non-BearWire surfaces such as web chat, Slack, and future macOS/channel adapters whenever a turn waits on human input, approval, resource binding, handoff, or tool results.

### 1. Core owns turn/tool/approval continuation semantics

The core coordinator owns:

- run lifecycle;
- model step lifecycle;
- client-obligation lifecycle;
- permission-result settlement;
- tool-result settlement;
- stale/duplicate result handling;
- batching of tool calls from one model step;
- the rule for whether model continuation is legal.

These are not ACP semantics and must not live in the ACP armature. They are also not BearWire transport semantics and must not be scattered across BearWire RPC methods.

### 2. BearWire is transport/projection, not the continuation state machine

BearWire methods such as `client.permission.result` and `client.tool.result` are one projection of coordinator inputs. Web chat actions, Slack button/reply callbacks, macOS app actions, or future channel-local actions should feed the same coordinator through their own projection. None of these surfaces independently decide to continue the model.

Required shape:

```rust
let outcome = turn_coordinator.record_client_result(...).await?;

match outcome {
    WaitingForMoreClientResults => ok_waiting(),
    DispatchLocalTool(request) => local_tool_request(request),
    ContinueModel(request) => spawn_continuation_task(request),
    IgnoredLateResult => late_result_ignored(),
    Failed(error) => run_failed(error),
}
```

Only the coordinator may produce `ContinueModel`.

### 3. Model continuation requires a closed obligation barrier

A model continuation may start only when the current model step has no open turn obligations for the required responder/surface. Tool-result obligations are the immediate pressure point, but the same barrier applies to permission decisions, human-input waits, resource-binding waits, handoff decisions, and future obligation kinds.

Open obligations include:

- waiting for permission;
- permission granted but waiting for armature-local tool execution;
- waiting for tool result;
- any non-terminal client wait for the current step.

Terminal/settled obligations include:

- tool result received;
- permission denied and converted into a tool/result message;
- failed/cancelled obligations that have been intentionally represented to the model or terminal run state.

### 4. Permission approval is not tool execution

For armature-local tools:

```text
permission approved → dispatch local tool → wait for tool result → continue model after step barrier closes
```

It is invalid for an armature-local permission approval to directly continue the model.

For Den-hosted tools, Den may execute the approved tool internally as part of the same coordinator transition, but the coordinator must still settle the obligation before continuation.

### 5. Model steps are batches

A model step may emit multiple tool calls. Continuation happens once per step, after all obligations created by that step are settled. Individual client results must not independently continue the model if sibling obligations remain open.

### 6. Actionable waits are obligations

The core concept is a protocol-neutral turn obligation. Each surface projects it into its own actionable shape. The canonical BearWire event for an armature-actionable wait is `client.waiting`, carrying:

- `data.obligation_id`;
- `data.expected_client_method`;
- nested `data.tool_call`;
- nested `data.permission` when permission is required.

`run.paused` is non-actionable status/diagnostic state. It must not drive permission UI or tool dispatch.

### 7. Tool errors are normal tool results

A local tool execution error, such as file-not-found, should settle the tool-result obligation with error status and be supplied to the model as a tool result. It should not be treated as infrastructure failure unless the BearWire/armature protocol itself failed.

### 8. Step/generation fencing is required

Turn obligations and obligation results must be fenced by durable identity. The target end-state includes a model-step identity, such as `turn_step_id`, so stale events/results from earlier steps cannot trigger continuation of a later step.

At minimum, all transitions must be scoped by:

```text
run_id + session_id + obligation_id + tool_call_id/permission_id
```

The target state adds:

```text
turn_step_id
```

### 9. Tool activity must be replayable as model context

Every tool request/result that can affect model behavior must be persisted as a first-class transcript artifact before the run is considered safely resumable or complete. The stored representation must be sufficient to reconstruct both:

- **the model replay view**: assistant tool call part with stable `tool_call_id`, canonical `tool_name`, and typed arguments; followed by a matching tool result part with status, structured result/error, and bounded text output;
- **the surface projection view**: human-friendly title, visible input summary, visible output/error summary, and enough raw/structured detail for ACP, web, Slack, or future clients to explain what happened.

A tool result is not replayable if it only says `Tool completed`, omits the arguments, omits the matching tool call, hides the error as generic text, or exists only in an armature-local cache. Edge adapters may cache richer UI detail temporarily, but durable Den transcript state is the source of truth for future model turns.

Failed or max-step turns are not exempt. If a tool call/result happened before terminal failure, it must remain visible to later model context and operator/debug UI. Terminal run state may be stored separately, but it must not cause the user prompt, assistant tool call, or tool result records from that turn to disappear.

### 10. Armature-local tool execution uses durable fenced leases

An armature must claim an open `ToolResult` obligation before starting local execution. A successful claim moves the obligation from waiting to claimed/running and returns an opaque attempt token, a server-clock lease deadline, and a server-selected renewal interval. Only that claimant may renew the lease or submit the result.

Every claim, renewal, result, expiry, and cancellation transition must match the authenticated responder and the complete durable identity:

```text
run_id + session_id + obligation_id + tool_call_id + attempt_token + open state
```

`turn_step_id` is also required once available. Claim is transactional: concurrent armatures cannot both acquire permission to execute. Renewal is idempotent, uses Den's database clock, and cannot revive an expired, cancelled, superseded, or settled obligation. Result, expiry, cancellation, and renewal use conditional transitions so exactly one canonical transition wins.

Lease expiry after execution was claimed means Den can no longer establish whether the local command is still running or already changed the workspace. The coordinator must settle the obligation and run as `outcome_unknown`, persist recovery evidence, and prohibit automatic re-execution. A reconnecting armature inspects `run.state`; possession of stale session state without the current attempt token grants inspection but not renewal or execution authority.

Lease ownership is core Den state. BearWire and other surfaces only transport claim, renewal, and result inputs. Process-local task registries may suppress duplicate work as an optimization, but they are not execution authority.

## Rationale

ACP-specific code was correctly pushed to the edge, but part of the old consolidated turn/tool state machine appears to have been lost or bypassed during the migration. The fix is not to move ACP semantics back into Den, and it is not to make BearWire the new center. The fix is to make the turn/obligation state machine explicitly core and protocol-neutral again, with BearWire, web chat, Slack, macOS, and future channels/armatures as projections over it.

The coordinator preserves the architectural separation:

```text
Den runtime core
  owns turn/step/obligation semantics and continuation barriers

Surface projections
  BearWire for trusted armatures
  web chat / Slack / macOS channel actions for human input, approval, resource binding, and channel-local interactions

Armatures / channel adapters
  own local execution, protocol projection, permission UI, channel UI, and local caches
```

## Consequences

### Positive

- Prevents model-continuation loops caused by permission approval without tool results.
- Prevents parallel continuations from sibling tool results in the same model step.
- Makes stale/duplicate/late client results a core state-machine concern.
- Makes BearWire easier to reason about because it becomes an input/output transport around the coordinator.
- Allows web chat, Slack, macOS, and future channels to satisfy the same core obligation model without inheriting ACP/BearWire semantics.
- Preserves ADR-0043: ACP remains edge-only while core turn coordination remains Den-owned.

### Costs

- Requires refactoring BearWire RPC methods away from direct `spawn_continuation_task` calls.
- Requires a new or restored core coordinator module and likely schema additions for model steps.
- Requires compatibility handling while existing BearWire-backed obligations lack `turn_step_id`.
- Requires stronger integration tests around multi-tool model steps and permission/tool-result barriers.

## Non-goals

- Do not move ACP UI/framing back into Den core.
- Do not make BearWire an ACP-shaped protocol.
- Do not require any surface to understand model-step batching beyond answering explicit obligations.
- Do not treat all tool execution errors as run infrastructure failures.

## Review checklist

When modifying turn, permission, or tool-result paths:

1. Does this code directly start model continuation? If yes, why is it in the coordinator?
2. Can the current model step have any open client obligations? If yes, continuation is illegal.
3. Is permission approval being confused with tool execution?
4. Are tool errors recorded as tool results rather than crashes?
5. Are actionable waits represented as surface projections of a core obligation (`client.waiting` for BearWire)?
6. Is `run.paused` treated as non-actionable status?
7. Are duplicate/late/stale obligation results fenced by durable IDs?
8. Is behavior tested with multiple tool calls or obligations in one model step?
9. Can more than one armature claim and execute the same obligation?
10. Can a stale claimant renew a lease or submit a result?
11. Do result, renewal, cancellation, and expiry races have one canonical winner?
12. Can any recovery path automatically re-execute a command whose outcome is unknown? It must not.
