# Den/BearWire/Armature/ACP Surface Replay Hardening Plan

**Status:** Proposed  
**Date:** 2026-07-07  
**Related:** [ADR-0034: BearWire as the Den ↔ armature wire](../decisions/adr-0034-bearwire-as-den-armature-wire.md), [ADR-0043: ACP Is an Edge Adapter; the Den Runtime Is Protocol-Agnostic](../decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md), [ADR-0049: ACP tool-call and permission UX semantics](../decisions/adr-0049-acp-tool-call-and-permission-ux.md), [Den-BearWire-Armature-ACP Hardening Plan](DEN_BEARWIRE_ARMATURE_ACP_HARDENING_PLAN.md)

## Purpose

Make the active Den → BearWire → armature → ACP path deterministic by enforcing one typed surface-event contract for both live streaming and session/history replay.

This plan responds to recurring ACP symptoms that have been difficult to fix permanently:

- `set_conversation_title` can render as an empty tool card or fail to surface a title update consistently;
- provider reasoning can appear as ordinary assistant text, especially after session load;
- runtime checkpoint tool cards can be empty;
- loaded history replays tool activity as text instead of ACP tool cards.

The working diagnosis is that these are not isolated ACP rendering bugs. They are contract bugs caused by live streaming, persisted BearWire events, conversation history, and armature load replay using different projections.

## Target invariant

Implement the invariant added to ADR-0034:

> A live BearWire event stream and a later session/history replay for the same run must project to equivalent user-visible armature state. Tool calls remain tool cards; reasoning remains reasoning/thought display or is omitted by explicit replay policy; session title/mode/plan changes remain typed updates. Replay must not collapse typed surface facts into assistant text.

## Non-goals

- Do not replace ACP as the editor-facing protocol.
- Do not make ACP canonical inside Den.
- Do not build a generalized event-sourcing framework before proving the narrow surface contract.
- Do not expose provider chain-of-thought as canonical conversation history.
- Do not redesign all conversation persistence tables unless the narrower typed replay path proves insufficient.

## Definitions

### Surface event

A typed user-visible or armature-visible projection event. Examples:

- assistant message delta;
- reasoning/thought delta with replay policy;
- tool call started/updated/finished;
- permission/client wait;
- session title/mode/context update;
- plan/task-list update;
- run terminal outcome.

### Surface replay

A replayable sequence of surface events sufficient for an armature to reconstruct the same UI semantics as a live stream.

### Text history

A human-readable flattened transcript projection. It remains useful for simple chat history, but it is not sufficient for ACP session load because it cannot represent tool cards, thought chunks, or typed session updates.

## Current suspected fault lines

### 1. ACP load uses text-only `conversation.history`

`tools/bear-armature/src/main.rs` fetches `conversation.history`, reads only `{ role, text }`, and replays messages via `send_user_message_chunk` / `send_agent_message_chunk`.

`services/den/crates/den-bearwire/src/methods/conversation.rs` returns only text records. It does not expose tool-call rows, tool-result rows, reasoning display events, or session/resource updates in a replayable surface shape.

Expected consequence: loaded tool activity appears as plain text, and loaded reasoning can only appear as ordinary assistant text if it was persisted in visible history.

### 2. Den-hosted tools can emit only sparse completion events

`set_conversation_title` and other Den-hosted tools may execute inside `SessionTrackingStream` without first emitting a full `tool_call.requested` event to the armature. The later `tool_call.completed` event can contain only `tool_call.id/name` plus a summary.

Expected consequence: the armature must reconstruct a tool card from missing arguments/display metadata, producing empty or generic cards.

### 3. Runtime checkpoint tool calls are intercepted as finished events

Checkpoint calls are handled internally and returned as `ToolCallFinished` without necessarily exposing a full started tool-call surface event with checkpoint arguments/display metadata.

Expected consequence: checkpoint cards are empty or generic.

### 4. Session title load/list reads are inconsistent

Den session state exposes `conversation_title`, while some armature mapping paths historically looked for `title`. Live update and load/list title behavior can therefore disagree.

### 5. Reasoning separation is not guaranteed across all projections

Live BearWire has `message.reasoning.delta`, but the text-only history path has no type to distinguish reasoning from assistant answer content. Any visible persisted reasoning-like text will reload as ordinary assistant text.

## Implementation plan

### Phase 0 — Trace capture and boundary assertions

**Goal:** Establish exact observed failure boundaries before changing behavior.

Capture one failing run for each target symptom across these layers:

1. raw provider stream fragments;
2. `RuntimeSemanticEvent` / `RuntimeStreamEvent` emitted by Den runtime;
3. persisted `bearwire_events` rows;
4. BearWire SSE/JSON-RPC events received by `bear-armature`;
5. ACP `session/update` messages sent by the armature;
6. `conversation_messages` rows;
7. `conversation.history` response;
8. ACP session load/replay output.

**Suggested diagnostics:**

- add temporary test fixtures or debug snapshots, not broad production logging;
- store sanitized golden traces under a test fixture path if useful;
- explicitly mark whether each event is live-only, persisted-surface, or text-history.

**Exit gate:** For each symptom, the team can identify the first boundary where type/display information is lost.

### Phase 1 — Introduce `bearwire-protocol` and define typed surface event DTOs

**Goal:** Introduce a narrow, serde-backed `bearwire-protocol` crate shared by Den BearWire projection and armature replay/projection tests.

The crate charter is intentionally small:

> `bearwire-protocol` contains stable serde DTOs and lightweight validation helpers for the BearWire Den↔armature wire. It must not depend on Den runtime, Den service, HTTP server/client code, database crates, ACP, or model/provider clients.

Allowed initial dependencies should be limited to `serde` and `serde_json`. Prefer string wire identifiers over `uuid` unless a field is explicitly UUID-typed in the BearWire contract.

Candidate DTOs:

```rust
enum SurfaceEvent {
    AssistantMessageDelta,
    ReasoningDelta,
    ToolCallStarted,
    ToolCallUpdated,
    ToolCallFinished,
    ClientWaiting,
    SessionInfoUpdated,
    PlanUpdated,
    RunCompleted,
    RunFailed,
    RunCancelled,
}
```

Required fields for `ToolCallStarted`:

- stable `tool_call_id`;
- canonical provider/tool name;
- tool kind/action category;
- typed arguments/raw input;
- descriptor-owned display metadata;
- execution owner (`den`, `armature`, `mcp`, etc.);
- run/session references;
- replay policy.

Required fields for `ToolCallFinished`:

- same `tool_call_id`;
- status;
- concise summary;
- bounded raw/structured output or error;
- reference to the full started tool-call record when not repeated inline.

Required fields for reasoning:

- text/delta;
- source (`provider_reasoning`, etc.);
- replay policy (`none` or `thought`; other policies are intentionally unsupported until a product need is proven);
- explicit statement that it is not assistant answer text.

**Likely files:**

- `services/den/crates/bearwire-protocol/` for shared BearWire DTOs and validation helpers;
- `services/den/Cargo.toml` workspace membership and workspace dependency wiring;
- `services/den/crates/den-runtime/src/runtime/bearwire_projection/` for runtime-semantic → BearWire DTO projection;
- `services/den/crates/den-bearwire/` for BearWire RPC responses that serialize these DTOs;
- `tools/bear-armature/src/bearwire.rs` for typed decode and ACP projection.

`den-protocol` may re-export `bearwire-protocol` types later for Den-internal convenience, but `bear-armature` should depend on `bearwire-protocol` directly rather than on broad Den protocol/runtime crates.

**Exit gate:** A malformed or sparse surface event fails focused tests in strict mode instead of silently becoming a generic text/card projection, and `bear-armature` does not depend on `den-protocol` for surface replay.

### Phase 2 — Require full tool-call start records for every surfaced tool

**Goal:** Ensure every tool-card-producing flow has a full `ToolCallStarted`/`tool_call.requested` equivalent before completion.

Flows to fix:

1. armature-local tools;
2. Den-hosted tools (`set_conversation_title`, `session_info`, memory/task tools, web tools executed in Den);
3. checkpoint tool;
4. permission-mediated local tools;
5. forwarded MCP tools.

Implementation direction:

- Den runtime emits/persists a full tool-call surface event before executing a Den-hosted tool;
- checkpoint tool handling emits/persists a full checkpoint tool-call surface event before returning its tool result;
- completion events may remain sparse only when the full started record is queryable by `tool_call_id`;
- descriptor-owned display metadata is generated at the Den boundary for Den-hosted tools and preserved through BearWire.

**Specific bug probes:**

- `set_conversation_title` card title includes the requested title before and after completion;
- checkpoint card includes checkpoint objective/summary/next action or a descriptor-owned concise equivalent;
- missing `tool_call.name`, `tool_call.arguments`, or display metadata is a test failure for started events.

**Exit gate:** No user-visible tool card is first introduced by a sparse completion event.

### Phase 3 — Add surface replay API

**Goal:** Stop using text-only `conversation.history` for ACP session load/replay.

Add a BearWire method such as one of:

- `conversation.surface_history`;
- `session.surface_history`;
- `session.replay_surface_events`.

The method returns typed surface events in chronological order, with pagination and replay policy applied.

Rules:

- includes assistant text chunks or message parts;
- includes tool started/finished events with enough data to reconstruct ACP `ToolCall` updates;
- includes session info/title/mode/plan updates when relevant to the session UI;
- omits provider reasoning by default if replay policy is `none`; otherwise replays as thought, never assistant text;
- can coexist with `conversation.history`, which remains flattened text for simple human history.

**Likely files:**

- `services/den/crates/den-bearwire/src/methods/conversation.rs` or new `surface.rs`;
- `services/den/crates/den-runtime/src/bearwire_events.rs` / conversation persistence helpers;
- `tools/bear-armature/src/main.rs` session load/resume replay code.

**Exit gate:** ACP `session/load` and `session/resume` replay through the same projection helpers as live BearWire events, not through `{ role, text }` chunks.

### Phase 4 — Make session metadata updates first-class

**Goal:** Title/mode/plan updates must be typed surface events, not inferred from arbitrary tool-result prose.

Implementation direction:

- `set_conversation_title` emits/persists `SessionInfoUpdated { title, updated_at }` as an explicit surface event;
- update armature session list/load mapping to read Den’s `conversation_title` / `conversation_title_updated_at` fields consistently;
- preserve `conversation_title_synced_at` or equivalent only as Den bookkeeping, not as the UI source of truth;
- plan/task-list updates similarly replay as typed plan updates.

**Exit gate:** Setting a conversation title updates live ACP session info, persists in Den session state, appears in session list/load, and replays consistently after restart.

### Phase 5 — Reasoning separation hardening

**Goal:** Prevent provider reasoning from ever becoming visible assistant answer text through either live streaming or replay.

Implementation direction:

- add tests from provider raw events to runtime semantic events for each provider/backend currently used;
- assert `ReasoningTextDelta` maps to `message.reasoning.delta` with replay policy;
- assert `message.reasoning.delta` maps to ACP `AgentThoughtChunk` live;
- assert surface replay omits or thought-replays reasoning according to policy;
- add a detector test for malformed `message.delta` carrying reasoning markers, but do not rely on heuristics as the normal path.

**Exit gate:** No reasoning fixture can appear as ACP `AgentMessageChunk` in live stream or load replay.

### Phase 6 — Golden trace tests across all boundaries

**Goal:** Make projection drift impossible to miss.

Add golden trace tests for:

1. `set_conversation_title` Den-hosted tool;
2. checkpoint tool;
3. local file read success/failure;
4. permission-mediated web fetch/local fetch;
5. provider reasoning delta;
6. loaded session with prior tool activity.

Each golden trace should assert:

- runtime semantic events;
- BearWire persisted events;
- ACP live updates;
- conversation text history where applicable;
- surface replay output;
- ACP load/resume output.

**Exit gate:** A change that preserves live streaming but breaks load replay fails the same test suite.

### Phase 7 — Strict mode and legacy cleanup

**Goal:** Stop tolerating incomplete surface envelopes in tests and development.

Implementation direction:

- add strict decode for surface events in armature tests;
- reject missing started tool-call data when completion is used to create a visible card;
- keep production compatibility only where required and instrument it as degraded compatibility;
- remove compatibility branches once Den no longer emits old shapes.

**Exit gate:** The armature no longer performs archaeological reconstruction for normal active flows; compatibility fallbacks are rare, measured, and covered by migration tests.

## Implementation progress (2026-07-07)

The first implementation pass has landed the narrow surface replay contract and the most important ACP parity protections.

Completed:

- Added `bearwire-protocol` as the narrow shared BearWire DTO crate for method, wire, RPC, and surface replay contracts.
- Moved BearWire wire/method DTOs out of broad Den-internal crates so `bear-armature` can depend on `bearwire-protocol` directly rather than `den-protocol`.
- Added `conversation.surface_history` and wired ACP `session/load` / `session/resume` to replay typed surface records instead of text-only `conversation.history`.
- Preserved `conversation.history` as the flattened text projection for simple/non-ACP history consumers.
- Added typed `SurfaceHistoryEvent` replay records for messages, tool calls, tool results, reasoning deltas, and session info updates.
- Fixed message timestamp serialization so `SurfaceHistoryEvent::Message.created_at` is a string, not an `OffsetDateTime` sequence.
- Ensured surface replay omits reasoning with `replay_policy = none` and replays opt-in reasoning as ACP thought chunks, never assistant text.
- Added Den-hosted and checkpoint tool started events before their completion events in the runtime stream.
- Added descriptor/title/argument coverage for `set_conversation_title` started events.
- Added checkpoint started-before-finished coverage.
- Added `policy.execution_target = "den"` on Den-owned surfaced tool starts and taught the armature to treat canonical `data.policy.execution_target` as display-only Den execution ownership rather than an armature-local execution request.
- Updated surface replay tool-card projection to use canonical BearWire `data.tool_call` payloads rather than legacy top-level `tool_call_id` / `tool_name` / `args` fields.
- Updated armature title mapping to prefer Den `conversation_title` over legacy `title` on session list/load context.
- Added focused tests covering:
  - Den-hosted started events;
  - checkpoint started/finished ordering;
  - `conversation.surface_history` message/tool/session/reasoning records;
  - ACP load/resume replaying tool records as tool cards rather than assistant text;
  - live and replayed reasoning staying in thought UI;
  - Den-owned tool starts rendering without `client.tool.result` local execution posts;
  - provider Responses reasoning deltas becoming `ReasoningTextDelta`.

Residual follow-up:

- The dedicated persisted surface-event stream is intentionally deferred. The current merged `conversation.surface_history` projection is sufficient while ordering/pagination needs are bounded and covered by focused tests; revisit only if exact cross-source ordering becomes product-critical.
- Expand strict mode from focused decode tests to a broader development/test gate that rejects incomplete normal-path surface envelopes across all producers.
- No additional reasoning replay policies are planned. `none` and `thought` are sufficient for current product behavior; unsupported policies are omitted rather than replayed.

## Delivery order

Recommended sequence:

1. trace capture and explicit boundary assertions;
2. `bearwire-protocol` crate with typed surface event DTOs;
3. full tool-call started records for Den-hosted and checkpoint tools;
4. surface replay API;
5. armature load/resume uses surface replay;
6. first-class session metadata updates;
7. reasoning separation hardening;
8. golden traces;
9. strict mode and legacy cleanup.

## Validation matrix

| Symptom | Required passing behavior |
| --- | --- |
| `set_conversation_title` empty card | Live stream includes full started card with requested title and completion summary. |
| Title does not stick | Den state has `conversation_title`; ACP live update, session list, session load, and replay agree. |
| Reasoning as assistant text live | Provider reasoning reaches ACP only as thought/reasoning display. |
| Reasoning as assistant text on load | Surface replay omits/thought-replays reasoning by policy; text history is not used for ACP UI replay. |
| Checkpoint empty card | Checkpoint emits full started surface event and finished status with useful summary. |
| Tool cards load as text | ACP load/resume replays typed tool events as `ToolCall` updates. |

## Decisions recorded

1. `conversation.surface_history` remains a merged projection over canonical conversation structured rows, selected BearWire events, and session metadata for now. A dedicated persisted surface-event stream is intentionally deferred until exact cross-source ordering/pagination becomes necessary.
2. Reasoning replay supports only two policies: `none` (omit from replay) and `thought` (replay as ACP thought/reasoning display). These are sufficient for current product behavior.
3. Text-only `conversation.history` remains available for non-ACP/simple clients, but ACP load/resume must continue using typed surface replay.
4. The initial `bearwire-protocol` surface contract should stay narrow; add DTOs beyond `SurfaceHistoryEvent` only when active BearWire projection paths need them.

## Immediate next step

Before implementation, produce one sanitized failing trace for `set_conversation_title` and one for checkpoint cards. Use those traces to decide whether the first code change should be:

- emitting full tool-start surface events for Den-hosted/checkpoint tools; or
- adding the surface replay endpoint first.

The likely answer is to start with full tool-start events because both live card correctness and replay correctness depend on that source data existing.
