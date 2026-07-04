# Den Bear Spec

This document is the stable product/runtime contract for what a Bear is in the Den-native architecture.

It complements [den-native-runtime](den-native-runtime.md) by focusing on Bear identity, roles, capabilities, and role boundaries rather than storage and turn mechanics.

## Scope

A Bear is a durable assistant identity hosted by Den. A Bear operates through a fixed set of roles, each of which is a capability profile over the same native runtime model.

This spec covers:

- role purposes
- typical surfaces
- memory and capability boundaries
- tool and approval posture
- Bear-level invariants

## Bear invariants

Every Bear has these invariants:

1. one stable user-facing identity across surfaces;
2. one Den-owned runtime/control plane;
3. canonical cognition in per-Bear SQLite;
4. canonical task/work management in Docket and Den Postgres;
5. replayable transcript and tool activity in Den conversation persistence;
6. role-specific capability boundaries enforced by policy, not by informal prompt guidance alone.

## Roles

The core Bear roles are:

| Role | Typical surfaces | Primary purpose |
|------|------------------|-----------------|
| `chat` | web chat, messaging, future conversational channels | synchronous conversational help |
| `pair` | ACP armatures and trusted work surfaces | live collaborative work with the human |
| `review` / `curate` | internal review and memory-curation flows | memory integration, review, approval, curation |
| `work` | approved background execution | scoped autonomous or semi-autonomous execution |
| `watch` | inbound events, polling, webhooks | observation intake and interpretation |

The exact vocabulary may differ between documents where `review` and `curate` are separated more explicitly, but the architectural point is stable: the Bear uses differentiated roles with different trust and capability boundaries.

## Role capability profiles

### `chat`

- conversational, user-facing help
- light retrieval and explanation
- no broad local workspace authority by default
- may create plans or work requests, but does not perform broad autonomous execution

### `pair`

- trusted interactive collaboration inside an active work surface
- client-mediated local tools and approvals
- role-local note taking and planning
- may request broader background work rather than silently expanding scope

### `review` / `curate`

- evaluates observations, proposals, and memory changes
- controls promotion of durable knowledge
- reviews or approves certain higher-risk transitions
- owns shared-memory cleanliness and cross-role curation

### `work`

- executes approved background work
- uses scoped tools, tasks, and sandboxes
- writes auditable results and progress
- does not become a general unrestricted agent

### `watch`

- receives and interprets inbound events
- records observations
- does not perform arbitrary outbound action by default
- provides signals for later review, planning, or execution

## Memory boundary

Roles do not share one undifferentiated memory pool.

- shared canonical knowledge lives in curated Bear-global memory
- role-local knowledge remains scoped until reviewed/promoted
- transcript history is not itself the Bear's canonical knowledge store
- tasks and jobs are infrastructure, not Bear cognition

## Tool boundary

Tool access is descriptor-owned and policy-owned.

- Den-hosted tools execute inside Den
- armature-local tools execute through trusted clients
- role capability profiles decide which tools are visible and under what approval posture
- durable changes and high-risk actions should remain reviewable and auditable

## Work-surface boundary

A Bear should maintain continuity across work surfaces without flattening them together.

Examples of work surfaces:

- repositories
- local workspaces
- services and deployments
- Docket projects
- Cabinet Missions
- long-running responsibilities

Plans, memory, tasks, and runtime context should attach to these surfaces when possible.

## Approval and autonomy boundary

The Bear can participate in both interactive and autonomous work, but not through one undifferentiated authority model.

- interactive local actions may require armature/client approval
- background execution requires approved work through Den-managed control paths
- review/curation and human operators remain able to gate durable or high-risk transitions

## Related docs

- [den-native-runtime](den-native-runtime.md)
- [bears and den](bears-and-den.md)
- [bear roles](bear-roles.md)
- [pair role](pair-role.md)
- [memory model](memory-model.md)
- [tasks and autonomy](tasks-and-autonomy.md)
- [capabilities and skills](capabilities-and-skills.md)
