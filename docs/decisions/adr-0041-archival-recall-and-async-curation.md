# ADR-0041 — Archival Recall and Asynchronous Curation

**Status:** Accepted (2026-06-13)
**Deciders:** Hans
**Related:**
- [ADR-0031 — SQLite-first canonical store](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)
- [ADR-0038 — Platform embedding standard and derived recall index](adr-0038-platform-embedding-standard-and-derived-recall-index.md)
- [ADR-0021 — Semantic bear memory](adr-0021-semantic-bear-memory.md)
- [ADR-0018 — Reflection system](adr-0018-reflection-system.md)
- [ADR-0032 — Den context compaction architecture](adr-0032-den-context-compaction-architecture.md)
- [ADR-0015 — Multi-user memory](adr-0015-multi-user-memory.md)
- [Den archival memory and ingestion contract](../architecture/den-archival-memory-and-ingestion-contract.md)
- [Reflection run taxonomy](../architecture/reflection-run-taxonomy.md)
- [Memory curation plan](../roadmap/MEMORY_CURATION_PLAN.md)

## Context

Retiring Letta removed two things the native runtime has not yet replaced:

1. **Archival memory** — Letta's vector-searchable store of overflow facts/passages, used for fuzzy long-term recall beyond in-context memory blocks.
2. **An engine that fills it** — Letta's agent paged information into archival memory via tools; on the native runtime nothing distills sessions into durable, recallable knowledge.

Today (native runtime): canonical memory is per-Bear SQLite `memory_records` ([ADR-0031](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)); `memory_search` is a SQL `LIKE` scan; the derived Qdrant recall index ([ADR-0038](adr-0038-platform-embedding-standard-and-derived-recall-index.md)) is accepted but unimplemented; pair reflection emits a **deterministic** summary of the last ~20 messages rather than extracting durable memories; `supersedes_memory_id` exists in the schema but is never written; and nothing proactively mines closed sessions for memory candidates.

We surveyed leading agent-memory systems (2025–2026): **Letta/MemGPT** (sleep-time compute: a background agent reorganizes memory off the hot path), **mem0 / MenteDB / Hindsight** (extraction-first capture: distill atomic facts instead of indexing raw messages; consolidate with dedup + conflict resolution; track *how knowledge evolved*), **Zep/Graphiti** (bi-temporal validity windows and edge invalidation instead of overwrite; hybrid vector+keyword+graph retrieval), and the **Generative Agents** lineage (per-item importance/salience; reflection triggered by cumulative importance; retrieval scored by `recency × relevance × importance`).

This ADR decides how those ideas are realized **without breaking the bears invariants**: SQLite stays canonical, Qdrant stays derived/ephemeral, curation is a bounded Reflection responsibility (not "governance"), and `work` never reads raw `pair/`.

## Decision

### 1. "Archival memory" is the derived recall index, not a new store

There is **no new canonical "archival" tier**. The Letta-archival replacement is:

- **canonical** durable knowledge in SQLite `memory_records` (`core/` + role branches), plus
- the **derived** Qdrant recall index ([ADR-0038](adr-0038-platform-embedding-standard-and-derived-recall-index.md)) and its Postgres passage registry over canonical sources.

What makes something archival-worthy is that **curation extracted and retained it**, not that a transcript scrolled out of context. This avoids the noisy-index failure mode (raw chat dumped into a vector DB) that pushed mem0/Zep/Letta toward extraction-first.

### 2. Three memory layers (canonical / source vs. derived)

| Layer | Contents | Store | Status |
|-------|----------|-------|--------|
| **Episodic** (source material) | conversation transcripts, compaction artifacts, `memory_observations` (watch), raw `pair/summaries` | Postgres transcripts + per-Bear SQLite | source, **not** indexed by default |
| **Semantic** (durable knowledge) | extracted, deduped, conflict-resolved assertions: `note`, `decision`, `reflection`, `summary` in `core/` + role branches | per-Bear SQLite `memory_records` | **canonical** |
| **Recall** (derived) | passages/vectors over the semantic layer (+ policy-gated episodic subset) | Qdrant + Postgres passage registry | **derived, ephemeral, rebuildable** |

The episodic → semantic boundary is crossed **only** by an explicit curation decision. Transcripts never auto-promote to archival memory (consistent with the [archival contract](../architecture/den-archival-memory-and-ingestion-contract.md) and ADR-0038).

### 3. Curation is the sleep-time engine, with three jobs

Curation (`curate` role) runs **off the hot path** as bounded Reflection runs ([ADR-0018](adr-0018-reflection-system.md)), analogous to Letta sleep-time compute. The live turn never blocks on memory management. It does three jobs:

1. **Proposal review** (exists): resolve pending `memory_proposals`.
2. **Archive harvest** (new — the "proactively review session archives" requirement): scan un-mined closed sessions / compaction artifacts via an **extraction-first** pass that distills durable facts/decisions/preferences/lessons into candidate semantic entries → `memory_proposals`. Conversational filler is discarded.
3. **Consolidation** (new): dedupe candidates against existing canonical memory; on contradiction, **supersede** rather than overwrite, encoding the transition; synthesize higher-level `reflection` records when cumulative salience crosses a threshold; promote low-risk results to `core/`.

Harvest is delivered as a new Reflection lane, **`archive_harvest`** ([reflection-run-taxonomy.md](../architecture/reflection-run-taxonomy.md)), triggered event-driven (session close, cumulative-salience threshold) plus a throttled/adaptive heartbeat — never a fixed cron. Curation stays bounded, audited, risk-classed, and human-overrideable; `requires_human` / high-sensitivity items escalate rather than auto-applying.

### 4. Consolidation by supersession + temporal narrative (no overwrite)

Contradictions are resolved by writing a **new** record that sets `supersedes_memory_id` on its predecessor and encodes the change in content ("previously X; now Y"), preserving history. The head record is the current valid assertion; the superseded chain is the audit trail. This finally wires the existing-but-unused `supersedes_memory_id` and is the bears-native equivalent of Zep's edge invalidation / Hindsight's temporal narrative — **without** adopting a graph database.

### 5. Importance/salience is first-class on durable memory

`salience` becomes a first-class scalar on `memory_records` (not only `memory_observations`). It is used to (a) trigger reflection/consolidation when cumulative salience crosses a threshold and (b) rank recall. This is the single signal that made Generative Agents' retrieval beat pure cosine similarity, and it is cheap.

### 6. Recall is hybrid and scored

In-loop retrieval merges three lanes — deterministic **anchors** (key memory projection), **vector** (Qdrant), and **keyword** (`LIKE`) — ranked by `recency × relevance × importance`, ACL-filtered per multi-user identity ([ADR-0015](adr-0015-multi-user-memory.md)). It **degrades gracefully** to anchors + `LIKE` when Qdrant is unavailable; turns must not fail. This extends ADR-0038 §5 with the scoring policy.

### 7. Bi-temporal-lite, not a temporal knowledge graph

Adopt validity windows on `memory_records` (`valid_from` / `invalid_at` alongside the existing `created_at` transaction time) and lean on the existing `memory_links` table for **lightweight typed entity links** (people, work surfaces, domains, missions). This gives point-in-time recall and "how knowledge evolved" cheaply. A full Graphiti-style graph DB as canonical store is **explicitly rejected** — it conflicts with SQLite-canonical (ADR-0031). Revisit only if demonstrated multi-hop reasoning needs exceed what typed links provide.

### Schema deltas (sketch, for implementation)

Append-only and single-writer-per-Bear invariants are preserved; these are additive columns plus newly *used* existing columns.

```sql
-- memory_records additions
ALTER TABLE memory_records ADD COLUMN salience  TEXT NOT NULL DEFAULT 'normal'; -- low|normal|high|critical (or 1..10)
ALTER TABLE memory_records ADD COLUMN valid_from TEXT NULL;   -- "valid time" (event time); created_at remains transaction time
ALTER TABLE memory_records ADD COLUMN invalid_at TEXT NULL;   -- set when superseded/invalidated; history preserved
-- supersedes_memory_id: already present; begin WRITING it during consolidation.

-- harvest provenance: track which episodic sources have been mined, so harvest is idempotent
CREATE TABLE IF NOT EXISTS memory_harvest_marks (
    mark_id        TEXT PRIMARY KEY,
    bear_id        TEXT NOT NULL,
    sequence_no    INTEGER NOT NULL,
    source_kind    TEXT NOT NULL,   -- conversation | compaction_artifact | observation | pair_summary
    source_ref     TEXT NOT NULL,   -- conversation_id / artifact id / etc.
    source_hash    TEXT NULL,
    harvested_at   TEXT NOT NULL,
    run_id         TEXT NULL,       -- reflection_run_outcomes.run_id
    proposal_ids_json TEXT NOT NULL DEFAULT '[]'
);
```

The Qdrant passage registry and vector lifecycle remain as specified in ADR-0038 §3/§6; harvest produces canonical records, and `archive_index` indexes them — the two lanes stay separate.

## Consequences

**Positive**
- A concrete, bears-native replacement for Letta archival memory that adds no second canonical store.
- Sessions actually become durable memory (extraction-first harvest), addressing the biggest current gap.
- Memory stays clean over time via dedup + supersession instead of append-only accumulation.
- Point-in-time / "how it changed" recall without a graph DB.
- Memory management never blocks live turns (sleep-time posture).

**Negative / costs**
- Harvest and consolidation are LLM passes with real token cost; must be budgeted and throttled.
- **Hallucination propagation**: a bad extraction can become durable knowledge — mitigated by quality/confidence filtering at harvest, `requires_human` for sensitive items, and mandatory provenance back to source `conversation_messages`.
- Dedup must reconcile candidates against canonical SQLite (and the Qdrant registry) — content-hash + semantic dedup.
- Additive schema migration and new consolidation logic in `curate`.

**Supersedes / amends**
- Extends [ADR-0038](adr-0038-platform-embedding-standard-and-derived-recall-index.md) §5 with retrieval scoring (`recency × relevance × importance`) and graceful degradation.
- Updates the [archival contract](../architecture/den-archival-memory-and-ingestion-contract.md) and [reflection taxonomy](../architecture/reflection-run-taxonomy.md) (adds `archive_harvest`).
- Realizes the proactive-curation intent in the [memory curation plan](../roadmap/MEMORY_CURATION_PLAN.md).

## Naming

| Concept | Term |
|---------|------|
| Distilling durable memory from episodic sources | **harvest** (`archive_harvest` lane) |
| Dedup + conflict-resolve + synthesize into canonical | **consolidation** |
| Derived semantic retrieval (Letta-archival replacement) | **recall** (Qdrant index per ADR-0038) |
| Per-item importance signal | **salience** |
| Background, off-hot-path curation posture | **sleep-time** curation |

"Governance" remains reserved for runtime context management; memory review is **curation** ([ADR-0039](adr-0039-trust-profiles-and-governance-modes.md), `AGENTS.md`).

## Follow-ups (not decided here)

- Exact salience scale (`low|normal|high|critical` vs. 1–10) and the cumulative-salience reflection threshold.
- Extraction prompt/schema for harvest (what counts as a durable fact vs. filler) and quality-filter thresholds.
- Semantic-dedup strategy and similarity threshold for consolidation (and whether it reuses the recall index).
- Whether typed entity links (§7) are derived on demand or persisted, and their vocabulary.
- Sequencing lives in [Memory Automation Roadmap](../roadmap/MEMORY_AUTOMATION_ROADMAP.md) and the [Derived recall index plan](../roadmap/DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md).
