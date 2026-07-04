# Den Concepts Overview

This document maps the current repository and runtime concepts to the live Bear Den architecture.

It is for readers who want to understand where major responsibilities live in the codebase after reading the architectural model in [den runtime](den-runtime.md) and [overview](overview.md).

## Repository shape

Bear Den is a monorepo with three main kinds of content:

| Area | Purpose |
|------|---------|
| `services/den/` | Den Rust workspace: runtime core, edges, persistence, and Den-hosted tool execution |
| `tools/bear-armature/` | ACP/BearWire armature adapter for trusted local work surfaces |
| `docs/` | Product, architecture, guide, ADR, and roadmap documentation |

## Implementation map

### Den runtime and edges

Most current implementation lives under `services/den/`.

Key runtime-oriented crates:

| Crate / area | Responsibility |
|--------------|----------------|
| `crates/den-runtime/` | Native agent loop, turn execution, context assembly, semantic events, continuation |
| `crates/den-core/` | Core vocabulary, descriptors, model-facing Den tool surface, shared types |
| `crates/den-service/` | Shared service state and domain services used by edges and runtime |
| `crates/den-memory/` | Per-Bear SQLite memory store and helpers |
| `crates/den-docket/` | Docket jobs/tasks/work management |
| `crates/den-bearwire/` | BearWire edge for trusted armatures |
| `crates/den-llm/` | Bifrost/LLM client and model registry helpers |

Composition root:

| Path | Responsibility |
|------|----------------|
| `services/den/src/main.rs` / `src/lib.rs` | startup, dependency injection, router composition |
| `services/den/src/core/tools/` | concrete builtin Den-hosted tool executors |

### Armature adapter

| Path | Responsibility |
|------|----------------|
| `tools/bear-armature/` | BearWire + ACP adapter, local tool execution, permission projection, workspace-aware approvals |

### Supporting services

| Area | Responsibility |
|------|----------------|
| `services/bifrost/` | model gateway used by Den for inference |
| optional deployment/infrastructure files | containers, startup wiring, dev/test stack |

## Architectural concepts and where they live

| Concept | Canonical architecture meaning | Typical implementation area |
|---------|--------------------------------|-----------------------------|
| Bear | Durable assistant identity | Postgres registry + per-Bear SQLite cognition |
| Role / stance | Capability profile over one native loop | runtime + compiled config + policy descriptors |
| Conversation / transcript | Canonical interaction record | Den Postgres |
| Prompt compilation | Managed prompt assembly and storage | compiled config services + runtime context assembler |
| Memory record / promotion | Canonical Bear cognition | `den-memory` + SQLite |
| Workboard / Docket task | Work management infrastructure | `den-docket` + Postgres |
| Client obligation / approval | Human or armature action needed to continue a turn | runtime coordinator + BearWire/ACP edge |
| Den-hosted tool | Tool executed inside Den | `den-core` descriptors + binary executors |
| Armature-local tool | Tool executed through trusted client surface | BearWire + `bear-armature` |

## Current source-of-truth documents

When you need the conceptual truth rather than the code layout:

- runtime: [den runtime](den-runtime.md)
- product/system picture: [overview](overview.md)
- crate decomposition: [den crate architecture](den-crate-architecture.md)
- Bear/stance contract: [den bear spec](den-bear-spec.md)
- memory: [memory model](memory-model.md)
- tasks/autonomy: [tasks and autonomy](tasks-and-autonomy.md)
- channels/armatures: [bear channel and ACP](bear-channel-and-acp.md)
