# Bear Charter and Cabinet Missions

A Bear's **charter** is its durable purpose and responsibility boundary. A Cabinet **Mission** is a page in Cabinet whose subtree gathers the shared knowledge for an initiative, and which one or more Bears and people may work from.

They are related but not the same thing.

## Summary

- a charter is a property of a Bear, not a separate entity
- a Bear organizes work through domains, work surfaces, routines, tasks, and memory
- a Cabinet Mission is a **page** in Cabinet, not a container type of its own: its child pages are the grouped material, and its page policy/membership govern who may read and edit that subtree
- Missions carry knowledge, not work state: a Docket Job may name a Mission page, but the Mission holds no Job, task, or run identity
- if you need a separate identity, policy boundary, or memory boundary, you likely need another Bear

## Bear charter

A charter answers:

- why this Bear exists
- what responsibility boundary it owns
- what kinds of knowledge, work, and tools are in scope

Examples:

- care for the house
- build and operate the SaaS product
- coordinate executive administrative flow

## Bear-scoped organization

Useful Bear-level concepts include:

| Concept | Meaning |
|---------|---------|
| Domain | durable area of responsibility or knowledge, local to one Bear |
| Work surface | durable resource the Bear acts on (repository, design, server, document) |
| Job | durable work objective with a task tree ([ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md)) |
| Routine | recurring responsibility |
| Task | executable work unit |
| Run | one execution attempt |

There is no `Project` entity. A bounded initiative is expressed by a Docket Job
(the work) plus, when it needs shared documentation, a Cabinet Mission page
(the knowledge).

## Work surfaces vs Missions

A work surface is the durable resource the Bear acts on. A Mission page is
shared knowledge that may span several work surfaces, Jobs, and Bears.

Choose by what you are grouping:

- grouping work *by resource* (many Jobs against one repository) — that is a work surface, and no Mission is needed
- grouping *documentation* for an initiative that spans resources — that is a Mission page, with the material as its child pages
- one work surface may exist with no Mission at all
- one Bear may read and edit several Missions while retaining one charter

Because a Mission is an ordinary Cabinet page, it needs no separate lifecycle:
create it like any page, nest plans and references under it, and let its page
policy decide who may see the subtree. See the
[Cabinet contract](cabinet-contract.md) for the page-tree and policy rules.

## When to create another Bear

Create another Bear when the responsibility needs a separate:

- assistant identity
- memory boundary
- membership or privacy boundary
- tool/capability profile
- autonomy or policy boundary

## Related docs

- [bears and den](bears-and-den.md)
- [memory model](memory-model.md)
- [tasks and autonomy](tasks-and-autonomy.md)
