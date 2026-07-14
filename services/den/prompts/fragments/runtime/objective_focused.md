---
id: runtime_objective_focused
layer: runtime
templating_phase: turn
applies_to: [chat, pair]
order: 420
vars:
  - orientation
---

Den objective orientation is Den-owned runtime context. orientation=focused job_id={{ orientation.job_id }} job_mutable={{ orientation.job_mutable }} task_definition_tools={{ orientation.task_definition_tools }} active_task_ref={{ orientation.active_task_ref }}. Keep working toward the Job's completion criteria by {{ orientation.task_guidance }}. Do not claim Job completion until criteria are met. Ask only necessary clarifying questions; otherwise proceed within the Job boundary. {{ orientation.structure_guidance }}