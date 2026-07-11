# ADR-0054: Capability Discovery and Code Mode

**Status:** Proposed  
**Date:** 2026-07-11  
**Deciders:** Hans

**Related:**

- [ADR-0016: Pair Tool Discovery and Scope Orientation](adr-0016-pair-tool-discovery-and-scope-orientation.md)
- [ADR-0025: Tool Naming and Execution Strategy](adr-0025-tool-naming-and-execution-strategy.md)
- [ADR-0028: Environment Affordance and Resource Boundaries](adr-0028-environment-affordance-and-resource-boundaries.md)
- [ADR-0039: Trust Profiles and Governance Modes](adr-0039-trust-profiles-and-governance-modes.md)
- [ADR-0050: Agent Loop Control, Adaptive Budgets, and Runtime Checkpoints](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md)
- [Capabilities and Skills](../architecture/capabilities-and-skills.md)

## Context

Den will continue to add tools, skills, integrations, execution surfaces, memories, policies, and domain-specific procedures. Listing every model-facing tool and schema in the live context does not scale: it consumes context window, teaches the model stale details, and pushes Den toward brittle task-type or stance-specific prompt bundles.

Several adjacent decisions already cover parts of the problem:

- ADR-0016 requires structured tool discovery and scope orientation instead of injecting runtime/tool scaffolding into user messages.
- ADR-0025 defines model-facing tool naming, descriptor ownership, and execution location strategy.
- ADR-0028 requires resource domains, capability domains, and workflow domains to be visually and structurally distinct at point of use.
- The capabilities-and-skills architecture note defines capabilities as product-level abilities that may map to tools, credentials, policies, prompt fragments, memory structures, or sandbox rights.

What remains undecided is the sustainable model-facing strategy for a large and growing catalog. In particular:

- Den should not require the runtime to guess up front whether a Bear is doing coding, research, ops, curation, browser work, or planning.
- Den should not solve context pressure by proactively selecting a large likely-tool bundle for every turn.
- Tools and skills should not have unrelated discovery mechanisms.
- Code Mode is promising for large MCP/tool catalogs, but should not become a separate conceptual world from skills, policies, and other Bear capabilities.

## Decision

Den will use **capability discovery** as the primary model-facing strategy for large tool and skill catalogs.

A capability is the model- and product-facing thing the Bear can use to get work done. Capability discovery may return a bundle containing:

- concrete tools;
- skills and procedures;
- policies and approval guidance;
- memories or other contextual references;
- surfaces such as workspaces, browsers, external systems, or conversations;
- execution options such as direct invocation, Code Mode, or delegated runs.

The runtime prompt should not enumerate the full catalog. Instead, it should teach the model that the catalog is discoverable and that it should search or browse capabilities when it needs external state, side effects, specialized procedures, project knowledge, or an execution substrate not already visible.

Den will prefer **taxonomy-enabled discovery with search and browsing** over capability profiles or broad proactive tool injection. Taxonomy is registry metadata, not the only navigation mechanism.

Den will also maintain a compact **recently discovered** working set in runtime context. This is a recency cache, not a capability profile. It exists so the model can reuse capabilities it has already discovered during the current run, turn, session, or job without re-searching or requiring the full catalog in prompt.

Finally, Den will treat **Code Mode as an execution option inside capability discovery**, not as a separate Bear mode. Discovery should consistently advise that Code Mode is available when appropriate, and should recommend it more aggressively for composition-heavy workflows.

## Terminology

Use **capabilities** as the model-facing and product-facing term.

Rationale:

- Den already uses capability language in UI and architecture.
- "Capability discovery" is legible to both humans and models.
- Capabilities naturally cover tools, skills, integrations, execution substrates, and permissions.
- "Resources" is too broad and sounds like raw material or storage from a human product perspective.

Implementation may still use generic internal names such as `entry`, `resource`, or `registry item` where the storage layer needs to hold heterogeneous records. Those should not become the primary model-facing or UI label.

Preferred product/model terms:

- Capability Catalog
- capability discovery
- capability result
- capability bundle
- recently discovered
- execution option
- Code Mode executor

Avoid using "resource" as the umbrella UI label for this feature.

## Model-facing contract

A stable prompt fragment should communicate the following invariant in concise language:

```text
You are not shown the full Capability Catalog.

Use capability discovery when you need to inspect or affect external state,
use a specialized procedure, access project memory, or choose an execution option.

Discovery may return capability bundles containing tools, skills, policies,
memories, examples, surfaces, and executors.

Prefer direct invocation for simple one-off actions. Prefer Code Mode when
a task requires loops, batching, filtering, aggregation, retries, joins, or
large intermediate state.
```

The model should not need to know whether the current Bear is doing "code work", "browser work", "ops work", or "curation work" before discovering capabilities. The task type should emerge from the model's local need and the capabilities it requests.

## Discovery shape

Den should expose a small stable discovery surface, conceptually:

```ts
capability.search({ query: string, filters?: CapabilityFilters })
capability.describe({ ref: CapabilityRef })
capability.invoke({ ref: CapabilityRef, args: unknown }) // for invokable entries, if direct invocation is used
```

Exact tool names and schemas may differ, but the model-facing surface should stay small and stable.

Discovery results should include enough information for safe selection without loading the whole schema corpus:

- stable reference;
- type or bundle kind;
- short summary;
- taxonomy tags;
- good-for / not-good-for hints;
- risk and side-effect posture;
- scope or target surface;
- related skills, policies, memories, tools, and executors;
- whether direct invocation, Code Mode, or delegation is recommended;
- how to request details or invoke the capability.

Detailed schemas and long examples should be loaded lazily through description or documentation search.

## Recently discovered working set

The runtime may inject a compact section such as:

```text
Recently discovered:
- workspace.read_file — read a text file from the mounted workspace
- workspace.search_files — search files by path or content
- skill.minimal-code-change — make the smallest safe workspace edit and run one focused check
- executor.code_mode — compose multiple capability calls with loops, parsing, aggregation, or retries
```

This section should be:

- short;
- scoped to the current run, turn, session, or job;
- replaced by recency and usefulness rather than grown indefinitely;
- clear about risk and scope for mutating capabilities;
- removed or compacted when it stops helping the current work.

It must not become a hidden capability profile that grants authority by implication. Authorization remains stance-, policy-, surface-, and approval-governed.

## Code Mode posture

Code Mode should be promoted as a normal execution option returned by capability discovery.

Discovery should recommend Code Mode when the likely workflow involves:

- more than a few related tool calls;
- loops or batching;
- filtering, parsing, aggregation, joins, or transformations;
- retries or conditional execution;
- large intermediate outputs where only a small computed result should return to the conversation;
- composing several capabilities into one operation.

Discovery should prefer direct invocation when:

- the task is a single simple read or action;
- the action is risky and should remain individually reviewed;
- the user expects transparent step-by-step interaction;
- no transformation or composition is needed.

Code Mode is not a separate global mode for the Bear. It is an executor capability. A Bear can discover direct tools, skills, policies, and Code Mode in the same capability result.

## Registry guidance

Capability registry entries and bundles should support both search and taxonomy browsing.

Useful metadata includes:

- name and stable ref;
- kind: tool, skill, policy, memory, surface, executor, connector, example, or bundle;
- taxonomy tags;
- summary;
- risk class;
- side-effect posture;
- stance applicability;
- scope or target surface requirements;
- invocation or loading method;
- examples and schema references;
- deprecation or replacement hints;
- related entries.

`ponytail:` The first implementation can be a simple registry plus lexical/tag search over curated summaries. The ceiling is poor recall for fuzzy intent and ambiguous cross-domain requests. The upgrade path is semantic indexing and better result ranking, not preloading the catalog into prompt.

## Consequences

### Positive

- Reduces context-window pressure from large tool catalogs.
- Gives tools and skills one model-facing discovery story.
- Avoids brittle up-front task-type or mode classification.
- Keeps Code Mode visible and recommended where it is actually useful.
- Lets taxonomy remain useful without making the model manually browse a giant tree.
- Gives the runtime a natural place to surface policy, risk, and scope before invocation.

### Tradeoffs

- Simple tasks may pay one extra discovery call when the needed capability is not already visible.
- Discovery result quality becomes important infrastructure.
- The model may need prompt reinforcement to discover instead of hallucinating unavailable tool names.
- The registry must avoid becoming another bloated prompt surface through overlong summaries.
- Code Mode needs a constrained SDK/runtime policy so it does not bypass normal capability governance.

## Alternatives considered

### Enumerate all tools and schemas in context

Rejected. This does not scale with catalog growth and wastes context on unused capabilities.

### Proactively select likely tools for each turn

Rejected as the primary strategy. It requires reliable up-front task classification, which is brittle because Bear work often changes domain mid-turn and may combine code, browser, memory, planning, and external systems.

### Capability profiles as the main mechanism

Rejected for now as over-engineered. Profiles may still exist as authorization or stance configuration, but they should not be the primary model-facing solution to catalog awareness.

### Taxonomy-only browsing

Rejected as insufficient by itself. Taxonomy is useful metadata, but the model should also be able to search by natural-language need.

### Code Mode as the sole strategy

Rejected. Code Mode is excellent for composition and dynamic tool use, but it does not inherently solve discovery of skills, policies, memories, or governance guidance. Den should expose Code Mode through capability discovery rather than making it a separate conceptual path.

## Implementation notes

Near-term work should be deliberately small:

1. Add a Capability Catalog discovery surface with search, describe, and invocation/loading semantics.
2. Include tools and skills in the same discovery corpus first; add memories, policies, surfaces, and executors as result types as schemas settle.
3. Add a compact "recently discovered" runtime context section.
4. Add Code Mode as an executor capability and advertise it in relevant discovery results.
5. Keep concrete direct tools available only when they are already part of the current runtime surface or have been discovered/loaded.
6. Update capability and tool descriptors to include short good-for/not-good-for, risk, scope, and execution-option hints.
7. Prefer deleting duplicated prompt tool lists as discovery becomes reliable.
