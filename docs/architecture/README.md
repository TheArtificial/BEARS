# Architecture

Conceptual models, stable contracts, and architecture overviews for Bear Den.

> **Current direction (2026-06):** Bear Den is migrating to a single Den-native, in-process agent runtime — Letta, Letta Code/Codepool, and the git MemFS memory sidecar are being removed; Bear memory/cognition is canonical in per-Bear SQLite ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)), and tasks/jobs are Docket-canonical in Den Postgres ([ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md)). **Read [den-native-runtime.md](den-native-runtime.md) first** — it is the architecture source of truth and supersedes Letta-era framing in the docs below. Migration plan: [DEN_NATIVE_RUNTIME_PLAN.md](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md).

## Suggested reading order

- [den-native-runtime (target architecture)](den-native-runtime.md)
- [den crate architecture](den-crate-architecture.md)
- [context compilation scenarios](context-compilation-scenarios.md)
- [prompt fragment registry](prompt-fragment-registry.md)
- [overview](overview.md)
- [bear roles](bear-roles.md)
- [bears and den](bears-and-den.md)
- [den architecture](den-architecture.md)
- [memory model](memory-model.md)
- [tasks and autonomy](tasks-and-autonomy.md)
- [task schema](task-schema.md)
- [role vocabulary](role-vocabulary.md)

## Contents

### Core concepts
- [bear roles](bear-roles.md)
- [bears and den](bears-and-den.md)
- [bear charter and cabinet missions](bear-charter-and-cabinet-missions.md)
- [identity and membership](identity-and-membership.md)
- [capabilities and skills](capabilities-and-skills.md)
- [planning](planning.md)

### Runtime and systems
- [den-native-runtime (target architecture, canonical)](den-native-runtime.md)
- [den crate architecture](den-crate-architecture.md)
- [context compilation scenarios](context-compilation-scenarios.md)
- [prompt fragment registry](prompt-fragment-registry.md)
- [overview](overview.md)
- [workflow state overview](workflow-state-overview.md)
- [den architecture](den-architecture.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [den bear spec](den-bear-spec.md)
- [den concepts overview](den-concepts-overview.md)
- [den conversation runtime schema](den-conversation-runtime-schema.md)
- [agent and bear environments](agent-and-bear-environments.md)
- [bear environment tool contract](bear-environment-tool-contract.md)
- [pair role](pair-role.md)

### Memory, reflection, and work
- [memory model](memory-model.md)
- [observations and subscriptions](observations-and-subscriptions.md)
- [reflection system](reflection-system.md)
- [reflection run taxonomy](reflection-run-taxonomy.md)
- [tasks and autonomy](tasks-and-autonomy.md)
- [task schema](task-schema.md)

### Migration and terminology
- [den-native-runtime (target architecture)](den-native-runtime.md) — canonical post-Letta architecture
- [role vocabulary](role-vocabulary.md)
- [letta dependency matrix](letta-dependency-matrix.md) — historical migration inventory
