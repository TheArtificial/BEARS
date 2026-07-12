# Den Conversation Runtime Schema

This document summarizes the architectural shape of Den-owned conversation and runtime state.

It is a reference model for how conversation identity, transcript history, run state, tool activity, and approvals fit together in the Den-native architecture.

## Summary

- Den owns canonical conversation identity and transcript persistence
- tool calls and approvals are first-class runtime artifacts, not message archaeology
- conversations, runs, events, tool calls, and approvals are related but distinct entities
- provider-specific compatibility references are optional metadata, not canonical identity

## Core entities

The architecture expects these conceptual entities:

| Entity | Purpose |
|--------|---------|
| Conversation | canonical thread identity and metadata |
| Message / transcript artifact | readable user/assistant/tool/system history |
| Run / turn | one bounded execution of the runtime |
| Event | fine-grained execution and audit trail |
| Tool call | first-class tool request/result lifecycle |
| Approval / client obligation | first-class human or armature action lifecycle |

## Design principles

### 1. Transcript is canonical

Conversation history is Den-owned and replayable.

### 2. Messages are not enough

Pending execution state, approvals, and tool lifecycles should not be inferred only from flattened message text.

### 3. Tool activity must be replayable

If the model saw a tool request or result, Den must persist enough structured data to replay that fact later.

### 4. Approval is a state machine

Permission or client-obligation state is first-class runtime state, not a side note on a message.

### 5. Compatibility references are optional metadata

External/provider references may exist, but Den ids remain canonical.

## Recommended record families

The exact table names may vary, but the architecture expects records equivalent to:

- conversations
- conversation messages / transcript artifacts
- conversation events
- conversation runs or turn records
- tool-call records
- approval or client-obligation records

## What each family should capture

### Conversation

- Bear
- stance
- surface/session association
- title and archive/delete state
- canonical identifiers

### Transcript artifacts

- user messages
- assistant messages
- assistant tool-call artifacts
- tool results
- other durable model-relevant artifacts

### Runs / turns

- lifecycle state
- timestamps
- failure/cancellation information
- relationship to the active conversation

### Tool calls

- stable tool-call id
- canonical tool name
- typed arguments
- lifecycle status
- result or structured error

### Approvals / obligations

- what the system is waiting for
- which surface must respond
- current status
- final decision/outcome

## Why this separation matters

Without this separation:

- UIs become dependent on brittle message parsing
- replay becomes incomplete
- approvals and tool failures become harder to reason about
- future edges cannot project the same core state consistently

## Related docs

- [den runtime](den-runtime.md)
- [acp-runtime-contract](acp-runtime-contract.md)
- [bear channel and ACP](bear-channel-and-acp.md)
