# ACP Runtime Contract

This document defines the current ACP-facing runtime contract for Bear Den.

ACP is an **edge protocol** for trusted armatures. The runtime beneath it is Den-native and protocol-neutral.

## Summary

- ACP does not own turn semantics
- the Den runtime owns turn execution, tool orchestration, continuation, approvals, and replayable state
- ACP projects that state into armature-friendly protocol messages and permission flows
- BearWire is the current Den-to-armature wire used under the ACP adapter path

## Scope

This contract covers the runtime behaviors ACP needs from Den:

- session-to-conversation binding
- turn submission and streaming
- tool-call projection
- client obligations such as permission waits and local-tool execution
- cancellation
- history replay and status inspection

It does not redefine the full runtime architecture.

## Design principles

### 1. Behavior-first

ACP should depend on Den-owned runtime behavior, not on hidden transport-specific assumptions.

### 2. Protocol-neutral core

The turn loop, continuation logic, client-obligation coordination, and transcript persistence belong to the Den runtime core.

### 3. Replayable tool and approval state

Tool calls, results, and approvals must be represented as durable structured runtime artifacts, not only as temporary UI state.

### 4. Stable edge projection

ACP should see stable surface behavior for:

- conversation/session lifecycle
- turn streaming
- tool-call updates
- permission requests
- cancellation

## Core contract areas

### Session and conversation binding

ACP sessions bind a client session to a Bear, stance, and canonical conversation state.

### Turn execution

ACP submits a human turn to Den and receives streamed runtime output and status.

### Tool projection

Den projects:

- Den-hosted tool execution results
- armature-local tool requests
- tool status transitions and summaries

Tool projection is for model-visible tool exchanges whose results matter to continuation or replay. Execution ownership stays explicit in the projection:

- Den-hosted / Den-owned tool events are display-only from the armature's perspective. ACP may render them as tool cards, but the armature must not execute them locally or answer them with `client.tool.result`.
- Armature-local tool requests are client obligations. The armature may request permission, execute the local action, and return the result to Den.
- Execution ownership is descriptor-resolved once in Den. Live events and `run.state` recovery use the same owner; recovery must not turn a Den-owned display event into local execution.

Live ACP tool-card projection should be idempotent and monotonic for each tool call. Terminal card states such as `completed` and `failed` are final for the live surface, and stale updates from older turns/runs should not regress the visible card state.

### Non-blocking structured updates

Den also projects surface/control-plane updates that do not block model continuation, such as conversation-title updates, advisory task progress, or other durable UI state. ACP clients may render and replay these updates, but they are not client obligations and do not require a tool-result response from the armature.

### Client obligations

When Den needs an armature action, it emits a structured client obligation such as:

- permission request
- local tool execution request
- local tool result wait

### Cancellation

ACP can request cancellation of the current active turn or operation through Den-owned cancellation semantics. For a persisted run, cancellation becomes visible only when Den atomically transitions the run, settles its obligations and active steps, and appends `run.cancelled`. Local tool-card cancellation is not a substitute for the run terminal event.

### History and replay

ACP can load or replay prior conversation/tool history from Den-owned transcript state. Replay may also include typed non-blocking structured updates when their replay policy says they are part of session/work surface state.

## Runtime-owned types

Conceptually, ACP relies on Den-owned concepts such as:

- runtime conversation reference
- runtime turn/run reference
- transcript/history record
- tool-call record
- client obligation / approval record
- runtime stream event

The exact Rust types may evolve, but the conceptual ownership should not.

## What ACP should not own

ACP should not become the source of truth for:

- conversation identity
- turn continuation legality
- approval settlement logic
- tool replay state
- model transcript history

Those belong to the runtime core.

## Related docs

- [den runtime](den-runtime.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [den conversation runtime schema](den-conversation-runtime-schema.md)
- [ADR-0043: ACP as edge adapter, protocol-agnostic core](../decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)
- [ADR-0048: core turn/client-obligation coordinator](../decisions/adr-0048-core-turn-client-obligation-coordinator.md)
