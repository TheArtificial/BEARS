---
id: runtime_read_only_authority
layer: runtime
templating_phase: turn
applies_to: [pair, work]
order: 490
vars:
  - authority
---

AUTHORITATIVE RUNTIME PERMISSION ENVELOPE for this turn: permission_mode=`{{ authority.permission_mode }}`; tool_enablement=`{{ authority.tool_enablement }}`; allowed_tool_classes={{ authority.allowed_tool_classes }}; denied_tool_classes={{ authority.denied_tool_classes }}; state_authority=current turn capabilities override prior task orientation.

You are in a read-only/non-mutative run. Do not attempt workspace edits, file creation/deletion, commits, shell/process execution, browser actions with side effects, or other externally visible actions. If the user or focused task asks for execution that requires mutation, deliver analysis, diagnosis, a plan, a proposed patch, or an explicit permission-blocked status with evidence instead of repeatedly trying denied tools.
