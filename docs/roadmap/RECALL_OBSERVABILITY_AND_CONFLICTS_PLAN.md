# Recall Observability and Read-Time Conflicts Plan

## Status

In progress (2026-07-31). Implements two 2026-07-30 amendments:

- **Part A (Phases A1+A2) landed** (commit `afda4b93`): `recall_watermark` in `den-service/src/recall/watermark.rs` (indexable heads vs live registry passages; run stats from the `recall_index` lane; `None` when Qdrant unconfigured), surfaced via the `memory_status` payload's `recall` object, the admin memory dashboard, and `recall_watermark_for_bear` / `is_healthy()`. Phase A3 (turn annotation) deferred until after Part B's rendering conventions.
- **Part B open**: conflict predicate, `conflicting` marker, `memory_conflict` observations.

- [ADR-0038 §8 — Recall consistency watermark](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md) ("is this Bear fully recallable right now?" must have a definite, visible answer),
- [ADR-0041 §8 — Read-time contradiction surfacing](../decisions/adr-0041-archival-recall-and-async-curation.md) (conflicting live records are surfaced, never silently ranked away).

Both are read-side consumers of existing machinery; **no new canonical schema**.

## Current state (audited 2026-07-30)

- **No watermark exists.** Indexing idempotence is per-record `content_hash` comparison against the `recall_passages` registry (`services/den/migrations/20260614190000_recall_passages.up.sql`, `den-service/src/recall/registry.rs`); nothing answers "how far behind is the index?". The canonical high-water function exists on the SQLite side (`memory_sequence_high_water`, `den-memory/src/records.rs`), and the indexable-head predicate exists in `den-service/src/recall/reconcile.rs` (`list_indexable_heads`).
- **`memory_status` is minimal**: `{configured, available, storage, role, file_count}` (`den-memory/src/tools.rs::sqlite_memory_status`), orchestrated in `den-core/src/tools/memory/mod.rs`.
- **Recall assembly**: turn-start recall in `den-runtime/src/agent_loop/assembler.rs` (best-effort `## Recalled memory` around `recall_for_turn`), hybrid merge in `den-service/src/recall/query.rs::hybrid_memory_search`, rendering in `render_recall_block`.
- **Temporal fields are live**: `valid_from` / `invalid_at` / `supersedes_memory_id` in `den-memory/src/schema.sql`, read by the temporal leg (`recall/temporal.rs`, effective time = `COALESCE(valid_from, created_at)`), written with supersession back-fill in `records.rs::append_memory_record`.
- **Observations**: `den-memory/src/observations.rs::create_memory_observation` with a status enum (PendingReview/ReviewQueued); observation kinds are free-form strings — no schema change needed for a new kind.
- **Reflection run history** (last successful/failed `recall_index` runs) lives in Postgres `bear_reflection_runs` (`den-runtime/src/reflection/conductor.rs`).

## Part A — Recall consistency watermark (ADR-0038 §8)

### Phase A1 — Watermark computation

Add a `recall_watermark(bear_id)` function in `den-service/src/recall/` returning:

- `canonical_seq` — `memory_sequence_high_water` from the Bear's SQLite,
- `indexed_seq` — the highest `sequence_no` such that every **indexable head** (reuse `list_indexable_heads`'s predicate) at or below it has a live, `content_hash`-matching, non-deleted registry entry,
- `lag_count` — indexable heads above `indexed_seq` not yet indexed,
- `last_success_at` / `failed_run_count` — from `bear_reflection_runs` for the `recall_index` lane.

Non-indexable records advance the watermark without indexing work (per the ADR). Computation is a join of registry state against canonical heads — derived, never stored as truth. Cache briefly (per-turn or few-seconds TTL) since admin and turn paths may both ask.

When Qdrant is not configured, report the watermark surface as `unavailable` (recall is keyword-only by design), not as infinite lag.

### Phase A2 — Surfacing

1. **`memory_status` tool**: extend `sqlite_memory_status` output with the watermark fields so the model can answer "what can you see?" truthfully. Descriptor/schema updates in `den-core/src/tools/descriptor/mod.rs`.
2. **Admin recall diagnostics**: same fields per Bear plus reindex-job progress, in the existing admin recall views (`den-web`).
3. **Health check**: a per-Bear lag/failure check exposed where operational checks live (admin hub stats now; a `den doctor`-style CLI when one exists). Threshold configurable, default generous (indexing is eventually consistent by design).

### Phase A3 — Turn annotation (optional, last)

When `lag_count > 0`, `render_recall_block` appends one line — "recall index N records behind" — so neither model nor user mistakes degraded recall for absent memory. Keep it one line; the full picture lives in `memory_status`.

## Part B — Read-time contradiction surfacing (ADR-0041 §8)

### Phase B1 — Conflict predicate

Add a pure function (in `den-service/src/recall/` or `den-memory`) over a set of retrieved records:

Two records **conflict** iff they share a `logical_path` **or** a primary `subject` entity ([ADR-0042](../decisions/adr-0042-memory-entity-relationships-and-bear-entity-layer.md)), their validity windows overlap (`COALESCE(valid_from, created_at)` .. `invalid_at`), both are live heads, and neither supersedes the other (directly or via chain).

Bounded and hot-path-cheap: evaluated only over the records already retrieved for the turn (post-merge in `hybrid_memory_search` / `recall_for_turn`), never a corpus scan.

### Phase B2 — Surfacing in recall output

- `render_recall_block` marks conflicting passages explicitly (`conflicting` marker naming both records) instead of letting ranking pick a silent winner; both records are included even if one would otherwise be cut by ranking.
- `memory_search` tool results carry the same marker.
- Add conflict presence to `recalled_memory_session_diagnostic` in the assembler.

### Phase B3 — `memory_conflict` observations

On detection, emit a `memory_conflict` observation via `create_memory_observation`:

- kind string `memory_conflict` (free-form kinds — no migration),
- **idempotent per unordered record pair**: key on sorted `(memory_id_a, memory_id_b)` stored in observation metadata; check for an existing non-resolved observation with the same pair before insert,
- flows into the existing curate review queue; resolution is the standard consolidation path (supersede one side, merge, or record the disagreement). Detection creates work items; it never resolves.

Sleep-time consolidation may reuse the Phase B1 predicate for corpus-wide sweeps later — explicitly out of scope here.

## Verification

- **Watermark**: unit tests over fixture registries (fully indexed, lagging, failed-job, non-indexable-only records ⇒ watermark advances); DB-backed integration test in `services/den/tests/` gated on `DATABASE_URL` + `QDRANT_URL` per repo convention.
- **Conflicts**: fixture pairs — divergent heads on one path (conflict), superseded chain (no conflict), disjoint validity windows (no conflict), shared subject entity (conflict); idempotency test for repeated detection of the same pair; assembler snapshot test showing the `conflicting` marker.
- `memory_status` schema/descriptor round-trip tests.

## Sequencing

A1 → A2 ship together (computation without surfacing has no value). B1 → B2 next; B3 with it or immediately after. A3 last — it depends on A1 and benefits from B2's rendering conventions.

## Non-goals

- No stored watermark table (derived only).
- No corpus-wide conflict sweeps on the hot path.
- No automatic conflict resolution — curate and humans resolve; recall only reports.
