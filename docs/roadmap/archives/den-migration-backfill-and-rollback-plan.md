# Den Migration Backfill and Rollback Plan

> **Historical (2026-06).** Phase 8 production backfill is retired because there are no production Letta-runtime Bears to migrate. This document is retained as historical Letta extraction/backfill planning and should not drive active roadmap work without an explicit revisit. Optional archived-bundle imports use [`MEMFS_TO_SQLITE_ETL_IMPLEMENTATION_PLAN.md`](MEMFS_TO_SQLITE_ETL_IMPLEMENTATION_PLAN.md).

This document defined the planning baseline for migration/backfill/rollback mechanics in the Letta extraction.

## Purpose

Even with a correct target architecture, Letta extraction is unsafe unless Den also defines:

- how legacy state maps into Den-owned state,
- how mixed-origin periods are handled,
- how read/write switching is staged,
- and how rollback remains possible when confidence is incomplete.

## Architectural stance

The goal is to move toward Den-owned runtime and storage, not to create a generalized runtime-provider platform.

Migration tooling should support staged Letta removal while preserving a Den-native conceptual model.

## Required planning areas

### 1. State mapping

Migration planning must define how the following map from Letta-era behavior into Den-owned state:

- conversation identifiers,
- session identifiers,
- transcript/history records,
- approvals and tool-turn state where needed,
- role/runtime bindings,
- and any remaining provider-side metadata required for compatibility windows.

### 2. Backfill classes

Not all data needs identical treatment. Planning should distinguish:

- canonical transcript backfill,
- read-model backfill,
- migration of reference ids/mappings,
- optional archival/index rebuilds,
- and operator-only diagnostic/state reconstruction.

### 3. Dual-write / mixed-origin periods

Migration planning must define:

- where dual-write is required,
- where write-once/read-fallback is sufficient,
- how canonical-read eligibility is determined,
- and how mixed-origin sessions remain explainable to operators.

### 4. Rollback rules

Rollback planning must define:

- what signals trigger rollback consideration,
- which reads/writes can be switched back independently,
- how partially migrated state is handled,
- and what data is preserved if execution routing is rolled back.

### 5. Operator procedure

Migration is not complete without an operator-facing procedure for:

- enabling a migration slice,
- validating it,
- observing mixed-origin behavior,
- and disabling or rolling back safely.

## Planning principles

### Prefer reversible read-switches over destructive rewrites

Where possible, keep canonical data append-oriented and reversible, and prefer explicit read-switch criteria over destructive replacement.

### Preserve provenance during mixed-origin periods

Operators should be able to tell whether a record, session, or read path is Den-primary, Letta-primary, backfilled, or mixed-origin.

### Separate execution rollback from storage rollback

A rollback of execution routing does not necessarily require rolling back Den-owned stored transcript or read models.

### Avoid migration-only abstractions becoming permanent architecture

Backfill and rollback tooling should help the migration complete, not impose a long-term provider-shaped domain model on Den.

## Minimum v1 migration-plan expectations

A v1 migration/backfill/rollback plan is acceptable if it provides:

- explicit state mapping categories,
- explicit mixed-origin and dual-write guidance,
- explicit rollback criteria and boundaries,
- and an operator-oriented rollout/rollback checklist.
