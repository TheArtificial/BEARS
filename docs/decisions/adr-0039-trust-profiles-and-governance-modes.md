# ADR-0039 — Trust profiles and governance modes

**Status:** Accepted (2026-06-12)
**Deciders:** Hans
**Related:**
- [ADR-0036 — Bear profile registry and binding vocabulary](adr-0036-bear-profile-registry.md)
- [ADR-0037 — Work sandbox, egress gateway, and upstream auth](adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md)
- [ADR-0035 — Den-native in-process agent runtime](adr-0035-den-native-in-process-agent-runtime.md)
- [ADR-0034 — Jobs and tasks (Docket)](adr-0034-jobs-and-tasks-work-management.md)
- [ADR-0026 — Work handoff and human escalation](adr-0026-work-handoff-and-human-escalation.md)
- [ADR-0006 — Bear work surfaces](adr-0006-bear-work-surfaces.md)
- [`interactive-profiles-and-role-axes.md`](../architecture/interactive-profiles-and-role-axes.md)
- [`work-surfaces-and-conversations.md`](../guides/work-surfaces-and-conversations.md)

## Context

Bear Den models capability-specific runtimes as **profiles** (`chat`, `pair`, `curate`, `work`, `watch`) over one Den-native agent loop ([ADR-0035](adr-0035-den-native-in-process-agent-runtime.md), [ADR-0036](adr-0036-bear-profile-registry.md)). A profile bundles a trust boundary (which memory branches are readable/writable, approval/autonomy defaults, cross-role visibility) with a default compiled prompt and tool roster.

Designing the **handoff seams** between interactive and autonomous work exposed that profiles were doing two jobs at once:

1. a **durable trust contract** — non-negotiable boundaries such as "`work` must not read raw `pair/`" and "`curate` is the sole routine writer to `core/`"; and
2. a **runtime execution switch** — "this sandbox is a `pair` session" vs "this sandbox is a `work` run".

Three scenarios make the conflation untenable:

- **Remote `pair` loses its client.** The user expects the session to keep running in the cloud until they return. Naively this is "flip `pair` → `work`", but that would silently change memory scope and approval semantics mid-session.
- **Long-running `work` the user wants to interrogate.** Naively this is "attach a `pair` turn", but that collides with turn ownership and the `work`/`pair` memory wall.
- **Panic / checkpoint.** The user wants to stop an autonomous loop, snapshot its worktree, and resume interactively — without pretending the run was `pair` all along.

In every case the **work surface** is stable, the **sandbox/workspace** can persist, but **who is supervising the run right now** changes. Treating "human in the loop?" as a profile flip breaks the very invariants profiles exist to protect.

ADR-0037 already introduced a coarse `run_mode ∈ { interactive, autonomous }` on `RunAuthContext`. That is the precursor to this decision; it needs to generalize into an explicit, durable runtime dial that is **orthogonal** to the trust contract.

## Decision

Split the two jobs into two named, orthogonal concepts.

### 1. Trust profile (`Profile` in code)

A **trust profile** is the slow-changing, durable trust-and-memory contract for a class of work. It owns:

- memory read/write scopes (which branches: `pair/`, `work/`, `core/`, …);
- approval / autonomy class defaults;
- cross-role visibility rules (lethal-trifecta split);
- the default compiled prompt slice and default tool roster.

Values are unchanged: `chat`, `pair`, `curate`, `work`, `watch`. This is exactly what [ADR-0036](adr-0036-bear-profile-registry.md) calls a **profile**; this ADR renames the *concept* to **trust profile** in documentation and keeps the **code shorthand `Profile`** (`BearProfile`, `bear_profile_bindings.profile`, `profile_slug`, …). No schema or enum change is required by this ADR.

A trust profile is **not** the identity of a long-running sandbox or run. It is applied **per turn** as a template.

### 2. Governance mode (`Mode` in code)

A **governance mode** is the fast-changing supervision contract on a **run / workspace session**: *how is this execution being supervised right now?* It is run-scoped and mutable over the life of one run.

Initial modes:

| Mode | Human | Intent |
|------|-------|--------|
| `interactive` | Present (ACP/web) | Live collaboration; approvals via client/channel |
| `grace` | Recently disconnected | Finish in-flight turn; no new client-only tools; await return |
| `autonomous_continuation` | Absent, grace expired | Continue under executor-leaning effective policy; durable handoffs on block |
| `observational` | Present, read-only | Inspect/interrogate a run without owning its turns |
| `frozen` | Panic / handoff | Turn cancelled; worktree checkpointed; awaiting disposition |

Governance mode is documented as **governance mode**; the **code shorthand is `Mode`**. It generalizes ADR-0037 `run_mode`: `interactive`/`autonomous` become projections of governance mode onto `RunAuthContext` (e.g. `autonomous_continuation` ⇒ `run_mode = autonomous`).

### 3. Effective policy is a product, not a label

The runtime computes, per turn:

```text
EffectivePolicy = TrustProfile × GovernanceMode × Armature × RunAuthContext
```

- **TrustProfile** — durable boundaries and defaults (this ADR / ADR-0036).
- **GovernanceMode** — supervision dial (this ADR).
- **Armature** — where actuators live: ACP client tools, Den sandbox, or none (`interactive-profiles-and-role-axes.md`).
- **RunAuthContext** — git/auth/operation actor selection ([ADR-0037](adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md)).

The model never *infers* this cross product. Den enforces the tool roster, memory write target, and approval class for the effective policy, and exposes the components through `session_info`.

### 4. Lifetimes are separated

| Object | Lifetime | Owner |
|--------|----------|-------|
| **Work surface** | Bear-durable | Bear ([ADR-0006](adr-0006-bear-work-surfaces.md)) |
| **Workspace session** | Run-scoped materialization (one clone + egress gateway pair) | Docket run ([ADR-0037](adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md)) |
| **Governance mode** | Per-run, mutable timeline | Docket run |
| **Trust profile** | Applied per turn as a template | `bear_profile_bindings` ([ADR-0036](adr-0036-bear-profile-registry.md)) |

The **same** workspace session can span multiple governance modes. A trust profile change is a change of template for *new* work, not a re-pointing of the sandbox.

### 5. Invariants that must not bend

Governance mode changes supervision; it must never be used to launder trust:

- `work`-class effective policy **must not read raw `pair/` or `chat/`**, even under `observational` inspection by a human.
- Outbound git/auth always follows `RunAuthContext`, never "whatever mode feels convenient".
- Approvals are authoritative in **Den**, not "maybe the disconnected client will answer" ([ADR-0026](adr-0026-work-handoff-and-human-escalation.md)).
- `curate` and `watch` remain distinct trust profiles; they are **not** governance modes of an interactive session.

### 6. Transitions are durable and model-visible

Each transition emits a durable, model-visible event and updates run state:

```text
governance.changed {
  from, to,
  reason,                       // client_disconnected | grace_timeout | panic | explicit_handoff | human_returned | inspect_open | inspect_release
  run_id, work_surface_ref, workspace_session_id,
  trust_profile,                // effective profile for subsequent turns
  branch, checkpoint_ref,       // when a snapshot was taken
  human_expectation             // user-facing one-liner
}
```

`session_info` exposes `trust_profile`, `governance_mode`, `armature`, `work_surface`, and `workspace_session_id`, plus `linked_runs` so an interactive session re-entry can reference background runs by id, branch, and PR rather than implying local edits.

### 7. Timing and intervention (policy hooks, not hard-coded)

- **Grace window** — heartbeat loss enters `grace`; a bounded timeout transitions to `autonomous_continuation`. Durations are policy, not constants in this ADR.
- **Turn-boundary steering** — `observational` mode may read and ask at any time; steering (new instruction/task) applies at the next turn boundary, except panic.
- **Panic** — cancels the active turn (`CancellationToken`), commit-and-pushes the worktree to a checkpoint branch, sets `frozen`, and optionally materializes a new workspace session for interactive continuation from that branch (same work surface, new manifestation).

## Naming

| Documentation term | Code shorthand | Notes |
|--------------------|----------------|-------|
| **Trust profile** | `Profile` | `chat`/`pair`/`curate`/`work`/`watch`; existing `BearProfile`, `profile`, `profile_slug` symbols unchanged |
| **Governance mode** | `Mode` | `interactive`/`grace`/`autonomous_continuation`/`observational`/`frozen` |
| Profile (bare) | — | Legacy alias for **trust profile**; avoid in new runtime prose |
| `run_mode` (ADR-0037) | — | Coarse `interactive`/`autonomous`; derived from governance mode |

Do not reuse `Mode` for human-identity trust, channel selection, or work-surface resolution state.

## Consequences

- **`pair` ↔ `work` flipping largely disappears** as a runtime mechanism. Offline continuation, interrogation, and panic/resume are governance-mode transitions on a stable run + workspace session.
- **Profiles keep their meaning** as durable trust contracts and prompt/tool defaults, and as product language (`bear-roles.md`). They stop being sandbox-lifetime identifiers.
- **Schema impact is additive.** Governance mode is a new run-scoped field plus a transition log; trust profile vocabulary and `bear_profile_bindings` are unchanged. ADR-0037 `run_mode` becomes a derived projection.
- **`curate` and `watch` are unaffected**; they are not interactive runs and do not carry governance modes beyond a fixed autonomous supervision.
- **UX can stay continuous** ("your session is still running") while ops truth stays honest (`governance_mode = autonomous_continuation`, executor-leaning effective policy on the same `workspace_session_id`).
- **Follow-up (not decided here):** the concrete `WorkspaceSession` + governance-timeline schema, transition guards, and whether `chat`/`pair` eventually collapse into one `interactive` trust profile distinguished only by armature.
