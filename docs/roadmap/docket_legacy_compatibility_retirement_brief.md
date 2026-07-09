# Brief: Retire Docket legacy compatibility

## Mission

Retire Den's remaining Docket legacy compatibility layer so **Docket jobs/tasks** are the only durable work-management source of truth, while **session task lists** remain a checked-out working projection rather than a parallel task system.

This is a cleanup/refinement brief, not a new planning system. Prefer deleting compatibility code over adding adapters. Where compatibility must remain temporarily, isolate it, document the sunset condition, and make it non-canonical.

## Product intent

Den now has clear work-management concepts:

- **Docket job**: durable objective across turns.
- **Docket task**: durable unit of execution, investigation, or decision.
- **Session task list**: visible Bear/human working projection that may be local-only or checked out from Docket.
- **Plan mode / workplan artifact**: approval artifact flow for implementation plans, not Docket and not the visible task list.

The goal is to remove old naming, old storage assumptions, and compatibility paths that make these concepts look interchangeable.

## Core deliverable

Produce a clean, tested retirement of legacy Docket compatibility paths, including:

- inventory of remaining legacy references and behavior;
- removal or quarantine of obsolete compatibility code;
- canonical naming in APIs, tools, models, migrations, docs, and UI text;
- preservation or explicit migration/archive handling for existing data;
- regression coverage proving current Docket and session task-list semantics still work.

## Key concepts to preserve

### Docket remains canonical

Durable jobs/tasks belong in Docket. New durable work should use Docket APIs/tools directly.

### Session task lists are projections

A session task list is the live visible working view. It may project a Docket job/task subtree, but it is not the durable source of truth. Sync/checkout behavior should remain explicit.

### Plan mode is separate

Plan mode and workplan artifacts are for user approval of implementation plans. They should not masquerade as Docket jobs/tasks or session task lists.

### Compatibility is not product policy

If a legacy bridge exists only to support historical names or old clients, mark it as compatibility, keep it thin, and avoid letting new features depend on it.

## Suggested scope

### 1. Inventory legacy surfaces

Find and classify references to old concepts and names, including but not limited to:

- `WorkPlan`, `workplan`, `work_plan`, `bear_work_plans`, `activity board`, `plan_mode` where it is confused with Docket;
- compatibility text in model-tool descriptors;
- legacy DB tables, migrations, views, or conversion helpers;
- docs that describe superseded Letta/MemFS/workplan task behavior as current;
- tests that encode old behavior;
- UI labels that make task lists, workplans, and Docket jobs sound identical.

Do not mechanically delete every `workplan` reference. Plan-mode approval artifacts may still be legitimate. The important distinction is whether the reference represents obsolete Docket/task compatibility.

### 2. Define canonical terminology boundaries

Make the code and docs consistently reflect:

- Docket = durable jobs/tasks;
- session task list = checkout/projection/working view;
- plan mode/workplan artifact = approval artifact flow;
- activity/status memory = memory, not Docket;
- `work` = autonomous execution stance, where relevant, not a durable task model.

Prefer boring names. Avoid introducing a new umbrella abstraction unless one already exists and is clearly necessary.

### 3. Remove or quarantine compatibility code

For each legacy path, choose the smallest safe action:

1. Delete it if unused.
2. Replace it with canonical Docket/task-list behavior if behavior is still needed.
3. Move it behind an explicitly named compatibility/migration/archive boundary if existing data still needs it.

If data migration is required, add a small migration or backfill with idempotent behavior and clear verification.

### 4. Refine model tools

Review model-facing tools for confusing compatibility language.

Expected direction:

- `create_job`, `list_jobs`, `get_job`, `update_job`, `create_task`, `list_tasks`, `update_task`, and task status tools are canonical for durable work.
- `checkout_task_list` and `sync_task_list` are canonical for task-list projection behavior.
- Deprecated tools should either be removed, hidden, or have warnings that point to the canonical tool.
- Tool descriptions should not teach models to use old workplan/task APIs for durable work.

Keep provider/API compatibility only if needed by existing clients, and make it visibly deprecated.

### 5. Refine web/UI text

Update UI labels and help text so humans can tell the difference between:

- a Docket job;
- a Docket task;
- a session task list projection;
- a plan-mode approval artifact.

Do not redesign the UI unless necessary. Text cleanup and removal of misleading affordances are enough.

### 6. Update docs

Update roadmap, architecture, and decision docs that still present superseded behavior as current.

Use front-matter notes or short supersession callouts when preserving historical documents is better than editing them heavily.

At minimum, ensure current docs point readers to:

- ADR-0034 for Docket jobs/tasks;
- ADR-0045 for session task-list checkout/projection semantics;
- current Den runtime docs for runtime ownership.

### 7. Tests and checks

Add focused regression coverage for the canonical behavior:

- durable job/task CRUD still works;
- assigning and updating task status still works;
- checking out a Docket task list projection still works;
- syncing authorized projection changes back to Docket still works;
- local-only session task-list items are not silently treated as durable Docket tasks;
- plan-mode approval artifacts do not become Docket jobs/tasks by accident.

Also add one cheap repository-level check if practical, such as a test or script assertion that banned legacy symbols do not appear outside allowlisted migration/archive/docs locations.

## Acceptance criteria

The work is done when all of the following are true:

### Functional

- Durable work creation and updates use canonical Docket jobs/tasks.
- Session task-list checkout/sync behavior remains explicit and working.
- Plan-mode approval artifacts remain separate from Docket and task-list projections.
- Old compatibility paths are deleted or isolated behind clearly named compatibility/migration/archive boundaries.
- Existing data is preserved, migrated, or intentionally archived with a documented path.

### Terminology

- Runtime/tool descriptions no longer imply that legacy workplans/activity boards are the durable task system.
- UI/docs use Docket, task list, and plan-mode terms consistently.
- Any remaining legacy names are allowlisted and justified.

### Safety

- No migration silently drops user work.
- No local-only session task-list item is accidentally promoted to durable Docket state without explicit sync/promotion semantics.
- Deprecated external/client compatibility is removed only when tests or docs show it is safe.

### Tests/checks

- Existing Docket tests pass.
- New or updated tests cover checkout/sync and plan-mode separation.
- A search for obsolete terms has a small documented allowlist.
- Migrations/backfills, if any, have an idempotent verification path.

## Non-goals

Do not spend time on:

- replacing Docket;
- building a new planning system;
- redesigning the task-list UX;
- implementing the `work` sandbox;
- changing model/provider transport semantics;
- removing legitimate plan-mode approval artifact support;
- deleting historical docs just because they mention old names.

## Preferred tradeoffs

- Delete dead compatibility over wrapping it.
- Rename only when it removes real ambiguity.
- Keep migrations small and reversible/idempotent where possible.
- Prefer explicit checkout/sync boundaries over magical projection behavior.
- Prefer tests around behavior over broad snapshot churn.
- If unsure whether a path is still externally used, quarantine/deprecate first and document how to remove it later.

If an intentional shortcut remains, mark it with a `ponytail:` comment that names the ceiling and upgrade path.

Example:

```rust
// ponytail: this allowlist preserves historical workplan docs only; it is not a runtime compatibility layer.
// Upgrade path: remove the allowlist entry when the archived doc is superseded or deleted.
```

## Suggested final handoff

At the end, provide:

1. Inventory of legacy compatibility surfaces found.
2. What was deleted, renamed, migrated, or quarantined.
3. Any remaining allowlisted legacy references and why they remain.
4. Migration/backfill instructions, if applicable.
5. Tests/checks run.
6. Known risks for external/client compatibility.
7. Recommended follow-up cleanup.
