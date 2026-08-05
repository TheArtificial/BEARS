# Artifacts, Garage (S3), and Cabinet separation — Architecture Decision Record

## Status: Proposed

## Date: 2026-04-19

---

## Context

**Artifacts** are Den-managed content objects produced, consumed, uploaded, or cited around agent work. They include files, generated reports, patches, screenshots, logs, source documents, Cabinet attachments, and other payloads that are too large, durable, permission-sensitive, or provenance-sensitive to inline in conversations or events.

Artifacts **must not** be stored inside **Letta**. Artifact bytes belong in object storage or another explicit backing store with lifecycle, provenance, and access control.

**Garage** is the BEARS **S3-compatible** object store ([Garage Coolify deploy](../../../services/garage/COOLIFY_DEPLOY.md)). Den already plans to use it for presigned upload/download.

**Cabinet** (Outline-backed, Phase 2+) uses object storage for **attachments** in a **different concern**: long-lived documents, human editing, deck policy—not ephemeral chat/run blobs.

As Den adds Docket evidence, Cabinet attachments, delegated runs, and work surfaces, object keys and filesystem paths are not sufficient protocol handles. Den needs stable, opaque, permission-checked artifact refs.

### Harness vs control plane vs object storage

BEARS uses three product layers ([DEN_ARCHITECTURE.md](../DEN_ARCHITECTURE.md#three-layers-names)) plus **Garage** as infrastructure:

| Piece | Role for artifacts |
|-------|-------------------|
| **Harness (Letta Code)** | **Runtime I/O** during agent work: the **tool loop** runs here; uploads/downloads of artifact bytes are **initiated in the harness** (tools, skills, terminal helpers)—typically **via** Den-issued **presigned URLs** or thin **Den HTTP APIs** that enforce `bear_id` / `conversation_id` / membership before writing. |
| **Control plane (Den)** | **Policy, identity, and lifecycle**: bucket names, credential scope, **artifact ref minting**, **artifact registry** rows in Postgres, **presigned URL issuance**, **garbage collection** jobs, provenance metadata contract, and permission checks. Den does **not** run inside Letta’s tool sandbox; it **governs** how the harness and browser may use S3. |
| **Garage** | **Infrastructure** (like Postgres): S3-compatible storage **outside** Letta. Neither “Den” nor “Letta” as a layer—**both** the harness path and Den reach it with different roles. |
| **Letta (persistence)** | **No blob storage** for artifacts—only Den artifact refs in messages/blocks if needed. |

**Summary:** File storage **during** agent use feels like **Harness** because the harness performs runtime reads/writes. **Den** still owns artifact identity, policy, GC, and URL issuance.

---

## Decisions

### 1. Artifacts live outside Letta

All **binary, large, durable, permission-sensitive, or provenance-sensitive** payloads from agent use—including tool runs, **skills**-mediated steps, image/file generation, user uploads, delegated run outputs, Docket evidence, and **routine** runs—are stored outside Letta. Most artifact bytes live in Garage-backed object storage. Letta holds Den artifact refs as needed, not bytes or storage keys.

### 2. Den-minted artifact refs are the protocol handle

Artifacts are addressed in Den protocols by opaque Den-minted refs, not by Garage object keys, filesystem paths, filenames, URLs, Cabinet IDs, work-surface IDs, or model-generated IDs.

Example:

```text
artifact_01JZ8WJ3Z3N3W5F0Y2F7P9Q4BH
```

Artifact refs are:

- minted only by Den's artifact registry;
- opaque to models and clients;
- stable for the artifact lifetime;
- registry-backed;
- permission-checked on read, write, mount, attach, promote, and delete;
- content-address checked via stored hashes, but not content-address named.

Models, tools, and harnesses may suggest titles, filenames, summaries, and content types. They do not invent artifact refs.

### 3. Artifact registry is required

Den maintains an artifact registry row for every artifact ref. Object storage keys are implementation details behind the registry.

Conceptual registry fields:

| Field | Purpose |
|-------|---------|
| `artifact_ref` | Opaque Den-minted ID. |
| `kind` | `file`, `patch`, `report`, `test_report`, `memory_review`, `screenshot`, `attachment`, etc. |
| `title` | Human-readable title or suggested filename. |
| `storage_kind` | `garage_artifacts`, `garage_cabinet`, `db_text`, `external_ref`, etc. |
| `object_key` / `external_ref` | Backend locator; not exposed as the protocol ref. |
| `content_type`, `size_bytes`, `sha256` | Content metadata and integrity. |
| `bear_id`, `user_id`, `conversation_id` | Scope and ownership. |
| `parent_run_id`, `parent_task_id`, `job_id` | Docket/run provenance when applicable. |
| `creating_stance` | `chat`, `pair`, `work`, `curate`, `watch`, `system`, etc. |
| `source` | `agent`, `human_upload`, `routine`, `external_import`, etc. |
| `visibility` | `internal_audit`, `parent_only`, `user_visible`, `artifact_only`, or product-specific equivalent. |
| `lifecycle` | `pending`, `ephemeral`, `promoted`, `cabinet_durable`, `archived`, `deleted`. |
| `created_at`, `finalized_at` | Lifecycle timestamps. |

Exact schema is a Den implementation detail. The registry must preserve enough provenance for audit, permission checks, retrieval, and GC.

### 4. Two-phase artifact creation

Artifact creation is two-phase when bytes are uploaded or generated asynchronously:

```text
reserve artifact ref -> write/upload content -> finalize artifact
```

A reserved artifact exists in `pending` lifecycle state. It is not readable as complete content until finalized. Finalization records size, hash, content type, backend locator, and provenance.

### 5. Finalized artifacts are stable snapshots

A finalized artifact is a durable snapshot of content. Mutable execution state belongs in work surfaces, repos, branches, sandboxes, or Cabinet documents—not in finalized artifacts.

If content changes materially, Den should create a new artifact ref or an explicit revision relationship rather than silently mutating the finalized artifact.

### 6. Single artifacts bucket for ephemeral agent output and human upload

**User-uploaded files** used in conversations or agent work use the **same artifacts bucket** and **same structural conventions** as agent-generated ephemeral artifacts. Provenance distinguishes `source: agent`, `source: human_upload`, `source: routine`, and related values.

### 7. Conversation, Docket, run, and routine association

Artifacts should be associated with the relevant Den identities and scopes: conversation, bear, user, job, task, run, routine, source tool, work surface, and/or external source. Exact key prefixes are implementation-defined and hidden behind artifact refs.

### 8. Ephemeral by default + garbage collection

The `bears-artifacts` bucket is for temporary working files unless promoted elsewhere. **Den** runs garbage collection: delete or lifecycle-expire objects older than policy (per-org TTL, quota, or both). GC operates through the registry and must respect lifecycle and attachment state.

Artifact GC must not delete Cabinet-durable attachments.

### 9. Cabinet attachments use artifact refs without becoming ephemeral artifacts

Cabinet document attachments use a durable Cabinet storage concern, usually a different S3 bucket and credentials/policy. Cabinet objects may still be represented by Den artifact refs so that conversations, runs, Docket evidence, and Cabinet items can cite attachments uniformly.

Cabinet item and artifact remain separate concepts:

```text
Cabinet item = curated knowledge/document object
Artifact     = stored content/blob/reference with provenance
```

A Cabinet item may attach zero or more artifact refs:

```json
{
  "cabinet_ref": "cabinet_abc",
  "title": "Memory automation implementation brief",
  "attachments": [
    { "artifact_ref": "artifact_01JZ...", "role": "source_pdf" },
    { "artifact_ref": "artifact_01K0...", "role": "generated_report" }
  ]
}
```

Cabinet retention, ACLs, editing, and semantic organization are Cabinet policy. Artifact refs provide content identity, provenance, and access-checked retrieval.

**Do not conflate:** `bears-artifacts` ephemeral objects vs **Cabinet bucket** durable attachments. A shared artifact-ref handle does not imply a shared lifecycle.

### 10. Work surfaces produce and consume artifacts

A **work surface** is mutable execution state: repo checkout, sandbox filesystem, branch-backed workspace, mounted project root, build environment, or similar.

An **artifact** is a durable content snapshot or externally stored payload.

Work surfaces may produce artifacts:

```text
work surface -> patch/test report/screenshot/log/export -> artifact ref
```

Artifacts should record provenance back to the work surface when relevant:

```json
{
  "artifact_ref": "artifact_01JZ...",
  "kind": "test_report",
  "parent_run_id": "run_123",
  "work_surface_ref": "surface_repo_main",
  "source": {
    "git_commit": "abc123",
    "branch": "den/job-456",
    "paths": ["test-results/parser.log"]
  }
}
```

Artifacts may also be copied or mounted into work surfaces through explicit capabilities:

```json
{
  "capability": "artifact.read:artifact_01JZ...",
  "mount": {
    "artifact_ref": "artifact_01JZ...",
    "path": "/inputs/customer-data.csv",
    "mode": "read_only"
  }
}
```

A work-surface path is provenance or a mount location, not an artifact ref.

### 10a. Verified Docket outputs

When a Docket work surface uses an artifact as its output, the finalized artifact ref is the durable candidate that the surface verifies; a worker's report only links to or summarizes it. `GitPatch`, `FileBundle`, and `Report` outputs can therefore use Garage/registry artifacts even when the execution workspace is ephemeral or has no reachable Git remote.

The verification record must bind the finalized artifact's identity and digest to the work run, task, job, and work surface. For a Git-backed output, the corresponding verification record instead binds the verified commit and ref; it may additionally cite patch, test-report, or log artifacts. In either case, Docket settles completion only from the recorded verification result and required validation evidence—not from a model-generated SHA or a claimed push in run text.

### 11. Promotion path

Den may support moving or copying an ephemeral artifact into Cabinet with user confirmation and policy checks. Promotion creates or links a Cabinet-durable artifact/attachment and updates lifecycle so ephemeral GC no longer treats the content as disposable.

---

## Invariants

- Artifact refs identify durable Den-managed content objects.
- Den mints artifact refs; models and clients do not.
- Artifact refs are not object-storage keys, URLs, filesystem paths, work-surface IDs, Cabinet records, conversations, jobs, tasks, or runs.
- Work surfaces may produce or consume artifacts, Cabinet items may attach artifacts, and runs may cite artifacts as evidence.
- Finalized artifacts are stable snapshots. Mutable work belongs in work surfaces; durable evidence belongs in artifacts.
- Cabinet-durable artifacts and ephemeral run artifacts may share the artifact-ref protocol, but they do not share GC policy.

---

## Bucket layout (reference)

| Bucket | Purpose | GC |
|--------|---------|-----|
| **`bears-artifacts`** (name may vary per deploy) | Ephemeral agent outputs, human uploads in chat, routine file outputs, delegated-run outputs, Docket evidence | **Yes** (Den policy through registry) |
| **`bears-cabinet`** (or name aligned with Outline) | Cabinet / Outline attachments and Cabinet-durable artifact payloads | **No** (artifact GC rules); lifecycle per Outline/Cabinet policy |

Deploy: create **both** buckets in Garage; scope keys to least privilege (Den service key: artifacts read/write; Cabinet key: cabinet bucket only or via Outline).

---

## Consequences

- **Den:** S3 client, artifact registry table, artifact-ref minting, presigned URLs, permission checks, lifecycle/GC jobs, provenance metadata, and optional mount/copy APIs for work surfaces.
- **Letta Code / harness:** Tools that “save a file” reserve/finalize artifacts through Den APIs or presigned URLs; never persist large blobs in Letta DB; never invent artifact refs.
- **Docket / delegated runs:** Run outputs cite artifact refs for patches, reports, logs, memory proposals, and evidence.
- **Cabinet:** Cabinet items may attach artifact refs, including Cabinet-durable artifacts whose lifecycle is not governed by ephemeral artifact GC.
- **Work surfaces:** Workspaces produce artifacts as stable snapshots and consume artifacts only through explicit artifact-read/mount capabilities.
- **Routines:** Routine outputs that are files land in the artifacts bucket with `routine_id` and bear/user provenance — see [routines-automation.md](routines-automation.md).
- **Phase 1:** Garage + artifacts bucket + artifact registry + metadata + GC may trail **first** chat path; document order in [PHASE1_BOOTSTRAP.md](../roadmap/PHASE1_BOOTSTRAP.md).

---

## References

- [Garage Coolify deploy](../../../services/garage/COOLIFY_DEPLOY.md)
- [PLAN.md — Artifacts and object storage](../roadmap/PLAN.md#artifacts-and-object-storage-garage)
- [routines-automation.md](routines-automation.md)
- [DEN_ARCHITECTURE.md](../DEN_ARCHITECTURE.md)
- [ADR-0053: Stance-Scoped Delegated Runs](adr-0053-stance-scoped-delegated-runs.md)
