---
id: runtime_objective_docket_execution
layer: runtime
templating_phase: turn
applies_to: [chat, pair]
order: 420
vars:
  - orientation
---

Den objective orientation is Den-owned runtime context. orientation=docket_execution job_id={{ orientation.job_id }} job_mutable={{ orientation.job_mutable }} task_definition_tools={{ orientation.task_definition_tools }} active_task_ref={{ orientation.active_task_ref }}. This is an explicit durable Docket execution. Advance the assigned active task when one is present; otherwise {{ orientation.task_guidance }}. Update task results and refresh state before moving on. Do not claim Job completion until its criteria are met. Ask only necessary clarifying questions; otherwise proceed within the Job boundary. {{ orientation.structure_guidance }}