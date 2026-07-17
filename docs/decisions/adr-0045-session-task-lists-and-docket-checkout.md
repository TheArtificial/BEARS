# ADR-0045: Session task lists as Docket checkouts and working projections

**Status:** Proposed  
**Date:** 2026-06-20  
**Amends:** [ADR-0034: Jobs and Tasks Work-Management Model](adr-0034-jobs-and-tasks-work-management.md), [ADR-0027: Workflow-state ontology](adr-0027-workflow-state-ontology.md)  
**State inventory:** [Den state machine inventory](../architecture/den-state-machine-inventory.md)

> **State-inventory maintenance.** This ADR owns session task-list projections, Docket checkout/sync boundaries, and the task-list continuation gate. Changes to task-list source-of-truth rules, focus authority, or stop/continue behavior must update the state inventory and include tests that stale projections cannot force continuation.

## Context

BEARS has accumulated several task/planning concepts that are useful but easy to confuse:

- **Roadmaps / implementation plans / workplan artifacts** — durable human-readable planning documents in a work surface, Cabinet, or workplan-domain artifact store.
- **Docket jobs** — durable orchestration containers in Den Postgres.
- **Docket tasks** — durable task nodes in a Docket job hierarchy, with run-scoped execution state, criteria, audit, and dispatch semantics.
- **Session task lists** — the visible working task list used by `pair` or `work` during a session/run to maintain focus, expose progress, and coordinate with a human.

ADR-0034 intentionally made Docket canonical for jobs/tasks and described “session-bound tasks” in the same task model. That was directionally useful, but it risks making session focus state and Docket canonical task records look interchangeable. Conversely, treating session task lists as merely scratch state is too weak: a Bear in `pair` or `work` stance should be able to check out part of a Docket job’s task hierarchy, work it through the session task list, and sync changes back to Docket.

We need a practical model that makes task work effective without collapsing source-of-truth boundaries.

## Decision

A **session task list** is the Bear’s working view of tasks for a session/run. It is an editable working projection that may contain both:

1. **local-only task-list items** — session-scoped items with no canonical Docket backing record; and
2. **Docket-backed task-list items** — checked-out projections of Docket tasks, typically the children of a parent task in a Docket job hierarchy.

A session task-list item is therefore **not inherently identical** to a Docket task. It may be local-only or it may reference a Docket task. When it references a Docket task and policy allows, edits to the task-list item may sync back to Docket.

Docket remains canonical for:

- jobs;
- durable task identity;
- parent/child task hierarchy;
- task criteria and dispatch contracts;
- run-scoped task execution state;
- audit history and conflict detection.

The session task list remains canonical only for its own session-local working projection and UI/focus state.

Session task lists are **session- and stance-local by default**. Passive prompt injection should include only the task list owned by the current session/conversation and stance. A `pair` session task list must not appear as active prompt context for a `chat` stance turn merely because it is visible to the Bear. Other stances may inspect visible task lists through explicit tools when policy allows, but cross-stance passive context is reserved for Docket-backed/promoted work.

A task list becomes appropriate cross-stance durable work context only when its items are attached/synced/promoted to a Docket job/task hierarchy, or when a user explicitly asks a stance to inspect another session's task-list projection.

## Terms

| Term | Meaning |
| --- | --- |
| Roadmap / implementation plan | Durable document or artifact describing intended work. Not a task list. |
| Workplan artifact | Reviewable workplan-domain artifact, often created in planning mode. Not Docket execution state. |
| Docket job | Durable orchestration container for work. Bigger than an individual task. |
| Docket task | Durable task node in a job hierarchy. May have child tasks and run-scoped state. |
| Session task list | Visible working task list for `pair`/`work` stance in a session/run. |
| Task-list item | Item in a session task list; may be local-only or Docket-backed. |
| Checkout | Creating/updating a session task list from a Docket job/task subtree or other source. |
| Sync | Applying authorized task-list changes back to backing records such as Docket tasks. |
| Handoff | Requesting review/promotion/reconciliation of local task-list items or unsynced changes into durable Docket work. |

## Model

The effective relationship is:

```text
Docket job task tree        canonical source of truth
        │
        │ checkout / project
        ▼
session task list           Bear/human working view
        │
        │ edit / execute / mark progress
        ▼
sync or handoff
        │
        ▼
Docket job task tree        updated canonical state when policy allows
```

A session task-list item should carry source metadata when it is backed by durable state:

```json
{
  "id": "item-local-or-docket-id",
  "title": "Add BearWire web_fetch approval dialog",
  "status": "in_progress",
  "source_ref": {
    "kind": "docket_task",
    "job_id": "job_123",
    "task_id": "task_456"
  },
  "sync": {
    "state": "dirty",
    "mode": "direct"
  }
}
```

A local-only item uses a local source:

```json
{
  "id": "investigate-1",
  "title": "Inspect BearWire permission result path",
  "status": "completed",
  "source_ref": {
    "kind": "local"
  },
  "sync": {
    "state": "local_only"
  }
}
```

Suggested sync states:

- `local_only`
- `checked_out`
- `clean`
- `dirty`
- `syncing`
- `synced`
- `conflict`
- `review_required`

## Consequences

### Model-facing tools

Model-facing tool names should avoid the overloaded “plan” term for the visible session checklist. Prefer:

- `list_task_lists`
- `get_task_list_status`
- `update_task_list`
- `checkout_task_list`
- `sync_task_list`
- `request_task_list_handoff`

Legacy provider names such as `list_plans`, `get_plan_status`, `update_plan`, and `request_work_handoff` may remain accepted aliases during migration, but should not be the advertised names once descriptors are updated.

### Tool semantics

`update_task_list` should be allowed to update the visible session task list. Passive prompt context for task lists must be scoped to the current session/conversation and stance; broader visibility is for explicit read tools, not automatic prompt bleed. If task-list items are Docket-backed and policy allows direct sync, those changes may update Docket tasks. If policy requires review, the tool should preserve changes in task-list state and mark them `review_required` or ask for handoff/sync review.

`checkout_task_list` should create or refresh a session task list from a source such as:

- a Docket job;
- the children of a Docket parent task;
- a roadmap section;
- a local/human-provided checklist.

`sync_task_list` should apply pending changes to backing records when allowed and surface conflicts when not.

`request_task_list_handoff` should request review or promotion of local-only items or unsynced task-list edits into canonical Docket jobs/tasks or other durable work records.

### Runtime behavior

A Bear in `pair` or `work` stance may:

- check out the children of a parent Docket task into a session task list;
- complete, modify, split, reorder, or add task-list items while working;
- sync authorized changes back to Docket;
- surface conflicts in the task list when Docket changed underneath the session;
- keep local-only task-list items when the work is session-scoped or exploratory.

This is an effectiveness-oriented workflow, not a purity boundary. The boundary exists to preserve source-of-truth, audit, concurrency, and dispatch semantics.

### Continuation and stop invariant

The native runtime's autonomous continuation gate is tied to **active activity/task-list state**, not to prompt wording alone.

- For `pair`, the continuation gate activates only when there is an active session task list/activity plan for the current session/conversation and stance.
- For `pair` without an active task list, normal interactive behavior remains allowed.
- For `work`, an active session task list/activity plan is required runtime state, not an optional aid.
- For `work` without an active task list, the runtime must not fall back to generic conversational behavior; it must stop with an explicit missing-task-list blocker.

When the gate is active:

- progress-only final answers are not valid terminal responses while incomplete, unblocked task-list items remain;
- valid terminal responses are limited to completed work, a hard blocker, or a required safety/permission stop;
- interruption recovery should resume from the next incomplete unblocked task-list item rather than merely summarizing progress.

The gate must evaluate **incomplete actionable** items, not merely every non-completed item. A task-list item may also end in a terminal non-completion outcome such as blocked, not applicable, waived, cancelled, or unsafe/permission-gated. A reasoned non-action with evidence is a valid terminal outcome, not a failure to proceed. For example, a “Commit changes” item may be marked not applicable or cancelled when there are no relevant changes, only unrelated dirty files, no commit authorization, or committing would be misleading.

Model-facing continuation instructions should therefore say:

> Continue until the task is finished, blocked, not applicable, waived, or permission-gated. If the next planned action is inappropriate, mark that item with a terminal non-completion state and report the evidence.

The runtime must not force an assistant to perform an unsafe, unauthorized, empty, or misleading action merely because the item remains unchecked.

### Runtime checkpoint/task-list non-interference invariant

Runtime progress checkpoints are loop-control scaffolding, not task-management records.

A checkpoint may reference the active session task list, task-list item, Docket task, run id, and current task-list version. It may summarize what the model has learned, identify remaining uncertainty, and propose the next action. It must not itself mutate task-list or Docket state.

Only task-management tools such as `update_task_list`, `sync_task_list`, `checkout_task_list`, and `request_task_list_handoff` may change task-list state or request propagation into Docket. Docket-backed mutations remain subject to source/sync policy.

The continuation gate must evaluate active task-list/Docket state, not checkpoint prose. A checkpoint saying that work is done, blocked, waived, cancelled, unsafe, permission-gated, or not applicable is insufficient unless the corresponding task-list item is updated with evidence through the task tools.

If a checkpoint identifies that the task list is stale or misclassified, the next action should normally be one of:

- update the session task-list item with evidence;
- request handoff/review for local-only or unsynced changes;
- sync authorized changes to Docket;
- continue execution if no state change is justified.

### Task-gate rejection loops

The task outcome gate can itself create a loop if a model repeatedly tries the same invalid terminal response while the task list still has an actionable item. Den should treat repeated gate rejections as a first-class loop signal, distinct from tool-call rule-of-ko.

Required behavior:

- Track task-gate rejection count per active run/session.
- Fingerprint each rejected terminal attempt using the task-list id/version, next actionable item, final-response kind, and normalized assistant text.
- On the first rejection, nudge the model to continue or mark the item blocked/cancelled/not-applicable with evidence.
- On a repeated matching rejection, strengthen the nudge and explicitly forbid repeating the same final answer.
- After a small threshold, stop forcing continuation and surface a concise blocker requiring human review or task-state update.

The threshold stop should not look like a raw runtime crash. It should produce a user-facing blocker such as:

> I am stopping because I repeatedly reached the same task gate. The remaining item appears to need a task-state update, permission, or human review.

Den must not silently mutate task-list state at the threshold unless the model explicitly supplied a blocker/non-action reason that can be recorded as evidence.

This invariant applies to both `pair` and `work`, but `work` is stricter:

- `pair` may still terminate for genuine clarification when the active task list exists but safe continuation depends on missing user intent;
- `work` should prefer continued execution and should treat absence of an active task list as a runtime configuration/state error, not as an invitation to chat.

### Docket invariants retained

This ADR does not weaken ADR-0034’s execution invariant:

- Docket still orchestrates and records; it does not execute task bodies itself.
- Execution flows through Bear role runtimes.
- Docket remains the source of truth for Docket jobs/tasks and their hierarchy.
- Session task lists are a working view and sync surface, not a replacement store for Docket.
- Runtime checkpoints can guide execution but cannot substitute for session task-list updates, Docket events, or sync/handoff records.

## Non-goals

- Do not rename durable Docket jobs to tasks.
- Do not call roadmaps or workplan artifacts “task lists.”
- Do not force every session task-list item to be Docket-backed.
- Do not require every task-list edit to immediately mutate Docket.
- Do not hide source/sync state from Den just to make the UI vocabulary simpler.
- Do not treat runtime checkpoints, status text, or assistant summaries as task-list/Docket state changes.

## Migration notes

Near-term code may still use `bear_work_plans` and `den.work_plan.*` internally for the legacy activity board. Descriptors and model-facing guidance should migrate toward task-list language before internal table/module renames.

When the relational Docket schema lands, the checkout/sync model in this ADR should guide the transition from legacy `bear_work_plans.items` to Docket-backed task-list projections.
