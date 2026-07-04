# Agent and Bear Environments

This document defines the environment vocabulary Bear Den uses to distinguish durable Bear state from the narrower runtime projections used during a role's turn or task.

## Summary

- the **Bear Operating Environment** is the durable world Den maintains for a Bear
- a **role runtime** is the situated projection used for one role in one situation
- **environment projection** is how Den maps durable state into that role runtime
- **turn context** is the concrete model-facing slice for one turn or step

## Bear Operating Environment

The Bear Operating Environment is the durable Bear-level environment Den owns.

It includes things such as:

- Bear identity and profile
- canonical memory and curated knowledge
- role-local memory availability rules
- approved tools and capability policy
- skills and related projection metadata
- work surfaces and related bindings
- users, membership, and trust boundaries
- conversations, plans, tasks, and approvals

It is broader than any one prompt or session.

## Role runtime

A role runtime is the situated environment Den projects for a role in a particular situation.

Examples of inputs to a role runtime:

- current role
- current channel or armature
- current work-surface resolution
- tool surface
- relevant memory projection
- current human/task/session context
- approval posture and policy

The role runtime is narrower than the full Bear Operating Environment.

## Environment projection

Environment projection is Den's process of mapping Bear Operating Environment into a role runtime.

Examples:

- `pair` gets trusted local-tool affordances, work-surface hints, and interactive approval behavior
- `work` gets task context, sandbox/tool scope, and stronger execution constraints
- `watch` gets inbound event context and observation-writing rights
- `review`/`curate` gets memory/proposal/review-oriented context and tools

## Turn context

Turn context is the concrete slice the model sees for one turn or step.

It typically includes:

- compiled prompt
- projected memory
- prompt-memory blocks
- conversation history
- tool descriptors
- bounded runtime reminders and policy hints

Turn context is narrower than the role runtime, which is narrower than the Bear Operating Environment.

## Relationship between the concepts

```text
Bear Operating Environment
        -> role runtime
        -> turn context
```

Or in words:

- durable Bear state lives in the Bear Operating Environment
- Den projects the correct subset into a role runtime
- the runtime serializes the immediately relevant slice into turn context

## Roles, channels, armatures, and work surfaces

These are distinct environment axes:

| Concept | Meaning |
|---------|---------|
| Role | operating mode and capability boundary |
| Channel | conversation surface |
| Armature | trusted work surface with local tools |
| Work surface | durable domain of work such as repo, service, deployment, or project |

These axes combine to determine the environment projection for a given turn.

## Design discipline

Environment design in Bear Den means intentionally shaping:

- prompts
- tools
- memory projections
- approval rules
- role boundaries
- task/run context

The goal is to give each role enough of the world to do its job without collapsing all authority and all context together.

## Related docs

- [den-native-runtime](den-native-runtime.md)
- [bears and den](bears-and-den.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [memory model](memory-model.md)
- [pair role](pair-role.md)
