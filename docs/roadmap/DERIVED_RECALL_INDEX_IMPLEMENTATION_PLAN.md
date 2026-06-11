# Derived Recall Index — Implementation Plan

**Status:** Planned  
**Architecture:** [ADR-0038 — Platform embedding standard and derived recall index](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)  
**Contracts:** [Den archival memory and ingestion](../architecture/den-archival-memory-and-ingestion-contract.md), [Den-native runtime — turn context](../architecture/den-native-runtime.md#turn-context-assembly)

## Goal

Ship Den-owned **semantic recall** over canonical sources using:

- platform embedding standard **`bears-embed-v1`** (`text-embedding-3-small`, 1536d, via Bifrost),
- **Qdrant** derived vectors,
- Postgres **passage registry** metadata,
- turn-start bounded recall + hybrid **`memory_search`**.

Complements existing **key memory projection** (path anchors); does not replace SQLite canonical memory.

## Non-goals (initial phases)

- Rerank model task
- Transcript indexing
- Vectors in bear packages
- pgvector on Den Postgres
- Separate embedding models per source class (unless a future ADR adds e.g. code-only `bears-embed-code-v1`)

## Phase 0 — Platform wiring

- Add env: `EMBEDDING_STANDARD`, `EMBEDDING_MODEL`, `EMBEDDING_DIMENSIONS`, `QDRANT_URL`.
- Register embedding model in Bifrost `config.json`; Den `embedding` model task calls `/v1/embeddings`.
- Compose: optional `bears-qdrant` service (profile `recall`) with persistent volume.
- Preflight: warn when recall enabled but Qdrant unreachable; native runtime works without recall (LIKE fallback).

**Exit:** smoke embed of fixture text; health check passes.

## Phase 1 — Passage registry + Bear indexer

- Migration: `recall_passages` (or `bear_recall_passages`) table — passage id, bear_id, memory_id, logical_path, chunk_index, content_hash, embedding_standard, indexed_at, deleted_at.
- `core/recall/indexer.rs`: on memory record head change, chunk + hash dedup + embed + upsert Qdrant + registry row.
- Indexing policy per ADR-0038 (kinds, visibility, head-only).
- Async queue: enqueue on successful SQLite append (in-process or Docket task).

**Exit:** unit/integration tests — write memory → passage appears in Qdrant with correct payload; supersede removes old passages.

## Phase 2 — Bear recall in turn context

- `core/recall/query.rs`: embed recall query; filtered Qdrant search; merge/dedupe against key memory projection.
- Context assembler: optional `## Recalled memory` section with char budget (~2–3k) after `# Projected memory`.
- Diagnostic JSON alongside `key_memory_projection`.

**Exit:** ACP/native turn tests with seeded vectors; no duplicate anchor/recall text.

## Phase 3 — Hybrid `memory_search`

- Vector path when Qdrant configured; retain SQL `LIKE` fallback.
- Return provenance: logical_path, memory_id, score, snippet.

**Exit:** tool tests for both paths.

## Phase 4 — Cabinet integration

- Cabinet pipeline ([ADR-0008](../decisions/adr-0008-cabinet-reading-pipeline.md)) uses **same** `bears-embed-v1` into Qdrant (`source_class=cabinet_passage`).
- Cross-corpus query policy: optional Cabinet hits when mission/work-surface link + ACL allow.

**Exit:** end-to-end test — Cabinet passage + related Bear memory rank near each other for shared topic query.

## Phase 5 — `archive_index` + migration tooling

- Reflection `archive_index` run type drives reconcile sweep (registry vs canonical vs Qdrant).
- CLI/admin: `reindex-bear`, `reindex-cabinet`, `migrate-embedding-standard` (build v2 collection, progress, alias flip).

**Exit:** documented runbook for `bears-embed-v2` dry run on staging.

## Risks

| Risk | Mitigation |
|------|------------|
| Embed cost at Cabinet scale | Content-hash dedup; incremental indexing; batch embed jobs |
| Model lock-in | Versioned standards; parallel collections; measure before v2 flip |
| ACL leaks on unified collection | Mandatory payload filters; tests per source_class |
| Qdrant downtime | Degrade to anchors + LIKE search; no turn failure |

## Related

- [DEN_NATIVE_RUNTIME_PLAN.md](DEN_NATIVE_RUNTIME_PLAN.md) — native loop and context assembler
- [model-tasks-strategy.md](../research/model-tasks-strategy.md) — `embedding` task class
