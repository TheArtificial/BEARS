# Cabinet Contract

**Status:** Adopted — Phase 0 types and contract checks live in `den-cabinet`; the Phase 1 facade (`den_service::cabinet`) implements the Phase 1 subset. (Phase 0 of the [Cabinet implementation plan](../roadmap/CABINET_IMPLEMENTATION_PLAN.md))
**Related:** [Bear charter and Cabinet Missions](bear-charter-and-cabinet-missions.md), [ADR-0004 — Artifacts, Garage, and Cabinet separation](../decisions/adr-0004-artifacts-garage.md), [ADR-0008 — Research ingestion uses Cabinet](../decisions/adr-0008-cabinet-reading-pipeline.md), [Identity and membership](identity-and-membership.md)

This document is the provider-neutral contract for Cabinet: the typed identities, minimum records, Den facade operations, and authorization inputs/outcomes that every Cabinet implementation and client must honor. The backing provider (Den Postgres first; anything later) is an implementation detail behind this contract and must not leak into it.

## Summary

- Cabinet is Den's **single** shared knowledge layer: one Cabinet per Den deployment, partitioned by Missions and collections — not multiple cabinet instances.
- Humans and authorized Bears **edit directly** (true wiki). Every write produces an immutable version; revision history is the safety net. Review/approval is a policy hook reserved for Phase 2, not a Phase 1 gate.
- Den owns the facade, authorization, and policy. Agent tools and human UI both go through the facade; nothing reads or writes the backing store directly.
- Every operation takes an **explicit actor scope** (user or Bear + stance) and, where relevant, an explicit Mission scope. No special strings, no ambient identity.
- Cabinet items are knowledge records. Artifact refs hold content payloads (ADR-0004). External sources stay external. Derived recall passages are rebuildable projections. These four never merge.

## Identities

All Cabinet refs are Den-minted, opaque, and stable for the entity lifetime. Following the artifact-ref convention, a ref is a fixed prefix plus a 32-character lowercase hex suffix. Models and clients never invent refs.

| Entity | Ref prefix | Example |
|--------|-----------|---------|
| Cabinet item | `cabinet_item_` | `cabinet_item_01f3…` |
| Item version | `cabinet_version_` | `cabinet_version_9ab0…` |
| Collection | `cabinet_collection_` | `cabinet_collection_44c1…` |
| Mission | `mission_` | `mission_7e2d…` |
| Source link | `cabinet_source_` | `cabinet_source_be55…` |
| Attachment link | `cabinet_attachment_` | `cabinet_attachment_0c9f…` |
| Review record | `cabinet_review_` | `cabinet_review_d21a…` |

Notes:

- The protocol field name for an item ref is `cabinet_ref` (matches ADR-0004 and the existing `cabinet_ref` entity-handle type in `den-memory`).
- No ref is any other ref's prefix; parsing is unambiguous.
- A ref is not an object key, URL, filesystem path, title, or slug. Slugs/titles may exist for humans but are mutable display data, never identity.

## Records

Minimum required fields. Providers may store more; clients may rely only on what is here.

### Cabinet item

The durable knowledge object — a wiki document or typed knowledge record.

| Field | Requirement |
|-------|-------------|
| `cabinet_ref` | required, immutable |
| `kind` | required; `document` is the only Phase 1 kind; the enum is open for later kinds (`glossary_entry`, `decision`, `reference`, …) |
| `title` | required, mutable display data |
| `current_version` | required after first write; points at the latest published `cabinet_version_` |
| `collection_ref` | optional; at most one collection per item |
| `mission_ref` | optional; at most one Mission per item |
| `created_by` | required actor provenance (see Actor scope) |
| `created_at` | required |
| `lifecycle` | required: `active`, `archived`, `deleted` (tombstone) |

### Item version

An immutable snapshot of item content. Versions are the citation unit and the revision history.

| Field | Requirement |
|-------|-------------|
| `version_ref` | required, immutable |
| `cabinet_ref` | required; the owning item |
| `revision` | required; per-item monotonically increasing integer starting at 1 |
| `content` | required; Phase 1 content is Markdown text |
| `content_sha256` | required; hash of the canonical content bytes |
| `authored_by` | required actor provenance |
| `authored_at` | required |
| `base_version` | required except on revision 1; the version the author edited from (concurrency evidence, not a merge mechanism) |
| `review` | required review state (see Review state) |

A finalized version is never mutated or deleted while any citation to it may exist. Item deletion tombstones the item; versions remain readable to actors authorized on the item at tombstone time, for citation integrity. Hard purge is an operator/compliance action outside this contract's model-facing operations.

### Collection

An organizational grouping within the Cabinet, and the policy attachment point below Mission.

| Field | Requirement |
|-------|-------------|
| `collection_ref` | required, immutable |
| `name` | required, mutable |
| `mission_ref` | optional; a collection may belong to a Mission |
| `policy` | required policy record (see Authorization) |
| `created_by`, `created_at` | required provenance |

### Mission scope

A Mission is the shared cross-Bear work-and-knowledge container ([bear-charter-and-cabinet-missions.md](bear-charter-and-cabinet-missions.md)). This contract defines only what Cabinet needs from it:

| Field | Requirement |
|-------|-------------|
| `mission_ref` | required, immutable |
| `name` | required |
| `user_members` | required set of user IDs |
| `bear_members` | required set of Bear IDs |
| `policy` | required policy record |

Mission lifecycle and non-Cabinet Mission behavior are out of scope here. If Missions later become a first-class Den entity, that entity must present at least this shape to Cabinet.

### Source link

Provenance from a Cabinet item to material outside Cabinet. This is how research ingestion (ADR-0008) and manual citation attach origin without Cabinet owning external data.

| Field | Requirement |
|-------|-------------|
| `source_ref` | required, immutable |
| `cabinet_ref` | required; the citing item |
| `source_kind` | required: `url`, `offline`, `artifact`, `conversation`, `external_record` |
| `locator` | required; normalized URL, synthetic scheme (`book://isbn/…`, `offline://…`), `artifact_…` ref, conversation ID, or provider-record identity — matching `source_kind` |
| `role` | required: `origin`, `citation`, `related` |
| `created_by`, `created_at` | required provenance |

A source link is provenance, not content. Cabinet never fetches, caches, or owns the bytes behind a `url` or `external_record` locator.

### Attachment link

Binding from a Cabinet item to a Den artifact ref (ADR-0004 §9). Reserved until Phase 3; the record shape is fixed now so nothing else squats on it.

| Field | Requirement |
|-------|-------------|
| `attachment_ref` | required, immutable |
| `cabinet_ref` | required |
| `artifact_ref` | required; a finalized Den artifact ref |
| `role` | required: `source_pdf`, `generated_report`, `image`, `data`, `other` (open enum) |
| `created_by`, `created_at` | required provenance |

Cabinet owns item/ACL policy for the link. The artifact registry owns payload identity, lifecycle, and read authorization of the bytes. Linking never copies content and never exempts the reader from artifact read policy.

### Review state

Phase 1 is direct-edit: every version publishes immediately with `review: none`. The state exists so Phase 2 review policy has a place to land without a schema break, and so the existing deferred `cabinet_update` curate action has a target.

| State | Meaning |
|-------|---------|
| `none` | published directly; no review required by policy at write time |
| `pending` | version exists but is not `current_version`; awaiting review (Phase 2) |
| `approved` | reviewed and published (Phase 2) |
| `rejected` | reviewed and not published; retained in history (Phase 2) |

A `cabinet_review_` record (reviewer actor, decision, rationale, timestamps) accompanies any transition out of `pending`. Phase 1 implementations must reject attempts to create `pending` versions rather than silently publishing them.

## Actor scope

Every facade operation takes an explicit `ActorScope`:

- exactly one of `user_id` (human) or `bear_id` + `stance` (Bear), and
- optional call provenance: `conversation_id`, `run_id`, `task_id` when the write originates from a run.

There are no service-identity or wildcard actors on the model-facing facade. Ingestion services (ADR-0008) act as the Bear or user they are configured to publish for.

Actor provenance recorded on items, versions, and links preserves this scope verbatim: `{ actor_kind: user|bear, user_id?, bear_id?, stance?, conversation_id?, run_id? }`.

## Operations

The Den facade exposes these operations. Signatures are conceptual; transport (tool descriptor, HTTP route) is an implementation concern, but names, inputs, outputs, and authority requirements are contract.

| Operation | Phase | Authority | Behavior |
|-----------|-------|-----------|----------|
| `cabinet_search(scope, query, filters?) → [item summary]` | 1 | read | Metadata/text search over items the actor may read. Filters: `kind`, `collection_ref`, `mission_ref`, `lifecycle`. Results carry `cabinet_ref`, `current_version`, title, kind, scope binding, and updated-at. Unreadable items are absent, not redacted. |
| `cabinet_read(scope, cabinet_ref, version_ref?) → item + version` | 1 | read | Returns the item record and the requested version (default `current_version`), including full content, provenance, source links, and (Phase 3) attachment links. |
| `cabinet_history(scope, cabinet_ref) → [version summary]` | 1 | read | Revision list: `version_ref`, revision, author provenance, timestamp, review state, content hash. |
| `cabinet_create_item(scope, kind, title, content, collection_ref?, mission_ref?, source_links?) → item + version 1` | 1 | write | Creates the item and its first published version atomically. Scope binding is fixed at creation; rebinding is a Phase 2 organize operation. |
| `cabinet_update_item(scope, cabinet_ref, content, base_version, title?) → new version` | 1 | write | Appends a new immutable version and advances `current_version`. If `base_version` ≠ `current_version` at commit, fail with a structured conflict carrying the current version ref — the caller re-reads and reconciles; the facade never merges. |
| `cabinet_archive_item(scope, cabinet_ref)` / `cabinet_restore_item` | 1 | write | Lifecycle transitions `active ↔ archived`. Tombstone deletion (`deleted`) is write-authority plus policy; history remains readable per the version-immutability rule. |
| `cabinet_link_source(scope, cabinet_ref, source_kind, locator, role)` / `cabinet_unlink_source` | 1 | write | Manage source links. Unlink removes the link record; it never alters versions. |
| `cabinet_organize(scope, cabinet_ref, collection_ref?, mission_ref?)` | 2 | write on item + destination | Rebind an item's collection/Mission. Contract-reserved; not exposed in Phase 1. |
| `cabinet_review(scope, cabinet_ref, version_ref, decision, rationale) → review record` | 2 | review | Approve/reject a `pending` version; approval advances `current_version`. Contract-reserved. |
| `cabinet_link_attachment(scope, cabinet_ref, artifact_ref, role)` / `cabinet_unlink_attachment` | 3 | write + artifact read | Manage attachment links. Requires the actor to hold artifact read authority at link time. Contract-reserved until artifact-ref content transfer lands. |

Error taxonomy (structured, stable): `NotFound` (also returned for unauthorized reads — deny must not confirm existence), `NotAuthorized` (writes only, where existence is already readable), `Conflict` (stale `base_version`), `ValidationError`, `PolicyError` (operation valid but disallowed by collection/Mission/kind policy).

## Authorization

### Inputs

An authorization decision consumes, in order:

1. **Actor identity** — the explicit `ActorScope`.
2. **Den membership** — the user's or Bear's standing in this Den ([identity-and-membership.md](identity-and-membership.md)). Non-members get nothing.
3. **Mission membership** — when the item (or its collection) is bound to a Mission, the actor must be a Mission member for any access. Mission binding narrows access; it never widens access to unbound material.
4. **Collection/kind policy** — per-collection policy may further restrict: read-only for Bears, write requires review (Phase 2), specific kinds disallowed.
5. **Requested authority** — `read`, `write`, or `review`.

### Outcomes

The decision is `allow` or `deny` with a structured, logged reason. Rules:

- **Default for unbound items** (no Mission, no collection): readable and writable by every Den member — the open-wiki default. Deployments wanting stricter defaults set a default collection policy, not a special case.
- **Mission-bound items**: read and write require Mission membership (user or Bear). This is the plan's Phase 2 exit condition — a Mission governs its knowledge without broadening access to unrelated Cabinet material.
- **Bears are members, not superusers**: a Bear's access derives from its Mission/collection standing exactly as a user's does. Stance may narrow (e.g. policy may deny `work`-stance writes) but never widens.
- **Search and read denial is silent**: filtered from search, `NotFound` on read.
- Every mutating decision (allow or deny) is auditable: actor scope, operation, target refs, policy inputs, outcome.

## Distinctions (what Cabinet is not)

| Handle | Owner | Nature |
|--------|-------|--------|
| `cabinet_item_` / `cabinet_version_` | Cabinet | curated knowledge record and its immutable revision |
| `artifact_…` | Artifact registry (ADR-0004) | content payload/blob/external snapshot with provenance |
| Source locator (URL, `book://…`) | The external world | origin provenance; never Cabinet-owned bytes |
| Derived recall passage | Recall index (ADR-0038) | rebuildable, ACL-filtered projection; never canonical |

Consequences:

- Citing a Cabinet item from elsewhere in Den (Docket evidence, conversations) uses an artifact of kind `cabinet_document_snapshot` recording `(cabinet_ref, version_ref)` — per ADR-0004 §4.
- Recall passages derived from Cabinet content carry `(cabinet_ref, version_ref)` provenance and are filtered by Cabinet read authority at query time. Deleting or archiving an item must propagate to derived passages (Phase 3).
- Bear memory tools never write Cabinet, and Cabinet operations never write Bear memory. The existing `memory_write_entry` rejection of `cabinet_write` content stands.

## Invariants

1. Den mints all Cabinet refs; models, clients, and providers do not.
2. Every operation carries an explicit actor scope; there is no ambient or default actor.
3. Every item version is immutable, hash-stamped, and citable forever; content changes are new versions.
4. `current_version` only moves forward through `cabinet_update_item` or (Phase 2) an approved review.
5. Every record carries actor provenance sufficient to answer who wrote this, as whom, from where.
6. Authorization is evaluated on every operation against current membership and policy — never cached across actors, never bypassed by provider access.
7. Mission binding narrows access and never widens it; unauthorized existence is not disclosed.
8. Cabinet stores no artifact bytes and no external-source bytes.
9. Provider changes must not change refs, operation semantics, authority outcomes, or provenance fields.

## Contract checks

Phase 0 exits with an assertion-style check suite (Rust tests colocated with the contract types) that enforces, independent of any provider:

- ref mint/parse round-trips for every prefix, and rejection of malformed or cross-kind refs;
- every operation input type fails construction without an actor scope;
- item, version, source-link, and attachment-link records fail validation when identity, provenance, scope, or authority fields are missing;
- version construction rejects a missing `base_version` after revision 1 and any mutation of a finalized version;
- Phase 1 review-state handling rejects `pending` creation.

## Phase applicability

| Contract element | Phase 0 (types + checks) | Phase 1 (facade) | Later |
|------------------|--------------------------|------------------|-------|
| Item, version, collection, source link records | defined | implemented | — |
| Mission scope record | defined | membership-checked when present | Mission management (2) |
| Search/read/create/update/history/archive/source ops | defined | implemented | — |
| Organize, review ops + review states beyond `none` | defined | rejected | implemented (2) |
| Attachment links | defined | rejected | implemented (3) |
| Recall passage handoff | distinction defined | — | implemented (3) |

## Documentation obligations

Implementing Phase 1 against this contract requires updating `MODEL_EXPERIENCE.md` (new model-visible tools and their guidance) and creating a `docs/guides` entry covering permissions, direct-edit behavior, revision history, and source-link limitations, per the implementation plan's standing obligation.
