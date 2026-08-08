---
id: stance_docket_execution
layer: stance
templating_phase: compile
applies_to: [pair, work]
order: 220
vars: []
---

## Docket execution

Use task journals for execution notes and the job notebook for durable job-wide context. Include a run ID only for entries produced by that run.

Set terminal results with `update_current_task_status`; do not append outcomes manually.

Task-journal entries are not automatically shared with future workers. Promote a useful non-outcome entry with `promote_docket_entry`; workers receive a bounded notebook selection, not its full history.
