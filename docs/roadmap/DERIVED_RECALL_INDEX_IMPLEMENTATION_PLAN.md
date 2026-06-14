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

## Phase 0 — Platform wiring  ✅ landed (compose, env, config, embedding client, status check)

- ✅ Add env: `EMBEDDING_STANDARD`, `EMBEDDING_MODEL`, `EMBEDDING_DIMENSIONS`, `QDRANT_URL` — set on `bears-den` in `docker-compose.yaml` (+ `.env.example`); `QDRANT_URL` empty = recall disabled (LIKE fallback).
- ✅ Compose: optional `bears-qdrant` service (profile `recall`) with persistent `bears-qdrant-data` volume. Nothing `depends_on` it; the default stack is unchanged. Enable with `COMPOSE_PROFILES=recall` + `QDRANT_URL=http://bears-qdrant:6333`.
- ✅ Den `Config` consumes the new env (`qdrant_url`, `embedding_standard`, `embedding_model`, `embedding_dimensions`) in `den-core/src/config.rs`.
- ✅ Register embedding model in Bifrost `config.json` (`text-embedding-3-small` authorized on the openai provider key); Den embedding client (`den-llm::EmbeddingClient`) calls `/v1/embeddings` with the active standard's model + dimensions. *(Invocation from the indexer lands in Phase 1.)*
- ✅ Qdrant client (`den-runtime::recall::QdrantRecall`) + startup collection bootstrap (`den/src/startup.rs`) + `/status.json` health check (`web/stack_health.rs`); a recall-enabled full-stack smoke is wired via `SMOKE_RECALL=1` in `scripts/smoke{,-stack}.sh`.
- ⏳ Preflight (`services/preflight/preflight.py`): warn when recall enabled but Qdrant unreachable (runtime `/status.json` check already covers this at request time); native runtime works without recall (LIKE fallback).

**Exit:** smoke embed of fixture text; health check passes. *(Health check ✅; live fixture embed pending a real embedding-capable key in the smoke env.)*

## Phase 1 — Passage registry + Bear indexer  ✅ landed (1a indexer core + 1b auto-enqueue)

- ✅ Migration: `recall_passages` Postgres table (`bear_id` FK, `memory_id`, `logical_path`, `chunk_index`, `content_hash`, `embedding_standard`, `source_class`, `point_id`, `indexed_at`, `deleted_at`; unique on `(bear_id, memory_id, chunk_index, embedding_standard)`). Registry metadata only — vectors in Qdrant, canonical text in SQLite (ADR-0038 §3).
- ✅ `den-runtime::recall` module: `qdrant` (point upsert/delete/count), `chunking` (~2.4k-char chunks + overlap, SHA-256 `content_hash`), `policy` (indexable decision, deterministic `point_id`, Qdrant payload), `registry` (Postgres CRUD with dedup + soft-delete), `indexer` (`RecallIndexer::index_record` / `remove_record`, embedder-agnostic via `PassageEmbedder`; `EmbeddingClient` impl + `DeterministicEmbedder` test stub).
- ✅ Indexing policy per ADR-0038: `visibility = normal`; `shared` records indexed; `profile_local` limited to `note`/`decision`/`summary`; `scratch`/`log` excluded. (Head-only selection relies on canonical head queries; supersession writes are still pending in `den-memory`.)
- ✅ **Phase 1b — async queue**: a `bear_reflection_runs` `recall_index` lane + dedicated worker loop, mirroring `memory_curate` (Den has no Docket job queue). `enqueue_recall_index` coalesces (one queued run per Bear); the worker claims with `FOR UPDATE SKIP LOCKED` and calls `recall::reconcile_bear`, which indexes every indexable canonical **head** per logical path and removes passages for memory ids that are no longer heads (supersede/delete). Enqueued on the canonical durable-memory producer — curate **core promotion** (`apply_core_promotion`); the whole-Bear reconcile then picks up all indexable heads (shared + profile-local `note`/`decision`/`summary`). Worker is spawned only when `RUN_WORKERS` **and** `QDRANT_URL` are set; recall-disabled runs are drained as skipped.

**Exit:** ✅ unit tests (chunking/policy/payload/point-id) + gated **live Postgres + Qdrant** integration test (`tests/recall_indexer.rs`, stub embedder, no API key) proving write memory → one Qdrant point per chunk → idempotent re-index (no duplicates) → `remove_record` deletes the points; plus an infra-free `list_indexable_heads` test proving head-selection + policy filtering (latest-at-path wins, `scratch` excluded).

> **Follow-ups (not blocking):** producer paths without a `PgPool` in scope (work-surface scaffolds, pair `memory_write_entry`) don't yet enqueue directly — they're covered when a curate reconcile runs for the Bear; threading enqueue there (or a periodic reconcile sweep) closes the gap. Supersession is still selected by sequence (no `supersedes_memory_id` writes yet in `den-memory`).

## Phase 2 — Bear recall in turn context

- `core/recall/query.rs`: embed recall query; filtered Qdrant search; merge/dedupe against key memory projection.
- Context assembler: optional `## Recalled memory` section with char budget (~2–3k) after `# Projected memory`.
- Diagnostic JSON alongside `key_memory_projection`.

**Exit:** ACP/native turn tests with seeded vectors; no duplicate anchor/recall text.

## Phase 3 — Hybrid `memory_search`

- Vector path when Qdrant configured; retain SQL `LIKE` fallback.
- Return provenance: logical_path, memory_id, score, snippet.

**Exit:** tool tests for both paths.

## Phase 3.5 — Temporal + bounded graph recall legs

Extends the hybrid retriever ([ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md) §6) beyond vector + keyword + anchors with two cheap legs, both over **canonical SQLite** (no new store). Borrowed from Hindsight's TEMPR (temporal + graph strategies) without adopting its graph/temporal store.

**Temporal leg.**

- Parse explicit/relative time expressions in the query ("last spring", "in June", "before the migration") into a time range; filter/boost candidates by `valid_from`/`invalid_at` (event time) and `created_at` (transaction time) — ADR-0041 §7 bi-temporal-lite.
- Point-in-time recall: "as of `<date>`" returns the record that was the valid head at that time (walk the supersession chain), not only the current head.
- Recency stays a ranking factor for untimed queries.

**Bounded graph leg (record↔entity expansion).**

- Starting from entities resolved in the query/turn context, expand over the **bipartite** record↔entity relation graph (`memory_relations`): entity → its records → co-occurring entities → their records.
- **Depth-capped (default 2 hops), read-only, retrieval-time only.** No stored transitive edges, no inference, no entity↔entity edges — consistent with [ADR-0042](../decisions/adr-0042-memory-entity-relationships-and-bear-entity-layer.md) anti-RDF guardrails. This is query expansion, not a knowledge graph.
- Only `recall_effect != gate` relations participate; access-bearing gating still applies via the required `AccessContext` (the `memory_access_rules` query) before results return.
- Answers indirect queries ("where does Alice work?") by reaching `Alice → works_at → Google → located_in → Mountain View` through shared-entity hops.

**Exit:** recall tests for (a) a relative-time query resolving to the correct historical head, and (b) a 2-hop entity query returning a record never directly matched by vector/keyword; both respect `AccessContext`.

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
