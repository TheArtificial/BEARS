# BearWire v1 protocol refinement roadmap

**Status:** Draft  
**Date:** 2026-06-22  
**Related:** [BearWire armature wire plan](BEARWIRE_ARMATURE_WIRE_IMPLEMENTATION_PLAN.md), [BearWire JSON specification](../architecture/bearwire-json-spec.md), [ADR-0034: BearWire as the Den ↔ armature wire](../decisions/adr-0034-bearwire-as-den-armature-wire.md), [ADR-0043: ACP Is an Edge Adapter; the Den Runtime Is Protocol-Agnostic](../decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)

## Purpose

Pressure-test the BearWire v1 event grammar before it hardens as the Den ↔ armature wire.

Research into AG-UI highlighted that BearWire is directionally right for trusted armatures, but some BearWire v1 names and structures should be sharper:

- run lifecycle boundaries should be invariant-driven;
- messages should identify role/kind at start;
- mutable resources should use explicit snapshot/delta semantics;
- run progress, frontend-visible activity, and diagnostics should be separate;
- Den's persisted run obligations should have first-class wire events;
- client result method names should consistently reflect directionality;
- legacy `role` / `role_agent_id` vocabulary should move toward Bear stance / runtime binding vocabulary.

This roadmap records protocol refinements to make before BearWire v1 becomes the only armature wire.

## Non-goals

- Do not replace BearWire with AG-UI.
- Do not make AG-UI the Den runtime's canonical event model.
- Do not change ACP's external contract to Zed as part of this roadmap.
- Do not introduce channel-local or web-chat assumptions into armature semantics.
- Do not add scattered tool-name routing. Tool execution location remains descriptor-owned.

## Design stance

BearWire remains Den-specific and armature-first:

```text
Den runtime semantic events
        │
        ├─ BearWire projection -> trusted armatures -> ACP/Zed/local tools
        ├─ AG-UI projection    -> rich user-facing channels/apps
        └─ Channel renderers   -> Slack/WhatsApp/webhook/etc.
```

AG-UI is useful as a vocabulary pressure test, especially around run lifecycle, message streaming, state snapshot/delta, activity messages, and interrupts. BearWire should borrow the useful grammar while keeping Den's stronger authority model for local tools, permissions, obligations, sessions, and resources.

## Target v1 invariants

### Run lifecycle

Every non-rejected run follows one of these logical paths:

```text
run.accepted? -> run.started -> run.completed
run.accepted? -> run.started -> run.failed
run.accepted? -> run.started -> run.cancelled
run.accepted? -> run.started -> run.expired
run.accepted? -> run.started -> run.paused -> run.resumed -> terminal
```

Rules:

- `run.accepted` is optional only if Den starts execution synchronously enough that `run.started` is the first observable run event.
- If emitted, `run.accepted` means Den accepted/enqueued the run request.
- `run.started` means execution actually began.
- Exactly one terminal outcome is emitted for each started run.
- Terminal events are `run.completed`, `run.failed`, `run.cancelled`, and `run.expired`.
- `run.warning` is non-terminal.

### Message lifecycle

Text-bearing messages follow:

```text
message.started -> message.delta* -> message.completed
message.started -> message.delta* -> message.aborted
```

Rules:

- `message.started` carries `role` and `message_kind`.
- `message.delta` is only for text deltas for the matching message.
- Non-text structured additions should use `message.part`, `activity.*`, or `resource.*`, not overload text deltas.

### Obligation lifecycle

Any run state that requires armature action should be represented by a durable obligation:

```text
obligation.opened -> obligation.resolved
obligation.opened -> obligation.cancelled
obligation.opened -> obligation.expired
obligation.opened -> obligation.failed
```

Rules:

- `bearwire_run_obligations` remains Den's source of truth.
- Wire events reflect persisted obligation state, not transient in-memory waits.
- Client result methods validate `(session_id, run_id, obligation_id, expected_method)` before settling.
- Duplicate replay of the same result is deterministic.
- Conflicting duplicate results are rejected.

### Resource mutation lifecycle

Mutable resources use snapshot/delta semantics:

```text
resource.snapshot -> resource.delta*
```

Rules:

- `resource.snapshot` replaces the client's model for that resource.
- `resource.delta` carries RFC 6902 JSON Patch operations where practical.
- Domain-specific `changes` objects should be avoided unless the patch form is impossible or intentionally not JSON-shaped.

## Proposed protocol deltas

### 1. Add role and kind to `message.started`

Current sketch:

```json
{
  "message_id": "msg_123",
  "run_id": "run_123",
  "index": 0
}
```

Proposed:

```json
{
  "message_id": "msg_123",
  "run_id": "run_123",
  "index": 0,
  "role": "assistant",
  "message_kind": "text"
}
```

Recommended `role` values:

- `assistant`
- `user`
- `system`
- `tool`
- `activity`
- `reasoning`
- `diagnostic`

Recommended `message_kind` values:

- `text`
- `reasoning_summary`
- `activity`
- `tool_result`
- `diagnostic`

Notes:

- Do not expose raw chain-of-thought as `reasoning` content.
- Visible reasoning should be summaries or intentionally surfaced traces.
- Diagnostic-only messages must not enter user-visible history unless explicitly rendered for debugging.

### 2. Introduce activity events

BearWire currently uses `run.progress` for status text, phase, queue, and heartbeat. Keep it for lightweight run progress, but add activity events for structured frontend-visible work.

Proposed event types:

```text
activity.snapshot
activity.delta
activity.completed
activity.failed
```

Example:

```json
{
  "type": "activity.snapshot",
  "subject": "resource/activity/act_123",
  "data": {
    "activity_id": "act_123",
    "run_id": "run_123",
    "activity_type": "tool_execution",
    "content": {
      "title": "Searching memory",
      "status": "running",
      "items": []
    }
  }
}
```

Example delta:

```json
{
  "type": "activity.delta",
  "subject": "resource/activity/act_123",
  "data": {
    "activity_id": "act_123",
    "activity_type": "tool_execution",
    "patch": [
      { "op": "add", "path": "/items/-", "value": { "label": "Found 3 notes" } }
    ]
  }
}
```

Use activity events for:

- server tool progress;
- sub-agent lifecycle;
- memory update notices;
- plan/checklist progress;
- search/scrape/fetch progress;
- channel-renderable but non-transcript UI.

Do not use activity events for canonical Bear memory.

### 3. Introduce first-class obligation events

Current concepts are spread across `tool_call.blocked`, `permission.requested`, `run.paused`, and client result methods. Preserve those domain events, but add obligation events as the stable continuation UX and authority surface.

Proposed event types:

```text
obligation.opened
obligation.resolved
obligation.cancelled
obligation.expired
obligation.failed
```

Example:

```json
{
  "type": "obligation.opened",
  "subject": "resource/obligation/obl_123",
  "data": {
    "obligation_id": "obl_123",
    "session_id": "ses_123",
    "run_id": "run_123",
    "kind": "permission_decision",
    "status": "pending",
    "client_method": "client.permission.result",
    "tool_call_id": "tc_123",
    "permission_request_id": "perm_123",
    "message": "Allow editing README.md?",
    "response_schema": {
      "type": "object",
      "properties": {
        "approved": { "type": "boolean" }
      },
      "required": ["approved"]
    },
    "expires_at": "2026-06-22T12:00:00Z"
  }
}
```

Recommended `kind` values:

- `client_tool_result`
- `permission_decision`
- `human_input`
- `resource_binding`

Recommended `status` values:

- `pending`
- `resolved`
- `cancelled`
- `expired`
- `failed`

Relationship to existing events:

- `tool_call.blocked` remains the tool-domain event.
- `permission.requested` remains the permission-domain event.
- `run.paused` remains the run-lifecycle event.
- `obligation.opened` is the generic client-action contract.

### 4. Replace vague resource updates with snapshot/delta

Current sketch:

```json
{
  "resource": {
    "kind": "workspace",
    "id": "repo_123",
    "version": "main"
  },
  "changes": {
    "branch": "main"
  }
}
```

Proposed snapshot:

```json
{
  "type": "resource.snapshot",
  "data": {
    "resource": {
      "kind": "plan",
      "id": "plan_123"
    },
    "value": {
      "title": "Refactor stream handling",
      "steps": []
    }
  }
}
```

Proposed delta:

```json
{
  "type": "resource.delta",
  "data": {
    "resource": {
      "kind": "plan",
      "id": "plan_123"
    },
    "patch": [
      { "op": "replace", "path": "/steps/0/status", "value": "complete" }
    ]
  }
}
```

Compatibility:

- Keep `resource.updated` as an alias during migration if already emitted.
- New producers should prefer `resource.snapshot` / `resource.delta`.
- Clients should apply deltas in sequence and request/resume from a snapshot if patch application fails.

### 5. Clarify `run.progress`

Keep `run.progress`, but narrow its semantics.

Recommended payload:

```json
{
  "run_id": "run_123",
  "kind": "phase",
  "text": "Running tools"
}
```

Recommended `kind` values:

- `status_text`
- `phase`
- `queue`
- `heartbeat`

Rules:

- `heartbeat` is ephemeral and must not be persisted as transcript.
- `status_text` may be user-visible but is not assistant transcript.
- Structured progress belongs in `activity.*`.
- Durable plan/resource changes belong in `resource.snapshot` / `resource.delta`.

### 6. Normalize client result method names

Use method names that reflect directionality: the client is returning a result to Den.

Preferred methods:

```text
client.capabilities.update
client.resource.update
client.tool.result
client.permission.result
```

Avoid or deprecate ambiguous names:

```text
client.tool.call
client.permission.request
```

Rationale:

- Den emits `tool_call.requested` and `permission.requested` events.
- The armature responds with `client.tool.result` or `client.permission.result`.
- `client.permission.request` reads as though the client is asking Den for permission, which is not the common armature flow.

### 7. Move BearWire vocabulary from role to stance

Current envelope fields include:

```json
{
  "role": "pair",
  "role_agent_id": "agent_123"
}
```

Target fields:

```json
{
  "stance": "pair",
  "runtime_binding_id": "binding_123"
}
```

Rules:

- Product/model-facing vocabulary uses Bear **stances**: `chat`, `pair`, `curate`, `work`, `watch`.
- Reserve **session mode** for Pair tool-policy state: `ask`, `plan`, `write`.
- `role` and `role_agent_id` may remain compatibility aliases at the wire boundary until v1 cutover, but should not be advertised in new docs or model-facing surfaces.

### 8. Clarify envelope timestamp and raw event fields

The current envelope uses `time`. Before v1 hardens, decide whether to keep `time` or rename to `timestamp`.

If keeping `time`, specify:

- value is RFC 3339 UTC timestamp;
- clients must not infer ordering from timestamp when `sequence` is present;
- replay ordering is by `sequence`.

Optional debug fields for transformed provider events:

```json
{
  "debug": {
    "raw_event_source": "openai.responses",
    "raw_event": {}
  }
}
```

Rules:

- Raw events are bounded/redacted.
- Raw events are omitted by default unless debug/trace mode is enabled.
- Raw events are not part of stable client behavior.

### 9. Make `scope` persistence rules normative

Current envelope has:

```json
{
  "scope": "persistent"
}
```

Make default scope guidance explicit:

| Event family | Default scope |
| --- | --- |
| `run.started`, terminal run events | `persistent` |
| transcript `message.*` | `persistent` |
| `tool_call.*` settlement/audit events | `persistent` |
| `permission.*` and `obligation.*` | `persistent` |
| `run.progress` `heartbeat` | `ephemeral` |
| `run.progress` `status_text` | `ephemeral` unless explicitly promoted |
| `activity.*` | `ephemeral` by default; `persistent` only for durable user-visible activity |
| `diagnostic.reported` | `ephemeral` or diagnostic-only persistence |
| `resource.snapshot` / `resource.delta` | resource-specific |

Persistence rules must preserve Den's transcript projection distinction:

- model replay;
- user-visible history;
- diagnostic-only history;
- live UI activity.

## Phased delivery plan

### Phase 0 — Spec audit and compatibility table

Goal: make the current BearWire JSON spec and implementation plan agree before changing code.

Tasks:

- Add a BearWire v1 refinement section to `bearwire-json-spec.md` or link this roadmap from it.
- Record current event/method names and their proposed v1 names.
- Identify emitted names already used by `den-bearwire` and `bear-armature`.
- Decide whether `time` remains or becomes `timestamp`.
- Decide whether `resource.updated` remains as alias or is removed before default BearWire rollout.

Exit criteria:

- One reviewed compatibility table exists.
- No unresolved mismatch between roadmap, JSON spec, and implementation plan for client result method names.

### Phase 1 — Message and run lifecycle tightening

Goal: clarify high-volume event shapes without changing behavior.

Tasks:

- Add `role` and `message_kind` to `message.started` projection.
- Document run lifecycle invariants in the JSON spec.
- Add tests that each started run emits exactly one terminal run event.
- Add tests that message streams emit `message.started` before `message.delta` and terminate with `message.completed` or `message.aborted`.

Exit criteria:

- BearWire event projection tests cover message lifecycle and terminal run invariants.
- Adapter remains backward-compatible with missing `role` / `message_kind` during the transition.

### Phase 2 — Obligation events

Goal: expose Den's persisted obligation authority directly on the wire.

Tasks:

- Add `obligation.*` wire types.
- Emit `obligation.opened` when persisted `bearwire_run_obligations` rows become pending.
- Emit terminal obligation events when obligations settle, expire, fail, or are cancelled.
- Update `client.tool.result` and `client.permission.result` docs to require `obligation_id` where available.
- Add replay/reconnect tests proving obligation state can be reconstructed from persistence.

Exit criteria:

- Armature can render blocked/waiting UX from `obligation.opened` without scraping `tool_call.blocked` or `permission.requested`.
- Existing tool/permission events remain available for domain-specific detail.

### Phase 3 — Resource snapshot/delta and activity events

Goal: separate durable resources, structured activity, and run progress.

Tasks:

- Add `resource.snapshot` and `resource.delta` to spec and projection.
- Define JSON Patch usage for `resource.delta`.
- Add `activity.snapshot`, `activity.delta`, `activity.completed`, and `activity.failed`.
- Move plan updates and rich tool/subagent/memory UI events toward `activity.*` or `resource.*` depending on durability.
- Keep `run.progress` for lightweight status/phase/queue/heartbeat only.

Exit criteria:

- Clients can distinguish transcript, activity, resource state, and ephemeral progress.
- No ephemeral status/progress event is persisted as assistant transcript.

### Phase 4 — Method naming cleanup

Goal: make BearWire RPC method names directionally unambiguous.

Tasks:

- Standardize client result methods as `client.tool.result` and `client.permission.result`.
- Deprecate or remove `client.tool.call` and `client.permission.request` if present in docs/code.
- Add `client.capabilities.update` if capability reporting is currently folded into `resource.update` or adapter environment payloads.
- Update `bear-armature` fallback/compatibility handling.

Exit criteria:

- JSON spec, implementation plan, den-bearwire handlers, and bear-armature client use the same method names.
- Legacy method aliases are confined to compatibility boundaries.

### Phase 5 — Stance terminology migration

Goal: align BearWire vocabulary with Bear stance terminology before broad user/model-facing rollout.

Tasks:

- Add `stance` and `runtime_binding_id` envelope fields.
- Keep `role` and `role_agent_id` as compatibility aliases only if needed.
- Update BearWire docs and UI/debug labels to distinguish Bear stance from session mode.
- Ensure model-facing/provider-facing tool descriptors do not advertise legacy role-agent naming.

Exit criteria:

- New BearWire docs use `stance` and `runtime_binding_id`.
- Compatibility aliases are documented as legacy.

### Phase 6 — v1 lock and migration cleanup

Goal: freeze BearWire v1 wire grammar and remove transitional ambiguity.

Tasks:

- Update `bearwire-json-spec.md` with final v1 schemas.
- Add golden trace snapshots for refined event shapes.
- Update `BEARWIRE_ARMATURE_WIRE_IMPLEMENTATION_PLAN.md` Phase 4 gate to include this roadmap's exit criteria.
- Remove or mark deprecated any transitional method/event aliases not needed for one-release fallback.
- Document v1 compatibility policy.

Exit criteria:

- BearWire can default on for armatures with stable run/message/tool/permission/resource semantics.
- `/acp/**` removal or thin-shim work can proceed without protocol grammar churn.

## Testing strategy

| Layer | Coverage |
| --- | --- |
| Unit | Runtime semantic event -> refined BearWire event projection |
| Golden traces | Full prompt/tool/permission/cancel/reconnect traces with stable event snapshots |
| Integration | `den-bearwire` RPC + SSE session/run tests |
| Adapter | `bear-armature` parses old and refined shapes during compatibility window |
| Persistence | obligation replay and terminal settlement from DB state |
| Manual smoke | Zed plain chat, file tool, permission-gated edit, denial, cancellation, close, reconnect |

## Compatibility notes

- BearWire is not yet the only production armature wire, so prefer making refinements before Phase 4 default rollout.
- Where compatibility is needed, accept old field names and emit new fields.
- Avoid long-lived dual names in docs. Mark old names as legacy aliases.
- Do not stage a large mechanical code rename without golden trace coverage.

## Open questions

- Should `time` be renamed to `timestamp` before v1, or kept to avoid churn?
- Should `resource.updated` remain as a permanent convenience event, or only an alias for `resource.snapshot` / `resource.delta` during migration?
- Should obligation events replace `run.paused` for some waits, or should `run.paused` always accompany at least one pending obligation?
- Which activity types should be standardized in v1 versus left application-defined?
- Which BearWire events are persisted in canonical conversation history versus protocol replay logs only?
