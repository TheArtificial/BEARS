# Docket test regime

Docket owns task selection, canonical execution attempts, continuation decisions, and recovery. A selected task is an objective; it is **not** execution authority. Pair work may continue only while Docket owns a matching canonical attempt with a valid fence.

This is Den's first domain-owned test regime. Future domains use sibling runners rather than adding opaque tests to a shared catch-all command:

```text
scripts/test-docket.sh
scripts/test-cabinet.sh
scripts/test-armature.sh
scripts/test-runtime.sh
```

## Lanes

```bash
cd services/den
./scripts/test-docket.sh policy
./scripts/test-docket.sh postgres
./scripts/test-docket.sh pair-loop
./scripts/test-docket.sh recovery
./scripts/test-docket.sh all
```

Database-backed lanes require a disposable Postgres database:

```bash
DATABASE_URL=postgres://postgres:postgres@localhost:5432/den_test \
  ./scripts/test-docket.sh all
```

The runner rejects unknown lanes and fails if its selector discovers zero tests. A successful compile with zero discovered tests is not test evidence.

| Lane | Purpose |
| --- | --- |
| `policy` | Pure transition, fencing, and continuation-decision tests. |
| `postgres` | Durable task/job/attempt control-plane transitions and atomic terminal settlement. |
| `pair-loop` | Client-visible focused-loop start and deterministic multi-slice continuation coverage. |
| `recovery` | Released-attempt idempotency and stale-owner fencing. |

`all` runs every implemented lane. Every lane has at least one real selector; the runner fails rather than silently accepting an empty lane.

## Pair-loop contract

A Pair-loop test must observe the real runtime lifecycle, not merely a database row or accepted RPC.

1. Assigning a ready task projects the objective but creates no running attempt or Pair run.
2. Explicit focus transfers control to Docket and creates a correlated Docket run, Pair run, canonical attempt, and fence.
3. The client-visible stream must contain `run.started` for that Pair run before any `run.completed`, `run.failed`, or terminal Docket-control release.
4. Across each Docket-authorized bounded slice, the same task, attempt identity, fence epoch, and Pair-run lineage stay authoritative until Docket chooses a different state.
5. Only Docket's `Continue` may authorize another slice. Generic chat `run.start` must not replace a live focused attempt.
6. Explicit completion, a genuine external block, user interruption, approval wait, or owner loss must produce their distinct terminal/paused outcomes; no stale continuation may revive released authority.

Use correlation IDs and bounded event waits. Do not use an external LLM or timing sleeps as proof that a loop started or continued.

## Current tests

`docket_execute_starts_pair_loop_for_selected_task` is a **postgres/control-plane** test. It covers task assignment, focus ownership, child creation, default settlement, successor selection, completion, and chat handback. Its event wait and pre-settlement assertions are the immediate failure-to-complete regression coverage: before the test settles anything, the exact focus-created host run must be client-visible, remain `running`/`continuing`, have one matching running Docket attempt, and have emitted no terminal event. It does **not** prove a live loop survives a continuation boundary.

`focused_pair_loop_continues_across_two_bounded_slices` is the feature-gated **pair-loop** integration test. Its typed, run-correlated fixture supplies two actual `RuntimeSemanticEvent::BoundedSlice` events and then a pending stream. It proves that the production continuation path constructs the initial stream plus two continuations for the same focused Pair run, task, canonical attempt, and fence epoch. It then settles the only task through `docket.jobs.settle_task`, verifies the attempt is released, the response selects `job_completed` (ordinary-chat handback), and no further continuation is scheduled.

## CI

The `Docket lifecycle` CI job runs `./scripts/test-docket.sh all` against Postgres with `SQLX_OFFLINE=false`. It is intentionally separate from compile/lint and general unit jobs. Changes to Docket, BearWire focus control, canonical attempts, or relevant migrations must keep this job green.
