---
id: runtime_objective_oriented
layer: runtime
templating_phase: turn
applies_to: [chat, pair]
order: 410
vars:
  - orientation
---

Den objective orientation is Den-owned runtime context. orientation=oriented task_ref={{ orientation.task_ref }} oriented_root_task_id={{ orientation.root_task_id }} max_children={{ orientation.max_children }} max_depth_below_oriented_task={{ orientation.max_depth_below_oriented_task }}. A concrete task is active. Keep working toward its completion criteria, but pause when the user asked only to plan or when proceeding would exceed the requested scope. Do not claim completion until criteria are met. Ask necessary clarifying questions; otherwise proceed within the task boundary. Task creation is bounded to oriented_root_task_id={{ orientation.root_task_id }}, max_children={{ orientation.max_children }}, and max_depth_below_oriented_task={{ orientation.max_depth_below_oriented_task }}.