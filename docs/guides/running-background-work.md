# Running a job as background work

How to hand a piece of work to a bear so it runs autonomously in a sandbox —
no chat session open, results reported back to you — and how to follow it.

Background work runs in the **`work` stance**: one dispatched Docket Job gets one isolated sandbox/workspace/session. Inside that Job Run, the Bear advances the Job's task tree one task at a time, cannot ask you questions mid-run, and reports progress against task and Job completion criteria. If the job's commit policy allows it, the changes are pushed to a
branch on your repository's upstream.

## What you need

- A **work surface**: a named git upstream managed at **`/work/surfaces`**.
  Anyone can create one (repo URL, default branch, default toolchain image,
  optional deploy key/token — stored encrypted, never shown again). The
  creator owns it and **assigns bears** to it; only assigned bears can run
  jobs on it. Ask the bear to run `get_work_catalog` to see which surfaces
  it can use.
- A bear you are a member of that is **assigned to the surface**, and a
  running work stack (sandbox provider + Den workers). If dispatch fails
  with "no sandbox provider configured", see the
  [ops guide](sandbox-server-ops.md).

## Create the job

A job bundles a durable goal, one or more tasks, and the policies that govern
execution. Two equivalent paths:

**Conversationally** (chat or pair session) — ask the bear, for example:

> Create a job on the `site` work surface to fix the broken footer links.
> One task, assigned to work, commit policy per_task. Criteria: every footer
> link resolves, and the change is committed.

The bear uses `create_job`. The pieces that matter:

- **Tasks assigned to `work`** are the ones that run in sandboxes. Each task
  needs concrete **completion criteria** — the run is judged against them,
  so "the tests pass" beats "improve the code".
- **`commit_policy`** decides what happens to source changes:
  - `per_task` / `per_job` — each successful run's commits are **pushed to
    the job's work branch** on the upstream.
  - `none` — no changes expected (investigations).
- **`work_branch`** (optional) — the upstream branch pushed to. Leave it
  unset and the job gets `den/job-<short-id>`. The repository's default
  branch is never pushed implicitly.

**Web UI** — go to `/work` → **New work job**. The form covers the same
fields: bear, goal, work surface (a select of surfaces assigned to your
bears), commit policy, optional branch, and task rows (title +
semicolon-separated criteria).

## Dispatch it

Nothing runs until the Job is dispatched:

- In conversation: "dispatch the footer Job" (`dispatch_work` with `job_id`). Optionally pick a toolchain **image** from the catalog (`get_work_catalog` lists `rust`, `node`, `godot`, etc.) when the surface default is not right.
- In the UI: the Job page has one **Dispatch job** action.

Dispatch queues one **Job Work Run**. A background worker claims it, prepares the work surface once, starts one sandbox and headless Work session, and the Bear advances the Job's runnable task tree in task order. Task status remains run-scoped Docket state inside that Job Run. Runs of different Jobs may execute concurrently; a Job cannot have two active Work Runs.

## Follow progress and read results

- **`/work`** lists active runs (auto-refreshing) and history. The run page
  shows state, the recognized work surface, changed files with the diff, a
  log tail, resource usage, and — for pushable jobs — the **published
  branch and commit**.
- In conversation: `list_work_runs` / `get_work_run` return the same
  information, including publish results in `result_refs`.
- **Cancel** and **Retry** are on the run page (retry starts a new attempt).

Outcomes:

- **succeeded** — the bear marked the task done against its criteria. For
  pushable jobs the run page shows the branch/commit that landed upstream;
  fetch the branch and review or merge it like any other.
- **blocked** — the bear could not finish and recorded a specific reason.
  Fix the blocker (or refine the task) and retry.
- **failed** — infrastructure trouble (provisioning, timeouts, crashes).
  These retry automatically up to `WORK_MAX_ATTEMPTS`.
- A success with **"PUBLISH FAILED"** in the summary means the work happened
  but the push didn't (e.g. branch conflict); the diff on the run page still
  has everything.

## What the sandbox can and cannot do

- It works on a **fresh clone** — your working copies are never touched.
- By default it has **no network access except back to Den**: no package
  downloads, no external APIs. Pick a toolchain image that already contains
  what the task needs.
- It cannot push; publishing happens outside the sandbox, only where the
  commit policy allows, and never to the default branch.
