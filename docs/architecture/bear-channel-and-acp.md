# Bear Channels and ACP

This document explains how Bear Den distinguishes conversational **channels** from trusted work-surface **armatures**, and how ACP fits into that model.

## Summary

- A **channel** is a conversation surface.
- An **armature** is a trusted work surface that can expose local tools.
- ACP is the protocol Bear Den uses for armature-style clients.
- BearWire is the Den-to-armature wire used by the current ACP adapter path.
- The runtime below those surfaces is Den-native and protocol-neutral.

## Core distinction

### Channels

Channels carry conversation between humans and Bears.

Examples:

- web chat
- messaging/chat integrations
- future mobile or chat surfaces

Channels may support rich UI, attachments, and approvals, but they are not assumed to have trusted access to the user's local machine or workspace.

### Armatures

Armatures provide a trusted work-surface harness for the Bear.

Examples:

- ACP-enabled editors
- future local IDE integrations
- future local workspace CLI/TUI surfaces

An armature may expose:

- filesystem tools
- git tools
- terminal/process execution
- browser or local environment integrations
- permission UI for risky local actions

This is a stronger trust contract than a normal chat channel.

## Where ACP fits

ACP is the protocol used between an armature client and the adapter/client surface that speaks to Den.

In the current architecture:

1. the user interacts with an ACP-capable armature;
2. the local adapter/armature speaks ACP to the client and BearWire to Den;
3. Den runs the core turn loop;
4. Den projects tool activity, obligations, and results back through BearWire/ACP.

ACP is therefore an **edge protocol**, not the runtime itself.

## BearWire role

BearWire is the Den-to-armature wire for trusted armature flows.

It carries:

- session/turn lifecycle messages
- semantic runtime events
- tool call projection
- client obligations such as permission requests and local tool execution

BearWire is not the source of truth for runtime semantics. The Den runtime owns those semantics; BearWire projects them.

## Session and conversation model

Client sessions are not the same thing as canonical conversations.

- client session state exists to bind a surface session to a Bear and role
- canonical transcript and tool history live in Den conversation persistence
- the same runtime concepts should be usable across ACP, web, and other surfaces

## Tool ownership boundary

The crucial execution split is:

- **Den-hosted tools** execute inside Den
- **armature-local tools** execute through the trusted armature boundary

Examples of Den-hosted tools:

- memory tools
- session/context tools
- web retrieval tools
- workboard/task tools

Examples of armature-local tools:

- filesystem read/edit
- git status/diff/add/commit
- terminal/process execution
- local browser integrations exposed by the armature

The model sees one coherent tool surface, but execution location is descriptor-owned and policy-owned.

## Approval model

Armature-local actions often require human approval.

Typical flow:

1. the model requests an armature-local action
2. Den emits a client obligation
3. the armature renders permission UI with action-specific wording and scope choices
4. the human approves or rejects
5. the armature executes the local action if approved and returns the result
6. Den resumes the same turn with the result

The runtime coordinator, not the edge alone, decides when continuation is legal.

## Surface mapping

| Surface type | Typical role | Local tools? | Example |
|--------------|--------------|--------------|---------|
| Channel | `chat` | usually no | web chat |
| Armature | `pair` | yes, trusted and permission-gated | ACP-enabled editor |
| Scheduled/background | `work` | sandbox/server tools rather than user-local tools | Docket-dispatched run |
| Event intake | `watch` | no human-local tools required | webhook or polling flow |

## Design implications

This model implies:

- do not pretend a channel is an armature unless it truly has a trusted tool boundary
- do not move runtime semantics into ACP-specific code
- do not route Den-hosted tools through the armature just because the user is on an ACP surface
- do not hide or reveal tools turn-by-turn based on prompt heuristics

## Related docs

- [den-native-runtime](den-native-runtime.md)
- [den crate architecture](den-crate-architecture.md)
- [pair role](pair-role.md)
- [agent and bear environments](agent-and-bear-environments.md)
- [ADR-0043: ACP as edge adapter, protocol-agnostic core](../decisions/adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)
- [ADR-0048: core turn/client-obligation coordinator](../decisions/adr-0048-core-turn-client-obligation-coordinator.md)
