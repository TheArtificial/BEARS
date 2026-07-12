# Stance-scoped delegated runs implementation plan

**Status:** Proposed — delay until artifact refs are in place  
**Primary ADR:** [ADR-0053: Stance-Scoped Delegated Runs](../decisions/adr-0053-stance-scoped-delegated-runs.md)  
**Prerequisite:** [Artifact refs implementation plan](ARTIFACT_REFS_IMPLEMENTATION_PLAN.md) / [ADR-0004](../decisions/adr-0004-artifacts-garage.md)  
**Related ADRs:** [ADR-0014](../decisions/adr-0014-multi-role-runtime-architecture.md), [ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md), [ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md), [ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md), [ADR-0045](../decisions/adr-0045-session-task-lists-and-docket-checkout.md), [ADR-0050](../decisions/adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md)

## Goal

Allow a Bear in any stance to request background or delegated work while keeping stance authority boundaries hard. Delegated runs should use the narrowest capable stance (`work`, `curate`, `watch`, or `pair`), communicate with their parent through structured Den records/events/artifacts, and never inherit the initiating stance's tools, credentials, or side-effect authority.

Delegation should not require a Docket Job. Simple background work may be anchored to the current conversation turn or parent run. Jobs are for durable, user-trackable work with task state, acceptance criteria, resumability, commit/work-surface policy, or cross-turn lifecycle management. Conversation-scoped delegated runs may later be promoted into Jobs while preserving run and artifact provenance.

## Delegation levels and anchors

Use the lightest rung that preserves traceability:

1. **Inline tool call:** synchronous work inside the current turn; no delegated run and no Job.
2. **Background delegated run:** asynchronous, bounded work anchored to a conversation turn, parent run, artifact, Cabinet item, Job, or task; no Job required.
3. **Job-backed delegation:** durable managed work anchored to a Docket Job/task when the user needs explicit progress tracking, acceptance criteria, handoff, resumability, commit policy, or completion evidence across turns.

Every delegated run must have an anchor. A Job/task is one valid anchor, not the required anchor. Unanchored background processes are not allowed.

Suggested anchor shape:

```text
delegated_run
- run_id
- parent_run_id nullable
- anchor_kind    -- conversation_turn | job | task | artifact | cabinet_item
- anchor_id
- initiating_stance
- resolved_stance
- objective
- status
- result_summary nullable
```

Artifact refs are valid outputs for any delegated run anchor. If a conversation-scoped delegated run grows into durable work, promotion should create a Job/task linked back to the originating run/turn and re-link or additionally link existing artifact refs as initial evidence/output.

## Non-goals

- No general free-form agent-to-agent chat bus.
- No direct tool or credential inheritance from parent to child.
- No automatic mixed-domain decomposition in the first pass.
- No requirement that all background work run as `work`.
- No durable memory promotion, external sends, commits, deploys, or destructive actions without existing Den approval policy.
- No requirement that every delegated run create or attach to a Docket Job.

## ADR fulfillment

This plan fulfills ADR-0053 by implementing:

- [ ] Stance separate from execution mode.
- [ ] A single model-facing `delegate_run` affordance.
- [ ] Delegation broker between model and child loop.
- [ ] Deterministic stance resolver with `desired_stance` treated as advisory.
- [ ] Authorization from first principles: resolved stance, target, autonomy, user consent, trust profile, and job policy when a Job is involved.
- [ ] Scoped capability minting for child runs.
- [ ] Durable parent/child run records with explicit anchors; Jobs are optional anchors.
- [ ] Structured parent/child events, decision requests, commands, and completion results.
- [ ] Artifact-ref outputs using ADR-0004; child loops do not mint artifact refs directly.
- [ ] Audit log of request, resolution, authorization, capabilities, and result.

## Implementation phases

### Phase 0 — Confirm prerequisites

**Goal:** Do not build subagent plumbing before the handles it needs exist.

- [ ] Artifact refs exist for run outputs and evidence.
- [ ] Runtime/delegated-run records can attach artifact refs.
- [ ] Docket run/task records can attach artifact refs when delegation is job-backed.
- [ ] Runtime event model can carry typed child-run events or can be minimally extended.
- [ ] Current stance/tool envelopes are explicit enough to mint a restricted child capability bundle.

**Exit gate:** A delegated run can produce a report/test log/patch/memory review as an artifact ref without inventing an output identity scheme.

### Phase 1 — Delegation request and broker skeleton

**Goal:** Add the one model-facing request path without launching arbitrary agents directly.

- [ ] Add conceptual `delegate_run` service/tool shape with:
  - objective
  - context refs
  - desired stance
  - target kind/ref
  - autonomy
  - urgency
  - wait policy
  - success criteria
- [ ] Add broker normalization and validation.
- [ ] Add deterministic stance resolver:
  - repo/workspace/code/test/command -> `work`
  - memory/profile/archive/recall -> `curate`
  - wait/poll/monitor/scheduled check -> `watch`
  - reasoning/planning-only -> `pair` or clarification
- [ ] Reject or ask clarification for mixed-domain tasks in the first version.
- [ ] Add `ponytail:` comments documenting rule-table resolver ceiling and upgrade path.

**Exit gate:** The platform can accept a delegation request, resolve or reject the stance, and produce an auditable dry-run decision without starting a child loop.

### Phase 2 — Authorization and capability minting

**Goal:** Hold the permission constraint firm.

- [ ] Add initiator-to-delegation permission matrix.
- [ ] Check target surface access independently of initiating stance.
- [ ] Check requested autonomy against stance policy, target policy, trust profile, and job policy when a Job/task anchor exists.
- [ ] Require user approval for destructive, externally visible, durable-memory, push/deploy, or scope-expanding actions.
- [ ] Mint scoped capability bundles for child runs.
- [ ] Ensure child tools are derived only from minted capabilities, not parent tool availability.
- [ ] Add audit events for requested stance, resolved stance, denied capabilities, granted capabilities, and approval reason.

**Exit gate:** A child run spec can be created with explicit allowed/denied capabilities, and tests prove parent capabilities are not inherited.

### Phase 3 — Durable parent/child run records

**Goal:** Make delegation resumable, inspectable, cancellable, and auditable.

- [ ] Extend run records with parent run refs plus an explicit anchor kind/ref (`conversation_turn`, `job`, `task`, `artifact`, or `cabinet_item`), initiating stance, resolved stance, target, objective, wait policy, and capability bundle ref.
- [ ] Add child-run statuses: queued, running, blocked, needs approval, completed, failed, cancelled.
- [ ] Add parent-visible child handle returned from `delegate_run`.
- [ ] Add cancellation and status lookup APIs.
- [ ] Connect child runs to Docket tasks/jobs when applicable, without forcing simple conversation-scoped backgrounding through Docket.
- [ ] Add promotion path from a conversation-scoped delegated run to a Docket Job/task while preserving originating run/turn and artifact links.

**Exit gate:** Parent loops and UI can list anchored child runs and inspect their current status without direct access to child prompt/tool state; simple backgrounding works without creating a Job.

### Phase 4 — Parent/child communication protocol

**Goal:** Replace informal agent chat with structured events and artifacts.

- [ ] Add child-to-parent event types:
  - started
  - progress
  - artifact created
  - blocked
  - requires decision
  - completed
  - failed
  - cancelled
- [ ] Add parent-to-child command types:
  - cancel
  - pause/resume if supported
  - add context
  - narrow scope
  - revise objective
  - answer decision
- [ ] Re-authorize any command that expands scope, target, autonomy, or side effects.
- [ ] Add visibility levels: internal audit, parent only, user visible, artifact only.
- [ ] Add normalized completion result with summary, artifact refs, changes made, decisions needed, and followups.

**Exit gate:** A parent can start, observe, cancel, answer, and receive completion from a child run using Den-owned records/events only.

### Phase 5 — Stance-specific loop launch

**Goal:** Start with the smallest real delegated run type.

Recommended order:

1. [ ] `work` read-only or propose-only delegated run against a work surface.
2. [ ] `curate` propose-only memory/archive review.
3. [ ] `watch` read-only monitoring run.
4. [ ] `pair` background reasoning only if a real product need remains.

For each stance:

- [ ] Define allowed tool envelope.
- [ ] Define default autonomy.
- [ ] Define output artifact kinds.
- [ ] Define blocking/escalation behavior.
- [ ] Add one runnable check that the stance cannot use a denied capability.

**Exit gate:** At least one stance can execute a child loop end-to-end and report through the protocol.

## Human UI affordances

Build these around real run records, not screenshots of logs.

- [ ] Inline run card in conversation/task surfaces showing goal, anchor, stance, status, permissions summary, and actions.
- [ ] Human-readable permission summary:
  - can read/edit/run/etc.
  - cannot push/deploy/write memory/send externally/etc.
- [ ] Approval prompts for blocked permission requests with clear risk and options.
- [ ] Summarized progress stream with expandable raw events/tool trace.
- [ ] Completion receipt with summary, artifacts, changed files/surfaces, side effects, and followups.
- [ ] Cancel action in the first version; pause/resume and scope narrowing can follow.
- [ ] Run detail page only after inline cards are insufficient.
- [ ] Parent/child tree view only after nested delegation becomes common.
- [ ] "Promote to Job" affordance only after conversation-scoped background runs demonstrably grow into durable work.

## Model experience affordances

Models should request objectives, not scheduler plumbing.

- [ ] One model-facing `delegate_run` tool/affordance.
- [ ] Broker returns a compact child-run object:
  - `run_ref`
  - resolved stance
  - status
  - latest summary
  - allowed parent actions
- [ ] Runtime pushes blocked/completed child events back to the parent loop where possible.
- [ ] Add minimal status tools only as needed:
  - `get_run_status`
  - `list_run_events`
  - `send_run_command`
- [ ] Decision requests are structured and say whether user approval is required.
- [ ] Parent models can narrow/cancel/add context without knowing child internals.
- [ ] Child results are normalized enough for parent models to summarize directly.

## Acceptance criteria

Delegated runs are ready when:

- [ ] Background execution mode is not conflated with `work` stance.
- [ ] The broker, not the model, selects or validates the child stance.
- [ ] A child run cannot use parent-only tools, credentials, or authority.
- [ ] Scope expansion always re-enters authorization.
- [ ] Parent/child communication uses structured events, commands, decisions, and artifact refs.
- [ ] Human users can see what a child is doing, what it can do, and stop or approve it.
- [ ] Models can delegate common work with one simple affordance and receive structured results.
- [ ] Audit logs explain who requested delegation, which anchor was used, what stance was resolved, what was granted/denied, and what artifacts/results were produced.
- [ ] Simple background delegation can run conversation-scoped without creating a Docket Job, while durable work can still use or later promote into Jobs.
