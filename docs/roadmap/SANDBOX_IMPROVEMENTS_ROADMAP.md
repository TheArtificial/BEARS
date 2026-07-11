# Work sandbox: recommended future improvements

Post-v1 roadmap for the `work` sandbox system. Each item has a rough plan
outline. Current state and known ponytails:
`docs/guides/work-sandbox-internals.md`.

Priority note: items 7–9 (per-root build caches, command-criteria
verification, per-job run serialization) jumped the queue after review —
they bite on the first real jobs (rust surfaces recompile cold every run,
success is self-graded, and multi-task jobs race their work branch). Item 9
is **implemented**; 7 and 8 are the recommended next two.

Since implemented (2026-07-11), superseding the static roots file: **managed
work surfaces** (den-level entities, user-permissioned, bear-assigned,
encrypted credentials, `/work/surfaces` UI) and the **Den-managed image
catalog + image-store ops UI** (`/admin/sandbox`: engine store, registry
pulls, one-click shipped-variant builds, background operations). The
provider is configured exclusively via `PUT /sandbox/v1/managed-config`;
`SANDBOX_ROOTS_CONFIG` is deprecated and ignored. Items below that say
"roots file" now mean the managed surface/catalog settings.

## 1. Docker within sandboxes

**Why**: several real work surfaces (this repository included) need docker
for their own test suites (testcontainers, compose-backed integration
tests). v1 sandboxes have no docker CLI or daemon, and mounting the host
socket into a sandbox is a non-starter — it is host-root-equivalent and
would erase the isolation boundary.

**Recommended shape: per-sandbox rootless dind sidecar.** A third
per-sandbox container (alongside the sandbox and the egress relay) running
`docker:dind-rootless`, attached only to the sandbox's internal network; the
sandbox gets `DOCKER_HOST=tcp://den-sbx-dind-<id>:2375`. Nested containers
live inside the sidecar's own storage and network namespaces, so the egress
restriction still holds for them (their only route out is the sidecar, whose
only route is the internal network).

Plan outline:

1. Provider: `CreateSandboxRequest.docker: bool` (default false) + a
   `docker: true|false` capability flag per catalog image (a dind-capable
   task usually also needs the CLI in the sandbox image — add
   `docker-cli`-bearing image variants).
2. Backend: provision/destroy the sidecar with the same labeling scheme
   (`den.sandbox.dind=<id>`); wire `DOCKER_HOST`; storage on a tmpfs or a
   per-run directory under the workspace parent so teardown reclaims it.
3. Policy: dind requires explicit opt-in per root (roots-file flag) — it
   meaningfully raises resource cost and attack surface; capacity accounting
   should weigh dind sandboxes double.
4. Image availability inside dind: nested pulls are blocked by the internal
   network (by design). Options, in preference order: (a) pre-seed at
   provision time — `docker save` on the host, `docker load` through the
   sidecar for an allowlist of images declared per root; (b) an optional
   registry-mirror relay (same socat pattern) to a trusted pull-through
   cache. Start with (a).
5. Fallbacks documented honestly: rootless dind needs cgroup v2 + fuse;
   where unavailable, `sysbox` runtime (if installed) or refusing with an
   actionable reason beats privileged dind. Never fall back to the host
   socket.
6. Tests: arg-builder units; an env-gated e2e that runs `docker run
   hello-world` (pre-seeded) inside a sandbox.

**Effort**: ~3–5 days including the image variants and seeding.

## 2. In-flight cancellation and multi-turn runs

**Why (cancel)**: today cancel destroys the sandbox; the turn's obligations
starve and the continuation watchdog cleans up — works, but graceless (a
recorded ponytail in `work_dispatch.rs`). **Why (multi-turn)**: one turn
with a fixed deadline caps task size; `bearwire::handle_prompt` also has its
own per-turn ceiling that long work orders hit first (ponytail in
`headless.rs`).

Plan outline: deliver a durable cancel/continue signal through the BearWire
event channel the armature already polls; teach headless mode to run a
checkout → turn → re-checkout loop until the run-scoped task status is
terminal, the signal says stop, or the deadline budget is spent; thread the
checkout deadline into the poll loop. Den side: `work.checkout` returns
remaining budget; the worker distinguishes "turn ended, run continuing" from
terminal outcomes (the `reporting` transition already keys off terminal
events only).

**Effort**: ~2–3 days; mostly armature-side.

## 3. Per-task network and resource policy

**Why**: network posture and limits are currently provider/Den-wide
(`WORK_SANDBOX_NETWORK`, `SANDBOX_DEFAULT_TIMEOUT_SECS`); real jobs differ
(an 11ty build may legitimately need a registry mirror; a Godot test run
needs more memory than a docs edit).

Plan outline: add an optional `sandbox_policy` JSON column on `bear_jobs`
(timeout, memory/cpus/pids, network mode, egress allowlist); validate
against per-root ceilings in the roots file (the host stays authoritative);
worker merges job policy into `CreateSandboxRequest`; expose in `create_job`
/ `/work/new`; extend the relay to N forward targets for a small allowlist
(one socat per target, or one relay with multiple listeners).

**Effort**: ~2 days.

## 4. Publish ergonomics: PR-style handoff and conflict recovery

**Why**: v1 pushes branches; a human still has to notice and merge. And a
non-fast-forward publish (raced branch) is a dead end that requires manual
git surgery.

Plan outline: (a) optional per-root `forge` config (GitHub/GitLab API base +
token env) so a successful publish can open/refresh a PR/MR and record its
URL in `result_refs.published.pr_url`; (b) on non-fast-forward, retry once
by provisioning a fresh clone of the current branch tip and cherry-picking
the run's commits (pure host-side git in `roots.rs`), surfacing a
`publish_conflict` state if that also fails; (c) per_job semantics done
properly — accumulate per-task commits on the job branch but only open the
PR when the job's criteria are all met.

**Effort**: ~3 days including forge-API plumbing behind a feature-gated
credential.

## 5. Streaming logs and richer run observability

**Why**: the UI shows a harvested log tail; while a run is live you refresh.
Operators debugging a wedged run want a live tail and a per-run event
timeline (provisioned → checkout → tool calls → turn end → publish).

Plan outline: provider `GET /sandboxes/{id}/logs?follow=true` (chunked
`docker logs -f` passthrough with the existing byte caps); a small SSE
endpoint in `den-web` proxying it; run-page auto-tail while active. Timeline
comes free from data that already exists (run row transitions +
`bear_task_events`) — render it, don't invent storage.

**Effort**: ~2 days.

## 6. Remote/multi-provider dispatch

**Why**: one `SANDBOX_SERVER_URL` today. Toolchain-heavy fleets will want a
GPU host, a mac host, etc.

Plan outline: `SANDBOX_SERVERS` (name → url/token map) in Den config; roots
resolution becomes (provider, root); the worker fans claims across
providers, preferring ones whose catalog carries the requested root; run
rows already record `sandbox_server_url`. The provider needs no changes —
that was the point of the standalone design.

**Effort**: ~2–3 days Den-side.

## 7. Per-root build caches

**Why**: every run starts from a cold clone and cold toolchain state. For
an 11ty site that is tolerable; for a Rust workspace (this repository is an
intended work surface) each run pays a full `cargo build` before the first
test — most of the timeout budget gone before any work happens.

Plan outline: per-root persistent cache directories under
`SANDBOX_WORKSPACES_DIR/caches/<root>/` (or a dedicated volume), declared
per root in the roots file (`"caches": ["cargo", "npm"]`) and bind-mounted
into sandboxes at conventional paths (`/home/bear/.cargo/registry`,
`CARGO_TARGET_DIR=/cache/target`, `~/.npm`, …) via a small cache→mount table
in the backend. Trade-off to document: runs within one root share cache
state (poisoning is possible, isolation weakens run-to-run) — acceptable for
trusted surfaces, per-root opt-in. Add cache size to the health payload and
a `DELETE /roots/{name}/cache` operator endpoint. **Effort**: ~1–2 days.

## 8. Command-criteria verification at harvest

**Why**: a run succeeds iff the model marks the task done — it grades its
own homework. Docket already has the vocabulary for better:
`DocketCriterionKind::Command` criteria exist but are never executed for
work runs.

Plan outline: add the one lifecycle verb the provider lacks — `POST
/sandbox/v1/sandboxes/{id}/exec` (`docker exec` into the workspace,
timeout- and output-bounded, workspace cwd). At harvest, when the model
marked the task done, run the task's/job's command-kind criteria in the
sandbox before teardown; any nonzero exit downgrades the outcome to blocked
with the command output as the reason, recorded per criterion in
`bear_criterion_state`. Narrative criteria stay model-assessed. Prompt
addendum so the model knows verification runs afterward. Headless-testing
surfaces (Godot) are the motivating case. **Effort**: ~2 days.

## 9. Per-job run serialization — IMPLEMENTED

**Why**: the duplicate-run guard was per-*task*, so a job with two work
tasks could run both concurrently — and with `per_task` publishing they
race the same work branch: both provision from the same base, the second
push is a guaranteed non-fast-forward `publish_failed`, and the branch no
longer accumulates tasks sequentially as designed.

Implemented in `den-docket/src/work_runs.rs`: `claim_next_work_run` only
claims a queued run when its job has no other in-flight run (claimed/
provisioning/running/reporting), so queued runs drain one at a time per job
in queue order; expired-lease takeovers are unaffected. A post-claim
recheck releases a freshly claimed run back to queued if a concurrent
worker raced an older sibling into flight (deterministic older-run-wins
tie-break). Dispatching several tasks up front now queues them all and
executes them sequentially against the accumulating work branch.

## 10. Resource-limit defaults actually applied

**Why**: the backend supports `--memory/--cpus/--pids-limit`, but the
dispatch worker sends none of them — sandboxes are unbounded except wall
clock; one runaway build can OOM the engine.

Plan outline: `SANDBOX_DEFAULT_MEMORY_MB` / `SANDBOX_DEFAULT_CPUS` /
`SANDBOX_DEFAULT_PIDS` in Den config, threaded into `SandboxLimits` by the
worker; per-root ceilings later fold into item 3 (per-task policy).
**Effort**: ~½ day.

## 11. Blocked-run attention loop — visibility half DONE

**Why**: a blocked run just sat in `/work`; nobody was told. Autonomy needs
a return path to a human (or a triaging bear).

Done: pull-based visibility. `attention_work_runs` (latest-attempt runs
that ended blocked/failed/timed_out with no retry queued) surfaces as a
"Needs attention" section on `/work` and as `work_attention` (with blocked
reasons) in `get_job`, so any bear or human checking a job sees what needs
triage without extra calls.

Remaining (push half): on blocked/failed finalize, post a message into the
job creator's conversation with the bear (reason + run link + suggested
next step), reusing the existing conversation-persistence path; the
chat/pair bear can refine the task (`update_task`) and re-dispatch.
Optional digest for `succeeded` with publish results. Later: a
`watch`-stance policy that auto-triages retryable blockages.
**Effort remaining**: ~1 day.

## 12. Job lifecycle completion

**Why**: nothing marks a job `completed` when its last task succeeds and
criteria hold; jobs accumulate in `running` and the UI cannot distinguish
"in progress" from "done but unjudged".

Plan outline: after a successful final work task (no remaining
pending/blocked tasks), evaluate job criteria — command-kind via item 9's
exec, narrative via the existing `evaluate_criterion` flow — and set the
job `completed`, or surface "all tasks done, awaiting criteria review" in
`/work` and the job tools. **Effort**: ~1 day after item 8.

## 13. Scheduled jobs

**Why**: `bear_job_runs` already carries `trigger`/`schedule_ref` columns
that only ever hold `manual`/`event`. Recurring maintenance ("rebuild and
publish nightly") is a natural work-stance fit.

Plan outline: `bear_jobs.schedule` (cron expr) + a small scheduler sweep in
the existing workers loop that opens a scheduled `bear_job_runs` row and
enqueues the job's work tasks (serialization from item 9 keeps them
ordered); guard against overlapping runs of the same job; surface next-run
time in `/work` and `get_job`. **Effort**: ~1–2 days.

## 14. Smaller items

- **Catalog-declared image digests** (`image: "…@sha256:…"` encouraged in
  docs; optionally verify at provision) — supply-chain hygiene, ~½ day.
- **Workspace cache reuse** for big repos: keyed by root+ref, clone from a
  local cache instead of the pristine mirror every run (git alternates),
  ~1 day; measure first — local clones are already cheap via hardlinks.
- **dind-aware usage accounting**: fold sidecar disk/cpu into
  `SandboxUsage`, ~½ day (after item 1).
- **Relay-orphan sweep**: provider-side cleanup of `den-sbx-gw-*` /
  `den-sbx-net-*` whose sandbox container vanished outside the API, ~½ day.
- **`per_job` distinct from `per_task`** (single squash/PR at job
  completion) — folds into item 4c.
