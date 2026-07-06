# Model Experience

How Bear Den manages what the model experiences: what it sees each turn, which tools appear and where they run, what memory is visible, what counts as a legal continuation, and what warnings arrive in-band. This is a consolidated view; the canonical sources are indexed at the end and win on any disagreement.

## The governing idea

Den does not hand the model "everything," and it does not let the model guess its own situation. Every turn, Den deliberately compiles a bounded **turn context** from explicit, separately-owned layers — and wherever Den already knows a runtime condition (permissions, budgets, active work), Den renders the single applicable instruction before inference instead of asking the model to choose. The model's experience is a managed product of the system, assembled the same way regardless of which surface (ACP armature, web chat, background dispatch) the turn came from.

## What the model sees: the context layers

Each turn is assembled in this order ([docs/architecture/den-runtime.md § turn context assembly](docs/architecture/den-runtime.md#turn-context-assembly), [docs/architecture/context-compilation-scenarios.md](docs/architecture/context-compilation-scenarios.md)):

```text
system:  compiled stance prompt            (pre-turn compiled, Postgres)
       + key memory projection             (bounded anchors from per-Bear SQLite)
       + derived recall                    (optional vector passages, labeled as recall)
       + runtime supplements               (prompt-memory blocks, compaction state,
                                            channel/tool reminders)
messages: projected transcript incl. replayable tool calls/results + current step
tools:    merged Den-hosted + client descriptors
```

| Layer | What it is | Who owns it |
|-------|------------|-------------|
| **Compiled stance prompt** | Identity, stance contract, operator steering. Compiled ahead of the turn from repository-authored prompt fragments plus runtime-authored (e.g. admin-edited) content into `bear_compiled_configs`. The turn hot path never parses prompt files or renders DB templates. | Prompt compiler ([ADR-0046](docs/decisions/adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md), [prompt fragment registry](docs/architecture/prompt-fragment-registry.md)) |
| **Key memory projection** | A small, stance-scoped briefing of canonical memory: shared identity anchors, active work-surface anchors, stance-local highlights. Latest-head records only, under strict character budgets — never the whole memory store. | Context assembler over per-Bear SQLite |
| **Derived recall** | Semantically retrieved passages when the vector index is configured. Explicitly labeled as recall — a search result over memory, not memory itself. | Derived recall index ([ADR-0038](docs/decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)) |
| **Prompt-memory blocks** | Editable standing context scoped by Bear/stance/session/work surface. Distinct from long-term memory and from transcript. | Den Postgres ([contract](docs/architecture/den-prompt-memory-block-contract.md)) |
| **Runtime supplements** | Turn-local reminders rendered from typed runtime state: tool/memory scope for a trusted session, active Docket execution, date/budget reminders, compaction state. | Repository-owned runtime fragments |
| **Transcript** | The model-replay projection of canonical conversation storage — a distinct projection from user-visible history. | Den conversation persistence |
| **Tool descriptors** | The model-facing tool surface and schemas for this stance and surface. | Descriptor registry + client capabilities |

## What the model deliberately does not see

The exclusions are as designed as the inclusions:

- **Other stances' raw memory branches.** Projection is stance-scoped; `work` never sees raw `pair/` or `chat/` (see [SAFETY.md](SAFETY.md)).
- **Unreviewed proposals and pending observations** — reachable only through tools or curation review, never proactively projected.
- **Docket/task state as "memory."** Work management is control-plane infrastructure, surfaced through task tools and runtime fragments, not memory projection.
- **Superseded record history.** Proactive projection shows latest heads; history stays tool-mediated.
- **The whole transcript, forever.** Long conversations are compacted: older context becomes derived summary state, kept explicitly separate from raw transcript and from memory, with active goals, constraints, and tool/approval state preserved.
- **Raw provider/protocol machinery.** ACP is an edge adapter; the model should never learn ACP-specific fake capabilities as core semantics ([ADR-0043](docs/decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)).

## The tool surface

- **Names are stable and provider-safe.** Model-facing names are concise action names (`memory_search`, `web_fetch`, `fs_edit_file`); canonical internal names are scoped and dotted (`den.memory.search`). Legacy aliases are accepted at boundaries but never advertised ([ADR-0017](docs/decisions/adr-0017-provider-safe-tool-naming.md), [ADR-0025](docs/decisions/adr-0025-tool-naming-and-execution-strategy.md)).
- **Execution location is descriptor-owned.** Whether a tool runs inside Den, through the armature, or in a sandbox is metadata, never inferred from the name.
- **The surface is stable across turns.** Tools are not hidden or revealed per-turn by prompt heuristics — that teaches the model false capabilities. Availability changes only with the stance, surface, or explicit policy.
- **Tool exchanges are replayable transcript state**: stable call id, canonical name, typed arguments, matching result or error, bounded output. What the model saw a tool do is always reconstructable next turn.

## Memory visibility

The model touches memory in exactly three ways, each bounded:

1. **Projection** — the proactive briefing described above.
2. **Recall** — optional derived passages, labeled as recall.
3. **Tools** — `memory_browse` / `memory_read` / `memory_search` for on-demand retrieval, and stance-local writes (e.g. `pair` writes `pair/` entries; promotion to shared memory goes through review, never directly).

Trusted facts come from typed context, not chat text: human identity for an ACP `pair` session comes from the session token via `session_info`, and conflicting claims in the conversation do not override it.

## Continuation, obligations, and budgets

- **Continuation is core runtime behavior.** After a tool result or an approval decision, the core turn coordinator — not an edge or the model — decides when the turn may legally continue; open client obligations block continuation ([ADR-0048](docs/decisions/adr-0048-core-turn-client-obligation-coordinator.md)).
- **Approvals pause, then resume, the same turn.** A denied action returns as a result the model can react to, not a broken session.
- **Agent loop control is ledger-first.** Wall-clock, tool-class, failure, ko, checkpoint, and control-level signals govern loop health; the model receives explicit low-budget warnings and concise runtime checkpoint nudges in-band, while hidden operational outcome records preserve the truth for future replay ([ADR-0050](docs/decisions/adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md)). Some exploration budgets such as `read`/`search` may be replenished after a successful meaningful mutative step so interactive turns can perform bounded post-edit verification, but global fuses such as wall-clock, total tool calls, repeated failure, and emergency hard-step limits remain turn-global. Context-window pressure is tracked against the fully assembled request before inference ([ADR-0047](docs/decisions/adr-0047-context-window-budget-and-token-estimation.md)); on provider overflow, Den compacts and retries once, preserving in-flight tool results and unresolved approvals.
- **Work has stopping conditions.** Docket tasks carry concrete `completion_criteria`, so the model has a defined "done" rather than an open-ended loop, and completion requires a factual `result_summary` ([ADR-0034](docs/decisions/adr-0034-jobs-and-tasks-work-management.md)). How hard the loop drives vs. yields is owned by the run's governance mode, referenced against those criteria ([ADR-0039](docs/decisions/adr-0039-trust-profiles-and-governance-modes.md)).
- **Session task lists are working projections**, session/stance-local; durable cross-stance work belongs in Docket ([ADR-0045](docs/decisions/adr-0045-session-task-lists-and-docket-checkout.md)).

## Hygiene rules that protect the experience

From `AGENTS.md` (binding for contributors):

- **No control data smuggled through transcript text.** If a typed protocol field or event exists, use it; sentinel strings and embedded XML/JSON blocks in prose are forbidden.
- **Parse once at boundaries**; typed concepts stay typed all the way through.
- **Model replay and user-visible history are different projections** of one canonical store. History fixes must verify both persistence and next-turn request construction.
- **Prompt prose lives in fragments or managed data, not Rust literals**, so what the model reads is versioned, compiled, and auditable.

## Canonical source index

Core runtime: [ADR-0035](docs/decisions/adr-0035-den-native-in-process-agent-runtime.md) · [ADR-0043](docs/decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md) · [ADR-0048](docs/decisions/adr-0048-core-turn-client-obligation-coordinator.md)

Context assembly: [den-runtime.md](docs/architecture/den-runtime.md) · [context-compilation-scenarios.md](docs/architecture/context-compilation-scenarios.md) · [prompt-fragment-registry.md](docs/architecture/prompt-fragment-registry.md) · [ADR-0046](docs/decisions/adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md) · [prompt-memory contract](docs/architecture/den-prompt-memory-block-contract.md)

Transcript: `AGENTS.md` ("Conversation History and Transcript Projection") · `services/den/AGENTS.md` ("Conversation history")

Agent loop control and budgets: [ADR-0050](docs/decisions/adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md) · [ADR-0047](docs/decisions/adr-0047-context-window-budget-and-token-estimation.md)

Work management: [ADR-0034](docs/decisions/adr-0034-jobs-and-tasks-work-management.md) · [ADR-0045](docs/decisions/adr-0045-session-task-lists-and-docket-checkout.md) · [DOCKET_IMPLEMENTATION_PLAN.md](docs/roadmap/DOCKET_IMPLEMENTATION_PLAN.md)

Tool surface: `AGENTS.md` ("Tool Naming", "BearWire, ACP, and Tool Routing") · [ADR-0025](docs/decisions/adr-0025-tool-naming-and-execution-strategy.md) · [ADR-0017](docs/decisions/adr-0017-provider-safe-tool-naming.md)

Memory visibility: `AGENTS.md` ("Memory and Reflection") · [ADR-0031](docs/decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md) · [ADR-0041](docs/decisions/adr-0041-archival-recall-and-async-curation.md)

Stances and governance: [ADR-0039](docs/decisions/adr-0039-trust-profiles-and-governance-modes.md) · [ADR-0037](docs/decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md) · [SAFETY.md](SAFETY.md)

Typed boundaries: `AGENTS.md` ("Typed Boundaries and String Hygiene")
