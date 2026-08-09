# Task Delegation Lifecycle Plan

**Status:** Deferred — planning only; do not expose `delegate_task` until an end-to-end execution path exists.

**Related plans:** [Stance-scoped delegated runs implementation plan](STANCE_SCOPED_DELEGATED_RUNS_IMPLEMENTATION_PLAN.md), [Agent loop control implementation plan](AGENT_LOOP_CONTROL_IMPLEMENTATION_PLAN.md), [Docket implementation plan](DOCKET_IMPLEMENTATION_PLAN.md), [Work run finalization resilience roadmap](WORK_RUN_FINALIZATION_RESILIENCE_ROADMAP.md)

**Related ADRs:** ADR-0034 (Jobs and tasks), ADR-0045 (session task lists and Docket checkout), ADR-0050 (agent loop control), and ADR-0053 (stance-scoped delegated runs).

## Decision and scope

Task delegation is deferred. There must be no read-only placeholder, intent-only tool, or record that represents a delegated task without actually executing it. Such a surface would imply parallel work and lifecycle guarantees that Den does not yet provide.

When implemented, `delegate_task` means that a parent execution context explicitly asks a child execution to complete one bounded task and later receives a structured result. It is a real execution facility, not a synonym for task creation, task selection, planning, or `dispatch_work`.

This plan narrows the broader stance-scoped delegated-runs roadmap to the task-oriented product contract needed by Pair and Work. It does not replace that roadmap's stance resolution, capability minting, artifacts, or audit requirements. `delegate_task` is a task-oriented specialization of the broader delegated-run infrastructure: the broader plan may establish non-task-anchored delegation independently, but it must not expose or simulate the task-facing operation before this plan's gates are met.

## Invariants

1. **One user concept.** The model-facing operation is `delegate_task` in both Pair and Work. Avoid product-facing `subagent`, `spawn`, and stance-specific delegation names.
2. **Existing context only.** Delegation uses the parent’s existing execution context. It never provisions a sandbox, creates a Docket Job, or silently dispatches background Work.
3. **Explicit child selection.** The parent names a concrete child task. The platform does not choose the next runnable task as an implicit delegate.
4. **Parent remains accountable.** A delegate may report completion, failure, evidence, and a proposed task outcome. Only the parent (or an existing authorized settlement path) accepts work and settles the parent-owned task tree.
5. **No authority inheritance.** Child capabilities are minted from policy; they are not copied from the parent’s tool list, credentials, approvals, or ambient authority.
6. **Bounded scope.** A child may act only on its task, approved inputs, assigned execution surface, and granted capabilities. Scope expansion requires a new parent request and re-authorization.
7. **Same workspace-safety policy.** Pair and Work differ in their execution surface, not in concurrent mutation rules. Do not make a Work-only exception because its workspace is isolated from the user checkout.
8. **Durable observability.** A real delegation has a stable lifecycle, cancellation, normalized result, and parent-visible evidence. Raw child conversation is not the contract.

## Relationship to Pair, Work, and Docket

```text
Pair session current task
  └── delegate_task(explicit child task) ──► child execution in Pair's existing context

Docket Job
  └── dispatch_work(Job) ──► one isolated Work sandbox/run for the Job
       └── delegate_task(explicit child task) ──► child execution in that existing sandbox
```

### Pair

Pair ordinarily works its selected session current task directly. A Pair delegation is permitted only when the selected task has an explicit bounded child task (or other policy-approved task relationship). It does not turn the child into a Job and does not change Pair’s selected current task.

### Work

`dispatch_work` is deliberately **Job-scoped**: a sandbox is costly and autonomous, so one Work run is created for the dispatched Job rather than once per task. The Work supervisor can delegate explicit tasks within the Job’s approved task tree while retaining the Job-scoped sandbox. A delegated child must not select tasks outside that tree or dispatch another sandbox.

### Docket

Docket tasks provide task identity, hierarchy, criteria, and durable task state. A task is not automatically delegated because it exists, is pending, or is the current projected item. Job dispatch and task delegation are separate operations with separate lifecycles.

## Required lifecycle

The minimum durable lifecycle is:

```text
requested
  → authorized | rejected
  → queued
  → running
  → reported
  → accepted | needs_decision | blocked | failed | cancelled
```

- **requested:** parent supplies explicit child task, bounded objective, required inputs, and requested capability/surface policy.
- **authorized/rejected:** broker validates parent authority, task relation, target surface, policy, and concurrency reservation before any child starts.
- **queued/running:** child execution has a stable delegation/run identity and only the minted capability envelope.
- **reported:** child submits normalized summary, evidence/artifact references, changes, blockers, and proposed disposition.
- **accepted:** parent validates the report and uses the appropriate existing Docket/task settlement mechanism. Child reporting alone must not settle a task.
- **needs_decision:** child cannot proceed without a scoped parent or human answer. An answer that expands authority is re-authorized.
- **blocked/failed/cancelled:** terminal child outcomes are visible to the parent and release reservations. They do not silently choose replacement work.

A delegation record needs, at minimum:

```text
id
parent execution/run reference
parent task reference
child task reference
anchor (session or Job/Work run)
execution context/surface reference
objective and bounded input/context references
requested and granted capability bundle references
workspace reservation reference, if mutation is granted
status and timestamps
result summary, evidence/artifact references, and proposed disposition
cancellation/decision references
```

Use existing run, event, and artifact mechanisms where they meet these requirements. Do not add a parallel generic task-history table merely to duplicate Docket or runtime records.

## Preconditions and implementation phases

### Phase 0 — Reconcile existing plans and execution records

- Identify the canonical runtime-run record and the extension point for parent/child linkage.
- Reuse the artifact-reference design for result/evidence identity.
- Specify how a Pair attached context and a Work sandbox are represented as one typed execution-surface input.
- Update the state-machine inventory and loop-control plan with lifecycle ownership and terminal transitions.

**Exit gate:** There is one proposed record/lifecycle design compatible with the broader stance-scoped delegated-runs plan and no second dispatch model.

### Phase 1 — Scope validation and authorization broker

- Require an explicit `child_task_id`; reject implicit next-task selection.
- Validate that the child is inside the parent’s permitted task scope.
- In Work, validate the child belongs to the dispatched Job’s approved task tree.
- In Pair, validate the child is related to the selected current task or otherwise explicitly authorized by policy.
- Resolve the existing execution surface; do not create one.
- Mint a child capability envelope independently of parent capabilities.
- Require user approval for authority that existing policy requires (external effects, destructive actions, deploy/push, durable memory, or scope expansion).

**Exit gate:** Tests prove a child cannot escape task scope, execution surface, or parent authority.

### Phase 2 — Shared workspace concurrency and mutation policy

A real delegate cannot share a writable checkout safely by convention alone.

- Define reservations against the typed execution surface, with ownership, scope, expiry/heartbeat, and release on every terminal path.
- Start with the narrowest safe mutation capability: one writer per reserved scope; overlapping writes are rejected or blocked.
- Define whether non-overlapping path reservations are reliable enough initially; if not, use an intentionally global workspace reservation and mark its ceiling with a `ponytail:` comment and an upgrade path.
- Ensure Pair attached workspaces and Work sandbox workspaces use the same reservation API and overlap semantics.
- Decide how the parent behaves while a child has a reservation (read allowed; conflicting writes blocked or require cancellation/approval).

**Exit gate:** Parallel delegates cannot make conflicting writes in either Pair or Work, and cancellation/crash recovery releases stale reservations safely.

### Phase 3 — Real child executor adapters

- Implement one shared delegated-run launcher and lifecycle driver.
- Add the Pair adapter targeting its existing attached context.
- Add the Work adapter targeting the existing Job sandbox, never a new sandbox.
- Supply only bounded task context and minted tools to the child.
- Route output, tool evidence, failures, and completion into the normalized delegation result.
- Ensure child loops cannot recursively dispatch or delegate unless a later, explicit policy allows it.

**Exit gate:** A delegated task performs real work end-to-end in both contexts and returns a structured result to its parent.

### Phase 4 — Parent review, settlement, and user surfaces

- Return a stable delegation handle and compact status to the parent.
- Provide parent status, cancellation, and scoped decision-response actions.
- Require parent review before accepting a result or applying proposed task settlement.
- Render concise status/evidence in conversation and detailed structured/raw evidence in run diagnostics from the same normalized outcome model.
- Add user-visible approval and cancellation flows required by policy.

**Exit gate:** A parent can delegate, observe, cancel, answer a bounded question, review evidence, and settle the appropriate task without inspecting child internals.

## Acceptance tests

Before exposing `delegate_task`:

- Pair and Work invoke the same delegation service and produce the same lifecycle states.
- A delegation never creates a Job or a sandbox.
- Work delegation remains inside the dispatched Job’s task tree.
- The child cannot use a tool or credential available only to its parent.
- Explicit selection is required; pending task order never selects a delegate automatically.
- Conflicting concurrent writes are prevented on both execution-surface kinds.
- Parent cancellation, child failure, and process loss leave auditable terminal state and release reservations.
- A child report does not itself settle a Docket task; parent acceptance is required.
- The smallest runnable integration check proves a real child execution can produce a result in Pair and Work contexts.

## Explicit non-goals for the first implementation

- No free-form multi-agent chat.
- No automatic task decomposition or automatic delegation.
- No recursive delegation.
- No task-subtree delegation unless scope and settlement semantics are defined separately.
- No separate Work-only delegation API.
- No per-task Work sandbox provisioning.
- No placeholder/read-only/intention-only `delegate_task` tool.

## Revisit trigger

Resume this plan only when real delegated execution is prioritized and its prerequisites—artifact/result references, a canonical child-run lifecycle, and workspace reservation semantics—can be delivered as a coherent series. Until then, Pair works its current task directly and Work progresses through its dispatched Job inside its single existing sandbox.
