# Brief: Build the native `work` sandbox system

## Mission

Build a substantial first version of Den’s native **`work` sandbox**: a system that lets Docket-backed jobs/tasks be assigned to a `work` stance, executed in an isolated workspace, observed by humans, and managed through both model tools and a web UI.

This should be a real vertical slice, not a toy. It should be capable of taking a Docket task, preparing an appropriate work surface, executing bounded coding-agent work in a sandbox, recording results, surfacing status/usage, and reporting completion or blockage back into Docket.

You are trusted to refine Docket and adjacent APIs where needed, as long as concepts stay clean and migrations/backward compatibility are handled deliberately.

---

## Product intent

Den has multiple stances. Chat is the synchronous user-facing front door. `work` should become the autonomous coding/execution stance.

The desired future flow:

1. User or assistant creates a durable Docket job.
2. Some tasks are assigned to `work`.
3. A `work` runner checks out a task.
4. The runner recognizes the work surface, chooses/prepares a sandbox type, executes allowed operations, captures logs/results/changed files/resource usage.
5. Docket reflects task state accurately.
6. Humans can observe/manage the job from a web UI.
7. Model-facing tools can create, inspect, update, and dispatch jobs/tasks without hand-editing DB state.

---

## Core deliverable

Implement the first production-shaped version of the `work` sandbox system, including:

- Docket-driven work dispatch.
- Sandbox lifecycle management.
- Work surface recognition.
- Multiple sandbox type support, even if only one or two are fully implemented.
- Execution logging and result capture.
- Resource observability.
- Web UI for job/task/sandbox monitoring.
- Model tools for managing jobs/tasks and dispatching work.
- Tests/smoke checks proving the full path works.

This does **not** need to be perfect or maximal. It does need to be coherent, shippable, and hard to accidentally misuse.

---

## Key concepts to preserve

### Docket

Docket is the durable work system.

Canonical concepts should remain something like:

- **Job**: durable objective across turns.
- **Task**: executable or decision/investigation unit, optionally assigned to a stance.
- **Current task status**: pending, in progress, blocked, done, cancelled.
- **Completion criteria**: concrete criteria that define done.

Refine Docket as needed, but keep it boring and explicit.

Avoid confusing Docket jobs/tasks with transient chat task lists. Session task lists can project from Docket, but they are not the source of truth for durable work.

### `work` stance

A bear in `work` stance is not in a chat. It can not require synchronous user interaction to make progress on assigned tasks. It should operate from explicit task definitions, policies, and sandbox permissions.

### Work surface

A work surface is the thing being operated on:

- local repo/workspace
- checked-out branch
- copied temp workspace
- generated scratch workspace
- possibly future remote repo or artifact bundle

The system should recognize and record what kind of surface it is operating on.

### Sandbox

A sandbox is the execution boundary around work.

The implementation should support a typed model, for example:

- `local_workspace_readonly`
- `local_workspace_writable`
- `ephemeral_copy`
- `container`
- `remote_ephemeral`

Not all need full implementation now. But the system should be designed so sandbox type is explicit, visible, and enforceable.

---

## Suggested scope

### 1. Docket refinements

Add or improve Docket support for autonomous execution.

Likely needs:

- Assign tasks to `work`.
- Claim/check out a task for execution.
- Prevent duplicate concurrent claims.
- Track task run attempts.
- Track execution status separately from durable task definition.
- Store result summaries, logs, changed-file summaries, and error/blockage reasons.
- Associate runs with sandbox/work-surface metadata.
- Make retry/cancel behavior explicit.

Minimum useful entities/fields may include:

- job status
- task status
- assigned stance
- run/attempt id
- runner id
- lease/claim expiry
- started_at / finished_at
- sandbox type
- work surface ref
- result summary
- result refs/log refs
- resource usage snapshot

Do not add elaborate workflow machinery unless needed. Prefer the smallest durable state model that makes autonomous work safe and inspectable.

### 2. Work dispatch loop

Build a `work` runner/dispatcher that can:

- discover eligible `work` tasks;
- claim one safely;
- prepare the work surface;
- create/select sandbox;
- execute task steps;
- capture stdout/stderr/logs;
- detect changed files;
- mark task done/blocked/failed/cancelled appropriately;
- release or complete the claim.

The runner may initially be a command or service, depending on the existing app shape.

Acceptance path should include a simple task like:

> In a small repo, change a file or run a check, then report the result back to Docket.

### 3. Sandbox lifecycle

Implement a real sandbox abstraction with explicit lifecycle:

- create/prepare
- execute command or tool operation
- inspect state
- collect artifacts/logs
- clean up or preserve for debugging

At least one writable sandbox should work end-to-end. Prefer an `ephemeral_copy` of a local workspace if that fits the existing environment; it is usually safer than mutating the source repo directly.

If implementing a simpler local writable sandbox first, mark it clearly with a `ponytail:` comment noting its ceiling and upgrade path, e.g. global host process isolation limitations, no cgroup enforcement, etc.

Do not pretend a sandbox is stronger than it is.

### 4. Work surface recognition

Add recognition for common project/workspace facts:

- is this a git repo?
- current branch / commit
- dirty state
- package manager hints
- language/framework hints
- obvious test/build commands
- presence of lockfiles
- repo root vs subdirectory
- whether workspace is writable
- whether untracked changes exist

This recognition should be used to choose reasonable default behavior and should be recorded on the run.

Keep heuristics simple. Edge-case correctness matters for destructive choices: never overwrite or discard user changes casually.

### 5. Sandbox type selection

Implement a small policy layer that decides or validates sandbox type.

Examples:

- If work surface is dirty, prefer ephemeral copy or block unless explicitly allowed.
- If task requires code modification, do not use readonly sandbox.
- If no container runtime is available, degrade gracefully to supported local sandbox or block with a useful reason.
- If expected resource usage is high, require explicit policy or queueing.

The exact policy can be minimal, but it should be visible and testable.

### 6. Model tools

Expose model-facing tools for job/task management.

Likely useful tools:

- create job
- list jobs
- get job
- update job status
- create task
- list tasks
- update task definition
- update current task status
- claim/checkout next work task
- execute or enqueue work task
- inspect sandbox/run status
- cancel run
- fetch logs/result summary

If tools already exist, refine them instead of duplicating them.

Model tools should have clear schemas, safe defaults, and should not expose arbitrary destructive host operations.

### 7. Web UI

Add a minimal but useful web UI for humans/operators.

It should show:

- jobs
- tasks
- task status
- assigned stance
- current/last run
- sandbox type
- work surface
- logs/result summary
- resource usage
- errors/blockers
- controls where safe: enqueue, cancel, retry, archive/complete

Keep UI boring. Server-rendered or existing frontend conventions are fine. Avoid building a giant dashboard framework if the app already has a simpler pattern.

The UI should make it obvious when work is running and when server resources are being consumed.

### 8. Observability and resource management

Add observability for sandbox usage sufficient to operate this safely.

Track at minimum:

- run count
- active runs
- queued/claimable tasks
- run duration
- command count
- exit status
- approximate stdout/stderr/log size
- changed-file count
- sandbox disk usage if cheap to compute
- cleanup success/failure
- cancellation status

Nice to have:

- CPU time / memory if available cheaply from platform/container
- per-run event log
- per-run timeline
- simple resource limits: timeout, max log bytes, max concurrent runs

Do not write a metrics platform from scratch. Use existing logging/metrics conventions if present. If none exist, add the smallest structured event log that can support the UI and debugging.

### 9. Results and artifacts

Each run should leave behind enough information to review what happened:

- task id / job id
- runner id
- sandbox id/type
- work surface info
- commands/tool calls executed
- stdout/stderr/log refs
- exit statuses
- file changes summary
- final status
- result summary

If changed files are produced in an ephemeral sandbox, make their location or patch/diff available. Do not silently apply changes to the source workspace unless explicitly intended by the selected sandbox type/policy.

### 10. Safety and policy

Default to safe behavior.

Important rules:

- Do not destroy user changes.
- Do not commit, push, deploy, publish, or call external services unless explicitly allowed.
- Do not leak secrets into logs.
- Bound command execution with timeout and max output.
- Make sandbox strength explicit.
- Ensure cancellation and cleanup paths exist.
- Ensure failed cleanup is visible.

---

## Suggested implementation strategy

1. **Survey existing code**
   - Locate Docket models/APIs/tools.
   - Locate current task-list projection code.
   - Locate web UI patterns.
   - Locate command/tool execution utilities.
   - Locate config/env conventions.
   - Locate tests.

2. **Design the minimum state model**
   - Add run/attempt tracking if absent.
   - Add sandbox/work-surface metadata.
   - Keep migration simple.

3. **Implement work-surface recognition**
   - Pure functions where possible.
   - Add small tests.

4. **Implement sandbox abstraction**
   - Start with one real sandbox type.
   - Make unsupported types explicit rather than fake.

5. **Implement work runner**
   - Claim task.
   - Prepare sandbox.
   - Execute.
   - Record result.
   - Update Docket.

6. **Add model tools**
   - Reuse existing Docket tools where possible.
   - Add missing execution/run inspection tools.

7. **Add UI**
   - Jobs/tasks/runs overview.
   - Run detail/log view.
   - Minimal safe actions.

8. **Add observability**
   - Structured run events.
   - Resource counters.
   - UI visibility.

9. **Add end-to-end smoke test**
   - Create job/task.
   - Assign to `work`.
   - Run worker.
   - Verify task status/result/logs/artifacts.

---

## Acceptance criteria

The work is done when all of the following are true:

### Functional

- A Docket job can contain a task assigned to `work`.
- A `work` runner can claim that task without racing another runner.
- The runner can prepare a recognized work surface.
- The runner can execute in an explicit sandbox type.
- The run produces logs/result metadata.
- Docket reflects final task status accurately.
- Failed work is marked blocked/failed with useful reason.
- Cancellation or timeout is handled visibly.
- At least one sandbox type works end-to-end.

### UI

- A human can view jobs, tasks, and run status in the web UI.
- The UI shows active/past sandbox runs.
- The UI shows logs or log references.
- The UI shows resource usage enough to identify runaway or expensive runs.
- Safe controls exist for at least enqueue/run, cancel, and retry if these fit the current app model.

### Model tools

- Model-facing tools can create/list/get/update jobs and tasks.
- Model-facing tools can dispatch or enqueue `work`.
- Model-facing tools can inspect run status/results.
- Tool schemas are explicit and safe.

### Safety

- Dirty workspaces are detected and handled safely.
- Logs are bounded.
- Command execution is timeout-bounded.
- Destructive/external actions are not enabled by default.
- Sandbox cleanup is attempted and cleanup failures are visible.

### Observability

- Active runs are visible.
- Completed runs include duration and status.
- Resource/log usage is recorded.
- There is enough structured data to debug failed runs.

### Tests/checks

- There is at least one end-to-end smoke test for a trivial `work` task.
- There are focused tests for work-surface recognition and sandbox selection/policy.
- Existing Docket tests continue to pass.
- Any migration has a clear test or verification path.

---

## Non-goals

Do not spend time on these unless they fall out naturally:

- Building a perfect distributed queue.
- Supporting every sandbox backend.
- Building a full CI/CD platform.
- Implementing remote cloud execution unless already mostly present.
- Building an elaborate dashboard.
- Creating new abstractions not needed by the first real flow.
- Automatically committing/pushing code.
- Replacing Docket wholesale.

---

## Preferred tradeoffs

- Prefer one strong vertical path over many half-working paths.
- Prefer explicit state over clever inference.
- Prefer deletion/refinement of old Docket compatibility code over adding parallel systems.
- Prefer native platform features and already-installed dependencies.
- Prefer simple structured logs over a custom observability stack.
- Prefer safe blocking over unsafe mutation.
- Prefer boring, inspectable code.

If you intentionally take a shortcut, mark it with a `ponytail:` comment explaining the ceiling and the upgrade path.

Example:

```ts
// ponytail: this sandbox only isolates via an ephemeral directory, not OS-level process isolation.
// Upgrade path: add container/cgroup-backed SandboxDriver before allowing untrusted code.
```

---

## Expected final handoff

At the end, provide:

1. Summary of what was implemented.
2. How to run the worker.
3. How to create/dispatch a sample job.
4. How to view it in the UI.
5. Sandbox types supported and unsupported.
6. Resource limits/observability added.
7. Safety assumptions and known gaps.
8. Tests/checks run.
9. Follow-up recommendations.
