# Phase 1 MCP Catalog and Attachments Plan

**Status:** Active Phase 1 product slice.

This plan splits the general MCP catalog/attachment product work out of [`PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md`](PHASE1_NATIVE_PRODUCT_DEBT_PLAN.md). It does not replace the narrower [`HOST_BROWSER_MCP_BRIDGE_IMPLEMENTATION_PLAN.md`](HOST_BROWSER_MCP_BRIDGE_IMPLEMENTATION_PLAN.md), which is specifically about a host-browser MCP bridge.

Related plans:

- [`CAPABILITY_DISCOVERY_AND_CODE_MODE_IMPLEMENTATION_PLAN.md`](CAPABILITY_DISCOVERY_AND_CODE_MODE_IMPLEMENTATION_PLAN.md) — descriptor/taxonomy discovery model.
- [`BEAR_CAPABILITY_CONFIGURATION_AND_PORTABILITY_PLAN.md`](BEAR_CAPABILITY_CONFIGURATION_AND_PORTABILITY_PLAN.md) — portable Bear capability/configuration records.
- [`HOST_BROWSER_MCP_BRIDGE_IMPLEMENTATION_PLAN.md`](HOST_BROWSER_MCP_BRIDGE_IMPLEMENTATION_PLAN.md) — host browser bridge source.

## Goal

Let operators discover, configure, attach, inspect, and troubleshoot MCP capabilities for Bears and stances without confusing catalog records, Bear attachments, runtime discovery, and channel/armature execution boundaries.

## Scope

### 1. Catalog records

Represent MCP server/config entries with:

- name, description, source kind, transport, and version/pin where applicable;
- required secrets/config fields without storing secret values in ordinary metadata;
- allowed stances and channel/armature compatibility;
- risk class and capability descriptors;
- discovery/status diagnostics.

### 2. Bear/stance attachments

- Attach/detach catalog entries to a Bear and one or more stances.
- Store per-attachment configuration references and policy.
- Show whether the attachment is configured, discoverable, partially unavailable, or failing.

### 3. Descriptor integration

- MCP tools appear only through descriptor routing for surfaces that can actually execute them.
- Web channel turns must not inherit armature-local MCP tools by accident.
- Source metadata should remain visible enough for diagnostics.

## Non-goals

- Do not build generic process orchestration for arbitrary MCP servers unless already required by a chosen deployment path.
- Do not put secret values in exportable Bear packages or ordinary logs.
- Do not merge host-browser bridge specifics into the general catalog model.
- Do not use MCP attachments as a shortcut around stance policy or descriptor routing.

## Implementation steps

1. Inventory existing MCP discovery/config data from armatures, Den descriptors, and Bear capability configuration work.
2. Define the smallest catalog and attachment schema/API that reuses existing capability records where possible.
3. Add operator UI for catalog list/detail and Bear/stance attachment status.
4. Add attachment diagnostics: configured, secrets missing, unreachable, discovery failed, no tools, tools discovered.
5. Feed valid attachments into descriptor routing with stance/surface checks.
6. Add a smoke check that a web chat turn does not receive an armature-local MCP tool solely because a pair/work armature attachment exists.

## Acceptance criteria

- Operators can attach an MCP catalog entry to a Bear/stance and see configuration/discovery status.
- Runtime descriptor output reflects MCP tools only for compatible, configured, discovered surfaces.
- Missing secrets and failed discovery produce actionable operator diagnostics.
- Host-browser MCP bridge work remains referenced, not duplicated.
