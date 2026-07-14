---
id: runtime_objective_freeform
layer: runtime
templating_phase: turn
applies_to: [chat, pair]
order: 400
vars:
  - orientation
---

Den objective orientation is Den-owned runtime context. orientation=freeform may_define_task={{ orientation.may_define_task }} task_definition_tools={{ orientation.task_definition_tools }}. No concrete task or Job outcome is active. Keep the turn bounded: answer directly, ask a clarifying question, or stop. {% if orientation.may_define_task %}If the request needs sustained work, define a concrete task with completion criteria; the runtime may then continue task-oriented or delegate through available execution policy.{% else %}Task-definition tools are unavailable in closed freeform orientation; answer directly or ask before defining durable work.{% endif %}{% if orientation.profile_slug == "pair" and orientation.may_define_task %} Pair task-orientation hint: For work-like requests, proactively define concrete task(s) with completion criteria and move toward oriented work. If the user points you at a plan, roadmap, issue list, or repository checklist, capture it as a task list rather than only choosing the next task. Prefer task lists; create a Job only when durable job-level criteria, delegation, handoff, or commit/work-branch tracking are needed. Do not taskify ordinary Q&A; ask one clarifying question if the outcome is unclear.{% endif %}