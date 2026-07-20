# Phase 1 Task-list and Workflow UX Plan

**Status:** Active Phase 1 product slice.

This plan splits task-list/workflow UX out of [`PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md`](PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md). It does not replace [`DOCKET_IMPLEMENTATION_PLAN.md`](DOCKET_IMPLEMENTATION_PLAN.md), which is the canonical backend/jobs/tasks plan.

Related plans:

- [`DOCKET_IMPLEMENTATION_PLAN.md`](DOCKET_IMPLEMENTATION_PLAN.md) — Docket jobs/tasks and task-list projection mechanics.
- [`PHASE1_OPERATOR_CONSOLE_PLAN.md`](PHASE1_OPERATOR_CONSOLE_PLAN.md) — high-level operator overview.
- [`SANDBOX_IMPROVEMENTS_ROADMAP.md`](SANDBOX_IMPROVEMENTS_ROADMAP.md) — work sandbox follow-ups.
- [`ARTIFACT_REFS_IMPLEMENTATION_PLAN.md`](ARTIFACT_REFS_IMPLEMENTATION_PLAN.md) — evidence/result refs.

## Goal

Make tactical session task lists, durable Docket work, workplan artifacts, run status, and approval queues legible without merging their meanings.

## Concepts to preserve

- **Task list:** conversation working projection visible to the user/model; normally the active top-level task/subtree of the conversation-linked Docket objective.
- **Conversation-linked objective:** one mutable Docket-owned structured work tree per conversation, created only once task orientation is invoked.
- **Docket job/task:** durable work objective/state in Den Postgres; durable Jobs may later be promoted from conversation objective subtrees.
- **Workplan artifact:** workflow-state artifact for formal planning/review, not semantic memory.
- **Run/work run:** execution attempt with tool calls, sandbox state, evidence/results, and failures.
- **Approval:** explicit human/operator decision before high-risk or external-effect execution.

## Scope

### 1. Shared turn/work status payload

Expose one canonical status payload for UI clients that includes:

- current mode/tool-policy state where relevant;
- active task list/projection;
- linked Docket job/task ids;
- workplan artifact state;
- approval/execution unlock state;
- current or recent run status.

### 2. UI surfaces

- Session task-list view: active, done, blocked, cancelled items.
- Docket job/task detail: hierarchy, criteria, status, result refs.
- Pending handoffs/reviews.
- Work-run status and failures.
- Approval queue for high-risk work.

### 3. Terminology and ownership

- Use task-list wording for visible checklists.
- Reserve Docket for durable jobs/tasks.
- Reserve workplan for formal workflow-state artifacts.
- Do not reconstruct status independently in each client when a shared payload exists.

## Non-goals

- Do not redesign Docket schema here.
- Do not reintroduce the legacy activity-board/task-system model.
- Do not make Pair Ask/Plan/Write modes a separate durable mutation gate unless a current plan explicitly requires it.
- Do not store workflow state as semantic memory.

## Implementation steps

1. Inventory existing Docket/task-list/workplan/run APIs and UI consumers.
2. Define or choose the canonical status payload, deleting duplicate reconstruction where possible.
3. Update operator and BearWire/ACP-facing surfaces to consume that payload.
4. Add views for active task list, pending handoffs, Docket sync/review state, and high-risk approvals.
5. Link evidence/result refs through artifact refs where available.
6. Add a small check around status projection/data shaping.

## Acceptance criteria

- Pair can resume an ACP/BearWire session and recover the conversation-linked objective/task-list projection/workplan state.
- Operators can see active conversation objectives/task-list projections, pending handoffs, durable Docket-backed work, run failures, and approvals without confusing their ownership.
- High-risk work runs surface in an approval queue before execution.
- UI copy consistently distinguishes task list, Docket, workplan artifact, run, and approval.
