# BearWire state management

BearWire carries delivery messages between Den and an Armature. It is **not** a
second domain-state machine. Before adding protocol state, identify the
existing Den-owned record that already owns the lifecycle.

## Canonical owners

| Concern | Canonical owner | BearWire's role |
|---|---|---|
| Client connection and session | BearWire session state | Connect, resume, and project state |
| Turn/run lifecycle | `turn_runs` | Project `run.state` |
| Tool and permission work, claim, lease, and result | `turn_obligations` | Project open obligations and carry claim/renew/result requests |
| Persistent notification replay | BearWire event log | Deliver a replayable notification; never decide work completion |
| Work-run settlement | Docket work-run lifecycle | Notify/provide the Armature-facing projection only |

`turn_obligations` is the authority for an action that needs an executor,
claim/lease ownership, retries after reconnect, or a terminal result. See the
source references in `den-bearwire/src/methods/client.rs` (claim, renewal, and
result acceptance), `den-bearwire/src/methods/run.rs` (open-obligation
projection), and `tools/bear-armature/src/bearwire.rs`
(`service_run_state_tool_obligations`).

## The rule

> Do not create a BearWire command ledger or command state machine for work
> that can be represented as a typed `turn_obligation`.

Persistent events are delivery projections. They can be replayed and may be
observed more than once; they must not become an independent source of action
status. An obligation owns the action payload, eligible responder, lease, and
terminal result.

A new stateful BearWire facility needs an explicit reason it cannot use an
obligation, documented next to its schema and reviewed as a new lifecycle
owner.

## Two execution paths

### Model-requested tool work

```text
model emits a tool request
→ Den persists a tool-result obligation
→ Armature claims it with an attempt token and renews the lease
→ Armature performs the native tool operation
→ Armature submits the correlated result
→ Den accepts one valid terminal result and advances the run
```

### Den-directed deterministic Armature action

A deterministic action follows the same lifecycle. It must not require a model
turn or expose an arbitrary command interface.

```text
Den derives typed action data from authoritative records
→ Den persists a typed obligation bound to its run/session/workspace
→ Armature claims it through the normal obligation protocol
→ Armature executes its native constrained capability
→ Armature submits the normal correlated result
→ Den's owning lifecycle consumes the result
```

For example, task settlement can create a `publish_task` obligation. Docket
derives its typed publication target and idempotency key from the task and work
run; Den authorizes the attempt and owns the durable settlement record;
Armature prepares or validates its workspace-local artifact. The publication
provider performs the external effect using scoped credentials that are never
shown to the model and need not be present in the sandbox. The first backend is
Git, but the obligation represents publication rather than a Git command, so
artifact registries and deployment targets can use the same lifecycle.

The payload must contain only typed, validated resource references and target
configuration. Den must not send filesystem paths, shell text, arbitrary Git
arguments, or credentials. The result records immutable provider evidence (for
Git, commit OID and remote ref; for an artifact registry, a digest; for a
deployment, a release/revision ID). Docket settles a task only after the
provider reports that evidence through the valid leased attempt.

## Design checklist

Before adding a BearWire message, answer:

1. **Which existing record owns the lifecycle?** Prefer `turn_obligations` for
   executable work and the event log only for delivery.
2. **What is the idempotency key?** Replays and reconnects must not duplicate a
   side effect.
3. **Who can execute it?** Bind responder, run/session, and workspace at
   creation; validate them on claim and result.
4. **How does it recover?** Use the existing lease expiry and replay path rather
   than a parallel retry loop.
5. **Who consumes the result?** The owning Den lifecycle, not the event stream.

Keep source comments short and link here rather than re-explaining this split.
