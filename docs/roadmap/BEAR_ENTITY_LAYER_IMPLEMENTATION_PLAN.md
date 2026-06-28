# Bear Entity Layer — Implementation Plan

**Status:** In progress — Phases 0–4 landed/partial; Phase 6 read tools and Bear web entity browser landed; anchors, relation writes/governance, and portability pending  
**Architecture:** [ADR-0042 — Memory–Entity Relationships and the Bear Entity Layer](../decisions/adr-0042-memory-entity-relationships-and-bear-entity-layer.md)  
**Related:** [ADR-0031 — SQLite-first canonical store](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), [ADR-0041 — Archival recall and async curation](../decisions/adr-0041-archival-recall-and-async-curation.md), [ADR-0038 — Derived recall index](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md), [ADR-0006 — Work surfaces](../decisions/adr-0006-bear-work-surfaces.md), [ADR-0040 — Connections](../decisions/adr-0040-connections-and-work-surface-presentation.md), [bear package](../guides/bear-package.md), [memory model](../architecture/memory-model.md)

For the canonical role model and current names, see [bear roles](../architecture/bear-roles.md). "Role" = **trust stance**; `curate` is the curation stance.

## Goal

Give Bears a first-class way to relate memory to **entities** (person, org, event, mission, domain, work surface, connection, artifact) under one unified, descriptor-owned model:

- a **bear-local entity layer** (`entities` + `entity_handles`) — the Bear's portable awareness and resolution of every entity it knows, with one resolution/trust lifecycle, handles, and merge/split;
- a **two-table relation layer** (`memory_relations` descriptive + `memory_access_rules` access-bearing) routed by relation descriptor, with `memory_links` as a read view;
- **enforced visibility gating** (access-bearing relations) and **entity-filtered recall/anchors** (descriptive relations);
- **portability**: the entity layer ships in the cognition export and re-links `canonical_ref` on import.

Work surface and connection become **entity types**; the existing work-surface resolver becomes the **first per-type resolver**. Additive migration — no big-bang.

## Non-goals

- **Entity↔entity edges in bear memory** (registry/Cabinet owns the knowledge graph).
- **Extracting registries' own knowledge** about an entity into the package (Cabinet People content, etc.).
- **Full RDF**: no open predicates, no inference/closure, no reification; relations are fixed `record → entity` only.
- **A separate vector store for entities** — semantic recall stays ADR-0038 Qdrant.
- **`contact` as a type** — a contact is a `person` with address-book provenance.

## Phase 0 — Descriptor foundations  ✅ landed (`den-memory/src/descriptors.rs`)

No schema yet; establish the descriptor-owned vocabularies (per `AGENTS.md`: descriptor-resolved, no scattered `match` arms).

- **Entity-type descriptors**: `person`, `org`, `event`, `mission`, `domain`, `work_surface`, `connection`, `artifact` (`place` reserved). Each declares: owning registry (`cabinet`/`den`/`calendar`/`bear_local`), default trust, anchor-eligibility, valid handle types, resolver binding.
- **Relation descriptors**: `subject`/`source`/`participant`/`applies_when` (class `descriptive`) and `audience`/`confined_to` (class `access_bearing`). Each declares: canonical dotted id + model-facing alias, `class` (immutable), `cardinality`, `applies_to_entities`, `allowed_qualifiers`, `recall_effect` (`boost`/`gate`/`none`), `anchor_projecting`.
- **Handle-type descriptors** with `strength` (`strong`/`weak`), overridable per entity type (e.g. `checkout` weak though path-unique).
- Descriptor resolver module; legacy aliases accepted at routing boundaries only, not advertised.

**Exit:** registry unit tests; relation→class lookup is deterministic and immutable; adding a type/relation is a descriptor-only change.

## Phase 1 — Entity layer schema + store  ✅ landed (`den-memory/src/entity.rs`, `schema.sql`)

Per-Bear SQLite (`den-memory` crate: `schema.sql`, `migrate.rs`, new `entity.rs`).

- Migration: `entities` (opaque ULID `entity_id`, `bear_id`, `sequence_no`, `type`, `display_name`, `resolution`, `trust`, `canonical_ref`, `superseded_by_entity_id`, `metadata_json`, `created_at`) and `entity_handles` (`handle_id`, `entity_id`, `handle_type`, `handle_value`, `source`, `trust`, `state`, `created_at`).
- Store API: create/get/list entities; attach/detach handles; resolution-state transitions; **merge** (survivor + loser `state=merged`, forward `superseded_by_entity_id`; reads follow the pointer) and **split** (new entity, re-home handles).
- Append-only discipline; integrate the existing bear-wide sequence allocator.

**Exit:** store unit tests including merge (forward-pointer reads resolve to survivor) and split (handle re-home).

## Phase 2 — Resolution engine + first resolver (work surface)  ✅ landed (`den-memory/src/resolver.rs`)

- Resolution algorithm (ADR-0042 §11): normalize signals → handles; **strong exact-match ⇒ resolve** (attach handles, raise resolution/trust); **weak-only ⇒** candidate search → `candidate`/`ambiguous` or new `provisional`; **never auto-merge across different strong identities**; conflicting strong matches → `curate`/human queue.
- Per-type **resolver interface**; implement the **`work_surface` resolver first** by adapting current work-surface resolution (`git_remote` strong, `checkout` weak) onto the shared lifecycle — preserving today's behavior.
- Session/ACP-token identity ⇒ a `person` at `confirmed` + asserted trust; never inferred from chat text.

**Exit:** resolver tests — strong match resolves; weak stays provisional; Ryan-style multi-handle stays separate without an authoritative mapping; conflicting strong → review queue. Work-surface parity preserved.

## Phase 3 — Relation layer (two tables + view); retire `entity_ref`  ✅ landed (`den-memory/src/relations.rs`)

- Migration: `memory_relations` (descriptive) and `memory_access_rules` (access-bearing), identical shape (`link_id`, `bear_id`, `sequence_no`, `src_memory_id`, `entity_id`, `relation`, `qualifiers_json`, `author_stance`, `author_agent_id`, `confidence`, `state`, `supersedes_link_id`, `created_at`); `memory_links` **view** = union with a `class` column.
- **Descriptor-routed writes**: the relation descriptor's class selects the table; writers never choose. Validate `qualifiers_json` against `allowed_qualifiers`. `memory_access_rules` enforces **target entity `resolution ≥ resolved`**.
- Retire `memory_records.entity_ref` and the legacy `memory_links` base table.

**Exit:** write tests — descriptive vs access routing; misfiling impossible; access-bearing rejects unresolved targets; union view returns both with correct `class`.

## Phase 4 — Enforcement + recall/projection integration  🟡 partially landed (gate + projection + entity-filter recall leg); boost + bounded-graph deferred

**Landed** (`den-memory/src/access.rs`, `den-runtime/.../key_memory_projection.rs`):

- `AccessContext` is the **fail-closed** gate (`record_visible`) and the **only** reader of `memory_access_rules`; an empty context hides every access-gated record.
- Key memory projection takes a **mandatory `access: AccessContext`** field — omitting it is a compile error. Every projected record is gated; `audience` (ANY-of addressees) and `confined_to` (ALL-of scopes, hard non-leak) are enforced, and the `omitted_by_access` diagnostic records suppressions. The production caller passes `AccessContext::empty()` until session identity is resolved to entities (Phase 6) — a no-op today since no access rules exist yet, which is exactly why the gate lands **before** access-rule writes.

**Landed — entity-filter recall leg** (`den-memory/src/relations.rs`, `den-runtime/src/recall/{policy,reconcile,query}.rs`):

- `relations::descriptive_entity_ids_by_source` — one bulk query mapping each record to its distinct **descriptive** (`recall_effect = boost`) entity ids; the access-bearing gate (`memory_access_rules`) is excluded. Reconcile fetches it once per pass and threads it onto each `IndexRequest`.
- The Qdrant passage payload now denormalizes `entity_ids` (derived data; SQLite + the relation tables remain canonical), so passages can be filtered/boosted by entity without a SQLite round-trip.
- `recall::search_bear_memory_for_entities` (+ private `entity_scope_filter`): bear-wide, entity-membership-scoped semantic search — the query-side consumer of the denormalized ids and the seed leg for bounded-graph expansion + entity-centric admin recall. Best-effort/`disabled`-tagged like the other recall entry points.
- Tests: `descriptive_entity_ids_by_source` excludes access-bearing rows; payload carries `entity_ids`; `entity_scope_filter` shape; gated live round-trip (`entity_scoped_recall_filters_by_payload_entity_ids`) proving a matching entity filter retrieves the passage and an unrelated one excludes it.

**Deferred:**

- Score **boost** in the **vector** leg (vs. the landed graph-leg `entity_overlap` boost) for shared-entity overlap, and `applies_when` proactive surfacing in projection (descriptive boost). Needs query-time resolved entities (work surfaces already resolve; session identity is Phase 6).
- ~~The **bounded graph recall leg**~~ — **landed** in [DERIVED_RECALL Phase 3.5](DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md) (`bounded_graph_expand` + `memory_search` graph leg).

**Exit (gate slice, met):** projection tests — `confined_to` prevents cross-surface leakage, fail-closed by default, granting the scope surfaces it; building projection without `AccessContext` fails to compile. **Exit (entity-filter recall slice, met):** payload denormalization + entity-scoped retrieval proven end-to-end against live Qdrant. **Exit (vector boost + applies_when surfacing):** pending query-time entity resolution / Phase 6. **Exit (bounded-graph):** met (Phase 3.5).

## Phase 5 — Anchors generalize

- Extend logical-path projection (`logical_path.rs`) so **resolved + salient** entities get anchors: `core/people/<id>/…`, `core/missions/<id>/…` beside `core/work_surfaces/<slug>/…`.
- Transient/low-salience mentions stay **query-derived** ("entity page" = `memory_links` view filtered by `entity_id`).
- V1 salience promotion threshold is settled: at least one `subject`-linked `high|critical` record, or a `confirmed` entity with at least two `normal` `subject`-linked records.
- V1 projection is explicit-anchor-only: project generated anchor paths when records exist; do not synthesize projection content from `memory_links` fallback yet.

**Exit:** projection tests for explicit entity anchors; entity-page derivation over the view.

## Phase 6 — Tools + curation/governance  🟡 read tools landed

- ✅ Model-facing read tools (descriptor-named): `entity_browse`, `entity_resolve`.
- ✅ Member-facing Bear web UI entity browser/detail pages: `/bear/{slug}/entities` and `/bear/{slug}/entities/{entity_id}`.
- Pending: relation writes for **descriptive** relations from `chat`/`pair`/`work`/`watch`.
- Restricted to `curate`/Den: `entity_merge`, `entity_split`, weak-candidate promotion, and **all** `memory_access_rules` writes (differential write authz per table).
- `curate` Reflection lane(s) for candidate promotion, merge/split review, and access-rule authoring (consistent with ADR-0041 curation).
- Expose resolved entities in `session_info`.

**Exit:** tool + authz tests — `chat` cannot write access rules; `curate` can; merge/split curate-only; audit trail present in `memory_access_rules`.

## Phase 7 — Portability (bear package)

- Entity tables ship automatically in the **cognition export**; bump `memory_schema_version`.
- Import: existing `bear_id` rewrite covers the new tables; `entity_id` stays stable; **re-link `canonical_ref`** against destination registries via strong handles; demote non-re-resolving entities to `provisional` (clear `canonical_ref`); access rules **inert until re-resolved** (fail-closed).
- Optional manifest **entity summary** (counts by type, resolved vs provisional).

**Exit:** export/import round-trip tests — cross-host `canonical_ref` re-link + demotion; access rules fail-closed after import; entity counts reported.

## Likely files

- `services/den/crates/den-memory/src/`: `schema.sql`, `migrate.rs`, `records.rs`, `logical_path.rs`, new `entity.rs` / `resolver.rs`, relation write paths.
- `services/den/src/core/memory/`: `tools.rs`, `curate_executor.rs`, `admin_inspect.rs`, recall/prompt projection.
- `services/den/src/core/agent_loop/key_memory_projection.rs` (entity-aware projection + `AccessContext`).
- Bear-package export/import (per [bear package](../guides/bear-package.md) "Where implementation will live").

## Failure-mode acceptance checklist

Identity and multi-profile hazards every phase must hold against.

**Entity resolution**

- **Cross-source identity (the "Ryan" case):** the same display name arriving as `slack_user` + `email` + `cabinet_ref` does **not** auto-merge; fusion requires an authoritative mapping or an explicit merge. Weak handles (`mention`, `checkout`) keep an entity `provisional`.
- **Split correction:** a wrong merge is repairable — `entity_split` re-homes the offending handle and the relations carrying its `source_handle`; provenance-less relations queue for `curate`. No relation is silently mis-attributed.
- **Stable identity across sessions:** `entity_id` persists across conversations/sessions (conversation binding is control-plane); recall/anchors over a known entity survive a new session — no "ghost" re-creation of an entity the Bear already knows.

**Merge/split integrity**

- **No history loss:** merge never hard-deletes — the loser is `state=merged` with a forward pointer; reads resolve to the survivor; the operation is auditable.

**Cross-profile safety (multi-agent overwrite / role drift)**

- **No cross-branch overwrite:** a trust stance cannot mutate another stance's records or relations; `core/` is written only via `curate`; the bear-wide sequence allocator prevents concurrent clobber (append-only + supersession, never in-place edit).
- **Authorship recorded:** every record/relation carries `author_stance`; access-rule writes are `curate`/Den-only and enforced, not advisory.

**Visibility (fail-closed)**

- **No gating on unresolved identity:** `memory_access_rules` only target `resolved`/`confirmed` entities; an `audience`/`confined_to` rule against a provisional/mergeable entity is rejected.
- **Import stays closed:** after import, access rules whose target failed to re-resolve are inert (memory hidden), never open.

**Traceability**

- **Auditable resolution:** entity resolution, merge, split, and access-rule changes are reconstructable from append-only rows (the `memory_access_rules` table is the visibility audit log).

## Open items (from ADR-0042)

- Final table naming (`memory_access_rules` vs alternatives).
- Whether `work_surface_ref` retires once `entities` lands, or remains a denormalized convenience.
- Salience threshold for anchor promotion (Phase 5).

## Sequencing notes

- Phases 0–3 are the schema/model core and can land before any recall/UX work.
- Phase 4 is the security-critical phase — do not expose access-bearing relation writes (Phase 6) before the mandatory-`AccessContext` enforcement (Phase 4) is in place.
- The `work_surface` resolver (Phase 2) is the parity anchor: nothing about current work-surface behavior should regress as it moves onto the shared lifecycle.
- **Cross-plan dependency:** the bounded-graph recall leg ([DERIVED_RECALL Phase 3.5](DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md)) depends on the Phase 3 relation layer and reuses the Phase 4 `AccessContext` gate; it adds no stored edges. Sequence Phase 3.5 of that plan after Phases 3–4 here.
