# Den Archival Memory and Ingestion Contract

This document defines the architectural contract for Den-owned archival recall and source ingestion.

Archival recall is a **derived retrieval layer** over canonical sources. It is not a second source of truth.

## Summary

- canonical sources remain in Bear cognition, transcript-derived approved records, and approved linked corpora
- derived passages and vector indexes support recall and retrieval
- provenance, update, and deletion semantics are Den-owned
- retrieval is distinct from prompt memory, transcript history, and shared memory

## Layers

Den should distinguish these layers explicitly:

1. canonical source material
2. archival/recall objects
3. derived passages or chunks
4. retrieval indexes
5. query-time retrieval results

These layers are related but not interchangeable.

## Canonical source material

Examples of canonical sources:

- shared Bear memory
- approved role-local material when policy allows
- work-surface canonical docs
- approved Cabinet-linked sources
- imported external documents with explicit provenance
- explicitly approved transcript-derived material

## Derived recall

Derived recall exists to make canonical material retrievable.

It may include:

- chunked passages
- embeddings and vector indexes
- ranking/filter metadata
- provenance back to canonical source objects

Canonical truth remains outside the retrieval index.

## Invariants

### 1. Provenance is mandatory

Every archival object and every derived chunk must point back to a canonical source and version or content hash.

### 2. Den owns lifecycle semantics

When a source changes, Den must know how to refresh, supersede, or delete derived material.

### 3. Retrieval is not prompt memory

Prompt memory is standing in-context state. Retrieval is query-time recall.

### 4. Retrieval is not transcript history

Transcript is session truth. Retrieval is a derived access path over approved source material.

## Ingestion lifecycle

At minimum, Den ingestion should support:

1. source registration
2. source normalization and provenance capture
3. chunk derivation
4. index materialization
5. source update detection
6. supersede/delete handling
7. query-time retrieval with provenance

## Query semantics

Den should define:

- scope and ACL filtering
- ranking behavior
- provenance returned with matches
- rules for when retrieval results can enter runtime context

Retrieval should be work-surface-aware when appropriate, not Bear-global by default.

## Relationship to other memory systems

### Durable memory

Durable memory is canonical Bear knowledge.

### Prompt memory

Prompt memory is editable in-context state selected by policy.

### Transcript

Transcript is canonical interaction history.

### Compaction artifacts

Compaction artifacts preserve bounded continuity for conversations. They are not general recall indexes.

## Minimum expectations

An acceptable architecture must provide:

- Den-owned source registration
- source-to-chunk provenance
- content-version or hash-aware refresh logic
- explicit retrieval boundaries and ACLs
- clear separation between canonical source material and derived recall

## Related docs

- [memory model](memory-model.md)
- [den-native-runtime](den-native-runtime.md)
- [den-prompt-memory-block-contract](den-prompt-memory-block-contract.md)
