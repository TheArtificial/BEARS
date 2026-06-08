# ADR: Den-Native In-Process Agent Runtime

**Status:** Accepted
**Date:** 2026-06-08
**Deciders:** Hans
**Supersedes:** [ADR-0014](adr-0014-multi-role-runtime-architecture.md) (harness/API-direct split), [ADR-0013](adr-0013-memfs-sidecar-repo-views.md) (MemFS as canonical memory), [ADR-0005](adr-0005-bear-memory-tool-boundary.md) (Letta Code as required runtime) on the points below.

**Related:**
- [ADR-0031](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md) — per-Bear SQLite memory
- [ADR-0033](adr-0033-model-tasks-layer.md) — model-tasks and strategy policy selection
- [ADR-0034](adr-0034-jobs-and-tasks-work-management.md) — Docket jobs/tasks in Den Postgres
- [Den-Native Runtime architecture](../architecture/den-native-runtime.md)
- [Migration plan](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md)

## Context

Bear Den was built on Letta as the agent execution process. That introduced a visible control-plane/execution-process split: Den orchestrated turns, but Letta owned the LLM loop, conversation materialization, run-ids, approval semantics, and (for harness-backed roles) Letta Code/Codepool as a second process boundary.

Finishing the prior Letta migration plan would have enshrined this split as a permanent trait seam (`RuntimeTurnBackend` with only a Letta implementation). The target end-state is simpler: **one in-process agent loop in Den for every role**, talking directly to Bifrost for inference, with Bear memory in per-Bear SQLite and tasks in Docket (Den Postgres).

## Decision

BEARS adopts a **Den-native, in-process agent runtime** as the sole execution substrate for all Bear roles.

### Runtime shape

1. **One agent loop, in-process, for every role.** The Letta-era "harness-backed vs API-direct" distinction is deleted. Roles differ only by **capability profile**: tool roster, memory scope, approval/autonomy policy, and optional code sandbox.
2. **One loop primitive** (assemble context → stream model → execute tools → persist), parameterized by a thin **strategy policy** (`plan?` / `reflect_on_fail?` / `critique?` / `fanout_n`), not by forked runtimes or a pluggable "agent-pattern" framework.
3. **A turn is a Tokio task** owned by Den. Cancellation is a `CancellationToken`, not external run-ids.
4. **Den owns** conversation identity, transcript, message/context state, approvals, and compaction. No conversation materialization to Letta, no run-ids, no approval-deny recovery against a remote process, no synthetic `TurnCompleted` to paper over provider gaps.
5. **Bifrost** (`LLM_API_URL`) is the inference substrate (OpenAI-compatible `chat/completions` with tool-calling), called directly by Den.

### Storage boundary

- **Per-Bear SQLite** ([ADR-0031](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)): canonical Bear cognition — memory records, references, proposals, observations, promotion/curate decisions, reflection-run **outcome** records.
- **Den Postgres**: control plane — conversations/transcript, approvals, role-runtime registry, Docket jobs/tasks ([ADR-0034](adr-0034-jobs-and-tasks-work-management.md)), reflection **scheduler/queue**.
- **Reflection-run split:** scheduler/queue in Postgres; canonical run record + outcomes in SQLite; queue row references SQLite run id only.
- **Git** retains human-authored artifacts only (skills, prompts, policies, schemas). The git MemFS sidecar is **not** canonical for live machine-written memory.

### Strategy policy (not a pattern zoo)

Reasoning patterns are composable knobs over the single loop, selected by the [ADR-0033](adr-0033-model-tasks-layer.md) policy layer:

| Knob | Realization |
|------|-------------|
| `plan?` | Docket task tree + pair plan-mode |
| `reflect_on_fail?` | Docket `command` criteria + SQLite reflection note + re-dispatch |
| `critique?` | Optional post-step revise pass |
| `fanout_n` | Docket child runs / subagent turns |

LATS tree search and LLM Compiler DAG engines are **deferred**.

### Transitional compatibility

During migration, `AGENT_RUNTIME=letta|native` (default `letta` until parity proven) selects the turn backend. Letta, Codepool, and MemFS are removed only after Phase 8 teardown (compose changes require explicit approval).

## Consequences

### Positive

- Deletes the vestigial Den/Letta process boundary and the speculative multi-runtime abstraction.
- One concurrency model (turn = task, cancel = token) across all roles.
- Memory and cognition in SQLite align with ADR-0031; tasks in Docket align with ADR-0034.
- Bifrost is used for inference, not only metadata.

### Negative / tradeoffs

- Den must reproduce context/compaction and tool-calling fidelity Letta provided implicitly.
- Per-Bear SQLite introduces schema, migration, and logical-path projection work.
- Phase 7 (coding harness / sandbox) remains the largest sub-project.
- One-time backfill from Letta history and MemFS content.

## Non-goals

- No new general-purpose vector store; semantic recall remains derived over SQLite sources.
- No pluggable multi-runtime framework retained for optionality.
- No bear-local task store.
- No "agent-pattern" framework.
