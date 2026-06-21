# Den model registry and Bifrost configuration plan

> **Direction changed (2026-06).** Bifrost-as-inference-substrate stands, but Den now calls Bifrost directly — Letta is not in the inference path or a runtime consumer. Treat "Letta = persistence/agent runtime" references as historical. Canonical target: [Den-Native Runtime](../architecture/den-native-runtime.md) ([migration plan](DEN_NATIVE_RUNTIME_PLAN.md)).

For the canonical role model and current role names, see [bear roles](../architecture/bear-roles.md).
Status: proposed implementation plan.

This document describes a target architecture in which **Bifrost** owns live model availability, provider keys, routing, pricing, and Model Catalog facts, while **Den** maintains a model metadata overlay for validation, display, context-window estimates, profile/task suitability, and operational reconciliation. It also describes the current repository state, recommended metadata sources, and a migration path from the current Bifrost-first metadata projection.

Related docs:

- [`../architecture/DEN_ARCHITECTURE.md`](../architecture/DEN_ARCHITECTURE.md)
- [`../deployment/DEPLOYMENT.md`](../deployment/DEPLOYMENT.md)
- [`PLAN.md`](PLAN.md)
- [`PHASE1_DECISIONS.md`](PHASE1_DECISIONS.md)
- [`../../services/bifrost/COOLIFY_DEPLOY.md`](../../services/bifrost/COOLIFY_DEPLOY.md)
- [`../../services/bifrost/config.json`](../../services/bifrost/config.json)
- [`../../services/den/src/core/bifrost.rs`](../../services/den/src/core/bifrost.rs)

---

## Goal

Give **Den** enough model metadata to validate Bear configuration and plan runtime context, while leaving **Bifrost** as the owner of live model availability, provider keys, and routing.

The desired steady state is:

```text
Bifrost available models/provider keys -> Den metadata reconciliation -> Bear/runtime validation and context budgeting
```

More specifically:

1. **Bifrost** owns which models are callable in this deployment.
2. **Bifrost** owns provider keys, allowlists, aliases used for execution, routing, failover, and gateway governance.
3. **Den** owns curated overlay metadata such as display corrections, conservative context/max-output overrides, profile/task suitability, and product labels.
4. **Den** validates Bear configuration by reconciling configured model handles against Bifrost availability plus Den metadata.
5. **Den** may report or plan sync differences, but it should not assume it is the canonical source for Bifrost provider configuration.

---

## Problem statement

The repository already contains a useful bootstrap of this idea, but ownership is currently partially inverted.

### Current state

- `services/bifrost/config.json` contains a BEARS-specific `bears.models` metadata section.
- `services/den/src/core/bifrost.rs` reads Bifrost model metadata and converts it into Den/Letta-facing model options.
- The existing metadata already includes fields such as:
  - `handle`
  - `provider`
  - `model`
  - `display_name`
  - `context_window`
  - `max_output_tokens`
  - `supports_tools`
  - `supports_responses_api`
  - `supports_vision`
  - `enabled`

That is a good bootstrap, but the long-term boundary should be clearer:

- **Bifrost decides what models are currently available and how they route to providers.**
- **Den tracks what it knows about models and uses that metadata for validation, display, and runtime planning.**

### Why this matters

If Den relies only on Bifrost's live model list, then:

- Den lacks stable context-window and max-output estimates for planning.
- Bear Admin cannot explain model tradeoffs consistently.
- configured Bear models cannot be validated with useful diagnostics.
- token budgeting and compaction policy must guess from strings instead of metadata.
- Bifrost availability drift is hard to spot from Den's operator surfaces.

---

## Key architectural decision

Treat **model capability metadata** as Den-owned validation/planning state, and treat **Bifrost availability/configuration** as the execution-plane source of truth.

### Ownership split

#### Den owns

- curated model metadata overlays (display names/notes, conservative context-window and max-output overrides)
- Den-side aliases for validation and canonicalization
- profile/task suitability hints used by Bear Admin and runtime planning
- a simple `selectable` flag for Den UI exposure when needed
- provenance/confidence where metadata is manually curated or inferred
- reconciliation reports comparing Den metadata with Bifrost availability
- future token-budgeting logic

#### Bifrost owns

- provider credentials and secrets
- provider endpoint wiring
- runtime request execution
- provider key model allowlists and gateway aliases
- provider routing / failover / weighting where used
- Bifrost Model Catalog data (availability, provider mapping, pricing, provider-reported capabilities)
- live model availability for this deployment
- OpenAI-compatible execution surface

This preserves the repo's broader architecture:

- **Den** = control plane
- **Letta** = persistence and agent runtime layer
- **Bifrost** = model execution plane

---

## Canonical naming

Den metadata keys should be provider-qualified for clarity, but Bifrost remains the authority on which handles are actually routable.

Recommended format:

```text
{provider}/{provider_model_id}
```

Examples:

- `openai/gpt-4.1`
- `openai/gpt-4.1-mini`

This is preferred over bare handles such as:

- `gpt-4.1`
- `gpt-4.1-mini`

### Why provider-qualified keys

1. Avoid collisions across providers.
2. Make registry entries globally unambiguous.
3. Let aliases remain presentation or convenience features instead of identity.
4. Give Den a stable namespace for policy, defaults, and migration.

### Alias policy

Den should maintain aliases separately from canonical identity.

For example, `openai/gpt-4.1` may resolve from aliases like:

- `gpt-4.1`
- `openai-gpt-4.1`

Resolution guidance:

1. exact canonical key match wins
2. unique alias match resolves
3. ambiguous alias match fails loudly
4. final persisted Bear configuration should prefer the Bifrost-routable/canonical handle after validation

---

## Why context window must be first-class

**Context window** should be a first-class field in Den, not just display decoration.

Den needs metadata estimates for:

1. **Model picker clarity**
   - users and operators need to understand long-context suitability
2. **Token budgeting**
   - Den should be able to estimate prompt fit before execution
3. **Validation against Bifrost availability**
   - Den can warn when a configured Bear model is unavailable or lacks metadata
4. **Planning and policy**
   - higher-level logic can intentionally choose between cheaper/smaller and larger-context models
5. **User experience**
   - early warnings are better than late runtime failures

The same control-plane logic applies to:

- `max_output_tokens`
- tool support
- responses API support
- vision support
- future structured-output or reasoning flags

---

## Proposed conceptual data model

The final implementation can vary, but the architecture benefits from three conceptual objects.

### `DenModelRegistryEntry`

A canonical registry entry for one logical model.

Illustrative shape:

```ts
type DenModelRegistryEntry = {
  key: string;                     // "openai/gpt-4.1"
  provider: string;                // "openai"
  provider_model_id: string;       // "gpt-4.1"
  display_name: string;            // "OpenAI GPT-4.1"

  aliases: string[];

  capabilities: {
    context_window: number | null;
    max_output_tokens: number | null;
    supports_tools: boolean | null;
    supports_responses_api: boolean | null;
    supports_vision: boolean | null;
    supports_json_mode: boolean | null;
    supports_reasoning_controls: boolean | null;
    supports_streaming: boolean | null;
  };

  status: {
    selectable: boolean;              // Den UI exposure; Bifrost owns live availability
  };

  provenance: {
    sources: Array<{
      source_type: "provider_docs" | "provider_api" | "litellm" | "manual";
      source_ref: string;
      observed_at: string;
      fields: string[];
      confidence: "low" | "medium" | "high";
    }>;
  };
};
```

This is Den's metadata answer to:

- what model is this?
- what aliases resolve to it?
- what capabilities does it have?
- how certain are we?
- should it be exposed?

### `DenResolvedModelAwareness`

A Den-side awareness object derived from Bifrost availability plus Den metadata.

Illustrative shape:

```ts
type DenResolvedModelAwareness = {
  registry_key: string;            // "openai/gpt-4.1"
  available_in_bifrost: boolean;
  provider: "openai";
  provider_model_id: "gpt-4.1";

  capabilities: {
    context_window: number | null;
    max_output_tokens: number | null;
    supports_tools: boolean | null;
    supports_responses_api: boolean | null;
    supports_vision: boolean | null;
  };

  bifrost: {
    handle: string;                // eventually "openai/gpt-4.1"
    provider_key: string;
    model: string;
  };
};
```

This is the bridge between Den’s semantic registry and Bifrost’s runtime format.

### `BifrostExecutionRequest`

A runtime request shape used when execution is routed through Bifrost.

Illustrative shape:

```ts
type BifrostExecutionRequest = {
  model: string;
  messages: unknown[];
  tools?: unknown[];
  stream?: boolean;
  max_tokens?: number;
  temperature?: number;
  response_format?: unknown;
};
```

Bifrost does not need to understand Den’s full registry schema. It only needs a resolved runtime handle and provider mapping.

---

## Source-of-truth and data sourcing strategy

The registry needs a practical sourcing hierarchy.

### Recommended precedence

Use this order of trust where possible:

1. **provider API** when it exposes authoritative capability data
2. **provider documentation** as the main human-auditable source
3. **LiteLLM** as a broad bootstrap and gap-filler
4. **manual Den curation** for incomplete, ambiguous, or contradictory cases

### LiteLLM role

LiteLLM is useful because it:

- aggregates many providers
- normalizes many model names
- often includes context-window-style metadata
- is practical for bootstrap seeding

Recommended usage:

- use LiteLLM to seed candidate entries
- keep provenance showing fields originated from LiteLLM
- treat LiteLLM-derived values as lower confidence than provider-native confirmation unless independently verified

### Provider docs role

Provider docs are often the best auditable source for:

- context window
- max output
- multimodal support
- tool/function-calling support
- deprecation state

### Manual override role

Some providers publish incomplete or inconsistent information. Den therefore needs explicit manual curation with provenance such as:

- source type `manual`
- operator rationale
- timestamp
- fields overridden

Guidance: prefer explicit unknowns (`null`) over invented certainty.

---

## Den ↔ Bifrost reconciliation

### Core principle

Bifrost configuration is **not** a materialized view of Den. Bifrost owns live provider configuration and availability; Den compares that surface with its metadata registry so operators can validate Bear configuration and spot gaps.

Operational flow:

1. Bifrost exposes currently available/routable models through its metadata or management APIs.
2. Den keeps model metadata estimates and aliases for validation/planning.
3. Den compares Bifrost availability with Den metadata.
4. Den surfaces:
   - available Bifrost models with known Den metadata
   - available Bifrost models missing Den metadata
   - Den metadata entries not currently available in Bifrost
   - configured Bear models that are unavailable or metadata-unknown
5. Optional future tooling may either patch `services/bifrost/config.json` or call Bifrost management APIs, but that is an executor choice rather than the ownership boundary.

### Current repository fit

Today `services/bifrost/config.json` contains both:

- provider execution config
- a BEARS-specific `bears.models` metadata block

In the revised design, Bifrost's provider config and live metadata remain availability signals. Den may keep a local metadata registry and compare against Bifrost; it should not require Bifrost config to be generated from Den for normal operation.

### What Den should report

Den should use Bifrost Model Catalog data before manual Den curation wherever possible. Den should report, conceptually:

1. **availability-facing comparison**
   - Bifrost model handles seen live
   - Den metadata keys that match those handles/aliases
   - Bifrost handles with no Den metadata
   - Den metadata entries not currently Bifrost-available
2. **configuration validation**
   - Bear defaults using unavailable models
   - Bear defaults using aliases that resolve to canonical/routable handles
   - Bear defaults with unknown context-window estimates

This may later drive a file patcher or a Bifrost management API sync, but status/reporting comes first.

---

## Current repository baseline

The existing repository already supports a first bootstrap of the target design.

### Den-side metadata consumer

`services/den/src/core/bifrost.rs` currently defines a `BifrostModelMetadata` struct with fields including:

- `handle`
- `provider`
- `model`
- `display_name`
- `context_window`
- `max_output_tokens`
- `enabled`
- `supports_tools`
- `supports_responses_api`
- `supports_vision`

It also converts that metadata into Den/Letta-facing model options for presentation.

### Bifrost-side metadata producer

`services/bifrost/config.json` already stores BEARS model metadata for entries such as:

- `gpt-4o-mini`
- `gpt-4o`
- `gpt-4.1-mini`
- `gpt-4.1`

That config is useful as a seed source for the Den registry, but it should not remain the long-term semantic owner.

---

## Migration plan

Use a staged migration rather than a flag day.

### Phase 0: accept current bootstrap

Keep the current flow working:

- Bifrost exposes metadata
- Den reads it
- Den uses it for model-selection UX

This is an acceptable temporary state.

### Phase 1: add a Den-owned registry

Introduce a Den-side registry table or config source with:

- canonical key
- aliases
- capabilities
- simple `selectable` state for Den UI exposure
- provenance and confidence

Seed it from:

- `services/bifrost/config.json`
- LiteLLM-derived metadata where helpful
- provider docs and manual review

During this phase, Den may still compare against Bifrost metadata for compatibility checks.

### Phase 2: adopt canonical handles

Move from Bifrost handles like:

- `gpt-4.1`

To canonical handles like:

- `openai/gpt-4.1`

Maintain temporary alias compatibility for older references.

### Phase 3: reconcile with Bifrost availability

Compare Den metadata against live Bifrost availability and surface:

- Bifrost-available models with Den metadata
- Bifrost-available models missing Den metadata
- Den metadata entries not currently available in Bifrost
- configured Bear defaults that fail availability or metadata checks

Possible operational forms:

- `/status` model registry panel
- CLI/API dry-run sync plan
- optional future Bifrost API sync or config patcher

### Phase 4: resolve through `DenResolvedModelAwareness`

Introduce a Den resolver that transforms:

- requested canonical key or alias
- bear policy or defaults
- feature requirements

Into:

- `DenResolvedModelAwareness`

Use that spec as the source for:

- UI display
- provisioning choices
- execution references
- Bifrost availability validation
- future token-budgeting logic

### Phase 5: optional sync executor

If operators want Den-assisted Bifrost updates, add an explicit executor that can either:

- produce a patch for `services/bifrost/config.json`, or
- call Bifrost management APIs when Bifrost runs with `config_store` enabled.

This executor should be opt-in and auditable; Bifrost remains the owner of provider keys and live availability.

---

## Operational guidance

### Keep provenance with the field values

Capability data should always be traceable to a source, such as:

- provider docs URL
- provider API reference
- LiteLLM source ref
- manual operator note

### Prefer `null` over guesses

If a field is not known, store `null` instead of inventing a value.

Examples:

- `max_output_tokens: null`
- `supports_reasoning_controls: null`

### Validate metadata and availability before accepting Bear config

Before Den accepts or updates Bear model configuration, validate:

- the requested handle resolves uniquely through Den aliases or Bifrost availability
- the resolved model appears available in Bifrost when Bifrost is reachable
- Den has enough metadata for display and budgeting, or can warn that metadata is unknown
- no ambiguous aliases are accepted

### Surface reconciliation differences

Den should compare metadata expectations with observed Bifrost availability and surface differences in logs or operator views.

### Treat display labels as presentation only

`display_name` is for UI. Identity should be based on:

- canonical key
- provider model id
- resolved execution mapping

---

## Relationship to Letta and BEARS runtime

Letta should not become the canonical owner of model capability metadata for BEARS.

Instead:

- Den owns metadata and validation logic
- runtime components consume a Bifrost-available model choice plus Den metadata where known
- Bifrost remains the execution gateway and availability source under the runtime path

This keeps the control-plane / runtime / execution split consistent with the rest of the repository architecture.

---

## Open questions

1. Should Den store canonical provider-qualified handles or Bifrost-routable handles on Bears?
   - current recommendation: store the validated Bifrost-routable/canonical handle when available.
2. Should pricing metadata live in the same registry?
   - probably later, not required for the first milestone.
3. Should policy be attachable globally, per Bear, and per profile?
   - likely yes over time.
4. Should LiteLLM ingestion be scheduled automatically?
   - useful later, not required for the first implementation.
5. Should provider APIs be polled for availability/metadata drift where available?
   - valuable, but not necessary for the first cut.
6. Should Den ever push changes to Bifrost?
   - only through an explicit audited sync executor; default remains compare/report.

---

## Recommended immediate next steps

1. define a Den-side metadata registry shape
2. seed metadata from `services/bifrost/config.json`, provider docs, and manual curation
3. normalize aliases to provider-qualified Den metadata keys such as `openai/gpt-4.1`
4. validate Bear model choices against Bifrost availability
5. surface Bifrost/Den metadata reconciliation in status/admin views
6. introduce a resolver that emits `DenResolvedModelAwareness`
7. later, use the same metadata for token budgeting, defaults, and runtime diagnostics

---

## Summary

The recommended architecture is:

- **Den owns model metadata for validation and planning**
  - context window estimates
  - max output estimates
  - capability flags
  - aliases
  - provenance
  - confidence
  - simple UI selectability
- **Bifrost owns availability and execution**
  - provider auth
  - provider routing
  - provider key model allowlists and aliases
  - request execution
- **Den reconciles with Bifrost**
  - compares Den metadata with Bifrost-available models
  - validates configured Bear defaults
  - may optionally plan or execute audited sync later
- **LiteLLM is a bootstrap source**
  - useful for broad initial coverage
  - not the final authority
- **provider docs and provider APIs are authoritative where possible**
  - with manual Den curation when necessary

This keeps the control-plane / execution-plane boundary clean and sets up BEARS for better model selection, token budgeting, auditability, and future runtime flexibility.
