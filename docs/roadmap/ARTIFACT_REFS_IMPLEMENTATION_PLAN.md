# Artifact refs implementation plan

**Status:** Proposed — implement first  
**Primary ADR:** [ADR-0004: Artifacts, Garage (S3), and Cabinet separation](../decisions/adr-0004-artifacts-garage.md)  
**Related ADRs:** [ADR-0006](../decisions/adr-0006-bear-work-surfaces.md), [ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md), [ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md), [ADR-0053](../decisions/adr-0053-stance-scoped-delegated-runs.md)

## Goal

Make artifact refs the general Den handle for durable content objects: uploaded files, generated reports, patches, screenshots, logs, Cabinet attachments, Docket evidence, and future delegated-run outputs.

Artifact refs should be Den-minted, registry-backed, permission-checked, provenance-rich, and stable after finalization. Models and clients should never use object keys, filesystem paths, signed URLs, or model-invented IDs as protocol handles.

## Relationship to Jobs, runs, and other Den subjects

Artifacts are durable content/evidence/output handles. Jobs are durable work-management state. Runs are execution attempts. Keep those nouns separate:

- Jobs define the work objective, task tree, acceptance criteria, status, commit/work-surface policy, and completion decisions.
- Runs record what happened during an execution attempt.
- Artifacts hold or point at durable content such as logs, reports, diffs, screenshots, uploads, and receipts.
- Artifact existence is evidence, not truth: criterion/task/job status remains an explicit Docket decision that may cite artifact refs.

Docket and runtime records should link to artifact refs; they should not own blobs or use object keys/workspace paths as durable identity. The first implementation should prefer one small generic attachment/link model over per-surface special cases unless policy or query needs force a split:

```text
artifact_links
- artifact_ref
- subject_kind    -- conversation | job | task | run | criterion | delegated_run | cabinet_item | etc.
- subject_id
- role            -- input | source | output | evidence | test_report | diff | completion_receipt | etc.
- created_by_role
- created_at
```

Job-level artifact collections can be projections over direct job links plus task/run/criterion links. Do not add a separate "job artifact collection" object until users need manual curation, ordering, or policy that links cannot cover.

Lifecycle and authorization remain artifact-service responsibilities. A Job link may request or justify promotion, for example from ephemeral run output to job-audit evidence, but Job visibility must not bypass artifact read policy.

## Non-goals

- No full Cabinet product rebuild.
- No general document management UI.
- No delegated-run implementation dependency; artifacts should land first and be useful without subagents.
- No content-addressed public refs. Store hashes for integrity, but keep refs opaque.
- No cross-storage abstraction beyond what artifact refs need now.
- No requirement that artifact refs be attached to Jobs. Conversation-scoped and delegated-run-scoped artifacts are valid.

## ADR fulfillment

This plan fulfills ADR-0004 by implementing:

- [ ] Den-minted opaque `artifact_...` refs.
- [ ] Required artifact registry rows for every artifact ref.
- [ ] Storage backends hidden behind the registry (`garage_artifacts`, future `garage_cabinet`, `db_text`, `external_ref`, etc.).
- [ ] Two-phase creation: reserve -> write/upload -> finalize.
- [ ] Pending artifacts unreadable as finalized content.
- [ ] Finalized artifacts treated as stable snapshots.
- [ ] Permission checks on read, write, mount/copy, attach, promote, and delete.
- [ ] Provenance links to conversation, run, task, job, work surface, creating stance, source, and user/bear where applicable.
- [ ] Generic artifact links from refs to Den subjects such as conversations, jobs, tasks, runs, criteria, delegated runs, and Cabinet items.
- [ ] Lifecycle states for pending, ephemeral, promoted, Cabinet-durable, archived, and deleted.
- [ ] GC that respects lifecycle and Cabinet attachment state.

## Implementation phases

### Phase 1 — Registry and core service

**Goal:** Create the canonical artifact identity and lifecycle API.

- [ ] Add artifact registry schema and migration.
- [ ] Add artifact model types with `artifact_ref`, `kind`, `storage_kind`, metadata, provenance, visibility, lifecycle, and timestamps.
- [ ] Add service methods:
  - [ ] `reserve_artifact(...) -> artifact_ref`
  - [ ] `finalize_artifact(...)`
  - [ ] `get_artifact_metadata(...)`
  - [ ] `authorize_artifact_access(...)`
  - [ ] `mark_artifact_deleted(...)`
- [ ] Generate refs in Den only, using an opaque stable ID format such as `artifact_` + ULID/UUIDv7.
- [ ] Add small service-level checks for reserve/finalize/read authorization and invalid lifecycle transitions.

**Exit gate:** Den can reserve, finalize, read metadata for, and lifecycle-check an artifact without exposing storage keys.

### Phase 2 — Garage-backed upload/download

**Goal:** Make artifact refs usable for real bytes.

- [ ] Add presigned upload URL issuance for pending Garage artifacts.
- [ ] Add finalize path that records object key, size, content type, and SHA-256.
- [ ] Add presigned download/preview URL issuance gated by artifact read authorization.
- [ ] Enforce pending-vs-finalized readability.
- [ ] Add GC candidate query for expired ephemeral artifacts.
- [ ] Add one integration or env-gated check for reserve -> upload/write substitute -> finalize -> metadata/read URL flow.

**Exit gate:** Human uploads and agent-produced files can become finalized artifact refs backed by Garage.

### Phase 3 — Conversation and Docket evidence integration

**Goal:** Replace ad hoc content handles in common Den surfaces.

- [ ] Allow conversation events/messages to cite artifact refs for attachments and generated outputs.
- [ ] Add generic artifact link/attachment records for Den subjects, including at least conversation, job, task, run, criterion, and delegated-run anchors.
- [ ] Allow Docket jobs/tasks/runs/criteria evidence to attach artifact refs with roles such as `input`, `source`, `output`, `evidence`, `test_report`, `diff`, `runtime_checkpoint`, or `completion_receipt`.
- [ ] Add run/task provenance when artifacts are created by work/runtime activity.
- [ ] Render artifact refs in task/run completion receipts.
- [ ] Keep criterion/task/job state separate from artifact presence; completion decisions cite evidence refs but are not implied by them.
- [ ] Stop leaking object keys or workspace paths as durable evidence handles.

**Exit gate:** A task/run can produce or cite artifact-backed evidence, and the UI/model layer can display the artifact metadata by ref.

### Phase 4 — Cabinet attachment integration

**Goal:** Let Cabinet use artifact refs without conflating Cabinet records with blob storage.

- [ ] Add Cabinet attachment link model: Cabinet item -> artifact ref + attachment role.
- [ ] Support `cabinet_durable` lifecycle or equivalent retention promotion for attached artifacts.
- [ ] Ensure artifact GC never deletes Cabinet-durable attachments.
- [ ] Preserve Cabinet ACL/retention policy separately from ephemeral run artifact policy.
- [ ] Add attach/detach operations with authorization checks.

**Exit gate:** Cabinet items can attach artifact refs as source documents, generated reports, or evidence, with durable retention semantics.

### Phase 5 — Work surface produce/consume flows

**Goal:** Make the artifact/work-surface boundary explicit.

- [ ] Record `work_surface_ref`, git branch/commit, source paths, and command/run provenance on artifacts produced from work surfaces.
- [ ] Add explicit copy/mount affordance for artifacts consumed by work surfaces.
- [ ] Keep artifact refs separate from filesystem paths and work surface IDs.
- [ ] Add a small check that a work-surface-produced artifact records provenance and can be resolved after the workspace path changes.

**Exit gate:** Work surfaces can produce artifacts and consume uploaded artifacts without treating paths as artifact identity.

### Runtime checkpoint artifacts

Agent loop-control checkpoints should revisit this artifact-ref plan when checkpoint retention is implemented. Runtime checkpoint reports are artifacts rather than status reports: durable audit/debug payloads attached primarily to runs, optionally linked to focused Jobs/tasks as audit evidence, and never task/job status transitions by themselves.

Expected shape:

- artifact kind: `runtime_checkpoint`;
- primary link: `subject_kind = run`, `role = checkpoint`;
- optional links: focused Job/task with an audit/evidence role;
- checkpoint presence does not imply task completion, blockage, waiver, cancellation, or Docket history-visible progress.

If loop-control lands before artifact refs, any temporary checkpoint table should include a migration path to `runtime_checkpoint` artifact refs instead of becoming a permanent competing store.

## Human UI affordances

Build the small surfaces first:

- [ ] Artifact chip/card rendering anywhere an artifact ref appears.
- [ ] Metadata view: title, kind, content type, size, source, creator, lifecycle, visibility.
- [ ] Preview for text, markdown, diff, JSON, images, and browser-supported PDFs; download fallback for everything else.
- [ ] Provenance panel showing creating run/task/conversation/work surface when present.
- [ ] Lifecycle actions where authorized: keep, archive, delete, attach to Cabinet, detach from Cabinet, promote to durable, copy/mount into work surface.
- [ ] Permission-aware unavailable state that avoids metadata leakage when policy requires it.
- [ ] Artifact collections on runs, Docket tasks, conversations, and Cabinet items.

## Model experience affordances

Models should work with typed refs and summaries, not storage details.

- [ ] Expose artifact metadata as a compact model-facing shape:
  - `artifact_ref`
  - `kind`
  - `title`
  - `summary`
  - `content_type`
  - `readable`
  - `lifecycle`
- [ ] Add/read model-facing operations only as needed:
  - [ ] `get_artifact_metadata`
  - [ ] `read_artifact_text` or equivalent ranged text read
  - [ ] `preview_artifact` for non-text/large content
  - [ ] `create_artifact` / reserve+finalize wrapper where appropriate
  - [ ] `attach_artifact`
  - [ ] `copy_or_mount_artifact_to_work_surface`
- [ ] Ensure models may suggest titles/summaries/filenames but cannot mint refs.
- [ ] Ensure model-visible errors explain authorization/lifecycle failures without exposing backing storage.

## Acceptance criteria

Artifact refs are ready when:

- [ ] No model-facing or client-facing protocol needs Garage object keys or filesystem paths as durable content handles.
- [ ] Every artifact ref resolves through Den registry and permission checks.
- [ ] Pending artifacts cannot be read as complete artifacts.
- [ ] Finalized artifacts are stable snapshots.
- [ ] Conversation, Docket, Cabinet, and work-surface flows can cite artifact refs.
- [ ] Users can preview/download/attach artifacts from obvious UI affordances.
- [ ] Models can cite, inspect, create, and attach artifacts through typed operations without storage trivia.
- [ ] GC respects lifecycle and never deletes Cabinet-durable attachments.
