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
./scripts/test-docket.sh postgres
./scripts/test-docket.sh pair-loop
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
| `policy` | Pure transition, fencing, and continuation-decision tests. Reserved until those tests are factored into a dedicated module. |
| `postgres` | Durable task/job/attempt control-plane transitions and atomic terminal settlement. |
| `pair-loop` | Live BearWire stream behavior using a deterministic local scripted provider. |
| `recovery` | Owner loss, stale delivery, reconciliation, and retryable terminalization. Reserved until the recovery suite is factored. |

`all` currently runs the implemented `postgres` and `pair-loop` lanes. A reserved lane is deliberately not included until it has real tests; CI must never claim coverage from an empty selector.

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

`docket_execute_starts_pair_loop_for_selected_task` is a **postgres/control-plane** test. It covers task assignment, focus ownership, child creation, default settlement, successor selection, completion, and chat handback. It does not prove a live loop survives a continuation boundary.

`docket_execute_starts_pair_loop_for_selected_task` currently participates in both the **postgres** and **pair-loop** lanes: its settlement assertions are control-plane coverage, while its explicit event wait is the first pair-loop regression assertion. It fails when a focus-created loop terminates before emitting its client-visible `run.started` event. This is a transitional overlap, not the final structure; later work must extract a minimal dedicated scripted-provider test for deterministic multi-slice continuation, awaiting-user, interruption, and recovery scenarios.

## CI

The `Docket lifecycle` CI job runs `./scripts/test-docket.sh all` against Postgres with `SQLX_OFFLINE=false`. It is intentionally separate from compile/lint and general unit jobs. Changes to Docket, BearWire focus control, canonical attempts, or relevant migrations must keep this job green.
