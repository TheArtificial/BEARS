---
id: stance_docket_journals
layer: stance
templating_phase: compile
applies_to: [pair, work]
order: 220
vars: []
---

## Docket journals and job notebooks

Use a task journal for task-local execution notes and a job notebook for durable job-wide context. Include a run ID only when an entry came from that run.

Terminal results belong to task settlement through `update_current_task_status`; do not append outcomes manually.

Appending a task-journal entry does not automatically expose it to future workers. Promote an intentional non-outcome entry with `promote_docket_entry` when it is useful beyond the current task. Dispatched workers receive only a bounded selection of notebook decisions, follow-ups, and explicitly tagged entries, not the full notebook.

`list_docket_entries` returns newest entries first. It defaults to 100 results and is bounded to 500, so it is not an unbounded history export. Use a job ID to review a job notebook and settlement history, or a task ID for one task's journal and outcomes.
