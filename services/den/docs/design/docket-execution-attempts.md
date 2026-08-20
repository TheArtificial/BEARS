# Canonical Docket execution attempts

## Status

Implementation contract for the continuation-authority revision in
`docs/roadmap/AGENT_LOOP_CONTROL_IMPLEMENTATION_PLAN.md`. This is the required
foundation before runtime delivery is added to scheduler observations.

## Boundary

Docket owns the **outer** continuation loop for every autonomous task attempt:

```text
eligible task -> authorize attempt -> receive durable outcome
              -> continue | advance | retry | pause | stop | await user
```

Pair and Work own their **inner** model/tool loops. They choose prompts and
local tool actions, enforce a Docket stop at safe boundaries, and report facts.
They never choose another Docket task or decide task-tree continuation.

A Pair `current_task_id` is an objective, not execution authority. A Work run's
assigned Job bounds task selection, but is not by itself authority to execute a
task. An autonomous start or resume needs a valid Docket execution attempt.
Ordinary untasked Pair conversation remains Den/runtime-driven and creates no
attempt.

## Record

`docket_execution_attempts` is the one durable authority record. It is distinct
from task status, Pair runs, Work runs, routing reservations, and turn-attempt
telemetry.

```rust
enum ExecutionAttemptOwner {
    Pair { session_id: ClientSessionId, pair_run_id: PairRunId },
    Work { work_run_id: WorkRunId },
}

enum ExecutionAttemptState {
    Authorized,
    Running,
    Paused,
    AwaitingUser,
    Stopping,
    Settled,
    Released,
}
```

Each record has an immutable ID, `task_id`, owner kind/correlation, monotonic
`fence_epoch`, state, authorization key, creation/update timestamps, and
state-specific timestamps (`started_at`, `paused_at`, `settled_at`,
`released_at`). The owner correlation is typed columns/foreign keys, not JSON.
A fence is opaque to the runtime except for equality: it must accompany every
start, resume, control acknowledgement, and outcome report.

`Authorized`, `Running`, `Paused`, `AwaitingUser`, and `Stopping` are live.
`Settled` and `Released` are terminal. Only one live attempt may exist for an
exclusive task/owner scope. The precise partial unique indexes are determined
by whether a task supports concurrent owners; the initial contract is one live
attempt per task and one per Pair session or Work run.

## Transitions

| Transition | Actor / atomic boundary | Idempotency and fencing |
| --- | --- | --- |
| authorize | Docket transaction: verify task eligibility and owner binding; allocate next fence; write attempt | authorization key returns the same live attempt; conflicting owner/task is rejected |
| start | runtime request accepted by Docket transaction, then runtime begins local loop | requires `Authorized` + exact fence; replay returns the same state |
| pause | runtime reports a bounded checkpoint; Docket transaction stores pause fact | requires exact live attempt/fence; duplicate report is a no-op |
| await user | Pair reports precise question; Docket stores question reference and pauses attempt | Pair only; exact fence; duplicate question key is a no-op |
| resume | Docket verifies explicit authenticated user response/resume for `AwaitingUser`, or continuation policy for `Paused`, then moves to `Authorized` or creates successor with new fence | stale fence is rejected; no implicit resume on reconnect |
| stop | Docket changes a live attempt to `Stopping`; runtime observes it at a safe boundary and reports release/outcome | stop is idempotent; it prevents new start/resume, not retroactive cancellation of a tool call |
| settle | runtime proposes typed terminal outcome; Docket validates task version/fence and atomically records outcome, task reduction, and attempt settlement | outcome idempotency key returns the accepted result; stale/duplicate conflicting reports are rejected |
| release | Docket reconciler releases an abandoned or superseded live attempt with synthetic provenance | compare-and-set on live state and fence avoids releasing a successor |

`Running` is set only by the successful Docket start transition, before inner
loop work. A selected task with no live attempt is never projected as executing.
A runtime may send progress/heartbeat facts while running, but they do not
renew authority or select further work.

## Outcomes and continuation

Runtime reports typed facts, not scheduling decisions: `completed`, `no_change`,
`blocked`, `failed`, `cancelled`, `checkpoint`, `awaiting_user`, and local stop
acknowledgement. Docket validates and reduces task/run state, then decides the
next continuation. `awaiting_user` is Pair-only, needs a precise question, and
is ineligible until authenticated user input or explicit resume is recorded.
Work has no user-input outcome; retry/timeout/escalation derives from Job/task
policy.

Commit/artifact delivery is post-settlement evidence. A delivery retry cannot
reopen an accepted attempt outcome.

## Failure and recovery

Transactions that grant authority also persist the attempt; transactions that
accept a terminal outcome also persist the reduction. Cross-process delivery is
at-least-once and every receiver validates attempt ID plus fence.

On restart/reconciliation, Docket must identify and resolve: ownerless live
attempts, live attempts whose Pair/Work owner no longer exists, selected tasks
presented as executing without a live attempt, expired Work ownership, and
reports bearing stale fences. It appends synthetic recovery provenance and
releases or stops the old attempt; it never fabricates model output. Lease
expiry means no new work and cannot roll back an already-issued external call.

## Legacy mapping

- `docket_execution_sessions` is migration input/correlation only; it is not a
  continuation authority after attempt creation.
- Pair runs and Work runs are owner-local execution/checkpoint history. They
  reference the attempt and fence; they do not authorize themselves.
- Docket routing reservations and turn-attempt telemetry remain inner-loop
  dispatch evidence and must reference the canonical attempt where task work is
  autonomous.
- Scheduler observations reference attempt ID, owner correlation, and fence.
  Existing observation records remain readable compatibility history; do not
  build live delivery until this rebase exists.

## Required persistence tests

Database-backed tests cover duplicate authorization, double start, stale fence,
duplicate terminal report, Pair await-user/resume authorization, owner loss,
and Work retry/advance. Runtime-gate tests cover Pair start/resume and Work
dispatch rejection without a valid authorized attempt.
