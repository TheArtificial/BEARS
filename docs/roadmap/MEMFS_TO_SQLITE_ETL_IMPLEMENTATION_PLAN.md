# MemFS → SQLite ETL — Implementation Plan

**Status:** Optional historical/operator tooling. Phases 1–3 landed (`den import-memfs`, git-dir + bundle, history/supersession, fixture tests, bear-admin Letta bundle upload). Phases 4–5 are retired because there are no production Letta-runtime Bears to migrate.  
**Architecture:** [ADR-0031 — SQLite-first canonical store](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), [Memory model](../architecture/memory-model.md)  
**Related:** [`den-migration-backfill-and-rollback-plan.md`](den-migration-backfill-and-rollback-plan.md), [`DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md`](DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md) (post-import `recall_index`), [ADR-0013 — MemFS sidecar](../decisions/adr-0013-memfs-sidecar-repo-views.md) (historical source layout)

## Goal

Optional, repeatable, idempotent **ETL import** of a Bear's legacy **MemFS git repository** into that Bear's **per-Bear SQLite** canonical memory (`memory_records`, and where reconstructible `memory_promotions`), preserving:

- the **logical-path anchor UX** agents and prompts already depend on (`core/…`, `{role}/…`, work-surface paths),
- **role visibility boundaries** (shared vs profile-local),
- **provenance** (MemFS branch, git path, commit oid, author timestamp),
- and enough **history** to rebuild head selection and (optionally) supersession chains.

After import, Den-native memory tools, key memory projection, and the derived recall index operate on SQLite alone. The MemFS repo becomes an **archival export**, not a live write path. This is not an active production migration path.

## Non-goals

- **Letta archival memory / `.af` exports** — derived semantic indexes, not canonical. Rebuild via `recall_index` / `den reindex` after SQLite import ([ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)).
- **Live dual-write** MemFS ↔ SQLite — import is a cutover/backfill step, not ongoing sync.
- **Task / Docket state** — schema-owned artifacts under `*/tasks/`, `work/results/`, etc. may be referenced in metadata but **task canonical state stays in Den Postgres** ([ADR-0034](../decisions/adr-0034-jobs-and-tasks-work-management.md)).
- **Entity layer backfill** — entity/relations import is a separate follow-on ([BEAR_ENTITY_LAYER_IMPLEMENTATION_PLAN.md](BEAR_ENTITY_LAYER_IMPLEMENTATION_PLAN.md)); v1 ETL may stash unresolved entity hints in `metadata_json` only.
- **Replacing git for human-authored repo artifacts** — skills, prompts, policies remain git-canonical per ADR-0031; this ETL targets **Bear cognition markdown** that lived in MemFS role branches.

## Source vs target

### Source: MemFS bare repo (per Bear)

Canonical shape (see [ADR-0013](../decisions/adr-0013-memfs-sidecar-repo-views.md), `services/den/scripts/init_bear_repo.sh`):

```text
refs/heads/talk    →  chat/…          (branch name `talk`, tree prefix `chat/`)
refs/heads/pair    →  pair/…
refs/heads/curate  →  curate/… + core/…
refs/heads/work    →  work/…
refs/heads/watch   →  watch/…
```

Each branch is path-enforced (`pre-receive` hook): roles may only write under their allowed prefixes. **`core/` only appears on the `curate` branch** in the canonical repo.

Sidecar **per-agent view repos** (if present in a legacy deployment) are **not** the import source — always import from the **Bear canonical bare repo** (`bears.memfs_repo_path` or operator-provided bundle).

### Target: per-Bear SQLite (`den-memory`)

Primary table: `memory_records` ([`schema.sql`](../../services/den/crates/den-memory/src/schema.sql)).

| MemFS concept | SQLite field |
|---|---|
| Branch + tree path | `logical_path` (via [`LogicalMemoryPath`](../../services/den/crates/den-memory/src/logical_path.rs)) |
| `core/…` | `scope_type = shared`, `scope_profile = NULL` |
| `{role}/…` | `scope_type = profile_local`, `scope_profile = {role}` |
| Path segment / convention | `kind` (`note`, `decision`, `summary`, `log`, `scratch`, `reflection`, or filename stem) |
| File body | `content_text` |
| Commit time | `created_at`, `valid_from` (event time) |
| Successive versions at same path | `supersedes_memory_id` chain (head = latest normal visibility, not superseded) |
| Git provenance | `metadata_json` (`memfs_import`: branch, path, commit, tree entry mode) |

Optional: `memory_promotions` rows when import can infer `curate` promotion from `core/` history (best-effort in v1; may defer).

## Export / acquisition

**Preferred transport: `git bundle`.** A bundle is a single file, preserves full object history, and can be produced from environments where only a minimal shell exists (legacy sidecar container, bare-repo volume mount, operator laptop).

### Operator export (from live bare repo)

```bash
# On the host/volume holding the canonical bare repo
git --git-dir="/path/to/{bear_id}.git" bundle create "bear-{bear_id}-memfs.bundle" \
  refs/heads/talk refs/heads/pair refs/heads/curate refs/heads/work refs/heads/watch
```

Verify:

```bash
git bundle verify bear-{bear_id}-memfs.bundle
git bundle list-heads bear-{bear_id}-memfs.bundle
```

### Export from a minimal container (no host git dir)

When the bare repo lives **inside** a container filesystem:

1. `docker cp` / volume snapshot is **not** sufficient alone — prefer bundle creation **inside** the container, then copy the bundle out.
2. Minimal pattern:

```bash
docker exec "$CONTAINER" git --git-dir="$REPO" bundle create /tmp/export.bundle --all
docker cp "$CONTAINER:/tmp/export.bundle" "./bear-{bear_id}-memfs.bundle"
```

If `git` is missing in the image, use a one-shot utility image with the repo volume mounted read-only and run the same `git bundle create` against `--git-dir`.

### Staging layout (recommended)

```text
imports/{bear_id}/
  bear-{bear_id}-memfs.bundle    # immutable audit artifact
  manifest.json                  # operator notes: export time, source host, branch tips at export
  report.json                    # produced by importer (counts, skips, errors)
```

Keep the bundle after import — it is the rollback reference and audit trail, not re-ingested on every run.

## Transform rules

### 1. Branch → scope mapping

| Git branch | Tree prefix(es) | `scope_type` | `scope_profile` |
|---|---|---|---|
| `talk` | `chat/` | `profile_local` | `chat` |
| `pair` | `pair/` | `profile_local` | `pair` |
| `curate` | `curate/` | `profile_local` | `curate` |
| `curate` | `core/` | `shared` | `NULL` |
| `work` | `work/` | `profile_local` | `work` |
| `watch` | `watch/` | `profile_local` | `watch` |

Reject or quarantine paths outside these prefixes for the branch being scanned.

### 2. Path → `logical_path` + `work_surface_ref`

Use existing [`LogicalMemoryPath::from_logical_path`](../../services/den/crates/den-memory/src/logical_path.rs) for stable round-trip:

- `core/bear-overview.md` → shared overview anchor
- `core/work_surfaces/{slug}/architecture.md` → shared + `work_surface_ref = {slug}`
- `pair/work_surfaces/{slug}/current-understanding.md` → profile-local + work surface
- `{role}/{kind}.md` → profile-local kind file

**Normalize:**

- Strip leading `/`; require `.md` suffix (or map extensionless legacy paths).
- Collapse duplicate slashes; reject `..` segments.

### 3. Path → `kind`

Resolution order:

1. Explicit directory convention ([memory model §Role-specific memory](../architecture/memory-model.md)): `…/notes/…` → `note`, `…/logs/…` → `log`, `…/decisions/…` → `decision`, `…/summaries/…` → `summary`, `…/scratch/…` → `scratch`, `…/reflection(s)/…` → `reflection`.
2. Work-surface file stem: `architecture.md` → `architecture` (already encoded in logical path).
3. Fallback: final path component without `.md`, else `note`.

Set `metadata_json.memfs_import.inferred_kind = true` when heuristic (not directory convention).

### 4. History → records + supersession

For each importable `(branch, logical_path)`:

1. Walk commits on that branch that touch the path (oldest → newest).
2. For each commit version with non-empty markdown body (skip `.gitkeep`, binary, &lt; min size):
   - Append one `memory_records` row.
   - `created_at` / `valid_from` = commit author timestamp (RFC3339 UTC).
   - Link `supersedes_memory_id` to the prior version at the same logical path (when `--import-history` enabled).
3. **Default v1 (`--heads-only`):** import **only the tip** blob per `(branch, path)` — one row per logical path, no supersession chain. Faster, enough for projection + recall heads.

Head selection after import must match [`list_indexable_heads`](../../services/den/crates/den-runtime/src/recall/reconcile.rs): latest `sequence_no` at path, `visibility = normal`, not superseded.

### 5. Skip / quarantine policy

| Path pattern | Action |
|---|---|
| `*/tasks/*`, `*/results/*` (schema-owned workflow artifacts) | **Skip** by default; optional `--include-workflow-artifacts` imports as `kind = task_artifact` with `metadata_json.schema_owned = true` |
| Empty, `.gitkeep`, &lt; N bytes | Skip |
| Non-`.md` / binary | Skip (log in report) |
| Path violates branch prefix rules | Quarantine (do not import) |
| Duplicate re-import (same commit oid + path) | Skip (idempotent) |

### 6. Visibility

Default `visibility = normal`. If MemFS had soft-delete conventions (future: tombstone paths), map to `visibility != normal` — v1 assumes all imported content is normal.

## Load strategy

### Write path

All inserts through **`BearMemoryStore::append_record`** (or a dedicated **`import_append`** that accepts explicit timestamps and bypasses `now()` for backfill) so:

- `bear_sequence` monotonic allocator stays consistent,
- migrations apply,
- hooks for `recall_index` enqueue can fire once at end (not per row).

**Recommended:** bulk insert inside a transaction with sequence pre-allocation, then single `enqueue_recall_index(bear_id)` (or operator runs `den reindex --bear …`).

### Idempotency

Store in `metadata_json`:

```json
{
  "memfs_import": {
    "branch": "curate",
    "path": "core/work_surfaces/my-repo/overview.md",
    "commit": "abc123…",
    "blob_sha": "…",
    "import_run_id": "…",
    "imported_at": "2026-06-17T…"
  }
}
```

Unique constraint strategy (pick one in implementation):

- **A (recommended):** partial unique index on `(bear_id, metadata_json->>'$.memfs_import.commit', logical_path)` for imported rows only, or
- **B:** deterministic `memory_id = uuid_v5(namespace, "{branch}|{path}|{commit}")` for imports.

Re-running import on the same bundle must **not** duplicate rows.

### Ordering

Import branches in dependency-friendly order:

1. `curate` / `core/` (shared anchors first),
2. role branches (`pair`, `work`, `chat`, `watch`, `curate` local),
3. optional history replay oldest→newest within each path.

## Phases

### Phase 0 — Spec + fixtures  ◻

- Freeze this mapping doc against 2–3 real Bear bundle samples (sanitized).
- Add **golden fixture** bare repo under `services/den/tests/fixtures/memfs-import/` (minimal: one file per branch, one work-surface path, one multi-commit path).
- Document role rename (`talk` branch → `chat` profile) in fixture README.

**Exit:** fixture bundle verifies with `git bundle verify`; table of expected `memory_records` rows checked by hand.

### Phase 1 — Extract + parse library (`den-memfs-import` or `den-memory::import`)  ◻

- Rust crate/module (prefer **`den-memory`** submodule to keep SQLite types local; CLI in **`den`** binary like `reindex`).
- Inputs: path to `.bundle` **or** `--git-dir` for dev.
- Clone/unbundle to temp dir; enumerate branches; list `(branch, path, commit, blob)` candidates.
- Pure transform: `MemfsBlob` → `ImportRecordDraft { logical_path, scope, kind, content, timestamps, metadata }`.
- Unit tests: path/kind mapping, scope rules, skip quarantine, idempotency key.

**Exit:** `cargo test` on transform with fixture; JSON dry-run output (`--dry-run`) listing drafts without writing SQLite.

### Phase 2 — Load + report  ◻

- Wire drafts → `memory_records` (+ optional `memory_promotions`).
- Flags: `--heads-only` (default) | `--import-history`, `--include-workflow-artifacts`, `--dry-run`.
- Emit `report.json`: imported / skipped / quarantined counts, per-branch totals, head paths sample.
- Guard: refuse import if bear SQLite already has &gt; N records **unless** `--force` (operator acknowledgment).

**Exit:** infra-free integration test — import fixture into temp bear DB; assert row counts, logical paths, role scope, head query matches; re-run idempotent.

### Phase 3 — CLI + operator UX  ◻

```bash
den import-memfs --bear <uuid> --bundle /path/to/bear-{id}-memfs.bundle [--heads-only] [--dry-run]
den import-memfs --bear <uuid> --git-dir /path/to/{id}.git   # dev only
```

- Optional admin UI: upload bundle + trigger import job (later; CLI first per [lazy migration tooling preference from prior elicitation]).
- Post-import checklist printed to stdout:

  1. Review `report.json`
  2. Spot-check admin memory browse / record detail
  3. Run `den reindex --bear <uuid>` when `QDRANT_URL` set
  4. Archive bundle to `imports/{bear_id}/`

**Exit:** documented operator procedure in this doc's [Runbook](#operator-runbook); smoke script hook optional.

### Phase 4 — Optional validation / rollback  ◻ retired as production gate

- **Validation queries:** count by `scope_type` / `scope_profile`; list top logical paths; compare bundle tip SHAs to imported metadata.
- **Spot diff (optional):** render head markdown from SQLite vs `git show branch:path` for sampled anchors.
- **Rollback:** restore bear SQLite from pre-import snapshot (file copy of `{bear_id}.sqlite`) — import must document snapshot step; **do not** delete bundle.
- **Cutover:** not required for production; native runtime already rejects MemFS client tools.

**Exit:** no active production gate. Use these checks only when importing an archived bundle.

### Phase 5 — Post-import derived recall  ◻

Not part of the ETL binary, but required for parity:

- Run `den reindex --bear <uuid>` or enqueue `recall_index` worker.
- Verify admin recall panel + `memory_search` hybrid legs over imported heads.

**Exit:** gated live Qdrant test or manual operator checklist when recall enabled.

## Operator runbook (optional archived bundle)

1. **Snapshot** the target bear SQLite file (`BEAR_SQLITE_DATA_DIR/{bear_id}.sqlite` → `{bear_id}.pre-import.sqlite`).
2. **Export** canonical bare repo to `bear-{bear_id}-memfs.bundle` (see [Export](#export--acquisition)).
3. **Dry-run:** `den import-memfs --bear {bear_id} --bundle … --dry-run` — review counts and quarantines.
4. **Import:** `den import-memfs --bear {bear_id} --bundle … --heads-only`.
5. **Validate:** admin memory browse; optional SHA spot checks against bundle.
6. **Reindex recall (if enabled):** `den reindex --bear {bear_id}`.
7. **Archive** bundle + `report.json` under `imports/{bear_id}/`.
8. **Cutover:** confirm native roles use SQLite tools only; MemFS volume may be retired after soak period.

## Risks

| Risk | Mitigation |
|---|---|
| Path/kind heuristic mismatch | Golden fixtures + dry-run report; manual quarantine bucket |
| Large repos / long history | Default `--heads-only`; batch commits in `--import-history` mode |
| Sequence allocator gaps | Single-transaction bulk import; post-import `sequence` = max+1 |
| Double import | Idempotency keys on commit+path |
| Recall drift after import | Mandatory reindex step in runbook |
| Workflow artifact noise | Skip `tasks/`/`results/` by default |
| `talk` vs `chat` naming | Explicit branch→profile map; never emit `scope_profile = talk` |

## Open questions

1. **`--import-history` default?** Heads-only is safer for v1; history mode needed for point-in-time recall fidelity on migrated content.
2. **Promotion reconstruction?** Infer `memory_promotions` when `core/` commit message or path pattern indicates curate promotion, or leave promotions empty.
3. **Admin UI vs CLI-only v1?** CLI + runbook first; UI upload when operators ask.
4. **Bear package inclusion?** Should exported bundles become part of [bear package](../guides/bear-package.md) transfer, or operator-side storage only?

## Related

- [DEN_NATIVE_RUNTIME_PLAN.md](DEN_NATIVE_RUNTIME_PLAN.md) — notes that production Phase 8 backfill is retired
- [den-migration-backfill-and-rollback-plan.md](den-migration-backfill-and-rollback-plan.md) — historical mixed-origin + rollback framing
- [DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md](DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md) — post-import reindex
- [MEMORY_TOOLS_IMPLEMENTATION_PLAN.md](MEMORY_TOOLS_IMPLEMENTATION_PLAN.md) — logical-path tool UX the import must preserve
