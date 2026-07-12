# ADR-0055: Bear Capability Configuration, Connections, and Portability

**Status:** Proposed  
**Date:** 2026-07-13  
**Deciders:** Hans

**Related:**

- [ADR-0054: Capability Discovery and Code Mode](adr-0054-capability-discovery-and-code-mode.md)
- [ADR-0052: Three-Layer Agent Steering](adr-0052-three-layer-agent-steering.md)
- [ADR-0039: Trust Profiles and Governance Modes](adr-0039-trust-profiles-and-governance-modes.md)
- [Capabilities and Skills](../architecture/capabilities-and-skills.md)
- [Connections and Work Surface Presentation](adr-0040-connections-and-work-surface-presentation.md)

## Context

ADR-0054 establishes capability discovery as the model-facing strategy for large catalogs of tools, skills, policies, surfaces, and executors. Discovery needs a backing product and policy model that answers separate questions:

1. What capabilities exist?
2. Which concrete instances are available in the current Den/session/environment?
3. Which capabilities may this Bear use?
4. Which capabilities should travel with a Bear when it is added to another Den?

These questions must remain separate because Den is adding skills, MCP servers, armatures, local workspaces, Code Mode executors, and external connections quickly. A single global "installed capabilities" model would either leak authority between Bears or make Bear portability brittle.

The important product constraints are:

- Skills bundled with a Bear should **just work** when the Bear is added to Den.
- Bundled skills must not automatically affect other Bears.
- Connections are per Bear.
- Capability configuration, including enablement, is per Bear.
- Bear stances may override Bear capability configuration.
- Portable Bear exports must not include secrets, local session state, or live connection authority.

## Decision

Den will manage capabilities with four distinct layers:

1. **Capability definitions** — durable catalog records describing what a capability is.
2. **Capability instances** — concrete environment/session/provider bindings that are available now or in a configured Den.
3. **Bear capability configuration** — per-Bear enablement and policy for capability definitions, abstract capability requirements, or provider patterns.
4. **Stance capability overrides** — per-Bear, per-stance adjustments to the Bear's default capability configuration.

Den will also treat **Bear-bundled skills** as part of the Bear's portable definition. When a Bear that includes skills is added to Den, those skills become available to that Bear automatically according to the Bear's own capability and stance configuration. This does not install, enable, or grant those skills to any other Bear.

Connections are scoped to a Bear. A GitHub, Linear, Slack, filesystem, MCP server, or other integration connection configured for one Bear does not grant authority to another Bear. Other Bears may independently configure equivalent connections, but there is no implicit global sharing.

## Layer model

### Capability definitions

Capability definitions are durable catalog records. They describe what something is, not whether a particular Bear can currently use it.

Examples:

```yaml
ref: den.memory.search
kind: tool
provider: den
summary: Search Bear memory.
risk: read_only
portable: true
```

```yaml
ref: mcp.github.create_issue
kind: tool
provider: mcp.github
summary: Create a GitHub issue through a configured MCP server.
risk: external_write
portable: conditional
requires:
  - connection.provider: github
  - authority: github_account
```

```yaml
ref: skill.rust.code_review
kind: skill
summary: Review Rust code for correctness and maintainability.
risk: prompt_behavior
portable: true
```

Definitions are safe to browse, search, sync, and include in catalog indexes.

### Capability instances

Capability instances are concrete bindings of definitions into a current environment, session, provider, surface, or connection.

Examples:

```yaml
definition_ref: den.memory.search
instance_id: den.default
provider: den
execution_locality: den_runtime
authority: bear_runtime
lifetime: durable
```

```yaml
definition_ref: armature.fs.read_text_file
instance_id: acp-session-123:workspace
provider: armature
execution_locality: user_session_local
surface: acp.current_workspace
authority: local_user_adapter
lifetime: current_session
```

```yaml
definition_ref: mcp.github.create_issue
instance_id: builder:github:hans
provider: mcp.github
connection_ref: github:hans
authority: bear_scoped_connection
lifetime: while_connection_valid
```

Instances may be durable, Den-managed, connection-bound, workspace-bound, or session-bound. Portable Bear definitions should not include live instances except as requirements to resolve later.

### Bear capability configuration

Capability configuration is per Bear. It controls enablement, approval requirements, scopes, risk posture, and provider preferences.

Example:

```yaml
bear_id: builder
capabilities:
  capability.workspace.read:
    enabled: true
  capability.git.read:
    enabled: true
  capability.workspace.write:
    enabled: true
    approval_required: false
  capability.command.run:
    enabled: false
  mcp.github.create_issue:
    enabled: true
    approval_required: true
```

This configuration may refer to exact capability refs, abstract capability requirements, tags, provider patterns, or bundles. Exact storage and matching can evolve, but the policy owner is the Bear, not the global catalog.

### Stance capability overrides

Stance overrides are per Bear and per stance. They tune or restrict the Bear's default capability configuration for a runtime stance such as `chat`, `pair`, `curate`, `work`, or `watch`.

Example:

```yaml
bear_id: builder
stances:
  chat:
    capability.workspace.write:
      enabled: false
    mcp.github.create_issue:
      enabled: false
  pair:
    capability.command.run:
      enabled: true
      approval_required: true
  work:
    capability.command.run:
      enabled: true
      approval_required: false
```

Default safety rule: a stance override may restrict, require approval, narrow scope, or tune an enabled Bear capability. It must not silently grant authority the Bear does not have. If Den later supports stance-specific grants, they must be explicit policy records, not accidental inheritance behavior.

### Bear-bundled skills

Skills included in a Bear are part of that Bear's portable definition.

When the Bear is added to Den:

- bundled skills are imported or registered as needed;
- those skills are attached to and enabled for that Bear according to the Bear definition;
- Den may deduplicate identical skill content internally;
- no other Bear receives the skill automatically;
- no extra approval prompt is required merely because the skill exists.

Skills are prompt/procedure capabilities. If a skill requires external authority, tool use, credentials, or side effects, those risks are governed by the corresponding capability configuration, connection, and approval policy. The skill itself does not smuggle authority.

## Connections

Connections are per Bear authority bindings.

Examples include:

- GitHub or Linear OAuth/account connections;
- Slack, Zendesk, or other channel/service integrations;
- MCP server bindings;
- workspace or filesystem bindings;
- browser/CDP sessions;
- armature-provided local tool surfaces.

A connection record should identify at least:

- Bear id;
- provider;
- connection reference;
- authority source;
- enabled/disabled state;
- allowed scopes or surfaces;
- lifetime/refresh posture;
- which capability definitions or abstract requirements it can satisfy.

Connections may satisfy capability instances for one Bear only. Sharing the same underlying user account with another Bear requires another Bear-scoped connection record or an explicit admin/user action.

## Bear portability

A portable Bear export should include:

- Bear identity/configuration needed to recreate the Bear;
- bundled skills or exportable skill references/content hashes;
- Bear capability configuration;
- stance capability overrides;
- abstract capability requirements and recommendations;
- connection requirements by provider/purpose.

A portable Bear export must not include:

- OAuth tokens, API keys, or other secrets;
- user-local filesystem paths as durable authority;
- live MCP server process state;
- live armature or browser sessions;
- capability instances that only exist in one Den/session/environment.

On import or add-to-Den:

1. Den imports the Bear definition.
2. Den makes bundled skills available to that Bear.
3. Den restores Bear capability configuration and stance overrides.
4. Den reports missing required or recommended connection requirements.
5. Den resolves available instances from current Den tools, Bear connections, armatures, MCP servers, work surfaces, and installed skills.
6. Other Bears are untouched.

The product behavior should feel like adding an app with its own bundled behavior and requested permissions: the app's built-in features work, while external accounts and local/session-bound authorities must be connected or resolved in the new environment.

## Interaction with capability discovery

Capability discovery uses this model but does not replace it.

Search and describe results should distinguish:

- catalog definitions that exist;
- instances available to this Bear now;
- capabilities enabled for the Bear;
- capabilities restricted or disabled by the current stance;
- missing connections or unavailable session-local providers.

Discovery may say, for example:

```text
mcp.github.create_issue exists in the catalog, but Builder Bear has no enabled GitHub connection.
```

or:

```text
armature.fs.read_text_file is available for this pair session on the current ACP workspace; it is session-local and not portable.
```

The model-facing capability catalog should not imply authority from catalog presence alone.

## Consequences

### Positive

- Bear portability is practical without exporting secrets or local state.
- Skills bundled with a Bear work immediately for that Bear.
- One Bear's skills, connections, and capability settings do not leak to other Bears.
- Stances can be safer without cloning entire Bear configurations.
- MCP and armature/local tools fit the same model as Den-native tools without pretending they are durable global capabilities.
- Capability discovery can explain missing or disabled capabilities instead of hallucinating availability.

### Tradeoffs

- Den needs more configuration objects than a single global catalog table.
- Capability resolution must merge catalog definitions, Bear config, stance overrides, connections, and runtime inventory.
- UI needs to distinguish catalog browsing, Bear permissions/configuration, and current availability.
- Import UX must report missing connections clearly while still letting bundled skills work.

## Alternatives considered

### Global skill installation affects every Bear

Rejected. It creates surprising cross-Bear behavior and makes Bear imports externally visible to unrelated Bears.

### Prompt for every bundled skill on Bear import

Rejected. Bundled skills are part of the Bear's definition and should just work for that Bear. Risky side effects are governed by tool/connection/capability grants, not by skill text merely being present.

### Global connections shared by all Bears

Rejected. This leaks authority and makes it hard to reason about which Bear can act on which external account or surface.

### Capability config only by stance

Rejected. Stances are runtime postures for a Bear. The Bear still needs a default capability posture that travels with it and can be overridden by stance.

## Implementation notes

Near-term implementation should be boring:

1. Add data structures for Bear capability configuration and stance overrides.
2. Add Bear-scoped skill attachment/import semantics for bundled skills.
3. Add Bear-scoped connection records and connection requirements.
4. Teach capability discovery to report enabled/disabled/available/missing status for the current Bear and stance.
5. Keep dispatcher authorization as the enforcement point; catalog presence and discovery are not grants.

`ponytail:` The first implementation can store exact refs and simple wildcard/tag patterns before building a rich policy language. The ceiling is coarse matching for complex provider requirements. The upgrade path is explicit requirement resolution and policy predicates after MCP/skills/armature use cases harden.
