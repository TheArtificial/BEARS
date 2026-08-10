# Bear memory (concise guide)

How **durable Bear knowledge** is stored, what is *not* memory, and how Den **assembles context** each turn — for readers who know the stack but not agent-harness details.

**Related:** [ADR-0031 — SQLite-first canonical store](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), [`memory-model.md`](../architecture/memory-model.md) (full model), [`den-runtime.md`](../architecture/den-runtime.md#turn-context-assembly) (turn assembly), [ADR-0038 — Derived recall](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md), [ADR-0041 — Archival recall and async curation](../decisions/adr-0041-archival-recall-and-async-curation.md), [ADR-0046 — file-backed prompt fragments](../decisions/adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md), [`bear-package.md`](bear-package.md) (export/import)

## Canonical store

A **Bear** keeps durable knowledge in its own **SQLite database** (`memory.sqlite`). That file is the source of truth for what the Bear knows — not chat logs, not Cabinet, not Den Postgres.

### What goes in memory

- **Shared `core/`** — curated facts, decisions, work-surface overviews (promoted by **curate**).
- **Stance-scoped branches** — notes under `chat/`, `pair/`, `work/`, etc. (`scope_type=role_local`); they can stay local forever.
- **Curation trail** — proposals, promotions, observations, and Reflection-run outcomes (how knowledge was reviewed).

Memory is **append-only**: updates add new records and may supersede older ones (`supersedes_memory_id`) rather than editing rows in place. Supersession preserves history, so the chain also powers point-in-time recall (`valid_from` / `invalid_at`) — "what did the Bear believe on date X" and "how did this belief change."

### How memory grows

Nothing a Bear talks about becomes durable memory automatically — a transcript scrolling out of context is **not** promotion. Durable knowledge is created only by an explicit curation decision, off the hot path:

```text
session closes / salience threshold
  → archive_harvest extracts evidence-backed claims (or discards, with reasons)
  → memory_proposals
  → curate reviews: promote / supersede / reject
  → core/ updated (supersede, never overwrite)
```

Harvest is **extraction-first**: it distills durable facts, decisions, preferences, and lessons with provenance back to source messages, and discards conversational filler and task residue. A harvest run may legitimately produce nothing. High-sensitivity or low-confidence items escalate to human review rather than auto-applying. See [ADR-0041 — Archival recall and async curation](../decisions/adr-0041-archival-recall-and-async-curation.md).

Quality is also defended at **read time**: when recall retrieves two live records that contradict each other (same path or subject, overlapping validity, divergent heads), both are surfaced with a `conflicting` marker — the Bear reports the disagreement instead of confidently asserting a stale side — and a `memory_conflict` observation is queued for curation to resolve ([ADR-0041 §8](../decisions/adr-0041-archival-recall-and-async-curation.md)).

### What is not Bear memory

| Concern | Where it lives |
|---------|----------------|
| Conversations, tool audit | Den Postgres |
| Jobs and tasks | Docket (Den Postgres) |
| Qdrant vectors | Derived recall index — rebuildable from SQLite |
| Secrets | Host secret store, never in memory |

## How the Bear sees memory each turn

Den builds **Turn Context** in layers. The model does **not** receive the whole database. (den-runtime.md numbers the three system-prompt layers 1–3; the rows below add the transcript/tool tail this guide cares about, so they are named rather than numbered.)

| Mechanism | Role |
|-----------|------|
| **Compiled stance prompt** (den-runtime Layer 1) | Identity and instructions from `bear_compiled_configs`, compiled from repository-authored fragments plus Bear/runtime-authored compile-time prompt content |
| **Key memory projection** (Layer 2) | Small **path-based** slice of SQLite: shared anchors (`core/bear-overview.md`, …), work-surface docs when a surface is resolved, recent stance-local highlights, optional situation briefing (situation records are a v1.1 parity gap) |
| **Derived recall** | Hybrid, policy-gated retrieval over canonical semantic memory: vector (Qdrant), keyword, temporal filtering, and bounded graph expansion where configured and authorized; it degrades to anchors + keyword if Qdrant is unavailable. Vector/keyword are live; richer extraction, semantic dedup, and the graph leg are still landing (ADR-0041) |
| **Runtime supplements** (Layer 3) | Prompt-memory blocks (editable standing Postgres context), compaction state, and channel/session runtime context |
| **Transcript and tools** | Recent conversation and tool results, plus `memory_read`, `memory_browse`, and **`memory_search`** for inspecting more durable memory |

**Anchors** = what we show at known logical paths (deterministic, budgeted).  
**Recall** = what is relevant to this turn through policy-gated hybrid retrieval (fuzzy / cross-topic).  
**Runtime supplements** = standing and session-specific Den context; prompt-memory blocks are durable configuration, but not Bear cognition.  
**Tools** = fetch more when proactive context is not enough.

See [Turn context assembly](../architecture/den-runtime.md#turn-context-assembly), [Prompt Fragment Registry](../architecture/prompt-fragment-registry.md), and [v1 projection policy](../architecture/den-runtime.md#v1-selection-policy-locked) for path lists and char budgets.

## Model experience

> **Design intent, not a full inventory of shipped surfaces.** This section states what the model's memory experience *should* expose; treat "should" as a target. Where a specific diagnostic exists today, den-runtime.md and the reflection/recall plans are authoritative.

The model's memory experience should be explicit enough to answer user questions like "what can you see?" without inventing hidden state.

When a Bear reasons about memory, it should be able to distinguish:

- **Conversation context** — recent messages and tool results currently in the model window.
- **Projected memory** — selected durable-memory snippets injected into the prompt before the turn.
- **Recalled memory** — search/embedding results retrieved for the current turn.
- **Persistent memory** — the larger SQLite-backed store available through memory tools.
- **Task/work state** — Docket/task-list state, not semantic memory.
- **Runtime/tool surface** — tool schemas, environment diagnostics, and compaction metadata.

Useful model-facing surfaces should therefore expose:

- which layer supplied a fact;
- whether that layer is durable, transient, or task-local;
- why a memory was projected or recalled;
- whether context was compacted, omitted, or unavailable;
- what the model can inspect next with tools.

The desired failure mode is also explicit: if Den cannot provide a field, surface `unknown`/`unavailable` rather than letting the model guess. Short chat answers can stay concise, but detailed diagnostics should be available when the user is debugging memory behavior.

## Work surfaces

Memory is Bear-wide, but answers should **ground on the current project, repo, or surface** when one is known. Both anchor projection and derived recall apply work-surface filters when a primary surface is resolved or confirmed. Cross-corpus Cabinet recall is separately policy- and ACL-gated; it is not automatic. See [`work-surfaces-and-conversations.md`](work-surfaces-and-conversations.md).

## Cost and footprint

Memory is designed so the **hot path stays cheap** and the expensive work happens off-turn.

| When | Work | LLM cost |
|------|------|----------|
| **Every turn** | Compiled prompt, key memory projection (deterministic SQLite reads, char-budgeted), runtime supplements | none |
| **Every turn** | Derived-recall query embed, bounded to 3–5 injected passages | one embedding call (skipped when recall is unavailable) |
| **Sleep-time** | Harvest + consolidation (extraction, dedup, synthesis) | LLM passes — event-driven and throttled, **never blocking a live turn** |

**Minimum stack:** Den Postgres + per-Bear SQLite. Qdrant is **optional** — without `QDRANT_URL`, derived recall degrades to anchors + keyword (`LIKE`) and turns still succeed. Vectors are always rebuildable, so a self-hoster can add or drop Qdrant without data loss. `QDRANT_URL` is the **only operational dial** for memory behavior ([complexity budget](../architecture/memory-model.md#complexity-budget)). Secrets and provider keys live in the host secret store, never in memory.

Because indexing is asynchronous, recall can lag canonical memory. Each Bear exposes a **recall watermark** (`indexed_seq` vs `canonical_seq`, via `memory_status` and admin diagnostics) so "is this Bear fully recallable right now?" has a definite answer and degraded recall is visible rather than silent ([ADR-0038 §8](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)).

## Portability

A **cognition export** contains `manifest.yaml` + `memory.sqlite` + optional `artifacts/` (for example, skills and exported policies). An **operator snapshot** additionally carries durable prompt-memory blocks plus non-secret operational configuration such as web-source policy and watch-subscription configuration. Do not ship conversations, jobs, or vectors. On import, rebuild the **Bear-memory** Qdrant recall index from SQLite; shared Cabinet recall is rebuilt from its own canonical sources. See [`bear-package.md`](bear-package.md).

**Embedding-provider footprint.** The active embedding standard (`bears-embed-v1`) currently routes to a hosted model (`text-embedding-3-small`) through Bifrost. A self-hoster with no embedding provider runs anchors + keyword recall today with no loss of canonical memory. Because the standard is versioned ([ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)), moving to a **local** embedder is a supported migration — a new `bears-embed-v2` collection, backfill, alias flip — not a fork. Embeddings are the one recurring provider cost in the memory path, so this is the lever to pull for fully local operation.

## Mental model

```text
Bear memory     = long-term cognition (SQLite)
Den Postgres    = sessions, ops, scheduling
Qdrant          = derived semantic index (disposable)
Turn context    = thin proactive slice (anchors + hybrid recall + runtime supplements) + transcript/tools for the rest
```

**One line:** Bear memory is **long-term cognition in SQLite**; Den assembles a **thin proactive slice** (path anchors, optional hybrid recall, and runtime supplements) and exposes the transcript and **tools** for everything else.
