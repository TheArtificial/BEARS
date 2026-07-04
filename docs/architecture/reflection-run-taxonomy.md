# Reflection Run Taxonomy

This document names the major reflection run types used in Bear Den.

Reflection runs are bounded background executions used for review, curation, maintenance, and improvement work.

## Summary

- reflection is the overall system
- a reflection lane is a type of work
- a reflection run is one bounded execution in a lane
- different lanes exist because different review and maintenance responsibilities should not be collapsed together

## Top-level model

```text
Reflection
  -> lanes
  -> runs
  -> outcomes
```

## Common lanes

| Lane | Purpose |
|------|---------|
| `pair_reflect` | summarize and structure pair-local learnings |
| `memory_curate` | review local memory and maintain shared memory |
| `archive_harvest` | mine durable candidates from approved/eligible source material |
| `archive_index` | refresh derived recall over canonical sources |
| `watch_observation_review` | review observations from inbound events |
| `work_result_review` | review results of approved background work |
| `skill_review` | evaluate reusable procedures or skill proposals |
| `skill_apply` | apply approved behavior/skill changes under policy |
| `introspection` | inspect behavior failures, patterns, and costs |
| `health_check` | operational/runtime health review |
| `cleanup` | bounded cleanup or supersession work |
| `human_review_escalation` | surface sensitive or unresolved cases |

## Why separate lanes exist

Different lanes differ in:

- allowed tools
- trust/risk level
- who or what makes the semantic decision
- storage effects
- approval requirements

For example, memory curation should not be treated the same as health checking, and archive indexing should not be treated the same as skill application.

## Lane outcomes

Different lanes may produce:

- memory promotions or supersessions
- proposals for later review
- derived recall refresh requests
- result summaries
- escalations to humans or operators
- operational diagnostics

## Related docs

- [reflection system](reflection-system.md)
- [memory model](memory-model.md)
- [tasks and autonomy](tasks-and-autonomy.md)
