# Role Vocabulary

Bear Den should be described as a system of **Bears**, **roles**, **work surfaces**, **channels**, and **armatures**.

This document defines the preferred vocabulary.

## Core terms

### Bear

The durable assistant identity.

### Role

The primary operating boundary inside a Bear.

A role defines:

- capability posture
- memory scope
- tool surface
- trust and approval behavior
- typical surfaces and tasks

### Runtime instance

The concrete execution binding or active runtime realization of a role when that distinction matters in implementation or diagnostics.

### Channel

A conversation surface.

### Armature

A trusted work surface that can expose local tools.

### Work surface

A durable domain of work such as a repository, service, deployment, project, or responsibility area.

## Preferred framing

Prefer language such as:

- one Bear identity with multiple roles
- role-specific runtime contexts
- Den-owned runtime core with edge projections

Avoid making “agent” the primary architectural term.

## The word “agent”

Use carefully.

Acceptable uses:

- general industry discussion
- historical material
- concrete provider-specific compatibility references when unavoidable

Avoid using it as the default noun for the current architecture when you mean Bear, role, or runtime instance.

## Why this matters

This vocabulary keeps product identity, runtime execution, and trust boundaries distinct.

It prevents confusion between:

- the Bear users think they are interacting with
- the role boundary currently active
- the runtime machinery executing that role

## Related docs

- [bears and den](bears-and-den.md)
- [den bear spec](den-bear-spec.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [pair role](pair-role.md)
