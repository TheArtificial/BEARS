# Bears and Den

A **Bear** is the durable assistant identity users interact with. **Den** is the system that hosts, governs, routes, and persists Bears.

This document explains the product boundary between Bear identity and Den infrastructure.

## Summary

- A Bear is the user-facing assistant identity.
- Den is the runtime and control plane that makes that identity real.
- A Bear spans stances, work surfaces, conversations, memory, and tasks.
- Den spans policy, routing, approvals, persistence, scheduling, and runtime execution.

## What a Bear is

A Bear is one coherent assistant from the user's perspective.

A Bear can:

- remember durable knowledge;
- converse across multiple surfaces;
- collaborate in trusted work surfaces through `pair`;
- create and consume tasks and plans;
- learn through review and reflection;
- and participate in background work and event handling through specialized roles.

Internally, a Bear operates through multiple roles, but those roles are not separate assistants. They are controlled capability profiles over one system-owned runtime model.

## What Den is

Den is the infrastructure and control plane for Bears.

Den is responsible for:

- Bear and human identity management;
- access control and membership;
- routing surfaces to roles and trust boundaries;
- running the native turn loop;
- managing prompts, context assembly, and tool surfaces;
- storing canonical conversations, approvals, and work state;
- scheduling reflection and autonomous work;
- and enforcing policy over memory, tasks, and external action.

Users talk to Bears. Den is the system that hosts them safely and consistently.

## What a Bear is not

A Bear is not:

- a single conversation;
- a single stance;
- a channel bot alone;
- an IDE session alone;
- a task record alone;
- or Den itself.

## What Den is not

Den is not:

- the assistant persona;
- the model provider;
- the user's long-term knowledge by itself;
- or the thing the user thinks they are talking to.

Den owns execution, policy, and persistence. The Bear is the assistant identity that Den presents and maintains.

## Relationship to stances

Different surfaces engage different Bear stances:

| Surface or situation | Typical stance |
|----------------------|----------------|
| web chat, messaging, future conversational channels | `chat` |
| ACP and trusted work-surface collaboration | `pair` |
| reflection, review, promotion, approval | `review` / `curate` depending on vocabulary in scope |
| approved background execution | `work` |
| inbound webhooks, polling, and observation intake | `watch` |

The stance changes. The Bear identity remains stable.

## Relationship to work surfaces

A Bear does not operate in one flat undifferentiated world. It engages **work surfaces** such as repositories, services, deployments, Docket projects, Cabinet Missions, or long-running responsibilities.

Den helps attach plans, memory, tasks, observations, and runtime context to those work surfaces so the Bear can maintain continuity without confusing one domain of work for another.

## Product language

Prefer:

- “your Bear” for the assistant identity
- “Den hosts and manages Bears”
- “Bear stances” for stance contracts
- “membership” or “access roles” for human permissions

Avoid:

- “Den answered the user” except in infrastructure/operator contexts
- describing roles as separate assistants
- using “Bear stance” when you mean human access role

## Related docs

- [den bear spec](den-bear-spec.md)
- [bear stances](bear-stances.md)
- [memory model](memory-model.md)
- [tasks and autonomy](tasks-and-autonomy.md)
- [capabilities and skills](capabilities-and-skills.md)
- [identity and membership](identity-and-membership.md)
