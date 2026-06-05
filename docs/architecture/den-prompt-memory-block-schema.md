# Den Prompt Memory Block Schema Direction

This document defines the initial schema direction for Den-owned prompt memory blocks.

It implements the persistence-oriented portion of the [Den Prompt Memory Block Contract](./den-prompt-memory-block-contract.md).

## Goals

The schema direction must let Den:

- store editable prompt blocks as Den-owned objects,
- attach them to explicit scopes,
- preserve lifecycle and provenance,
- compile them into prompt context by policy,
- and keep them distinct from transcript, retrieval, and durable memory storage.

## Logical entities

### 1. Prompt block

Recommended logical fields:

- `id`
- `block_kind`
- `title` nullable
- `content_text`
- `content_json` nullable for future structured variants
- `scope_kind`
- `scope_ref`
- `role` nullable
- `channel` nullable
- `lifecycle_state`
- `priority`
- `created_at`
- `updated_at`
- `created_by`
- `updated_by`
- `supersedes_block_id` nullable

Notes:

- `scope_ref` should point to a Den-owned scope identity such as Bear id, role/work-surface reference, or session id.
- `priority` should help prompt compilation resolve ordering among otherwise eligible blocks.

### 2. Prompt block revision or audit event

Recommended logical fields:

- `id`
- `block_id`
- `event_kind`
- `old_content_text` nullable
- `new_content_text` nullable
- `old_lifecycle_state` nullable
- `new_lifecycle_state` nullable
- `actor_ref`
- `reason` nullable
- `created_at`

Notes:

- A separate audit table is preferable to silent in-place mutation if we need dependable provenance.
- If revision tables are too heavy for the first slice, an append-only audit log with enough old/new state is still required.

### 3. Prompt compilation decision record

Recommended logical fields if persisted:

- `id`
- `session_or_run_ref`
- `block_id`
- `included`
- `exclusion_reason` nullable
- `effective_priority`
- `budgeted_chars_or_tokens` nullable
- `created_at`

Notes:

- This can begin as diagnostic/runtime logging rather than immediate durable persistence.
- The purpose is to explain inclusion behavior when debugging prompt assembly.

## Scope model

The schema should support these initial scope kinds:

- `bear`
- `role`
- `work_surface`
- `session`

Optional future scope kinds may be added later, but v1 should not require provider-managed conversation or agent identities as the primary scope model.

## Lifecycle model

The schema should support, at minimum:

- `active`
- `superseded`
- `archived`
- `draft` if workflows require non-active authoring

Only `active` blocks should be included by default.

## Provenance requirements

The schema must preserve enough provenance to answer:

- who created this block,
- who last changed it,
- what it superseded,
- why it was archived or superseded,
- and why it was included or omitted from prompt compilation where diagnostics exist.

## Relationship to other stores

### Transcript store

Prompt blocks must not be stored as if they were transcript events.

### Durable memory store

Prompt blocks may reference or derive from durable memory, but should not require all durable memory to become prompt blocks.

### Retrieval/index store

Prompt blocks are standing scoped prompt inputs, not search results or embeddings.

## Minimum v1 schema bar

A v1 schema is acceptable if it provides:

- block identity,
- block kind,
- explicit scope,
- lifecycle state,
- content,
- supersession linkage,
- and auditability for mutation.
