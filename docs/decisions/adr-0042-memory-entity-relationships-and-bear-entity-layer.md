# ADR-0042 — Memory–Entity Relationships and the Bear Entity Layer

**Status:** Proposed (2026-06-14)
**Deciders:** Hans
**Related:**
- [ADR-0031 — SQLite-first canonical store](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)
- [ADR-0041 — Archival recall and asynchronous curation](adr-0041-archival-recall-and-async-curation.md)
- [ADR-0038 — Platform embedding standard and derived recall index](adr-0038-platform-embedding-standard-and-derived-recall-index.md)
- [ADR-0006 — Bear work surfaces](adr-0006-bear-work-surfaces.md)
- [ADR-0040 — Connections and work-surface presentation](adr-0040-connections-and-work-surface-presentation.md)
- [ADR-0039 — Trust profiles and governance modes](adr-0039-trust-profiles-and-governance-modes.md)
- [ADR-0015 — Multi-user memory](adr-0015-multi-user-memory.md) (Letta-era; to be reframed for native by this ADR)
- [ADR-0008 — Cabinet reading pipeline](adr-0008-cabinet-reading-pipeline.md)
- [Memory model](../architecture/memory-model.md)

## Context

Retiring Letta cost the native runtime a capability it has not replaced: **delineating memory about specific entities**. In the Letta era, MemFS templates and ADR-0015's isolated `human` / `person:{name}` blocks let a Bear keep per-user memory. The native SQLite model ([ADR-0031](adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md)) delineates memory by **trust profile** (`scope_type`/`scope_profile`) and by **work surface** (`work_surface_ref` + anchor projection), but has no first-class way to relate memory to a person, contact, event, organization, or other entity. The `entity_ref` column exists but is never populated; `memory_links` (`dst_ref_type`/`dst_ref`/`link_type`) is the only generic hook and is unused.

Two observations frame the decision:

- **"Person" is the wrong unit to hard-code.** We will also want contacts, events, missions, domains, organizations, artifacts, places. Hard-coding `person` repeats the Letta mistake at a different layer.
- **Relating memory to an entity is not one relationship, and "entity" is not one storage tier.** A note can be *about* an entity, *for* an entity's eyes only, *from* an entity, *involving* several entities, or *applicable when* an entity is in play. And the same real-world person ("Ryan") shows up as a doc mention, a Slack user, and a Cabinet author — identities that must be **resolved into one entity and stay correctable** (the Slack Ryan may turn out to be someone else).

This sits cleanly in the [four-axis model](../architecture/interactive-profiles-and-role-axes.md): memory scope partitions by **trust profile** (Trust axis) and **work surface** (Resource axis). Entity-relatedness is **none of those axes** — it is a cross-cutting *relation on a record*, not a profile scope ([ADR-0039](adr-0039-trust-profiles-and-governance-modes.md)). It must therefore be modeled as typed relations, not as another `scope_*` partition.

The design must be **flexible for new relationship and entity types without becoming RDF**: no arbitrary predicates, no triple store, no inference.

## Decision

### 1. Memory references entities; it does not define them

Bear memory holds **statements about** and **typed relations to** entities. The canonical object for an entity lives in its owning **registry**, not in bear memory:

- People / Missions / Knowledge → **Cabinet** ([ADR-0008](adr-0008-cabinet-reading-pipeline.md))
- Users / Connections / Work surfaces / Docket projects → **Den control-plane** ([ADR-0040](adr-0040-connections-and-work-surface-presentation.md), [ADR-0034](adr-0034-jobs-and-tasks-work-management.md))
- Events → calendar / external systems

**Entity↔entity edges** ("Ryan authored Doc X", "Work surface W is reached through Connection C") stay in those registries. Bear memory stores only **record→entity** relations and its **own reference + resolution** of each entity. A Bear may hold a **provisional** entity until it is canonicalized (mirroring the work-surface lifecycle).

### 2. Two layers

1. **Bear entity layer (bear-local, portable):** `entities` + `entity_handles` — the Bear's awareness and resolution of every entity it knows, with a `canonical_ref` into the owning registry when resolved. This is the portable unit (travels in the bear package).
2. **Memory–entity relation layer:** relations from a `memory_record` to a bear-local `entity_id`, typed and classified (§3).

### 3. The relation is not singular: descriptive vs access-bearing (settled — "fork 1")

Relation **kinds** split into two **classes**, because they have different enforcement semantics even though they share a shape:

- **Descriptive** — filter/boost only, never gate visibility: `subject` (aboutness), `source` (provenance/origin), `participant` (multi-party involvement), `applies_when` (context/trigger, e.g. an event or work surface).
- **Access-bearing** — gate visibility/recall, enforced: `audience` (only surface when acting with/for this entity — the native heir to Letta's isolated `human` block), `confined_to` (hard scope; e.g. client-A memory must not leak into client-B's work surface).

The same entity may be referenced under either class (e.g. a work surface via `applies_when` to *boost* work-surface-first recall, vs `confined_to` to *gate* leakage). Work-surface grounding precedence stays **descriptive**; only true confinement is access-bearing.

### 4. Open, governed, descriptor-owned vocabularies — not RDF (settled)

Entity types, relation types, and presentation labels are all **descriptor-owned** (consistent with [ADR-0040](adr-0040-connections-and-work-surface-presentation.md) and `AGENTS.md`: no scattered `match` arms or hardcoded allowlists). Adding a relation or entity type is a **descriptor edit, not a schema migration**. Writes validate against the registry.

Anti-RDF guardrails (invariants):

- **Fixed edge domain/arity:** every relation is exactly `memory_record → entity`. No entity↔entity edges, no record↔record predicates, no reification.
- **Objects are resolved entity ids, never literals.**
- **Relations are a curated descriptor registry, not open IRIs.**
- **No inference:** relations are stored facts; no transitive closure, ontology, or inverse-derivation.
- **Class is a closed, immutable 2-value enum** (`descriptive | access_bearing`) owned by the relation descriptor.
- **Flexibility comes from bounded qualifiers, not type proliferation** (property-graph-lite): an edge may carry a small, descriptor-allowed set of typed qualifiers (e.g. `confidence`, `is_primary`, `source_handle`).

A relation descriptor declares: canonical dotted id + model-facing alias, `class`, `cardinality`, `applies_to_entities`, `allowed_qualifiers`, `recall_effect` (`boost` | `gate` | `none`), and `anchor_projecting`.

### 5. One unified entity model; `work_surface` and `connection` are entity types (settled — "fork A")

There is **one** entity-resolution model and machinery. **Work surface** and **Connection** are entity *types* within it, not parallel tables:

- The work-surface resolution lifecycle **is** the entity-resolution lifecycle (§6); the work-surface anchors (observed checkout vs canonical git remote) **are** the handle model (§6). Keeping them parallel would duplicate the hard part (resolution).
- Canonical homes stay **per-type** via `canonical_ref`: externally-backed work surfaces resolve through **Connections** (Den control-plane, owner-scoped, [ADR-0040](adr-0040-connections-and-work-surface-presentation.md)); internally-backed ones resolve to Cabinet/Docket. No work-surface type is "bear-local canonical." `connection` is its own entity type; the work_surface↔connection edge is registry-owned (§1).
- Migration is **additive**: the existing work-surface resolution code becomes the **first per-type resolver**; nothing about today's work-surface behavior must break (consistent with ADR-0040's "no code/schema churn"). `work_surface_ref` may remain as a denormalized convenience during transition.

### 6. One resolution + trust lifecycle for all entity types (settled — "fork 3")

All entity types share one lifecycle, because the epistemic problem is type-agnostic: the Bear sees identifying signals over time, decides "new entity or known one?" under uncertainty, and must stay correctable.

- **Resolution states:** `observed → provisional → resolved → confirmed`, plus `rejected` and `merged`/`superseded` (reuse the [work-surface states](../architecture/memory-model.md#canonical-work-surface-anchors)).
- **Trust** is "who asserted the identity" (session/ACP token, normalized git remote, calendar id = strong; chat-text inference = weak), not the entity type.
- **Multiple entry points, one lifecycle:** authoritative-by-construction entities (a Den user from the token; a real Cabinet Mission id) enter directly at `confirmed`/`resolved` with `trust = asserted`. This is a fast path, not a second lifecycle.
- **Handles/aliases** are first-class: an entity has many `entity_handles` (`slack:U123`, `cabinet:author:ryan`, `mention:"Ryan"`, `email:…`) that attach/detach without destroying the entity.
- **Merge and split** are first-class, audited operations (correcting a bad merge = split + re-home the offending handle), exactly like work-surface merge/reconcile.
- **Per-type resolvers** plug type-specific canonicalization and handle-strength policy into the shared states.

### 7. Relation storage: two physical tables, descriptor-routed (settled — "fork C")

Because access-bearing relations are a **security boundary**, the safe path is structural rather than a remembered predicate. Two physical tables:

- `memory_relations` — descriptive record→entity relations; broad write access; boost/filter only.
- `memory_access_rules` — access-bearing record→entity relations; **the only table the recall gate consults**; append-only (so it *is* the visibility audit log); tighter write authority (e.g. `curate`/Den only); may enforce stricter constraints (e.g. gate targets must be `resolved`/`confirmed` + trusted — gating on a mergeable provisional identity is itself a leak risk).
- `memory_links` (read **view**) — `memory_relations ∪ memory_access_rules` with a `class` column, for cross-cutting reads (projection, "everything about entity E").

Properties:

- **The descriptor still owns the class and routes the write** to the correct table; writers never choose, so misfiling is impossible.
- This eliminates the *misclassification* failure mode entirely (a descriptive query cannot return access-bearing rows). The *forgot-the-gate* failure mode is closed in code by making an `AccessContext` (the `memory_access_rules` query) a **required input** to the recall assembler, so omission is a compile error rather than a silent open-fail.
- **`entity_ref` is retired** (the table identity encodes class; the relation layer carries aboutness). The "primary subject" used for path anchoring becomes a `subject` relation flagged `is_primary`, or the existing `logical_path`.
- Relations are **append-only with supersession/retraction** (`state ∈ {active, retracted, superseded}` + `supersedes_link_id`), consistent with `memory_records`. Retracting "this note is about Ryan" is a relation op; re-identifying Ryan is an entity-layer merge/split — clean separation.

### 8. Anchors generalize beyond work surface (settled — "fork 5")

Path anchors were never work-surface-specific; work surface was just the first entity type to earn them. **Resolved + salient** entities get projected anchors (`core/people/<id>/…`, `core/missions/<id>/…` alongside `core/work_surfaces/<slug>/…`); **transient / low-salience mentions stay query-derived views** over the relation layer. The promotion threshold is the same kind of "enough trusted signal + salience" decision used for work surfaces.

### 9. Portability

The **bear entity layer travels in the bear package** so relations never dangle on export ([bear package](../guides/bear-package.md)): the package carries `entities` + `entity_handles` (bear-local ids, types, display names, handles, resolution state, trust, provenance, `canonical_ref`). On **import**, `canonical_ref`s are re-linked against the destination registries; unresolved ones stay provisional. We export the Bear's **references and resolution**, not the registries' knowledge — **extracting Cabinet's own knowledge about an entity is explicitly deferred**. When a Bear is exported/imported, Den must communicate the set of entities the Bear is aware of.

### 10. Relationship to ADR-0015 and ADR-0041

- This ADR is the **native replacement** for [ADR-0015](adr-0015-multi-user-memory.md)'s per-user memory: "the human you are talking to" is a `person` entity at `confirmed`/asserted trust (identity from the session/ACP token), and per-user isolation is an `audience` access-bearing relation — not a Letta isolated block.
- Recall scoring ([ADR-0041](adr-0041-archival-recall-and-async-curation.md), [ADR-0038](adr-0038-platform-embedding-standard-and-derived-recall-index.md)) gains **entity filters**: descriptive relations boost; access-bearing relations gate; the Qdrant passage payload should carry resolved `entity_id`s for filtered recall.

## Schema deltas (sketch)

Canonical bear SQLite (`den-memory`). Final names/columns settle with D/E.

```sql
-- Bear entity layer (portable)
CREATE TABLE entities (
    entity_id      TEXT PRIMARY KEY,         -- bear-local stable id
    bear_id        TEXT NOT NULL,
    sequence_no    INTEGER NOT NULL,
    type           TEXT NOT NULL,            -- descriptor-owned vocabulary
    display_name   TEXT NULL,
    resolution     TEXT NOT NULL DEFAULT 'observed', -- observed|provisional|resolved|confirmed|rejected|merged
    trust          TEXT NOT NULL DEFAULT 'inferred', -- inferred|asserted
    canonical_ref  TEXT NULL,                -- cabinet:/den:/calendar: ref when resolved
    supersedes_entity_id TEXT NULL,          -- merge/split provenance
    metadata_json  TEXT NOT NULL DEFAULT '{}',
    created_at     TEXT NOT NULL
);

CREATE TABLE entity_handles (
    handle_id    TEXT PRIMARY KEY,
    bear_id      TEXT NOT NULL,
    entity_id    TEXT NOT NULL,
    handle_type  TEXT NOT NULL,              -- slack|email|cabinet_author|mention|git_remote|checkout|...
    handle_value TEXT NOT NULL,
    source       TEXT NULL,
    trust        TEXT NOT NULL DEFAULT 'inferred',
    state        TEXT NOT NULL DEFAULT 'active', -- active|detached
    created_at   TEXT NOT NULL
);

-- Relation layer: two descriptor-routed tables, same shape
CREATE TABLE memory_relations (         -- descriptive only
    link_id          TEXT PRIMARY KEY,
    bear_id          TEXT NOT NULL,
    sequence_no      INTEGER NOT NULL,
    src_memory_id    TEXT NOT NULL,
    entity_id        TEXT NOT NULL,
    relation         TEXT NOT NULL,         -- descriptor id; class=descriptive
    qualifiers_json  TEXT NOT NULL DEFAULT '{}',
    author_profile   TEXT NOT NULL,
    author_agent_id  TEXT NULL,
    confidence       TEXT NULL,
    state            TEXT NOT NULL DEFAULT 'active', -- active|retracted|superseded
    supersedes_link_id TEXT NULL,
    created_at       TEXT NOT NULL
);

CREATE TABLE memory_access_rules (      -- access-bearing only; audit surface
    -- identical columns to memory_relations; relation.class=access_bearing
    -- the ONLY table the recall gate consults
    link_id          TEXT PRIMARY KEY,
    bear_id          TEXT NOT NULL,
    sequence_no      INTEGER NOT NULL,
    src_memory_id    TEXT NOT NULL,
    entity_id        TEXT NOT NULL,         -- constraint: target entity resolution >= resolved
    relation         TEXT NOT NULL,
    qualifiers_json  TEXT NOT NULL DEFAULT '{}',
    author_profile   TEXT NOT NULL,
    author_agent_id  TEXT NULL,
    confidence       TEXT NULL,
    state            TEXT NOT NULL DEFAULT 'active',
    supersedes_link_id TEXT NULL,
    created_at       TEXT NOT NULL
);

-- Convenience read view for cross-cutting reads
CREATE VIEW memory_links AS
    SELECT *, 'descriptive'   AS class FROM memory_relations
    UNION ALL
    SELECT *, 'access_bearing' AS class FROM memory_access_rules;
```

- **Retire** `memory_records.entity_ref` (vestigial) and the legacy `memory_links` base table (`dst_ref_type`/`dst_ref`/`link_type`), migrating any future use to the two-table model.

## Consequences

- Per-user / per-entity memory delineation returns, generalized: persons, contacts, events, missions, orgs, work surfaces, and connections share one model, one lifecycle, one toolset, one anchor mechanism.
- Visibility gating is structural and auditable; misclassification of access-bearing relations is unrepresentable.
- New relationship and entity types are descriptor edits, not migrations; the design stays bounded (no RDF, no entity↔entity edges in memory).
- Work surfaces are unified into the entity model without a big-bang rewrite; the existing resolver is the first per-type resolver.
- Bear portability requires exporting the entity reference layer; Den must surface "entities this Bear knows" on export/import.
- New surface area to build: entity resolver(s), handle ingestion, merge/split tooling and audit, and entity-aware recall/anchor projection.

## Open questions

- **D — entity-id scheme + handle vocabulary.** Final entity-type set (`person`, `contact`, `org`, `event`, `mission`, `domain`, `work_surface`, `connection`, `artifact`, `place`?), handle-type vocabulary, and which handle types are "strong" (auto-resolve/merge) vs "weak" (stay provisional).
- **E — bear-package entity section.** Exact exported fields and the import re-linking rules for `canonical_ref` against destination registries (pending [bear package](../guides/bear-package.md) verification).
- **Relation vocabulary v1 final** — confirm `subject`/`source`/`participant`/`applies_when` (descriptive) and `audience`/`confined_to` (access-bearing); whether weak references are `subject` + low `confidence` or a distinct `mentions` relation.
- **Naming** — `memory_access_rules` vs `memory_visibility` vs `memory_access_scopes`; view materialization (likely plain view).
- **Work-surface canonical home** — confirm externally-backed surfaces resolve through Connections only, and whether `work_surface_ref` is retired once `entities` lands.
- **Salience threshold** for promoting an entity from query-derived view to projected anchor.
- **ADR-0015 disposition** — supersede vs amend, once this lands.
