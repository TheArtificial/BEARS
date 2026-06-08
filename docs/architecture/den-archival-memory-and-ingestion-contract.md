# Den Archival Memory and Ingestion Contract

> **Note (2026-06).** This replaces Letta archival memory: semantic recall is Den-owned over per-Bear SQLite canonical sources ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)), not Letta Archives. See [Den-Native Runtime](den-native-runtime.md) ([migration plan](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md)).

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
   - derived search structures such as embeddings/vector indexes or other retrieval-friendly materializations.

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

Transcript history should not automatically become archival memory. Any transcript-derived archival source should be an explicit Den-owned decision.

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

- query scope filters,
- provenance returned with matches,
- ranking behavior where relevant,
- and rules for when retrieval results may be injected into runtime prompt assembly.

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
- source-to-chunk provenance,
- update/delete semantics,
- explicit query-time retrieval boundaries,
- and a clear separation between canonical sources and derived indexes.
