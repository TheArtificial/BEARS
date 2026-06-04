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

### Current Phase 5 extraction audit

The current ACP `pair` path already exposes a few strong extraction candidates and a few areas that should remain ACP-edge specific.

#### Strongest shared candidates

1. **Turn lifecycle primitives**
   - `core/role_runtime.rs` already contains role/channel-scoped turn ownership, terminal result shape, and cancellation registration.
   - This looks like the best existing shared core because it is not intrinsically ACP-transport-specific.

2. **Canonical conversation persistence seam**
   - `core/conversation_events.rs` plus the persistence context/record model already represent a transport-neutral event/message persistence contract.
   - The ACP stream currently consumes this through helper wrappers in `api/acp/stream/runtime.rs`.
   - This seam appears reusable for `review`/`watch` provided the role-specific provenance builder stays at the edge.

3. **Runtime-event to terminal-outcome contract**
   - `api/acp/stream/sse_stream.rs` currently applies a disciplined turn-result/terminal-event contract around runtime failure/cancellation/completion.
   - The policy surface is partly ACP-shaped today, but the underlying role-turn result model is a plausible shared runner primitive.

4. **Tool continuation settlement shape**
   - `api/acp/stream/mapping.rs` and ACP tool-turn coordination currently expose a reusable conceptual seam:
     - runtime requests a tool,
     - execution route is classified,
     - a continuation receiver is retained,
     - runtime resumes from a resolved tool result.
   - The exact adapter/approval payloads are ACP-specific, but the continuation lifecycle itself is a candidate shared concept.

#### Things that should stay ACP-edge specific for now

1. **Adapter SSE/event mapping**
   - ACP message/event shapes, session-info updates, plan-mode payloads, and approval-request transport details should remain at the ACP edge.

2. **ACP tool routing policy**
   - direct adapter-local vs Den-server routing, approval presentation, and unsupported-tool fallback are currently ACP client-contract decisions.

3. **Prompt/context orchestration**
   - ACP prompt guidance, adapter contract handling, workflow UI projections, and session-title sync are not good first extraction targets.

#### Smallest extraction-friendly next code slice

The best low-risk Phase 5 move appears to be:

- **extract transport-neutral canonical persistence helper(s) out of `api/acp/stream/runtime.rs` into shared core-facing runtime/persistence support**, while
- leaving ACP event mapping, tool routing, and adapter payload policy in ACP modules.

Why this slice first:
- it already uses a transport-neutral canonical record model,
- it reduces ACP ownership of generic persistence glue,
- it helps future `review`/`watch` migration without forcing premature runtime-provider abstraction.

### Phase 5 progress in this slice

- A first extraction-friendly refactor is now landed:
  - `core/conversation_events.rs` now exposes a transport-neutral `canonical_persistence_context(...)` constructor over primitive inputs.
- ACP still owns the ACP-specific context adapter in:
  - `api/acp/stream/runtime.rs` via `canonical_persistence_context_from_acp(...)`.
- `api/acp/stream/sse_stream.rs` now consumes that ACP adapter helper rather than depending on ACP-local persistence construction details directly.
- A second small extraction step is also landed:
  - `core/conversation_events.rs` now exposes shared spawn helpers for assistant-output and turn-outcome persistence.
  - `api/acp/stream/sse_stream.rs` now delegates those transport-neutral record-building details to core helpers instead of inlining them.
- A third small extraction step is also landed:
  - `core/conversation_events.rs` now exposes a shared spawn helper for canonical tool-result persistence.
  - `api/acp/stream/sse_stream.rs` now delegates transport-neutral tool-result record construction to core instead of inlining it.
- A fourth small extraction step is also landed:
  - `core/conversation_events.rs` now exposes a shared spawn helper for canonical tool-request persistence.
  - `api/acp/stream/runtime.rs` now delegates transport-neutral tool-request record construction to core instead of inlining it.

### Continuation / tool-settlement seam audit

The next stronger shared seam is not another canonical record helper; it is the lifecycle around pending tool turns and runtime continuation.

#### Current lifecycle split

1. **Registration lives partly in ACP runtime, partly in core coordinator**
   - ACP runtime decides routing and approval semantics.
   - `core/acp_tool_turns.rs` owns the actual pending-turn registry and delivery contract.

2. **Continuation retention lives in ACP stream orchestration**
   - `api/acp/stream/mapping.rs` and `sse_stream.rs` retain adapter/local continuation receivers.
   - This is where ACP-specific event framing and runtime resumption are still tightly interleaved.

3. **Settlement / cleanup is currently mixed**
   - core coordinator already owns result delivery, recently-settled replay caching, timeout synthesis, and request/session cleanup helpers.
   - ACP stream code still performs some settlement-side orchestration directly:
     - persist canonical tool result,
     - notify turn controller,
     - remove pending turn from coordinator,
     - queue continuation,
     - emit UI-facing completion/status events.

#### Best first extraction candidate in this seam

The safest next extraction candidate appears to be a **shared tool-settlement helper/contract** that groups the transport-neutral parts of post-result handling:

- canonical tool-result persistence,
- coordinator removal / settled-result bookkeeping,
- normalized settlement summary usable by role runners,

while leaving these at the ACP edge:
- route classification,
- approval semantics,
- adapter/SSE status text,
- plan/mode UI projection,
- runtime-specific continuation driving.

#### Recommended next code slice

The smallest safe follow-on implementation slice is:

- introduce a shared core-facing helper that consumes a settled tool result plus coordinator/session identifiers and performs the transport-neutral settlement bookkeeping,
- return a small summary structure that ACP can use to drive turn-controller updates and UI events.

Status: **initial slice landed**
- `core/acp_tool_turns.rs` now exposes `settle_after_result(...)` returning `AcpToolSettlementSummary`.
- `api/acp/stream/sse_stream.rs` now delegates pending-turn removal bookkeeping to that shared coordinator helper before ACP-specific controller/UI follow-up.
- A second small settlement slice is also landed:
  - `AcpToolSettlementSummary` now carries normalized classification flags (`completed_ok`, `timed_out`).
  - `api/acp/stream/sse_stream.rs` now consumes those core-owned settlement flags for turn-controller updates instead of re-deriving status policy inline.
- A third small settlement slice is also landed:
  - `AcpToolSettlementSummary` now carries a normalized `display_tool_name` fallback.
  - `api/acp/stream/sse_stream.rs` now consumes that core-owned display name when building completion status text instead of re-deriving the fallback inline.

#### Continuation resume contract audit

The next broader seam after settlement is the **resume contract** between queued tool results and runtime continuation.

##### Current continuation split

1. **ACP stream owns queueing and resume triggering**
   - `queued_tool_result_continuation` lives in `api/acp/stream/sse_stream.rs`.
   - ACP decides when upstream/runtime conditions allow resumption.

2. **ACP stream still inlines continuation request construction**
   - it converts a queued `AcpToolResultRequest` into either:
     - `RuntimeContinuation::ApprovalDecision`, or
     - `RuntimeContinuation::ToolResult`.
   - it also inlines status mapping (`ok` / `timeout` / error) and missing-`tool_call_id` refusal behavior.

3. **Runtime driving remains ACP-specific**
   - creation of `ApiState`, `RoleRuntimeBinding`, `AcpTurnContinueRequest`, and resumed stream diagnostics stays tightly coupled to ACP stream/provider orchestration.
   - this part is not yet a good extraction target because it still mixes provider selection and stream framing assumptions.

##### Best first continuation-resume extraction candidate

The safest next candidate appears to be a **shared continuation request builder/helper** that converts a settled ACP tool result into a normalized runtime continuation input.

That helper can own transport-neutral pieces such as:
- requiring original `tool_call_id`,
- mapping ACP tool-result status into `RuntimeToolResultStatus`,
- choosing approval-decision vs tool-result continuation shape,
- carrying normalized continuation-preparation diagnostics/result.

These should remain at the ACP edge:
- deciding when the queued continuation may run,
- emitting adapter error/status events,
- constructing `ApiState` / binding / streamed continuation execution,
- post-resume event framing and diagnostics.

##### Recommended next implementation slice

The smallest safe follow-on slice after settlement is:

- introduce a core-facing helper that transforms `AcpToolResultRequest` into a prepared runtime continuation payload (or a structured preparation error),
- update `sse_stream.rs` to consume that prepared continuation while retaining ACP-specific event/error framing.

Status: **initial continuation-preparation slice landed**
- `core/acp_tool_turns.rs` now exposes `prepare_runtime_continuation(...)` plus structured result/error types.
- `api/acp/stream/sse_stream.rs` now delegates missing-`tool_call_id` validation, status mapping, and approval-vs-tool-result continuation shaping to that shared helper.
- ACP stream still owns user-facing error framing and resumed runtime execution.
- A second small continuation-resume slice is also landed:
  - `core/acp_turn_runner.rs` now exposes `default_acp_tool_continue_stream_context()`.
  - `api/acp/stream/sse_stream.rs` now delegates the default resumed-tool stream context construction to that shared helper instead of rebuilding the default inline.

##### Pre-resume execution-context audit

After those slices, the remaining ACP-side resume setup is concentrated in three places:

1. **Execution-state assembly remains ACP-owned**
   - `sse_stream.rs` still constructs a fresh `ApiState` for resumed continuation execution.
   - This includes config/client wiring (`LettaClient`, `BifrostClient`) plus process-local registries.
   - This is not yet a good extraction target because it is tightly tied to API/runtime wiring rather than continuation semantics.

2. **Binding selection remains ACP-owned**
   - `sse_stream.rs` still builds `RoleRuntimeBinding` inline for the compatibility backend (`letta`).
   - This may become a future seam if runtime binding selection becomes transport-neutral, but today it is still ACP/provider policy.

3. **Diagnostics reset is the smallest remaining shared-friendly candidate**
   - resumed continuation setup still resets `AcpStreamDiagnostics` in-line after successful resume preparation/execution handoff,
   - and flips `saw_requires_approval_stop = false` in multiple nearby places.
   - This is the cleanest remaining pre-resume helper candidate if we want one more extraction before touching larger runtime wiring.

##### Best next non-trivial candidate

The safest next candidate is a **small resumed-stream diagnostics initializer/reset helper** that centralizes the default diagnostics state expected after a continuation resume is launched.

That helper could own:
- creation of fresh `AcpStreamDiagnostics`,
- reset of `saw_requires_approval_stop`,
- any future default flags that should be consistently cleared on resumed runtime execution.

Status: **diagnostics reset helper landed**
- `api/acp/stream/support.rs` now exposes:
  - `AcpStreamDiagnostics::resumed_continuation_defaults()`
  - `AcpStreamDiagnostics::reset_for_resumed_continuation()`
- `api/acp/stream/sse_stream.rs` now delegates resumed diagnostics initialization/reset to those helpers instead of mutating `saw_requires_approval_stop` inline.

##### ACP resume execution wiring audit

A broader audit of the remaining resumed execution wiring shows that `ApiState` and `RoleRuntimeBinding` assembly should stay ACP-owned for now.

1. **`ApiState` construction is API-surface wiring, not continuation semantics**
   - `sse_stream.rs` rebuilds `ApiState` in resume and cleanup paths using:
     - SQL pool,
     - shared config,
     - freshly wrapped `LettaClient` / `BifrostClient`,
     - process-local ACP registries.
   - The same shape also appears in `api/service.rs` and test helpers, which indicates this is application/runtime wiring rather than a continuation-specific contract.
   - Extracting this too early would mostly move dependency assembly around without creating a transport-neutral seam.

2. **`RoleRuntimeBinding` is still provider-policy encoded**
   - resumed ACP continuation currently chooses compatibility backend `"letta"` inline.
   - nearby runtime code and tests also encode backend-specific binding values (for example `"runtime:letta"` and `"letta"`).
   - until runtime binding selection is normalized across providers/runners, this remains ACP/provider policy rather than reusable continuation preparation.

3. **Future extraction condition**
   - a shared seam becomes more realistic only if Den introduces:
     - a reusable runtime-execution context factory used by multiple runners, or
     - normalized binding-selection policy independent of ACP transport decisions.
   - absent that, extracting these now would create indirection without meaningful reuse.

Decision: **keep `ApiState` assembly and `RoleRuntimeBinding` selection ACP-owned for now.**

### Recommended next broader Phase 5 seam

The strongest next boundary is to return to the **broader runtime/persistence seam** rather than continue ACP resume-path factoring.

Specifically, the best next target is:

#### Shared conversation-event persistence entrypoint / policy seam

Why this is stronger than more ACP resume work:
- `core/conversation_events.rs` already owns the extracted transport-neutral spawn helpers for:
  - assistant output,
  - turn outcome,
  - tool result,
  - tool request.
- That file is now the clearest shared anchor for non-ACP reuse.
- The next broader reuse win is likely not another ACP stream helper, but clearer ownership of:
  - persistence entrypoint policy,
  - event-category normalization,
  - or record/spawn orchestration used by future non-ACP runners.

Why this seam now:
- it is already partially centralized,
- it aligns with the migration goal of Den-first canonical persistence,
- it has better cross-runner reuse potential than ACP-local execution wiring,
- it avoids overfitting Phase 5 to Letta/ACP stream mechanics.

#### Recommended next implementation slice under that seam

The next concrete slice should be:

- audit `core/conversation_events.rs` for the next duplication or policy split between:
  - canonical record construction,
  - spawn orchestration,
  - and caller-owned event categorization/provenance shaping,
- then extract the next transport-neutral persistence entrypoint/helper that future runners can call without ACP-specific knowledge.

##### Conversation-event persistence seam audit

Current caller/core split after the recent extractions:

1. **core already owns record construction and spawn orchestration**
   - `core/conversation_events.rs` owns:
     - `spawn_persist_canonical_conversation_record(...)`
     - `spawn_persist_assistant_output(...)`
     - `spawn_persist_turn_outcome(...)`
     - `spawn_persist_tool_result(...)`
     - `spawn_persist_tool_request(...)`

2. **ACP callers still own provenance creation policy**
   - ACP callsites in `sse_stream.rs` and `runtime.rs` repeatedly construct:
     - `ConversationEventProvenance::acp_session(acp_session_id.clone())`
   - then immediately pass that provenance into core persistence helpers.

3. **The remaining split is policy-shaped, not just syntactic**
   - provenance creation is currently transport-specific (`acp_session`) but the repeated pattern suggests the next reusable seam is a small **transport-adapter persistence entrypoint** rather than another raw spawn helper.
   - the transport-neutral core should still avoid importing ACP concerns directly, but ACP-facing code can likely centralize this policy in one ACP adapter helper instead of rebuilding provenance at each callsite.

##### Best next persistence extraction candidate

The strongest next slice appears to be:

- an **ACP-facing persistence adapter helper** that bundles:
  - `canonical_persistence_context_from_acp(...)`
  - ACP-session provenance creation
  - and a small number of common ACP persistence entrypoints

This is stronger than further raw-core helper extraction because:
- core helper coverage is already reasonably good,
- the repeated duplication now lives more in ACP adapter policy than in canonical record construction,
- it preserves the transport-neutral boundary while still reducing ACP callsite noise.

##### Recommended next implementation slice

The safest next implementation slice is:

- introduce one small ACP adapter helper for the most repeated persistence path(s), starting with ACP-session provenance bundling for assistant-output / turn-outcome / tool-result persistence,
- keep `core/conversation_events.rs` transport-neutral,
- reduce repeated ACP callsite provenance shaping before considering any deeper persistence-policy refactor.

Status: **ACP persistence adapter seam completed for current ACP event set**
- `api/acp/stream/runtime.rs` now exposes:
  - `acp_session_provenance(...)`
  - `spawn_persist_acp_assistant_output(...)`
  - `spawn_persist_acp_turn_outcome(...)`
  - `spawn_persist_acp_tool_result(...)`
  - `spawn_persist_acp_conversation_resolved(...)`
  - `spawn_persist_acp_tool_request(...)`
- `api/acp/stream/sse_stream.rs` now delegates assistant-output, turn-outcome, and tool-result persistence through ACP adapter helpers instead of rebuilding ACP-session provenance inline.
- `api/acp/stream/runtime.rs` now also delegates conversation-resolved and tool-request persistence through ACP adapter helpers, finishing the current ACP-side provenance/context bundling pass while keeping `core/conversation_events.rs` transport-neutral.
- A follow-on schema-alignment cleanup is now also landed:
  - `core/conversation_events.rs` now persists tool requests as canonical `tool_call` rows and tool-result-style diagnostic events as canonical `tool_result` rows instead of the schema-invalid synthetic `tool_event` type.
- A larger shared-entrypoint cleanup is now also landed:
  - `core/conversation_events.rs` now owns `normalized_structured_event(...)`, which centralizes schema-valid canonical event-category normalization for structured records.
  - `api/acp/stream/runtime.rs` now delegates structured canonical category normalization to that shared core entrypoint instead of open-coding the `tool_event` / `tool_call` / `workflow_event` mapping at the ACP adapter edge.
- A broader visible-vs-structured entrypoint simplification is now also landed:
  - `core/conversation_events.rs` now owns `normalize_persisted_gateway_record(...)`, which centralizes the shared decision between visible transcript rows (`message` + role) and normalized structured canonical records.
  - `api/acp/stream/runtime.rs` now exposes one ACP adapter entrypoint, `spawn_canonical_gateway_record_persistence(...)`, instead of maintaining separate ACP helper branches for message vs structured persistence.
- ACP callsites/tests are now converged on the unified seam for current known cases:
  - ACP tests no longer reference the old split helper names.
  - the remaining ACP-side synthetic structured test input now uses schema-valid `tool_result` directly instead of the legacy `tool_event` alias.

Decision rationale:
- stop micro-extracting the resume path,
- return to the next stronger Phase 5 boundary outside ACP resume wiring,
- prefer a broader runtime/persistence seam with higher non-ACP reuse value.

These should remain at the ACP edge:
- `ApiState` assembly,
- `RoleRuntimeBinding` selection,
- actual `continue_acp_turn_with_runtime(...)` invocation,
- adapter event/error framing.

Why this matters:
- the shared/core side now owns the generic persistence context constructor,
- ACP remains responsible only for adapting ACP session/request fields into that generic constructor,
- shared/core now also owns more of the transport-neutral assistant-output / turn-outcome / tool-result / tool-request persistence glue,
- the next reuse win is the settlement bookkeeping boundary between the coordinator and ACP stream orchestration,
- after settlement, the next strongest reusable boundary is continuation-request preparation before provider/runtime-specific streaming resumes,
- future non-ACP runners can reuse the same core constructor and helper layer without importing ACP event policy.

---

### Reuse-readiness assessment after current Phase 5 cleanup

After the unified gateway persistence convergence, the next concrete non-ACP reuse target looks clearer.

#### Strongest immediate reuse target

**`review` transcript/event persistence** appears to be the best next consumer of the extracted seam because it likely needs:
- canonical visible message persistence,
- workflow-event persistence,
- transport-neutral structured record normalization,
- and less ACP-style continuation/tool-routing complexity than `watch`.

Why `review` first:
- its likely persistence needs are closer to the now-cleaned message/workflow seam,
- it should benefit from the extracted canonical constructors and gateway-record normalization without needing ACP SSE framing,
- it gives a better test of whether the current core seam is truly transport-neutral before tackling `watch` ingestion/dedup specifics.

#### Concrete next reuse slice

The next broader migration slice should therefore be:

- audit the `review` runtime entrypoint(s) for transcript/event writes,
- identify where they currently bypass or duplicate canonical conversation persistence,
- and adapt one `review` persistence path onto the shared `core/conversation_events.rs` seam without importing ACP adapter policy.

Status: **larger review lifecycle projection slice landed**
- `core/conversation_events.rs` now exposes both:
  - `spawn_persist_workflow_event(...)`
  - `spawn_persist_assistant_summary_message(...)`
- `core/memory_proposals.rs` now projects review lifecycle actions through a shared non-ACP helper that can emit:
  - canonical workflow events
  - canonical visible assistant-style summary messages
- The current non-ACP canonical review lifecycle now covers:
  - `memory_proposal_created`
  - `memory_proposal_resolved`
  - visible summary projection for major lifecycle states (requested / approved / rejected / deferred / retained_local / superseded / needs_human_review)
- Proposal resolution carries richer outcome projection fields when available:
  - `result_path`
  - `result_commit`
  - alongside reviewer/decision metadata
- The curate core-apply flow now threads real artifact outcome data into proposal resolution:
  - `den.memory.apply_core_update` passes the returned MemFS `path` and `canonical_tip` into `resolve_for_bear(...)`
  - resulting `memory_proposal_resolved` canonical events now carry actual result artifact metadata for that apply path.
- `den.memory.request_review` now includes conversation/session/request/runtime provenance in `source_refs`, so more proposal-created lifecycle projection can attach to canonical conversations.
- This gives the pair-reflection → review/memory-proposal flow both a Den-owned canonical workflow trail and operator-visible summary projection outside ACP stream persistence.

#### Next likely follow-on after proposal lifecycle events

The next broader non-ACP slice should likely move from current review lifecycle projection into **deeper operator-facing audit coverage**, for example:
- projecting memory/core apply results into even richer visible summaries or dedicated review transcript surfaces,
- adding focused tests around non-ACP visible summary persistence for proposal lifecycle transitions,
- or taking the same shared projection pattern into another non-ACP subsystem before tackling `watch`-specific ingestion concerns.

At this point, the review/memory-proposal flow has both workflow-event and visible-summary projection; the best next leverage is either deeper test coverage for this slice or extending the same pattern to the next non-ACP consumer.

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

### Workflow/diagnostic persistence policy

Until a stronger product requirement says otherwise, we should treat these surfaces as belonging to one of two buckets:

#### Canonical transcript/event records

These should be persisted as Den-owned canonical records because they describe transcript-visible content, durable tool/workflow provenance, or terminal turn state that matters for history reconstruction and migration correctness:

- visible user prompts,
- visible assistant final output,
- tool request records,
- tool result records,
- conversation resolution provenance,
- terminal turn outcomes.

#### Intentionally ephemeral or session-scoped surfaces

These should remain non-canonical unless a future read/replay requirement proves otherwise:

- status/reasoning text,
- mode-update UI state,
- session-info/title sync events,
- plan-update/workboard projections,
- plan-approval fallback UI payloads.

These surfaces are useful for ACP UX and operator context, but they do not currently define the durable transcript contract.

#### Error policy

- Errors that terminate a turn and materially affect transcript/run outcome should be represented through canonical terminal outcome persistence.
- Non-terminal adapter/runtime errors may remain stream-visible but non-canonical by default unless we identify a concrete replay, audit, or read-switch requirement for them.
- If a non-terminal error later proves necessary for durable auditability, it should be added as a specific canonical structured-event class rather than promoted incidentally through generic UI/error payloads.

### Validation/readiness audit

- **Fast unit/lib coverage is strongest for:**
  - canonical helper construction,
  - prompt-path provenance payload shape,
  - assistant-output provenance,
  - conversation-resolved metadata shape,
  - turn-outcome provenance,
  - tool request/result helper semantics,
  - timeout/error tool-result metadata shape,
  - canonical dedup key serialization.
- **Smoke-stack/release-confidence coverage is strongest for:**
  - migrated schema presence,
  - live Postgres uniqueness enforcement,
  - stack-level persistence contract sanity.
- **Highest-value remaining validation gaps before more coding:**
  1. stronger end-to-end-style tests for prompt-path and `conversation_resolved` persistence behavior,
  2. smoke/integration confirmation that replay and continuation semantics hold against the full stack, not just the fast unit/lib layer.

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

### Replay/continuation validation results

- Existing smoke scripts do **not** currently include a dedicated ACP replay/continuation stack scenario.
- The strongest available practical validation today is a combination of:
  - focused ACP lib tests for replay settlement behavior,
  - focused ACP lib tests for continuation mapping/waiting behavior,
  - broader ACP lib test sweep,
  - existing smoke-stack validation for schema + persistence contract sanity.
- Concretely re-run during this slice:
  - `cargo test --lib --manifest-path /workspace/services/den/Cargo.toml acp_stream_waits_for_tool_result_and_continues_runtime`
  - `cargo test --lib --manifest-path /workspace/services/den/Cargo.toml acp_tool_result_endpoint_treats_replayed_identical_result_as_idempotent`
  - `cargo test --lib --manifest-path /workspace/services/den/Cargo.toml acp_tool_result_endpoint_marks_changed_replay_as_conflict`
- Result: all targeted validations passed.
- Current confidence statement:
  - replay settlement semantics are strong at the fast-test layer,
  - continuation wiring/waiting semantics are strong at the fast-test layer,
  - but there is still **no bespoke smoke-stack scenario** proving replay + continuation together against the live stack.

### Remaining work before this migration slice is complete

1. **Keep the workflow/diagnostic persistence policy aligned with implementation**
   - canonical vs ephemeral policy is now documented,
   - future changes should either follow that policy or update it intentionally.

2. **Strengthen end-to-end coverage for already-wired canonical paths**
   - prompt-path persistence through the live stack,
   - `conversation_resolved` persistence through the live stack.
   - Fast-path read eligibility is now better protected by canonical history mapping tests, including:
     - Den-owned prompt+assistant transcript ordering,
     - diagnostic-only canonical rows staying invisible,
     - Letta fallback filling visible history when canonical rows are absent,
     - current Den-first behavior when canonical history is partial (user-only or assistant-only visible rows).
   - The practical current policy is now explicit in the ACP history handler: canonical history is preferred only when canonical rows produce at least one visible transcript message; diagnostic-only-only canonical pages fall back to Letta/runtime history.
   - Stack-level proof is still desirable.

3. **Add bespoke smoke-stack coverage for replay + continuation if release confidence requires it**
   - current fast-test confidence is strong,
   - but no dedicated stack-level scenario exists yet.
   - Current smoke audit result:
     - `scripts/smoke-stack.sh` builds the local stack, seeds data, and delegates to `scripts/smoke.sh`.
     - `scripts/smoke.sh` runs `tests/smoke/test_stack.py` inside the bundled runner container.
     - `tests/smoke/test_stack.py` currently covers health checks, ACP auth gating, a prompt/conversation boundary scenario against Letta history, and now a dedicated ACP tool-result replay/idempotency scenario.
   - Bespoke replay scenario now added:
     1. create an ACP session/prompt that tries to trigger a known tool request,
     2. capture the emitted `tool_call_id` when the runtime actually enters the tool path,
     3. POST the matching tool result once and assert accepted continuation behavior,
     4. poll ACP conversation history and assert downstream assistant output appears after resumed continuation,
     5. POST the same tool result again and assert idempotent replay acceptance (`duplicate_result_ignored` / `already_settled`).
   - Current limitation: live providers may still sometimes satisfy the prompt without taking the tool path; in those runs the smoke test records successful resolution and skips replay-path assertions instead of failing the whole stack.

4. **Keep smoke-stack integration validation as the release-confidence layer**
   - verify migrated schema presence,
   - verify live duplicate suppression/uniqueness behavior,
   - verify stack wiring and seeded runtime behavior before release/cutover.

### Highest-value next slice after Phase 5 seam work

After completing the ACP persistence adapter seam, the highest-value remaining slice is **validation tightening**, not another extraction.

Recommended next priority:
1. **Strengthen end-to-end-style coverage for already-wired canonical paths**
   - especially prompt-path persistence,
   - and `conversation_resolved` persistence.
2. **Then consider bespoke smoke-stack replay/continuation coverage** if release confidence still needs a live-stack proof.

Status: **validation tightening progressing**
- Added focused ACP canonical validation for ACP-session provenance helper behavior.
- Added focused validation that `conversation_resolved` canonical records preserve ACP-session provenance and conversation id metadata through the helper path.
- Added focused prompt-path validation confirming the canonical user-prompt record shape matches the metadata assembled in `prompt_flow.rs` (`event`, `source`, `scope_id`, `role`, `acp_session_id`, `client`, `request_id`).
- Canonical ACP test sweep now passes with the added validation coverage.

Why this should be next:
- the roadmap now shows good helper/unit coverage and materially improved seam boundaries,
- the biggest remaining risk is confidence on already-wired behavior through the full stack,
- additional micro-refactors are lower value than proving the migrated paths behave correctly in integrated execution.

Concrete implementation entry point:
- start with the strongest fast feedback path for missing confidence:
  - add/strengthen focused tests around prompt-path and `conversation_resolved` persistence behavior,
  - then evaluate whether smoke-stack scripting needs a dedicated replay/continuation scenario.

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
