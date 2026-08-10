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
- Each job-to-work-surface assignment declares a mutation policy: `required` (the default), `optional`, or `forbidden`.
- `required` means the job must produce a verified durable mutation on that surface; `optional` permits but does not require one; `forbidden` makes the surface context-only and denies mutation capability.
- A work surface declares supported `WorkOutputKind` values and may provide a default.
- A worker report is not completion evidence.
- Completion evidence is derived from assigned work-surface mutation policy and the surface-specific verifier, not from the `execution | investigation | decision` task-kind enum or a model-authored per-task output-requirement schema.
- A required surface mutation completes only with a surface-verified durable output plus recorded required-validation evidence (or an explicit waiver). A job with no required mutation may complete with durable result evidence and any required validation.
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

### 2. Define typed output contracts, publication intent, and evidence storage

- Model job creation remains task-first: the model describes tasks and chooses/receives their work surfaces; it does not declare task kind plus a separate output-requirement schema.
- Add a `MutationPolicy` to each job-to-work-surface assignment: `required` (default), `optional`, or `forbidden`.
  - `required`: the job must produce a verified durable mutation on this surface before it settles successfully.
  - `optional`: a mutation is permitted but a report-only outcome remains valid; any mutation that occurs is still recorded and verified by the surface.
  - `forbidden`: the surface is context-only; mutation capabilities are not offered and any attempted mutation is rejected.
- Keep mutation policy separate from publication policy: mutation policy says whether an effect is expected or allowed; `commit_policy`/publication policy says when and how a permitted Git effect is externally finalized.
- Derive completion evidence from the assigned surface policy and its verifier, not from the `execution | investigation | decision` task-kind enum. A required Git surface needs verified Git/publication evidence; a required Cabinet surface needs an authorized durable record/revision reference; no required surface mutation needs durable result evidence and required validation only.
- Add a typed `publish_task` intent owned by Docket and delivered as a leased BearWire obligation. It identifies the task/work run, output target, expected artifact boundary, and idempotency key; it does not contain shell text, filesystem paths, raw Git arguments, or credentials.
- Keep publication as one durable operation: a provider may require Armature to prepare or validate a workspace-local artifact, but the provider owns the external effect and uses scoped credentials outside model context and, where policy requires, outside the sandbox.
- Add durable publication attempt/result and verification records, including work run, task, job, surface, output locator/artifact reference, provider evidence, digest where applicable, verifier state, timestamps, and structured failure reason.
- Reuse Den artifact refs for patch, file-bundle, and report output; do not use free-form locators or provider-defined kind strings.

**Done when:** invalid output-kind/target pairings are unrepresentable in Rust; a publication attempt has one idempotency key and one terminal outcome; provider credentials cannot appear in an Armature/model payload; and a migration preserves existing work-run history without inventing verified output.

### 3. Implement publication-provider and verification adapters

- Git publication: have the provider verify the expected commit/artifact boundary and publish to the declared job/output ref using short-lived, target-scoped authority. Record the commit OID and remote ref as immutable evidence.
- Git patch, file bundle, and report: verify finalized artifact existence and digest/provenance binding; use an appropriate artifact/publication provider when external publication is required.
- Armature may prepare or validate a workspace-local candidate, but it never receives reusable provider credentials and never decides Docket settlement.
- Make sandbox completion capture a patch/bundle candidate when it cannot produce a verifiable Git output; preserve raw logs regardless.
- Unsupported, missing, or unverifiable candidate output yields a structured blocked result, retaining the candidate/evidence for recovery.

**Done when:** Git and artifact-backed outputs each have one executable provider/verifier path, a provider result can be replayed idempotently without duplicating its external effect, and a report can be accepted on a Git-backed surface if that surface supports reports.

### 4. Gate task settlement and validation

- Make work-run finalization persist candidate output, publication evidence, and run validation evidence before task settlement.
- For `commit_policy = per_task`, replace any sandbox-owned implicit auto-commit/push path with one `publish_task` intent and provider result. Retain any per-job publication behavior only where it is explicitly selected by `commit_policy = per_job`.
- Require verified output for every work surface assigned with `required` mutation policy. A job with no required surface mutation settles from durable result evidence and required validation. The model does not select this evidence policy by task kind or a per-task output schema; Den derives it from the job's surface assignments and surface adapters.

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

## Publication and credentials

Docket owns the decision to publish a completed task and records its durable
intent, idempotency key, target, and settlement. Each job-to-work-surface
assignment has a mutation policy: `required` (default), `optional`, or
`forbidden`. `required` means successful job settlement needs a verified
durable mutation on that surface; `optional` permits a surface mutation without
requiring one; `forbidden` makes the surface context-only and denies mutation
capabilities. This expectation/authorization policy is distinct from a
publication policy such as Git `per_task` or `per_job`, which controls when and
how an allowed Git change becomes externally final. The model describes tasks
and uses assigned surfaces; it does not author a task-kind-dependent output
matrix. Surface adapters derive the needed evidence from their policy and
verifier.

BearWire projects and leases a typed `publish_task` obligation; it does not
become a second publication state machine. Armature can prepare or validate
workspace-local output but does not receive reusable credentials or arbitrary
command text. A Den-controlled publication provider performs the external
effect with short-lived, target-scoped authority and reports immutable evidence.
This is deliberately backend-neutral: Git branch publication is the first
provider, while Cabinet revisions, artifact registries, and deployments can use
the same lifecycle.

## Non-goals

- A generic plugin/string-based provider output protocol.
- A broad artifact platform beyond the four initial typed output kinds.
- Automatically retrying or replaying the lost sandbox execution.
- Treating a Git work surface as commit-only.
