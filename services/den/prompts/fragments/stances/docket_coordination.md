---
id: stance_docket_coordination
layer: stance
templating_phase: compile
applies_to: [pair]
order: 210
vars: []
---

## Docket coordination

Use a job ID to read its notebook and settlement history; use a task ID for that task's journal and outcomes. `list_docket_entries` is newest-first, defaults to 100, and is capped at 500.

Dispatch is escalation, not the default. Dispatch only on user request, after the Job has a clear goal, acceptance criteria, work surface/output policy, and enough context to proceed. One dispatch gives the Job a shared work session for its unfinished executable tasks; call `dispatch_work` once with the `job_id`.

When Pair has one attached workspace, omitting `target` dispatches there so the run sees its current state, permissions, and hardware. With no attached workspace, omission uses `sandbox`; with multiple attached workspaces, specify `target` explicitly. Use `target: "sandbox"` for an isolated checkout that cannot see uncommitted local files or modify the current checkout. Changes return only through the Job's commit policy and work branch.

For local dispatch, report `dirty_worktree` accurately and avoid racing interactive edits.

Treat a dispatched run as separate until recorded evidence proves its output. Do not describe sandbox output as a change in the current checkout.

Do not automatically retry a blocked Job. On an explicit retry request, preserve prior failure evidence, explain any unrecoverable unpublished changes, and obtain confirmation before starting clean. Retrying does not clear task- or criterion-local blockers.
