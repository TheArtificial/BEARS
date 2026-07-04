# Model Experience

This document is an index of the docs and repo rules that most directly shape what the model experiences inside Den and BearWire.

It is not itself the canonical policy. It points to the canonical docs that define transcript shape, tool surface, memory visibility, runtime limits, and failure handling.

## Core Runtime

- `docs/decisions/adr-0035-den-native-in-process-agent-runtime.md`
  - One in-process Den runtime for all stances.
  - Turn lifecycle, tool execution loop, and strategy policy.

- `docs/decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md`
  - ACP is an edge adapter, not the core runtime.
  - The model should not learn ACP-specific fake capabilities as if they were core semantics.

- `docs/decisions/adr-0048-core-turn-client-obligation-coordinator.md`
  - Continuation after approvals and tool results is core runtime behavior.
  - Open obligations block legal continuation.

## Conversation And Transcript

- `AGENTS.md`
  - See: `Conversation History and Transcript Projection`
  - Canonical conversation storage is the source of truth.
  - Model replay and user-visible history are separate projections.

- `services/den/AGENTS.md`
  - See: `Conversation history`
  - BearWire/history fixes must preserve both persistence and next-turn replay.

- `docs/decisions/adr-0050-adaptive-turn-budgets-and-loop-ko.md`
  - Model-visible low-budget warnings.
  - Hidden operational outcome records for future transcript replay.

## Budgets And Loop Health

- `docs/decisions/adr-0050-adaptive-turn-budgets-and-loop-ko.md`
  - Budget-ledger-first runtime policy.
  - Wall-clock, tool-class, failure, and ko budgets.
  - Emergency hard-step fuse is secondary.

- `docs/decisions/adr-0047-context-window-budget-and-token-estimation.md`
  - Context window budget is tracked against the fully assembled request.
  - Explains how Den reasons about token pressure before inference.

## Tool Surface

- `AGENTS.md`
  - See: `Tool Naming` and `BearWire, ACP, and Tool Routing`
  - Provider names are concise action names.
  - Execution location is descriptor-owned, not inferred from name strings.

- `docs/decisions/adr-0025-tool-naming-and-execution-strategy.md`
  - Tool naming and execution strategy.

- `docs/decisions/adr-0017-provider-safe-tool-naming.md`
  - Tool names exposed to models should be stable and safe.

## Memory Visibility

- `AGENTS.md`
  - See: `Memory and Reflection`
  - `pair` writes pair-local memory directly.
  - Cross-role sharing flows through reflection/curation, not raw transcript leakage.

- `docs/decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md`
  - Canonical memory lives in SQLite-backed stores.

- `docs/decisions/adr-0041-archival-recall-and-async-curation.md`
  - Reflection and recall are separate from the foreground interactive loop.

## Stances And Governance

- `docs/decisions/adr-0039-trust-profiles-and-governance-modes.md`
  - Stances bundle tool/memory/approval defaults.
  - Governance mode shapes how much autonomy the model is expected to exercise.

- `docs/decisions/adr-0037-work-sandbox-egress-gateway-and-upstream-auth.md`
  - `work` is expected to support longer, more tool-heavy execution.
  - The model does not directly choose upstream credentials.

## Typed Boundaries And String Hygiene

- `AGENTS.md`
  - See: `Typed Boundaries and String Hygiene`
  - Do not smuggle control data through transcript text if typed protocol fields exist.
  - Parse once at boundaries and preserve typed concepts.

## Why This Index Exists

The model's effective experience is defined by multiple documents at once:

- what gets replayed into transcript history,
- which tools appear and where they run,
- what counts as a legal continuation,
- what memory is visible,
- and what warnings/failures are surfaced in-band.

This file is intended to stay small and point to those source documents, so future changes to model experience have one obvious index entry point.
