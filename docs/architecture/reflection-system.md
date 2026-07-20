# Reflection System

Reflection is Bear Den's auditable background review, curation, and improvement system.

It is how the system turns raw activity into reviewed memory changes, proposals, summaries, and maintenance work without collapsing those responsibilities into the interactive turn.

## Summary

- Reflection is the umbrella system for bounded background review and improvement
- a reflection run is one bounded execution
- a reflection lane is a kind of work within that system
- Den owns scheduling, locking, orchestration, and audit
- review/curation lanes make semantic decisions; Den does not fake those decisions with heuristics

## Core concepts

| Term | Meaning |
|------|---------|
| Reflection | the overall background review/learning system |
| Reflection run | one bounded execution of reflection work |
| Reflection lane | a specific work type such as memory curation or introspection |
| Reflection proposal | a proposed durable change discovered through review or analysis |
| Conductor | Den-side orchestration that schedules and advances reflection work |

## What Reflection does

Reflection can:

- curate shared memory
- evaluate stance-local learnings
- summarize or promote useful durable knowledge
- inspect behavior failures and repeated patterns
- produce proposals for skill or workflow changes
- refresh or request maintenance of derived recall indexes
- surface unresolved or risky situations for human review

## Major lanes

| Lane | Purpose |
|------|---------|
| `memory_curate` | review local memory and maintain shared memory |
| `introspection` | inspect repeated behavior, failures, and patterns |
| `skill_review` | turn repeated useful behavior into governed skill proposals |
| `archive_index` | refresh derived recall over canonical sources |
| `health_check` | operational/runtime health review |
| `cleanup` | bounded cleanup or supersession work |
| `human_review_escalation` | surface risky or unresolved cases |

## Storage boundary

Reflection is split across the same architecture boundary as the rest of the system:

- canonical reflection outcomes that change Bear cognition belong in per-Bear SQLite
- scheduler/queue and orchestration state belong in Den Postgres

This keeps cognition with cognition and infrastructure with infrastructure.

## Conductor responsibilities

The conductor is Den-side infrastructure, not a Bear stance.

It should:

1. select pending work for a lane
2. enforce Bear/lane locking and budgets
3. assemble bounded context and allowed tools
4. invoke the appropriate stance/runtime behavior
5. record run state, decisions, and audit events
6. surface status to operators and future UI projections

## Memory curation

Memory curation asks: what should the Bear remember?

It reviews stance-local memory, observations, proposals, and outcomes, then decides whether to:

- retain locally
- summarize
- promote to shared memory
- mark reviewed or superseded
- create a follow-up proposal

## Introspection

Introspection asks: what happened in the Bear's behavior?

It may inspect:

- repeated errors
- costly or slow workflows
- recurring human corrections
- useful repeated procedures
- prompt/tool/policy mismatches

It usually produces observations or proposals, not direct high-risk changes.

## Adaptation

Adaptation asks: how should behavior improve?

Reflection can propose:

- skill changes
- workflow changes
- policy follow-ups
- prompt or instruction refinements

High-risk changes remain explicitly governed.

## Budgets and bounds

Every reflection run should be bounded.

Examples of budgets:

- wall-clock time
- number of proposals reviewed
- number of writes or promotions
- retrieval/tool budget
- cost budget

Reflection is meant to do useful bounded work and stop.

## Visibility and audit

Reflection should be visible and reviewable.

Operators and future UIs should be able to see:

- recent reflection runs
- pending proposals
- lane outcomes
- failures and escalations
- what changed in memory or policy as a result

## Related docs

- [memory model](memory-model.md)
- [tasks and autonomy](tasks-and-autonomy.md)
- [den runtime](den-runtime.md)
- [reflection run taxonomy](reflection-run-taxonomy.md)

## When reflection happens

Reflection is not limited to interactive turns or explicit session close events. Session close is one useful lifecycle signal, but it is not the only opportunity to reflect.

The conductor may schedule reflection runs when bounded source material becomes eligible for review. Common triggers include:

- explicit lifecycle events, such as session close
- stale-open sessions that have had no recent activity
- open sessions that have accumulated enough new activity since the last reflected watermark
- periodic maintenance sweeps
- human or admin review requests
- worker recovery or retry after failed reflection work

Open-session reflection does not close the session. It is a checkpoint over a bounded activity window. A later close event may schedule another final checkpoint for any remaining unreflected activity.

Reflection scheduling should be idempotent. Each run should record the source window or watermark it reviewed so future runs can avoid duplicating work. Automatic reflection may produce no durable memory changes; recording that a window was reviewed is still a valid outcome.
