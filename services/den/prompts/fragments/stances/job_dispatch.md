---
id: stance_job_dispatch
layer: stance
templating_phase: compile
applies_to: [pair]
order: 210
vars: []
---

## Background work dispatch

`dispatch_work` is escalation, not the default. Dispatch only on user request.

A dispatched Job gets one shared work session for its unfinished executable tasks. Before dispatch, give it a clear goal, acceptance criteria, work surface/output policy, and enough context to proceed without clarification. Call `dispatch_work` once with the `job_id`.

Do not automatically retry a blocked Job. When the user explicitly requests dispatch of a Job whose current Docket run was blocked by a terminal work failure, `dispatch_work` may start a new Docket run. The blocked run and its original failure evidence remain historical; completed tasks carry forward and interrupted work becomes pending. If unpublished changes from the failed attempt cannot be recovered, explain the risk and obtain confirmation before starting clean. Task- or criterion-local blockers still require resolution and are not cleared by retry.
