# Cabinet research ingestion plan

**Status:** Proposed  
**Decision source:** [ADR-0008 — Cabinet research ingestion](../decisions/adr-0008-cabinet-reading-pipeline.md)  
**Cabinet contract:** [Cabinet implementation plan](CABINET_IMPLEMENTATION_PLAN.md)

## Boundary

This plan delivers the self-hosted reading, bookmarking, highlighting, and research-source pipeline. It **uses** Cabinet as an authorized destination for selected durable research material and provenance links; it does not define Cabinet's item model, permissions, provider, or agent-facing API.

Karakeep remains the canonical highlight/bookmark store selected by ADR-0008. Wallabag and KOReader remain reading clients/integrations. Qdrant is a derived retrieval index governed by the [derived recall plan](DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md), not canonical research or Cabinet storage.

## Goals

- Capture reading status, source metadata, bookmarks, and highlights from the selected self-hosted stack.
- Normalize and deduplicate collected sources/highlights with durable provenance.
- Publish approved research material to Cabinet through the Cabinet facade and link it to the originating external record.
- Produce access-filterable recall passages only after Cabinet ingestion and recall contracts exist.

## Non-goals

- Replacing Cabinet shared knowledge or implementing its ACLs/review model.
- Allowing collectors to bypass Cabinet policy or write directly to its backing provider.
- Treating Qdrant as a canonical source.

## Phases

### Phase 0 — Ingestion contract

- Define typed external-source, bookmark, highlight, read-status, provenance, and idempotency identities.
- Specify which records remain in the research store and which become Cabinet proposals/items.
- Define conflict, deletion, retry, and secret-handling behavior for collectors.

**Exit:** repeat collection of the same source/highlight is idempotent and preserves origin identity.

### Phase 1 — Reading and bookmark synchronization

- Implement the selected Karakeep, Wallabag, and KOReader flows from ADR-0008.
- Synchronize read-later/read state with explicit ownership and conflict handling.
- Keep provider credentials in deployment configuration, not plans, source records, or Cabinet content.

**Exit:** a representative article can move through the self-hosted reading flow without creating duplicate bookmarks or statuses.

### Phase 2 — Highlight and source normalization

- Collect highlights from supported sources, normalize text/notes/source metadata, and deduplicate before writing to Karakeep.
- Support non-URL/offline sources with explicit provenance rather than fabricated URLs.

**Exit:** every collected highlight has a stable source identity or an explicit offline-source provenance record.

### Phase 3 — Cabinet publication and recall handoff

- Submit selected research material through Cabinet's authorized proposal/create path.
- Link the resulting Cabinet item to the source/highlight identity without making the external store Cabinet's provider.
- Hand eligible Cabinet content to the derived recall producer after its ACL-filtered passage contract is available.

**Exit:** a reviewed research item can be cited in Cabinet and recalled only within its authorized scope.

## Documentation obligations

Every phase that changes model-visible research discovery, source citation, or Cabinet publication must review and update `MODEL_EXPERIENCE.md`. Every user- or operator-visible integration must create or update documentation in `docs/guides`, including setup, data ownership, synchronization limits, provenance, and recovery behavior.
