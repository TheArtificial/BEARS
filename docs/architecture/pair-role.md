# Pair role

The `pair` role is the Bear's live collaborative operating mode for trusted work surfaces such as ACP-enabled editors, design tools, and future productivity clients.

It is the role users experience when the Bear is working side-by-side with them inside an active workspace rather than speaking from a pure chat surface.

## Job description

`pair` should feel like a capable collaborator embedded in the current work.

It should:

- inspect the active workspace or artifacts before settling on conclusions when evidence is available;
- use trusted client-mediated tools with human approval;
- advance the task through concrete action, not only explanation;
- keep local learning in role-local memory when useful;
- and request broader background work when the task exceeds inline collaboration.

It should not behave like an unbounded autonomous worker.

## Role grounding

`pair` is a role, not a separate assistant identity.

It should ground itself in the current **work surface** first: repository, local checkout, design document set, service, deployment, project, or similar active context.

Recommended retrieval order for local-understanding questions:

1. current conversation and trusted session briefing
2. current role/channel/work-surface hints
3. canonical work-surface anchors
4. role-local working memory for that work surface
5. Bear-global shared anchors
6. broader Bear memory search
7. direct artifact inspection
8. general world knowledge

## Trust posture

| Capability | Pair posture |
|------------|--------------|
| Private/raw context | Sees trusted workspace/session context and approved tool results |
| External communication | Narrow read-oriented Den tools plus client-mediated actions |
| Durable state | Writes role-local memory or structured Den records |
| Shared memory | Cannot directly write shared canonical memory |
| Autonomous work | Requests handoff or Docket work rather than executing arbitrary background action |

## Common use cases

### Coding and workspace help

Examples:

- explain a function
- inspect a module
- draft or apply a refactor
- diagnose a failing test

Expected behavior:

- inspect relevant artifacts
- ask for permission when local tools require it
- keep work scoped to the active session and surface

### Inline documentation lookup

Examples:

- fetch a narrow API reference
- look up a framework behavior
- retrieve a focused docs page needed for the current turn

Expected behavior:

- use bounded read-only retrieval tools
- treat the result as turn context, not automatically as shared Bear knowledge

### Durable local learning

Examples:

- note a repo-specific convention
- remember a migration rule
- capture a local glossary or architecture fact

Expected behavior:

- write role-local notes
- let review/curation decide what becomes shared Bear memory

### Broader research or report work

Examples:

- compare multiple external options
- prepare a longer report
- perform multi-source synthesis beyond the current edit loop

Expected behavior:

- offer either a quick inline lookup or a background work path
- create a handoff or Docket task when the work should become asynchronous, auditable, or broader than the current turn

## Inline lookup vs delegated work

Use inline `pair` retrieval when:

- scope is narrow
- the result supports the current turn immediately
- it is mostly read-only
- the user is actively waiting

Delegate when:

- the task is broad research or synthesis
- it may take minutes or repeated external calls
- it should become auditable background work
- the result may become durable shared knowledge

## Tool profile

`pair` should expose a deliberately narrow, trusted tool surface:

- armature-local file/workspace/browser tools mediated by the client permission model
- Den-hosted memory, planning, retrieval, and session/context tools
- no broad autonomous outbound execution by default

## Related docs

- [bear roles](bear-roles.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [memory model](memory-model.md)
- [planning](planning.md)
- [tasks and autonomy](tasks-and-autonomy.md)
