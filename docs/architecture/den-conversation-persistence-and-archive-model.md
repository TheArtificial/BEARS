# Den Conversation Persistence and Archive Model

This document describes the current architectural model for Den-owned conversation persistence, transcript replay, compaction artifacts, and archive state.

Conversations are canonical Den records. They are not edge-local caches and not provider-owned state.

## Summary

- Den owns canonical conversation identity and metadata
- transcript artifacts are append-only and replayable
- compaction and archive state are derived, linked records rather than replacements for transcript truth
- tool calls and approvals must remain connected to the transcript and runtime record model
- every non-secret item delivered to a model must have a durable, user-visible transcript projection

## Core goals

- preserve a canonical, auditable conversation history
- support model replay and user-visible history as separate projections over the same stored state
- support archive, restore, and compaction behavior without mutating transcript truth
- keep conversations linked to approvals, tool calls, and runtime turns without collapsing them into one table

## Core entities

The architecture expects these conceptual record families:

| Entity | Purpose |
|--------|---------|
| Conversation | durable thread identity and metadata |
| Transcript artifact | ordered user/assistant/tool/system history |
| State snapshot | optional fast resume/read-model checkpoint |
| Compaction artifact | summary or bounded context artifact derived from transcript |
| Archive record | lifecycle and archived projection state for a closed/superseded conversation |

## Design principles

### 1. Den owns canonical conversation state

Conversation identity, title/archive state, transcript history, and runtime-facing replay records are Den-owned.

### 2. Transcript is append-only

Messages and transcript artifacts should be treated as append-only except for explicit redaction/audit mechanisms.

### 3. Derived state stays separate

Compaction summaries, snapshots, and archive projections are linked to transcript truth. They do not replace it.

### 4. Transcript supports multiple projections

The same stored transcript must support:

- model replay
- user-visible conversation history
- operator/debug inspection

Those projections may differ in presentation, but the underlying source of truth is shared. Every non-secret item delivered to a model must have a durable user-visible projection; clients may collapse runtime artifacts, but must retain the exact delivered representation or a stable reference, delivery boundary, source, and redaction state for inspection.

## Conversation

A conversation is the durable logical thread.

Typical metadata includes:

- owning Bear
- active stance or originating surface context
- session/surface association where relevant
- current title
- lifecycle state such as active or archived
- timestamps and related references

## Transcript artifacts

Transcript artifacts are immutable ordered records such as:

- user messages
- assistant messages
- tool call artifacts
- tool result artifacts
- system/developer/runtime-visible markers when needed for replay
- model-context delivery artifacts, including source, delivery boundary, exact delivered payload or stable reference, and explicit redaction state

They must be rich enough to support future model context reconstruction.

## Snapshots and compaction

Snapshots and compaction artifacts are optimization and context-boundary tools.

Examples:

- runtime resume checkpoints
- compaction summaries
- bounded archive summaries

These artifacts should point back to the source transcript range they summarize.

## Archives

An archive record represents the archived lifecycle of a conversation.

It may include:

- archive reason
- archive version
- related compaction/summary artifact
- restore linkage when a conversation is resumed or superseded

Archive state is distinct from the live conversation row so lifecycle remains explicit.

## Relationship to other state

### Sessions

Client sessions or surface bindings are not the canonical transcript log. They are routing/binding state.

### Workboard and tasks

Plans, jobs, and tasks may reference conversations, but they do not replace transcript persistence.

### Tool calls and approvals

Tool calls and approvals are first-class runtime records linked to the same conversation/turn system. They should not exist only as free-form message text.

## What this model should let the system do

- replay prior turns for future model context
- show user-facing history without losing tool context
- inspect failures and approvals operationally
- archive and restore old threads safely
- compact long conversations without destroying transcript truth

## Related docs

- [den conversation runtime schema](den-conversation-runtime-schema.md)
- [den runtime](den-runtime.md)
- [acp-runtime-contract](acp-runtime-contract.md)
