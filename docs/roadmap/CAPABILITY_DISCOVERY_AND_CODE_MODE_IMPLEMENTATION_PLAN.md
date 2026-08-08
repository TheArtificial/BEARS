# Capability Discovery and Code Mode Implementation Plan

**Status:** Proposed

**Decision source of truth:** [ADR-0054 — Capability Discovery and Code Mode](../decisions/adr-0054-capability-discovery-and-code-mode.md)

**Related plans:**

- [Bear capability configuration and portability](BEAR_CAPABILITY_CONFIGURATION_AND_PORTABILITY_PLAN.md)
- [Model experience audit and refresh](MODEL_EXPERIENCE_AUDIT_REFRESH_PLAN.md)
- [Prompt fragment registry](PROMPT_FRAGMENT_REGISTRY_IMPLEMENTATION_PLAN.md)
- [Skills implementation](SKILLS_IMPLEMENTATION_PLAN.md)
- [Pair tool discovery and scope policy](PAIR_TOOL_DISCOVERY_AND_SCOPE_POLICY.md)
- [Context window budget](CONTEXT_WINDOW_BUDGET_IMPLEMENTATION_PLAN.md)

## Plan boundaries

This plan owns the model-facing capability lifecycle: catalog taxonomy, search/describe, availability-aware discovery, lazy loading, bundles, governed invocation, and Code Mode. Bear/stance authority, connections, portability, and general MCP catalog attachments are owned by [Bear capability configuration and portability](BEAR_CAPABILITY_CONFIGURATION_AND_PORTABILITY_PLAN.md). Provider-specific bridges remain in their provider plans; Pair-specific scope and tool-call UX remain in [Pair tool discovery and scope policy](PAIR_TOOL_DISCOVERY_AND_SCOPE_POLICY.md).

## Documentation and model experience

Every phase that changes model-facing capability discovery, descriptors, invocation, or Code Mode must review and update [`MODEL_EXPERIENCE.md`](../../MODEL_EXPERIENCE.md). Create or update corresponding documentation in [`docs/guides`](../guides), covering the supported discovery and invocation workflow, user-visible limitations, and operator setup where applicable.

## Goal

Reduce model-context pollution from growing tool, skill, and execution catalogs by adding a small, stable capability discovery surface. Bears should learn how to discover and reuse relevant capabilities instead of receiving the full catalog in every runtime prompt.

The first implementation should be deliberately boring: taxonomy-enabled search and describe over curated catalog entries, a compact "recently discovered" runtime reminder, and explicit guidance that Code Mode is available as an execution option for composition-heavy workflows.

## Non-goals

- Do not introduce capability profiles as the primary model-facing solution.
- Do not require the runtime to classify each turn as code, browser, ops, curation, or planning work before exposing useful options.
- Do not replace existing descriptor, policy, BearWire, sandbox, or approval governance.
- Do not make Code Mode a global Bear mode or a bypass around capability governance.
- Do not build semantic ranking, dynamic tool grants, or a universal invocation router before lexical/tag discovery proves insufficient.

## Design constraints

- The model-facing word is **capability**. Avoid using "resources" as the umbrella UI/model label.
- Discovery returns **capability bundles** that may include tools, skills, policies, memories, surfaces, examples, and execution options.
- Taxonomy is catalog metadata, not the only navigation path.
- Discovery result text must be short enough that discovery does not become the new prompt bloat.
- "Recently discovered" is a recency cache, not an authority grant. Authorization remains stance-, surface-, policy-, and approval-governed.
- Discovery and recently-discovered text must distinguish durable Den-managed capabilities from session-local capability instances exposed by armatures, adapters, browser sessions, or work runners.
- Capability availability should account for per-Bear capability configuration, stance overrides, and Bear-scoped connections as defined by [ADR-0055](../decisions/adr-0055-bear-capability-configuration-connections-and-portability.md).
- Labels such as "local" are insufficient without locality, surface, authority, and lifetime metadata.
- Direct invocation remains preferred for simple one-off actions. Code Mode should be recommended for loops, batching, filtering, aggregation, joins, retries, or large intermediate state.

## Phases

### Phase 0 — Catalog inventory and taxonomy seed

1. Inventory current model-facing tools, skills, prompt guidance, policies, surfaces, and execution substrates that should eventually appear in capability discovery.
2. Define a small initial taxonomy:
   - `workspace.read`, `workspace.write`, `git.read`, `command.execute`,
   - `browser.inspect`, `browser.act`,
   - `web.read`,
   - `memory.read`, `memory.write`, `memory.review`,
   - `jobs.manage`, `work.dispatch`,
   - `skills.read`, `skills.propose`, `skills.review`,
   - `prompt.context`, `executor.code`, `executor.delegate`.
3. Add or identify descriptor fields needed for discovery summaries:
   - stable ref,
   - kind,
   - summary,
   - taxonomy tags,
   - provider/origin,
   - execution locality,
   - authority source,
   - availability/lifetime,
   - risk/side-effect posture,
   - stance applicability,
   - scope/surface requirements,
   - good-for / not-good-for hints,
   - related skills, policies, tools, and executors,
   - Code Mode compatibility for local/session-bound providers,
   - deprecation/replacement hints.
4. Mark the first-pass catalog as curated metadata, not generated prose.
5. Identify which existing tools are durable Den-managed capabilities versus session-local capability instances exposed by an armature, adapter, browser session, or work runner.

**Exit criteria:** there is a checked-in inventory or schema sketch that maps current tools and skills to initial capability entries, including locality/surface metadata for local and armature-provided tools, without changing runtime behavior.

### Phase 1 — Read-only capability search and describe

1. Add a Den-owned Capability Catalog service over the existing descriptor/skill metadata.
2. Add read-only model-facing operations conceptually equivalent to:
   - `capability_search(query, filters?)`,
   - `capability_describe(ref)`.
3. Start with lexical and taxonomy/tag search over curated summaries.
4. Return compact results with refs, summaries, risk posture, provider/origin, execution locality, authority source, availability/lifetime, scope, and execution hints.
5. Include tools and approved skills first; allow policies, memories, surfaces, and executors as result kinds once their metadata is ready.
6. Add tests for search filtering, unknown refs, disabled/deprecated entries, Bear capability configuration, stance overrides, missing Bear-scoped connections, and stance applicability.

**Exit criteria:** a Bear can discover and describe existing tools/skills through a small stable surface without the full catalog being projected into prompt, and results distinguish catalog presence from Bear+stance authority and current availability.

### Phase 2 — Prompt integration and recently discovered working set

1. Add a repository prompt fragment for the ADR-0054 model-facing contract:
   - the full catalog is not shown,
   - use discovery for external state, specialized procedures, project memory, and execution options,
   - direct invocation is preferred for one-off work,
   - Code Mode is preferred for composition-heavy workflows.
2. Project a compact `Recently discovered` section into runtime context after successful discovery calls.
3. Bound the section by recency and size; do not let it grow indefinitely.
4. Include risk/scope notes for mutating or externally visible capabilities.
5. Include locality, surface, authority, and lifetime notes for session-local capabilities.
6. Add context-budget attribution so capability discovery text can be measured and trimmed.
7. Begin deleting duplicated static tool-list guidance only after discovery quality is good enough for the affected stance.

**Exit criteria:** runtime prompts contain capability-discovery guidance plus a bounded recently-discovered working set, and context diagnostics can attribute their token cost.

### Phase 3 — Skills, policies, memories, and surfaces as bundle members

1. Extend discovery results so a capability can be a bundle rather than a single invokable tool.
2. Include approved and Bear-bundled skills from the Skills catalog with role applicability and required capability hints. Bear-bundled skills should just work for that Bear when imported/added, while remaining invisible to other Bears unless separately attached.
3. Include relevant policy references and approval guidance where a capability has side effects.
4. Include memory/search/cabinet capabilities as discoverable entries without treating memory contents themselves as tool schemas.
5. Include surfaces such as workspace, browser, conversation, work run, or external connector where scope matters.
6. Add UI affordances for browsing/searching the Capability Catalog by taxonomy and for seeing bundle contents.

**Exit criteria:** the same discovery story covers tools and skills, with policy/scope guidance visible before invocation.

### Phase 4 — Invocation/loading strategy

1. Keep direct native tool calls for simple actions where the tool is already part of the current roster.
2. For discovered but not-yet-visible capabilities, choose the smallest practical loading strategy supported by the current provider/runtime:
   - describe-only guidance when static tool rosters require preloading,
   - roster refresh at safe boundaries,
   - or a generic `capability_invoke(ref, args)` wrapper where policy and schema validation can remain Den-owned.
3. Resolve invocation against the current Bear's capability config, stance override, Bear-scoped connections, and runtime inventory. Catalog presence alone is not authority.
4. Make schema loading lazy: search results stay short; `describe` or docs lookup provides full invocation details only when needed.
5. Preserve existing approval gates for mutating, destructive, or externally visible actions.
6. Add audit events that distinguish discovery, description, loading, and invocation.

**Exit criteria:** discovered capabilities can be acted on through an explicit, governed path without preloading the full catalog into every turn.

### Phase 5 — Code Mode executor capability

1. Add Code Mode as an executor capability in the catalog, not as a global Bear mode.
2. Define the constrained SDK surface for invoking discovered capabilities from code.
3. Ensure Code Mode receives only the capabilities and scopes it is allowed to use for the current stance/session/run.
4. Declare each Code Mode executor's execution locality and provider compatibility. A Den-managed executor must not imply access to armature-local files, user-local commands, or adapter/browser session state unless those providers are explicitly mediated and authorized.
5. Have discovery recommend Code Mode when a likely workflow involves:
   - more than a few related calls,
   - loops or batching,
   - parsing/filtering/aggregation/joins/transforms,
   - retries or conditional execution,
   - large intermediate outputs.
6. Have discovery prefer direct invocation for one-off reads/actions and individually reviewed risky operations.
7. Add output-size controls so Code Mode returns compact computed results instead of dumping intermediate state into conversation context.

`ponytail:` The first Code Mode executor can be Den-managed only and limited to Den-owned or explicitly mediated capabilities. The ceiling is that Code Mode will not speed up some live armature-local workflows. The upgrade path is a governed armature-local executor or mediated batching API with explicit locality, authority, and data-movement rules.

**Exit criteria:** Bears can see Code Mode as an execution option during capability discovery, and the runtime can execute constrained Code Mode workflows without bypassing normal governance.

### Phase 6 — Cleanup and optimization

1. Remove obsolete prompt fragments and runtime text that enumerate large static catalogs.
2. Add semantic indexing only if lexical/tag search is demonstrably insufficient.
3. Improve ranking using observed successful discovery selections and explicit descriptor metadata, not hidden prompt profiles.
4. Add operator diagnostics for missing metadata, stale summaries, duplicate entries, and bloated discovery results.
5. Add migration notes for archived plans that used tool-list or skills-only discovery assumptions.

**Exit criteria:** the Capability Catalog is the default way to make Bears aware of large catalogs, with Code Mode available for composed execution and static prompt catalog lists retired where practical.

## Minimal first slice

The smallest shippable slice is:

1. a static in-repo catalog generated or curated from existing descriptors for a handful of high-traffic domains;
2. `capability_search` and `capability_describe` as read-only Den tools;
3. a prompt fragment telling the Bear to use discovery instead of assuming the full catalog is visible;
4. a bounded "recently discovered" context section;
5. Code Mode represented as a catalog entry with guidance, even if executable Code Mode lands later.

Bear capability configuration, stance overrides, Bear-scoped connections, and bundled skill import are tracked in the companion [Bear capability configuration and portability plan](BEAR_CAPABILITY_CONFIGURATION_AND_PORTABILITY_PLAN.md). The discovery first slice should avoid schema choices that would prevent those status fields from being merged later.

`ponytail:` This first slice can use lexical/tag search over curated YAML or Rust metadata. The ceiling is mediocre recall for fuzzy, cross-domain requests. The upgrade path is semantic indexing and better ranking after the catalog shape is validated, not preloading more tools into prompt.

## Verification strategy

- Unit tests for catalog metadata validation and search filters.
- Snapshot tests for prompt fragments and recently-discovered context projection.
- Integration tests showing a model-visible search/describe path for at least workspace, memory, skills, and executor entries.
- Policy tests proving discovery does not grant invocation authority.
- Locality/surface tests proving similarly named Den-managed, armature-local, and work-run capabilities remain distinguishable in search, describe, and recently-discovered output.
- Context-budget checks comparing full static tool projection against capability-discovery projection.
- Code Mode tests, once implemented, proving constrained SDK access and compact result return.
- Code Mode locality tests proving a Den-managed executor cannot silently access session-local armature, adapter, or browser state.

## Open questions

- Should `capability_invoke` exist, or should discovery only load native provider tools at safe boundaries?
- Where should catalog metadata live long term: descriptor registry, prompt fragment registry, Skills catalog, or a dedicated Capability Catalog table?
- How much of the Capability Catalog should be user-visible in UI versus only model/runtime-visible?
- What is the minimum safe SDK surface for Code Mode?
- Should Den ever provide an armature-local Code Mode executor, or should local batching be mediated through explicit adapter APIs?
- What concise locality/authority vocabulary is most legible to models and users?
- Should "recently discovered" be scoped to a run, turn, session, job, or a configurable combination?
