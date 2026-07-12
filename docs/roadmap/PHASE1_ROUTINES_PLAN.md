# Phase 1 Routines Plan

**Status:** Active Phase 1 product slice.

This plan splits routines out of [`PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md`](PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md). Routines are Den-native schedules or triggers assigned to one Bear/stance. They are not host cron, not a separate workflow engine, and not automatic skill/memory learning.

Related plans:

- [`DOCKET_IMPLEMENTATION_PLAN.md`](DOCKET_IMPLEMENTATION_PLAN.md) — durable jobs/tasks for work that needs execution tracking.
- [`ARTIFACT_REFS_IMPLEMENTATION_PLAN.md`](ARTIFACT_REFS_IMPLEMENTATION_PLAN.md) — opaque refs for routine outputs/evidence.
- [`PHASE1_GARAGE_ARTIFACTS_PLAN.md`](PHASE1_GARAGE_ARTIFACTS_PLAN.md) — product artifact browser/storage UX.
- [`MEMORY_CURATION_PLAN.md`](MEMORY_CURATION_PLAN.md) and [`SKILLS_IMPLEMENTATION_PLAN.md`](SKILLS_IMPLEMENTATION_PLAN.md) — review lanes for proposed memories/skills.

## Goal

Provide first-class Den routines that operators can create, disable, inspect, and audit. A safe routine should run through the native runtime, produce a traceable result, and store file outputs as artifacts.

## Scope

### 1. Routine definition

Minimum routine record:

- Bear and stance;
- schedule or trigger;
- enabled/disabled state;
- prompt/action payload;
- policy/risk classification;
- last run, next run, and audit fields.

### 2. Management UI

- List routines by Bear/status.
- Create/edit/disable routines.
- Show last result, next run, failures, and output refs.
- Make inherited Bear membership/policy visible.

### 3. Execution path

- Safe, non-external-effect routines may run through the native runtime.
- Coding or external-effect routines should route through Docket/`work` where sandbox and approval semantics apply.
- Routine outputs that are files should become Garage artifacts, preferably via artifact refs once available.

## Non-goals

- Do not create a new general workflow engine.
- Do not auto-install skills or write durable memory from unattended routine runs.
- Do not bypass Docket/sandbox/approval paths for risky or external-effect work.
- Do not require host cron as the product model.

## Implementation steps

1. Inventory any existing schedule/worker primitives and runtime invocation APIs.
2. Define minimal routine tables/API using existing Bear, stance, policy, and run identifiers.
3. Build CRUD/list/detail UI in the operator console or a linked routines section.
4. Implement safe execution for one constrained routine type.
5. Store run results and file outputs with provenance; use artifact refs where available, otherwise keep the storage seam explicit for later migration.
6. Add review hooks only as proposals: memory proposal, skill proposal, or Docket job/task proposal.
7. Add a minimal scheduler/execution smoke check.

## Acceptance criteria

- An operator can create, disable, and inspect a routine.
- A safe routine can run and produce an auditable result/output ref.
- Failures and skipped/disabled states are visible.
- Routines do not mutate memory, install skills, or perform high-risk external effects without explicit proposal/review/approval paths.
