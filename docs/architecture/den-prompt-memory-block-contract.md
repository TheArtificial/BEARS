# Den Prompt Memory Block Contract

This document defines the architectural contract for Den-owned editable prompt memory blocks.

Prompt memory blocks are explicit, scoped in-context state used during prompt assembly. They are not the same thing as durable Bear memory, transcript history, or retrieval results.

## Summary

- prompt memory blocks are Den-owned editable context objects
- every block has explicit scope and lifecycle
- prompt blocks are compiled into runtime context by policy
- prompt blocks remain distinct from transcript, shared memory, and derived recall

## What a prompt memory block is

A prompt memory block is an editable context object intended for direct inclusion in turn context.

Examples:

- stance-local guidance
- work-surface context reminders
- session focus notes
- bounded operator/user instruction blocks

## Core terms

### Block scope

The attachment boundary where a block applies.

Typical scopes:

- Bear-wide
- stance-local
- work-surface-attached
- session-scoped

### Block lifecycle

Whether a block is active, superseded, archived, draft, or otherwise excluded from default compilation.

### Prompt compilation

The Den-owned process that selects, orders, and budgets prompt memory blocks alongside compiled prompt, projected memory, transcript, and other runtime supplements.

## Invariants

### 1. Blocks are explicit

Den owns block identity, content, scope, and lifecycle.

### 2. Blocks are scoped

No block should apply everywhere by accident.

### 3. Blocks are auditable

Creation, mutation, supersession, and archival should be attributable.

### 4. Blocks are not generic memory promotion

Blocks may be informed by memory, but block editing is not the same thing as promoting shared Bear knowledge.

## Relationship to other context systems

### Versus transcript history

Transcript says what happened. Prompt memory says what standing context should shape future turns.

### Versus durable memory

Durable memory is broader Bear cognition. Prompt blocks are a smaller directly compiled subset.

### Versus derived recall

Derived recall is query-time retrieval. Prompt blocks are standing scoped context selected by policy.

### Versus compaction artifacts

Compaction artifacts summarize older transcript state. Prompt blocks are editable scoped context objects.

## Prompt compilation expectations

Prompt compilation should define:

1. eligibility
2. ordering
3. precedence between broader and narrower scopes
4. budgeting and omission behavior

Specific scopes should outrank broader defaults when they conflict.

## Minimum block families

An architecture-level minimum set is:

- stance guidance blocks
- work-surface context blocks
- session focus blocks
- bounded instruction blocks

## Related docs

- [memory model](memory-model.md)
- [den runtime](den-runtime.md)
- [agent and bear environments](agent-and-bear-environments.md)
