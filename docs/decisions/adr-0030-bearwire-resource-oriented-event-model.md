# ADR: BearWire resource-oriented event model

**Status:** Accepted (2026-08-16)  
**Date:** 2026-05-26  
**Deciders:** Hans

## Context

BearWire is the trusted runtime/control protocol between Den and BEARS-controlled edge runtimes such as ACP adapters, desktop companions, remote daemons, CI/devcontainer runners, and Reflection-related workers.

As BearWire evolves, two closely related design needs have become clear:

1. BearWire needs a stable semantic event vocabulary that feels idiomatic to engineers familiar with distributed control planes, job/run orchestration, and agent runtimes.
2. BearWire needs a generic resource-oriented model that can represent many protocol-addressable objects without baking historically specific concepts into the top-level event taxonomy.

Earlier BearWire thinking established event envelopes, capability negotiation, session lifecycle, tool callbacks, permission mediation, diagnostics, and replay/resume concerns. Separately, Den runtime work has also introduced structured semantic events for turn execution and continuation flows.

Those lines of work are directionally aligned, but the mapping between internal runtime semantics and BearWire wire events must be made more explicit and more idiomatic.

In particular, BearWire needs to distinguish cleanly between:

- execution lifecycle;
- streamed output lifecycle;
- delegated tool-call lifecycle;
- permission gates;
- typed resource binding and discovery; and
- transport/RPC errors versus normal runtime failure outcomes.

We also want BearWire vocabulary to generalize well beyond a single historically specific concept such as "work surface" while still supporting typed workspace-like bindings where needed.

## Decision

BearWire will adopt a **resource-oriented semantic event model**.

The model has three layers:

1. **semantic facts** — what happened in the system;
2. **wire events** — stable BearWire event types and payloads streamed over the protocol; and
3. **control methods** — JSON-RPC methods used to start, continue, cancel, inspect, acknowledge, or otherwise control work.

BearWire will use **resource** as the generic protocol abstraction for identifiable runtime objects and typed bindings.

BearWire event taxonomy will be organized around stable semantic domains rather than backend payload formats or UI labels:

- connection
- session
- run
- message
- tool_call
- permission
- resource
- diagnostic
- health
- version
- memory
- reflection

BearWire will prefer lifecycle-centered event names such as:

- `run.started`
- `run.paused`
- `message.delta`
- `tool_call.blocked`
- `resource.bound`

rather than preserving narrowly scoped or transport-shaped names such as raw backend event documents or one-off status labels.

## Rationale

### Distributed-systems idiom

Control-plane and remote-runtime protocols are easier to understand and extend when they model:

- identifiable resources;
- lifecycle transitions;
- explicit pause/resume/cancel semantics;
- clear separation between execution state and output streams; and
- structured distinction between transport failures and domain failures.

This makes BearWire feel familiar to engineers who know RPC systems, schedulers, workflow engines, debuggers, and orchestration runtimes.

### Agent-runtime idiom

Agent systems naturally distinguish between:

- a bounded execution attempt;
- streamed assistant output;
- delegated tool use;
- human approval gates; and
- resumable execution.

The BearWire model should preserve those distinctions directly.

### Generalized resource model

A generic `resource` namespace lets BearWire represent:

- workspace-like contexts;
- repositories;
- bound runtime targets;
- future durable artifacts;
- permission requests;
- other typed objects

without forcing the event taxonomy to be redesigned every time a new object class becomes important.

### Better continuity between internal semantics and wire semantics

Den may continue to use typed internal runtime event representations. BearWire should act as the stable wire projection of those semantic facts, not as a thin wrapper around backend transport residue.

## Event model

### Core semantic domains

BearWire event taxonomy is organized around these semantic domains:

- `connection`
- `session`
- `run`
- `message`
- `tool_call`
- `permission`
- `resource`
- `diagnostic`
- `health`
- `version`
- `memory`
- `reflection`

### Resource-oriented identity

BearWire should use typed resource identity wherever generic protocol object identity is needed.

Example:

```json
{
  "resource": {
    "kind": "workspace",
    "id": "repo_123",
    "uri": "git+https://github.com/example/project",
    "display_name": "example/project"
  }
}
```

Recommended resource fields:

- `kind`
- `id`
- `uri` (optional)
- `display_name` (optional)
- `version` (optional)
- `metadata` (optional, bounded)

### Event envelope

BearWire events continue to use a common JSON-RPC notification envelope with a stable `type` and event-specific `data`.

The envelope should support resource-oriented subjects such as:

```text
resource/session/ses_123
resource/run/run_123
resource/message/msg_123
resource/tool_call/tc_123
resource/workspace/repo_123
resource/permission_request/perm_123
```

Shorter forms may also be accepted if used consistently.

## Canonical event taxonomy

### Connection lifecycle

- `connection.opened`
- `connection.capabilities`
- `connection.heartbeat`
- `connection.warning`
- `connection.closing`
- `connection.lost`

### Session lifecycle

- `session.opened`
- `session.bound`
- `session.resumed`
- `session.state`
- `session.closed`
- `session.invalidated`

### Run lifecycle

A **run** is one bounded execution attempt, such as a prompt turn, continuation step, reflection run, or comparable unit of work.

- `run.accepted`
- `run.started`
- `run.progress`
- `run.paused`
- `run.resumed`
- `run.completed`
- `run.failed`
- `run.cancelled`
- `run.expired`
- `run.warning`

### Message lifecycle

- `message.started`
- `message.delta`
- `message.part`
- `message.completed`
- `message.aborted`

### Tool-call lifecycle

- `tool_call.requested`
- `tool_call.dispatched`
- `tool_call.blocked`
- `tool_call.started`
- `tool_call.progress`
- `tool_call.completed`
- `tool_call.failed`
- `tool_call.cancelled`
- `tool_call.warning`

`tool_call.blocked` should use payload reasons such as:

- `permission_required`
- `missing_capability`
- `policy_hold`
- `quota_hold`
- `awaiting_runtime`

### Permission lifecycle

- `permission.requested`
- `permission.granted`
- `permission.denied`
- `permission.expired`
- `permission.revoked`

### Resource lifecycle

The `resource.*` family is the generic BearWire mechanism for typed object detection, binding, and update.

- `resource.detected`
- `resource.bound`
- `resource.updated`
- `resource.unbound`
- `resource.rejected`

These events must indicate the resource kind explicitly.

### Diagnostics and governance

- `diagnostic.reported`
- `health.reported`
- `version.reported`
- `memory.review_requested`
- `memory.write_recorded`
- `reflection.run_started`
- `reflection.run_completed`
- `reflection.proposal_created`

## Persistence and UI projection policy

Event meaning, live delivery, persistence, and UI projection are independent decisions. An event does not need durable storage merely because a live client needs it, and persistence does not imply display in every UI.

The initial projection audiences are:

- **livestream** — watching execution; may include transient placeholders that explain current activity;
- **review** — reading a run's result; the most concise, outcome-oriented projection;
- **audit** — examining behavior and performance; the most detailed human projection, but still redacted rather than a raw provider or secret-bearing log.

Matrix values: **D** = durable; **C** = durable but compactable after the replay/live window; **E** = ephemeral; **separate** = persist the underlying resource/obligation rather than this notification. Projection values are **yes**, **summary**, **current** (only while unresolved), **conditional**, and **no**.

The JSON specification remains the payload-level inventory. Adding a canonical event requires updating this matrix in the same change.

### Implementation inventory (2026-08-16)

This matrix is the policy target; implementation is deliberately staged rather than implied by acceptance:

| Policy path | Current implementation | Follow-up |
| --- | --- | --- |
| Durable BearWire replay | `den_runtime::bearwire_events::append_bearwire_event`, consumed by the HTTP event-page endpoint | Filter/coalesce remaining compactable rows at their producer or projection boundary. |
| `client.waiting` (`separate`) | `persist_runtime_event_as_bearwire` first persists the authoritative `turn_obligation`, then publishes a safe `client.waiting` observation to `DenState`'s process-local fan-out; it is not appended to the durable sequence. | Expose the reconnect snapshot and subscriber transport in the livestream projection task. |
| `run.progress` (`E`) | `persist_run_progress` records operational telemetry only; it does not append an event. `publish_run_progress` is the safe fan-out helper, but no producer calls it yet. | Wire producers to the helper while implementing the livestream projection. |
| Review and audit | Existing event-page/review consumers still render durable event rows directly. | Implement normalized audience-specific projections in their dedicated tasks. |

### Ephemeral livestream delivery and reconnect

Events classified **E** are not inserted into the durable BearWire event sequence. Den delivers them through a bounded, process-local, best-effort livestream fan-out. The fan-out is deliberately not a second event log: it has no replay cursor, does not survive a Den process restart, and may drop events for a slow or disconnected subscriber. A later ephemeral update must therefore be sufficient to replace an earlier one.

A livestream subscriber first receives a derived current snapshot, then best-effort ephemeral updates and durable events as they occur. On reconnect it must reconstruct its view from durable run/session state and unresolved authoritative resources, rather than expecting transient history to be replayed. The snapshot includes the active run state, unresolved client obligations, and any other current values required by the livestream projection; it does not invent a history of prior progress placeholders.

`client.waiting` is a live notification derived from a durable client obligation. The obligation, not the notification, is the source of truth for reconnect, timeout, and authorization validation. A reconnect snapshot exposes unresolved obligations so a client can recover an actionable prompt without a replayed `client.waiting` event.

### Connection and session

| Event | Persistence and retention | Livestream | Review | Audit |
| --- | --- | --- | --- | --- |
| `connection.opened` | D; bounded operational retention | no | no | summary |
| `connection.capabilities` | C; latest snapshot | no | no | summary |
| `connection.heartbeat` | E; metrics only | no | no | no |
| `connection.warning` | D; bounded | conditional | conditional | yes |
| `connection.closing` | E; superseded by terminal state | current | no | summary |
| `connection.lost` | D when it affects a session/run | yes | conditional | yes |
| `session.opened` | D | no | no | summary |
| `session.bound` | D | summary | conditional | yes |
| `session.resumed` | D | summary | no | yes |
| `session.state` | C; latest snapshot, never poll-spam | current | no | summary |
| `runtime.objective_orientation` | D; latest is current | conditional | conditional | yes |
| `model.selection.changed` | D; latest is current | summary | conditional | yes |
| `session.metadata.updated` | C by metadata key | conditional | conditional | summary |
| `session.closed` | D; terminal | summary | no | yes |
| `session.invalidated` | D; terminal | yes | conditional | yes |

### Run and work progress

| Event | Persistence and retention | Livestream | Review | Audit |
| --- | --- | --- | --- | --- |
| `run.accepted` | D | current | no | yes |
| `run.started` | D | yes | no | yes |
| `run.progress` (`status_text`, `phase`) | E; replace current display | current | no | conditional |
| `run.progress` (`queue`, `heartbeat`) | E; current state/metrics only | current | no | no |
| `run.paused` | D; resolved by resume/terminal | current | conditional | yes |
| `run.resumed` | D | summary | no | yes |
| `run.completed` | D; terminal | yes | yes | yes |
| `run.failed` | D; terminal | yes | yes | yes |
| `run.cancelled` | D; terminal | yes | yes | yes |
| `run.expired` | D; terminal | yes | yes | yes |
| `run.warning` | D; compact exact duplicates | conditional | conditional | yes |
| `work.progress.updated` | C; latest per work item | yes | summary | yes |

`run.progress` is deliberately non-durable. If an observation is required to explain a failure or performance problem, emit a typed warning, diagnostic, or lifecycle fact instead of retaining periodic status text.

### Messages

| Event | Persistence and retention | Livestream | Review | Audit |
| --- | --- | --- | --- | --- |
| `message.started` | C; compact into terminal message | current | no | summary |
| `message.delta` | C; coalesce into completed content | yes | no | summary |
| `message.reasoning.delta` | C; coalesce and bound | conditional | no | summary |
| `message.part` | D; structured content | yes | yes | yes |
| `message.completed` | D; supersedes deltas for review | yes | yes | yes |
| `message.aborted` | D; terminal | yes | conditional | yes |

Review renders an assembled message once, rather than displaying its deltas, parts, and completion separately. Reasoning projection is limited to provider-exposed reasoning permitted by visibility policy; hidden chain-of-thought and private scratchpad content are never projected.

### Tools, client obligations, and permissions

| Event | Persistence and retention | Livestream | Review | Audit |
| --- | --- | --- | --- | --- |
| `tool_call.requested` | D; replayable call record | summary | summary | yes |
| `tool_call.dispatched` | D; timing/target evidence | current | no | yes |
| `client.waiting` | separate; obligation is authoritative | current | no | summary |
| `tool_call.blocked` | D for exceptional blockers; not duplicate permission waits | current | conditional | yes |
| `tool_call.started` | D; timing evidence | current | no | yes |
| `tool_call.progress` | E; replace current display | current | no | conditional |
| `tool_call.completed` | D; terminal | summary | summary | yes |
| `tool_call.failed` | D; terminal | yes | summary | yes |
| `tool_call.cancelled` | D; terminal | summary | conditional | yes |
| `tool_call.warning` | D; compact exact duplicates | conditional | conditional | yes |
| `permission.requested` | D; decision context | current | conditional | yes |
| `permission.granted` | D; resolves request | summary | conditional | yes |
| `permission.denied` | D; resolves request | yes | conditional | yes |
| `permission.expired` | D; resolves request | yes | conditional | yes |
| `permission.revoked` | D; governance transition | yes | conditional | yes |

`client.waiting` is live delivery of an already durable obligation. It must not create a second durable history fact that merely says the client is waiting. Audit derives the wait and duration from the obligation and its resolution. Permission-related `tool_call.blocked` is a legacy projection and must not duplicate that wait in new streams.

### Resources, diagnostics, and governance

| Event | Persistence and retention | Livestream | Review | Audit |
| --- | --- | --- | --- | --- |
| `resource.detected` | C; compact into binding/latest state | conditional | no | summary |
| `resource.bound` | D; binding used by run | summary | conditional | yes |
| `resource.updated` | C; latest per resource/version | conditional | conditional | summary |
| `resource.unbound` | D | conditional | conditional | yes |
| `resource.rejected` | D | yes | conditional | yes |
| `diagnostic.reported` | D; bounded by severity/policy | conditional | conditional | yes |
| `health.reported` | C; latest snapshot/metric aggregate | no | no | conditional |
| `version.reported` | C; latest per component/run | no | no | summary |
| `memory.review_requested` | D; request and disposition | conditional | conditional | yes |
| `memory.write_recorded` | D; governance evidence, never secret content | conditional | conditional | yes |
| `reflection.run_started` | D | summary | no | yes |
| `reflection.run_completed` | D; terminal | summary | conditional | yes |
| `reflection.proposal_created` | D; proposal reference | yes | summary | yes |

### Cross-cutting rules

1. **Review is outcome-oriented.** Assemble messages, pair tool requests with outcomes, omit resolved placeholders, and collapse repeated lifecycle detail. Material failures and user decisions remain visible.
2. **Livestream placeholders are replaceable.** Queue, progress, dispatch, waiting, and partial-message displays may live in client state without becoming durable history.
3. **Audit is detailed, not raw.** Preserve identity, ordering, timing, correlation, lifecycle, warnings, decisions, and bounded summaries. Raw tool arguments/results, provider payloads, credentials, and hidden reasoning require separately authorized diagnostics.
4. **Durable facts are immutable.** Supersession controls projection and compaction; it must not rewrite terminal outcomes or decision evidence so as to change history.
5. **Current status is derived.** Waiting duration, active phase, and similar status come from unresolved obligations and lifecycle timestamps, not periodic persisted observations.
6. **Projection is correlated.** Pair starts with outcomes and requests with decisions by stable identifiers instead of rendering every event as an unrelated chat message.
7. **Security applies before projection.** Visibility and redaction are evaluated per audience before content leaves the server. Persistence is not authorization to display.

## Semantic mapping guidance

The following semantic mappings should guide BearWire projections:

| Semantic fact | BearWire event type | Notes |
| --- | --- | --- |
| assistant text delta | `message.delta` | Canonical streamed text output event. |
| status text update | `run.progress` | Use payload kind such as `status_text`. |
| tool call requested | `tool_call.requested` | Initial delegated-capability request. |
| generic runtime issue | classify before projection | Prefer `run.warning`, `run.failed`, `tool_call.failed`, `diagnostic.reported`, or RPC `error`. |
| backing context resolved | `session.bound` or `resource.bound` | Use `session.bound` for interactive session binding; use `resource.bound` when the typed resource is the focus. |
| waiting for continuation | `run.paused` | Use payload reason `awaiting_continuation`. |
| turn completed | `run.completed` | Canonical terminal success event. |
| turn failed | `run.failed` | Canonical terminal failure event. |
| turn cancelled | `run.cancelled` | Canonical terminal cancellation event. |

## Continuation semantics

Continuation should be modeled as normal execution lifecycle state.

When a run is waiting on an external action, BearWire should use `run.paused` with an explicit reason, for example:

- `awaiting_continuation`
- `awaiting_input`
- `awaiting_approval`
- `awaiting_resource_binding`

When execution resumes, BearWire should use `run.resumed`.

This keeps BearWire aligned with common workflow and remote-runtime patterns:

- started
- progress
- paused
- resumed
- completed / failed / cancelled / expired

## Error model

BearWire must distinguish between:

1. **RPC/transport errors** — represented as JSON-RPC error responses;
2. **streamed warnings and diagnostics** — represented as events such as `run.warning`, `tool_call.warning`, or `diagnostic.reported`; and
3. **terminal execution failures** — represented as lifecycle events such as `run.failed` or `tool_call.failed`.

BearWire should avoid relying on a generic top-level streamed `error` event as the main runtime taxonomy.

## Control method alignment

BearWire control methods should align with the semantic domains above.

### Connection methods

```text
initialize
shutdown
heartbeat
```

### Session methods

```text
session.open
session.resume
session.close
session.state
```

### Run methods

A run-oriented naming model is preferred where practical:

```text
run.start
run.cancel
run.state
run.timeline
```

If transitional compatibility requires a more generic operation-oriented method family, the semantic model should still treat those methods as run lifecycle controls.

### Event transport methods

```text
event
 event.ack
 event.replay
```

### Tool and permission methods

```text
client.tool.result
client.permission.result
```

### Resource methods

```text
resource.register
resource.update
resource.unregister
resource.bind
resource.reject
```

### Diagnostics and governance methods

```text
diagnostic.report
health.check
version.report
memory.review_requested
reflection.run_requested
```

## Consequences

### Positive

- Gives BearWire a more idiomatic and extensible semantic vocabulary.
- Separates execution state from output state and tool state more clearly.
- Creates a generic resource model that can support workspace-like bindings without hard-coding one legacy concept into the protocol surface.
- Improves continuity between Den internal semantic events and BearWire wire events.
- Makes continuation, pause/resume, and blocking semantics easier to reason about.
- Produces a more legible basis for future JSON and Rust BearWire specs.

### Trade-offs

- Introduces a more deliberate semantic layer that must be documented and kept coherent.
- Requires migration guidance from older BearWire terminology and any operation-first naming already in flight.
- Generic `resource.*` events can be too abstract if payload schemas are not kept precise.

## Non-goals

- Do not collapse all domains into a single undifferentiated `resource.*` stream.
- Do not treat backend/provider event shapes as the canonical BearWire vocabulary.
- Do not use generic streamed `error` as the sole model for warnings, transport failures, and terminal run failures.
- Do not remove typed workspace-like bindings; instead represent them as typed resources.

## Follow-on documents

The resource-oriented event model defined by this ADR should be reflected in:

- a JSON-based BearWire protocol document describing event envelopes, payload schemas, and method families; and
- a Rust-based BearWire design document describing type models, enums, identifiers, and projection rules; and
- [ADR-0034: BearWire as the Den ↔ armature wire](adr-0034-bearwire-as-den-armature-wire.md) plus [implementation plan](../roadmap/BEARWIRE_ARMATURE_WIRE_IMPLEMENTATION_PLAN.md) for shipping the wire on Den and armatures.

## Related documents

- [ADR-0007: BearWire protocol](adr-0007-bearwire-protocol.md)
- [ADR-0029: Den structured runtime events](adr-0029-den-structured-runtime-events.md)
- [ADR-0024: terminology: actuators, resources, and role names](adr-0024-terminology-actuators-resources-and-role-names.md)
- [Den conversation runtime schema](../architecture/den-conversation-runtime-schema.md)
- [Reflection run taxonomy](../architecture/reflection-run-taxonomy.md)
