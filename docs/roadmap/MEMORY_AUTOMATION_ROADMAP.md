# Memory Automation Roadmap

> **Direction changed (2026-06).** Canonical memory is per-Bear SQLite ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)); Letta Archives and `pair/` MemFS branches are removed. Long-term recall is a **derived Qdrant index** over canonical SQLite ([ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)); the engine that *fills* it (extraction-first **harvest** + **consolidation** by supersession) and recall scoring are defined in [ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md). Canonical target: [Den runtime](../architecture/den-runtime.md) ([runtime plan](DEN_RUNTIME_PLAN.md)).

For the canonical stance model and current stance names, see [bear stances](../architecture/bear-stances.md).
Status: implementation roadmap; P0 pair-reflection proposal enqueue is implemented for ACP close.

This roadmap sequences the remaining work needed for `pair` learning to become useful to `work` through reflection, curation, `core/`, Cabinet, task context, and the **derived recall index**.

Related docs:

- [Pair Reflection and Work Memory Sharing Plan](PAIR_REFLECTION_AND_WORK_MEMORY_PLAN.md) — focused pair→curate→work boundary design.
- [Memory Curation Plan](MEMORY_CURATION_PLAN.md) — focused memory proposal and core-write curation design.
- [Reflection System Shared Infrastructure Plan](REFLECTION_SYSTEM_PLAN.md) — queue, runner, scheduler, and shared control-plane design.
- [Memory Tools Implementation Plan](MEMORY_TOOLS_IMPLEMENTATION_PLAN.md)
- [Memory Model](../concepts/../architecture/memory-model.md)

---

## Target end state

```text
pair learns useful workplace knowledge
→ pair writes role-local memory (per-Bear SQLite)
→ pair reflection summarizes/consolidates pair memory
→ pair reflection creates memory proposals
→ archive_harvest mines older closed sessions for more candidates
→ queued `memory_curate` reflection run resolves proposals (dedup + supersession)
→ curate updates core / requests recall indexing / prepares task context / creates Cabinet proposals
→ work receives approved task context and can search permitted recall scopes
```

`work` must never read raw `pair/`. It should benefit through curated channels only.

---

## P0 — Pair reflection to curate trigger

Status: implemented for the ACP close path, except UI surfacing.

### Goal

Pair reflection should immediately feed curation without waiting for manual human action.

### Deliverables

1. ✅ Pair reflection writes a `pair/summaries/` entry on ACP close.
2. ✅ Pair reflection creates a `bear_memory_proposals` row referencing that summary.
3. ✅ Pair reflection enqueues a queued `bear_reflection_runs` row with `lane = memory_curate` and `trigger = pair_reflection`.
4. ✅ ACP close remains responsive; curation does not run inline during ACP close.
5. 🟡 UI shows generated memory proposals on the Bear memory dashboard, including native SQLite proposal rows, and recent reflection-run performance (queue wait/runtime/outcomes); dedicated run detail/retry UI remains pending.

### Notes

- The proposal uses `suggested_action: unspecified` initially.
- The automatic proposal has `source_role = pair`, `source_paths = [pair/summaries/...]`, `sensitivity = normal`, and `requires_human = false`.
- Curate decides final outcome.
- Human review is an escalation path, not the default workflow.

---

## P1 — Automated curation conductor

### Goal

Make `curate` autonomous for memory review and `core/` cleanliness.

### Conversation policy

Use one curate conversation per Bear + lane + UTC day.

```text
conversation_key = memory_curate:YYYY-MM-DD
```

Rollover:

- MVP: new conversation per UTC day.
- Later: also roll over when context is near full or operational policy requests reset.

### Trigger policy

MVP triggers:

- pair reflection creates a memory proposal;
- manual admin/operator trigger.

Future trigger:

- dynamic heartbeat based on memory pressure signals.

### Data model

Implemented lane-neutral storage:

- `bear_reflection_runs`
  - `id`
  - `bear_id`
  - `lane`
  - `trigger`
  - `status`
  - `role_agent_id`
  - `conversation_id`
  - `conversation_key`
  - `conversation_date`
  - `input_summary jsonb`
  - `output_summary jsonb`
  - `error text`
  - `started_at`
  - `completed_at`
  - `created_at`
- `bear_reflection_run_items`
  - optional per-run item links; MVP stores proposal IDs in `input_summary`
- `reflection_conversations`
  - `bear_id`
  - `role_agent_id`
  - `lane`
  - `conversation_date`
  - `conversation_key`
  - `conversation_id`
  - `created_at`
  - `last_used_at`

Unique key:

```text
bear_id + lane + conversation_date
```

### Runner behavior

A memory-review cycle should:

1. Load pending proposals.
2. Resolve or create the daily curate conversation.
3. Prompt the `curate` role with bounded context.
4. Allow only approved Den memory/proposal tools.
5. Record cycle state and outputs.
6. Surface cycle activity in UI.

### Dynamic heartbeat later

Not MVP, but design for:

```text
evaluate_curate_memory_pressure(bear_id) -> should_run, lane, reason, priority
```

Signals:

- pending proposal count;
- age of oldest proposal;
- recent pair reflection volume;
- watch observations;
- work results;
- stale `core/` indicators;
- failed/queued cycles.

---

## P2 — Model-assisted pair reflection

### Goal

Upgrade deterministic pair summaries into useful role-local reflection.

### Behavior

A model-assisted pair reflection pass should extract:

- durable technical decisions;
- repeated failure modes;
- repo/workplace conventions;
- human preferences relevant to pair work;
- candidate memory proposals;
- items that should remain local.

### Inputs

- recent ACP pair messages;
- local tool activity summary;
- pair memory entries created during the session;
- relevant `core/` orientation;
- authenticated human identity from ACP token.

### Outputs

Writes only to `pair/`:

```text
pair/summaries/
pair/reflections/
pair/decisions/
pair/notes/
```

May create memory proposals.

### Constraints

- No `core/` writes.
- No Cabinet writes.
- No external tools.
- No raw cross-role branch reads.
- Not a sixth Bear role.

---

## P2.5 — Proactive archive harvest (`archive_harvest`)

### Goal

Turn closed session archives into durable memory candidates, not just the active session. This is the [ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md) `archive_harvest` Reflection lane.

### Behavior

- 🟡 Compaction-artifact harvest is implemented: scan **un-mined** compaction artifacts, distill structured summary sections, and create human-review memory proposals. Broader closed-conversation mining and model-assisted extraction remain open.
- ✅ Emit memory proposals (candidate durable entries) with harvest provenance (`source_hash`, `run_id`, source refs); do not write `core/` (that is `memory_curate`).
- 🟡 Apply a deterministic quality/risk filter before a candidate becomes a proposal: transient follow-up-only artifacts and goal/workflow-only summaries are marked harvested without a proposal; durable decisions/constraints/artifacts receive confidence metadata; artifact-only candidates are retained at medium confidence; person/secret/external-risk signals set proposal sensitivity for human review. Fixture coverage exists for each deterministic branch. Richer model-assisted confidence scoring remains open.

### Triggers

- `session_archived`;
- `cumulative_salience_threshold`;
- throttled/adaptive heartbeat (never a fixed cron).

### Data model

✅ `memory_harvest_marks` (per-Bear SQLite) is implemented for idempotency: `source_kind` (`conversation` | `compaction_artifact` | `observation` | `pair_summary`), `source_ref`, `source_hash`, `harvested_at`, `run_id`, `proposal_ids_json`. Current compaction-artifact harvest records `source_hash` and the reflection `run_id`. See [ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md) schema deltas.

### Constraints

- transcripts are source material, never auto-promoted to memory;
- bounded token budget per run;
- feeds `memory_curate` for dedup, supersession, and promotion.

---

## P3 — Derived recall indexing (`archive_index`)

### Goal

Maintain a **derived Qdrant recall index** over canonical SQLite (and Cabinet) sources ([ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)). This replaces Letta Archives. Vectors are derived and rebuildable; SQLite remains canonical.

### What is indexed

Per [ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md) §4 and [ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md): `visibility=normal` shared records and role-local `note`/`decision`/`summary` (**latest non-invalid head only**, respecting `supersedes_memory_id`/`invalid_at`); approved proposal outcomes; Cabinet material where approved. Excluded by default: `scratch`, raw `log` streams, pending proposals/observations, superseded/archived bodies, and transcripts.

### Data model (passage registry, not vectors)

Vectors live in **Qdrant**; Den **Postgres** holds passage-registry metadata only ([ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md) §3): passage id, `embedding_standard`, `source_class`, canonical source ids, `content_hash`, chunk bounds, `indexed_at`, supersession/delete state. Detailed schema lives in the [Derived recall index implementation plan](DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md).

### Sync behavior

- unchanged `content_hash`: no-op;
- changed `content_hash`: re-embed and replace the passage;
- superseded/deleted/archived canonical source: delete passages by source id + hash or skip on reconcile;
- bear package import rebuilds vectors from `memory.sqlite`; vectors are never shipped;
- search results point back to canonical sources.

### Write boundary

Indexing runs through Den/curate workflows; role agents do not maintain the shared index. `archive_harvest` produces canonical records; `archive_index` indexes them — the two lanes stay separate.

---

## P4 — Semantic recall search for work

### Goal

Allow `work` to benefit from curated pair/curate learning without reading raw `pair/`.

### Tool

Upgrade `memory_search` to **hybrid** (vector recall over Qdrant when configured, else SQL `LIKE`), ranked by `recency × relevance × importance × freshness_trend` ([ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md)); degrade gracefully to anchors + `LIKE` when Qdrant is unavailable. Vector/keyword/graph hits now carry salience plus lifecycle/freshness indicators. An optional `den.memory.recall` / `memory_recall` may be added if a dedicated recall entry point is preferred over overloading `memory_search`.

### Work policy

`work` may search:

- shared `core/` recall;
- Cabinet Mission recall scopes attached to the task/Bear;
- task-permitted Cabinet/recall scopes.

`work` must not search:

- raw role-local memory of other roles unless explicitly curated/attached;
- raw `pair/`;
- unrelated Bears (ACL by membership + identity scope, [ADR-0015](../decisions/adr-0015-multi-user-memory.md)).

### Outputs

Recall results must include:

- passage snippet;
- `source_class` and canonical id;
- source path/ref;
- `content_hash`/version;
- relevance score;
- instruction to fetch the canonical source when exact truth matters.

---

## P5 — Work task context bridge

### Goal

Attach curated pair-derived knowledge to approved `work` tasks.

### Flow

```text
pair memory / pair reflection
→ memory proposal
→ curate review
→ task context attachment
→ work receives scoped context
```

### Deliverables

1. Extend task/work schemas with memory source refs and curated context excerpts.
2. Curate can attach selected proposal summaries to task context.
3. Work task prompt includes:
   - curated summary;
   - source refs;
   - relevant `core/` paths;
   - permitted recall scopes;
   - explicit scope and tools.
4. UI shows which pair-derived memory informed a task.

### Constraints

- Attach distilled summaries, not raw pair logs.
- Include provenance.
- Respect human/sensitivity policy.
- Keep task context bounded.

---

## P6 — Core and archive outcome refinement

### Goal

Keep shared memory clean and searchable.

### Deliverables

1. `memory_apply_core_update` supports bounded append/create/replace workflows, writing new SQLite records and setting `supersedes_memory_id` on replace ([ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md)).
2. Curate can compact `core/` records/sections via supersession (not destructive overwrite).
3. Curate can request recall indexing of curated summaries (`archive_index`).
4. UI shows source proposal → `core/` record → recall passage mapping.
5. Revert/rollback flow is designed for bad shared memory updates (supersession chain is the audit trail).

---

## Observability requirements

The UI should surface:

- pair reflection runs;
- memory proposals;
- queued and completed reflection runs;
- proposal decisions;
- `core/` updates;
- archive indexing changes;
- work task context attachments;
- failures/skips/retries.

Humans should see what the system is doing and override when necessary, without approving every routine memory operation.

---

## Immediate next implementation sequence

1. ✅ Pair reflection creates a memory proposal and enqueues a `memory_curate` run.
2. ✅ Add lane-neutral `bear_reflection_runs`, `bear_reflection_run_items`, and `reflection_conversations` storage.
3. ✅ **Tool exposure (read side)** — `chat`, `pair`, `curate`, `work`, and `watch` have read/status/search descriptors; write/review policy for `work`/`watch` remains open.
4. Next: add manual/queued conductor runner for the `memory_curate` lane.
5. Next: surface generated proposals and queued reflection runs in UI.
6. ✅ Apply core [ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md) store deltas (`salience` on `memory_records`, `valid_from`/`invalid_at`, store-level supersession invalidation, `memory_harvest_marks`). Next: use them from curate/consolidation policy.
7. Then: add model-assisted pair reflection (P2) and the `archive_harvest` lane (P2.5).
8. ✅ Derived Qdrant recall index (P3) and hybrid scored `memory_search` (P4) are landed; remaining recall work is live ops exercise and deeper salience/freshness scoring.
9. Then: work task context bridge (P5).
