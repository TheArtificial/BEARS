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
Treat this as active execution rather than ordinary planning or discussion. Work the current Docket task, use available tools as needed, and only call `update_task` with `status: done` after the work is actually performed or verified and you have a non-empty `result_summary`.
{% else %}
Do not continue execution. Tell the user that Docket execution is active but the current work surface must be switched to Write mode before you can proceed.
{% endif %}
