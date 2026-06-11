# ADR-0038 — Platform Embedding Standard and Derived Recall Index

**Status:** Accepted (2026-06-09)  
**Related:** [ADR-0031 — SQLite-first canonical store](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), [ADR-0008 — Cabinet reading pipeline](adr-0008-cabinet-reading-pipeline.md), [ADR-0033 — Model tasks layer](adr-0033-model-tasks-layer.md), [Den archival memory contract](../architecture/den-archival-memory-and-ingestion-contract.md), [Den-native runtime](../architecture/den-native-runtime.md)

## Context

Bear Den needs semantic recall to replace Letta archival memory and to support large-scale Cabinet content ([ADR-0008](adr-0008-cabinet-reading-pipeline.md) already targets **Qdrant** for Cabinet passages).

Requirements:

- **Canonical sources stay canonical** — per-Bear SQLite memory ([ADR-0031](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)) and Cabinet source stores are source of truth; vectors are derived.
- **Cross-corpus alignment** — a Bear should find semantically related **Bear memory** when reading or retrieving **Cabinet** material (and vice versa) when policy allows.
- **Long-lived embedding contract** — Cabinet will grow large; re-embedding is costly. Model choice is a **platform decision**, not a per-feature knob.
- **Complement path projection** — [key memory projection](../architecture/den-native-runtime.md#layer-2--key-memory-projection-sqlite) covers stable anchors; semantic recall covers fuzzy/broad knowledge not at fixed logical paths.
- **Den Postgres is control plane** — conversation state, Docket, `bear_compiled_configs`, prompt-memory blocks. Vector payloads must not live in Den Postgres at scale.

Earlier docs deferred a “general-purpose vector store.” This ADR narrows the scope: **one platform embedding standard** and **one derived recall engine (Qdrant)** for approved source classes — not a second memory system.

## Decision

### 1. Platform embedding standard (versioned)

Den defines a **platform embedding standard** — a long-lived contract identified by name, independent of deploy:

| Field | Initial value (`bears-embed-v1`) |
|-------|----------------------------------|
| **Standard id** | `bears-embed-v1` |
| **Model** | `text-embedding-3-small` (OpenAI) |
| **Dimensions** | `1536` |
| **Route** | Bifrost `POST /v1/embeddings` via `LLM_API_URL` |
| **Model task** | `embedding` per [ADR-0033](adr-0033-model-tasks-layer.md) |

**Rules:**

- **One active standard** for all Den-managed recall indexes (Bear memory passages, Cabinet passages, future approved source classes) unless a second standard is explicitly approved (e.g. code-only fork).
- **Never mix dimensions** in one Qdrant collection.
- **Model or dimension change** ⇒ new standard id (`bears-embed-v2`), **new Qdrant collection(s)**, background reindex, alias flip — not in-place mutation of existing vectors.
- Env/config pins the active standard: `EMBEDDING_STANDARD`, `EMBEDDING_MODEL`, `EMBEDDING_DIMENSIONS`.

Bear memory and Cabinet **share the same standard** so query vectors and scores are comparable across corpora.

### 2. Vector store: Qdrant

**Qdrant** is the Den-owned derived vector store for semantic recall.

- **Not** pgvector on Den Postgres (control plane).
- **Not** vectors inside per-Bear SQLite (portability and scale).
- Optional dedicated recall Postgres remains out of scope unless ops constraints require it; Qdrant is the default either way.

**Collection naming** includes the embedding standard, e.g.:

- `den_recall_bears-embed-v1` — unified collection for all source classes, **or**
- `den_recall_bear_memory_bears-embed-v1` + `den_recall_cabinet_bears-embed-v1` — if isolation is preferred (same model/dims; merge at query time).

Prefer **one collection + payload filters** when ACL filtering is solid; split collections only for operational isolation.

**Point payload** (minimum):

- `embedding_standard`, `source_class` (`bear_memory` | `cabinet_passage` | …)
- `content_hash`, `chunk_index`
- Bear scope: `bear_id`, `scope_type`, `scope_profile`, `work_surface_ref`, `logical_path`, `memory_id`, `kind`, `visibility`
- Cabinet scope: `cabinet_ref`, `mission_ref`, `source_url`, … (per Cabinet metadata model)

**Active read path** queries only the collection for the active `EMBEDDING_STANDARD`.

### 3. Passage registry (Postgres metadata, not vectors)

Den Postgres holds **passage registry metadata** only:

- passage id, `embedding_standard`, `source_class`, canonical source ids, `content_hash`, chunk bounds, `indexed_at`, supersession/delete state.

Vectors live in Qdrant. Registry enables idempotent upsert, delete-on-supersede, and reindex job progress without treating Qdrant as source of truth.

### 4. Indexing policy (what gets embedded)

**Bear memory** (from SQLite `memory_records`):

- Include: `visibility=normal` shared records; role-local `note`, `decision`, `summary` (configurable); **latest head only** per logical path (respect `supersedes_memory_id`).
- Exclude by default: `scratch`, raw `log` streams, pending proposals/observations, superseded bodies.

**Cabinet** (per ADR-0008 pipeline):

- Archived article text, highlights, and approved mission-linked material as chunked passages.

**Transcript** — not indexed unless an explicit future policy allows transcript-derived archival sources ([archival contract](../architecture/den-archival-memory-and-ingestion-contract.md)).

**Chunking:** ~400–800 token targets with overlap; store `chunk_index` and `content_hash`. **Re-embed only when `content_hash` changes.**

### 5. Retrieval semantics

Three context lanes (see [den-native-runtime](../architecture/den-native-runtime.md#turn-context-assembly)):

| Lane | Mechanism |
|------|-----------|
| **Anchors** | Key memory projection (path-based, deterministic) |
| **Recall** | Vector search over derived passages (this ADR) |
| **Standing edits** | Prompt memory blocks |

**Turn-start recall (bounded):**

1. Build query text from user message + session focus + primary work surface + active profile.
2. Embed with active platform standard.
3. Search with filters: always `bear_id` for Bear memory; optional Cabinet filters when policy links mission/work surface and ACL permits.
4. Merge with anchor projection; dedupe by `memory_id` / `content_hash`.
5. Inject top **3–5** passages under a provenance-labeled section (char budget separate from anchor projection).

**Tool recall:**

- Upgrade `memory_search` to **hybrid**: vector search when Qdrant is configured, else SQL `LIKE` fallback.
- `memory_read` / `memory_browse` unchanged for exact paths.

**Rerank** (cross-encoder / rerank model task) is optional v2; not required for initial ship.

### 6. Ingestion and lifecycle

- **On canonical write** — enqueue index job (async; Docket or Den worker).
- **`archive_index` reflection runs** — reconcile registry + Qdrant against canonical heads ([reflection taxonomy](../architecture/reflection-run-taxonomy.md)).
- **On supersede/delete** — delete passages by canonical source id + `content_hash`.
- **Bear package import** — **do not** ship vectors; rebuild index from imported `memory.sqlite` into active standard after import ([bear package](../guides/bear-package.md)).
- **Embedding migration** — build `*_bears-embed-v2` collection in parallel, backfill, flip read alias, retire v1 after validation.

### 7. Security and boundaries

- Shared embedding geometry does **not** imply shared access. Every query applies **source_class + ACL filters** (Bear membership, Cabinet mission visibility).
- Cross-corpus search is **policy-gated** (e.g. linked mission + resolved work surface), not default global merge.

## Consequences

**Positive**

- One embedding contract for Cabinet + Bear recall; comparable cross-corpus retrieval.
- Qdrant aligns with ADR-0008; no second vector system.
- Index generations make rare model migrations operable without corrupting live search.
- Content-hash dedup limits day-to-day embed cost at Cabinet scale.

**Negative / costs**

- New infra: Qdrant service (compose profile or managed).
- Bifrost must expose embedding models in config.
- Reindex jobs and passage registry are non-trivial engineering.
- Platform standard locks model choice for years; changing it is a deliberate migration project.

**Supersedes / amends**

- Softens “no general-purpose vector store” language in [den-native-runtime](../architecture/den-native-runtime.md) and [memory-model](../architecture/memory-model.md): Den **does** operate a **derived** Qdrant recall index — not a second canonical memory store.
- Letta Archives and Letta pgvector are not the target; this ADR is the replacement.

## Related implementation

- [Derived recall index implementation plan](../roadmap/DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md)
