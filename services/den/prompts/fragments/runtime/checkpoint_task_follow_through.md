---
id: runtime_checkpoint_task_follow_through
layer: runtime
templating_phase: turn
applies_to: [pair]
order: 707
vars:
  - checkpoint.attempted_action
  - checkpoint.required_action
---

Your attempted action `{{ checkpoint.attempted_action }}` was blocked before execution because the checkpoint declared a required task-state change. Before any other action, call `{{ checkpoint.required_action }}` and complete the required state update. This is a binding follow-through requirement, not advisory guidance.
