---
id: runtime_operational_outcome_summary
layer: runtime
templating_phase: turn
applies_to: [chat, pair]
order: 706
vars:
  - outcome.kind
---

{% if outcome.kind == "provider_stream_error" -%}
Previous turn ended with a retryable provider stream transport error after continuation started. Recent tool results were preserved, but no final answer was delivered. Verify persisted state, recent tool results, and any task/worktree status before deciding whether there is work left to do.
{%- elif outcome.kind == "continuation_timeout" -%}
Previous turn timed out after a client/local-tool result was received and continuation started, but the resumed runtime produced no event before the watchdog expired. Recent tool results were preserved, but no final answer was delivered. Verify persisted state, recent tool results, and any task/worktree status before deciding whether there is work left to do.
{%- elif outcome.kind == "continuation_start_failed" -%}
Previous turn failed while starting continuation after client results were delivered. Recent tool results were preserved, but no final answer was delivered. Verify persisted state, recent tool results, and any task/worktree status before deciding whether there is work left to do.
{%- elif outcome.kind == "turn_budget_exhausted" -%}
Previous turn stopped for budget or loop-safety reasons before delivering a final answer. Recent tool results were preserved. There is no infrastructure repair action for the model; if the user asks to proceed, verify persisted state, recent tool results, and any task/worktree status before deciding whether there is work left to do.
{%- elif outcome.kind == "operational_failure" -%}
Previous turn ended with an operational failure before final answer delivery. Do not assume the requested work completed or incomplete from prior assistant text alone. Verify persisted state, recent tool results, and any task/worktree status before deciding whether there is work left to do.
{%- else -%}
Previous turn ended before final answer delivery. Verify persisted state, recent tool results, and any task/worktree status before deciding whether there is work left to do.
{%- endif %}
