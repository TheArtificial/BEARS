# Den Native Runtime — Implementation Plan

**Status:** Active. This is the canonical migration plan for removing Letta and running a single Den-native, in-process agent runtime.

**Architecture source of truth:** [`../architecture/den-native-runtime.md`](../architecture/den-native-runtime.md).

This plan supersedes the Letta-backed runtime direction in older roadmap docs (Phase 1 Letta/Codepool stack, ACP-as-Letta-conversation, MemFS-canonical memory, harness-backed vs API-direct split). See [the architecture doc](../architecture/den-native-runtime.md#what-this-supersedes) for the full supersession list.

## Goal

Replace Letta entirely with a single Den-native, in-process agent runtime: a streaming tool-calling loop that talks directly to Bifrost, stores all Bear memory/cognition in per-Bear SQLite ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), retiring the git MemFS sidecar), reuses Den's existing orchestration/persistence, unifies all roles under capability profiles, and deletes the control-plane/execution-process split (Letta server, Letta Code SDK, Codepool) along with its concurrency artifacts.

## Why the previous plan was wrong

The previous direction converged on a trait seam (`RuntimeTurnBackend` / `RuntimeCancellationBackend` / `RuntimeConversationBackend`) whose only implementation was Letta — a faithful re-model of Letta's HTTP process boundary. Finishing it would enshrine "Den control plane + Letta execution process," exactly the vestigial split we want gone. The replacement is smaller than it looks because Den already owns most of a runtime (canonical transcript, tool execution, tool-turn coordinator, in-process cancellation registry, turn phase machine, SSE orchestration, prompt assembly). Letta's remaining job is narrow: call the LLM, drive the tool-step loop, hold run/conversation state, and emit approval/stop semantics — all of which Den can own.

## Storage boundary (cognition vs control plane)

- **Per-Bear SQLite** (cognition, canonical): memory records/links, references, memory proposals, watch observations, promotion/curate decisions + audit, and reflection-run **outcome** records.
- **Den Postgres** (control plane): conversations/transcript, approvals, role-runtime registry, Docket jobs/tasks ([ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md)), and the reflection **scheduler/queue**.
- **Reflection-run split:** the scheduler/queue stays in Postgres; the canonical run record + outcomes live in SQLite next to `memory_promotions`. The queue references the SQLite run id; the only cross-store link is a transient id pointer.

See the [storage boundary section](../architecture/den-native-runtime.md#storage-boundary-bear-cognition-vs-den-control-plane) for detail.

## Loop strategies (composable policies, not a pattern zoo)

One step primitive; reasoning patterns are a thin, data-driven strategy policy (`plan?`/`reflect_on_fail?`/`critique?`/`fanout_n`) selected by the [ADR-0033](../decisions/adr-0033-model-tasks-layer.md) model-tasks policy layer. Plan&Solve is realized by Docket, Reflexion by Docket acceptance criteria + SQLite reflection, fan-out by Docket child runs / subagents. LATS tree search and LLM Compiler DAG engines are deferred. See [Loop strategies](../architecture/den-native-runtime.md#loop-strategies).

## Phases

### Phase 1 — Native Bifrost inference client

- New `core/llm/` (OpenAI-compatible): streaming `chat/completions` with tool-calling, targeting `LLM_API_URL` (default `http://bears-bifrost:8080/v1`).
- Add `llm_api_url` + default model handling to `config.rs` (currently `LLM_API_URL` is read only for `/status` warnings).
- Stream parser emits the existing semantic-event contract directly (drop the Letta JSON intermediate).
- Tests with recorded Bifrost SSE (text deltas, tool-call deltas, finish reasons).

### Phase 2 — Per-Bear SQLite canonical memory (ADR-0031)

Foundational; parallelizable with Phase 1. Replaces the git MemFS sidecar so the loop in Phase 3 is built on SQLite, not MemFS.

- New `core/memory/store/` backed by `sqlx` SQLite. Schema per ADR-0031: `memory_records`, `memory_links`, `memory_promotions`, plus a Bear-wide monotonic sequence allocator.
- Operational defaults: WAL, `synchronous=NORMAL`, `busy_timeout=5000`; single logical write path (`SqlitePool`, `max_connections(1)`); define per-Bear DB file lifecycle/placement.
- **Logical-path projection**: map logical paths to (`scope_type`, `scope_role`, `work_surface_ref`, `kind`) so `memory_browse`/`memory_read` keep the stable-anchor UX over rows.
- Migrate Den-hosted memory tools off MemFS to the SQLite store (`memory_write`, `memory_read`, `memory_review`, `memfs` retire/replace), routed through the memory manager to SQLite instead of the `/v1/git` MemFS API.
- Move all Bear cognition into per-Bear SQLite: `bear_memory_proposals`, `bear_observations`, and curate/promotion decisions + audit migrate from Den Postgres; `core/` promotion writes `memory_records`/`memory_promotions`.
- **Reflection-run boundary (split):** scheduler/queue stays in Den Postgres; the run record + outcomes move to SQLite next to `memory_promotions`; the queue references the SQLite run id.
- Transition: Codepool/Letta-backed `talk`/`work` keep their existing memory until Phase 7; MemFS is not deleted until Phase 8. Commit the `.sqlx` offline cache so CI compile-time query checks pass.

### Phase 3 — Native agent loop (ReAct stepper)

- New `core/agent_loop/`: prompt -> Bifrost stream -> assistant text + tool calls -> execute tools -> append results -> loop until `stop`/`max_steps`/cancel.
- Factor the loop so the step primitive is reusable and the run is parameterized by a `strategy_profile`; v1 ships the plain ReAct profile (all knobs off) plus the seam to read a profile. The policy selector and optional passes land in Phase 7.
- Context assembler builds model input from **`bear_compiled_configs` system prompt** + **key memory projection** (SQLite) + Den transcript + prompt-memory blocks + compaction — fully Den-owned. See [Turn context assembly](../architecture/den-native-runtime.md#turn-context-assembly).
- Persist each step to canonical transcript; Den-native approvals store (new table) + pause/resume integrated with the tool-turn coordinator.

### Phase 4 — Wire native loop under existing ACP orchestration for `pair`

- Replace the execution under `AcpRuntimeSseStream` with the native loop for `pair`, keeping orchestration (tool-turn coordinator, turn controller, SSE mapping) intact.
- Flag-gate `RUNTIME=native|letta` (per-env first, optional per-bear) for parity validation via `./scripts/smoke-stack.sh` + golden ACP traces.

### Phase 5 — Collapse the seam and shed Letta-isms

- Delete conversation materialization, `conv-*` creation, run-id cancellation, approval-deny recovery, synthetic `TurnCompleted`, stale-runtime cleanup.
- Remove `RuntimeTurnBackend`/`LettaRuntimeTurnBackend`/`DenRuntimeAcpTurnRunner` and the contract traits once native is default. The loop becomes plain Den code; no pluggable runtime boundary.
- Extend native runtime to `curate` and `watch` (already API-direct, no sandbox) via capability profiles.

### Phase 6 — Den-native role registry (replace provisioning)

- Make a per-role agent a Den-owned runtime profile (compiled system prompt + model + tool roster + memory scope). No external agent create.
- `bears.letta_agent_id` -> deprecated/nullable; introduce a Den-native binding id.
- Delete Letta `create_agent`/`patch_agent`/`recompile_agent`/drift and `filtered_tool_ids` (Den owns tool descriptors). Model catalog stays from Bifrost.

### Phase 7 — Native coding harness (replace Codepool + Letta Code) for `talk`/`work`

Largest phase; needs a design spike first.

- Spike/decision: code-execution sandbox model for `work` (recommended: Den-managed ephemeral workspace containers via the existing Docker socket access; one per active task/session, lifecycle-managed like turns). Decide whether `talk` needs a sandbox.
- Reuse the native loop; add coding tools (fs edit, shell) bound to the sandbox; route `talk`/`work` memory writes through per-Bear SQLite, retiring the Letta Code git memory path.
- Tasks for `work` come from **Docket** (Den Postgres, ADR-0034): Den dispatches a task to the native `work` loop running in the Bear's scoped memory context (the execution invariant), not via generic subagents. This replaces the MemFS intent/approved-task file pipeline for human-initiated jobs.
- Land the minimal **strategy policy**: extend the ADR-0033 model-tasks policy to emit a `strategy_profile`, implement `reflect_on_fail?` (on `command`-criterion failure: SQLite reflection note + re-dispatch) and `fanout_n` (best-of-N via Docket child runs / subagents); `critique?` optional; `plan?` deferred to Docket/pair plan-mode.
- Replace Codepool's warm-pool with Den-managed turn/sandbox lifecycle; web chat transport targets the native loop instead of `CodePoolClient`.

### Phase 8 — Teardown and data migration

- One-time backfill: import Letta-Postgres conversation history not already canonical into Den `conversations`/`conversation_messages`; import residual MemFS memory content into per-Bear SQLite `memory_records`. See [`den-migration-backfill-and-rollback-plan.md`](den-migration-backfill-and-rollback-plan.md).
- Remove `LettaClient` + `core/letta/`, Letta env vars, startup gates, preflight Letta checks.
- Remove the MemFS sidecar: delete `/v1/git` routing, the `memfs` tool, `LETTA_MEMFS_SERVICE_URL`, and the `bears-letta-data` volume.
- docker-compose: remove `bears-letta`, `bears-letta-postgres`, `bears-codepool`, `bears-memfs-manager`, and `bears-redis` (if Letta-only); drop Letta Code SDK deps. **Requires explicit approval before editing compose** per repo rules.
- Supersede the old migration docs; archive [`../architecture/letta-dependency-matrix.md`](../architecture/letta-dependency-matrix.md); update [`../architecture/memory-model.md`](../architecture/memory-model.md) to the SQLite-canonical model and mark ADR-0031 accepted.

## Risks and sequencing notes

- **Tool-calling fidelity:** provider/Bifrost tool-call streaming differs from Letta's framing; Phase 1 parser + Phase 4 golden traces de-risk this.
- **Context/compaction parity:** Den must reproduce in-context management Letta did implicitly (Phase 3 context assembler), including **`bear_compiled_configs` for system prompts** and **key memory projection** for proactive SQLite grounding — see [Turn context assembly](../architecture/den-native-runtime.md#turn-context-assembly).
- **Memory model shift (Phase 2):** moving from a markdown file tree to append-only SQLite records is the biggest conceptual change. The logical-path projection must preserve the stable-anchor UX prompts depend on; data migration of existing MemFS content is deferred to Phase 8.
- **Cross-store discipline:** Den Postgres (control plane) and per-Bear SQLite (cognition) must not grow a sync seam; control plane references cognition by id only.
- **Harness is the frontier (Phase 7):** sandbox isolation and lifecycle are a sub-project; keep `pair` (Phases 1-5) shippable and Letta-free independently of it.
- Keep Letta runnable behind `RUNTIME=letta` only until Phase 5 parity is proven, then remove. MemFS coexists with SQLite only transitionally until Phase 7.

## Non-goals

- No new vector store; SQLite is canonical record storage, not a semantic index.
- No pluggable multi-runtime abstraction retained "for optionality" — the native loop is the runtime.
- No bear-local task store: tasks/jobs stay Docket-canonical in Den Postgres (ADR-0034); SQLite stores memory/cognition only.
- No "agent-pattern" framework or pattern zoo: reasoning patterns are a small fixed set of composable strategy knobs (ADR-0033). LATS/LLM Compiler deferred.
