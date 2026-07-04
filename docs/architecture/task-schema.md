# Task Schema

This document describes the architectural shape of Bear Den task and task-result records at a level useful for system design and integration planning.

The canonical work-management model is [ADR-0034: Jobs and Tasks Work-Management](../decisions/adr-0034-jobs-and-tasks-work-management.md). This page is the architecture-facing summary of the objects an expert should expect to exist and how they relate.

## Summary

Bear Den distinguishes between:

- interactive planning and handoff state;
- canonical Docket work-management records;
- execution runs and results;
- and optional promoted knowledge derived from those results.

## Core objects

| Object | Owner | Purpose |
|--------|-------|---------|
| Work request / handoff | Den-facing interactive flow | capture a proposed transition from interactive work into managed background work |
| Job | Docket | larger work container or workflow grouping |
| Task | Docket | concrete unit of approved or schedulable work |
| Run | Docket + runtime | one execution instance of a task |
| Result | Docket/runtime state | output, status, and summary of a run |
| Promoted knowledge | Bear memory | durable learnings extracted from results when reviewed |

## Typical fields

Architecturally, a task or run record usually needs to capture some combination of:

- durable id
- originating Bear and role
- requesting human or source surface
- work-surface attachment
- desired outcome
- current lifecycle/status
- execution scope and allowed tools
- approval/audit metadata
- scheduling or trigger metadata
- result summary and artifact references

## Lifecycle shape

Typical lifecycle progression:

1. interactive request or handoff is created
2. task or job is approved/refined/scheduled
3. task is dispatched
4. one or more runs execute
5. results are recorded
6. follow-up planning, review, or memory promotion may occur

## Relationship to plans

- a workboard plan is not the same thing as a Docket task
- a plan may lead to a task
- a task may update plan state
- a plan artifact may justify or explain a task

## Relationship to memory

- tasks are infrastructure, not canonical Bear cognition
- results may produce memory proposals or curated summaries
- role-local notes and shared memory are separate from task records

## Relationship to approvals

Tasks can have multiple approval points:

- approval to convert a handoff/request into managed work
- approval to execute a high-risk run
- approval to promote resulting knowledge into shared Bear memory

These approvals are part of Den-managed control state, not informal prompt-only behavior.

## Related docs

- [tasks and autonomy](tasks-and-autonomy.md)
- [planning](planning.md)
- [den-native-runtime](den-native-runtime.md)
- [ADR-0034: jobs and tasks work-management](../decisions/adr-0034-jobs-and-tasks-work-management.md)
