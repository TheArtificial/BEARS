# Model Ops Implementation Plan

Status: draft implementation plan  
Date: 2026-06-20

## Goal

Model Ops is the BEARS subsystem for configuring, selecting, validating, routing, observing, and governing LLM model use across Bears, profiles, conversations, turns, and background model tasks.

The goal is to give Den robust Bear-aware model policy and observability while taking advantage of Bifrost as much as possible for live model availability, provider routing, failover, budgets, rate limits, usage telemetry, and gateway governance.

## Terminology

Use precise terms instead of overloading “orchestration.”

| Term | Meaning | Primary owner |
|---|---|---|
| Agent orchestration | Turn lifecycle, tool calls, approvals, compaction, subagent loops, runtime state | Den |
| Model Ops | Umbrella for model configuration, selection, validation, routing awareness, usage, and budgets | Den + Bifrost boundary |
| Model policy | Bear/profile/conversation/task rules for requested model behavior | Den |
| Model selection | Choosing a requested/selected model from policy and intent | Den, optionally delegated to router/selector later |
| Model routing | Routing the selected request to provider/model/key/fallbacks | Bifrost |
| Gateway governance | Provider keys, virtual keys, allowed models, routing rules, budgets/rate limits | Bifrost |
| Bifrost Model Catalog | Live model availability, provider/model mapping, pricing, provider-reported capabilities | Bifrost |
| Den model metadata overlay | Bear-facing labels, context/capability overrides, profile/task suitability, validation notes | Den |
| Requested model | What a human/profile/policy asked for | Den |
| Selected model | What Den chose for a conversation/turn before gateway execution | Den |
| Actual model | What Bifrost/provider actually used after routing/fallback | Bifrost reports; Den records |

## Design principles

1. **Bifrost owns live availability and execution.** Den must not duplicate provider-key routing, gateway failover, or low-level budget/rate enforcement when Bifrost can do it.
2. **Den owns Bear-aware model policy.** Bear defaults, profile overrides, conversation stickiness, intent heuristics, and model-suitability diagnostics are Den concerns.
3. **Bifrost's Model Catalog is Den's primary source for model facts where available.** Den owns curated overlays and Bear-specific suitability metadata, not a manually duplicated full catalog.
4. **Requested, selected, and actual model are distinct.** UI, ACP, BearWire, logs, and usage records should preserve the distinction.
5. **Conversation model selection should stick.** Auto/manual selection should not thrash turn-by-turn unless policy explicitly changes it.
6. **Auto mode is a policy, not a model.** Auto chooses from candidate sets using heuristics or future model-routing services.
7. **Budgets should be enforced in Bifrost when possible.** Den mirrors/ingests budget and usage state for Bear-visible UX and resolver decisions.
8. **Model mismatch detection is diagnostic first.** Escalating to subagents is a future runtime feature; v1 should flag candidates and adjust next-turn policy only where safe.

## Ownership split

### Den owns

- Bear default model policy.
- Per-profile model policy.
- Conversation-level model state and stickiness.
- User-facing model selection/change UI.
- Auto-mode candidate selection and intent heuristics.
- Den model metadata overlay:
  - Bear-facing display labels and notes,
  - conservative context-window / max-output overrides,
  - tool/vision/reasoning capability corrections when Bifrost data is missing or wrong,
  - profile/task suitability hints,
  - Den-side aliases for validation and user input.
- Bear-facing validation and diagnostics:
  - configured model unavailable,
  - metadata unknown,
  - model likely too weak/too expensive/too small.
- Bear-visible usage/cost summaries.
- Runtime/BearWire/ACP events describing selected/actual model facts.

### Bifrost owns

- Provider credentials and secrets.
- Provider keys and key-level allowed model lists.
- Provider/model routing and fallback.
- Gateway aliases used for execution.
- Virtual keys and gateway-level governance.
- Rate limits and budgets where configured.
- Live model availability for the deployment.
- Model Catalog data:
  - provider/model mapping,
  - provider list-models results,
  - pricing/cost metadata,
  - context and modality/capability metadata when available.
- Provider usage/cost telemetry where available.
- Low-level retry/cooldown/load-balancing behavior.

## Current baseline

Implemented or partially implemented today:

- Den-native runtime calls Bifrost directly for inference (ADR-0035).
- Den has a static model metadata overlay bootstrap in `den-llm::model_registry`.
- Bifrost provider keys are no longer explicitly limited to a manually maintained model allowlist when the desired behavior is all catalog-supported models.
- Den's Bifrost client reads availability and capability metadata from paginated Bifrost `GET /v1/models`; the legacy `/bears/models` sidecar is compatibility-only and must not be used for Bear Admin availability.
- Den holds a shared `BifrostCatalogSnapshot` / `BifrostCatalogStore` in edge state and refreshes it in the background via `BIFROST_CATALOG_REFRESH_SECS`.
- Bear Admin model configuration is catalog-first:
  - the submitted `Model` fields use a datalist populated from the full Bifrost snapshot,
  - curated dropdowns are convenience shortcuts only,
  - `inherit` means inherit the relevant default.
- Bear default model exists in `bears.default_model`.
- Per-profile model overrides exist in `bear_profile_model_settings`:
  - missing/blank profile model means inherit Bear default.
- Conversation-level model state exists in `conversation_model_state` and is used by web chat and BearWire/ACP session model controls.
- Runtime resolution uses conversation explicit/auto state → profile override → Bear default → system default.
- BearWire defines `session.model.get`, `session.model.set`, and `model.selection.changed`.
- Key-memory projection budget can use known model context-window metadata.
- `/status` includes a Den metadata vs Bifrost availability reconciliation panel.

Important note from Phase 1/1.5 churn:

- We have churned several times around whether Den should use Bifrost `/v1/models`, `/api/models/details`, `/api/models/base`, or the BEARS sidecar for model choices. For ordinary Bear Admin availability and model selection, the settled v1 rule is: **use the paginated `/v1/models` snapshot as the source for selectable/live models**. Management catalog endpoints can be evaluated later for richer metadata, but they must not replace `/v1/models` as the availability source without a deliberate design update.

Known gaps:

- Den overlay is still static Rust code rather than a small overlay artifact/cache hydrated from Bifrost catalog data.
- Den does not yet parse/use richer Bifrost management catalog endpoints (`/api/models/details`, `/api/models/base`) for supplemental metadata when management auth/config_store are available.
- The catalog snapshot has periodic refresh, but last-good error metadata/operator diagnostics are still thin.
- Model validation helpers are still spread across a few edge paths rather than one shared cross-crate API.
- No selected-vs-actual model execution/usage event persistence yet.
- No model usage/cost event ingestion yet.
- No Bear overview usage/cost panel yet.
- No budget mirror/resolver integration yet.
- No auto-mode heuristic selector yet.
- No subagent escalation implementation yet.

## Target model policy layers

Model resolution should be layered:

```text
Deployment defaults / Bifrost governance
  ↓
Bear default model policy
  ↓
Profile model policy
  ↓
Conversation model state
  ↓
Turn intent / task-class override
  ↓
Bifrost live routing / fallback
```

### Layer 1 — Deployment defaults

- System `DEFAULT_LLM_MODEL` remains final fallback.
- Bifrost virtual keys/routing rules may enforce deployment-level provider availability, fallback, budgets, and rate limits.

### Layer 2 — Bear default

- Existing `bears.default_model` remains the Bear-wide default.
- Must validate against Bifrost availability when edited.
- Can be used by profiles that inherit.

### Layer 3 — Profile default

- `bear_profile_model_settings` defines optional profile override.
- Profiles:
  - `chat`,
  - `pair`,
  - `curate`,
  - `work`,
  - `watch`.
- Missing/blank model = inherit Bear default.

### Layer 4 — Conversation model state

A conversation should remember model selection.

Conceptual fields:

```text
conversation_id
profile
selection_mode: auto | explicit
requested_model: nullable
selected_model: nullable
selected_reason: nullable
actual_last_model: nullable
actual_last_provider: nullable
fallback_count: integer
metadata_json
updated_at
```

Rules:

- New conversation inherits profile policy.
- Explicit user selection sticks to the conversation.
- Auto mode chooses from policy and intent, then sticks the selected model until state/policy changes.
- Bifrost fallback does not silently rewrite the requested/selected model; Den records actual model separately.

### Layer 5 — Turn/task override

Turn-level policy may choose another model for specific task classes or diagnostics:

- `agent_primary`,
- `agent_compaction`,
- `embedding`,
- `rerank`,
- `memory_extraction`,
- `short_generation`,
- `classification`,
- `structured_extraction`,
- `reflection`,
- `evaluation`.

Foreground turns should eventually carry intent/task tags such as:

- `simple_chat`,
- `coding`,
- `architecture`,
- `debugging`,
- `long_context`,
- `repair`,
- `autonomous_work`,
- `summarization`.

## Bifrost capabilities to leverage

Use Bifrost before implementing Den-local alternatives.

### Availability and catalog data

Use Bifrost catalog surfaces with separate responsibilities:

1. `GET /v1/models` is the v1 source for live availability in ordinary deployments. Den should use it through the shared `BifrostCatalogSnapshot`, with pagination, for Bear Admin selectable models, validation, BearWire model options, and status reconciliation.
2. `GET /api/models/details` may be used later as supplemental metadata when management auth/config_store is available, for richer catalog facts such as context, modalities, supported methods, and accessible keys. Do not use it as the primary availability source unless we deliberately redesign the boundary.
3. `GET /api/models/base` may be used later when Den needs base model/pricing-catalog awareness rather than deployment availability.
4. `/bears/models` sidecar is compatibility-only. It must not feed Bear Admin availability or the primary datalist.

Avoid manually enumerating every provider model in Den or in Bifrost provider-key `models` allowlists unless the goal is intentional restriction.

### Routing and failover

Use Bifrost:

- provider/model restrictions,
- virtual-key routing,
- weighted load balancing,
- automatic fallbacks,
- routing rules,
- CEL conditions,
- complexity tier routing,
- capacity variables such as `budget_used`, `tokens_used`, and `request`.

Den should provide useful request metadata/headers where needed so Bifrost routing rules can make decisions.

### Pricing, budgets, and usage

Prefer Bifrost's Model Catalog and governance APIs for pricing and hard enforcement:

- virtual-key budgets,
- provider/model limits,
- rate limits,
- budget-aware routing rules,
- gateway usage/cost telemetry,
- Model Catalog pricing data and Bifrost cost calculation where exposed.

Den should mirror or ingest results for Bear-level UX and high-level model policy.

### Actual model reporting

Den should capture actual model/provider used from:

- Bifrost response metadata if exposed,
- OpenAI-compatible usage fields,
- Bifrost logs/management APIs if needed,
- request IDs / event IDs for correlation.

## Data model plan

### Bifrost catalog + Den overlay model

Den should treat effective model metadata as:

```text
effective_model_metadata = Bifrost catalog/availability facts + Den curated overlay + Bear/profile policy
```

Bifrost-derived facts include:

- available/routable handles,
- provider/model mappings,
- accessible keys where management APIs expose them,
- pricing/cost metadata,
- context length and max output where available,
- modalities and supported methods where available.

Den overlay facts include:

- Bear-facing labels and grouping,
- conservative overrides/corrections,
- profile/task suitability tags,
- UI visibility/selectability hints,
- Den-side aliases and migration compatibility.

### Existing / v1

#### `bears.default_model`

Bear default model. Existing.

#### `bear_profile_model_settings`

Profile override or inherit.

```sql
bear_id UUID NOT NULL
profile TEXT NOT NULL
model TEXT NULL
created_at TIMESTAMPTZ
updated_at TIMESTAMPTZ
PRIMARY KEY (bear_id, profile)
```

`model IS NULL` or blank = inherit Bear default.

### Phase 1.5 — Catalog cache / overlay source

Add a lightweight Den cache/overlay if runtime calls to Bifrost catalog are too expensive or if status/admin pages need stable historical observations.

Possible tables:

```sql
model_catalog_cache (
  handle TEXT PRIMARY KEY,
  provider TEXT NULL,
  source TEXT NOT NULL, -- bifrost_v1_models | bifrost_api_models_details | bifrost_api_models_base
  metadata_json JSONB NOT NULL,
  observed_at TIMESTAMPTZ NOT NULL,
  expires_at TIMESTAMPTZ NULL
)
```

```sql
model_metadata_overrides (
  handle TEXT PRIMARY KEY,
  metadata_json JSONB NOT NULL,
  notes TEXT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
)
```

Do not add this until we confirm live Bifrost catalog queries are insufficient for admin/runtime needs.

### Phase 1.5 — Bifrost catalog-first metadata ✅

- Prefer Bifrost `/v1/models` over the legacy BEARS sidecar for availability.
- Let Bifrost Model Catalog/provider list-model APIs populate available models.
- Keep Den static registry as overlay/fallback only.
- Remove or relax explicit key-level model allowlists unless intentionally restricting access. Done for the default OpenAI key in `services/bifrost/config.json`.
- Future improvement: evaluate `/api/models/details` and `/api/models/base` for richer metadata when management auth is configured.

Exit:

- Bear Admin model selectors reflect Bifrost's current provider catalog without manually editing every model name.
- Den labels unknown-but-available Bifrost models as metadata gaps rather than hiding them.
- `/status` reports Bifrost-available-without-Den-overlay and Den-overlay-not-currently-available.

### Phase 2 — Conversation model state

Add:

```sql
conversation_model_state (
  conversation_id UUID PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
  selection_mode TEXT NOT NULL CHECK (selection_mode IN ('auto', 'explicit')),
  requested_model TEXT NULL,
  selected_model TEXT NULL,
  selected_reason TEXT NULL,
  actual_last_model TEXT NULL,
  actual_last_provider TEXT NULL,
  fallback_count INTEGER NOT NULL DEFAULT 0,
  metadata_json JSONB NOT NULL DEFAULT '{}',
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
)
```

### Phase 3 — Model usage events

Add or reuse event storage for:

```sql
model_usage_events (
  id UUID PRIMARY KEY,
  bear_id UUID NOT NULL,
  conversation_id UUID NULL,
  profile TEXT NULL,
  turn_id UUID NULL,
  task_class TEXT NOT NULL,
  model_requested TEXT NULL,
  model_selected TEXT NULL,
  model_actual TEXT NULL,
  provider_actual TEXT NULL,
  input_tokens BIGINT NULL,
  output_tokens BIGINT NULL,
  total_tokens BIGINT NULL,
  cost_usd NUMERIC NULL,
  currency TEXT NULL,
  bifrost_request_id TEXT NULL,
  metadata_json JSONB NOT NULL DEFAULT '{}',
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
)
```

### Phase 4 — Budget mirror/cache

Only after confirming Bifrost budget surfaces.

Possible Den mirror:

```sql
model_budget_states (
  scope_kind TEXT NOT NULL, -- instance | bear | profile | model
  scope_id TEXT NOT NULL,
  model TEXT NULL,
  period TEXT NOT NULL,
  limit_usd NUMERIC NULL,
  used_usd NUMERIC NULL,
  starts_at TIMESTAMPTZ NULL,
  resets_at TIMESTAMPTZ NULL,
  exhausted BOOLEAN NOT NULL DEFAULT FALSE,
  source TEXT NOT NULL, -- bifrost | den
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (scope_kind, scope_id, model, period)
)
```

## Runtime model resolver

Introduce a single resolver that returns a structured result, not just a string.

```rust
struct ModelResolution {
    requested_model: Option<String>,
    selected_model: String,
    actual_model: Option<String>,
    source: ModelSelectionSource,
    selection_mode: ModelSelectionMode,
    metadata_known: bool,
    available_in_bifrost: bool,
    context_window: Option<u32>,
    max_output_tokens: Option<u32>,
    reason: String,
}
```

Sources:

- `user_explicit`,
- `conversation_explicit`,
- `conversation_auto_sticky`,
- `profile_override`,
- `bear_default`,
- `system_default`,
- `task_override`,
- `fallback_recorded`.

Resolution order for primary turns:

1. explicit user request for this conversation/turn,
2. conversation explicit state,
3. conversation auto sticky state if still valid,
4. profile override,
5. Bear default,
6. system default,
7. Bifrost fallback during execution.

## UI plan

### Bear Models page

Implemented v1:

- Bear default selector.
- Per-profile inherit/override selectors.
- Resolved model/source display.
- Note: lane-specific assignments are TBD.

Next improvements:

- show metadata-known / metadata-unknown badges,
- show Bifrost availability status,
- show context/max output/capabilities inline,
- support “auto” per Bear/profile.

### Conversation UI / ACP

Add:

- current selected model display,
- auto/manual toggle,
- model selection dropdown,
- actual model/fallback notice,
- model changed event projection to ACP/web clients.

### Bear overview usage panel

Add:

- current period usage/cost,
- model breakdown,
- profile breakdown,
- fallback count,
- budget status,
- link to detailed usage view.

## Events and observability

Emit or persist semantic events for:

- `model.selection.changed`,
- `model.selection.resolved`,
- `model.execution.started`,
- `model.execution.fallback`,
- `model.execution.completed`,
- `model.usage.recorded`,
- `model.budget.exhausted`,
- `model.budget.reset`.

Event payloads should preserve:

- requested model,
- selected model,
- actual model,
- provider,
- selection source,
- selection reason,
- task class,
- Bifrost request/event id,
- usage/cost when known.

## Model-inappropriateness detection

Near-term diagnostics only.

Candidate signals:

- prompt too large for selected model,
- repeated tool-call failure,
- model emitted pseudo-tool text instead of native tool calls,
- model selected has no tool support for a tool-heavy turn,
- long-context memory projection was heavily omitted,
- Bifrost fallback occurred,
- expensive model used for trivial task,
- user explicitly asks for deeper reasoning/coding/architecture,
- governance mode shifts to autonomous continuation.

Near-term outputs:

- diagnostic flag on turn,
- optional next-turn auto-mode adjustment,
- operator-visible notices.

Future outputs:

- spawn stronger/weaker subagent candidate loop,
- compare/evaluate outputs,
- merge result into primary conversation.

## Implementation phases

### Phase 0 — Terminology and docs ✅

- Establish “Model Ops” as umbrella term.
- Clarify Bifrost owns availability/execution; Den owns policy/metadata/validation.
- Update model registry docs to reconciliation model.

### Phase 1 — Bear/profile model policy ✅

- Bear default model editable.
- Per-profile inherit/override table.
- Models page in Bear Admin.
- Runtime resolver uses profile override → Bear default → system default.
- Lane-specific assignments explicitly TBD.
- Models page shows availability and metadata-known/unknown status.
- Model fields use datalist-backed text inputs populated from full Bifrost `/v1/models` availability; curated dropdowns are non-submitting shortcut controls only.
- `inherit` is explicit in the UI and means inherit the relevant default.
- Runtime resolver has unit coverage for profile override → Bear default → system default.
- Auto mode is explicitly noted as planned, not yet available for Bear/stance defaults.

### Phase 1.5 — Catalog cache / overlay source

Add a lightweight Den cache/overlay if runtime calls to Bifrost catalog are too expensive or if status/admin pages need stable historical observations.

Possible tables:

```sql
model_catalog_cache (
  handle TEXT PRIMARY KEY,
  provider TEXT NULL,
  source TEXT NOT NULL, -- bifrost_v1_models | bifrost_api_models_details | bifrost_api_models_base
  metadata_json JSONB NOT NULL,
  observed_at TIMESTAMPTZ NOT NULL,
  expires_at TIMESTAMPTZ NULL
)
```

```sql
model_metadata_overrides (
  handle TEXT PRIMARY KEY,
  metadata_json JSONB NOT NULL,
  notes TEXT NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
)
```

Do not add this until we confirm live Bifrost catalog queries are insufficient for admin/runtime needs.

### Phase 2 — Conversation model state ✅

- Add `conversation_model_state` table.
- Add resolver logic for conversation sticky selection.
- Add web chat model selection controls.
- Display selected/effective conversation model in the web chat toolbar.
- Add BearWire/ACP model selection controls through `session.model.get`, `session.model.set`, and `model.selection.changed`.

Exit:

- changing model in one web or ACP conversation affects subsequent turns only for that conversation,
- selected model survives page reload/session continuation,
- ACP clients can display and update conversation-scoped model state.

Remaining:

- actual model/fallback reporting (Phase 3),
- explicit auto-mode heuristics beyond inherit stance/Bear default (Phase 6).

### Phase 2.5 — Configured selectable model options ✅

Goal: give Den a stable, operator-controlled list of models users can choose from, reconciled with Bifrost availability but not replaced by it.

Implemented:

- Add `model_selection_options` table seeded with Den's curated OpenAI model set.
- Use configured Den model options for Bear Admin, web chat, and BearWire/ACP model selectors.
- Keep Bifrost live catalog as status/availability metadata and as a supplemental advanced/autocomplete source.
- Allow selector stability even when Bifrost `/v1/models` temporarily shrinks or expands.

Exit:

- ACP model selector is stable across Bifrost catalog refreshes,
- Bear Admin and web chat selectors are stable across Bifrost catalog refreshes,
- Bifrost availability is diagnostic/status metadata, not the primary selector source,
- existing saved Den-configured models can be shown even when Bifrost availability is unknown or temporarily absent.

### Phase 3 — Actual model and usage capture ⬅ next

Goal: create a durable per-call trace from Den's requested/selected model through Bifrost/provider execution results, without reopening the model catalog source decision.

Implementation notes:

- Keep model availability/catalog reads on the existing `BifrostCatalogSnapshot` path. Phase 3 should not switch Bear Admin or runtime validation to `/api/models/details`, `/api/models/base`, or `/bears/models`.
- Add `model_usage_events` or equivalent event storage with requested/selected/actual model fields.
- Record Den-side facts before execution:
  - `bear_id`, `conversation_id`, profile/stance,
  - task class (`agent_primary` first; later compaction/reflection/etc.),
  - selection mode/source (`conversation_explicit`, `profile_default`, `bear_default`, etc.),
  - requested model and selected model.
- Capture Bifrost/provider facts after execution where available:
  - actual model/provider,
  - request id / response id for correlation,
  - input/output/total tokens,
  - cost/currency if Bifrost/provider exposes it,
  - fallback/routing metadata when exposed.
- Update `conversation_model_state.actual_last_model`, `actual_last_provider`, and `fallback_count` opportunistically from the same execution result.
- Emit runtime events for `model.execution.started`, `model.execution.completed`, `model.execution.fallback` when detectable, and `model.usage.recorded`.
- Surface the selected vs actual distinction to BearWire/ACP and web clients; do not mutate selected model just because Bifrost/provider fell back.

Likely first integration points:

- BearWire Pair run preflight already produces a `ResolvedRunModel` in `services/den/crates/den-bearwire/src/methods/run.rs`; use that as the selected-model source for Pair/ACP runs.
- Den runtime/Bifrost client response handling is where actual model and token usage should be captured.
- `conversation_persistence::ConversationModelState` already has actual/fallback columns ready for update.

Exit:

- each primary model call has requested/selected/actual model trace when enough information is available,
- token usage is recorded when provider/Bifrost reports it,
- selected-vs-actual model differences are observable in logs/events and persisted usage records,
- Phase 4 Bear usage UI can be built from the persisted usage/events table.

### Phase 4 — Bear usage and cost UI

- Bear overview usage panel.
- Detailed usage breakdown by model/profile/conversation.
- Cost estimates where available.

Exit:

- Bear admin can see current-period usage/cost and top models.

### Phase 5 — Budget integration

- Decide Bifrost virtual-key granularity.
- Query/ingest Bifrost budget state.
- Reflect budget exhaustion in Den resolver.
- Surface budget reset timing.

Exit:

- budget-exhausted models are avoided or flagged,
- Bifrost remains hard enforcement path where configured.

### Phase 6 — Auto mode v1

- Add `auto` mode for Bear/profile/conversation.
- Implement simple heuristics:
  - long-context → long-context model,
  - coding/debugging/architecture → stronger model,
  - simple chat/classification → cheaper model,
  - budget pressure → cheaper available model,
  - tool-heavy → tool-capable model.
- Stick auto-selected model at conversation level.

Exit:

- new conversations can use auto mode predictably with explainable reasons.

### Phase 7 — Advanced routing / Not Diamond / Bifrost complexity routing

- Evaluate Bifrost complexity routing and Not Diamond-style model routing.
- Decide whether auto mode delegates to Bifrost routing rules, external selector, or Den heuristic.
- Preserve requested/selected/actual model trace regardless of selector.

Exit:

- auto mode can use richer routing while Den still records explainable selection state.

### Phase 8 — Subagent escalation hooks

- Add inappropriate-model diagnostics as candidate triggers.
- Add optional stronger/weaker subagent loops once subagents exist.
- Evaluate/merge subagent outputs.

Exit:

- model escalation is explicit, observable, and governed by policy.

## Open questions

1. What Bifrost availability surface should Den prefer long-term: `/bears/models`, `/v1/models`, or management APIs?
2. What Bifrost virtual-key granularity maps best to Bear-level usage and budget UX?
3. Should Den store Bifrost-routable handles or Den canonical metadata keys on Bears/conversations?
4. Should `auto` be available at Bear/profile level immediately, or only at conversation level first?
5. What request metadata/headers should Den send to Bifrost to unlock routing rules and budget attribution?
6. What cost source is most reliable: Bifrost logs, provider usage fields, or Den-side estimates?
7. How should user-initiated model switches interact with active tool calls and pending approvals?
