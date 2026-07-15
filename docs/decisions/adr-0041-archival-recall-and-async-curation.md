# ADR-0041 — Archival Recall and Asynchronous Curation

**Status:** Accepted (2026-06-13); amended (2026-07-15)
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

Implementation status (2026-07): canonical memory is per-Bear SQLite `memory_records` ([ADR-0031](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)); `memory_search` is hybrid SQL/Qdrant/graph/temporal when recall is configured; `salience`, `valid_from`, `invalid_at`, lifecycle/freshness indicators, and `memory_harvest_marks` exist in the store; archive-harvest creates human-review proposals with provenance; and reviewed core promotion writes `supersedes_memory_id` while invalidating predecessors. Remaining gaps are richer model-assisted extraction, semantic dedup, synthesis, and product UI surfacing for review queues.

We surveyed leading agent-memory systems (2025–2026): **Letta/MemGPT** (sleep-time compute: a background agent reorganizes memory off the hot path), **mem0 / MenteDB / Hindsight** (extraction-first capture: distill atomic facts instead of indexing raw messages; consolidate with dedup + conflict resolution; track *how knowledge evolved*), **Zep/Graphiti** (bi-temporal validity windows and edge invalidation instead of overwrite; hybrid vector+keyword+graph retrieval), and the **Generative Agents** lineage (per-item importance/salience; reflection triggered by cumulative importance; retrieval scored by `recency × relevance × importance`).

This ADR decides how those ideas are realized **without breaking the bears invariants**: SQLite stays canonical, Qdrant stays derived/ephemeral, curation is a bounded Reflection responsibility (not "governance"), and `work` never reads raw `pair/`.

## Decision

### 0. 2026-07 amendment: compaction may schedule harvest, but must not define memory

Den originally aligned archive harvest with context compaction because compaction is a useful lifecycle signal: it identifies long-lived or closed session material, produces provenance-bounded summaries, and helps curation avoid rereading an unbounded transcript every time. That alignment remains useful for **scheduling, source selection, and retrieval hints**.

It is not a semantic contract. Compaction artifacts optimize for **continuing a session under a context budget**; durable memory optimizes for **future reusable truth**. Therefore:

- compaction artifacts are episodic source material, never memory-shaped truth;
- compaction summary buckets must not be promoted directly into `memory_proposals`;
- archive harvest may use compaction artifacts to choose candidate source spans, but the extraction pass must be memory-specific and evidence-backed;
- when a claim matters, harvest should prefer canonical transcript/message evidence over assistant-written continuation summaries;
- if only a compaction bucket supports a candidate and no durable semantic claim can be stated, harvest emits no proposal;
- task state, workflow refs, unresolved follow-ups, and artifact refs are not memory candidates unless the extraction pass can state a durable preference, decision, constraint, fact, or lesson with provenance.

The intended relationship is:

> Compaction decides what context may be worth rereading. Reflection decides what is worth remembering.

This amendment supersedes any implementation reading of this ADR that treated compaction buckets as proposal categories. Bucket filtering is acceptable only as a temporary guardrail to reduce junk; the target architecture is an extraction-first memory pass over episodic sources with explicit discard reasons and positive acceptance tests for meaningful memory.

The useful coupling is operational, not semantic:

- **Triggering:** compaction/session-close events are cheap signals that a span has aged out of hot context and may be worth reflection.
- **Scoping:** compacted spans bound the transcript window so harvest does not scan every message on every run.
- **Indexing hints:** summary sections may hint that a decision, preference, constraint, or conflict occurred, but the extractor must verify against source evidence before proposing memory.
- **Watermarking and backpressure:** compaction/span identifiers provide idempotency, requeue, and "not yet reflected" bookkeeping.
- **Debuggability:** proposals can trace from semantic candidate → evidence messages/artifacts → compaction span/run that scheduled the harvest.

The rejected shortcut is:

```text
compaction bucket → memory proposal
```

The accepted pipeline is:

```text
compaction event/span → memory-specific extraction over evidence → proposal or discard reason
```

The first acceptance bar for the replacement is deliberately small: given a source span where the user clearly states one durable preference/decision/fact plus transient task residue, harvest must produce one future-useful semantic proposal with evidence and discard the residue. If that positive smoke test fails, the system has not demonstrated meaningful memory regardless of queue or recall plumbing.

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
2. **Archive harvest** (new — the "proactively review session archives" requirement): scan un-mined closed sessions using compaction artifacts as scheduling/source-selection hints, then run an **extraction-first** memory pass over episodic evidence that distills durable facts/decisions/preferences/lessons into candidate semantic entries → `memory_proposals`. Conversational filler and continuation-only compaction residue are discarded with reasons. The harvest result may be empty.
3. **Consolidation** (new): dedupe candidates against existing canonical memory; on contradiction, **supersede** rather than overwrite, encoding the transition; synthesize higher-level `reflection` records when cumulative salience crosses a threshold; promote low-risk results to `core/`.

Harvest is delivered as a new Reflection lane, **`archive_harvest`** ([reflection-run-taxonomy.md](../architecture/reflection-run-taxonomy.md)), triggered event-driven (session close, cumulative-salience threshold) plus a throttled/adaptive heartbeat — never a fixed cron. Curation stays bounded, audited, risk-classed, and human-overrideable; `requires_human` / high-sensitivity items escalate rather than auto-applying.

### 4. Consolidation by supersession + temporal narrative (no overwrite)

Contradictions are resolved by writing a **new** record that sets `supersedes_memory_id` on its predecessor and encodes the change in content ("previously X; now Y"), preserving history. The head record is the current valid assertion; the superseded chain is the audit trail. This finally wires the existing-but-unused `supersedes_memory_id` and is the bears-native equivalent of Zep's edge invalidation / Hindsight's temporal narrative — **without** adopting a graph database.

### 5. Importance/salience is first-class on durable memory

`salience` becomes a first-class scalar on `memory_records` (not only `memory_observations`). It is used to (a) trigger reflection/consolidation when cumulative salience crosses a threshold and (b) rank recall. This is the single signal that made Generative Agents' retrieval beat pure cosine similarity, and it is cheap.

**Freshness trend (derived companion signal).** Alongside the stored `salience` scalar, durable/consolidated records carry a *computed* trend — `stable | strengthening | weakening | stale` — derived from when supporting evidence arrived (supersession-chain activity, repeated harvest corroboration, `valid_from` spread). It is **derived** (recomputed/cached, not a canonical edit) and feeds both recall ranking (§6) and re-harvest/consolidation triggers: a `weakening`/`stale` high-salience belief is a prompt for `curate` to revisit it. Borrowed from Hindsight's observation freshness, but computed from our existing supersession + harvest provenance rather than a separate store.

### 6. Recall is hybrid and scored

In-loop retrieval merges complementary lanes — deterministic **anchors** (key memory projection), **vector** (Qdrant), **keyword** (`LIKE`), a **temporal** filter (validity-window / point-in-time over §7), and a **bounded graph** expansion (depth-capped, read-only traversal over the record↔entity bipartite links, [ADR-0042](adr-0042-memory-entity-relationships-and-bear-entity-layer.md)) — ranked by `recency × relevance × importance × freshness_trend`, filtered per entity audience/visibility ([ADR-0015](adr-0015-multi-user-memory.md), ADR-0042 access-bearing relations). The temporal and graph legs add **no new store**; they query canonical SQLite. Access-bearing gating (the `memory_access_rules` query) is applied to **every** lane's output before results return. It **degrades gracefully** to anchors + `LIKE` when Qdrant is unavailable; turns must not fail. This extends ADR-0038 §5 with the scoring policy; the leg mechanics live in [DERIVED_RECALL Phase 3.5](../roadmap/DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md).

### 7. Bi-temporal-lite, not a temporal knowledge graph

Adopt validity windows on `memory_records` (`valid_from` / `invalid_at` alongside the existing `created_at` transaction time) and lean on the entity relation layer ([ADR-0042](adr-0042-memory-entity-relationships-and-bear-entity-layer.md)) for **lightweight typed entity links** (people, work surfaces, domains, missions). This gives point-in-time recall and "how knowledge evolved" cheaply. Multi-hop recall, when needed, is served by **bounded retrieval-time traversal** over the record↔entity bipartite links (depth-capped, read-only, no stored transitive edges or inference — see §6, [DERIVED_RECALL Phase 3.5](../roadmap/DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md), and ADR-0042 anti-RDF guardrails), not by persisting a graph. A full Graphiti-style graph DB as canonical store is **explicitly rejected** — it conflicts with SQLite-canonical (ADR-0031).

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
- **False confidence from compaction summaries**: continuation summaries can sound authoritative while encoding assistant interpretation, task residue, or outdated state. Harvest must treat them as hints and require a memory-specific claim plus provenance before proposal creation.
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

"Governance" remains reserved for runtime context management; memory review is **curation** ([ADR-0039](adr-0039-trust-profiles-and-governance.md), `AGENTS.md`).

## Follow-ups (not decided here)

- Cumulative-salience reflection threshold and deeper freshness weighting. The stored salience scale is currently `low|normal|high|critical`.
- Extraction prompt/schema for harvest (what counts as a durable fact vs. filler), quality-filter thresholds, and golden positive tests proving that obvious user preferences/decisions produce meaningful proposals while assistant prose/task residue is discarded.
- Semantic-dedup strategy and similarity threshold for consolidation (and whether it reuses the recall index).
- Whether typed entity links (§7) are derived on demand or persisted, and their vocabulary.
- Sequencing lives in [Memory Automation Roadmap](../roadmap/MEMORY_AUTOMATION_ROADMAP.md) and the [Derived recall index plan](../roadmap/DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md).
