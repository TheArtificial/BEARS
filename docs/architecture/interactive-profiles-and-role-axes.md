# Interactive profiles and role axes

**Status:** Draft (2026-06)  
**Related:** [`bear-roles.md`](bear-roles.md), [ADR-0006 — Bear work surfaces](../decisions/adr-0006-bear-work-surfaces.md)

This document separates three concepts that were historically bundled into **roles**, especially for human-present modes (`chat` and `pair`).

## Three axes

| Axis | Question it answers | Owned by | Examples |
|------|---------------------|----------|----------|
| **Resource** | What durable work context is this thread about? | **Bear** (work surface + anchors) | monorepo, Cabinet Mission, service deploy |
| **Armature** | What actuators and signals exist on this turn? | **Channel / session config** | ACP `client_tools`, workspace roots, web fetch only |
| **Trust** | Who approves effects and which memory is writable? | **Role capability profile** | pair-local writes, client approval, no raw `work/` reads |

**Work surfaces are Bear-level**, not role-owned ([ADR-0006](../decisions/adr-0006-bear-work-surfaces.md)). A Slack thread and an IDE session can refer to the same surface with different armatures.

## Role profiles (post-Letta)

Roles are **capability profiles** over one Den-native agent loop — not separate Letta agents or harness processes.

| Profile | Human present? | Primary trust boundary | Typical armature |
|---------|----------------|------------------------|------------------|
| **`chat`** | Yes | Channel conversation; Den-mediated tools | Web/Slack adapter; work surface **reference** mode |
| **`pair`** | Yes | Client-mediated effects; user-gated mutation | ACP + workspace roots; work surface **active** mode |
| **`curate`** | No | Sole writer to shared `core/`; cross-branch read | Narrow Den tool roster; conductor-driven |
| **`work`** | No | Approved outbound action; curated memory only | Den sandbox (shell/fs); Docket task dispatch |
| **`watch`** | No | Inbound-only; observations not actions | Subscription/webhook loop |

### Interactive split: `chat` vs `pair`

Both profiles assume a **human is present**. The distinction is not “trust only”:

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
| Work surface mode (`active` vs `reference_only`) | Profile default | Can override when channel gains workspace |
| Approvals / autonomy | Yes | Client UI (ACP) |
| Docket dispatch, sandbox | `work` only | Task binding |

## Non-interactive profiles (keep distinct)

`curate`, `work`, and `watch` remain separate profiles because they encode **non-negotiable trust boundaries** (lethal trifecta split), not channel choice:

- **`work`** must not read raw `chat/` or `pair/`.
- **`watch`** must not gain outbound or shell affordances.
- **`curate`** is the only routine writer to shared `core/`.

## Implementation direction

1. Treat **`chat` and `pair` as interactive profiles**; channel choice selects armature template, not a second Bear.
2. Attach **work surfaces to threads/sessions**, resolved from anchors and hints — not “owned by pair.”
3. Retire harness/runtime-family language (`letta_code_harness`, five provisioned “agents”).
4. Prefer **capability profile** in code comments and operator docs; keep **role** as the stable enum name for schemas.

## Naming

The implementation enum and Postgres registry use **`chat`** (formerly `talk`) for the conversational human-present profile, aligned with product language in [`bear-roles.md`](bear-roles.md).
