# Implementation Plan: Letta Migration

This plan implements the migration direction described in [Letta Migration Plan](./letta-migration-plan.md) and should be read alongside the existing Den-owned runtime, transcript, and compaction architecture documents it references.

The goal is to replace Letta as the active execution/control substrate with a Den-owned multi-role Bear runtime while preserving role semantics, transcript ownership, approval/cancellation safety, and operator visibility.

## Scope

This plan covers:

- Den-owned transcript and interaction source-of-truth completion,
- shared runtime/persistence seams extracted from ACP `pair`,
- non-blocking structured-event persistence,
- assistant final-message persistence correctness,
- canonical read-switch criteria,
- transcript-compaction dependency alignment,
- role-by-role substrate migration sequencing,
- migration/backfill mechanics,
- and rollout/rollback controls.

This plan does **not** fully specify archival retrieval replacement, editable prompt-memory replacement, or every long-term non-ACP runtime detail beyond what the Letta cutover directly depends on.

## Validation strategy

We should treat validation as two intentionally different regimes:

1. **Fast unit/library validation for active development**
   - This is the default validation loop while implementing migration slices.
   - It should rely on focused Rust unit/lib tests that run quickly and are practical to execute repeatedly during editing.
   - These tests should cover canonical record construction, dedup key derivation, persistence decision logic, replay handling, terminal-path semantics, and other logic that benefits from tight iteration.
   - For the current Den service, the primary frequent loop is `cargo test --lib --manifest-path /workspace/services/den/Cargo.toml`, optionally narrowed to targeted test groups while iterating.

2. **Smoke-stack integration validation for stack-level confidence**
   - This is the slower, environment-backed validation regime we should run before releases, major cutover points, or high-risk migration milestones.
   - It should verify that the built services, migrations, docker-compose stack, seed flows, and cross-service runtime behavior work together in a live environment.
   - In this repository, `scripts/smoke-stack.sh` is the right anchor for that regime because it builds the local images, starts the bundled stack, waits for readiness, applies the smoke seed, and runs `scripts/smoke.sh`.
   - Smoke-stack validation is the right place to verify migrated-schema presence, live Postgres uniqueness contracts, service wiring, and end-to-end behavior that unit tests should not try to simulate fully.

### Practical policy for this migration

- Use **unit/lib tests** as the required default proof for most code changes during implementation.
- Use **targeted live DB probes** only when needed to validate migration/schema assumptions that unit tests cannot establish by themselves.
- Use **smoke-stack validation** as the release/pre-release gate for Letta migration slices that change persistence, migrations, transcript ownership, or cross-service runtime behavior.
- Do **not** try to force every stack concern into the fast unit-test loop; keep the fast path fast.
- Do **not** treat smoke-stack checks as a substitute for good unit coverage; they are the slower confidence layer on top.

## Success criteria

The implementation is successful when:

- Den owns the canonical transcript contract for active migrated runtime surfaces.
- ACP `pair` can rely on Den-owned transcript, event, approval, and terminal-outcome persistence independent of Letta history reads.
- Structured tool/workflow events and finalized assistant output are persisted without perturbing ACP stream ordering.
- Canonical reads can prefer Den-owned conversation history for eligible sessions with explicit provenance and fallback behavior.
- Shared runtime/persistence primitives extracted from `pair` are sufficient to begin `review` and `watch` migration without re-embedding Letta semantics.
- Migration/backfill and rollback procedures are explicit enough to support staged cutover.

## Guiding constraints

- Transcript ownership stays ahead of execution-substrate replacement.
- Non-blocking persistence must not destabilize ACP stream semantics.
- Finalized assistant output must be persisted exactly once per terminal turn path.
- Tool, approval, and workflow spans remain auditable throughout migration.
- Den-owned control-plane state remains conceptually primary; provider identifiers stay implementation details.
- Initial rollout should favor correctness, auditability, and rollback over aggressive cutover speed.

---

## Phase 0 — Scope freeze and migration invariants

**Goal:** lock the implementation-facing migration contract before cutover logic spreads further.

### Tasks

1. Define the implementation invariants in one migration-facing note:
   - transcript source of truth
   - canonical visible message
   - structured diagnostic event
   - finalized assistant output
   - read-switch eligibility
   - provider-compatibility boundary
   - dual-write and fallback period
2. Enumerate protected runtime behaviors that must retain parity during migration:
   - ACP stream ordering
   - tool continuation correctness
   - approval pause/resume correctness
   - cancellation hygiene
   - operator/debug provenance
3. Define the first migration surfaces explicitly:
   - ACP `pair` transcript completion
   - shared runtime/persistence extraction
   - `review` and `watch` follow-on migration
4. Define what remains intentionally out of scope for the first cut:
   - full retrieval replacement
   - full prompt-memory/block replacement
   - complete `chat`/`work` harness replacement

### Acceptance

- A short implementation-facing migration contract exists.
- The protected parity behaviors are explicit and testable.
- The first migration surfaces and exclusions are named.

---

## Phase 1 — Canonical transcript ownership completion for ACP `pair`

**Goal:** finish Den-owned source persistence for the ACP-visible transcript path.

### Tasks

1. Confirm and document the currently landed source-persisted event classes:
   - user prompt message
   - conversation resolved event
   - tool request event
   - assistant final output
   - turn outcome
2. Close remaining transcript gaps, especially around:
   - finalized assistant-message persistence semantics,
   - tool-result persistence coverage,
   - failure/cancellation edge cases,
   - replay/retry safety.
3. Standardize ACP-visible transcript persistence boundaries using Den-owned canonical record helpers.
4. Preserve visible-history separation from diagnostic-only workflow/tool records.

### Acceptance

- A normal ACP turn yields a complete canonical transcript/event trail in Den.
- Final assistant output is source-persisted exactly once on terminal turn paths.
- Tool-result and workflow persistence coverage is explicit and tested.
- Visible-history reads remain cleanly separated from diagnostic-only records.

---

## Phase 2 — Non-blocking structured-event persistence hardening

**Goal:** ensure structured tool/workflow persistence never perturbs ACP stream ordering.

### Tasks

1. Standardize all ACP structured-event persistence on non-blocking spawned persistence paths.
2. Audit stream hot paths for inline writes that can reorder or stall adapter-visible events.
3. Route these classes through the non-blocking persistence seam:
   - tool request
   - tool result
   - conversation resolved
   - workflow transitions
   - terminal outcomes
4. Add explicit observability for persistence failures without making them stream-fatal by default.
5. Define when a persistence failure is advisory vs terminal.

### Acceptance

- Structured persistence does not block SSE emission ordering.
- Event persistence failures are observable and attributable.
- The hot path contains no unnecessary inline canonical DB writes for structured events.

---

## Phase 3 — Idempotency and dedup contract

**Goal:** make canonical persistence safe under retries, replay, and mixed-source migration periods.

### Tasks

1. Define idempotency keys or dedup semantics for:
   - assistant final output
   - tool request/result events
   - turn outcomes
   - conversation-resolved events
2. Decide which identifiers are authoritative for dedup:
   - request id
   - tool call id
   - provider message id
   - scope id + event type
   - dedicated idempotency key
3. Add tests for:
   - duplicate upstream frames
   - repeated continuation attempts
   - replay of persisted side effects
   - multi-writer compatibility windows
4. Document the dedup policy for operators and migration tooling.

### Acceptance

- Canonical persistence is safe under expected retry/replay conditions.
- Duplicate visible transcript rows are prevented or explicitly explainable.
- Dedup policy is documented and test-covered.
- Fast unit/lib validation covers the canonical dedup logic and helper semantics used during implementation.
- Smoke-stack validation covers live migrated-schema behavior before release/cutover decisions.

---

## Phase 4 — Canonical read-switch criteria and mixed-origin history

**Goal:** move history reads from Letta-backed compatibility sources toward Den-owned canonical conversations.

### Tasks

1. Define eligibility criteria for canonical-read preference, such as presence of:
   - source-persisted user prompt
   - assistant final output
   - structured tool/workflow events as needed
   - terminal outcome coverage
2. Preserve mixed-origin debugability during transition by keeping provenance visible.
3. Define fallback behavior for sessions that are:
   - too old,
   - partially persisted,
   - imported/backfilled,
   - or still Letta-primary.
4. Update ACP/admin/history read paths to switch by explicit criteria rather than ad hoc assumptions.

### Acceptance

- Eligible conversations can be read primarily from Den-owned history.
- Mixed-origin sessions remain explainable to operators.
- Read-switch policy is explicit and reversible.

---

## Phase 5 — Shared runtime/persistence seam extraction from ACP `pair`

**Goal:** turn the strongest current migration path into reusable Den-native role-runner primitives.

### Tasks

1. Identify ACP `pair` runtime/persistence elements that should become shared contracts:
   - turn lifecycle primitives
   - terminal outcome handling
   - event persistence seam
   - continuation semantics
   - cancellation hooks
2. Extract generic interfaces or helper layers only where real reuse exists.
3. Keep ACP-specific policy and actuator behavior at the edge rather than inside the shared core.
4. Document the boundary between:
   - shared role-runner behavior,
   - ACP actuator specifics,
   - temporary Letta-compatibility shims.

### Acceptance

- Shared runtime/persistence primitives exist without over-generalizing the provider layer.
- ACP-specific logic remains separable from common role-runner behavior.
- `review` and `watch` migration can reuse extracted seams.

---

## Phase 6 — Follow-on migration for `review` and `watch`

**Goal:** reduce Letta coupling on lower-complexity non-harness roles before tackling `chat`/`work`.

### Tasks

1. Audit current `review` and `watch` runtime entrypoints and Letta dependencies.
2. Reuse the extracted transcript/event/run seams from Phase 5.
3. Define role-specific parity requirements for:
   - `review`: memory-read completeness, governance flow correctness, audit trail completeness
   - `watch`: event-ingestion latency, dedup, observation persistence correctness
4. Migrate transcript and run-state ownership before replacing the remaining execution substrate.

### Acceptance

- `review` and/or `watch` have a clear Den-owned runtime migration path using shared seams.
- Letta coupling is reduced without waiting for `chat`/`work` harness migration.
- Role-specific parity criteria are explicit.

---

## Phase 7 — Historical migration and backfill mechanics

**Goal:** make the transition operationally safe for recent and legacy sessions.

### Tasks

1. Define backfill/import strategy for:
   - recent user-visible ACP and chat sessions,
   - tool/workflow records,
   - titles/archive markers,
   - provenance/id mapping.
2. Decide what is fully imported vs lazily read through compatibility paths.
3. Add identifier mapping and provenance fields linking Den-owned records to Letta ids.
4. Define verification checks for dual-write or import correctness.
5. Document rollback windows and mixed-history operator workflows.

### Acceptance

- Backfill/import expectations are explicit.
- Operators can pivot between Den and legacy identifiers during transition.
- Historical data handling is good enough for audit/debug and staged cutover.

---

## Phase 8 — Cutover controls, rollout, and rollback

**Goal:** make migration reversible and measurable at each expansion step.

### Tasks

1. Add rollout controls for:
   - canonical-read preference,
   - Den-native runtime path enablement,
   - role-by-role traffic selection,
   - fallback to compatibility reads where needed.
2. Define cutover metrics and thresholds for:
   - stream continuity
   - tool continuation correctness
   - approval correctness
   - cancellation correctness
   - projection consistency
   - operator-debuggability
3. Define rollback actions, including:
   - disable canonical-read preference
   - disable Den-native runtime path for selected roles
   - retain transcript history without data loss
4. Run bounded internal soak periods before broadening traffic.

### Acceptance

- Migration can be rolled out and rolled back by bounded controls.
- Read-switch and runtime-switch behavior are independently controllable.
- Cutover decisions can be justified by explicit metrics.

---

## Phase 9 — `chat`/`work` harness replacement preparation

**Goal:** prepare the highest-complexity Letta-backed surfaces after transcript/control-plane ownership is strong enough.

### Tasks

1. Inventory the remaining Letta Code / Codepool harness dependencies for `chat` and `work`.
2. Define which shared role-runner behaviors can replace harness assumptions directly.
3. Identify which harness behaviors still need dedicated Den-native replacements:
   - background run lifecycle
   - autonomy boundaries
   - reconnect/session continuity
   - summarization/compaction integration
4. Sequence the first `chat`/`work` cutover behind the lower-risk role migrations and transcript-read completion.

### Acceptance

- `chat`/`work` migration prerequisites are explicit.
- Harness replacement is sequenced after transcript/control-plane maturity.
- Remaining Letta dependency surface is narrow and documented.

---

## Current implementation status

This section tracks the current status of the Letta migration transcript-ownership slice.

### Validation posture

- Frequent implementation validation should continue to prefer focused/unit lib tests in `services/den`.
- Smoke-stack validation via `scripts/smoke-stack.sh` should be treated as the slower integration/release-confidence layer.
- For the current idempotency slice, we now have both:
  - fast unit/helper validation in Rust tests, including duplicate-like assistant replay scope checks and cancellation turn-outcome provenance checks, and
  - live smoke-stack-backed Postgres confirmation of the migrated uniqueness contract.

### Completed in this slice

- **Phase 1 — canonical transcript ownership completion for ACP `pair` (expanded)**
  - assistant final-message persistence now includes request-scoped dedup metadata in the canonical assistant-output record path
  - tool-result persistence is explicitly emitted from ACP local-tool result settlement in:
    - `services/den/src/api/acp/stream/sse_stream.rs`
  - transcript/event persistence coverage remains centered on Den-owned canonical record builders in:
    - `services/den/src/core/conversation_events.rs`

- **Phase 2 — non-blocking structured-event persistence hardening (continued)**
  - the implementation continues to use spawned persistence for assistant output, tool results, workflow events, and terminal outcomes
  - the large remaining transcript/event persistence paths stay off the ACP stream hot path

- **Phase 3 — idempotency and dedup contract (strengthened implementation)**
  - a canonical dedup key model now exists in:
    - `services/den/src/core/conversation_events.rs`
  - canonical event persistence now derives a stable `source_event_id` from structured provenance metadata and passes it into conversation-message persistence
  - conversation-message append now performs storage-backed duplicate suppression keyed by `(conversation_id, source_event_id)` in:
    - `services/den/src/core/conversation_persistence.rs`
  - a migration adds a unique partial index for canonical source event ids in:
    - `services/den/migrations/20260602191000_conversation_message_source_event_id_unique.up.sql`

- **Validation and targeted coverage**
  - `cargo test --lib --manifest-path /workspace/services/den/Cargo.toml` is passing after this slice
  - ACP-facing helper coverage now includes request-scoped assistant-output provenance assertions in:
    - `services/den/src/api/acp/mod.rs`
  - helper coverage also asserts structured provenance fields used for stable canonical dedup ids
  - live Postgres smoke-stack validation confirms the migrated `conversation_messages.source_event_id` unique index exists and rejects duplicate canonical inserts for the same `(conversation_id, source_event_id)` pair
  - append-message duplicate suppression no longer relies on invalid `ON CONFLICT` syntax against the partial unique index; it now safely detects duplicate insert races by handling the insert error and reloading the existing sequence

### Partially complete / still in progress

- **Phase 1** has a stronger terminal-outcome canonicalization path, and helper coverage now explicitly asserts request-scoped turn-outcome provenance; broader end-to-end confirmation across every ACP terminal/failure/cancellation edge case is still needed.
- **Phase 2** still needs a fuller audit of all workflow-transition and non-tool diagnostic events to verify consistent helper usage.
- **Phase 3** now has storage-backed idempotency for canonical events that carry stable provenance-derived source ids, and smoke-stack-backed live Postgres inspection confirms the migrated uniqueness contract is present; remaining work is to wire this into repeatable automated integration tests rather than manual stack-level validation.

### Broad canonical event coverage audit

| Event / record class | Canonical helper exists | Producer/persistence path seen | Validation currently present | Broad status / gap |
| --- | --- | --- | --- | --- |
| Visible user message | Yes (`visible_user_message`) | Yes; ACP prompt flow now persists user prompts through canonical visible-message persistence in `api/acp/stream/prompt_flow.rs` | Indirect history behavior tests + canonical helper compilation/test coverage | Improved: canonical helper path now used; remaining gap is stronger explicit test coverage for prompt-path provenance/dedup semantics |
| Assistant final output | Yes (`assistant_output`) | Yes; ACP SSE stream persists buffered final assistant output on terminal turn path in `api/acp/stream/sse_stream.rs` | Strong unit/helper coverage + smoke-backed schema confidence | In good shape; remaining gap is broader edge-path confirmation across all terminal modes |
| Tool request | Yes (`tool_request`) | Yes; ACP runtime maps tool requests through `CanonicalConversationRecord::tool_request(...)` in `api/acp/stream/runtime.rs` | Helper/unit coverage present | Stronger than initial audit suggested; remaining gap is making sure every request variant flows through this path |
| Tool result | Yes (`tool_result`) | Yes; ACP SSE stream persists settled tool results through `CanonicalConversationRecord::tool_result(...)` in `api/acp/stream/sse_stream.rs` | Helper/unit coverage present | Producer path confirmed; highest-value remaining gap is broader result variants (timeout/error/replay/continuation) coverage confirmation |
| Conversation resolved | Yes (`conversation_resolved`) | Yes; persisted from `AcpGatewayEvent::ConversationResolved` side effects in `api/acp/stream/runtime.rs`, and emitted as an initial ACP event in `api/acp/stream/orchestration.rs` when a resolved conversation is known | Producer-path audit confidence; still light explicit test coverage | Better than earlier audit suggested: path is wired, but still needs explicit tests proving canonical persistence behavior and clarifying whether it is transcript-read-critical or mainly provenance/debug support |
| Turn outcome / terminal result | Yes (`turn_outcome`) | Yes; ACP SSE stream persists terminal outcome on terminal turn emission in `api/acp/stream/sse_stream.rs` | Strong helper/unit coverage | In decent shape; remaining gap is fuller failure/cancellation/recovery path breadth |
| Generic workflow event | Yes (`workflow_event`) | No active producer path found in this audit apart from being the substrate for turn outcomes/conversation-resolved helpers | Basic helper/unit coverage | Broad gap: workflow-event helper surface exists, but explicit workflow-transition producer wiring appears sparse or absent |
| Generic tool event | Yes (`tool_event`) | Yes indirectly; underlies confirmed tool request/result producer paths | Basic helper/unit coverage | Substrate appears meaningfully used through tool request/result paths |
| Other structured events / diagnostics | Partial (via `structured_event`) | No meaningful active producer path found in this audit | Sparse | Medium-priority gap: many diagnostics may still be ephemeral/operator-only rather than canonically persisted |

### Workflow-transition and structured-diagnostic audit

| Surface | Client-visible today | Canonically persisted today | Likely purpose | Audit conclusion |
| --- | --- | --- | --- | --- |
| `turn_result` / terminal outcome | Yes | Yes | Canonical workflow/terminal record | This is the primary confirmed workflow-event path today |
| `conversation_resolved` | Yes | Yes | Provenance/session binding + possible read-switch support | Wired, but still needs explicit tests and a clearer statement of whether it is transcript-read-critical |
| `PlanUpdate` / `PlanUpdateJson` | Yes | No producer-path evidence found for canonical persistence | UI/workboard state projection | Appears client-visible but not canonically persisted; likely intentional unless transcript-read requirements say otherwise |
| `PlanApprovalFallback` | Yes | No producer-path evidence found for canonical persistence | UI/workflow assist for submitted plan approval | Appears diagnostic/workflow-facing rather than canonical transcript state |
| `ModeUpdate` | Yes | No producer-path evidence found for canonical persistence | Session-mode UI state | Appears ephemeral/session-state oriented, not canonical transcript state |
| `StatusText` / reasoning text | Yes | No canonical persistence path found in this audit | UX progress/debug text | Likely ephemeral by design |
| `Error` events | Yes | Mixed: terminal errors become `turn_result`; non-terminal adapter errors do not obviously persist canonically | Operator/user visibility | Need explicit policy on which errors deserve canonical event persistence vs transient surfacing only |
| `SessionInfoUpdate` | Yes | No canonical persistence path found in this audit | Session metadata/title sync | Session-state/UI only |

### Validation/readiness audit

- **Fast unit/lib coverage is strongest for:**
  - canonical helper construction,
  - assistant-output provenance,
  - turn-outcome provenance,
  - tool request/result helper semantics,
  - canonical dedup key serialization.
- **Smoke-stack/release-confidence coverage is strongest for:**
  - migrated schema presence,
  - live Postgres uniqueness enforcement,
  - stack-level persistence contract sanity.
- **Highest-value remaining validation gaps before more coding:**
  1. explicit tests for prompt-path canonical provenance/dedup semantics,
  2. explicit tests for `conversation_resolved` persistence behavior,
  3. broader tool-result variant tests (timeout/error/replay/continuation),
  4. a written policy for whether non-terminal errors and workflow UI events should remain ephemeral or become canonical records.

### Code-ready audit summary

At this point, the migration surface is audited enough to return to implementation work.

#### Confirmed canonical transcript/event paths

- visible user prompt
- assistant final output
- tool request
- tool result
- conversation resolved
- terminal turn outcome

#### Confirmed non-canonical / primarily ephemeral surfaces

- plan-update UI projections
- mode-update UI state
- session-info update events
- status/reasoning text

#### Remaining design decisions before or during the next coding slice

1. **Workflow policy decision**
   - decide whether any workflow-state projections beyond terminal outcomes and conversation resolution should become canonical records.

2. **Error persistence policy decision**
   - decide which non-terminal/transient errors are worth canonical persistence versus remaining stream-only diagnostics.

3. **Validation tightening on already-wired paths**
   - prompt-path canonical tests,
   - `conversation_resolved` tests,
   - broader tool-result variant tests.

### Remaining work before this migration slice is complete

1. **Make the workflow/diagnostic persistence policy explicit**
   - document which workflow/UI/diagnostic surfaces are intentionally ephemeral,
   - and which should graduate into canonical transcript/event records.

2. **Broaden confirmed tool-result coverage**
   - producer wiring is present,
   - remaining work is to validate timeout/error/replay/continuation variants systematically.

3. **Validate key canonical edge paths with focused tests**
   - prompt-path provenance/dedup semantics,
   - `conversation_resolved` persistence,
   - repeated assistant-output terminalization,
   - cancellation/failure/recovery interactions.
   - These should primarily land as fast unit/lib tests where practical.

4. **Keep smoke-stack integration validation as the release-confidence layer**
   - verify migrated schema presence,
   - verify live duplicate suppression/uniqueness behavior,
   - verify stack wiring and seeded runtime behavior before release/cutover.

### Practical migration summary

Current status can be summarized as:

- **Canonical transcript/event coverage:** materially improved
- **Assistant final-message persistence:** request-scoped and more dedup-friendly
- **Tool-result persistence:** explicit and source-persisted
- **Idempotency/dedup:** initial application-level guard landed, but not yet final-form
- **Canonical-read cutover readiness:** improved, but not yet complete

## Open questions to resolve during implementation

- What exact idempotency key strategy best fits transcript-visible and diagnostic events?
- Should assistant final output carry explicit finalized metadata beyond visible assistant text semantics?
- Which parts of tool-result persistence should be considered required for read-switch eligibility?
- Which of `review` or `watch` should be the first non-ACP consumer of shared role-runner seams?
- What is the smallest practical migration/backfill toolset that still preserves rollback confidence?

## Recommended implementation order

1. Phase 0 — scope freeze and invariants
2. Phase 1 — canonical transcript ownership completion for ACP `pair`
3. Phase 2 — non-blocking structured-event persistence hardening
4. Phase 3 — idempotency and dedup contract
5. Phase 4 — canonical read-switch criteria
6. Phase 5 — shared runtime/persistence seam extraction
7. Phase 6 — `review`/`watch` follow-on migration
8. Phase 7 — historical migration and backfill
9. Phase 8 — rollout and rollback controls
10. Phase 9 — `chat`/`work` harness replacement preparation

This ordering keeps transcript/control-plane ownership ahead of execution-substrate replacement and favors safe staged cutover over a broad hard migration.
