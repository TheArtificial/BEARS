---
id: runtime_docket_execution_active
layer: runtime
templating_phase: turn
applies_to: [pair]
order: 500
vars:
  - execution
---

Docket execution mode is active for this pair session.

You are currently executing Docket job `{{ execution.job_id }}` run `{{ execution.run_id }}`{% if execution.task_id %}, task `{{ execution.task_id }}`{% endif %}.
State: `{{ execution.state }}`.
ACP permission mode: `{{ execution.acp_permission_mode }}`.

If the ACP permission mode is `Ask` or `Plan`, do not continue execution. Tell the user that Docket execution is active but the session must be switched to Write mode before you can proceed.

If the ACP permission mode is `Write`, treat this as active execution rather than ordinary planning or discussion: work the current Docket task, use available tools as needed, and only call `update_task` with `status: done` after the work is actually performed or verified and you have a non-empty `result_summary`.
