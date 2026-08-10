# How job-work sandboxes work (Den internals)

Developer map of the `work`-stance execution path: the crates involved, the
durable state machine, the trip a work run takes from `dispatch_work` to a
pushed branch, and the invariants each seam maintains. Companion docs:
[user guide](running-background-work.md),
[ops guide](sandbox-server-ops.md), product brief
`docs/roadmap/sandbox_brief.md`.

## Cast of components

| Component | Where | Role |
|---|---|---|
| Work-run state machine | `den-docket/src/work_runs.rs` | Durable dispatch/claim/lease rows (`bear_work_runs`), checkout prompt, publish-policy plumbing |
| Dispatch worker | `den-runtime/src/work_dispatch.rs` | Claims runs, provisions sandboxes, monitors, harvests, publishes, finalizes |
| Sandbox provider | `den-sandbox` crate | Standalone HTTP service: roots, catalog, container backend, network isolation, publish |
| Armature headless mode | `tools/bear-armature/src/headless.rs` | In-sandbox process: checks out the work order, runs one turn, reports |
| BearWire work methods | `den-bearwire/src/methods/work.rs` | `work.checkout` / `work.report` for the in-sandbox armature |
| Model tools | `src/core/tools/workflow/mod.rs` | `create_job`, `dispatch_work`, `list/get_work_run`, `cancel_work_run`, `get_work_catalog` |
| Web UI | `den-web/src/work/` | `/work` pages: create, dispatch, observe |

Design invariant worth internalizing first: **Docket schedules, gates, and
records — it never executes** (ADR-0034). Execution state lives on the run
row; the sandbox host holds no durable state at all (it can be wiped and the
reaper re-adopts labeled containers).

## The durable state machine

One Work Run per dispatched Job execution. The current `bear_work_runs.task_id` implementation is transitional and is being migrated to a Job-scoped execution row keyed by `job_id` / `job_run_id`; task progress belongs in `bear_task_run_state`, not separate sandboxes.

```
queued → claimed → provisioning → running → reporting → succeeded
                                     │                  | blocked
                                     └── (sandbox died) | failed / cancelled / timed_out
```

- **Enqueue** (`enqueue_work_run`) validates the task is job-attached and
  `assigned_to_role = 'work'`, reuses or opens a `bear_job_runs` row, bumps
  `attempt`, and stores the requested `root_name` / `git_ref` /
  `image_name`. A partial unique index forbids two active runs per task.
- **Claims are global and lease-based** (`claim_next_work_run`,
  `FOR UPDATE SKIP LOCKED`): any worker takes the oldest claimable row —
  fresh `queued` rows or non-terminal rows whose lease expired (worker
  crash). Ownership is `runner_id` + `lease_expires_at`; `heartbeat_work_run`
  fences stale owners. Consequence for tests: DB-backed claim tests must
  serialize and purge leftovers (`DB_LOCK` / `purge_claimable_runs` in
  `work_runs_tests.rs`).
- **One active Work Run per Job**: a Job dispatch owns one sandbox/workspace/session. Inside it, the Work runtime advances runnable tasks sequentially using `bear_task_run_state`; task boundaries do not create new sandboxes. A post-claim recheck
  (older-run-wins) resolves the cross-worker race the committed-state gate
  cannot see; expired-lease takeovers are exempt because the taken-over run
  *is* the job's active run. Runs of different jobs still execute
  concurrently up to `SANDBOX_MAX_CONCURRENT`. Queue placement is derived
  at read time (`queued_run_positions`: 1-based position + the in-flight
  run being waited on) and surfaces in the `/work` pages, in
  `dispatch_work`'s response, and as a `queue` object on queued runs in
  `list_work_runs` / `get_work_run` — never stored.
- Terminal transitions go through `finalize_work_run` exactly once; audit
  events ride `bear_task_events`.

## Life of a run

1. **Dispatch** — `dispatch_work` (chat/pair), the `/work` UI form, or the
   optional `WORK_DISPATCH_AUTO` sweep enqueues a run.
2. **Claim + provision** (`provision_run`): the worker loads the dispatch
   context (bear slug, job creator, `commit_policy`, `work_branch`), then
   - pins the **job work branch** when the policy publishes
     (`ensure_job_work_branch`: `COALESCE(work_branch, 'den/job-<short8>')` —
     never overwrites an explicit branch);
   - mints an **ephemeral armature token** as the job creator and persists
     its id into `result_refs` immediately, so even a crashed worker's
     successor can revoke it;
   - builds `CreateSandboxRequest`: root (run's, else the job's
     `work_surface_ref`), `git_ref` (run's, else the job branch — the
     provider falls back to the default ref while the branch has no commits
     yet), catalog `image` name, `network` (restricted by default), env
     (`DEN_API_URL`, `DEN_TOKEN`, `DEN_WORK_ORDER_ID`, deadline, and the
     bear's `GIT_AUTHOR/COMMITTER_*` identity so in-run commits attribute
     correctly), and a `work_run_id` label for orphan reconciliation.
3. **In the sandbox** — the image's entrypoint is `bear-armature headless`.
   It validates its token, calls `work.checkout` (which binds the BearWire
   session to the run, opens a `docket_execution_sessions` row satisfying
   the work-stance gate, and returns the prompt built by
   `build_work_prompt`), then runs **one turn** against Den's agent loop
   with a self-kill deadline set under the container timeout. The prompt is
   the contract: criteria-driven, "never wait for input", mark done/blocked
   via `update_current_task_status`, and — for pushable jobs — "commit as
   you go; do not push". Permissions are auto-decided by
   `decide_permission_headless` (local fs/git/process allowed; browser and
   external-URL targets denied).
4. **Turn end** — a terminal run event flips the row to `reporting`
   (`record_work_run_turn_outcome`, keyed by the bound session).
   `work.report` from the armature is advisory only; the authoritative
   outcome is run-scoped Docket task status.
5. **Harvest** (`harvest_run`) — once per Job Run, the worker collects the cumulative diff, log tail, and usage from the provider; success requires the Job's runnable tasks and acceptance criteria to reach their terminal success conditions. On success with a pushable
   policy it calls the provider's `publish` endpoint (branch = job work
   branch) *before teardown*; the outcome lands in
   `result_refs.published` / `result_refs.publish_failed`. A publish failure
   never un-dones the task — the work happened; it is surfaced loudly
   instead. Then Docket gets `record_task_success/blocked`, the token is
   revoked, the run finalizes, and the sandbox is destroyed
   (`SANDBOX_PRESERVE_FAILED` keeps failed workspaces for debugging).
6. **Failure paths** — a sandbox that exits without a turn outcome is a lost
   turn: the run fails with the log tail attached and re-enqueues while
   `attempt < WORK_MAX_ATTEMPTS` (`maybe_requeue`; judged blocked/succeeded
   outcomes never retry automatically). Cancellation destroys the sandbox
   (no in-flight turn interruption in v1 — a documented ponytail) and
   finalizes as cancelled. An hourly orphan sweep destroys provider
   sandboxes whose runs are already terminal.

## Publish semantics

- `per_task` checkpoints/publishes after completed task boundaries while retaining the same Job sandbox; `per_job` publishes once when the Job Run completes. `none` does not publish because no source changes are expected.
- The push happens **host-side on the sandbox server** with the root's
  credentials (managed-surface credentials arrive via the Den config sync —
  encrypted at rest in Den, written to per-surface 0600 files on the
  provider) (`RootsManager::publish_workspace`): optional auto-commit of
  leftover changes ("work run `<id>`: uncommitted changes", Den identity),
  commit count against the provisioned base, push `HEAD:refs/heads/<branch>`
  to the upstream URL, then a pristine re-sync so the branch is immediately
  provisionable. Guards: upstream-less roots refuse (`publish_unsupported`),
  the default ref refuses without `allow_default_ref`, and a no-op publish
  (nothing past the base) skips the push rather than minting empty branches.
- Non-fast-forward pushes (two runs raced to one branch) surface as
  `publish_failed` — v1 does not force-push or rebase.

## Sandbox typing, catalog, and network

- `SandboxType` is a closed enum; only `container` is implemented and the
  policy layer (`den-sandbox/src/policy.rs`) rejects the rest **explicitly**
  — never silently degrade to a weaker boundary. The descriptor's
  `strength_label` states what the sandbox actually provides, including the
  network mode.
- Images resolve **by catalog name only** (`RootsManager::resolve_image`:
  request → root default → catalog default → `SANDBOX_IMAGE`); unknown names
  are rejected listing the catalog. Raw image references never travel on
  the dispatch path — the provider's applied managed config (admin-managed
  `sandbox_catalog_images`, pushed via `PUT /sandbox/v1/managed-config` and
  persisted on the provider) is the trust boundary.
- `NetworkMode::Restricted` (default): per-sandbox `docker network create
  --internal` plus a socat relay container that is the only bridge to the
  Den callback endpoint; the backend rewrites `DEN_API_URL` to the relay.
  Everything else in the container has no route out. See
  `backend/container.rs` for the arg builders (pure functions, unit-tested
  without a daemon).

## Extending things — where to look

- **New run metadata**: column on `bear_work_runs` (+ migration) →
  `WORK_RUN_COLUMNS` / `WorkRunRow` / `WorkRunEnqueue` → worker → tool
  summaries (`work_run_summary_json`) → `RunView` in `den-web/src/work/`.
- **New provider capability**: protocol types (`den-sandbox/src/protocol.rs`,
  serde-only) → server route → `SandboxClient` method → worker/tool callers.
  Keep the provider free of sqlx/den-service — it must run without the Den
  database.
- **Prompt changes**: `build_work_prompt` in `work_runs.rs`; it is
  checkout-time state, so tests live in `work_runs_tests.rs`.
- **Tests**: provider logic is deliberately unit-testable host-side (tempdir
  bare repos for publish, arg builders for docker); DB-backed tests follow
  the skip-without-`DATABASE_URL` convention; `scripts/work-e2e.sh` is the
  full acceptance walkthrough.
