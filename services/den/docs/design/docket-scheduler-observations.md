# Docket scheduler observations

## Purpose

Docket is the sole authority for task-tree eligibility, execution bindings, and
rejection policy. A scheduler observation is the narrow, typed bridge by which
Docket can notify a *currently live, already authorized* runtime session that
its binding is no longer usable.

Pre-dispatch rejection is not an agent-loop checkpoint: it returns a rejected
gate to Pair or Work, which must not dispatch or invoke the model.

## Authority boundary

```text
Docket execution binding / rejection decision
→ durable scheduler observation
→ exact live-session delivery
→ runtime applies only Docket's disposition
```

Runtime may pause a live stream, display a status, and install a checkpoint. It
must not derive task eligibility, count scheduler rejections, choose a retry
policy, or make a checkpoint acknowledgement authorize a task.

After any pause, execution requires a fresh Docket gate. A checkpoint response
is advisory and never changes task state or an execution binding.

## Typed contract

The producer-owned record should be equivalent to:

```rust
struct DocketSchedulerObservation {
    observation_id: Uuid,
    binding: DocketExecutionBinding,
    task_id: Option<Uuid>,
    reason: DocketExecutionReason,
    occurrence: u32,
    disposition: DocketSchedulerObservationDisposition,
    delivery_state: DocketSchedulerObservationDeliveryState,
}

enum DocketSchedulerObservationDisposition {
    Reconcile,
    Stop,
    RequireCheckpoint,
    RequireIntervention,
}

enum DocketSchedulerObservationDeliveryState {
    Pending,
    Delivered,
    Superseded,
}
```

`binding` is opaque to runtime and identifies the exact Pair session or Work
run that Docket previously authorized. For Pair, the existing
`docket_execution_sessions` row already carries `session_id`,
`source_client_session_id`, and `source_conversation_id`; these are the
correlation fields delivery must validate rather than re-deriving identity from
a task projection. `occurrence` is assigned and persisted by Docket, scoped to
that binding and reason; it is not a runtime retry count.

The initial implementation should expose only `Reconcile` and `Stop` until an
actual live-binding revocation needs `RequireCheckpoint`. `RequireCheckpoint`
must not be emitted merely because a pre-dispatch gate was rejected.

## Delivery rules

1. Docket writes an observation only while changing or invalidating an active
   binding.
2. BearWire resolves the observation only to the client/session represented by
   that binding; it does not broadcast it or synthesize a model prompt.
3. Runtime accepts only a delivery whose binding matches its active execution
   identity.
4. For `RequireCheckpoint`, runtime maps the typed observation to a future
   `CheckpointReason::TaskGateRejected` and uses the existing checkpoint
   installer.
5. Runtime records delivery acknowledgement as transport evidence only. It
   cannot change Docket task state, the observation disposition, or authorize
   dispatch.
6. `Reconcile`, `Stop`, and `RequireIntervention` are rendered as concise
   Den-owned statuses and end or pause the relevant control path without a
   model retry.

## Persistence and privacy

The durable record may link to the Docket job run, task, Work run, and
conversation/session correlation required for exact delivery. It must not
store prompt text, task-tree snapshots, raw provider/model output, or
checkpoint prose. Rich audit evidence stays in Docket diagnostics; the
conversation surface gets a short Den-formatted status.

## Required tests before implementation

- A pre-dispatch rejection creates no scheduler observation and no model
  checkpoint.
- A live binding invalidation persists one observation with Docket-owned reason,
  occurrence, and disposition.
- Delivery to a mismatched session/binding is rejected.
- Only `RequireCheckpoint` invokes runtime's checkpoint installer.
- A checkpoint acknowledgement leaves the gate rejected; a fresh allowed gate
  is required for further dispatch.
- Duplicate delivery is idempotent and cannot produce multiple checkpoints.
