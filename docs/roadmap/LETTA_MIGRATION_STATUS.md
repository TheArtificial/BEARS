# Letta Migration — Implementation Status

Living progress tracker for the [Letta Migration Implementation Plan](./LETTA_MIGRATION_IMPLEMENTATION_PLAN.md). Update this document when slices land; keep the implementation plan stable.

**Last updated:** 2026-06-08

---

## Phase summary

| Phase | Status | Notes |
| --- | --- | --- |
| 0 — Scope freeze and invariants | **Substantially complete** | Contract in [`acp-runtime-contract.md`](../architecture/acp-runtime-contract.md); workflow/diagnostic policy in implementation plan |
| 1 — Canonical transcript ownership (`pair`) | **Mostly complete** | Core paths wired; rare terminal-edge confirmation remains |
| 2 — Non-blocking structured-event persistence | **Mostly complete** | Spawned persistence on hot path; fuller workflow-transition audit pending |
| 3 — Idempotency and dedup | **Mostly complete** | Storage-backed dedup landed; automated integration test for uniqueness still desired |
| 4 — Canonical read-switch | **Mostly complete** | Den-first history for eligible sessions; startup/control surfaces still Letta-backed |
| 5 — Shared runtime/persistence extraction | **In progress** | Persistence seams largely extracted; turn execution still Letta-primary |
| 6 — `review` / `watch` follow-on | **Partial** | Review projection + Curate queue/worker scaffolding; real Curate execution substrate not built |
| 7 — Backfill mechanics | **Planning baseline** | [`den-migration-backfill-and-rollback-plan.md`](./den-migration-backfill-and-rollback-plan.md) |
| 8 — Rollout and rollback controls | **Not started** | |
| 9 — `chat`/`work` harness prep | **Deferred** | Correctly sequenced after lower-risk roles |

---

## Active epic: A2 — Complete ACP runtime contract boundary

**Goal:** Finish shrinking Letta to a narrow execution adapter behind `AcpTurnRunner` / `AcpConversationRuntime`.

Aligned with milestones in [`acp-runtime-contract.md`](../architecture/acp-runtime-contract.md).

| Slice | Status | Exit criteria |
| --- | --- | --- |
| 1. Structured semantic streaming on all turn paths | **In progress** | Start + continue emit `RuntimeSemanticEvent`; ACP no longer branches on Letta SSE fragments |
| 2. Conversation lifecycle contract | **Not started** | `prompt_flow` / session lifecycle use `AcpConversationRuntime`; lazy materialization tested |
| 3. Cancellation and hygiene normalization | **Not started** | `RuntimeErrorCategory` replaces Letta error string matching |
| 4. Validation gate | **In progress** | Contract tests green; smoke-stack replay/idempotency stable; idempotency integration test automated |

**Release gate:** slices 1–3 complete + validation gate green.

---

## Epic status

| Epic | Status |
| --- | --- |
| A — Turn/run lifecycle extraction | Partial — `AcpTurnRunner`, semantic events, settlement/continuation helpers landed; execution still via Letta adapter |
| A2 — Complete ACP runtime contract boundary | **Active** (see above) |
| B — Compaction design + core types | Complete (design contract) |
| B2 — Compaction runtime integration | Complete (initial live history/read-model slice) |
| C — Prompt memory blocks | Complete (initial runtime prompt-compilation slice) |

---

## What is still Letta-primary

- Turn **execution** (start/continue/cancel via `LettaRuntimeTurnBackend`)
- Conversation **materialization** at execution boundaries (`upstream_target`, lazy runtime creation)
- Session lifecycle cancellation heuristics matching Letta error strings (`api/acp/runtime_support.rs`)
- Pair reflection close-out optional Letta message history read
- `chat` / `work` Codepool harness (Phase 9)

---

## Phase 4 — Read-switch progress

- ACP `pair` conversation history serves canonical Den-owned history without live Letta fallback in the production read path.
- ACP session compaction degrades explicitly when Letta is unavailable.
- Pair reflection close-out prefers canonical persisted rows before Letta runtime message history.
- ACP local-tool SSE continuation regression (`late_tool_result_ignored`) resolved.
- **Remaining:** prompt/runtime conversation selection and other startup/control surfaces still require Letta-backed runtime bindings.

**Read-switch policy (ACP history handler):** canonical history is preferred only when canonical rows produce at least one visible transcript message; diagnostic-only canonical pages fall back to Letta/runtime history.

---

## Phase 5 — Extraction progress

### Canonical persistence seam (complete for current ACP event set)

- `core/conversation_events.rs`: transport-neutral constructors, spawn helpers, `normalized_structured_event`, `normalize_persisted_gateway_record`, typed `Projection` seam.
- `api/acp/stream/runtime.rs`: ACP adapter helpers (`acp_session_provenance`, `spawn_persist_acp_*`, `spawn_canonical_gateway_record_persistence`).
- Schema alignment: tool requests → `tool_call`; tool results → `tool_result` (not legacy `tool_event`).

### Tool settlement and continuation (landed)

- `core/acp_tool_turns.rs`: `settle_after_result`, `AcpToolSettlementSummary`, `prepare_runtime_continuation`.
- `core/acp_turn_runner.rs`: `default_acp_tool_continue_stream_context`.
- `api/acp/stream/support.rs`: `AcpStreamDiagnostics::reset_for_resumed_continuation`.
- **Decision:** keep `ApiState` assembly and `RoleRuntimeBinding` selection ACP-owned for now.

### Semantic streaming (partial)

- `core/letta_runtime_stream_parser.rs` → `RuntimeSemanticEvent`
- `core/runtime/bearwire_projection/mod.rs` → ACP gateway events
- ACP `sse_stream.rs` consumes semantic events on main turn loop; some paths may still use `RawBytes`

### Runtime contract (partial)

- `core/runtime/contracts/mod.rs`: `AcpTurnRunner`, `AcpConversationRuntime`, `EnsureConversationRequest/Result`
- `core/acp_turn_runner.rs`: `DenRuntimeAcpTurnRunner` delegating to Letta backends
- `core/acp_runtime.rs`: `ensure_acp_session_conversation`

### Strongest shared candidates (for future `review`/`watch`)

1. `core/role_runtime.rs` — turn ownership, terminal result, cancellation registration
2. `core/conversation_events.rs` — transport-neutral persistence
3. Turn-result / terminal-event contract in `sse_stream.rs`
4. Tool continuation settlement in `mapping.rs` + `acp_tool_turns.rs`

### Stay ACP-edge for now

- Adapter SSE/event mapping
- ACP tool routing policy
- Prompt/context orchestration

---

## Phase 6 — Review / Curate progress

### Review lifecycle projection (landed)

- `spawn_persist_workflow_event`, `spawn_persist_assistant_summary_message`
- Proposal lifecycle: `memory_proposal_created`, `memory_proposal_resolved`, visible summaries for all major states
- `den.memory.apply_core_update` threads MemFS `path` + `canonical_tip` into resolution projection
- `den.memory.request_review` includes conversation/session provenance in `source_refs`

### Typed Projection adoption (landed)

Consumers: `memory_proposals`, `pair_reflection`, `reflection_conductor`, `den_tools`

Constructors: proposal lifecycle + Curate run-state (`memory_curate_enqueued/started/completed/failed`)

### Curate worker (scaffolding — not full execution)

- `list_queued_memory_curate_runs`, `claim_next_memory_curate_run`
- `run_next_memory_curate_once` — claims run, resolves pending proposals to `needs_human_review`, marks complete/failed
- `run_memory_curate_worker_loop` — launched from `den::run()` via `RUN_WORKERS`
- **Honest status:** queue + projection + stub resolution exist; real Curate review execution substrate not built

### Test coverage

- 30+ canonical-focused tests in `core::conversation::events::tests`
- DB-backed projection integration tests for proposal and Curate lifecycle

---

## Epic B — Compaction (complete at design + initial runtime)

**Design artifacts:**

- `docs/decisions/adr-0032-den-context-compaction-architecture.md`
- `docs/architecture/den-context-compaction-contract.md`
- `docs/architecture/den-context-compaction-schema.md`
- `docs/architecture/den-context-compaction-observability.md`
- `docs/guides/context-compaction-guide.md`

**Code:** `runtime_conversations.rs`, `runtime_compaction.rs`, `runtime_compaction_observability.rs`, `runtime_compaction_store.rs`, ACP history/compaction status projection.

**Follow-on:** expand compaction into more live execution/prompt-assembly paths; persist richer envelope payloads durably.

---

## Epic C — Prompt memory blocks (complete at initial slice)

**Design:** `den-prompt-memory-block-contract.md`, schema, guide

**Code:** `prompt_memory_blocks.rs`, `prompt_memory_block_store.rs`, migration `20260605150000_prompt_memory_blocks`, `api/acp/prompt_context.rs`

**Validation landed in `api/acp/tests.rs`:**

- Persisted inclusion, archived exclusion, scope precedence, budget omission
- Selection matrix for inactive/mismatched rows
- Mutation: conflict-archive and supersession flows
- ACP prompt-context budgeting diagnostics

**Follow-on:** richer mutation workflows, admin visibility.

---

## Transcript ownership slice — completed work

### Phases 1–3 (landed)

- Request-scoped assistant-output dedup metadata
- Tool-result persistence from ACP local-tool settlement (`sse_stream.rs`)
- Spawned persistence for assistant output, tool results, workflow events, terminal outcomes
- Canonical dedup key model; `source_event_id` from provenance metadata
- Unique partial index: `20260602191000_conversation_message_source_event_id_unique`
- Duplicate suppression via insert-error handling + reload

### Validation

- `cargo test --lib --manifest-path services/den/Cargo.toml` passing
- Live Postgres smoke-stack confirms uniqueness contract
- Focused ACP provenance/dedup tests in `api/acp/tests.rs` and `api/acp/mod.rs`

### Partially complete

- Phase 1: broader end-to-end terminal/failure/cancellation edge cases
- Phase 2: fuller workflow-transition producer audit
- Phase 3: repeatable automated integration test for uniqueness (not manual smoke only)

---

## Canonical event coverage audit

| Event / record class | Helper | Producer | Validation | Gap |
| --- | --- | --- | --- | --- |
| Visible user message | Yes | `prompt_flow.rs` | DB-backed provenance/dedup tests | Broader e2e prompt-path |
| Assistant final output | Yes | `sse_stream.rs` | Strong unit + smoke schema | Rare terminal modes |
| Tool request | Yes | `runtime.rs` | Helper/unit | All request variants |
| Tool result | Yes | `sse_stream.rs` | DB-backed timeout/error/continuation | Deeper live integration |
| Conversation resolved | Yes | `runtime.rs`, `orchestration.rs` | DB-backed + replay/read | Stack-level proof |
| Turn outcome | Yes | `sse_stream.rs` | Failed/cancelled/recovered branches | Rare terminal modes |
| Generic workflow event | Yes | Sparse producers | Basic helper | Explicit transition wiring |
| Other diagnostics | Partial | Sparse | Sparse | Likely ephemeral by design |

---

## Workflow / diagnostic surface audit

| Surface | Client-visible | Canonically persisted | Conclusion |
| --- | --- | --- | --- |
| `turn_result` / terminal outcome | Yes | Yes | Primary workflow path |
| `conversation_resolved` | Yes | Yes | Provenance/read-switch support |
| `PlanUpdate` / `PlanUpdateJson` | Yes | No | Ephemeral UI (intentional) |
| `PlanApprovalFallback` | Yes | No | Ephemeral UI |
| `ModeUpdate` | Yes | No | Session UI state |
| `StatusText` / reasoning | Yes | No | Ephemeral UX |
| `Error` events | Yes | Mixed | Terminal → `turn_result`; non-terminal stream-only |
| `SessionInfoUpdate` | Yes | No | Session metadata |

---

## Validation posture

**Fast loop:** `cargo test --lib --manifest-path services/den/Cargo.toml`

**Release gate:** `scripts/smoke-stack.sh`

### Replay / continuation

**Lib tests (passing):**

- `acp_stream_waits_for_tool_result_and_continues_runtime`
- `acp_tool_result_endpoint_treats_replayed_identical_result_as_idempotent`
- `acp_tool_result_endpoint_marks_changed_replay_as_conflict`

**Smoke:** `tests/smoke/test_stack.py` includes tool-result replay/idempotency scenario. Limitation: live providers may skip tool path; test records skip instead of failing.

### Validation tightening (progressing)

- ACP-session provenance helper tests
- `conversation_resolved` provenance/metadata tests
- Prompt-path canonical record shape tests

### Remaining validation gaps

1. Automated idempotency integration test (not manual Postgres probe)
2. Stack-level prompt-path + `conversation_resolved` persistence proof
3. Stabilize smoke replay scenario reporting when tool path not taken

---

## Practical status summary

- **Canonical transcript/event coverage:** materially improved
- **Assistant final-message persistence:** request-scoped, dedup-friendly
- **Tool-result persistence:** explicit, source-persisted
- **Idempotency/dedup:** application guard landed; automation pending
- **Canonical-read cutover:** improved; startup surfaces remain
- **Turn execution:** still Letta-primary (Epic A2 target)

---

## Resolved decisions (formerly open questions)

| Question | Decision |
| --- | --- |
| `review` vs `watch` first non-ACP consumer? | **`review`** — closer to message/workflow seam, less continuation complexity |
| Idempotency key strategy? | Provenance-derived `source_event_id`; unique on `(conversation_id, source_event_id)` |
| Workflow projections beyond terminal outcomes? | **No** unless product requirement changes — see implementation plan policy |
| Non-terminal error persistence? | **Stream-only by default**; terminal errors via `turn_outcome` |

---

## Changelog

### 2026-06-08 — Document split

- Split living status from stable implementation plan per review.
- Declared Epic A2 as active focus.

### Prior slices (condensed)

- ACP persistence adapter seam completed
- Review/Curate typed projection + worker scaffolding
- Compaction B2 live history slice
- Prompt memory Epic C initial slice + validation pass
- Idempotency migration + smoke uniqueness confirmation
- Phase 5 extraction: settlement, continuation prep, gateway normalization
- Phase 4 read-switch + tool continuation fix
