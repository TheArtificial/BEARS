# Den Tool Surface and Runtime Configuration Boundary

> **Note (2026-06).** "During the Letta migration" framing refers to the Den runtime effort; the current runtime is the in-process Den loop. See [Den runtime](den-runtime.md) ([migration plan](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md)).

This document defines the implementation-facing boundary for Den-owned tool surfaces and runtime configuration during the Letta migration.

## Purpose

Den needs to replace Letta-shaped tool and runtime configuration ownership with an explicit Den-owned model.

This note defines:

- what Den owns about tool schemas and tool policy,
- what belongs at client/actuator edges such as ACP,
- what remains in temporary provider-compatibility shims,
- and how runtime/model/configuration settings should be framed.

## Architectural stance

The goal is not a generalized pluggable backend architecture.

Den may keep narrow provider compatibility seams where necessary, but the conceptual model should remain Den-native and internally cohesive. A monolithic Den is acceptable.

## Tool surface layers

Den should distinguish at least these layers:

1. **Tool registry metadata**
   - Den-owned model-visible tool names, descriptions, schemas, categories, scope guidance, and approval requirements.

2. **Tool policy**
   - Den-owned enablement, approval class, path/resource restrictions, role/mode gating, and audit expectations.

3. **Execution class**
   - server-side Den tools,
   - client-mediated tools such as ACP local workspace/process tools,
   - external/provider-mediated tools where compatibility remains necessary.

4. **Transport framing**
   - ACP-specific event framing, web/API-specific response shapes, and other edge presentation details.

These layers should not be conflated.

## Runtime configuration layers

Den should distinguish at least these configuration layers:

1. **Role profile configuration**
   - Den-owned model policy, tool availability, memory scope, autonomy/risk settings, compaction policy, and role defaults.

2. **Runtime execution policy**
   - per-session or per-run mode, context budget behavior, current role state, workflow/plan state, and turn lifecycle constraints.

3. **Provider binding details**
   - provider-specific model identifiers, compatibility backend flags, provider API knobs, or temporary adapter settings.

4. **Client/actuator contract details**
   - ACP-specific capabilities, adapter environment shape, client tool availability, and work-surface/workspace context.

Provider binding details should remain implementation-level rather than acting as the core identity/configuration model.

## Invariants

### 1. Den owns the model-visible tool contract

Tool names, schemas, descriptions, and approval semantics visible to the model should be Den-owned even when execution is delegated.

### 2. Den owns tool policy

Whether a tool may be used, under what approval conditions, and in which role/mode should be decided by Den policy rather than hidden provider behavior.

### 3. Edge-specific framing stays at the edge

ACP or other client-specific status text, event framing, and UI projections should not become the shared runtime policy model.

### 4. Provider settings are implementation details

Provider-specific runtime knobs may exist, but they should not replace Den-owned role profile and runtime policy concepts.

## Minimum migration outcome

A Letta replacement is acceptable only if:

- Den owns the tool registry and tool policy model,
- Den owns role/runtime configuration semantics,
- ACP/client contracts remain explicit edge layers,
- and provider settings are ringfenced as compatibility details rather than system-defining abstractions.

## Minimum v1 expectations

A v1 boundary is acceptable if it provides:

- Den-owned tool metadata and approval classes,
- explicit execution-class distinctions,
- Den-owned role/runtime configuration vocabulary,
- and a clear separation between Den configuration and provider binding details.
