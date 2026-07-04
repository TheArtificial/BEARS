# Interactive Stances and Role Axes

This document separates several concepts that are easy to conflate in interactive Bear Den behavior.

In particular, it distinguishes:

- role
- channel
- armature
- work surface
- trust stance
- governance mode

## Four axes

| Axis | Question it answers | Examples |
|------|---------------------|----------|
| Resource | What durable work context is this about? | repo, service, mission, deployment |
| Armature | What trusted local tools/signals exist here? | ACP client tools, workspace roots |
| Trust stance | What memory/tool/approval posture applies? | pair-local writes, client approval, no shared-memory write |
| Governance mode | How is this run supervised right now? | interactive, observational, autonomous continuation, frozen |

## Trust stances

Trust stances are durable trust contracts over one Den-native runtime loop.

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

## Governance modes

Governance mode describes how the run is supervised right now.

Examples:

- interactive
- observational
- autonomous continuation
- frozen

Trust stance is the slower trust/memory boundary. Governance mode is the faster supervision dial.

## Why this distinction matters

These axes prevent the system from collapsing all interactive behavior into one flat “role” concept.

They help describe:

- why `pair` and `chat` differ even when both are human-present
- why a run can change supervision without changing trust stance
- why work surfaces outlive one session or one channel

## Related docs

- [bear roles](bear-roles.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [pair role](pair-role.md)
- [role vocabulary](role-vocabulary.md)
