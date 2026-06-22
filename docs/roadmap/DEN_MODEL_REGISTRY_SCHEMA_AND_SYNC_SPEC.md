# Den Model Registry Schema and Sync Spec

> **Direction changed (2026-06).** Drop Letta-facing model option types (`LettaModelOption`); the Den registry feeds Bifrost only, called directly by the Den-native runtime. Canonical target: [Den-Native Runtime](../architecture/den-native-runtime.md) ([migration plan](DEN_NATIVE_RUNTIME_PLAN.md)).

For the canonical role model and current role names, see [bear roles](../architecture/bear-roles.md).
Status: proposed implementation spec.

## Objective

Define the first concrete implementation contract for a Den-owned model metadata overlay used for validation, display, context-window estimates, profile/task suitability, and reconciliation with Bifrost. Bifrost's Model Catalog remains the preferred source for live model availability, provider/model mapping, pricing, and provider-reported capability metadata.

This document is narrower and more concrete than `DEN_MODEL_REGISTRY_AND_BIFROST_CONFIG_PLAN.md`. That planning document explains the project shape and migration strategy. This spec defines the metadata model, source hierarchy, validation behavior, and Den↔Bifrost reconciliation boundary.

## Related docs

- `docs/planning/DEN_MODEL_REGISTRY_AND_BIFROST_CONFIG_PLAN.md`
- `docs/architecture/DEN_ARCHITECTURE.md`
- `services/den/src/core/bifrost.rs`
- `services/bifrost/config.json`
- `services/bifrost/COOLIFY_DEPLOY.md`

---

## Current repo-grounded baseline

Today, effective model metadata is stored in `services/bifrost/config.json` under the custom `bears.models` section.

That metadata currently includes fields such as:
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

Den currently consumes that metadata through `services/den/src/core/bifrost.rs`, which:
- fetches a JSON payload from `BIFROST_METADATA_URL`
- deserializes `models: Vec<BifrostModelMetadata>`
- filters to enabled models
- sorts models for presentation
- converts each record into a Letta-facing `LettaModelOption`

So the current architecture is effectively:
1. Bifrost config is the source of truth.
2. Bifrost exposes model metadata.
3. Den reads Bifrost’s metadata projection.
4. Den presents a simplified model list to clients.

The desired architecture in this spec clarifies that ownership:
1. Bifrost owns live availability, provider keys, provider allowlists, execution aliases, routing, and Model Catalog facts.
2. Den owns curated overlays and Bear-specific validation context.
3. Den hydrates/enriches model options from Bifrost Model Catalog surfaces wherever possible.
4. Den reports unknown metadata or availability mismatches, but does not maintain a duplicate authoritative model catalog.

---

## Design goals

1. Give Den a stable metadata overlay for model identity, capability corrections, profile/task suitability, and validation.
2. Use Bifrost Model Catalog as the primary source for availability, pricing, provider mappings, and provider-reported capabilities.
3. Preserve enough provenance to distinguish Bifrost-observed, provider-documented, inferred, and manually curated values.
4. Support multiple naming layers:
   - canonical provider-qualified key
   - provider-native model id
   - human-friendly display label
   - local aliases
   - legacy handles
5. Allow Den to enrich Bifrost-available model options with stable metadata and warnings.
6. Allow reconciliation reports, and optionally future sync/export tooling, to remain deterministic and auditable.
7. Support future providers beyond OpenAI without changing the schema shape.
8. Keep the first implementation simple enough to ship incrementally.

---

## Canonical identity model

### Canonical key

Each model registry entry should have a canonical provider-qualified key:

- `openai/gpt-4.1`
- `openai/gpt-4.1-mini`
- `openai/gpt-4o`
- `openai/gpt-4o-mini`

This key is the stable Den-side metadata identifier for the conceptual model entry. Bifrost may expose the same model through provider-native or gateway aliases.

### Identity layers

Each model can have several distinct identifiers:

- `key`: canonical Den identifier, provider-qualified
- `provider`: logical provider namespace such as `openai`
- `provider_model_id`: upstream provider’s model id used for execution
- `gateway_handle`: optional Bifrost-facing handle when different from the provider model id
- `display_name`: user-facing label
- `aliases`: alternative Den-resolvable names
- `legacy_handles`: historical names preserved for migration compatibility

These layers should not be conflated.

In the current repo state, `handle` and `model` are usually the same string. This spec treats that as an implementation convenience, not a permanent schema assumption.

---

## Core schema

## `DenModelRegistryEntry`

This is the Den metadata registry record. It is canonical for Den's metadata estimates, not for live provider availability.

```json
{
  "key": "openai/gpt-4.1",
  "provider": "openai",
  "provider_model_id": "gpt-4.1",
  "gateway": {
    "bifrost": {
      "handle": "gpt-4.1",
      "enabled": true
    }
  },
  "display_name": "OpenAI GPT-4.1",
  "family": "gpt-4.1",
  "release_channel": "general",
  "aliases": ["gpt-4.1", "openai:gpt-4.1"],
  "legacy_handles": [],
  "capabilities": {
    "context_window": {
      "value": 1047576,
      "provenance": "provider_docs",
      "confidence": "high"
    },
    "max_output_tokens": {
      "value": 32768,
      "provenance": "provider_docs",
      "confidence": "high"
    },
    "supports_tools": {
      "value": true,
      "provenance": "manual_curated",
      "confidence": "medium"
    },
    "supports_responses_api": {
      "value": true,
      "provenance": "manual_curated",
      "confidence": "medium"
    },
    "supports_vision": {
      "value": true,
      "provenance": "manual_curated",
      "confidence": "medium"
    }
  },
  "status": {
    "selectable": true
  },
  "sources": [
    {
      "kind": "provider_docs",
      "ref": "https://platform.openai.com/docs/models",
      "observed_at": "2026-05-19T00:00:00Z"
    },
    {
      "kind": "manual_curated",
      "ref": "repo bootstrap from services/bifrost/config.json",
      "observed_at": "2026-05-19T00:00:00Z"
    }
  ],
  "notes": null
}
```

### Required top-level fields

- `key: string`
- `provider: string`
- `provider_model_id: string`
- `display_name: string`
- `capabilities: object`
- `status.selectable: boolean`

### Recommended optional fields

- `gateway.bifrost.handle: string` (observed/reconciled, not Den-owned authority)
- `family: string`
- `release_channel: string`
- `aliases: string[]`
- `legacy_handles: string[]`
- `sources: SourceAttribution[]`
- `notes: string | null`

---

## Capability value envelope

Capability values may be stored as direct scalars in the first implementation. When sourced from provider docs or observed APIs, Den may later wrap tracked capabilities in a small provenance envelope.

## `CapabilityValue<T>`

```json
{
  "value": 32768,
  "provenance": "provider_docs",
  "confidence": "high",
  "observed_at": "2026-05-19T00:00:00Z",
  "source_ref": "https://platform.openai.com/docs/models"
}
```

### Fields

- `value`: typed capability value
- `provenance`: one of
  - `provider_api`
  - `provider_docs`
  - `litellm_bootstrap`
  - `manual_curated`
  - `inferred`
- `confidence`: one of
  - `high`
  - `medium`
  - `low`
- `observed_at`: optional timestamp
- `source_ref`: optional string reference

### Why this exists

This envelope allows Den to:
- merge multiple sources without losing origin
- expose confidence to operators
- prefer documented or directly observed values over inherited guesses
- re-run sync and identify stale values later

For the first implementation, not every field must be populated. But the shape should exist from the beginning so migration does not require schema churn.

---

## Source attribution model

## `SourceAttribution`

```json
{
  "kind": "provider_docs",
  "ref": "https://platform.openai.com/docs/models",
  "observed_at": "2026-05-19T00:00:00Z",
  "details": "context window and output token limits recorded manually"
}
```

### Fields

- `kind`
- `ref`
- `observed_at`
- `details` optional

This should exist both:
- at the entry level for broad lineage
- optionally inside individual capability envelopes for precise field-level attribution

---

## Resolved execution shape

The registry entry is canonical storage. Execution requires a resolved runtime shape.

## `DenResolvedExecutionSpec`

This is the Den-side runtime object produced after alias resolution, policy filtering, and gateway selection.

```json
{
  "requested_name": "gpt-4.1",
  "resolved_key": "openai/gpt-4.1",
  "provider": "openai",
  "provider_model_id": "gpt-4.1",
  "display_name": "OpenAI GPT-4.1",
  "gateway_target": {
    "kind": "bifrost",
    "handle": "gpt-4.1"
  },
  "capabilities": {
    "context_window": 1047576,
    "max_output_tokens": 32768,
    "supports_tools": true,
    "supports_responses_api": true,
    "supports_vision": true
  },
  "selection_metadata": {
    "resolved_via": "alias",
    "confidence": "high"
  }
}
```

### Notes

This object intentionally flattens capability envelopes into executable values.

Execution code should not need to reason about provenance on the hot path. Provenance belongs in the canonical registry and operator/debug surfaces.

### Required fields

- `requested_name`
- `resolved_key`
- `provider`
- `provider_model_id`
- `gateway_target`
- `capabilities`

---

## Bifrost availability shape

Den should not persist raw Bifrost config as its model metadata state. Instead, it should observe Bifrost availability and compare it with Den metadata.

## `BifrostExecutionRequest`

This is the minimal runtime request Den effectively needs in order to call Bifrost.

```json
{
  "model": "gpt-4.1",
  "provider": "openai",
  "canonical_key": "openai/gpt-4.1"
}
```

In practice, chat/completions payloads will include many additional fields, but for model-resolution purposes the important point is that Bifrost receives a Bifrost-visible model handle, not the full canonical Den registry entry.

## `BifrostAvailabilitySnapshot`

This is the observed Bifrost model surface Den compares against its metadata registry.

```json
{
  "handle": "openai/gpt-4.1",
  "provider": "openai",
  "model": "gpt-4.1",
  "display_name": "OpenAI GPT-4.1"
}
```

The exact shape can come from Bifrost `/bears/models`, `/v1/models`, or management APIs. Den should treat it as availability evidence, not as authoritative capability metadata.

---

## Runtime catalog snapshot (caching contract)

The reconciliation pipeline's runtime index (see [Reconciliation pipeline](#reconciliation-pipeline)) should be a concrete, shared, refreshable in-memory object rather than an ad-hoc per-request fetch.

Implemented baseline (2026-06): `services/den/crates/den-service/src/bifrost.rs` defines `BifrostCatalogSnapshot` / `BifrostCatalogEntry` and `BifrostCatalogStore = Arc<RwLock<BifrostCatalogSnapshot>>`. API and web state own snapshot stores and warm them best-effort at startup. BearWire run/session paths and web model UI/status paths now read the snapshot instead of fetching `/v1/models` per request. The former `den-llm::model_registry` process-global routing cache has been removed.

## `BifrostCatalogSnapshot`

```json
{
  "fetched_at": "2026-06-22T00:00:00Z",
  "source": "v1_models",
  "stale": false,
  "models": {
    "openai/gpt-4.1": {
      "available": true,
      "provider": "openai",
      "provider_model_id": "gpt-4.1",
      "gateway_handle": "gpt-4.1",
      "display_name": "OpenAI GPT-4.1",
      "context_window": 1047576,
      "max_output_tokens": 32768,
      "supports_tools": true,
      "supports_responses_api": true,
      "supports_vision": true
    }
  }
}
```

### Contract

- Keyed by canonical Den key (`resolve_model_handle` applied to each Bifrost handle); the raw `gateway_handle` is retained for execution.
- Held on shared edge application state (`DenState.bifrost_catalog` / `AppState.bifrost_catalog`) as `Arc<RwLock<BifrostCatalogSnapshot>>`, not refetched per request on the BearWire/web hot paths.
- Startup primes it once (best-effort). Periodic TTL refresh and explicit last-good error metadata remain to be implemented. On warm-up failure the initialized empty snapshot remains `stale`.
- Live capability flags (`supports_responses_api`, `supports_tools`, `supports_vision`, token limits) are read from this snapshot where the snapshot is available. The registry supplies these only as a documented fallback when a model is absent from the snapshot, at lower confidence.

### Capability resolution precedence (runtime hot path)

For a capability needed during request handling (e.g. API-style routing via `preferred_api_style_for_model`):

1. snapshot value for the resolved key, when the snapshot is fresh
2. snapshot value for the resolved key, when `stale` (logged)
3. static registry overlay value (documented fallback)
4. conservative default

Implemented baseline: BearWire run preflight reads the snapshot entry's `supports_responses_api`, derives `LlmApiStyle`, and passes it explicitly through `TurnStartRequest` / `AgentLoopSession`. `den-llm` exposes `preferred_api_style_for_model_with_catalog_support(model, supports_responses_api)` and contains no hidden global catalog state. The older pure `preferred_api_style_for_model(model)` remains only as a fallback for call sites that have not yet been handed a snapshot view.

### Single validation entry point

All "is this model usable?" checks — session model set, run preflight, bear default validation — should resolve and validate against the same snapshot through one helper, so the three current implementations converge and a persisted-but-now-unavailable model fails at preflight in one well-defined place rather than via three slightly different paths.

Implemented baseline: BearWire run preflight, BearWire session model selection, HTML bear create/edit, JSON bear create, and `/status` reconciliation read snapshot-derived availability. Remaining cleanup: extract the edge-local validation helpers into one shared cross-crate validation API and decide whether API/web should share one process-wide snapshot store rather than separate edge-owned stores.

---

## Alias resolution behavior

Den should resolve model requests using a deterministic precedence order.

### Resolution order

1. Exact canonical key match
   - example: `openai/gpt-4.1`
2. Exact alias match
   - example: `gpt-4.1`
   - example: `openai:gpt-4.1`
3. Exact legacy handle match
4. Optional future policy-based defaulting
   - example: family alias like `gpt-4.1-default`

### Constraints

- Alias collisions must be rejected at registry validation time.
- A canonical key cannot also be a different entry’s alias.
- Keep lifecycle simple for now: aliases either resolve or they do not. If richer deprecation semantics are needed later, add them deliberately.

---

## Data sourcing hierarchy

The Den metadata registry may combine several sources, but their trust ranking should be explicit.

### Preferred precedence for capability values

1. `provider_api`
2. `provider_docs`
3. `litellm_bootstrap`
4. `manual_curated`
5. `inferred`

### Interpretation

#### `provider_api`
Best when a provider exposes a machine-readable authoritative models endpoint including capability metadata.

#### `provider_docs`
Preferred fallback when docs provide clearer or more current limits than APIs.

#### `litellm_bootstrap`
Useful for broad initial coverage or backfilling many provider/model pairs quickly, but should usually not outrank direct provider information.

#### `manual_curated`
Useful for repo bootstrap, operator corrections, or temporary overrides when upstream data is incomplete.

#### `inferred`
Allowed only for soft assumptions and should not silently override better sources.

### First implementation recommendation

Use Bifrost `/v1/models` as the ordinary live availability source. Use `/api/models/details` and `/api/models/base` when management auth is available and richer catalog data is needed. Keep Den's local registry as a curated overlay/fallback, not as a complete manually maintained catalog.

This gives Den useful metadata while allowing Bifrost's Model Catalog and provider list-model APIs to keep the available model set fresh.

---

## Validation rules

The registry validator should reject invalid metadata before it is used for Bear configuration validation or operator status.

### Entry validation

Each entry must satisfy:
- non-empty `key`
- non-empty `provider`
- non-empty `provider_model_id`
- `key` prefix matches `provider/`
- `display_name` non-empty
- `context_window` positive if present
- `max_output_tokens` positive if present

### Global validation

Across the whole registry:
- canonical keys are unique
- aliases are globally unique
- legacy handles are globally unique unless explicitly tombstoned
- no ambiguous Den aliases
- provider references are known namespaces

### Availability reconciliation

When Bifrost is reachable:
- every configured Bear model should resolve to a Bifrost-available handle
- Bifrost-available handles without Den metadata should be surfaced as metadata gaps, not hard failures
- Den metadata entries not currently available in Bifrost should be surfaced as unavailable, not as Bifrost drift by default

---

## Reconciliation pipeline

The intended pipeline is:

1. Read Den metadata registry source.
2. Merge data-source overlays if configured.
3. Validate metadata entries and alias uniqueness.
4. Fetch or observe Bifrost-available model handles.
5. Produce the runtime index — a shared `BifrostCatalogSnapshot` (see [Runtime catalog snapshot](#runtime-catalog-snapshot-caching-contract)) — for Den validation, capability/API-style routing, and UI enrichment.
6. Surface reconciliation differences in status/admin views.
7. Optionally expose Den-native registry read APIs.

### Phase 1 recommended implementation split

#### Den-owned metadata overlay
A checked-in Den-side overlay file or Rust bootstrap, for example:
- `services/den/model_overrides.json`
- `services/den/config/model_metadata_overrides.json`
- or `den-runtime::llm::model_registry` while the shape is still small.

This overlay should contain only Den-specific labels, aliases, suitability hints, and corrections/overrides—not a full copy of the provider catalog.

#### Bifrost catalog and availability source
Use one or more Bifrost surfaces:
- `/v1/models` for ordinary live availability
- `/api/models/details` for richer capability metadata when management auth is enabled
- `/api/models/base` for base catalog/pricing awareness when management auth is enabled
- `/bears/models` compatibility metadata sidecar only as a fallback while it exists

#### Reconciliation step
A small Den-side tool or status helper that:
- reads Den metadata
- reads Bifrost-available handles
- reports available-with-metadata, available-without-metadata, and metadata-not-available entries

### Recommended ownership split

The cleanest medium-term split is:
- Den owns curated overlays and Bear-facing validation policy.
- Bifrost owns provider credentials, provider routing config, gateway-local operational flags, Model Catalog, pricing, and live availability.
- Optional sync/export tooling is an audited executor, not the architectural source of truth.

---

## Relationship to existing Bifrost config

The current `services/bifrost/config.json` mixes at least three concerns:

1. gateway operational settings
   - `client.disable_db_pings_in_health`
2. provider credential/routing settings
   - `providers.openai.keys[]`
3. model metadata presented to Den and users
   - `bears.models[]`

This spec proposes that only the third concern moves under Den canonical ownership first.

That means phase 1 does not need to redesign Bifrost configuration.
It only needs to let Den compare Bifrost availability with Den metadata and validate Bear model choices.

A later phase may add a file patcher or Bifrost management-API sync executor, but that is not required to establish the new architecture.

---

## Den API behavior

Once the metadata registry exists, Den should stop treating Bifrost metadata as authoritative for capability estimates, but it should continue treating Bifrost as authoritative for live availability.

### Desired behavior

Den should be able to:
- list Bifrost-available models enriched with Den metadata where known
- resolve aliases locally for validation/canonicalization
- compare local metadata with Bifrost-exposed availability for operator diagnostics

### Compatibility path

During migration, Den should keep using Bifrost metadata or management APIs as the availability signal, while capability/display metadata moves toward local registry reads.

---

## Availability reconciliation

Bifrost metadata is useful as the live availability surface.

Examples of reconciliation checks:
- configured Bear model missing from Bifrost availability
- Bifrost-available model has no Den metadata
- Den metadata entry is not currently available in Bifrost
- Den context-window estimate differs from Bifrost metadata when both are available

This is especially useful because `services/bifrost/COOLIFY_DEPLOY.md` documents a file-based GitOps deployment model where config files are mounted into the Bifrost container. Den should surface what is actually deployed rather than assuming local metadata matches gateway state.

---

## Migration path

### Phase 1: schema introduction

- Add Den metadata registry file or Rust bootstrap.
- Seed from the existing OpenAI entries in `services/bifrost/config.json` plus manual/provider-doc updates.
- Mark imported values primarily as `manual_curated` with repo-source references.

### Phase 2: compiler and generated projection

- Add a generator that emits Bifrost `bears.models[]` entries.
- Keep provider sections hand-managed.
- Validate that generated output is semantically equivalent to the current checked-in config.

### Phase 3: Den local resolution

- Update Den model-listing logic to read the canonical registry directly.
- Keep Bifrost metadata fetch available for verification or temporary fallback.

### Phase 4: richer sourcing

- Add provider docs/API harvesting where available.
- Upgrade field provenance and confidence.
- Add drift reporting and stale-data detection.

---

## Open questions

1. Should canonical registry storage live in Den service code, shared config, or a docs/config area with generation into both services?
2. Should Bifrost handles be required to equal provider model ids in phase 1, or merely default to them?
3. How much provider-specific execution metadata belongs in the canonical registry versus gateway/provider config?
4. Should Den expose provenance/confidence to end users, operators only, or not at all initially?
5. Should aliases be globally unique across all providers forever, or only within a provider namespace unless explicitly promoted?

---

## Recommended first concrete implementation

Implement the smallest useful slice:

1. Create `DenModelRegistryEntry` JSON for the current four OpenAI models.
2. Preserve current capability values and map them as imported manual curation.
3. Add a generator for Bifrost `bears.models[]`.
4. Keep current provider config untouched.
5. Add a Den-side resolver that can map:
   - canonical key
   - alias
   - legacy handle
6. Change Den model listing to prefer the local registry.

This yields the architecture change that matters most: Den becomes the authority for model identity and metadata, while Bifrost remains the gateway executor.
