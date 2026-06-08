# Implementation Plan: Letta Migration

This plan implements the migration direction in [Letta Migration Plan](./letta-migration-plan.md). For **current progress**, see [Letta Migration Status](./LETTA_MIGRATION_STATUS.md).

The goal is to replace Letta as the active execution/control substrate with a Den-owned multi-role Bear runtime while preserving role semantics, transcript ownership, approval/cancellation safety, and operator visibility.

## Current focus (2026-06-08)

**Active epic:** Phase 6 follow-on — Curate execution substrate and `review`/`watch` migration prep.

**Recently completed:** [Epic A2 — Complete the ACP runtime contract boundary](#epic-a2--complete-the-acp-runtime-contract-boundary) (see [status doc](./LETTA_MIGRATION_STATUS.md)).

**Not active:** Further persistence micro-extractions; backfill/rollout controls; `chat`/`work` harness work.

**Detailed progress:** [LETTA_MIGRATION_STATUS.md](./LETTA_MIGRATION_STATUS.md)

---

## Related documents

| Document | Role |
| --- | --- |
| [LETTA_MIGRATION_STATUS.md](./LETTA_MIGRATION_STATUS.md) | Living progress tracker |
| [letta-migration-plan.md](./letta-migration-plan.md) | Strategic migration direction |
| [acp-runtime-contract.md](../architecture/acp-runtime-contract.md) | ACP runtime boundary contract (Phase 0) |
| [letta-dependency-matrix.md](../architecture/letta-dependency-matrix.md) | Dependency inventory |
| [den-migration-backfill-and-rollback-plan.md](./den-migration-backfill-and-rollback-plan.md) | Backfill/rollback planning baseline |
| [den-archival-memory-and-ingestion-contract.md](../architecture/den-archival-memory-and-ingestion-contract.md) | Archival/ingestion boundary |
| [den-tool-and-runtime-configuration-boundary.md](../architecture/den-tool-and-runtime-configuration-boundary.md) | Tool/runtime config boundary |
| Compaction docs (ADR-0032, contract, schema, guide) | Epic B design |

---

## Scope

This plan covers:

- Den-owned transcript and interaction source-of-truth completion
- Shared runtime/persistence seams extracted from ACP `pair`
- Non-blocking structured-event persistence
- Assistant final-message persistence correctness
- Canonical read-switch criteria
- Transcript-compaction dependency alignment
- Role-by-role substrate migration sequencing
- Migration/backfill mechanics
- Rollout/rollback controls

This plan does **not** fully specify archival retrieval replacement, every long-term non-ACP runtime detail, or complete `chat`/`work` harness replacement beyond what the Letta cutover directly depends on.

---

## Validation strategy

Two intentionally different regimes:

1. **Fast unit/library validation (default during development)**
   - Focused Rust unit/lib tests: canonical record construction, dedup keys, persistence decisions, replay handling, terminal-path semantics.
   - Primary loop: `cargo test --lib --manifest-path services/den/Cargo.toml`

2. **Smoke-stack integration validation (release/pre-cutover gate)**
   - Built services, migrations, docker-compose stack, seed flows, cross-service behavior.
   - Anchor: `scripts/smoke-stack.sh`

**Policy:**

- Unit/lib tests are required default proof for most code changes.
- Targeted live DB probes only when unit tests cannot establish schema assumptions.
- Smoke-stack is the release gate for persistence, migrations, transcript ownership, or cross-service runtime changes.
- Do not force every stack concern into the fast loop; do not substitute smoke for good unit coverage.

---

## Success criteria

The implementation is successful when:

- Den owns the canonical transcript contract for active migrated runtime surfaces.
- ACP `pair` relies on Den-owned transcript, event, approval, and terminal-outcome persistence independent of Letta history reads.
- Structured tool/workflow events and finalized assistant output persist without perturbing ACP stream ordering.
- Canonical reads prefer Den-owned conversation history for eligible sessions with explicit provenance and fallback.
- Shared runtime/persistence primitives from `pair` are sufficient to begin `review` and `watch` migration without re-embedding Letta semantics.
- Migration/backfill and rollback procedures are explicit enough to support staged cutover.

---

## Guiding constraints

- Transcript ownership stays ahead of execution-substrate replacement.
- Non-blocking persistence must not destabilize ACP stream semantics.
- Finalized assistant output must be persisted exactly once per terminal turn path.
- Tool, approval, and workflow spans remain auditable throughout migration.
- Den-owned control-plane state is conceptually primary; provider identifiers stay implementation details.
- Initial rollout favors correctness, auditability, and rollback over aggressive cutover speed.
- Build **Den-owned seams**, not a generalized provider marketplace. Ringfence Letta; do not replace it with a permanent arbitrary-provider platform.

---

## Phase 0 — Scope freeze and migration invariants

**Status:** substantially complete. See [acp-runtime-contract.md](../architecture/acp-runtime-contract.md) and [workflow/diagnostic persistence policy](#workflow-and-diagnostic-persistence-policy) below.

**Goal:** lock the implementation-facing migration contract before cutover logic spreads further.

### Tasks

1. Define implementation invariants: transcript source of truth, canonical visible message, structured diagnostic event, finalized assistant output, read-switch eligibility, provider-compatibility boundary, dual-write/fallback period.
2. Enumerate protected runtime behaviors: ACP stream ordering, tool continuation, approval pause/resume, cancellation hygiene, operator/debug provenance.
3. Define first migration surfaces: ACP `pair` transcript completion, shared runtime/persistence extraction, `review`/`watch` follow-on.
4. Define exclusions: full retrieval replacement, complete `chat`/`work` harness replacement (initial cut).

### Acceptance

- Implementation-facing migration contract exists (ACP runtime contract + policies in this plan).
- Protected parity behaviors are explicit and testable.
- First surfaces and exclusions are named.

---

## Phase 1 — Canonical transcript ownership (`pair`)

**Goal:** finish Den-owned source persistence for the ACP-visible transcript path.

### Tasks

1. Source-persist: user prompt, conversation resolved, tool request, assistant final output, turn outcome.
2. Close gaps: finalized assistant semantics, tool-result coverage, failure/cancellation edges, replay/retry safety.
3. Standardize persistence via Den-owned canonical record helpers.
4. Separate visible history from diagnostic-only records.

### Acceptance

- Normal ACP turn yields complete canonical transcript/event trail.
- Final assistant output persisted exactly once on terminal paths.
- Tool-result and workflow coverage explicit and tested.
- Visible-history reads cleanly separated from diagnostics.

---

## Phase 2 — Non-blocking structured-event persistence

**Goal:** structured persistence never perturbs ACP stream ordering.

### Tasks

1. Standardize structured-event persistence on non-blocking spawned paths.
2. Audit hot paths for inline writes that reorder or stall SSE.
3. Route tool request/result, conversation resolved, workflow transitions, terminal outcomes through the seam.
4. Observability for persistence failures without stream-fatality by default.

### Acceptance

- Structured persistence does not block SSE ordering.
- Failures observable and attributable.
- No unnecessary inline canonical DB writes on the hot path.

---

## Phase 3 — Idempotency and dedup contract

**Goal:** canonical persistence safe under retries, replay, and mixed-source periods.

### Tasks

1. Idempotency keys for assistant output, tool request/result, turn outcomes, conversation-resolved.
2. Authoritative identifiers: request id, tool call id, scope id + event type, provenance-derived `source_event_id`.
3. Tests: duplicate frames, repeated continuations, replay, multi-writer windows.
4. Document dedup policy for operators and migration tooling.

### Acceptance

- Safe under expected retry/replay.
- Duplicate visible rows prevented or explainable.
- Unit tests cover dedup logic; smoke-stack covers live uniqueness before cutover.

---

## Phase 4 — Canonical read-switch and mixed-origin history

**Goal:** history reads move from Letta compatibility sources toward Den-owned canonical conversations.

### Tasks

1. Eligibility criteria: source-persisted prompt, assistant output, tool/workflow events as needed, terminal outcome.
2. Mixed-origin debugability with visible provenance.
3. Fallback for old, partial, imported, or Letta-primary sessions.
4. ACP/admin/history paths switch by explicit criteria.

### Acceptance

- Eligible conversations read primarily from Den-owned history.
- Mixed-origin sessions explainable to operators.
- Read-switch policy explicit and reversible.

**Residual work:** startup/control surfaces (conversation selection, runtime binding) still Letta-backed — tracked under Epic A2.

---

## Phase 5 — Shared runtime/persistence seam extraction (`pair`)

**Goal:** turn the strongest migration path into reusable Den-native role-runner primitives.

### Tasks

1. Identify shared elements: turn lifecycle, terminal outcomes, event persistence, continuation, cancellation.
2. Extract interfaces/helpers only where real reuse exists.
3. Keep ACP-specific policy at the edge.
4. Document boundary: shared role-runner vs ACP actuator vs Letta compatibility shim.

### Acceptance

- Shared primitives exist without over-generalizing the provider layer.
- ACP logic separable from common role-runner behavior.
- `review`/`watch` can reuse seams.

### Migration concern map

| Concern | Target Den boundary | Maturity | Cutover risk |
| --- | --- | --- | --- |
| Conversation identity / control plane | Den canonical session identity; Letta materializes runtime conversations at execution only | Strong | Medium |
| Turn/run execution lifecycle | Den role-runner contract; Letta as temporary execution adapter | **Partial-strong** — contract + semantic events landed; execution still Letta | High |
| Canonical transcript vs runtime state | Den transcript store separated from ephemeral runtime state | Strong | High |
| Conversation compaction | Den compaction subsystem with artifacts, triggers, replay semantics | Strong (design + initial runtime) | Medium |
| Editable prompt memory blocks | Den block model + prompt compiler | Strong (initial slice) | Medium |
| Archival / recall memory | Den semantics over canonical memory + retrieval | Partial | Medium |
| Source ingestion / retrieval | Den pipeline over canonical sources | Weak (design) | Medium |
| Tool registry / execution | Den registry + policy; ACP mediation at edge | Partial | Medium |
| Role/runtime configuration | Den role profiles; provider bindings as detail | Partial | Medium |
| Admin / diagnostics | Den read models + mixed-origin provenance | Strong | Medium |
| Identity / attachments | Den auth + Bear/role attachment model | Weak | Medium |
| Background maintenance | Den workers / Reflection | Partial (Curate queue stub) | Medium |
| Migration / backfill | Den tooling + dual-write/read-switch controls | Weak (planning) | High |

---

## Phase 6 — `review` and `watch` follow-on

**Goal:** reduce Letta coupling on lower-complexity non-harness roles before `chat`/`work`.

### Tasks

1. Audit `review`/`watch` entrypoints and Letta dependencies.
2. Reuse Phase 5 transcript/event/run seams.
3. Role-specific parity: `review` — governance/audit; `watch` — ingestion latency, dedup, observation persistence.
4. Migrate transcript and run-state ownership before execution substrate.

### Acceptance

- Clear Den-owned migration path using shared seams.
- Letta coupling reduced without waiting for harness migration.
- Parity criteria explicit.

**Decision:** `review` is the first non-ACP consumer (see status doc for projection/worker progress).

---

## Phase 7 — Historical migration and backfill

**Goal:** operationally safe transition for recent and legacy sessions.

See [den-migration-backfill-and-rollback-plan.md](./den-migration-backfill-and-rollback-plan.md).

### Acceptance

- Backfill/import expectations explicit.
- Operators can pivot between Den and legacy identifiers.
- Historical data sufficient for audit/debug and staged cutover.

---

## Phase 8 — Cutover controls, rollout, and rollback

**Goal:** reversible, measurable migration at each expansion step.

### Tasks

1. Rollout controls: canonical-read preference, Den-native runtime enablement, role-by-role selection, compatibility fallback.
2. Cutover metrics: stream continuity, tool continuation, approval, cancellation, projection consistency, operator debuggability.
3. Rollback actions with bounded scope.
4. Bounded internal soak before broadening traffic.

### Acceptance

- Rollout and rollback via bounded controls.
- Read-switch and runtime-switch independently controllable.
- Cutover justified by explicit metrics.

---

## Phase 9 — `chat`/`work` harness replacement preparation

**Goal:** prepare highest-complexity Letta-backed surfaces after transcript/control-plane maturity.

### Acceptance

- Prerequisites explicit.
- Harness replacement sequenced after lower-risk roles.
- Remaining Letta dependency surface narrow and documented.

---

## Near-term epics

Epic status and landed files: [LETTA_MIGRATION_STATUS.md](./LETTA_MIGRATION_STATUS.md).

### Epic A — Turn/run execution lifecycle extraction

**Goal:** reduce Letta to a narrow execution adapter.

**Deliverables:**

1. Transport-neutral start/resume/cancel lifecycle helpers (reuse only where real).
2. Lifecycle boundary documented: Den-owned vs ACP-edge vs Letta adapter.
3. Lazy runtime materialization invariants (Den-first; provider conversation at execution boundary).
4. Transport-neutral cancellation and stale-run hygiene.

**Lifecycle boundary:**

| Layer | Owns |
| --- | --- |
| Den-owned | Session conversation selection, lazy materialization, turn lifecycle helpers, terminal/tool-settlement bookkeeping, cancellation contracts |
| ACP-edge | SSE framing, approval/status text, prompt assembly, UI projections |
| Letta adapter (temporary) | Provider conversation creation, streaming, continuation submission, cancellation calls |

**Acceptance:** Den owns lifecycle contract at shared-core level; ACP transport separable; `review`/`watch` can reuse without ACP-only behavior.

---

### Epic A2 — Complete the ACP runtime contract boundary

**Status:** complete for the planned slice set. Milestones from [acp-runtime-contract.md](../architecture/acp-runtime-contract.md).

**Goal:** complete the runtime contract so Letta is only an execution adapter, not the implicit lifecycle owner.

#### Slice 1 — Structured semantic streaming

- Start and continue paths emit `RuntimeSemanticEvent` consistently.
- Parsing behind adapter seam (`letta_runtime_stream_parser` → `bearwire_projection`).
- Contract tests for tool request, approval pause, turn complete/fail/cancel, conversation resolved.

#### Slice 2 — Conversation lifecycle contract

- `prompt_flow` / session lifecycle depend on `AcpConversationRuntime`.
- Lazy materialization centralized; Den-canonical id first.
- Regression tests: new session, resumed session, explicit selection, access checks.

#### Slice 3 — Cancellation and hygiene normalization

- Replace Letta error string matching with `RuntimeErrorCategory`.
- Generalize `preflight_hygiene` / `cancel_turn` behind contract.

#### Slice 4 — Validation gate (parallel; required before epic done)

- Automate idempotency uniqueness integration test.
- Stabilize smoke-stack tool-result replay scenario.
- Stack-level prompt-path + `conversation_resolved` checks.

**Acceptance:** ACP depends on contract types for streaming, conversation lifecycle, and cancellation; Letta confined to adapter implementation; release gate green.

---

### Epic B — Conversation compaction (design complete)

**Goal:** Den-owned compaction replacing Letta-implied context shrinking.

**Deliverables:** trigger policy, artifact model, replay/read semantics, operator visibility.

**Acceptance:** triggers explicit; artifacts auditable; compacted history explainable; Letta not hidden authority for context shrinking.

**Follow-on (B2+):** more execution paths, richer durable envelopes — see status doc.

---

### Epic C — Prompt memory blocks (initial slice complete)

**Goal:** Den-owned editable in-context memory and prompt compilation.

**Deliverables:** block types/scopes, mutation/audit semantics, compilation rules, boundary with transcript/archival/compaction.

**Acceptance:** blocks are Den-owned concept; compilation rules explicit; no hidden provider-shaped state.

**Follow-on:** richer mutation workflows, admin visibility — see status doc.

---

## Workflow and diagnostic persistence policy

Stable policy unless a product requirement explicitly changes it.

### Canonical transcript/event records

Persist as Den-owned canonical records:

- visible user prompts
- visible assistant final output
- tool request and tool result records
- conversation resolution provenance
- terminal turn outcomes

### Intentionally ephemeral or session-scoped

Remain non-canonical unless a future read/replay requirement proves otherwise:

- status/reasoning text
- mode-update UI state
- session-info/title sync events
- plan-update/workboard projections
- plan-approval fallback UI payloads

### Error policy

- Errors that terminate a turn → canonical terminal outcome persistence.
- Non-terminal adapter/runtime errors → stream-visible, non-canonical by default.
- Future durable non-terminal errors → specific canonical structured-event class, not incidental UI promotion.

---

## Decisions

| Topic | Decision |
| --- | --- |
| First non-ACP seam consumer | `review` before `watch` |
| Idempotency | Provenance-derived `source_event_id`; unique on `(conversation_id, source_event_id)` |
| Workflow projections beyond terminal outcomes | Not canonical unless product requirement changes |
| Non-terminal errors | Stream-only by default |
| `ApiState` / `RoleRuntimeBinding` extraction | Defer — ACP-owned until multi-runner factory exists |
| Curate worker | Queue/projection/scaffolding landed; real execution substrate is separate future work |
| Architecture destination | Monolithic Den with narrow Letta compatibility seam — not a provider marketplace |

---

## Deferred work

| Item | Defer until |
| --- | --- |
| More Phase 5 micro-extractions (resume diagnostics, etc.) | Epic A2 complete |
| Full Curate execution substrate | Epic A2 seams stable |
| Phase 7–8 backfill/rollout controls | Runtime boundary stable |
| `watch` migration | `review` reuse validated |
| `chat`/`work` harness | Phases 1–6 materially complete |

---

## Recommended implementation order

1. Phase 0 — scope freeze and invariants ✓
2. Phase 1 — canonical transcript ownership (`pair`) — mostly complete
3. Phase 2 — non-blocking structured-event persistence — mostly complete
4. Phase 3 — idempotency and dedup — mostly complete
5. Phase 4 — canonical read-switch — mostly complete; residual in Epic A2
6. Phase 5 — shared runtime/persistence extraction — in progress
7. **Epic A2 — complete ACP runtime contract** ← current
8. Phase 6 — `review`/`watch` follow-on
9. Phase 7 — historical migration and backfill
10. Phase 8 — rollout and rollback controls
11. Phase 9 — `chat`/`work` harness preparation

This ordering keeps transcript/control-plane ownership ahead of execution-substrate replacement and favors safe staged cutover over a broad hard migration.
