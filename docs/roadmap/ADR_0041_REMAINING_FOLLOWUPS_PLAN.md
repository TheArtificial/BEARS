# ADR-0041 Remaining Follow-ups Plan

**Status:** focused implementation plan  
**Parent:** [ADR-0041 — Archival recall and asynchronous curation](../decisions/adr-0041-archival-recall-and-async-curation.md)

This plan covers the three remaining ADR-0041 follow-up tracks after the initial harvest, lifecycle, review, supersession, and recall-scoring mechanics landed.

## 1. Reflection visibility and performance UI

Goal: make sleep-time memory work understandable and operable by humans.

Landed baseline:

- Bear memory dashboard surfaces Postgres and native SQLite memory proposals.
- Dashboard shows recent reflection runs for `memory_curate`, `archive_harvest`, `recall_index`, and `context_compact`.
- Dashboard shows queue/running/completed/failed counts, average completed runtime, per-run queue wait, runtime, trigger, lane, and outcome counts.

Next slices:

1. Add lane-specific detail pages for a reflection run (`/bear/{slug}/memory/reflection/{run_id}`) with full `input_summary`, `output_summary`, error, linked proposal ids, and conversation link when present.
2. Add filters by lane/status and age buckets (`queued > N minutes`, `failed today`, `slow completed runs`).
3. Add operator affordances: retry failed run, enqueue `archive_harvest`, enqueue `memory_curate`, enqueue `recall_index`.
4. Add lightweight SLO indicators: oldest queued run age, failure rate over last 24h, p50/p95 runtime by lane.

Non-goals:

- Do not expose raw pair/chat transcripts through reflection UI.
- Do not make reflection decisions editable outside review tools.

## 2. Model-assisted extraction

Goal: replace deterministic summary mining with a bounded extraction pass that produces higher-quality durable candidates.

Current baseline:

- `archive_harvest` mines compaction artifacts deterministically.
- It drops transient follow-up-only artifacts and emits human-review proposals only for durable signals.
- It classifies obvious `person`, `secret_risk`, and `external_untrusted` signals for human review.

Next slices:

1. Define a typed extraction output schema: `candidate_type`, `claim`, `evidence_refs`, `confidence`, `sensitivity`, `suggested_action`, `target_hint`, `entities`, and `discard_reason`.
2. Add a bounded curate/profile model turn for `archive_harvest` using only source summaries/artifact refs, not raw transcript dumps by default.
3. Add confidence thresholds:
   - drop low-confidence candidates,
   - create `human_review` proposals for risky/sensitive candidates,
   - allow only normal low-risk candidates into autonomous curation review.
4. Extend pair reflection similarly: extract durable decisions/conventions/preferences separately from active task state.
5. Add tests with deterministic fixtures for noisy/transient summaries, risky content, conflicting facts, and high-quality durable facts.

Non-goals:

- No direct `core/` writes from extraction.
- No raw transcript indexing.
- No external web/tool calls during extraction.

## 3. Semantic dedup and synthesis

Goal: make consolidation smarter than exact string matching without silently rewriting shared truth.

Current baseline:

- Reviewed core updates supersede prior path heads and set `invalid_at`.
- Exact duplicate core updates record `dedupe_core_noop` provenance without writing a new record.
- Risky proposals defer or escalate rather than auto-promoting.

Next slices:

1. Add deterministic pre-dedup:
   - normalized content hash,
   - same target path + same claim fingerprint,
   - same source artifact hash.
2. Add semantic dedup proposal mode:
   - search relevant existing canonical heads,
   - produce a proposed resolution (`noop`, `supersede`, `synthesize`, `human_review`),
   - never auto-apply ambiguous semantic matches.
3. Add contradiction handling:
   - identify candidate vs existing head conflict,
   - propose a supersession narrative (`previously X; now Y`) for curate/human review.
4. Add synthesis records:
   - when repeated high-salience evidence accumulates, propose a `reflection` record summarizing the pattern,
   - keep links/provenance to source proposals/records.
5. Add evaluation fixtures: exact duplicates, near duplicates, contradictions, cumulative evidence, sensitive/person content.

Non-goals:

- No graph database or persistent inferred edges.
- No silent promotion of person/secret/external-risk content.
- No deletion of superseded history.
