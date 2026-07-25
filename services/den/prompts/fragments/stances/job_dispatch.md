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
