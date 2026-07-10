# Architecture

Conceptual models, stable contracts, and architecture overviews for Bear Den.

Bear Den is a **single Den-native system**:

- one in-process runtime loop owned by Den;
- direct inference through Bifrost;
- canonical Bear cognition in per-Bear SQLite;
- canonical jobs/tasks in Den Postgres through Docket;
- protocol edges such as ACP/BearWire, web chat, and future channels projecting the same core runtime.

There is no Letta, Letta Code/Codepool, or MemFS sidecar in the current architecture. Historical migration material may remain in the repo, but it is not part of the onboarding path for understanding the live system.

## Start here

If you need one linear path that yields a complete mental model without reading code, use this order:

1. [den runtime](den-runtime.md) — canonical runtime architecture and storage boundary
2. [overview](overview.md) — one-page system picture: components, flows, and responsibilities
3. [den crate architecture](den-crate-architecture.md) — how the implementation is decomposed inside the Rust workspace
4. [den bear spec](den-bear-spec.md) — what a Bear is, what stances exist, and what each stance is allowed to do
5. [bears and den](bears-and-den.md) — product identity vs control plane
6. [bear stances](bear-stances.md) — stance model and stance boundaries
7. [bear channel and ACP](bear-channel-and-acp.md) — channels, armatures, and trusted work surfaces
8. [context compilation scenarios](context-compilation-scenarios.md) — how prompt/context assembly behaves in practice
9. [non-blocking structured updates](non-blocking-structured-updates.md) — model-facing actions vs blocking tools, obligations, progress, and metadata updates
10. [runtime error UX policy](runtime-error-ux-policy.md) — how failures split across user copy, model continuity, and diagnostics
11. [memory model](memory-model.md) — canonical memory model and promotion boundaries
12. [reflection system](reflection-system.md) — how reflection, review, and curation operate
13. [tasks and autonomy](tasks-and-autonomy.md) — Docket work, approvals, and autonomous execution boundaries
14. [planning](planning.md) — workboard plans, plan mode, and plan artifacts
15. [capabilities and skills](capabilities-and-skills.md) — capability model, tools, and skill governance
16. [task schema](task-schema.md) — current task and task-result shapes
17. [stance vocabulary](stance-vocabulary.md) — canonical naming and terminology

## What this section should let you answer

Without reading the source, these docs should let you answer:

- what Den is and what a Bear is;
- how roles, channels, and armatures differ;
- where memory, tasks, approvals, and transcript state live;
- how a turn runs from user input to model output to tool execution;
- which parts of the system are protocol-neutral core and which are edge adapters;
- how reflection, planning, and autonomous work fit into the same architecture;
- and where the implementation lives in the Rust workspace.

## System at a glance

Bear Den consists of these architectural layers:

| Layer | Responsibility |
|------|----------------|
| Product model | Bears, roles, work surfaces, tasks, approvals, memory, and skills |
| Runtime core | Native turn loop, context assembly, tool orchestration, continuation, compaction, and event production |
| Persistence | Per-Bear SQLite for cognition; Den Postgres for conversations, approvals, identity, compiled configs, Docket, and schedulers |
| Tooling | Den-hosted tools, armature-local tools, external web/retrieval integrations, and sandbox execution |
| Edges | ACP/BearWire, web UI/chat, API surfaces, and future channel adapters |
| Inference | Bifrost as the unified model gateway |

## Reading paths by topic

### Runtime and execution

- [den runtime](den-runtime.md)
- [overview](overview.md)
- [den crate architecture](den-crate-architecture.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [context compilation scenarios](context-compilation-scenarios.md)
- [runtime error UX policy](runtime-error-ux-policy.md)

### Bear model and stances

- [den bear spec](den-bear-spec.md)
- [bears and den](bears-and-den.md)
- [bear stances](bear-stances.md)
- [pair stance](pair-stance.md)
- [stance vocabulary](stance-vocabulary.md)

### Memory, reflection, and learning

- [memory model](memory-model.md)
- [reflection system](reflection-system.md)
- [reflection run taxonomy](reflection-run-taxonomy.md)
- [capabilities and skills](capabilities-and-skills.md)

### Tasks, planning, and autonomy

- [tasks and autonomy](tasks-and-autonomy.md)
- [planning](planning.md)
- [task schema](task-schema.md)
- [workflow state overview](workflow-state-overview.md)

### Identity, governance, and scope

- [identity and membership](identity-and-membership.md)
- [bear charter and cabinet missions](bear-charter-and-cabinet-missions.md)
- [bear environment tool contract](bear-environment-tool-contract.md)

## Contents

### Core concepts

- [bears and den](bears-and-den.md)
- [bear stances](bear-stances.md)
- [bear charter and cabinet missions](bear-charter-and-cabinet-missions.md)
- [identity and membership](identity-and-membership.md)
- [capabilities and skills](capabilities-and-skills.md)
- [planning](planning.md)

### Runtime and systems

- [den runtime](den-runtime.md)
- [overview](overview.md)
- [den crate architecture](den-crate-architecture.md)
- [den bear spec](den-bear-spec.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [context compilation scenarios](context-compilation-scenarios.md)
- [prompt fragment registry](prompt-fragment-registry.md)
- [den concepts overview](den-concepts-overview.md)
- [workflow state overview](workflow-state-overview.md)
- [bear environment tool contract](bear-environment-tool-contract.md)
- [pair stance](pair-stance.md)

### Memory, reflection, and work

- [memory model](memory-model.md)
- [observations and subscriptions](observations-and-subscriptions.md)
- [reflection system](reflection-system.md)
- [reflection run taxonomy](reflection-run-taxonomy.md)
- [tasks and autonomy](tasks-and-autonomy.md)
- [task schema](task-schema.md)

### Reference and terminology

- [stance vocabulary](stance-vocabulary.md)
- [interactive stances and role axes](interactive-stances-and-role-axes.md)
- [task schema](task-schema.md)
- [den prompt memory block contract](den-prompt-memory-block-contract.md)

### Historical material

These remain useful for archaeology and migration history, but they are not part of the current architecture path:

- [letta dependency matrix](letta-dependency-matrix.md)
- [den architecture](den-architecture.md)
