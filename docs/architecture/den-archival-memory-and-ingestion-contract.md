# Den Archival Memory and Ingestion Contract

> **Note (2026-06).** This replaces Letta archival memory: semantic recall is Den-owned over per-Bear SQLite canonical sources ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)) and Cabinet sources ([ADR-0008](../decisions/adr-0008-cabinet-reading-pipeline.md)), using a **derived Qdrant index** and **platform embedding standard** ([ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)). The engine that *fills* archival memory — extraction-first harvest of session archives plus consolidation by supersession — and the retrieval scoring policy are defined in [ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md). See [Den-Native Runtime](den-native-runtime.md) ([migration plan](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md)).

This document defines the implementation-facing contract for Den-owned archival/recall memory and source ingestion during the Letta migration.

## Purpose

Den needs a clear replacement for the responsibilities historically blurred together under Letta archival memory, passage ingestion, and retrieval.

This contract defines:

- what counts as canonical source material,
- what belongs in archival/recall memory,
- what belongs in derived retrieval indexes,
- and how source ingestion, updates, provenance, and deletion should work.

## Architectural stance

The target is Den ownership, not a generalized provider marketplace.

An end state with a monolithic Den that owns canonical source records, ingestion policy, retrieval semantics, and recall behavior is acceptable. External indexes or retrieval engines may exist as implementation details, but they should not define the conceptual model.

## Core separation of concerns

Den should distinguish these layers explicitly:

1. **Canonical source material**
   - the durable source of truth such as `core/` memory, role-local memory, Cabinet artifacts, transcript-derived records where explicitly allowed, repo/workspace docs, or external imported documents.

2. **Archival/recall memory**
   - Den-owned long-lived recall objects representing curated or indexed recallable knowledge, with provenance back to canonical sources.

3. **Derived passages/chunks**
   - chunked or transformed representations used for indexing and retrieval.

4. **Retrieval indexes**
   - derived search structures: **Qdrant** vector collections keyed by **embedding standard** ([ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)); passage metadata in Den Postgres; not canonical.

5. **Query-time retrieval results**
   - relevance-scored outputs returned to runtime prompt assembly or operator tooling.

These are not interchangeable.

## Invariants

### 1. Canonical source remains outside the retrieval index

Indexes, passages, and retrieval results are derived views over canonical sources. They are not the source of truth.

### 2. Provenance is mandatory

Every archival/recall item and every derived chunk should be traceable back to a canonical source identifier, location, or content version.

### 3. Update and delete semantics are Den-owned

When canonical source content changes or is removed, Den must know how indexed/chunked material is updated, superseded, or deleted.

### 4. Retrieval is separate from prompt blocks and transcript compaction

Archival retrieval is query-time recall. It is not the same thing as editable prompt blocks or derived compaction artifacts.

### 5. Archival memory is not silent provider state

Den must own the write policy, query semantics, provenance, and lifecycle rather than inheriting hidden provider-managed archival behavior.

## Canonical source classes

Initial source classes should include:

- curated shared `core/` memory,
- role-local memory where archival indexing is appropriate,
- work-surface canonical docs,
- Cabinet artifacts and mission material where approved,
- imported external documents with explicit provenance,
- and selected transcript-derived material only when explicitly allowed by policy.

Transcript history should not automatically become archival memory. Any transcript-derived archival source should be an explicit Den-owned decision. The explicit, Den-owned policy that crosses this boundary is the **`archive_harvest`** Reflection lane ([ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md)): it distills durable candidates from closed sessions into proposals, which `memory_curate` reviews before any canonical write.

## Filling archival memory: harvest and consolidation

Source registration and indexing (below) describe how canonical material becomes retrievable. **What becomes canonical in the first place** is decided by asynchronous curation ([ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md)), which runs off the live turn (sleep-time posture):

- **Harvest (`archive_harvest`)** — an extraction-first pass over un-mined session archives and compaction artifacts distills atomic facts, decisions, preferences, and lessons (discarding filler) into memory proposals. Idempotent via harvest marks; provenance back to source `conversation_messages` is mandatory.
- **Consolidation (`memory_curate`)** — candidates are deduped against existing canonical memory. Contradictions are resolved by **supersession** (write a new record, set `supersedes_memory_id`, encode the transition), never by overwrite; history is preserved. Low-risk results promote to `core/`; sensitive items escalate to human review.

Only after consolidation writes a canonical record does the source become eligible for indexing into the derived recall index. **Harvest produces canonical memory; `archive_index` indexes it — the two stay separate.**

## Ingestion lifecycle

At minimum, Den ingestion should support:

1. source registration,
2. source normalization and provenance capture,
3. chunking/passage derivation,
4. indexing/materialization into retrieval structures,
5. source update detection,
6. delete or supersede handling,
7. query-time retrieval with provenance-preserving results.

## Recommended source record requirements

A source record should preserve enough information to answer:

- what canonical source this came from,
- which Bear/role/work surface it belongs to,
- what version or content hash was indexed,
- when it was indexed,
- what derived chunks/passages were produced,
- and whether the source is still active, superseded, or deleted.

## Chunk/passage requirements

Derived chunks/passages should preserve:

- source reference,
- source version/hash,
- chunk ordering,
- chunk text or structured payload,
- metadata needed for filtering,
- and deletion/supersession linkage.

## Query semantics

Den should define query semantics explicitly:

- query scope filters (ACL by Bear membership and identity scope per [ADR-0015](../decisions/adr-0015-multi-user-memory.md)),
- provenance returned with matches,
- ranking behavior,
- and rules for when retrieval results may be injected into runtime prompt assembly.

**Ranking** is hybrid and scored ([ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md)): merge deterministic anchors (key memory projection), vector recall (Qdrant), and keyword (`LIKE`), ranked by `recency × relevance × importance` where importance derives from record `salience`. Retrieval **degrades gracefully** to anchors + `LIKE` when Qdrant is unavailable; turns must not fail.

Retrieval should be work-surface-aware where applicable rather than Bear-global by default.

## Relationship to other memory systems

### Versus durable memory

Durable memory is the canonical long-lived knowledge layer. Archival retrieval is a derived recall mechanism over selected canonical sources.

### Versus prompt blocks

Prompt blocks are standing in-context inputs selected by policy. Retrieval is on-demand recall selected by query.

### Versus transcript history

Transcript is canonical session history. Retrieval may index selected transcript-derived content only when policy explicitly permits it.

### Versus compaction artifacts

Compaction artifacts preserve continuity within long-running sessions. Retrieval serves broader recall over selected source material.

## Minimum v1 expectations

A v1 archival/ingestion replacement is acceptable if it provides:

- Den-owned source registration,
- source-to-chunk provenance with **content-hash dedup**,
- update/delete semantics,
- explicit query-time retrieval boundaries,
- **platform embedding standard** (`bears-embed-v1`) shared across Bear memory and Cabinet indexing,
- Qdrant derived vectors (rebuildable from canonical sources),
- and a clear separation between canonical sources and derived indexes.

See [Derived recall index implementation plan](../roadmap/DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md).
