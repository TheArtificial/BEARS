# Interactive Stances and Role Axes

This document separates several concepts that are easy to conflate in interactive Bear Den behavior.

In particular, it distinguishes:

- stance
- channel
- armature
- work surface
- trust stance
- governance
- focused Job

## Four axes

| Axis | Question it answers | Examples |
|------|---------------------|----------|
| Resource | What durable work context is this about? | repo, service, mission, deployment |
| Armature | What trusted local tools/signals exist here? | ACP client tools, workspace roots |
| Trust stance | What memory/tool/approval posture applies? | pair-local writes, client approval, no shared-memory write |
| Governance | How is this run supervised right now? | interactive, grace, autonomous continuation, observational, frozen |
| Focused Job | Which durable Docket Job, if any, should stay centered? | none, `job_id` |

## Trust stances

Trust stances are durable trust contracts over one in-process Den runtime loop.

| Trust stance | Human present? | Primary boundary |
|--------------|----------------|------------------|
| `chat` | yes | conversational surface with Den-mediated tools |
| `pair` | yes | trusted local collaboration with client-gated effects |
| `curate` | no | shared-memory curation and review boundary |
| `work` | no | approved outbound execution boundary |
| `watch` | no | inbound observation boundary |

## Interactive split: `chat` vs `pair`

Both are human-present, but they are not the same:

- `chat` is conversational without assuming a trusted local workspace harness
- `pair` is collaborative with a trusted armature and active local work surface

## Governance

Governance describes how the run is supervised right now. It is the continuation-pressure axis, not the work-objective axis.

Examples:

- interactive
- grace
- observational
- autonomous continuation
- frozen

Trust stance is the slower trust/memory boundary. Governance is the faster supervision dial.

## Focused Job

A **focused Job** is the Docket Job designated as the durable objective for a run.

While a focused Job is present, Den steers continuation toward the next logical incomplete, unblocked task for that Job until the Job is complete, blocked, cancelled, focus is cleared, or loop-control checkpoints stop the run.

This is deliberately **not** a generic "focus target" abstraction yet. The only supported durable focus object is a Docket Job; if another object later needs equivalent lifecycle and completion semantics, generalize then.

Typical combinations:

| Trust stance | Governance | Focused Job |
|--------------|------------|-------------|
| `pair` | `interactive` | none |
| `pair` with explicit focus | `interactive` or explicit `autonomous_continuation` | designated Job |
| `work` | `autonomous_continuation` | designated Job required |
| `watch` | `observational` | optional Job being observed |

Given governance and focused Job, Den derives ephemeral **task focus**: the current best next incomplete/unblocked task or action. Task focus is a projection, not durable task state and not a user-selected mode.

## Why this distinction matters

These axes prevent the system from collapsing all interactive behavior into one flat “stance” concept.

They help describe:

- why `pair` and `chat` differ even when both are human-present
- why a run can change supervision without changing trust stance
- why `work` normally requires a focused Job while `pair` only enters focused Job behavior explicitly
- why work surfaces outlive one session or one channel

## Related docs

- [bear stances](bear-stances.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [pair stance](pair-stance.md)
- [stance vocabulary](stance-vocabulary.md)
