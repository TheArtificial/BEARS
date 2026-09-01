# Cabinet implementation plan

**Status:** In progress — Phase 0 landed (contract doc + `den-cabinet` types/checks); Phase 1 implemented (Postgres storage + `den_service::cabinet` facade, `cabinet_search/read/history/create/update/source_link/lifecycle` tools with the per-Bear `bears.cabinet_enabled` gate, person-only item deletion, `/cabinet` wiki UI, [user guide](../guides/cabinet.md)); Phases 2–4 not started.  
**Scope:** Shared, human-editable, policy-controlled knowledge for Den.  
**Decisions (2026-08):** one Cabinet layer per Den (Missions/collections partition it, not multiple cabinet instances); humans and authorized Bears edit directly (true wiki — every write publishes an immutable version, revision history is the safety net); review/approval is Phase 2 policy, not a Phase 1 gate; Phase 1 storage is Den Postgres behind the provider-neutral facade.  
**Related architecture:** [Cabinet contract](../architecture/cabinet-contract.md), [Bear charter and Cabinet Missions](../architecture/bear-charter-and-cabinet-missions.md), [ADR-0004 — Artifacts, Garage, and Cabinet separation](../decisions/adr-0004-artifacts-garage.md)

## Boundary

Cabinet is Den's shared knowledge layer: people and authorized Bears can create, read, organize, review, and update durable knowledge. A Cabinet Mission is a shared work and knowledge container; it is not a separate storage system.

Cabinet is **not** the reading stack, a bookmark manager, a highlight store, or a vector database. The reading/research pipeline is a Cabinet client and source producer; its delivery belongs to [Cabinet research ingestion](CABINET_RESEARCH_INGESTION_PLAN.md). Artifact payload retention belongs to the [artifact refs plan](ARTIFACT_REFS_IMPLEMENTATION_PLAN.md), and derived retrieval belongs to the [derived recall plan](DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md).

Den owns the agent-facing facade, authorization, and policy. The backing provider is an implementation decision behind that boundary; no active plan assumes Outline, Karakeep, or another provider is Cabinet itself.

## Goals

- Give humans a durable, editable shared-knowledge surface.
- Give authorized Bears a bounded Den-mediated API for finding, reading, creating, and updating Cabinet material.
- Support Cabinet Missions, collections, documents/items, source links, metadata, and artifact attachments without conflating them.
- Preserve provenance, review state, authorization, and immutable citations/snapshots where needed.

## Non-goals

- Replacing per-Bear canonical memory.
- Implementing research/reader synchronization or owning third-party reading data.
- Defining artifact blob storage or derived-vector indexing internals.
- Selecting a provider before the Cabinet contract and constraints are validated.

## Phases

### Phase 0 — Contract and boundary

- Define strongly typed identities and minimum records for Cabinet item, collection, Mission scope, source link, attachment link, review state (reserved for Phase 2 policy), and immutable item version/snapshot.
- Define the distinction between a Cabinet catalog/item, artifact ref, external source, and derived recall passage.
- Specify Den operations: search, read, history, direct create/update (each write publishes an immutable version), archive/restore, and source link/unlink, plus contract-reserved organize, review/approve, and attachment link/unlink operations; all accept explicit Bear/user/Mission scope rather than special strings.
- Define authorization inputs and outcomes: user membership, Bear membership, Mission membership, collection/kind policy, and read/write/review authority.

**Exit:** a provider-neutral API/schema contract and an assertion-style contract check for required IDs, scope, provenance, and authority fields.

### Phase 1 — Den facade and minimal shared knowledge

- Implement provider-neutral Cabinet storage behind Den's policy facade (Den Postgres first).
- Add authorized search/read and direct create/update flows for a minimal document/item type: humans and authorized Bears both edit directly, every write appends an immutable version, and stale-base updates fail with a structured conflict rather than merging.
- Record author, source/provenance, revision identity, and review state (`none` under Phase 1 direct-edit; `pending` versions are rejected until Phase 2).
- Keep direct backing-store access out of agent tools.

**Exit:** a human and an authorized Bear can share one item through Den, while an unauthorized Bear cannot read or alter it.

### Phase 2 — Missions, organization, and review

- Bind Cabinet items to optional Missions and collections.
- Add optional review/approval policy for Bear-authored changes where configured; direct edit remains the default.
- Add human-visible revision/provenance views and stable snapshot citations.

**Exit:** a Mission can govern shared knowledge without broadening access to unrelated Cabinet material.

### Phase 3 — Attachments and recall integration

- Integrate Cabinet attachments through artifact refs; Cabinet retains item/ACL policy while artifacts retain payload identity and lifecycle.
- Produce access-filterable passages for the derived recall index; recall remains derived and rebuildable.
- Do not begin this phase until the artifact-ref and recall contracts are available.

**Exit:** an authorized Cabinet item can cite an immutable attachment/version and contribute filtered recall passages without becoming a blob store.

### Phase 4 — Provider selection, migration, and operations

- Select or implement a provider only after Phase 1 validates the facade contract.
- Document migration, backup/export, audit, retention, operational ownership, and failure behavior.
- Keep provider-specific adapters behind the Phase 0 contract.

**Exit:** provider changes do not change model tool semantics, authority checks, or Cabinet identities.

## Documentation obligations

Every phase that changes model-visible Cabinet behavior must review and update `MODEL_EXPERIENCE.md`. Every user- or operator-visible phase must create or update the applicable documentation in `docs/guides`, including permissions, proposal/review behavior, and any source or attachment limitations.
