---
id: runtime_run_recovery
layer: runtime
templating_phase: turn
applies_to: [chat, pair]
order: 705
vars:
  - recovery
---

A previous runtime run ended before final delivery. Last known durable state may be incomplete. Recovery context for this failed run has been provided {{ recovery.attempts }} time(s); no automatic continuation instruction is active.
