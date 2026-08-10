# Context Window Budget Implementation Plan

## Status

Landed (audited 2026-07-31). Implements [ADR-0047 — Context Window Budget and Token Estimation](../decisions/adr-0047-context-window-budget-and-token-estimation.md).

Landed: `ContextBudgetReport` DTO (`den-protocol/src/context_budget.rs`), per-component estimation including distinct `key_memory_projection` and `recall` buckets (`den-runtime/src/context_budget.rs`), turn enforcement (`agent_loop/budget.rs`), per-turn persistence and admin surfacing, and Phase 4 calibration. Bifrost-reported prompt usage from both streaming APIs is folded atomically into a bounded EMA chars→tokens ratio on the canonical model-registry row. After five samples, the final-request estimator applies that ratio and labels the snapshot `calibrated_approximate`; otherwise it explicitly retains the chars/4 fallback. The observed character total is the serialized assembled request, so it includes key-memory projection and derived-recall content alongside the transcript and tool surface. Calibration remains advisory: a failed read or write never delays or fails inference. The persisted ratios are the evidence for periodic memory character-cap review; they do not create a tokenizer in the memory selection path.

## Goal

Give Den a reliable, explainable **context budget** check before inference by estimating token use for the fully assembled request payload, comparing it to resolved model limits, and exposing the result to compaction, diagnostics, and operator tooling.

## Scope

In scope:

- final-request token estimation,
- model-limit lookup,
- output-token reserve policy,
- per-component budget attribution,
- latest-session budget snapshot persistence for UI inspection,
- runtime diagnostics and admin/status exposure,
- calibration hooks against observed provider usage.

Out of scope:

- a full provider-agnostic tokenizer framework for every model family,
- changing compaction policy semantics themselves,
- cost billing/reporting beyond the budget-tracking data needed to support it.

## Why this is separate from compaction

Compaction decides *what* to shrink and *when*. Budget tracking decides *how much room is left* and *which part of the request consumed it*. The two are tightly related, but budget tracking should be usable before compaction is fully complete.

## Phases

### Phase 0 — Budget report shape

Add a shared runtime/service shape for a budget report, for example:

- model handle,
- context limit,
- reserved output tokens,
- estimated input tokens,
- estimated total,
- estimate precision,
- near/over budget booleans,
- per-component estimates.

Prefer a stable DTO in `den-protocol` or a narrow runtime-owned type only if it is not needed by edges.

### Phase 1 — Model limit resolution

Resolve model metadata for the selected model at turn assembly time:

- `context_length`
- `max_output_tokens`
- any relevant tool/method capability bits used by request shaping

Source priority:

1. resolved Den/Bifrost model metadata,
2. Den fallback metadata for known models when live metadata is absent.

### Phase 2 — Fully assembled request estimation

Estimate tokens on the final assembled request payload, not intermediate fragments.

Required buckets:

- compiled base prompt,
- transcript replay,
- prompt-memory blocks,
- **key memory projection** (path anchors — distinct bucket per the 2026-07-30 ADR-0047 amendment),
- **derived recall** (injected recall passages — distinct bucket, same amendment),
- tool schemas/tool surface,
- runtime notes/compaction supplements,
- output reserve.

Key memory projection and derived recall must be attributed separately (not merged as "retrieved memory"): their char caps are tuned independently ([den-runtime v1 budgets](../architecture/den-runtime.md#v1-budgets)) and the calibration loop in Phase 4 feeds back into those caps.

Exact model-aware tokenization is preferred where practical. Fallback approximation is acceptable if clearly labeled.

### Phase 3 — Runtime enforcement and diagnostics

Before inference:

- attach the budget report to runtime diagnostics,
- reject clearly over-budget requests before provider call,
- emit near-budget warnings,
- pass budget state into compaction/overflow policy.

This should surface in:

- turn logs,
- `session_info` / `bear_environment` runtime metadata,
- operator/admin views where useful.

### Phase 3.5 — Persist latest session budget snapshot

Persist the **latest** budget report for a session/conversation so humans can inspect
the most recent compiled-context breakdown in the web UI.

Requirements:

- one latest snapshot per active session/conversation view,
- overwrite/update on each new compiled budget report,
- no append-only history required,
- read-path support for the conversations/session UI.

Suggested stored fields:

- session/conversation identifier,
- model handle,
- context limit,
- reserved output tokens,
- estimated input/total tokens,
- estimate precision,
- near/over budget flags,
- per-component breakdown,
- updated timestamp.

### Phase 4 — Calibration against actual usage

**Complete (2026-07-31).** Den captures provider-reported prompt usage from
OpenAI-compatible Chat Completions usage chunks and Responses `response.completed`
events. It atomically folds each usable observation into the resolved canonical
model-registry entry, avoiding lost updates from concurrent streams. The stored ratio
is applied only after the minimum sample threshold and is exposed as calibration
provenance on the resulting budget snapshot. Unknown models, absent usage, and all
calibration persistence failures retain the safe uncalibrated estimate and do not
block a turn.

When provider/Bifrost usage data is available, compare:

- estimated prompt tokens,
- estimated completion reserve,
- actual prompt/completion usage.

Use this to tune fallback estimators per model family. Do not block runtime checks on this phase.

**Calibration home and memory-cap feedback (ADR-0047 amendment, 2026-07-30):**

- Persist per-model-family **chars→tokens correction ratios** in the model registry (alongside resolved model metadata), computed from observed Bifrost prompt-token usage against assembled character counts. Bifrost stays the usage authority; Den mirrors ratios for estimation.
- The ratios feed two consumers: the approximate estimator (Phase 2 fallback path) and **periodic re-tuning of the memory char caps** in [den-runtime v1 budgets](../architecture/den-runtime.md#v1-budgets). The memory subsystem never grows its own token estimator — char caps remain selection heuristics recalibrated from this measured loop.
- Include the key-memory-projection and derived-recall buckets in calibration comparisons so their ratios reflect memory-content character density (prose-heavy anchors vs code-heavy work-surface docs), not just transcript averages.

**Operating cadence:** periodically review calibrated ratios together with the
persisted component breakdowns, then adjust the static key-memory and derived-recall
character caps through their normal reviewed configuration/code path. The memory hot
path continues to use only characters; it neither calls a tokenizer nor makes a
per-request calibration lookup.

### Phase 5 — Policy integration

Use budget state to improve:

- compaction trigger thresholds,
- model-selection warnings,
- safe reserves by role/model,
- operator explanations for context pressure.

## Implementation notes

### Estimation strategy

Use a layered approach:

1. exact tokenizer/counting path when available,
2. calibrated approximation otherwise,
3. preserve component attribution in both cases.

### Output reserve policy

Keep reserve policy explicit and centralized. Start with a simple default plus model/role overrides rather than ad hoc call-site logic.

### Failure semantics

- If limits are unknown, continue with an approximate report and label it.
- If the request is clearly over budget, fail before Bifrost with an actionable Den error.

## Suggested first deliverables

1. shared budget report DTO,
2. model limit lookup during turn assembly,
3. request-level estimate on compiled payload,
4. runtime logs + `session_info` budget surface,
5. latest-session budget snapshot persistence for conversations web UI,
6. pre-inference over-budget guard.

## Verification

- unit tests for component attribution and reserve math,
- fixture tests for assembled request estimation,
- runtime tests for near-budget / over-budget behavior,
- calibration comparison logs where provider usage data is available.
