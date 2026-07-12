# Phase 1 Garage Artifacts Product Plan

**Status:** Active Phase 1 product slice.

This plan splits the Garage/artifacts product surface out of [`PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md`](PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md). It does not replace [`ARTIFACT_REFS_IMPLEMENTATION_PLAN.md`](ARTIFACT_REFS_IMPLEMENTATION_PLAN.md); artifact refs are the keystone handle, while this plan covers the Phase 1 operator/user UX around artifacts.

Related plans:

- [`ARTIFACT_REFS_IMPLEMENTATION_PLAN.md`](ARTIFACT_REFS_IMPLEMENTATION_PLAN.md) — canonical opaque refs and registry.
- [`PHASE1_ROUTINES_PLAN.md`](PHASE1_ROUTINES_PLAN.md) — routine outputs.
- [`SKILLS_IMPLEMENTATION_PLAN.md`](SKILLS_IMPLEMENTATION_PLAN.md) — large skill trees/export artifacts.
- ADR-0004 artifacts/Garage architecture.

## Goal

Make generated files, human uploads, routine outputs, skill artifacts, and work-run outputs visible and retrievable through Den policy without treating them as memory or Cabinet documents.

## Scope

### 1. Storage and metadata

Artifact metadata should include, where known:

- artifact ref or storage key;
- Bear, user, conversation, run, routine, or Docket provenance;
- source kind: human upload, agent output, routine output, skill package/tree, work-run log/output;
- content type, size, created time, retention/GC state;
- access policy and redaction/sensitivity flags where available.

### 2. Product surfaces

- Artifact list/detail pages for operators.
- User-facing download/open links where membership and policy allow.
- Upload affordance where already supported or cheap to add safely.
- Links from conversations, routines, Docket evidence, and skill package views.

### 3. Retention and safety

- Show retention/GC state clearly.
- Keep Cabinet attachments separate from Garage artifacts.
- Provide explicit future promotion path to Cabinet; do not blur the storage models.

## Non-goals

- Do not implement a parallel file model outside artifact refs/Garage.
- Do not make artifacts semantic memory.
- Do not move Cabinet attachments into the artifacts bucket/lifecycle.
- Do not expose presigned URLs or raw storage keys beyond policy boundaries.

## Implementation steps

1. Treat artifact refs as the preferred dependency. If refs are not landed, keep this plan at the UI/storage-seam level or implement only what can migrate cleanly.
2. Inventory existing Garage configuration, upload/download handlers, and artifact metadata.
3. Add artifact list/detail UI with provenance and retention fields.
4. Link artifact refs from conversations, routine runs, Docket evidence, and skill package surfaces where those refs exist.
5. Add guarded upload/download paths with membership and policy checks.
6. Document retention/GC behavior in the UI or operator docs.
7. Add a smoke check for policy-filtered artifact listing or download authorization.

## Acceptance criteria

- Operators can see artifacts with provenance, size/type, and retention state.
- Authorized users can retrieve allowed artifacts without raw storage leakage.
- Routine/runtime outputs can create or link an artifact record.
- Cabinet attachments remain visibly separate.
- The implementation aligns with artifact refs instead of inventing a competing handle.
