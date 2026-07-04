# Memory Model

Bear memory is the durable knowledge a Bear can use across roles, work surfaces, channels, and time.

In the current architecture, canonical Bear cognition lives in **per-Bear SQLite**. Memory is not the same thing as transcript history, task state, or external retrieval indexes.

## Summary

- canonical Bear cognition is stored in per-Bear SQLite
- shared knowledge and role-local knowledge are distinct
- work-surface grounding matters as much as Bear-global memory
- transcript history is not the Bear's memory store
- Docket jobs/tasks are infrastructure, not cognition
- derived recall indexes support retrieval but are not canonical truth

## Core distinctions

### Shared vs role-local memory

Shared memory is durable Bear knowledge that should be usable across roles and surfaces.

Role-local memory is scoped knowledge that may remain local indefinitely or later be promoted.

### Bear-global vs work-surface-local memory

Some memory is useful across the whole Bear.

Examples:

- charter and purpose
- glossary terms
- shared conventions
- broad architectural facts

Some memory is specific to one work surface.

Examples:

- repository architecture
- service-specific terminology
- local decisions and conventions
- current understanding of one project or deployment

### Memory vs transcript

- transcript history records what happened in a conversation or tool exchange
- memory records what the Bear should keep and reuse later

### Memory vs tasks

- memory is Bear cognition
- tasks/jobs are managed work state in Docket and Den Postgres

## Canonical storage

Per-Bear SQLite is the canonical store for:

- memory records
- links and supersession chains
- proposals and observations
- promotions and curation outcomes
- reflection outcomes tied to cognition

Den Postgres stores related but non-cognitive state such as:

- conversations and transcript artifacts
- approvals and client obligations
- prompt-memory blocks
- Docket tasks/jobs
- reflection queue and scheduling state

## Work-surface grounding

Many user questions are really about one current work surface rather than Bear memory in the abstract.

Examples of work surfaces:

- a repository
- a local workspace
- a service or deployment
- a Docket project
- a Cabinet Mission
- a long-running responsibility

Recommended retrieval order for local-understanding questions:

1. current conversation and trusted session briefing
2. current role/channel/work-surface resolution state
3. canonical work-surface anchors
4. work-surface role-local memory
5. Bear-global shared anchors
6. broader Bear memory search
7. direct artifact inspection or external docs
8. general world knowledge

## Canonical memory layout

The architecture still uses familiar logical paths such as `core/` and role-local branches, but those are logical-path projections over canonical memory records rather than a separate canonical filesystem.

Important conceptual areas:

| Logical area | Meaning |
|--------------|---------|
| `core/` | shared curated Bear memory |
| `pair/` | pair-local notes and learnings |
| `chat/` | chat-local notes and learnings |
| `review/` | review/curation-local notes |
| `work/` | execution-local notes and summaries |
| `watch/` | observation-local notes |

## Promotion and curation

Memory does not automatically become shared truth.

Typical flow:

1. a role writes local memory, an observation, or a proposal
2. review/curation examines it
3. it is retained locally, summarized, promoted, superseded, or rejected
4. if promoted, shared memory is updated in canonical Bear cognition

## Derived recall

Semantic retrieval is a **derived recall layer**, not a second memory truth.

It exists to support:

- vector-style retrieval over canonical memory and approved linked sources
- bounded turn-start recall
- hybrid on-demand search

Canonical truth remains in SQLite memory records and curated shared memory.

## Prompt-time memory use

At turn time, the runtime does not dump all Bear memory into the prompt.

Instead it assembles:

- compiled role prompt
- key memory projection
- optional derived recall passages
- prompt-memory blocks
- runtime supplements

See [den-native-runtime](den-native-runtime.md) for the exact context assembly model.

## Related docs

- [den-native-runtime](den-native-runtime.md)
- [bears and den](bears-and-den.md)
- [pair role](pair-role.md)
- [reflection system](reflection-system.md)
- [tasks and autonomy](tasks-and-autonomy.md)
