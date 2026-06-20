# Interactive stances and role axes

**Status:** Draft (2026-06)  
**Related:** [`bear-roles.md`](bear-roles.md), [ADR-0006 — Bear work surfaces](../decisions/adr-0006-bear-work-surfaces.md), [ADR-0039 — Trust stances and governance modes](../decisions/adr-0039-trust-profiles-and-governance-modes.md)

This document separates concepts that were historically bundled into **roles**, especially for human-present modes (`chat` and `pair`).

## Four axes

| Axis | Question it answers | Owned by | Examples |
|------|---------------------|----------|----------|
| **Resource** | What durable work context is this thread about? | **Bear** (work surface + anchors) | monorepo, Cabinet Mission, service deploy |
| **Armature** | What actuators and signals exist on this turn? | **Channel / session config** | ACP `client_tools`, workspace roots, web fetch only |
| **Trust stance** | Which memory is writable and what approval class applies? | **Trust stance** (`BearStance` in code; `BearProfile` compatibility alias during migration) | pair-local writes, client approval, no raw `work/` reads |
| **Governance mode** | How is this run supervised right now? | **Run / workspace session** (`Mode` in code) | interactive, grace, autonomous_continuation, observational, frozen |

**Trust stance** is the slow-changing trust-and-memory contract; **governance mode** is the fast-changing supervision dial on a run. They are orthogonal — see [ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md). The runtime computes, per turn:

```text
EffectivePolicy = TrustStance × GovernanceMode × Armature × RunAuthContext
```

**Work surfaces are Bear-level**, not role-owned ([ADR-0006](../decisions/adr-0006-bear-work-surfaces.md)). A Slack thread and an IDE session can refer to the same surface with different armatures.

## Trust stances (post-Letta)

**Trust stances** are durable trust contracts over one Den-native agent loop — not separate Letta agents or harness processes. Documentation says **trust stance**; the current code uses `BearStance`; `BearProfile` and `bear_profile_bindings.profile` are compatibility names during migration.

| Trust stance | Human present? | Primary trust boundary | Typical armature |
|--------------|----------------|------------------------|------------------|
| **`chat`** | Yes | Channel conversation; Den-mediated tools | Web/Slack adapter; work surface **reference** mode |
| **`pair`** | Yes | Client-mediated effects; user-gated mutation | ACP + workspace roots; work surface **active** mode |
| **`curate`** | No | Sole writer to shared `core/`; cross-branch read | Narrow Den tool roster; conductor-driven |
| **`work`** | No | Approved outbound action; curated memory only | Den sandbox (shell/fs); Docket task dispatch |
| **`watch`** | No | Inbound-only; observations not actions | Subscription/webhook loop |

A trust stance is applied **per turn** as a template. It is **not** the identity of a long-running sandbox or run; that lifetime belongs to the workspace session and its governance-mode timeline ([ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md)).

### Interactive split: `chat` vs `pair`

Both stances assume a **human is present**. The distinction is not “trust only”:

- **`chat`** — conversational surface without co-located workspace armature. Can *reference* work surfaces and capture task intents; does not treat the thread as *operating on* a checkout/repo via client tools.
- **`pair`** — collaborative surface **with workspace armature** (client tools, active work-surface resolution, plan/write modes, pair-local learning). External effects flow through the client approval path.

Trust follows armature; resources sit above both.

## What stays role-gated vs session config

| Concern | Role-gated | Session / channel config |
|---------|------------|---------------------------|
| Memory read scopes (`chat/` vs `pair/` vs `core/`) | Yes | — |
| Memory write scopes (pair-local direct write) | Yes | — |
| Tool roster (ACP client tools vs web fetch) | Partially | Client tool list, channel adapter |
| Work surface identity | — | Resolution state on thread/session |
| Work surface mode (`active` vs `reference_only`) | Stance default | Can override when channel gains workspace |
| Approvals / autonomy | Yes | Client UI (ACP) |
| Docket dispatch, sandbox | `work` only | Task binding |

## Governance modes (the runtime dial)

A **governance mode** describes how a run / workspace session is supervised *right now*. It changes over the life of one run without changing the trust stance or re-pointing the sandbox ([ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md)). Documentation says **governance mode**; the code shorthand is `Mode`.

| Mode | Human | Intent |
|------|-------|--------|
| `interactive` | Present (ACP/web) | Live collaboration; approvals via client/channel |
| `grace` | Recently disconnected | Finish in-flight turn; no new client-only tools; await return |
| `autonomous_continuation` | Absent, grace expired | Continue under executor-leaning effective policy; durable handoffs on block |
| `observational` | Present, read-only | Inspect/interrogate a run without owning its turns |
| `frozen` | Panic / handoff | Turn cancelled; worktree checkpointed; awaiting disposition |

Governance mode generalizes the coarse `run_mode ∈ { interactive, autonomous }` from [ADR-0037](../decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md): `autonomous_continuation` projects onto `run_mode = autonomous`.

**Handoff seams** (remote `pair` going offline, interrogating a long `work` run, panic/checkpoint) are governance-mode transitions on a stable run + workspace session — not trust-stance flips.

## Non-interactive trust stances (keep distinct)

`curate`, `work`, and `watch` remain separate trust stances because they encode **non-negotiable trust boundaries** (lethal trifecta split), not channel choice or supervision state:

- **`work`** must not read raw `chat/` or `pair/` — even under `observational` inspection.
- **`watch`** must not gain outbound or shell affordances.
- **`curate`** is the only routine writer to shared `core/`.

They are trust stances, **not** governance modes of an interactive session.

## Implementation direction

1. Treat **`chat` and `pair` as interactive trust stances**; channel choice selects armature template, not a second Bear.
2. Attach **work surfaces to threads/sessions**, resolved from anchors and hints — not “owned by pair.”
3. Model **governance mode** on the run / workspace session; do not flip trust stance to express "human went offline."
4. Retire harness/runtime-family language (`letta_code_harness`, five provisioned “agents”).
5. Prefer **trust stance** (`BearStance` in code; `BearProfile`/`bear_profile_bindings` are compatibility names during migration) and **governance mode** (`Mode` in code) in operator docs.

When `AGENT_RUNTIME=native`, `chat` and `pair` are trust stances over one Den loop; work-surface **binding** is conversation-scoped. See [`../guides/work-surfaces-and-conversations.md`](../guides/work-surfaces-and-conversations.md).

## Naming

The implementation enum (`BearStance`, with `BearProfile` as a temporary alias) uses **`chat`** (formerly `talk`) for the conversational human-present trust stance, aligned with product language in [`bear-roles.md`](bear-roles.md). "Trust stance" is the current documentation term for the old stance/profile concept; "governance mode" remains the documentation term for the `Mode` concept ([ADR-0039](../decisions/adr-0039-trust-profiles-and-governance-modes.md)).
