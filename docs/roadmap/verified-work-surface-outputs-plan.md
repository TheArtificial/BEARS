# Plan: Verified work-surface outputs and repair of job `8b99ebb4`

**Status:** Proposed implementation plan  
**Scope:** Docket and work-surface control plane; no stance-specific behavior

## Problem

Job `8b99ebb4-d1cb-4f77-a1f7-44a86a7256d2` exposed two independent failures:

1. A sandbox worker reported edits and a commit/push, but its result persisted neither a reachable Git ref nor a patch/artifact. The job therefore has no durable, independently verifiable output despite its narrative claim.
2. The task was marked done before output/validation was verified, while a stale job-run settlement later marked the job blocked for unfinished work. The final task projection and job-run state contradict one another.

The correction is not Git-only. A work surface determines which typed durable outputs it supports and how it verifies them; Docket stores the evidence and controls task/job settlement.

## Target invariant

- Focus/selection never starts work or changes lifecycle.
- A task is executing only while a live work run selects it.
- A work surface declares supported `WorkOutputKind` values and may provide a default.
- A task may constrain output only when necessary; otherwise it may accept the surface default or another supported output.
- A worker report is not completion evidence.
- A task requiring output completes only with a surface-verified durable output plus recorded required-validation evidence (or an explicit waiver).
- Task/job projections are recomputed from durable task outcomes, live work-run evidence, and verification records. Historical job-run snapshots cannot overwrite them.

Initial output kinds:

```rust
enum WorkOutputKind {
    GitCommit,
    GitPatch,
    FileBundle,
    Report,
}
```

## Implementation sequence

### 1. Audit and make the current failure reproducible

- Trace work-run finalization, task completion writes, job-run settlement, Git publication, sandbox artifact capture, and criteria evaluation.
- Add a regression scenario matching `8b99ebb4`: a worker claims a commit but yields no verified ref/artifact; required validation is unrunnable; settlement must leave the task blocked and must not mark it done.
- Add a second regression scenario where task completion occurs near job-run finalization; a stale job-run snapshot must not set a completed task tree to blocked (or vice versa).

**Done when:** both cases fail against the pre-change logic and are covered by focused tests.

### 2. Define typed output contracts and evidence storage

- Add `WorkOutputKind` and strongly typed candidate/verified-output structures in `den-core` or the smallest existing shared Docket/work-surface type module.
- Model a work surface's supported output kinds and optional default. Do not make default output mandatory for every task.
- Add durable run-output and verification records, including work run, task, job, surface, output locator/artifact reference, digest where applicable, verifier state, timestamps, and structured failure reason.
- Reuse Den artifact refs for patch, file-bundle, and report output; do not use free-form locators or provider-defined kind strings.

**Done when:** invalid output-kind/locator pairings are unrepresentable in Rust, and a migration preserves existing work-run history without inventing verified output.

### 3. Implement surface verification adapters

- Git commit: verify commit existence and reachability from the declared job/output ref.
- Git patch, file bundle, report: verify finalized artifact existence and digest/provenance binding.
- Make sandbox completion capture a patch/bundle candidate when it cannot publish a verified Git output; preserve raw logs regardless.
- Unsupported, missing, or unverifiable candidate output yields a structured blocked result, retaining the candidate/evidence for recovery.

**Done when:** Git and artifact-backed sandbox outputs both have one executable verifier path, and a report can be accepted on a Git-backed surface if that surface supports reports.

### 4. Gate task settlement and validation

- Make work-run finalization persist candidate output and run validation evidence before task settlement.
- Require verified output when the selected surface/task contract requires it.
- Require success evidence for non-waived command/check criteria. An unrunnable command is unmet/blocked, not passed.
- Remove any completion path based only on a worker result summary, claimed SHA, or claimed push.

**Done when:** completion cannot be written without its required verification records, and failure leaves a recoverable blocked task with durable evidence links.

### 5. Make job projection one-way and reconcile legacy state

- Recompute task/job state from current task outcomes, criteria, verified outputs, and live work runs in one settlement transaction or equivalent serialization boundary.
- Prevent historical job-run state from independently setting a job to blocked/completed.
- Add a reconciliation command/migration for records like `8b99ebb4`: identify completed tasks lacking required verified output, reclassify them as blocked with an explicit `output_unverified` reason, and recompute the containing job.

**Done when:** no durable state can say both “all required tasks complete” and “blocked because tasks are unfinished,” and legacy contradictions are explainable/recoverable.

### 6. Repair and rerun job `8b99ebb4`

- Inspect whether the claimed `4ea72858fbbb19644c6b843674a2f64ad36d9b3e` is reachable on any relevant ref. Do not trust the claim without verification.
- If a verifiable commit/ref or captured patch is found, attach it as candidate output and run the normal verifier.
- Otherwise preserve the worker log as evidence, mark the task blocked for missing durable output and unmet validation, and create/retry the task from a surface able to publish output.
- Run the correct feature-enabled focused validation or explicitly record why it is blocked; do not mark the task complete until the criteria are satisfied/waived through the normal mechanism.

**Done when:** the job has one coherent projected state and a task is either completed with verified output/validation or blocked with a truthful structured recovery path.

## Validation

At minimum:

```text
cargo test -p den-docket --lib
SQLX_OFFLINE=true cargo check -p den-web
```

Add focused integration tests for Git verification, artifact-backed sandbox output, report output on a Git-backed surface, missing output, failed/unrunnable validation, and job settlement ordering. Run the smallest relevant command for each changed crate before broad workspace checks.

## Non-goals

- A generic plugin/string-based provider output protocol.
- A broad artifact platform beyond the four initial typed output kinds.
- Automatically retrying or replaying the lost sandbox execution.
- Treating a Git work surface as commit-only.
