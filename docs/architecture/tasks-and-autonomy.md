# Tasks and Autonomy

Tasks are how Bear Den turns a request, observation, or plan into approved work that can execute outside the immediate interactive turn.

In the current architecture, work management is **Docket-canonical in Den Postgres**. Tasks are infrastructure the Bear uses, not part of canonical Bear cognition.

## Summary

- interactive roles may propose or request broader work;
- Den records and manages that work through Docket;
- `work` executes approved background work within scoped policy;
- `review`/curation and operators can gate or interpret higher-risk work;
- results can later feed memory, plans, or follow-up tasks.

## Why tasks exist

Some requests should not execute immediately inside a chat or pair turn.

Examples:

- recurring status checks
- longer research or synthesis
- scoped external actions
- work that should be auditable and resumable
- work that needs sandboxed execution

These become Docket work rather than ad hoc side effects.

## Core lifecycle

1. A human, stance, or event produces a work request.
2. Den records that request in work-management state.
3. Review/policy/human controls approve, refine, reject, or schedule it as needed.
4. Den dispatches approved work to `work`.
5. `work` executes within scoped tools, approvals, and runtime context.
6. Results are stored durably and may later feed memory promotion, summaries, or follow-up work.

## Role responsibilities

| Role/System | Responsibility |
|-------------|----------------|
| `chat` | capture user-facing requests that should become background work |
| `pair` | create or request handoff from active collaboration into broader work |
| `review` / `curate` | review, approve, reject, or refine work where policy requires it |
| Den | canonical work-management state, scheduling, policy, audit, dispatch |
| `work` | execute approved work within scoped runtime/tool boundaries |
| `watch` | produce observations that may trigger or motivate later work |

## What autonomy means

Autonomy in Bear Den is not unconstrained agent freedom.

It means work that is:

- requested, derived, or scheduled through a control path;
- bounded by stance and tool policy;
- auditable;
- resumable;
- and separate from the immediate interactive turn.

## Work surface continuity

Task and job records should attach to durable work surfaces when possible.

That allows:

- `pair` to create a handoff from a local repo or project context
- `work` to re-materialize that same ongoing work later
- results, plans, and memory to stay connected to the same work identity

## Results and learning

Task results do not automatically become shared Bear knowledge.

Typical flow:

1. work completes or updates the task
2. Den stores result/progress state
3. review/curation may summarize or promote durable learnings
4. shared Bear memory is updated only when appropriate

## Product language

Prefer:

- “approved background work”
- “scoped autonomous execution”
- “the Bear requested a handoff into Docket work”
- “`work` executed within an approved scope”

Avoid:

- “the chat stance can just do it later”
- “autonomy bypasses review or policy”
- “tasks are the Bear's memory”

## Related docs

- [planning](planning.md)
- [task schema](task-schema.md)
- [memory model](memory-model.md)
- [bears and den](bears-and-den.md)
- [ADR-0034: jobs and tasks work-management](../decisions/adr-0034-jobs-and-tasks-work-management.md)
