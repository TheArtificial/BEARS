# Work-Run Finalization Resilience Roadmap

## Status

Proposed.

## Problem

A work run can reach a normal Armature terminal event while its Docket work-run record is persisted as `blocked` because finalization observed an earlier snapshot with incomplete tasks. Later task-result writes complete the job, leaving contradictory durable state:

- Armature evidence says the turn completed.
- all job tasks may be `done` and the job may be completed;
- the work run remains terminally `blocked` with an outcome saying tasks were unfinished.

This is not primarily a UI-refresh problem. It is a finalization protocol bug: more than one runtime path reads mutable task state and independently derives an outcome, without a durable causal boundary between task mutations and a terminal turn event.

The result is fragile under normal concurrency, retries, crashes, and eventual delivery. It also makes the code difficult to read because outcome policy is split across dispatcher harvesting, Docket settlement, and presentation logic.

## Goals

1. Make a work-run terminal outcome deterministic, causally correct, and durable.
2. Recover safely from process crashes or duplicate delivery during finalization.
3. Give one module ownership of completion policy and user-facing structured blockers.
4. Keep terminal outcomes immutable during normal operation.
5. Preserve enough immutable evidence to diagnose why a run settled as it did.
6. Render the same canonical status and evidence in conversation, run diagnostics, and the web UI.

## Non-goals

- Do not make a completed Armature turn automatically mean that task criteria passed.
- Do not add a perpetual reconciliation loop that silently rewrites terminal work runs after unrelated task changes.
- Do not make the UI independently derive run status from its own live task query.
- Do not change the public terminal-state vocabulary unless a separate compatibility decision requires it.
- Do not introduce a second work coordinator or a broad new workflow framework.

## Design principles

### One authoritative finalizer

Docket owns the final work-run outcome reduction. Runtime dispatch and Armature integration collect and persist evidence; they do not decide `succeeded` versus `blocked` from a live task read.

### A terminal turn needs a causal boundary

Finalization must use the task/criterion projection that belongs to the terminal turn—not an arbitrary concurrent database snapshot. A turn is **sealed** only once its accepted task and criterion mutations are durably ordered before its terminal event.

### Terminal outcomes are immutable facts

Normal task changes after a sealed terminal turn do not revise that work run. They belong to an explicit later retry, resume, or new run. This preserves causal history and prevents an unrelated manual task completion from falsely healing a past failed/incomplete run.

### `reporting` is non-terminal recovery state

Use the existing `reporting` state as the durable finalization phase:

```text
queued -> claimed -> provisioning -> running -> reporting -> terminal
```

A crash or retry in `reporting` is recoverable. A terminal state is only committed after the finalizer has reduced sealed evidence and the relevant Docket projection.

### Structured evidence, not rendered strings

Persist typed outcome codes and blockers. Render concise wording in conversation and UI, with raw Armature evidence available in diagnostics. Avoid deriving policy from text or parsing rendered summaries.

## Target protocol

### 1. Persist executor evidence

When Armature reports a terminal turn event, persist immutable run evidence, including:

- terminal executor result (`completed`, `failed`, `cancelled`, or timed out);
- Armature report and diagnostic references;
- sandbox and publishing result, where applicable;
- terminal turn identifier and/or monotonic event sequence;
- finalizer/reducer version.

The work run transitions from `running` to `reporting`; it is not yet terminal.

### 2. Seal the turn

The event-ingestion boundary must order accepted task/criterion mutations and terminal turn events. Conceptually:

```text
task-status mutation #41 committed
task-status mutation #42 committed
terminal turn event #43 committed
turn sealed through #43
```

The seal means that all accepted mutations causally preceding the terminal event are durable and visible to the finalizer. It is an ordering guarantee, not a timeout, polling delay, or best-effort retry.

A mutation received after sealing is genuinely late. Product policy must make it either invalid for that job run or part of an explicit later run; it must not silently change the outcome of the sealed work run.

### 3. Finalize once from the sealed projection

A single idempotent Docket finalizer locks or otherwise serializes the work run and its owning job run, reads the sealed task/criterion projection, and commits the resulting settlement atomically where the storage boundary allows:

1. load immutable executor and publish evidence;
2. load task and criterion state for the work run's defined scope at the seal boundary;
3. calculate typed blockers;
4. reduce evidence and blockers into the canonical work-run outcome;
5. persist the terminal work-run state and structured outcome;
6. settle the job run where its completion rules are met;
7. emit exactly one terminal event.

If a previous finalization attempt already committed the same result, return that result. If a worker died before committing, a recovered `reporting` run safely runs the same reducer again.

## Ownership and module boundaries

| Area | Responsibility | Must not do |
| --- | --- | --- |
| Armature / work runtime | Execute turns, report terminal facts, persist ordered task/criterion mutations | Decide work-run success from ad hoc task reads |
| Turn/event ingestion | Assign/maintain event order and seal terminal turns | Apply terminal outcome policy |
| `den-docket` finalizer | Read sealed projection, reduce outcome, persist settlement atomically and idempotently | Execute task bodies or interpret raw transcript strings |
| UI and conversation projections | Render canonical outcome and evidence at appropriate detail | Recompute status from live task queries |

This leaves a narrow API at the dispatch boundary, conceptually `finalize_work_run_from_sealed_turn(run_id)`, rather than dispatcher-local helpers that precompute success, blocked state, or summary text.

## Canonical outcome model

Use strongly typed internal values; exact Rust names may differ:

```rust
struct CompletionGate {
    task_blockers: Vec<TaskBlocker>,
    criterion_blockers: Vec<CriterionBlocker>,
    executor_outcome: ExecutorOutcome,
    publish_outcome: PublishOutcome,
}

enum WorkRunOutcomeCode {
    Succeeded,
    CompletionGateUnmet,
    ExecutorFailed,
    PublishFailed,
    Cancelled,
    TimedOut,
}
```

Each blocker identifies its kind, scoped Docket identifier, observed status, and concise machine-readable reason. Avoid raw `serde_json::Value` beyond the boundary where an external Armature payload is parsed.

### Precedence policy

The reducer must state and test precedence explicitly:

1. Explicit cancellation yields `cancelled`.
2. A deadline/lease timeout yields `timed_out`.
3. Executor or infrastructure failure that prevents verification yields the applicable failure/blocked outcome.
4. Executor completion with unmet sealed task or criterion requirements yields `CompletionGateUnmet` (stored as the existing `blocked` state if that is the current compatible terminal vocabulary).
5. Executor completion with a satisfied sealed gate yields `succeeded`.

This separates **executor completed** from **work requirements were satisfied**. In UI copy, a completion-gate result should say that work ended before completion was verified, rather than presenting it as a generic task-level block.

## Data and migration requirements

The implementation should inventory existing Docket work-run and job-run tables before changing schema. Expected additions or confirmations:

- terminal turn/seal identifier and causal sequence or projection version;
- persisted structured outcome code and blockers;
- immutable executor and publication evidence references;
- finalization attempt/version metadata sufficient for idempotency and diagnosis;
- a unique terminal-event/finalization guard.

Prefer extending existing Docket records/migrations over creating parallel finalization tables unless the current schema cannot model immutable evidence cleanly. Schema changes must be migration-backed and preserve existing required query paths.

## Implementation phases

### Phase 0 — Trace and specify the current lifecycle

**Purpose:** Establish the real call graph and persistence ordering before modifying behavior.

- Identify every current writer of work-run state/outcome, job-run state, task-run state, criteria state, and terminal events.
- Locate all success/blocked predicates and outcome-summary formatting helpers.
- Document the exact relationship among Armature terminal events, tool/task status writes, dispatch harvesting, and job settlement.
- Select the authoritative turn/event sequence source and the exact work-run task scope.

**Exit criteria:** A short design note names the current writers, identifies every duplicate outcome reducer, and defines the seal source of truth and scope semantics.

### Phase 1 — Define typed finalization inputs and invariants

**Purpose:** Make correctness rules visible in types and tests before moving code.

- Define internal typed executor evidence, publication evidence, completion blockers, and outcome code structures.
- Specify finalization invariants:
  - a terminal run derives from one sealed turn;
  - finalization reads only state in that turn's scope and seal boundary;
  - terminal settlement is immutable;
  - duplicate finalization is idempotent;
  - one terminal Docket event is emitted.
- Decide policy for post-seal task mutations and document the error/result shape.

**Exit criteria:** The invariants and precedence table are documented next to the reducer API, with focused unit coverage for outcome reduction.

### Phase 2 — Establish sealed-turn ordering

**Purpose:** Eliminate the race rather than compensate for it afterward.

- Persist/confirm a durable sequence or equivalent causal token for task/criterion mutations and terminal turn events.
- Add a terminal turn seal operation that is only valid after causally prior mutations commit.
- Ensure finalization reads the projection at the seal boundary, not a later unconstrained live state.
- Reject or explicitly route post-seal mutations to subsequent work; do not silently attach them to the finalized run.

**Exit criteria:** A deterministic test proves that task completion committed before the terminal seal is included, while a mutation after the seal cannot alter that run's result.

### Phase 3 — Centralize Docket finalization

**Purpose:** Give one legible reducer ownership of terminal outcome.

- Implement one Docket service/finalizer operation for sealed work-run settlement.
- Move duplicated completion predicates and outcome-summary policy into this reducer.
- Atomically persist terminal work-run state/outcome, applicable job-run settlement, and the one terminal event.
- Make duplicate calls return the already-settled result without duplicate events.

**Exit criteria:** No dispatcher, handler, or UI path independently chooses work-run terminal status; all terminal outcomes originate in the Docket finalizer.

### Phase 4 — Simplify dispatcher and recovery behavior

**Purpose:** Keep runtime code factual and make crashes recoverable.

- Change harvesting to persist Armature facts, transition to `reporting`, seal the turn, and request finalization.
- Remove pre-finalization task reads that choose `Succeeded` or `Blocked`.
- Recover expired or interrupted `reporting` runs through the idempotent finalizer.
- Ensure genuine executor failures retain their immutable evidence and correctly reduce under the same policy.

**Exit criteria:** An injected failure between terminal evidence persistence and settlement recovers to the same final outcome after retry, without operator repair.

### Phase 5 — Projection and diagnostics alignment

**Purpose:** Prevent semantic drift across surfaces.

- Make web UI, conversation status, and run/log diagnostics consume the canonical structured outcome.
- Render concise current status in conversation/UI and richer blocker/evidence detail in diagnostics.
- Retain raw Armature report as evidence, not as the source of status policy.
- Clearly distinguish executor completion from completion-gate success.

**Exit criteria:** The reported contradiction cannot appear: a completed job and work-run view use the same canonical settlement, and diagnostic history remains available without being presented as current status.

### Phase 6 — Repair historical contradictions conservatively

**Purpose:** Correct provable legacy records without creating a hidden ongoing reconciler.

- Build a one-time, auditable repair command/report.
- Repair only records for which ordered evidence proves that relevant task/criterion writes preceded the terminal boundary yet were omitted by old finalization.
- Leave ambiguous historical records intact and expose their inconsistency diagnostically.
- Record repair provenance separately from the original evidence.

**Exit criteria:** A dry-run identifies candidate rows and rationale; production repair is explicit, reviewable, and idempotent.

## Verification plan

Keep tests focused around the actual lifecycle. At minimum:

1. **Reported race:** final task mutation commits before terminal seal; finalizer produces `succeeded` and completes the job run.
2. **Actual incomplete work:** seal occurs with a pending task or unmet criterion; finalizer produces `CompletionGateUnmet` and the compatible terminal state.
3. **Late mutation:** a task update after seal is rejected or belongs to a later run; it cannot convert the finalized run to success.
4. **Duplicate delivery:** repeated terminal event/finalizer request produces one terminal outcome and one terminal event.
5. **Crash in reporting:** a recovered worker finalizes the run correctly and idempotently.
6. **True blocker:** an explicitly blocked task is distinguishable from merely unfinished work in structured evidence and rendering.
7. **Executor failure:** executor error cannot be misreported as success merely because task state happens to be complete.
8. **Validation semantics:** a task cannot be marked done when its stated validation did not run or pass, unless an explicit auditable waiver policy exists.
9. **Projection consistency:** conversation and web/run views render the same outcome code and blockers from canonical stored data.

Use existing Rust test facilities and database test patterns; do not add a test framework solely for this work. The concurrency tests should use deterministic barriers or transaction/event hooks rather than timing sleeps.

## Rollout and observability

- Add structured logs/metrics around `reporting`, turn sealing, finalization attempts, idempotent replays, and outcome code counts.
- Initially compare the new reducer's result with the legacy path in diagnostics only, where possible, before retiring the old path.
- Deploy schema support before enabling the new finalizer behavior.
- Enable recovery of `reporting` runs only after idempotency and locking/serialization tests pass.
- Retire old duplicate reducers and stale status formatting immediately after migration; do not keep vestigial compatibility paths in active execution.

## Risks and decisions to resolve during Phase 0

| Question | Required decision |
| --- | --- |
| What precisely scopes a work run? | Define whether all job tasks, a dispatched subtree, or task-run rows associated with the run form the completion gate. |
| What establishes event order? | Use the authoritative persisted event stream/sequence, not wall-clock timing. |
| Can terminal work runs be reopened? | Default: no; create/resume a later run explicitly. |
| How should executor failure and unmet criteria compose? | Define precedence and retain both as structured evidence. |
| Is `blocked` the correct stored state for incomplete completion gates? | Preserve it initially if compatibility requires it; distinguish it with `CompletionGateUnmet` outcome code and UI copy. |
| What is a valid task `done` assertion? | Require evidence that completion criteria passed, or an explicit auditable waiver. |

## Definition of done

This roadmap is complete when a terminal work run is settled exactly once by a Docket-owned, idempotent reducer over a durably sealed turn projection; the run cannot be falsely blocked by a pre-completion snapshot; crash recovery from `reporting` is safe; later task changes cannot rewrite historical terminal truth; and every status surface renders the same canonical outcome and evidence.
