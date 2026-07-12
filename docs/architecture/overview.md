# Overview

This document is the one-page system picture for Bear Den.

For the full canonical runtime description, read [den runtime](den-runtime.md). This page is the compressed view: components, responsibilities, and the main execution/data flows.

## Core claim

Bear Den is a Den-owned assistant system with:

- one in-process runtime loop per turn, executed inside Den;
- one canonical Bear identity composed of multiple stances;
- one control plane for access, routing, approvals, tasks, and configuration;
- two canonical persistence domains:
  - per-Bear SQLite for Bear cognition and curated memory;
  - Den Postgres for conversations, approvals, compiled configs, Docket work, identity, and schedulers.

## Main components

| Component | Responsibility |
|-----------|----------------|
| **Den runtime core** | Turn orchestration, context assembly, tool execution, continuation, compaction, and event production |
| **Den Postgres** | Conversations, transcript artifacts, approvals, users, Bears, compiled configs, prompt memory blocks, reflection queue, Docket jobs/tasks |
| **Per-Bear SQLite** | Canonical Bear cognition: memory records, promotions, proposals, observations, reflection outcomes |
| **Bifrost** | Model gateway used directly by Den for inference |
| **ACP/BearWire edge** | Trusted armature integration for local tools, permission UI, and turn projection |
| **Web/API edges** | Browser chat, operator/admin surfaces, JSON/API entry points |
| **Sandboxes** | Den-managed isolated work surfaces for code execution, filesystem access, and outbound work when required |
| **Git** | Human-authored repository artifacts such as prompt fragments, policies, and skill source material |

## System diagram

```text
Humans / clients
  |- ACP armatures
  |- web chat / operator UI
  |- API consumers / future channels
          |
          v
       Den edges
          |
          v
   Den runtime core
  |- turn orchestration
  |- context assembly
  |- tool routing
  |- approval handling
  |- semantic event stream
          |
   -----------------------------
   |            |             |
   v            v             v
Bifrost   Den Postgres   per-Bear SQLite
                          |
                          v
                    canonical Bear cognition
```

## What Den is capable of

At system level, Den can:

- host durable Bear identities with multiple stances;
- route different surfaces to the correct trust posture and tool surface;
- run interactive turns with tools and approvals;
- maintain canonical Bear memory and stance-local knowledge;
- persist replayable conversations and tool interactions;
- manage workboard plans, Docket tasks, handoffs, and autonomous work;
- schedule and run reflection/review workflows;
- expose Den-hosted tools and armature-local tools through stable model-facing descriptors;
- compile stance prompts and runtime context from managed configuration;
- and project the same core runtime into ACP, web, and future channels.

## Primary execution flows

### 1. Interactive armature turn

1. The human sends a prompt through an ACP armature.
2. BearWire/ACP edge authenticates the session and resolves the Bear + stance.
3. Den assembles turn context from compiled prompt, projected memory, prompt-memory, transcript, and tool surface.
4. Den streams inference through Bifrost.
5. The model emits content and possibly tool calls.
6. Den executes Den-hosted tools directly and emits client obligations for armature-local tools.
7. Approvals and local tool results flow back through the edge into the same turn coordinator.
8. Den persists replayable transcript artifacts and returns user-visible updates.

### 2. Web/chat turn

1. The human sends a prompt through the Den web/chat surface.
2. Den resolves the Bear, channel, and stance.
3. The same runtime loop executes in-process.
4. Den-hosted tools run directly; channel-specific approvals or restrictions are enforced by Den policy.
5. Transcript and tool activity are persisted in the same canonical stores.

### 3. Background work

1. A Bear or human creates or approves Docket work.
2. Den scheduler/queue claims the work.
3. `work` executes under a constrained capability profile, often with sandbox access.
4. Results are written to Docket/Den state and may also produce memory proposals or curated summaries.

### 4. Reflection and curation

1. Den scheduler triggers a reflection or review run.
2. A reflection lane reads canonical memory and related signals.
3. Outcomes are stored in per-Bear SQLite and queue/scheduler state in Postgres.
4. Approved promotions update shared Bear memory and may refresh derived recall indexes.

## Storage model

### Per-Bear SQLite

Use this for Bear cognition:

- canonical memory records
- supersession chains and promotions
- stance-local knowledge
- observations and memory proposals
- reflection outcomes and related cognition artifacts

### Den Postgres

Use this for control-plane and interaction state:

- users, membership, Bears, and access control
- conversations and transcript projection data
- approvals and client obligations
- compiled prompts and prompt-memory blocks
- workboard plans and Docket jobs/tasks
- reflection scheduler/queue rows

## Boundaries that matter

### Bear vs Den

- The Bear is the assistant identity and its cognition.
- Den is the infrastructure, policy, runtime, and control plane that makes that Bear usable.

### Core runtime vs edges

- The runtime is protocol-neutral.
- ACP/BearWire, web, and APIs are projections over the same core events and state transitions.

### Cognition vs work management

- Bear memory is not the task tracker.
- Docket jobs/tasks are infrastructure the Bear uses, not part of the Bear's canonical cognition.

### Den-hosted vs armature-local tools

- Den-hosted tools execute inside Den.
- Armature-local tools execute through the trusted client/armature boundary and return results to Den.

## Related docs

- [den runtime](den-runtime.md)
- [den crate architecture](den-crate-architecture.md)
- [den bear spec](den-bear-spec.md)
- [bears and den](bears-and-den.md)
- [bear channel and ACP](bear-channel-and-acp.md)
- [memory model](memory-model.md)
- [reflection system](reflection-system.md)
- [tasks and autonomy](tasks-and-autonomy.md)
