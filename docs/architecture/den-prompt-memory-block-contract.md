# Den Prompt Memory Block Contract

This document defines the implementation-facing contract for Den-owned editable in-context memory blocks.

It exists to replace Letta-style editable prompt memory with a Den-native model that is explicit, auditable, and clearly separated from transcript history, archival retrieval, and broader Bear memory.

## Purpose

Den needs a first-class model for editable prompt memory that can be attached to a Bear, role, work surface, or other bounded execution scope and then compiled into runtime context.

This contract defines:

- what a prompt memory block is,
- how it differs from other memory and transcript state,
- the minimum block scopes and lifecycle semantics needed for migration,
- and how prompt assembly should include blocks without recreating hidden provider-owned state.

## Non-goals

This contract does not define:

- the final persistence schema,
- exact UI/editor affordances,
- exact block-authoring prompts,
- or a generalized provider-facing block API.

A monolithic Den implementation is acceptable. The target is Den ownership, not a pluggable provider framework.

## Canonical terms

### Prompt memory block

A **prompt memory block** is an editable, Den-owned context object intended for direct inclusion in runtime prompt assembly.

A prompt memory block is not just searchable memory. It is a deliberate in-context input used to shape future reasoning and action.

### Block scope

A **block scope** is the bounded attachment target that determines where a prompt memory block may apply.

Initial supported scopes should include:

- Bear-wide,
- role-local,
- work-surface-attached,
- and session-scoped where explicitly required.

### Block attachment

A **block attachment** is the association between a block and the scope it applies to.

Attachment should be Den-owned and explicit. We should not model provider-managed agent attachment as the conceptual source of truth.

### Block lifecycle

A **block lifecycle** describes whether a block is active, superseded, archived, or otherwise no longer included by default.

### Prompt compilation

**Prompt compilation** is the Den-owned process that selects, orders, and budgets prompt memory blocks alongside instructions, workflow state, transcript context, and other runtime inputs.

## Core separation of concerns

Den should treat these as distinct concepts:

- **canonical transcript history** — durable ordered session/runtime history,
- **prompt memory blocks** — editable in-context state for direct prompt inclusion,
- **durable memory** — broader long-lived Bear memory in `core/` or role-local areas,
- **archival retrieval** — derived indexed recall over canonical sources,
- **derived compaction artifacts** — prompt-bounding artifacts for older session history.

A prompt memory block must not silently stand in for transcript history, archival retrieval, or durable memory in general.

## Invariants

### 1. Prompt memory blocks are Den-owned editable objects

The source of truth for block identity, content, scope, and lifecycle must be Den-owned.

### 2. Prompt blocks are explicit prompt inputs

If a block is included in runtime context, prompt assembly should know it is a block-derived input rather than raw transcript or generic retrieval output.

### 3. Prompt blocks are auditable

Block creation, mutation, supersession, archival, and deletion intent should be attributable.

### 4. Prompt blocks are scoped

No block should be treated as universally applicable by accident. Every block must have an explicit scope and inclusion policy.

### 5. Prompt blocks are not durable memory promotion by default

A block may be derived from durable memory or may later inform durable memory, but block mutation is not itself a memory-promotion flow.

## Minimum v1 block types

A v1 replacement only needs a small set of block types.

### Role guidance block

Short editable guidance for one role, such as working norms, local constraints, or persistent tactical reminders.

### Work-surface context block

Editable context attached to one work surface, such as local architecture reminders, current project assumptions, or active subsystem caveats.

### Session focus block

Short-lived context attached to the current session when a user or runtime needs a durable focus note across several turns.

### User or operator instruction block

A bounded editable instruction-like block created through explicit Den-owned workflows when needed for stable behavior within an allowed scope.

## Scope and attachment requirements

### Bear-wide

Use for broad prompt-shaping context that is safe and relevant across roles or surfaces.

This should be rare and curated.

### Role-local

Use when the block is appropriate for one role but should not automatically shape other roles.

### Work-surface-attached

Use when the block describes one repo, service, mission, or other work surface and should travel with that surface rather than with a provider-side conversation object.

### Session-scoped

Use sparingly for short-lived focus/context that should persist across turns in one session but not become broader default context.

## Lifecycle and mutation semantics

Prompt memory blocks should support, at minimum:

- create,
- edit,
- supersede,
- archive,
- and delete-request semantics where allowed.

Recommended lifecycle states:

- `active`
- `superseded`
- `archived`
- `draft` if a workflow requires review before activation

Mutation expectations:

- block edits should preserve provenance,
- supersession should be explicit rather than silent overwrite when meaning materially changes,
- and archived or superseded blocks should not be included by default in prompt assembly.

## Prompt compilation contract

Prompt compilation should select blocks separately from transcript and retrieval inputs.

At minimum, compilation should define:

1. **Eligibility**
   - which scopes are relevant to the current runtime context,
   - which lifecycle states are includable,
   - and which roles/channels may consume the block.

2. **Ordering**
   - system/developer instructions first,
   - then active role/runtime policy,
   - then eligible prompt memory blocks,
   - then workflow/workplan state,
   - then transcript and compaction context,
   - then retrieval inputs where applicable.

3. **Precedence**
   - more specific scopes override or outrank broader scopes when they conflict,
   - session-scoped blocks outrank role-local blocks for the current session,
   - work-surface-attached blocks outrank Bear-wide defaults for that surface.

4. **Budgeting**
   - blocks should have bounded size,
   - prompt assembly should be able to truncate or omit lower-priority blocks rather than flattening everything together,
   - omitted blocks should be explainable in diagnostics where practical.

## Relationship to other memory systems

### Versus transcript history

Transcript history records what happened. Prompt blocks express editable standing context for future prompt assembly.

### Versus durable memory

Durable memory is broader Bear knowledge. Prompt blocks are the smaller subset intentionally compiled into active runtime context.

### Versus archival retrieval

Retrieval returns relevant indexed content at query time. Prompt blocks are standing scoped context selected by policy rather than search.

### Versus compaction artifacts

Compaction artifacts summarize older session history. Prompt blocks are editable scoped context objects and should not be treated as rolling transcript summaries.

## Migration requirements

A Letta replacement is acceptable only if Den can represent the responsibilities Letta-style prompt memory served, without reproducing provider-shaped hidden state.

That means Den must be able to:

- attach editable prompt context to explicit scopes,
- mutate that context with provenance,
- compile blocks into prompt context by explicit policy,
- and separate block state from transcript, retrieval, and durable memory.

## Minimum v1 expectations

A v1 implementation is acceptable if it provides:

- Den-owned block identity and scope,
- at least Bear-wide, role-local, and work-surface-attached scopes,
- explicit lifecycle state,
- prompt-compilation inclusion and precedence rules,
- and enough provenance to audit block changes and inclusion behavior.
