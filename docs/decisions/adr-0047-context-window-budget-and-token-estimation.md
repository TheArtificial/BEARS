# ADR: Context Window Budget and Token Estimation

**Status:** Accepted
**Date:** 2026-06-29
**Deciders:** Hans

**Related:**
- [ADR-0032 — Den Context Compaction Architecture](adr-0032-den-context-compaction-architecture.md)
- [ADR-0035 — Den-native in-process agent runtime](adr-0035-den-native-in-process-agent-runtime.md)
- [ADR-0046 — File-backed prompt fragments and compiled runtime prompts](adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md)
- [ADR-0050 — Agent Loop Control, Adaptive Budgets, and Runtime Checkpoints](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md)
- [Den runtime architecture](../architecture/den-runtime.md)
- [Den model registry and Bifrost config plan](../roadmap/DEN_MODEL_REGISTRY_AND_BIFROST_CONFIG_PLAN.md)

> **Consumed by [ADR-0050 §11](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md) (2026-07-06).** The budget report defined here is now read by the agent loop controller as a first-class continuation dimension alongside wall-clock and tool-class spend, driving checkpoint-then-compact sequencing. This ADR remains the authority for *how* context is estimated; ADR-0050 governs *how the loop reacts* to it.

## Context

Den assembles each turn from compiled prompt fragments, transcript history, prompt-memory blocks, retrieved memory, tool surface descriptors, runtime notes, and compaction artifacts. We already know model context limits and output ceilings matter for:

- prompt fit before execution,
- compaction and trimming triggers,
- model selection,
- user/operator diagnostics,
- and cost/latency control.

Today Den has partial ingredients but no single, durable policy for **context budget tracking**. Without that:

- prompt fit is judged late (often only by provider failure),
- compaction triggers are blind to the actual request shape,
- token usage estimates vary by subsystem,
- and “context budget” surfaces cannot explain where the budget went.

We need one Den-owned approach that works before every inference request, does not depend on providers returning usage data after the fact, and degrades gracefully when exact tokenization is unavailable.

## Decision

Den will track **context window budget** against the **fully assembled request payload** that is about to be sent to the model.

### 1. Budgeting happens on the compiled request, not on fragments in isolation

Token estimation is performed on the final assembled request shape, including:

- compiled base/system prompt,
- transcript replay,
- prompt-memory blocks,
- retrieved/derived memory inserts,
- tool schemas and tool-surface guidance,
- runtime notes and compaction artifacts,
- and an explicit output-token reserve.

Subsystems may still report per-component estimates, but the authoritative budget check is on the final request the model will receive.

### 2. Den keeps per-component attribution alongside the total

Budget tracking must produce both:

- a **total estimated input token count**, and
- a **breakdown by component** (compiled prompt, transcript, tools, prompt memory, retrieved memory, runtime notes, etc.).

This is required so compaction, trimming, and diagnostics can explain *why* a request is near or over budget.

### 3. Exact model-aware tokenization is preferred, approximate estimation is allowed

Den should prefer a model-aware tokenizer when available. In order of preference:

1. a Den-local tokenizer calibrated to the selected model family,
2. a model-aware token counting service exposed by the inference substrate,
3. a conservative approximation.

When exact tokenization is unavailable, Den may fall back to a calibrated approximation (for example character-based heuristics), but must mark the result as approximate in diagnostics.

### 4. Model limits come from resolved model metadata

Budget checks compare the estimate against the selected model’s known constraints:

- `context_length`
- `max_output_tokens`
- any role/runtime reserve policy Den applies

The preferred source is resolved model metadata from the Den/Bifrost model registry path. Den may keep a minimal fallback metadata table for models whose live metadata is temporarily unavailable.

### 5. Output reserve is part of the budget, not an afterthought

Den must reserve output tokens before deciding a request fits. A request is evaluated against:

```text
estimated_input_tokens + reserved_output_tokens <= model_context_limit
```

The reserve may vary by role, model, or strategy policy, but it must be explicit and visible in diagnostics.

### 6. Budget tracking is a runtime policy input, not just observability

Context budget output should drive:

- compaction triggers,
- request rejection before inference when clearly over budget,
- model-selection warnings or fallback suggestions,
- operator/admin surfaces,
- and session-level/runtime-level diagnostics.

### 7. Den calibrates estimates against observed provider usage when available

When the inference substrate/provider returns actual prompt or completion token usage, Den should store or log enough comparison data to calibrate estimates over time.

Calibration must not replace preflight estimation; it improves it.

> **Amendment (2026-07-30) — memory components and calibration home.** Two clarifications from the memory-budget review:
>
> 1. **Key memory projection** and **derived recall** are named components in the budget report's per-component attribution (§2), distinct from prompt-memory blocks. The memory subsystem's character caps ([den-runtime v1 budgets](../architecture/den-runtime.md#v1-budgets)) remain selection heuristics inside the projector; this ADR's final-request estimate is the only budget authority. Den does not grow a second, memory-local token estimator.
> 2. **Calibration data lives in the model registry**: observed Bifrost prompt-token usage is recorded against assembled character counts to maintain per-model-family chars→tokens correction ratios, which feed both the approximate estimator (§3, option 3) and periodic re-tuning of the memory char caps. This keeps Den out of the token-counting-authority business, consistent with the Model-Ops principle that Bifrost owns usage truth and Den mirrors it for policy and UX.

### 8. Den persists the latest session budget view for inspection

Den should persist the **most recent** context-budget report for a session so human
operators can inspect the latest compiled-context breakdown in the conversations web UI.

This is a **latest snapshot**, not a full history log:

- update/replace the latest persisted report for the session,
- expose it in conversation/session read models,
- do not introduce a separate append-only budget history unless a later need appears.

## Consequences

### Positive

- Den can reject or compact before provider failure.
- Compaction and prompt assembly gain a shared budget vocabulary.
- Session diagnostics can explain where context was spent.
- Model metadata becomes directly useful to runtime policy.
- Cost tracking and context tracking can share the same request breakdown.

### Negative / tradeoffs

- Requires tokenization logic or a counting integration per model family.
- Approximate fallback logic must be calibrated and clearly labeled.
- Request assembly code must preserve enough structure to attribute budget by component.

## Non-goals

- This ADR does **not** choose one universal tokenizer implementation today.
- It does **not** require exact estimates for every model before the feature can ship.
- It does **not** replace context compaction policy; it provides one of the key inputs to that policy.

## Implementation notes

The implementation should expose a Den-owned “budget report” object with at least:

- selected model handle,
- model context limit,
- reserved output tokens,
- estimated input tokens,
- estimate precision (`exact` / `approximate`),
- over-budget / near-budget flags,
- component-attributed estimates.

The implementation should also define where the latest session-scoped report is
persisted and how the conversations web UI reads it.

The active sequencing lives in [CONTEXT_WINDOW_BUDGET_IMPLEMENTATION_PLAN.md](../roadmap/CONTEXT_WINDOW_BUDGET_IMPLEMENTATION_PLAN.md).
