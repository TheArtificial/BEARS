# Bear Den operations plan

**Status:** Living document. This is the operational source of truth for running the Bear Den stack: what each data store is, what must be backed up, how to restore, and what to monitor.

**Scope.** This plan describes *operational strategy and policy*. Step-by-step deploy mechanics live in the per-service guides and are referenced, not duplicated:

- Stack composition and shared network: [`docker-compose.yaml`](../../docker-compose.yaml), [deployment/deployment.md](deployment/deployment.md)
- Den (control plane + native runtime): [services/den/COOLIFY_DEPLOY.md](../../services/den/COOLIFY_DEPLOY.md), [den-deploy.md](den-deploy.md)
- Bifrost (model gateway): [services/bifrost/COOLIFY_DEPLOY.md](../../services/bifrost/COOLIFY_DEPLOY.md)
- Garage (object store): [services/garage/COOLIFY_DEPLOY.md](../../services/garage/COOLIFY_DEPLOY.md)
- Processes, ports, health endpoints, `/status`: [infrastructure-and-ops.md](infrastructure-and-ops.md)
- CI image publish: [`.github/workflows/coolify-deploy.yml`](../../.github/workflows/coolify-deploy.yml)

---

## Service & data-store inventory

| Service | Role | Persistent state | Volume / location | Deploy guide |
|---------|------|------------------|-------------------|--------------|
| `bears-den` | Control plane + in-process native agent loop | Per-Bear **SQLite** (canonical Bear memory/tasks/entities) | `bears-den-sqlite-data` → `BEAR_SQLITE_DATA_DIR` (`/var/lib/den/bear-sqlite`) | [den](../../services/den/COOLIFY_DEPLOY.md) |
| Den **Postgres** | Control-plane DB (users, bears, membership, sessions, Connections, recall passage registry) | **PostgreSQL** | managed DB via `DATABASE_URL`, or bundled `bears-postgres` → `bears-postgres-data` (profile `bundled`) | [den](../../services/den/COOLIFY_DEPLOY.md) |
| `bears-qdrant` | Derived recall vector index ([ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)) | Vectors (**derived, rebuildable**) | `bears-qdrant-data` → `/qdrant/storage` (profile `recall`) | this doc |
| `garage` | S3-compatible object store (artifacts, Cabinet/Outline attachments) | Object metadata + data | Garage metadata/data volumes | [garage](../../services/garage/COOLIFY_DEPLOY.md) |
| `bears-bifrost` | Model gateway (OpenAI-compatible `/v1`) | **Stateless** — config is `services/bifrost/config.json` from Git | none | [bifrost](../../services/bifrost/COOLIFY_DEPLOY.md) |
| `bears-preflight-*` | Deploy-time config/DB validators | none (run-once) | none | [`docker-compose.yaml`](../../docker-compose.yaml) |

---

## Data classification: canonical vs derived

The single most important operational principle, inherited from the memory architecture ([ADR-0031](../decisions/adr-0031-sqlite-first-canonical-store-for-bear-agent-memory-and-tasks.md), [ADR-0038](../decisions/adr-0038-platform-embedding-standard-and-derived-recall-index.md)):

**Canonical (loss = data loss → must be backed up):**

- **Per-Bear SQLite** (`bears-den-sqlite-data`) — the source of truth for Bear memory, tasks, entities, relations.
- **Den Postgres** — users, bears, membership, sessions, Connections (control-plane identity), and the recall **passage registry** metadata.
- **Garage object store** — artifacts (agent outputs, user uploads) and Cabinet/Outline attachments that are not reproducible from anything else.

**Derived (loss = degraded performance, not data loss → backup is optional):**

- **Qdrant** (`bears-qdrant-data`) — recall vectors are fully reconstructable by re-embedding canonical SQLite (and Cabinet) via Bifrost. Treat it as a **cache, not a backup target**.

**Stateless (nothing to back up):**

- **Bifrost** — configuration is `config.json` tracked in Git; redeploy restores it.

---

## Backup strategy

### Per-Bear SQLite (canonical)

- **Mechanism:** Den runs **no built-in backup job**. Use one of:
  - the optional `bears-den-sqlite-data-backup` sidecar (`COMPOSE_PROFILES=volume-backup`), which archives the volume to S3-compatible storage on a cron (`DEN_SQLITE_VOLUME_BACKUP_CRON`, default `0 5 * * *`) using `SCALEWAY_*` credentials; or
  - platform **volume snapshots**.
- **WAL caution:** SQLite runs in WAL mode — prefer **quiesced copies or full-volume archives** over copying a single `.sqlite` file while Den is writing.
- **Logical export:** the [Bear package](bear-package.md) format is the portable per-Bear export/import and complements (does not replace) volume backups.
- **Target separation:** back up **off-box** (e.g. Scaleway S3), not into Garage on the same host.

### Den Postgres (canonical)

- **Managed DB (preferred):** rely on the provider's automated backups + PITR; verify retention.
- **Bundled `bears-postgres`:** has **no backup sidecar today** — operators must add `pg_dump`/base-backup on a schedule, or migrate to a managed DB. See [Open items](#open-items).
- This store holds the recall **passage registry**; backing up Postgres makes a Qdrant rebuild *fast and incremental* even without a Qdrant snapshot.

### Garage object store (canonical)

- Back up via Garage replication and/or volume snapshots of its metadata + data volumes. Artifacts and Cabinet attachments are generally not reproducible. See [garage guide](../../services/garage/COOLIFY_DEPLOY.md).

### Qdrant (derived — do NOT back up for correctness)

- **No backup is required for data safety.** If Qdrant is lost, recall **degrades gracefully** to anchors + keyword (`LIKE`) — turns do not fail ([ADR-0041](../decisions/adr-0041-archival-recall-and-async-curation.md) §6) — and the index is rebuilt by `archive_index` reconcile + `reindex-bear` / `reindex-cabinet` ([DERIVED_RECALL plan](../roadmap/DERIVED_RECALL_INDEX_IMPLEMENTATION_PLAN.md)).
- **Optional snapshots** are a **recovery-time / cost** optimization only: restoring a snapshot avoids re-embedding spend and wall-clock time at Cabinet scale. A stale snapshot is acceptable because `archive_index` reconciles drift on the next run.

### Bifrost (stateless)

- Nothing to back up; `config.json` lives in Git. Keep provider API keys in the secret manager / env, not in backups.

---

## Restore & disaster recovery

**Rebuild order** (dependencies first):

1. **Bifrost** — redeploy from Git (config.json); needed before embeddings/LLM work.
2. **Garage** — restore object volumes; Den needs S3 credentials at start.
3. **Den Postgres** — restore managed backup or `pg_restore`; control-plane identity + passage registry.
4. **Per-Bear SQLite** — restore volume snapshot / sidecar archive, or re-import Bear packages.
5. **Den** — redeploy; migrations apply on startup against `DATABASE_URL`.
6. **Qdrant** (if enabled) — restore optional snapshot, **or** start empty and let `archive_index` reindex from canonical sources. No restore needed for correctness.

**RPO/RTO posture:**

- **RPO** is governed by the canonical stores (SQLite sidecar cron + Postgres/Garage backup cadence). Qdrant does not affect RPO.
- **RTO** for recall is improved (not required) by a Qdrant snapshot; without one, recall is available immediately in degraded mode and sharpens as reindex completes.

**Drills:** periodically validate a restore of SQLite + Postgres into a scratch environment and confirm `/health/ready` and `/status.json` pass. See [Open items](#open-items).

---

## Monitoring & health

- **Liveness/readiness:** `GET /healthcheck`, `GET /health/ready` (web + API) — see [infrastructure-and-ops.md](infrastructure-and-ops.md).
- **Stack watch point:** `GET /status` (HTML) and `GET /status.json` (503 on any `fail`) cover Den Postgres `SELECT 1`, Bifrost health/metadata, and config-shape checks.
- **Qdrant:** `/status` has **no first-class Qdrant probe** on the default native path yet; add an external reachability monitor for `QDRANT_URL` when recall is enabled. See [Open items](#open-items).
- **Backups:** monitor sidecar/job success and alert on missed runs (the failure mode is silent until a restore is attempted).

---

## Open items

- **Automated Postgres backups for the bundled `bears-postgres`** (no sidecar today) — add `pg_dump`/base-backup or document managed-DB-only for production.
- **First-class Qdrant health probe** in `/status` / `/status.json`.
- **Scheduled restore drills** (SQLite + Postgres) with a documented verification checklist.
- **Garage backup runbook** — concrete replication/snapshot recipe and restore steps.
- **Backup encryption & retention policy** — define retention windows and at-rest encryption for off-box archives.
