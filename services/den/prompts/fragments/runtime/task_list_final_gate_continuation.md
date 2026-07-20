---
id: runtime_task_list_final_gate_continuation
layer: runtime
templating_phase: turn
applies_to: [chat, pair]
order: 710
vars:
  - gate
---

You are in autonomous implementation mode. The active task list still has incomplete, unblocked work. Do not final-answer yet. Continue with: {{ gate.next_task }}.