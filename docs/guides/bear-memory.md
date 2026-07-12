# Bear memory (concise guide)

How **durable Bear knowledge** is stored, what is *not* memory, and how Den **assembles context** each turn — for readers who know the stack but not agent-harness details.

**Related:** [ADR-0031 — SQLite-first canonical store](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), [`memory-model.md`](../architecture/memory-model.md) (full model), [`den-runtime.md`](../architecture/den-runtime.md#turn-context-assembly) (turn assembly), [ADR-0038 — Derived recall](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md), [ADR-0046 — file-backed prompt fragments](../decisions/adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md), [`bear-package.md`](bear-package.md) (export/import)

## Canonical store

A **Bear** keeps durable knowledge in its own **SQLite database** (`memory.sqlite`). That file is the source of truth for what the Bear knows — not chat logs, not Cabinet, not Den Postgres.

### What goes in memory

- **Shared `core/`** — curated facts, decisions, work-surface overviews (promoted by **curate**).
- **Profile-scoped branches** — notes under `chat/`, `pair/`, `work/`, etc.; they can stay local forever.
- **Curation trail** — proposals, promotions, observations (how knowledge was reviewed).

Memory is **append-only**: updates usually add new records and may supersede older ones, rather than editing rows in place.

### What is not Bear memory

| Concern | Where it lives |
|---------|----------------|
| Conversations, tool audit | Den Postgres |
| Jobs and tasks | Docket (Den Postgres) |
| Qdrant vectors | Derived recall index — rebuildable from SQLite |
| Secrets | Host secret store, never in memory |

## How the Bear sees memory each turn

Den builds **Turn Context** in layers. The model does **not** receive the whole database.

| Layer | Mechanism | Role |
|-------|-----------|------|
| 1 | **Compiled profile prompt** | Identity and instructions from `bear_compiled_configs`, compiled from repository-authored fragments plus Bear/runtime-authored compile-time prompt content |
| 2 | **Key memory projection** | Small **path-based** slice of SQLite: shared anchors (`core/bear-overview.md`, …), work-surface docs when a surface is resolved, recent profile-local highlights, optional situation briefing |
| 3 | **Derived recall** | When Qdrant is configured: semantic top passages from embedded chunks (and later Cabinet, same embedding standard) |
| 4 | **Prompt memory blocks** | Editable standing context in Postgres (session, work surface, profile) |
| 5 | **Tools** | `memory_read`, `memory_browse`, **`memory_search`** (vector + keyword when recall is enabled) |

**Anchors** = what we show at known logical paths (deterministic, budgeted).  
**Recall** = what is semantically near this turn (fuzzy / cross-topic).  
**Tools** = fetch more when proactive context is not enough.

See [Turn context assembly](../architecture/den-runtime.md#turn-context-assembly), [Prompt Fragment Registry](../architecture/prompt-fragment-registry.md), and [v1 projection policy](../architecture/den-runtime.md#v1-selection-policy-locked) for path lists and char budgets.

## Model experience

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

Memory is Bear-wide, but answers should **ground on the current project, repo, or surface** when one is known. Both anchor projection and derived recall apply work-surface filters when a primary surface is resolved or confirmed. See [`work-surfaces-and-conversations.md`](work-surfaces-and-conversations.md).

## Portability

Export = `manifest.yaml` + `memory.sqlite` (+ optional skills). Import on another Den; **rebuild** the Qdrant recall index from SQLite. Do not ship conversations, jobs, or vectors. See [`bear-package.md`](bear-package.md).

## Mental model

```text
Bear memory     = long-term cognition (SQLite)
Den Postgres    = sessions, ops, scheduling
Qdrant          = derived semantic index (disposable)
Turn context    = thin proactive slice (paths + recall) + tools for the rest
```

**One line:** Bear memory is **long-term cognition in SQLite**; Den assembles a **thin proactive slice** (path anchors + optional semantic recall) and exposes **tools** for everything else.
