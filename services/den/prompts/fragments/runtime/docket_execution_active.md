---
id: runtime_docket_execution_active
layer: runtime
templating_phase: turn
applies_to: [pair]
order: 500
vars:
  - execution
---

Docket execution mode is active for this {{ execution.surface.adapter }} {{ execution.surface.stance }} work surface.

You are currently executing Docket job `{{ execution.job_id }}` run `{{ execution.run_id }}`{% if execution.task_id %}, task `{{ execution.task_id }}`{% endif %}.
State: `{{ execution.state }}`.
Execution gate: `{{ execution.gate.state }}`.
Gate reason: `{{ execution.gate.reason }}`.
Required action: `{{ execution.gate.required_action }}`.
Permission source: `{{ execution.permission.source }}`.
Permission mode: `{{ execution.permission.mode_label }}`.

{% if execution.gate.state == "open" %}
Treat this as active execution rather than ordinary planning or discussion. Work the current Docket task and use available tools as needed. A focused run is not complete just because one task is done: after finishing a task, satisfy the Job's commit policy when write tools are available, record the task result with a non-empty `result_summary`, refresh the focused Job/task state, and continue the next incomplete unblocked task. Final-answer only when the Job is complete, blocked/gated, the user explicitly asked you to pause, or the runtime budget/tool permissions require stopping.
{% else %}
Do not continue execution. Tell the user that Docket execution is active but the current work surface must be switched to Write mode before you can proceed.
{% endif %}
