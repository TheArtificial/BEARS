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

### Client obligations

When Den needs an armature action, it emits a structured client obligation such as:

- permission request
- local tool execution request
- local tool result wait

### Cancellation

ACP can request cancellation of the current active turn or operation through Den-owned cancellation semantics.

### History and replay

ACP can load or replay prior conversation/tool history from Den-owned transcript state.

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
