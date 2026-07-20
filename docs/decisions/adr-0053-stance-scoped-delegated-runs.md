# ADR-0053: Stance-Scoped Delegated Runs

**Status:** Proposed  
**Date:** 2026-07-11  
**Deciders:** Hans

**Related:**

- [ADR-0004: Artifacts, Garage (S3), and Cabinet separation](adr-0004-artifacts-garage.md)
- [ADR-0010: Dynamic skills, reflection subagents, and bear configuration](adr-0010-dynamic-skills-subagents.md)
- [ADR-0014: Multi-role runtime architecture](adr-0014-multi-role-runtime-architecture.md)
- [ADR-0034: Jobs and Tasks Work-Management Model](adr-0034-jobs-and-tasks-work-management.md)
- [ADR-0039: Trust profiles and governance](adr-0039-trust-profiles-and-governance.md)
- [ADR-0041: Archival recall and async curation](adr-0041-archival-recall-and-async-curation.md)
- [ADR-0045: Session task lists as Docket checkouts and working projections](adr-0045-session-task-lists-and-docket-checkout.md)
- [ADR-0050: Agent loop control, adaptive budgets, and runtime checkpoints](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md)
- [ADR-0052: Three-Layer Agent Steering](adr-0052-three-layer-agent-steering.md)

> **Amended by [ADR-0056](adr-0056-docket-driven-turn-routing.md).** A Docket task with `routing_strategy: delegated` resolves to a delegated run through this ADR's broker: the router decides where the transcript lives; the broker decides what the run may do. Router placement is never a bypass around delegation authorization. Until the broker exists, `delegated` resolves to the `work` work-run lane; the broker generalizes that lane without changing task metadata. Execution profiles for delegated runs are resolved by the ADR-0033 model-tasks layer, not by this ADR's capability bundles.
>
> ADR-0056 also narrows this ADR for Docket-driven runs: the `ParentCommand` vocabulary reduces to run control (`start | pause | resume | stop`); `AddContext`, `NarrowScope`, `ReviseObjective`, and `AnswerDecision` are realized as ordinary audited Docket task-tree mutations (e.g. updating a blocked `decision` task) picked up at turn/task boundaries, not as a parallel message channel. Steering and observation rights follow bear rights — jobs introduce no new ACL. And **execution surface** (Den-provisioned sandbox armature vs. a user's live armature) is a third explicit authorization axis alongside stance and mode: `work` on a user's armature is still `work`, never a new stance and never `pair`.

## Context

A Bear feels like one assistant to the user, but internally Den uses multiple stances such as `chat`, `pair`, `work`, `curate`, and `watch`. As Den grows more background-capable, a Bear in one stance often needs to delegate work to another loop: implementation work, memory curation, monitoring, archive review, or longer analysis.

A tempting simplification is to treat all background work as `work`. That is wrong. Background execution is a mode; `work` is an authority domain. If every asynchronous loop runs as `work`, then `work` becomes a junk drawer for code execution, memory curation, monitoring, inbox processing, and archival cleanup. That weakens auditability and risks accidental permission inheritance.

Den needs a way for models to request delegated work with minimal tooling overhead while preserving hard boundaries:

- the initiating stance must not transfer its tools, credentials, or side-effect authority to the delegated loop;
- the delegated loop must run in the narrowest stance that can complete the objective;
- parent and child loops need structured communication for progress, questions, artifacts, cancellation, and completion;
- scope expansion must pass through policy again rather than through informal agent-to-agent chat.

## Decision

Den will model subagent/background work as **stance-scoped delegated runs**.

A delegated run is a durable child run launched through a Den-owned delegation broker. It has:

- a resolved stance;
- an execution mode, usually `background`;
- a durable parent/child relationship;
- an explicit objective and success criteria;
- scoped capabilities minted for that run;
- structured event and artifact communication with its parent.

The core invariant is:

> Delegation is a request, not inheritance. A delegated run is authorized from first principles using its resolved stance, target surface, autonomy, user consent, and job policy. The initiating stance may provide context and intent, but never transfers tools, credentials, or side-effect authority.

A second invariant governs communication:

> Parent/child communication is capability-neutral. Messages may convey intent, context, questions, and results, but they do not confer authority. Any request that changes scope, target, autonomy, or side effects must pass through the delegation broker again.

## Stance vs execution mode

Den will keep stance and execution mode separate:

```text
stance = authority domain / role envelope
mode   = foreground | background
```

Examples:

```text
chat foreground
pair foreground
work background
curate background
watch background
```

`work` is the default for delegated runs only when the objective touches repos, workspaces, command execution, builds, tests, patches, or similar artifact production.

Default stance mapping:

| Objective / target | Default stance |
|---|---|
| User-facing conversation | `chat` |
| Collaborative reasoning or planning | `pair` |
| Repo, workspace, code, tests, commands, patches | `work` |
| Memory, archive, profile, recall, consolidation | `curate` |
| Waiting, polling, monitoring, scheduled checks | `watch` |

Mixed-domain objectives should initially be rejected for clarification or split into separate delegated runs.

`ponytail:` The first stance resolver may be a deterministic rule table over target kind and objective verbs. The ceiling is ambiguous mixed-objective tasks. The upgrade path is explicit target typing and safe deterministic task decomposition.

## Model-facing delegation tool

Models should not need to know scheduler internals, queue names, sandbox plumbing, or capability policy. Den should expose one boring delegation tool, conceptually:

```ts
delegate_run({
  objective: string,
  context?: string | ContextRef[],
  desired_stance?: "work" | "curate" | "watch" | "pair",
  target?: {
    kind: "repo" | "workspace" | "memory" | "conversation" | "job" | "artifact" | "external",
    ref?: string
  },
  autonomy?: "propose_only" | "read_only" | "can_write" | "can_execute",
  urgency?: "now" | "background" | "scheduled",
  wait_policy?: "fire_and_report" | "wait_for_completion" | "wait_until_blocked_or_complete" | "stream_progress",
  success_criteria?: string[]
})
```

`desired_stance` is advisory. The delegation broker may accept it only if it is compatible with the objective, target, and policy. Otherwise the broker resolves a stance or asks for clarification.

## Delegation broker

A Den-owned delegation broker sits between the initiating model and any delegated loop:

```text
model/tool call
  -> delegation broker
  -> stance resolver
  -> authorization engine
  -> scheduler / Docket run record
  -> stance-specific agent loop
```

The broker is responsible for:

1. normalizing the request;
2. inferring or validating the target surface;
3. resolving the narrowest capable stance;
4. checking whether the initiating stance may request that delegation;
5. checking target-surface access;
6. checking requested autonomy and side-effect policy;
7. requesting user approval when required;
8. creating or linking durable Job/Task/Run records;
9. minting scoped capabilities;
10. launching the delegated run;
11. returning a child run handle to the parent.

The initiating model never directly launches arbitrary agents.

## Authorization and capability minting

Authorization must check four independent inputs:

1. **Delegation edge:** May the initiating stance request this kind of delegated run?
2. **Target surface:** Is the resolved stance allowed to access this repo, memory surface, conversation, job, external source, or artifact?
3. **Autonomy:** Is the requested autonomy allowed for this stance, target, trust profile, and job policy?
4. **Side effects:** Does the requested action require explicit approval because it writes, deletes, commits, pushes, deploys, sends messages, promotes durable memory, or performs externally visible actions?

The delegated run receives a capability bundle scoped to the authorization decision, for example:

```json
{
  "run_id": "run_child",
  "stance": "work",
  "mode": "background",
  "objective": "Fix the failing parser test",
  "target": { "kind": "repo", "ref": "workspace/foo" },
  "capabilities": [
    "fs.read:workspace/foo",
    "fs.write:workspace/foo",
    "cmd.run:workspace/foo",
    "git.diff:workspace/foo"
  ],
  "denied_capabilities": [
    "memory.promote",
    "external.send",
    "deploy.production"
  ],
  "commit_policy": "propose_only"
}
```

Capabilities are minted for the child run. They are not inherited from the parent run.

Recommended defaults:

| Initiating stance | Default delegation posture |
|---|---|
| `chat` | May request read-only/propose-only delegated work from user intent; writes require task/user authorization. |
| `pair` | May request write-capable `work` when attached to user-approved task/job policy. |
| `work` | May request child `work` or `watch` within the same job/surface; cross-domain delegation is narrow. |
| `curate` | May request curate work; repo writes are denied by default. |
| `watch` | Read-only monitoring by default; notification or side-effect actions require explicit authorization. |

Durable memory promotion requires review or explicit policy approval. Externally visible effects and destructive actions require explicit approval.

## Parent/child communication

Delegated loops and delegating loops communicate through Den-owned structured records, not through shared ambient prompt state or free-form private chat.

A child run has a parent reference:

```ts
parent: {
  conversation_id?: string,
  parent_run_id: string,
  parent_task_id?: string,
  initiating_stance: "chat" | "pair" | "work" | "curate" | "watch"
}
```

The parent receives a child handle:

```ts
child: {
  run_id: string,
  stance: string,
  status: "queued" | "running" | "blocked" | "completed" | "failed" | "cancelled",
  objective: string
}
```

Communication uses four surfaces:

1. **Run events** for lifecycle and progress.
2. **Durable artifacts** for real outputs such as patches, briefs, memory proposals, test reports, and audit findings. Delegated runs cite outputs using artifact refs as defined by [ADR-0004](adr-0004-artifacts-garage.md); child loops do not mint artifact refs directly.
3. **Decision requests** for questions, blocks, and approvals.
4. **Parent commands** for cancellation, pausing, resuming, added context, narrowed scope, or revised objectives.

The child does not receive direct control over the parent and the parent does not receive direct control over the child's tools.

### Event vocabulary

Minimal child-to-parent events:

```ts
type ChildEvent =
  | RunStarted
  | Progress
  | ArtifactCreated
  | Blocked
  | RequiresDecision
  | Completed
  | Failed
  | Cancelled
```

Minimal parent-to-child commands:

```ts
type ParentCommand =
  | Cancel
  | Pause
  | Resume
  | AddContext
  | NarrowScope
  | ReviseObjective
  | AnswerDecision
```

Commands that expand target, autonomy, side effects, or stance are not simple messages; they must be submitted as delegation amendments and authorized again.

## Context passing

Context passed to a child must be explicit and filterable:

```json
{
  "objective": "Audit ADR-0041 memory automation followups",
  "context": {
    "conversation_excerpt_ref": "ctx_123",
    "job_ref": "job_456",
    "files": [
      "docs/roadmap/ADR_0041_REMAINING_FOLLOWUPS_PLAN.md"
    ]
  }
}
```

Rules:

- context is copied or referenced explicitly;
- context is filtered for the child stance;
- secrets are redacted by default;
- the child cannot read the full parent transcript unless granted;
- context additions after launch are recorded as events, not hidden prompt mutation.

## Wait policies

Delegation requests declare how the parent should relate to the child run:

| Wait policy | Meaning |
|---|---|
| `fire_and_report` | Parent may finish the current turn and surface completion later. |
| `wait_for_completion` | Parent waits until the child completes or times out. |
| `wait_until_blocked_or_complete` | Parent waits only for a block or completion boundary. |
| `stream_progress` | Parent relays selected progress events to the user. |

The parent remains responsible for user-facing synthesis unless policy explicitly grants the child user-visible communication authority.

## Visibility

Child events and artifacts should include visibility metadata:

```ts
visibility = "internal_audit" | "parent_only" | "user_visible" | "artifact_only"
```

Parents must not blindly dump child logs to the user. Tool traces are normally `internal_audit`; concise status may be `parent_only` or `user_visible`; large outputs belong in artifacts.

## Blocking and escalation

A child must block instead of exceeding its authority. Example block:

```json
{
  "type": "child_blocked",
  "run_id": "run_child",
  "reason": "Requested operation exceeds capability bundle.",
  "attempted_capability": "memory.write.durable",
  "available_capabilities": [
    "memory.review.create",
    "conversation.read:current"
  ],
  "suggested_next_step": {
    "kind": "request_approval",
    "new_autonomy": "can_write"
  }
}
```

The parent may then continue without that operation, request approval, spawn another stance through the broker, or cancel the child.

## Completion contract

Every delegated run must end with a normalized result:

```json
{
  "type": "child_completed",
  "run_id": "run_child",
  "status": "completed",
  "summary": "Found 2 memory candidates and 1 stale note.",
  "artifacts": [
    { "kind": "memory_review", "ref": "artifact_01JZ8WJ3Z3N3W5F0Y2F7P9Q4BH" }
  ],
  "changes_made": [],
  "decisions_needed": [],
  "followups": [
    "Review proposed consolidation."
  ]
}
```

For `work`, artifacts may include diffs and test reports. For `curate`, artifacts may include memory review proposals. For `watch`, artifacts may include monitoring summaries or terminal status snapshots. All artifact refs in delegated-run events and completion results are Den-minted registry refs from ADR-0004, not model-generated IDs, storage keys, or work-surface paths.

## Cross-stance child delegation

A delegated run may request its own child run, but only through the same broker and authorization path.

Examples:

- a `work` run may request a `watch` child to monitor CI for the same job;
- a `work` run may request a `curate` propose-only child to review whether completed work should produce memory candidates;
- a `curate` run may not acquire repo-write authority simply because it asks a `work` child unless the delegation is independently authorized.

Delegation trees are allowed. Permission inheritance across edges is not.

## Audit log

Den must record the delegation request and authorization decision, including:

- initiating stance;
- requested stance, if any;
- resolved stance;
- objective;
- target;
- requested and granted autonomy;
- capabilities minted;
- capabilities denied;
- approval state;
- reason for the decision;
- parent and child run IDs.

Example:

```json
{
  "event": "delegation_authorized",
  "initiating_stance": "chat",
  "requested_stance": null,
  "resolved_stance": "curate",
  "objective": "Review this conversation for durable memory candidates",
  "target": "conversation/current",
  "autonomy": "propose_only",
  "capabilities_minted": [
    "conversation.read:current",
    "memory.review.create"
  ],
  "capabilities_denied": [
    "memory.write.durable"
  ],
  "approval": "not_required",
  "reason": "curate propose-only review from current conversation is allowed"
}
```

## Consequences

Benefits:

- background work no longer collapses into the `work` stance;
- model-facing delegation stays simple;
- stance and permission boundaries are explicit and auditable;
- child runs can report progress, ask questions, and produce artifacts without shared ambient authority;
- Docket/jobs can become the durable substrate for background work;
- future clients can render delegated-run progress consistently.

Costs:

- Den needs a broker, stance resolver, permission matrix, scoped capability minting, and child-run event model;
- mixed-domain tasks may require clarification or decomposition;
- parent/child protocols add some ceremony compared with informal agent chat.

This ceremony is intentional. It is the minimum structure needed to support autonomous background work without turning subagents into ambient authority leaks.

## Minimal implementation plan

1. Add the `delegate_run` model-facing tool shape.
2. Implement a rule-table stance resolver.
3. Implement an initial delegation-edge permission matrix.
4. Mint scoped capability bundles for delegated runs.
5. Create durable parent/child run records backed by existing Job/Task/Run concepts where possible.
6. Emit minimal child lifecycle events: started, progress, blocked, artifact created, completed, failed, cancelled.
7. Add parent commands for cancel and answer-decision first; defer pause/resume and objective revision unless needed.
8. Require normalized completion results.
9. Add audit events for every authorization decision.

Do not build a general agent chat bus, broad automatic decomposition system, or new subagent framework before the broker/capability boundary exists.
