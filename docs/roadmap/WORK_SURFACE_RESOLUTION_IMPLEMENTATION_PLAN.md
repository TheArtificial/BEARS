# Work-Surface Resolution Implementation Plan

> **Direction changed (2026-06).** Work-surface resolution uses per-Bear SQLite memory + Docket context, not MemFS orientation tools or Codepool examples. Canonical target: [Den runtime](../architecture/den-runtime.md) ([runtime plan](DEN_RUNTIME_PLAN.md)).

For the canonical stance model and current stance names, see [bear stances](../architecture/bear-stances.md).

## Status

In progress. The consumer boundary is implemented: Pair creates session-anchored Pair task trees, while Docket work jobs require one assigned managed work surface. Anchor recognition and confirmation remain to be implemented before Docket can safely default that required selection.

This plan follows the Pair Letta message-boundary and tool-discovery work.

## Problem

BEARS now distinguishes Bear memory, Workplaces, work surfaces, threads, and turns. `session_info` can expose work-surface hints, but hints are not the same as resolution. A Bear should know whether it is operating with no known work surface, one likely candidate, multiple candidates, a resolved work surface, or a user-confirmed work surface.

This resolution state should be visible to the Bear so it can communicate uncertainty, ask the user to verify, or ask the user to choose between candidates when scope affects memory, artifacts, plans, or actions.

## Goals

1. Represent work-surface resolution explicitly.
2. Keep resolution separate from persisted user-message content.
3. Surface resolution through `session_info`, orientation tools, and ACP session health/status surfaces.
4. Let the Bear communicate assumptions and ask for confirmation when needed.
5. Preserve user-confirmed resolution as provenance for later memory/plans/artifacts.
6. Avoid overconfident automatic classification.

## Non-goals

- Do not build a full Workplace/work-surface registry UI in the first slice.
- Do not require every turn to resolve a work surface.
- Do not make work-surface resolution an authorization boundary.
- Do not create a vector store or alternate semantic memory system.
- Do not infer aggressively from weak hints.

## Concepts

### Resolution state

| State | Meaning | Agent behavior |
|---|---|---|
| `unresolved` | No useful candidate is known. | Avoid broad-memory assumptions; ask or inspect when scope matters. |
| `candidate` | One likely candidate exists. | Proceed for low-risk work; state assumption when scope matters. |
| `ambiguous` | Multiple plausible candidates exist. | Ask user to choose or inspect to disambiguate. |
| `resolved` | Evidence identifies the work surface. | Use work-surface-first grounding. |
| `confirmed` | User explicitly confirmed the work surface. | Treat as authoritative for this thread unless contradicted. |
| `rejected` | Candidate was explicitly rejected. | Avoid that candidate unless new evidence appears. |

### Confidence

Use a simple confidence scale:

- `none`
- `low`
- `medium`
- `high`
- `confirmed`

### Evidence kinds

Initial evidence kinds:

- `explicit_channel_metadata`
- `user_reference`
- `workspace_root`
- `runtime_target`
- `conversation_selection`
- `memory_anchor`
- `git_remote` (later)
- `cabinet_mission` (later)
- `docket_project` (later)

## Target data shape

`session_info.work_surface` should evolve toward:

```json
{
  "status": "candidate",
  "confidence": "medium",
  "needs_user_confirmation": false,
  "active_candidate": {
    "slug": "bears-monorepo",
    "name": "BEARS monorepo",
    "kind": "repository",
    "confidence": "medium",
    "evidence": [
      { "kind": "workspace_root", "value": "/workspace" }
    ]
  },
  "candidates": [],
  "agent_guidance": {
    "may_state_assumption": true,
    "should_ask_user_when": [
      "multiple plausible work surfaces",
      "memory/action depends on scope",
      "user asks to continue prior work but current surface is unclear"
    ],
    "confirmation_examples": [
      "Should I treat this as the BEARS monorepo work surface?",
      "Are we working in Den or Codepool?"
    ]
  },
  "recommended_grounding_order": [
    "current conversation",
    "session_info",
    "current work-surface anchors",
    "role-local memory",
    "core memory",
    "workspace artifacts"
  ]
}
```

## UX integration with ACP lifecycle reset

The [ACP Lifecycle Reset Plan](./ACP_LIFECYCLE_RESET_PLAN.md) treats work-surface status as part of session health. Work-surface resolution should therefore be visible not only to the model through `session_info`, but also to the user/operator through `/status` or equivalent ACP status output.

User-facing status should be concise:

```text
Work surface: BEARS monorepo
Status: candidate
Confidence: medium
Grounding: workspace root /workspace
```

If unresolved or ambiguous, the status should say so without blocking unrelated low-risk work:

```text
Work surface: ambiguous. I may ask you to choose before writing memory or making scoped changes.
```

This UX must avoid overconfidence. A `candidate` should be described as an assumption, while `confirmed` can be described as user-confirmed.

## Phase 1: Enrich read-only orientation output

### 1.1 Add resolution fields to `infer_work_surface_hint`

Current output already includes candidates. Add:

- `status`
- `confidence`
- `needs_user_confirmation`
- `agent_guidance`
- `recommended_grounding_order`

Initial rules:

- no candidates → `status=unresolved`, `confidence=none`, `needs_user_confirmation=false`
- one candidate → `status=candidate`, `confidence=medium`, `needs_user_confirmation=false`
- multiple candidates → `status=ambiguous`, `confidence=low`, `needs_user_confirmation=true`

Do not mark anything `resolved` yet unless confirmed by memory anchors in a later phase.

### 1.2 Update `session_info` tests

Tests should verify:

- no hints returns unresolved/none,
- one hint returns candidate/medium,
- multiple hints returns ambiguous/low and `needs_user_confirmation=true`,
- guidance contains confirmation examples.

### 1.3 Keep descriptors unchanged unless needed

Existing descriptors already point to `session_info` and `memory_orient_work_surface`.

## Phase 2: Canonical anchor confirmation

### 2.1 Add anchor probe to `memory_orient_work_surface`

`memory_orient_work_surface` already has access to memory/MemFS. It should become the first tool that can move from candidate to resolved.

Given a candidate slug, check for canonical anchors:

```text
core/work_surfaces/<slug>/index.md
core/work_surfaces/<slug>/overview.md
core/work_surfaces/<slug>/architecture.md
core/work_surfaces/<slug>/decisions.md
pair/work_surfaces/<slug>/current-understanding.md
```

If anchors exist:

- `status=resolved`
- confidence `high`
- include canonical anchor paths
- include evidence kind `memory_anchor`

If no anchors exist:

- keep `candidate`
- suggest scaffold creation only when appropriate

### 2.2 Tests

- candidate slug with no anchors remains candidate.
- candidate slug with anchors becomes resolved.
- returned anchors distinguish `core/` and role-local paths.

## Phase 3: User confirmation state

### 3.1 Add session-level confirmation storage

Add a lightweight session/thread-level record for work-surface resolution.

Possible storage options:

- extend ACP/session runtime metadata,
- new `bear_session_work_surfaces` table,
- JSON field on a future shared pair session record.

Fields:

- `bear_id`
- `role`
- `channel_family`
- `session_id`
- `conversation_id`
- `work_surface_slug`
- `status` (`confirmed`, `rejected`)
- `confirmed_by_user_id`
- `source_text` / `confirmation_text`
- `created_at`, `updated_at`

### 3.2 Add a confirmation tool

Tentative provider name:

- `work_surface_choose`

Purpose:

- confirm one candidate,
- reject a candidate,
- set active thread work surface based on explicit user instruction.

Constraints:

- role/session scoped,
- not a memory write by itself,
- no broad Bear-global claim unless review later promotes it.

### 3.3 Update `session_info`

If confirmed state exists:

- `status=confirmed`
- `confidence=confirmed`
- include `confirmed_by` and timestamp
- include original evidence/confirmation text if safe.

## Phase 4: Stronger evidence sources

Add evidence sources gradually.

### 4.1 Repo metadata

For ACP, Den cannot inspect client files directly. Options:

- adapter reports git remote / repo root metadata in client context,
- agent uses local tools to inspect `.git/config` or package metadata,
- user explicitly identifies repo.

Do not make Den server assume local filesystem access.

### 4.2 Explicit user references

Detect explicit user references conservatively:

- “in the Den service”
- “for Codepool”
- “this BEARS repo”

These should create candidates or increase confidence, but not silently confirm unless the user clearly confirms.

### 4.3 External references

Later:

- Cabinet Mission ids,
- Docket project ids,
- deployment environment ids,
- service registry entries.

## Phase 5: Provenance propagation

When memory/plans/artifacts are created after resolution:

- include work-surface status/confidence,
- include confirmed work-surface slug when available,
- record whether confirmation came from user, memory anchor, workspace metadata, or inference.

This prevents role-local observations from becoming overbroad Bear-global facts.

### 5.1 Required surface selection for Docket work jobs

A Docket job is a **work root**, not a generic conversation container. It is
durable and dispatchable, so it must be bound to exactly one assigned managed
work surface at creation. Pair conversation planning belongs in a separate,
session-anchored **Pair task tree** and is never represented as a surface-less
Docket job.

The creation boundary is the enforcement point. `pair.create_job` must either
supply a valid managed surface explicitly or obtain one from the canonical
session-resolution result. It must otherwise fail before a job row or work run
is created with `work_surface_required`. Dispatch retains a defense-in-depth
preflight check for malformed legacy data, but it must not be the first normal
validation point.

Resolution rules:

1. An explicitly supplied `work_surface_ref` remains authoritative, subject to
   validation that it names a managed surface assigned to the Bear.
2. When no surface was supplied, consume the canonical session-resolution
   record—not raw runtime targets, workspace paths, or candidate slugs.
3. Auto-bind only when the record names one assigned managed surface and its
   status is `resolved` or `confirmed`. Persist both `work_surface_ref` and
   `work_surface_id` atomically with the job.
4. `unresolved`, `candidate`, `ambiguous`, and `rejected` states are not
   identities suitable for a write-capable work root. They must not bind a job.
5. For those states, or when the resolved/confirmed surface is unavailable or
   no longer assigned, fail creation with an actionable selection error. Do
   not create an unbound job and do not enqueue a work run.

The consumer must not add a second resolution algorithm inside Docket. A
job's stored binding remains stable if later inference changes.

Current implementation status:

- [x] Pair task trees use `session_anchor_id`; they do not create synthetic
  Docket conversation-objective jobs.
- [x] Pair job creation requires an assigned managed surface and rejects raw
  or unassigned roots.
- [x] The consumer accepts a typed adapter-environment session anchor only
  when its status is `resolved` or `confirmed`, then verifies Bear assignment.
- [x] Docket work-job persistence requires non-empty `work_surface_ref` and
  `work_surface_id`; the legacy synthetic conversation-objective path is
  removed.
- [ ] Produce the canonical session-resolution record through Phases 1–3.
- [ ] Add end-to-end coverage that a producer-created resolved/confirmed
  record defaults a Docket work job.

Tests:

- explicit assigned surface creates a work job with both surface fields;
- a unique resolved assigned surface defaults both fields;
- a confirmed assigned surface defaults both fields;
- an explicit surface overrides a session default;
- candidate, unresolved, ambiguous, and rejected states fail creation without
  persisting a job;
- unavailable or non-unique resolved identities fail creation without
  persisting a job;
- Pair task-tree create/list/checkout works with `session_anchor_id` and never
  appears as a Docket job;
- dispatch rejects malformed legacy work jobs before a run is queued.

## UX expectations

The Bear may say:

- “I’m treating this as the BEARS monorepo based on the workspace root.”
- “This could be Den or Codepool. Which work surface should I use?”
- “I don’t yet know which work surface this thread is about. Should I use the current workspace?”
- “Got it — I’ll treat this thread as Codepool.”

The Bear should not ask on every turn. It should ask when ambiguity materially affects retrieval, memory, planning, or action.

## Implementation checklist

### Immediate

- [ ] Add resolution fields to `infer_work_surface_hint`.
- [ ] Update `session_info` tests for unresolved/candidate/ambiguous behavior.
- [ ] Include recommended grounding order and agent guidance in `session_info.work_surface`.

### Next

- [ ] Enhance `memory_orient_work_surface` with canonical anchor confirmation.
- [ ] Add tests for anchor-based resolution.
- [ ] Design persistence for user-confirmed work-surface state.
- [ ] Add confirmation tool only after read-only orientation proves useful.

### Docket work-root integration

- [x] Separate Pair task trees (`session_anchor_id`) from Docket work jobs;
  remove synthetic conversation-objective jobs.
- [x] Require an assigned managed surface for Docket work-job creation.
- [x] Reject absent, raw, unknown, and unassigned selections at the creation
  boundary; do not create an unbound work job.
- [x] Consume a typed `resolved`/`confirmed` adapter-environment anchor when
  available, with assigned-surface verification.
- [x] Persist both `work_surface_ref` and `work_surface_id` for work jobs.
- [ ] Define and produce the canonical session-resolution record consumed by
  job creation (Phases 1–3).
- [ ] Replace the adapter-environment bridge with that canonical record once
  it exists, without changing the Docket decision rules.
- [ ] Add producer-to-job end-to-end regression coverage and dispatch
  malformed-data preflight coverage.

## Related docs

- `docs/concepts/../architecture/memory-model.md`
- `docs/architecture/adr/bear-workplaces.md`
- `docs/architecture/adr/pair-tool-discovery-and-scope-orientation.md`
- `docs/planning/PAIR_TOOL_DISCOVERY_AND_SCOPE_POLICY.md`
- `docs/planning/PAIR_LETTA_MESSAGE_BOUNDARY_PLAN.md`
