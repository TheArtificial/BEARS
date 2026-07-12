# Phase 1 Operator Console Plan

**Status:** Active Phase 1 product slice.

This plan splits the operator-console work out of [`PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md`](PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md). It is intentionally product/UI focused: expose existing Den-native state before adding new backend concepts.

Related plans:

- [`DEN_RUNTIME_PLAN.md`](DEN_RUNTIME_PLAN.md) — native stance runtime.
- [`DOCKET_IMPLEMENTATION_PLAN.md`](DOCKET_IMPLEMENTATION_PLAN.md) — durable jobs/tasks.
- [`SKILLS_IMPLEMENTATION_PLAN.md`](SKILLS_IMPLEMENTATION_PLAN.md) — Skills catalog/manifest/proposal UI.
- [`PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md`](PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md) — task-list and Docket UX details.

## Goal

Give operators one coherent browser surface for inspecting and safely managing native Den operations.

The console should answer:

- which Bears, users, memberships, stances, and channels exist;
- whether Den, Bifrost, Postgres, optional Qdrant, optional Garage, and work sandbox pieces are healthy enough;
- what conversations, jobs, task lists, runs, and approvals need attention;
- which capabilities are configured for a Bear without exposing obsolete Letta/Codepool/MemFS concepts.

## Scope

### 1. Core administration

- Users: create/edit, operator flag, bootstrap/login state.
- Bears: create/edit/duplicate, slug/public id, model/config summary, active stances.
- Memberships: grant/revoke and verify effective access.
- Bear detail: stance registry health, prompt/config summary, memory/task/artifact/status links.

### 2. Runtime and system status

- Den and Bifrost reachability.
- Postgres and optional Qdrant/Garage configuration state.
- Work-surface/sandbox summary where already exposed.
- Recent runtime errors and blocked queues.

### 3. Work visibility

- Active conversations/sessions.
- Active jobs/tasks/task-list projections.
- Recent, failed, blocked, and running runs.
- Pending human approvals or unsafe actions.

Task-specific controls belong in [`PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md`](PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md); the console may summarize and link to them.

## Non-goals

- Do not rebuild Docket, task-list, Skills, MCP, or artifact storage models in the console.
- Do not add broad automation or a new workflow engine.
- Do not show Letta health, Letta Code YAML, Codepool, or MemFS panels except as historical migration/debug references behind explicit legacy labels.
- Do not add destructive actions without confirmation and clear backend semantics.

## Implementation steps

1. Inventory current admin routes/templates and server handlers.
2. Remove or quarantine obsolete Letta-era panels and terminology.
3. Add/clean overview sections:
   - Bears and stance health;
   - users and memberships;
   - runtime/system health;
   - active work and approvals;
   - configured capabilities summary.
4. Link out to focused slices for Skills, MCP catalog, artifacts, routines, and task-list/Docket details instead of duplicating their screens.
5. Add safe actions only where existing APIs have clear semantics: refresh, view details, copy ids/refs, grant/revoke membership, create/edit/duplicate Bear.
6. Add minimal smoke coverage for any non-trivial data shaping used by the console.

## Acceptance criteria

- An operator can create a Bear, add a user, grant membership, and verify chat access entirely in the browser.
- The console exposes active/blocked/failed work and links to the right detailed product surface.
- System status is understandable without `curl`.
- Copy uses current Den terminology: Bear, stance, channel, armature, memory, task list, Docket, artifact.
- No new backend concepts or dependencies are introduced solely for the console.
