# ADR: Model Tasks Layer

**Status:** Proposed
**Date:** 2026-05-30
**Deciders:** Hans
**Related research:** [docs/research/model-tasks-strategy.md](../research/model-tasks-strategy.md)
**Related decisions:** [ADR-0050: Agent Loop Control, Adaptive Budgets, and Runtime Checkpoints](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md)

## Context

Den requires task-classed model invocation across both the primary foreground agent loop and supporting platform work. Examples of supporting workloads include:

- embeddings for retrieval and vector search
- agent-scoped compaction summaries
- short constrained generation such as conversation and job titles
- labeling and classification
- structured extraction
- memory extraction
- reflection and evaluation
- reranking

Many supporting workloads differ materially from ordinary primary agent inference:

- many do not require tool use,
- many are more cost-sensitive,
- many have stricter output-contract requirements,
- some are latency-sensitive while others are naturally asynchronous,
- and several should prefer deterministic or strongly validated outputs over general-purpose fluency.

If these uses are implemented as ad hoc model or provider calls inside feature code, Den will accumulate inconsistent selection logic, duplicated prompt conventions, weak observability, and unclear fallback behavior.

The primary agent loop also needs routing policy inside the foreground path. A normal conversation turn, a planning step, a checkpoint, a pre-risk review, and a cheap grounding probe may all be `agent_primary` work, but they do not necessarily need the same model request profile.

Den already has or is planning a model registry. However, a registry alone is not enough. A registry answers what models exist. Den also needs a layer that answers which approved model should be used for a given task class or agent-loop step, and how that invocation should be run safely.

## Decision

Den will introduce a first-class **model tasks** layer for task-classed model invocation, including foreground agent-loop calls and platform support tasks.

This layer will organize model invocation by explicit task class rather than by ad hoc caller-specific provider selection.

The layer is not a replacement for the model registry. It routes among approved models. The registry describes available models and capabilities; a Bear/profile-level model library constrains which of those models a Bear may use; the model tasks layer resolves the model request profile for a Den-defined task class or agent-loop step.

The initial task taxonomy is:

- `agent_primary`
- `agent_compaction`
- `embedding`
- `rerank`
- `memory_extraction`
- `short_generation`
- `classification`
- `structured_extraction`
- `reflection`
- `evaluation`

`agent_compaction` remains intentionally agent-scoped. It should not be renamed to generic `compaction` unless Den later introduces non-agent compaction workloads with materially different inputs, contracts, or failure handling.

`agent_primary` is the foreground agent-loop class. It may carry step/context metadata rather than forcing every loop-control case into a new top-level task class. Initial step metadata should include:

- `ordinary_turn`
- `planning`
- `task_selection`
- `execution`
- `checkpoint`
- `pre_risk_review`
- `summarization`
- `cheap_probe`

ponytail: keep these as `agent_primary` step metadata until routing, validation, or queueing semantics differ enough to justify new top-level task classes.

This taxonomy is grounded in a mix of benchmark and product taxonomy patterns rather than arbitrary feature naming. In particular:

- embedding benchmarks such as MTEB separate retrieval-adjacent tasks like retrieval, reranking, classification, similarity, clustering, and summarization,
- retrieval benchmarks such as BEIR justify treating retrieval-related work as its own architectural concern,
- taxonomy-oriented evaluation frameworks such as HELM support organizing around use-case classes rather than only provider capability labels,
- and public product taxonomies such as Hugging Face Tasks show the practical value of distinct categories like summarization, feature extraction, ranking, and classification.

Reference set for this decision:

- MTEB: https://aclanthology.org/2023.eacl-main.148/
- BEIR: https://openreview.net/forum?id=wCu6T5xFjeJ
- HELM: https://crfm.stanford.edu/2022/11/17/helm.html
- Hugging Face Tasks: https://huggingface.co/tasks

The model tasks layer will:

1. route each task class to eligible models or providers,
2. apply task-specific execution policy,
3. validate outputs against task-specific contracts where appropriate,
4. handle retries and fallbacks,
5. support synchronous, asynchronous, and batch execution modes,
6. and emit observability and cost metadata by task class.

The model registry remains a distinct subsystem. It describes available models and capabilities. The model tasks layer decides how those models are used for Den-defined task classes.

For agent-loop calls, the model tasks layer resolves a provider-neutral **model request profile** from the task class, step metadata, Bear model library, model registry capabilities, loop-control policy, risk, budget, governance, and objective-orientation state. A profile may include:

- `model_ref`
- `reasoning_effort` or equivalent provider-neutral thinking-level request
- token/output limits
- sampling parameters
- execution mode
- validation/fallback policy

Reasoning effort is therefore one routing dimension, not a separate bypass around routing or loop-control enforcement.

Capable controller/checkpoint models may recommend bounded delegation of routine subtasks to weaker or cheaper approved models. Weaker models may request escalation. Runtime/model-task policy validates both directions. Model output must not name arbitrary provider IDs; routing targets must be symbolic model refs approved by the Bear model library and model registry. Delegation must be bounded by task scope, tool/file permissions, risk, and budget, and must be audited.

## Consequences

### Positive

- Den will have a coherent platform-level policy for task-classed model invocation, including foreground agent-loop steps and support work.
- Feature code can depend on stable task-oriented interfaces rather than direct provider calls.
- Cost, latency, validation, and fallback behavior can be tuned per task class.
- Observability becomes much clearer because model usage is categorized by purpose.
- Den can choose cheaper or more specialized models for utility tasks without weakening primary-agent quality.
- Den can route different foreground agent-loop steps, such as checkpoints or pre-risk reviews, without scattering model-specific conditionals through runtime code.
- More capable models can recommend bounded delegation to approved weaker/cheaper models while runtime policy keeps enforcement dominant.
- Den keeps a clear contract boundary between short constrained wording tasks and schema-oriented extraction tasks.
- Den keeps compaction policy explicitly tied to agent runtime needs rather than over-generalizing it as ordinary summarization.

### Costs and obligations

- Den must define and maintain task profiles and routing policy.
- Den must implement task-specific validation and fallback behavior.
- Den must decide which tasks are synchronous, asynchronous, or batch-oriented.
- Den must preserve separation between the model registry and the model tasks layer.
- Some existing or future feature code will need refactoring to route through the task layer.

### Non-goals

This ADR does not yet fix:

- the precise API shape of the model tasks layer or `ModelRequestProfile`,
- the storage mechanism for task policies,
- exact provider selection defaults,
- detailed queue and worker architecture for async task execution,
- or exact default routes for every `agent_primary` step.

Those implementation details should follow this decision.

## Alternatives considered

## 1. Ad hoc service-local provider calls

Let each service choose and invoke models directly.

Rejected because:

- it creates policy drift,
- duplicates prompt and fallback logic,
- weakens observability,
- and makes cost governance difficult.

## 2. Use only a model registry with no task layer

Expose a shared model registry and let callers select models directly from it.

Rejected because:

- a registry alone does not encode task-specific policy,
- it does not answer which model should be used for which Den workload,
- and it still pushes routing and safety decisions into feature code.

## 3. Treat all model use as part of the primary agent loop

Use the same inference path and default model choices for all model-driven behavior.

Rejected because:

- many model tasks have very different cost, latency, and validation requirements,
- some tasks do not require tool use or large context windows,
- and many utility tasks would be over-provisioned and over-priced if treated like primary agent inference.

## 4. Let loop control select provider models directly

Let the agent loop controller pick provider/model identifiers and provider-specific thinking parameters itself.

Rejected because:

- it duplicates the model tasks layer,
- bypasses Bear model-library constraints,
- mixes loop-health policy with provider routing,
- and makes delegation harder to audit and bound.

## Rationale

This decision reflects a practical architectural split:

- the **model registry** describes what exists and what capabilities are known,
- a Bear/profile **model library** constrains what a Bear may use,
- the **model tasks** layer decides which approved model request profile to use for a Den-defined task class or agent-loop step,
- and the **primary agent loop** consumes those profiles while runtime loop-control enforcement remains authoritative.

The model tasks framing is preferable because it is concrete and operational. It makes it clear that Den is not merely cataloging abstract capabilities; it is executing distinct classes of work with distinct requirements.

This decision also complements the Den context compaction architecture. Compaction is one example of a model task that likely requires:

- a different model than the primary agent loop,
- a different prompt contract,
- different validation rules,
- and different fallback behavior.

It also complements agent loop control. ADR-0050 classifies when the runtime needs an ordinary continuation, checkpoint, pre-risk review, grounding probe, or bounded delegation. This ADR provides the routing layer that turns that classified need into an approved model request profile.

## Implementation guidance

Implementation planning should assume:

- an explicit task taxonomy in code,
- `agent_primary` step metadata for foreground loop-control routing,
- task profiles that encode policy dimensions such as cost sensitivity, latency budget, output contract, and fallback behavior,
- routing built on top of the model registry,
- Bear/profile model-library constraints before any model is eligible,
- a provider-neutral `ModelRequestProfile` output concept,
- optional provider-neutral reasoning-effort metadata as one profile field,
- bounded delegation/escalation recommendations validated by runtime policy,
- validation hooks per task class,
- and observability keyed by task class.

The first task classes that should probably receive concrete implementations are:

- `agent_compaction`
- `embedding`
- `short_generation`
- `structured_extraction`

These classes are immediately useful and make the need for task-specific policy obvious.

## Status

Proposed.
