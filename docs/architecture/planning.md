# Planning in Bear Den

Planning in Bear Den means a user-visible plan for active work and the surrounding control structures that let a Bear coordinate, pause, hand off, and resume that work.

Planning is separate from long-term memory and separate from Docket task execution, but it connects to both.

## Summary

- planning is the short- to medium-term structure around active work;
- workboard plans are operational progress state, not the entire project archive;
- pair mode (`Ask`, `Plan`, `Write`) shapes the interactive editing posture, not a separate runtime;
- plans can attach to work surfaces and later hand off into Docket work;
- plan artifacts may exist for approval, audit, or later reference.

## Two layers

### 1. Workboard progress tracking

The first layer is the lightweight todo/progress list the Bear updates while working.

Bear Den uses a Den-owned workboard for this:

- current visible steps
- one `in_progress` item at a time
- completion, blocking, and cancellation state
- stance provenance
- optional work-surface attachment

This is the live collaboration surface for multi-step edits, debugging, planning, and visible progress.

### 2. Pair session mode

ACP `pair` sessions use **Ask**, **Plan**, and **Write** modes.

These are client- and user-controlled workflow modes that shape the allowed tool posture and expected collaboration style:

1. Ask: read/search/inspect posture
2. Plan: explicit planning and proposal posture
3. Write: mutation/execution/browser posture, still subject to approval and policy

The model still uses workboard tools for planning; modes are not themselves model-operated planning tools.

## Planning objects

| Object | Owner | Purpose | Durability |
|--------|-------|---------|------------|
| Work surface | Den canonical model and anchors | Durable context for related work | durable continuity object |
| Workboard plan | Den DB | Current visible todo/progress state | resume/status durability |
| Plan artifact | Den-controlled artifact or approved workspace file | User-reviewable plan proposal or saved implementation plan | durable artifact |
| Docket task / job | Docket in Den Postgres | Approved background or autonomous work | canonical work-management record |
| Work result | Docket + related artifacts/memory | Outcome of approved work | durable result with optional curation |

## Work-surface continuity

Plans should attach to the same durable work surfaces used elsewhere in Bear Den.

A common flow is:

1. `pair` identifies the current repo/service/deployment/project;
2. it creates or updates a workboard plan attached to that work surface;
3. a plan artifact may be produced for human review or future reference;
4. if work becomes broader or asynchronous, the plan can hand off into Docket work;
5. later `work` runs against the same work surface rather than treating the task as unrelated.

## Pair behavior

`pair` should use planning for:

- non-trivial multi-step edits
- debugging loops
- user-visible progress tracking
- handoff preparation
- broader problem decomposition

`pair` should not force a plan for every tiny answer or one-step edit.

## Work behavior

`work` executes approved background work, not arbitrary channel-originated plans.

It may surface progress back into Den work-management state, but execution authority comes from approved Docket work, not from a visible planning list alone.

## Planning and memory

Planning state is not automatically shared Bear memory.

Use this ladder:

1. keep tactical progress in the workboard
2. use a plan artifact when approval, audit, or later retrieval matters
3. write stance-local memory when rationale should survive the current mini-project
4. request curation when lessons or decisions should become shared Bear knowledge
5. use Docket work when the task becomes autonomous/background execution

## Current implementation shape

The current architecture assumes:

- Den-owned workboard state and events
- ACP pair mode state
- work-surface-aware plan continuity
- handoff from active planning into Docket work
- runtime/context reminders that help the model use plans appropriately

## Related docs

- [tasks and autonomy](tasks-and-autonomy.md)
- [task schema](task-schema.md)
- [bear stances](bear-stances.md)
- [memory model](memory-model.md)
