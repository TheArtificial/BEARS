# Docket Work checkpoint control

## Status

Implementation contract for Docket-owned rejection escalation. It extends
[Canonical Docket execution attempts](docket-execution-attempts.md) without
introducing a second continuation authority.

## Authoritative correlation

A Work checkpoint is owned by the canonical `docket_execution_attempts` row,
not by a sandbox session or a Den runtime run alone.

```text
execution attempt (id, fence, WorkRun)
  -> Work checkout binds exactly one BearWire session
  -> a runtime turn starts for that bound session
  -> runtime run records checkpoint artifact
```

`work.checkout` already binds the Work run to the BearWire `session_id`. Before
starting a runtime turn, BearWire resolves that live Work run from the same
session and passes it as `CheckpointAuditContext`; checkpoint persistence then
attaches the artifact to the Work run, Job, and task. This session binding is
the only bridge from a Work attempt to a runtime run.

Do not add a `work_run -> runtime_run` reservation, create a synthetic runtime
run, or let the checkpoint installer decide whether Work may continue. A runtime
run is allocated only by the normal turn-start path. Its checkpoint is evidence
for the already-authorized execution attempt.

Persist the attempt correlation alongside the existing checkpoint audit fields:

```text
bear_run_checkpoints:
  execution_attempt_id UUID NULL REFERENCES docket_execution_attempts(id)
  execution_attempt_fence_epoch BIGINT NULL
```

The runtime resolves the live Work run by session, then resolves its live
canonical Work attempt. It writes both values atomically with the checkpoint.
The pair must match the attempt's current fence. Existing checkpoint rows remain
readable with both values null.

## RequireCheckpoint control

Add `RequireCheckpoint` to `DocketExecutionDisposition`. It is a Docket
control decision, distinct from scheduler-observation delivery dispositions.
It means: **the currently authorized Work attempt may not start another runtime
turn until it acknowledges a checkpoint directive.** It is not permission to
continue.

Docket persists a directive keyed by:

```text
(execution_attempt_id, fence_epoch, rejection reason, occurrence)
```

The directive has `pending | acknowledged | superseded` state and the produced
checkpoint artifact reference. Replaying the same rejection returns the same
pending directive; a stale fence cannot acknowledge it. A newer attempt/fence
supersedes an unresolved directive for the old attempt.

The escalation policy remains Docket-owned. The existing repeated-rejection
counter may choose `RequireCheckpoint` at its configured threshold; it must not
call runtime code directly.

## Acknowledgement and reauthorization

1. `work.checkout` receives a rejected `RequireCheckpoint` gate with directive
   ID, attempt ID, and fence. It receives no workspace permission and does not
   start a model turn.
2. A bound Work runtime starts the normal checkpoint-only turn using the
   existing checkpoint installer. That turn resolves the Work run and attempt
   through the session binding above, and persists the artifact with the
   directive correlation.
3. Runtime acknowledges the directive to Docket with directive ID, artifact
   reference, attempt ID, and exact fence. Docket atomically validates the
   artifact correlation and marks it acknowledged. Duplicate acknowledgement is
   successful only when all immutable values match.
4. Acknowledgement clears only the checkpoint wait. It neither marks the gate
   `Allowed` nor dispatches Work. The next checkout/dispatch must obtain a
   **fresh Docket `Allowed` decision** for the current attempt/fence.

If checkpoint creation or acknowledgement fails, the directive stays pending
and dispatch remains denied. On recovery, Docket replays the pending directive;
runtime can safely retry checkpoint creation using a deterministic directive
checkpoint ID. If the Work session/runtime run disappears, the attempt recovery
path stops or releases the attempt according to normal lease rules. It never
fabricates acknowledgement.

## Direct boundary decision contract

At each meaningful Work safe boundary—checkout, before a new runtime turn, and
before resuming after a checkpoint—Work sends its immutable attempt ID and
exact fence to Docket. Docket owns the response and returns exactly one typed
decision:

```text
Allow | RequireCheckpoint(directive) | Pause | Release | Stop
```

`Allow` is an authorization for only the next bounded runtime window. It does
not select another task, renew a lease, or reset a hard budget/failure fuse.
`RequireCheckpoint` is the durable directive defined above; `Pause`, `Release`,
and `Stop` prevent a new turn. The runtime treats an absent, stale, or
mismatched response as non-allowing.

The request and response are correlated by `(attempt_id, fence_epoch,
boundary_key)`. Docket persists the policy-changing result atomically with the
attempt/directive state. Repeating an exact request returns the same decision;
a stale fence is rejected; a successor fence never inherits an old `Allow`.
Transient transport retry may reissue the exact boundary request, but needs no
scheduler-observation/outbox record. After restart, Docket reloads the live
attempt and any pending directive, returns the deterministic current decision,
and Work must revalidate before continuing.

## Implementation boundaries

- `den-docket/model.rs`: add typed `RequireCheckpoint` gate/disposition and
  directive models.
- `den-docket/work_runs.rs`: persist/replay rejection directives and require a
  fresh post-acknowledgement authorization decision.
- `den-bearwire/methods/work.rs`: project a rejected directive without granting
  sandbox permission; add an explicit checkpoint acknowledgement request.
- `den-bearwire/methods/run.rs`: resolve the attempt alongside existing
  session-to-Work audit context before checkpoint persistence.
- `den-runtime/agent_loop/checkpoints.rs`: persist attempt/fence correlation
  and expose the resulting artifact reference for acknowledgement.

## Required regressions

1. Threshold rejection creates one idempotent `RequireCheckpoint` directive.
2. Checkout with a pending directive is denied and cannot dispatch tools.
3. Checkpoint artifact carries matching Work run, Job, task, attempt, and fence.
4. Exact acknowledgement is idempotent; stale/mismatched fence or artifact is
   rejected.
5. Acknowledgement alone remains non-dispatchable; a fresh `Allowed` gate is
   required.
6. Restart/retry preserves a pending directive; a successor fence cannot
   acknowledge its predecessor's directive.
7. Boundary decisions are idempotent for an exact attempt/fence/boundary key;
   stale or restarted Work must revalidate and never treats an absent decision
   as `Allow`.
