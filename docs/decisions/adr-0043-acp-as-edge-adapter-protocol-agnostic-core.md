# ADR: ACP Is an Edge Adapter; the Den Runtime Is Protocol-Agnostic

**Status:** Proposed
**Date:** 2026-06-15
**Deciders:** Hans

**Related:**
- [ADR-0035](adr-0035-den-native-in-process-agent-runtime.md) — the single in-process agent loop and strategy policy this ADR protects
- [ADR-0029](adr-0029-den-structured-runtime-events.md) — Den structured runtime (semantic) events
- [ADR-0030](adr-0030-bearwire-resource-oriented-event-model.md) — BearWire resource-oriented event model (the canonical event seam)
- [ADR-0003](adr-0003-acp-session-bindings.md) — ACP session bindings (genuinely edge-only)
- [Den-Native Runtime architecture](../architecture/den-native-runtime.md)
- [ACP Runtime Contract](../architecture/acp-runtime-contract.md)
- [Den crate split plan](../roadmap/DEN_CRATE_SPLIT_PLAN.md)

## Context

ACP (Agent Client Protocol) is a wire protocol for editor/agent clients (Zed, Cursor, and the like). It should be one **edge** among several — a thin adapter that translates ACP requests into core operations and projects the core's canonical event stream back out as ACP wire events. The Den server's core — bears, memory, the agent loop, turns, sessions, tools — should be unaware that ACP exists.

We have already *decided* this, more than once:

- **ADR-0029** says ACP "acts too much like a backend protocol interpreter instead of a client/channel adapter," and that Den should "define its own meaningful internal event vocabulary instead of preserving backend wire-format structure through the stack."
- **ADR-0030** (BearWire) establishes a canonical, resource-oriented semantic event model and a durable distinction between **Den server semantics** and **trusted armatures / edge runtimes**.
- **ADR-0035** establishes a single, protocol-neutral, in-process agent loop for every role.

The **code drifted from the decision.** ACP was the first (and for a long time only) real client, and the native runtime was grown by mapping ACP/Letta stream events into Den. So the *canonical* model became the *ACP* model. The evidence (2026-06):

- `den-runtime` — the agent execution core, which must be protocol-agnostic — has **7 top-level `acp_*` modules** (`acp_tools`, `acp_plan_mode`, `acp_events`, `acp_tool_turns`, `acp_turn_controller`, `acp_sessions`, `acp_turn_runner`) and **~399** `acp`-prefixed symbols. `den-core` has ~46. `den-memory`, `den-docket`, and `den-llm` have **0** — so the leak is concentrated exactly in the turn/loop layer, where it does the most damage.
- The shared application state is half-ACP and is **owned by the ACP edge crate**: `den_acp::service::ApiState` carries `acp_tool_turns` and `acp_turn_cancellations` fields. Because that state lives in `den-acp`, the generic REST/OAuth edge (`den-api`) must depend on the ACP edge to obtain it — a dependency inversion.
- Geological naming leaks remain inside the "neutral" runtime: e.g. `acp_events::map_native_letta_stream_event_to_acp_event` carries both *Letta* and *ACP* names in a core module.

The recently completed `den-acp` / `den-api` crate de-aliasing (removing `extern crate self as api` and the re-export shims) made the layering *visible*, but it did not fix the layering: the arrows and the vocabulary still say "ACP is the center."

## Decision

Treat ACP as an **edge adapter** and make the Den runtime **protocol-agnostic in the code**, not just in the prose. Concretely:

### 1. The core owns turns, sessions, tool-turns, and events under neutral names

The agent loop and its machinery — turn lifecycle, tool-turn coordination, cancellation, session/conversation identity, the semantic event stream — are **core concepts** (ADR-0035). They live in `den-runtime` / `den-core` and **must not carry an `acp` prefix or ACP-shaped types**. The canonical event vocabulary is the BearWire semantic-event model (ADR-0029/0030), not `AcpGatewayEvent`.

Target end state: **zero `acp_*` modules or `Acp*` public types in `den-runtime` and `den-core`.**

### 2. ACP is a single adapter crate — the only place that knows ACP

All ACP-specific concerns live in one adapter (`den-acp`, reframed as *the ACP adapter*, not *the ACP runtime*): JSON-RPC/HTTP framing, the ACP session handshake/resume and bindings ([ADR-0003](adr-0003-acp-session-bindings.md)), SSE assembly/mapping, plan entries, client tool advertisement, permission prompts, ACP auth tokens, and the **projection** between canonical semantic events (BearWire) and ACP wire events. The adapter depends on the core; the core never depends on the adapter.

### 3. Shared application state is neutral and composition-owned

`ApiState` is renamed to a protocol-neutral state type (e.g. `DenState` / `AppState`) holding only protocol-agnostic dependencies (db pool, config, Bifrost client, runtime handles, memory store). Per-protocol concerns (e.g. ACP turn/cancel registries) are **not** fields on the shared state; they are owned by their adapter. The state type lives in a composition/state layer below all edges, not in any edge crate.

### 4. Edges are peers

ACP, the JSON/REST + OAuth API, and the web UI are **sibling adapters**. Each depends on the core; **none depends on another.** `den-api` must not depend on `den-acp` to obtain shared state or core types. The composition root (the binary) wires the core and mounts each adapter.

### 5. Classification rule

Every `acp_*` / `Acp*` symbol is exactly one of:

- **Core concept wearing ACP clothes** → rename to neutral vocabulary, keep in `den-runtime`/`den-core`.
- **Genuine wire concept** → move to the ACP adapter.

Illustrative (to be finalized by the audit in implementation step 1):

| Current | Classification | Target |
|---|---|---|
| `acp_turn_controller` (`AcpTurnPhase`, `AcpTerminalStatus`, `AcpToolExecutionRoute`) | core turn lifecycle | runtime `turn_controller` (neutral) |
| `acp_tool_turns` (tool-turn coordinator, continuation) | core control flow | runtime `tool_turns` |
| `acp_sessions` (`AcpSessionRow`, `upsert_session`, mode) | mostly core conversation/session; ACP mode + resume are wire | core `sessions` + adapter binding |
| `acp_turn_runner` | core loop (ADR-0035) + ACP wiring | core loop; ACP wiring in adapter |
| `acp_events` (`AcpGatewayEvent`, `map_native_letta_stream_event_to_acp_event`) | **wire** projection | ACP adapter projection over BearWire events |
| `acp_plan_mode`, `acp_tools` (client tool advertisement, plan entries) | **wire** | ACP adapter |
| `ApiState.acp_tool_turns` / `.acp_turn_cancellations` | wire (adapter-owned) | adapter-owned registries |

### 6. Migration is staged, non-destructive, and contract-fenced

This is a renaming-and-relocation refactor, not a behavior change. It proceeds behind the existing parity harness — the golden ACP trace tests already assert the full pipeline (`OpenAI SSE → semantic events → BearWire projection → adapter SSE`, see [DEN_NATIVE_RUNTIME_PLAN](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md) Phase 4). Each step keeps those traces green. No `docker compose down`; compose changes need explicit approval.

## What must NOT be lost (fencing the agent-loop conclusions)

The machinery being renamed here **is** the ADR-0035 agent loop. The ACP surgery must preserve, not dilute, these conclusions:

- **One agent loop, in-process, for every role.** Roles differ only by capability profile (tool roster, memory scope, autonomy policy, sandbox). The turn controller / tool-turn coordinator / session machinery are that loop's organs — they are protocol-neutral and stay in the core. (The fact that they are currently named `acp_*` is the bug this ADR fixes, not evidence that they belong to ACP.)
- **One loop primitive; patterns as a thin strategy policy** (`plan?` / `reflect_on_fail?` / `critique?` / `fanout_n`), selected by the ADR-0033 model-tasks layer — **not** a forked-runtime or pluggable "agent-pattern" framework. ReAct is the substrate; Reflexion / Reflection / best-of-N are compositions realized via Docket + per-Bear SQLite + subagent fan-out. LATS tree search and LLM Compiler DAG engines remain **deferred**. (See [ADR-0035](adr-0035-den-native-in-process-agent-runtime.md) §strategy policy and [den-native-runtime.md#loop-strategies](../architecture/den-native-runtime.md#loop-strategies).)
- **The canonical seam is BearWire semantic events** (ADR-0029/0030). The renaming must route through that seam (core emits semantic events; the adapter projects them), so that adding the next edge (REST streaming, desktop companion, CI runner) is a new projection — never another fork of the loop.

Renames in the runtime must move *toward* this neutral vocabulary; they must not invent a parallel abstraction or re-shape the loop around a different protocol.

## Consequences

### Positive

- The dependency graph and the vocabulary finally match the decided architecture: core in the middle, protocols at the edges.
- Adding a new client protocol becomes "write an adapter + a projection," with no change to the loop.
- The agent loop, turn model, and strategy policy are stated once, protocol-neutrally, instead of being entangled with one wire format.
- Removes the `den-api → den-acp` inversion at its root (shared state is neutral and composition-owned).

### Negative / tradeoffs

- A large mechanical rename (~400 symbols) plus a real boundary to draw between core and wire; some judgment calls (sessions, plan mode, tool advertisement).
- Churn in `den-runtime`/`den-core` public surfaces and in the ACP adapter; the golden-trace harness must be trusted and maintained throughout.
- Temporary period where neutral and `acp_*` names coexist behind re-export shims during the staged migration (to be removed at the end, per the crate-split discipline).

## Non-goals

- Not a behavior change to ACP or to the agent loop; ACP clients see identical traces.
- Not a new event model — BearWire (ADR-0030) is the canonical seam; we are not inventing another.
- Not an "agent-pattern" framework (ADR-0035 non-goal stands).
- Not a second runtime or a pluggable multi-runtime abstraction (ADR-0035 non-goal stands).
- Building the additional edges (REST streaming, desktop, CI) is out of scope here; this ADR only makes them *possible* by fixing the boundary.

## Implementation (first step)

Before any renames, produce the **classification audit**: enumerate every `acp_*` / `Acp*` symbol in `den-runtime` and `den-core`, bucket each as *core (rename, keep)* or *wire (move to adapter)*, and record the target name/home. That audit sizes the work and becomes the checklist; the staged rename-and-relocate then follows it behind the golden-trace tests.

> **Audit complete (2026-06-15):** [ADR-0043 Classification Audit](../roadmap/ADR_0043_PROTOCOL_AGNOSTIC_CORE_AUDIT.md) buckets all seven `den-runtime` `acp_*` modules plus the `den-core` touchpoints, with target names/homes and a recommended staging sequence. Two notes refine this ADR: (1) the illustrative table above tentatively put `acp_plan_mode` under *wire*, but the audit keeps plan-mode as a **core capability** (its tool surface already lives in `den-core`); (2) the audit confirms a neutral event seam (`RuntimeSemanticEvent`/`bearwire_projection`) already exists, so `acp_events` is a clean wire move. The audit also flags that the `bearwire_projection/golden_traces_tests.rs` safety net this ADR assumes **does not exist yet** and must be added before the first rename.
