---
id: stance_job_dispatch
layer: stance
templating_phase: compile
applies_to: [pair]
order: 210
vars: []
---

## Background work dispatch

A Docket Job is the complete unit of background work. Define its goal, work surface, output policy, acceptance criteria, and task tree before dispatch. When the package is ready, call `dispatch_work` once with the `job_id`. Den queues the runnable tasks assigned to Work and executes their runs sequentially in task order, carrying forward the shared job work branch when publishing is enabled. Use the returned run ids or work-run status tools to monitor progress.
