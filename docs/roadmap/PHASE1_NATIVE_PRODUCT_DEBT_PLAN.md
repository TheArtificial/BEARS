# Phase 1 native product debt plan

**Status:** Active consolidation/index plan.

This document is now the Phase 1 native product index. The broad workstreams are split into focused plan documents so implementation can proceed without treating "Phase 1 native" as one oversized bucket:

- Operator console: [`PHASE1_OPERATOR_CONSOLE_PLAN.md`](PHASE1_OPERATOR_CONSOLE_PLAN.md)
- Web chat and onboarding: [`PHASE1_WEB_CHAT_ONBOARDING_PLAN.md`](PHASE1_WEB_CHAT_ONBOARDING_PLAN.md)
- Skills: [`SKILLS_IMPLEMENTATION_PLAN.md`](SKILLS_IMPLEMENTATION_PLAN.md)
- MCP catalog and attachments: [`PHASE1_MCP_CATALOG_ATTACHMENTS_PLAN.md`](PHASE1_MCP_CATALOG_ATTACHMENTS_PLAN.md)
- Routines: [`PHASE1_ROUTINES_PLAN.md`](PHASE1_ROUTINES_PLAN.md)
- Garage artifacts product surface: [`PHASE1_GARAGE_ARTIFACTS_PLAN.md`](PHASE1_GARAGE_ARTIFACTS_PLAN.md)
- Task-list and workflow UX: [`PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md`](PHASE1_TASK_LIST_WORKFLOW_UX_PLAN.md)

Keep this file as the native-aligned consolidation of historical Phase 1 decisions. Use the focused documents for delegation and implementation details.

This document salvages the still-current product intent from these superseded roadmap documents:

- [`PHASE1_BOOTSTRAP.md`](PHASE1_BOOTSTRAP.md)
- [`PHASE1_DECISIONS.md`](PHASE1_DECISIONS.md)
- [`MULTI_ROLE_RUNTIME_IMPLEMENTATION_PLAN.md`](MULTI_ROLE_RUNTIME_IMPLEMENTATION_PLAN.md)
- [`TASK_SYSTEM_IMPLEMENTATION_PLAN.md`](TASK_SYSTEM_IMPLEMENTATION_PLAN.md)

Those source documents predate the Den-native runtime and still mention Letta, Letta Code, Codepool, MemFS, and per-agent provisioning. Use them as historical rationale only. Current implementation must target the Den-native loop, BearWire armatures, Docket tasks, per-Bear SQLite memory, and descriptor-owned tool routing.

## Goals

1. Finish the Phase 1 product surface on the native runtime.
2. Preserve useful product promises from the historical plans without carrying forward obsolete runtime paths.
3. Make operator-facing flows coherent: Bears, users, memberships, stances, skills, MCP, routines, artifacts, task lists, and memory should be visible without requiring `curl` or historical Letta concepts.

## Salvaged Decisions

| Area | Native-aligned decision |
|---|---|
| Operator console | Keep server-rendered MiniJinja/forms as the baseline. Mobile-first, refresh/redirect is acceptable, progressive JS only. |
| Chat UI | Den-hosted Deep Chat remains the first-party web chat. No Open WebUI support burden. |
| Streaming | SSE remains the browser chat contract for `/v1/chat/send`. |
| Public IDs | Public APIs use `bear_id` plus `slug` where helpful. Do not expose legacy runtime ids as user-facing identifiers. |
| Bear customization | Operators can create, edit, and duplicate Bears. Runtime bindings are native stances, not Letta agents. |
| Membership | Users and Bears remain many-to-many. Membership is enforced on every chat, BearWire, and admin-relevant action. |
| Personal Bear onboarding | New-user onboarding should be able to create or assign a Personal Bear and redirect into chat with a safe onboarding prompt. |
| Memory promise | Users should see small curated always-on memory plus longer material findable through Den memory/recall. No Letta memory block UI. |
| Routines | Routines are first-class Den schedules assigned to one Bear/stance; they inherit Bear membership and policy. |
| Artifacts | Agent outputs, human uploads, routine files, and large skill trees live in Garage/artifact storage with metadata and retention/GC. Cabinet attachments remain separate. |
| Skills | Skills are Den-managed catalog/manifest/proposal artifacts. Use [`SKILLS_IMPLEMENTATION_PLAN.md`](SKILLS_IMPLEMENTATION_PLAN.md). |
| Task lists | Session task lists are tactical working projections. Docket jobs/tasks are durable work state. Workplan artifacts are a separate workflow-state domain, not semantic memory. |
| Human approval | High-risk external effects and task runs need operator/HITL queues with auditable approve/reject decisions. |

## What Not To Salvage

- Letta provisioning, `letta_agent_id`, Letta Code YAML generation, Letta health panels, Codepool paths, or MemFS worktrees as active product flows.
- Skills installed through Letta Code directories or Letta APIs.
- Task intent files under MemFS as the canonical task pipeline.
- Conversation ownership by Letta or harness-specific conversation mapping.
- Memory dashboards that read Letta `human` / `person:*` blocks.

## Workstreams

### 1. Operator Console Baseline

Deliver one coherent console path for native operations:

- users: create/edit, operator flag, login/bootstrap state;
- Bears: create/edit/duplicate, stance registry health, model choice, system prompt/config summary;
- memberships: grant/revoke and verify effective access;
- native runtime status: Den/Bifrost/Postgres/Qdrant where configured;
- no Letta Code YAML, Letta health, Codepool, or MemFS panels.

Exit criteria:

- An operator can create a Bear, add a user, grant membership, and verify the user can chat with that Bear entirely in the browser.
- UI copy uses Bear, stance, channel, armature, memory, task list, and Docket terminology correctly.

### 2. Web Chat And Onboarding

Keep web chat as the Phase 1 user proof point:

- `/bear/{slug}` remains the reference browser client for `/v1/chat/send` SSE;
- new users can be assigned or provisioned a Personal Bear from a native Bear template/config;
- onboarding prompt invites safe personalization and does not imply Letta memory blocks;
- membership failure is explicit and actionable.

Exit criteria:

- A new user can sign in, get a Personal Bear, land in chat, and see a streamed response.
- Non-members get a 403 or equivalent user-visible denial.

### 3. Native Skills Product Slice

Canonical plan: [`SKILLS_IMPLEMENTATION_PLAN.md`](SKILLS_IMPLEMENTATION_PLAN.md).

Phase 1 product scope:

- catalog list/detail;
- Bear skill manifest tab;
- attach/detach approved skills with role applicability;
- proposal queue with approve/reject controls;
- descriptor discovery for `skill_list`, `skill_read`, and `skill_propose`;
- package export/import hooks for approved skill artifacts.

Exit criteria:

- An operator can attach an approved skill to a Bear for one or more stances.
- A role can discover its approved skills through descriptor-owned tools.
- A runtime role can propose a skill but cannot install it directly.

### 4. MCP Catalog And Attachments

Salvaged intent: Den owns an MCP catalog and per-Bear/per-stance attachments. Native alignment changes the projection path.

Tasks:

- define MCP catalog records and attachment records;
- represent source kind, transport, required secrets, allowed stances, risk class, and capability descriptors;
- show attachment status and discovery diagnostics in operator UI;
- integrate with descriptor routing so MCP tools are exposed only on surfaces that can actually execute them;
- keep channel adapters separate from armature-local MCP assumptions.

Exit criteria:

- Operators can attach an MCP server/config to a Bear/stance.
- `session_info` or equivalent status shows discovered MCP capability state.
- Web channel turns do not gain armature-local MCP tools by accident.

### 5. Routines

Salvaged intent: routines are first-class Den schedules assigned to one Bear.

Native-aligned tasks:

- define routine records: Bear, stance, schedule, enabled state, prompt/action, policy, last run, next run, and audit;
- implement CRUD/list UI;
- execute through the native runtime for non-sandboxed routines where safe;
- route coding/external-effect routines through Docket/`work` only after the work sandbox exists;
- store routine outputs as Garage artifacts and optionally create memory/skill/task proposals through review lanes.

Exit criteria:

- An operator can create, disable, and inspect a routine.
- A safe routine can run and produce an auditable result/artifact.
- Routines do not auto-install skills or write durable memory without explicit proposal/review flow.

### 6. Garage Artifacts

Salvaged intent: artifacts are not memory and not Cabinet.

Tasks:

- configure artifact storage and bucket policy;
- add metadata for Bear, user, conversation/run/routine, provenance, content type, size, created_at, and retention state;
- support presigned upload/download where appropriate;
- define retention/GC policy;
- integrate skill artifacts, user uploads, routine outputs, and future work-run logs.

Exit criteria:

- Operator/user can upload or retrieve an artifact through Den policy.
- Routine or runtime output can create an artifact with provenance.
- GC/retention behavior is documented and visible enough for operators.

### 7. Task Lists, Docket, And Workflow State

Canonical plan: [`DOCKET_IMPLEMENTATION_PLAN.md`](DOCKET_IMPLEMENTATION_PLAN.md).

Salvaged from the task-system plan:

- Pair Ask/Plan/Write modes are UI/tool-policy state, not a separate durable mutation gate.
- Session task lists are tactical working projections.
- Workplan artifacts are workflow-state records, not semantic memory.
- Docket jobs/tasks are durable execution state.
- Operator UI needs active task lists, pending handoffs, durable Docket work, runs, failures, and high-risk approval queues.

Phase 1 product tasks:

- expose one canonical turn-state/status payload for mode, tool classes, active task list, workplan state, approval state, and execution unlock state;
- make operator and BearWire/ACP surfaces consume that payload rather than reconstructing state from legacy fields;
- use task-list wording for visible checklists and reserve workplan for formal review artifacts;
- show pending task-list handoffs and Docket sync/review state.

Exit criteria:

- Pair can resume an ACP/BearWire session and recover active task-list/workplan state.
- Operators can see current task-list state, pending handoffs, and Docket-backed work without confusing their ownership.
- High-risk work runs surface in an approval queue before execution.

### 8. Product Documentation And Deployment Polish

Tasks:

- update deploy/operator docs for native Phase 1 setup;
- remove Letta Code, Codepool, and MemFS setup from happy-path docs;
- document required env for Den, Bifrost, Postgres, optional Qdrant, optional Garage;
- document smoke tests for web chat, BearWire pair, skills, MCP, routines, and artifacts once each slice lands.

Exit criteria:

- A new operator can stand up the native stack and complete the Phase 1 browser happy path without reading historical Letta docs.

## Suggested Implementation Order

1. Operator console cleanup and terminology pass.
2. Web chat/onboarding browser happy path.
3. Skills Phase 1 product slice.
4. Garage artifact foundation.
5. MCP catalog and attachment status.
6. Routines CRUD and safe native execution path.
7. Task-list/workflow-state operator UX.
8. Documentation and smoke test polish.

## Source Document Disposition

- `PHASE1_BOOTSTRAP.md` — archive after this plan and deploy docs cover operator/product scope.
- `PHASE1_DECISIONS.md` — archive after decisions above are accepted as the native replacement.
- `MULTI_ROLE_RUNTIME_IMPLEMENTATION_PLAN.md` — archive after remaining stance/UI/skills concepts are covered by this plan, [`SKILLS_IMPLEMENTATION_PLAN.md`](SKILLS_IMPLEMENTATION_PLAN.md), and [`DEN_RUNTIME_PLAN.md`](DEN_RUNTIME_PLAN.md).
- `TASK_SYSTEM_IMPLEMENTATION_PLAN.md` — archive after [`DOCKET_IMPLEMENTATION_PLAN.md`](DOCKET_IMPLEMENTATION_PLAN.md) absorbs any remaining workflow-state/turn-state details, or keep as reference until Docket phases 3-5 land.
