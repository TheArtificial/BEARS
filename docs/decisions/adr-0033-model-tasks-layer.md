# ADR: Model Tasks Layer

**Status:** Proposed
**Date:** 2026-05-30
**Deciders:** Hans
**Related research:** [docs/research/model-tasks-strategy.md](../research/model-tasks-strategy.md)

## Context

Den requires model-driven functionality beyond the primary foreground agent loop. Examples include:

- embeddings for retrieval and vector search
- agent-scoped compaction summaries
- short constrained generation such as conversation and job titles
- labeling and classification
- structured extraction
- memory extraction
- reflection and evaluation
- reranking

These workloads differ materially from primary agent inference:

- many do not require tool use,
- many are more cost-sensitive,
- many have stricter output-contract requirements,
- some are latency-sensitive while others are naturally asynchronous,
- and several should prefer deterministic or strongly validated outputs over general-purpose fluency.

If these uses are implemented as ad hoc model or provider calls inside feature code, Den will accumulate inconsistent selection logic, duplicated prompt conventions, weak observability, and unclear fallback behavior.

Den already has or is planning a model registry. However, a registry alone is not enough. A registry answers what models exist. Den also needs a layer that answers which model should be used for a given task and how that task should be run safely.

## Decision

Den will introduce a first-class **model tasks** layer for non-primary model-driven work.

This layer will organize model invocation by explicit task class rather than by ad hoc caller-specific provider selection.

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

## Consequences

### Positive

- Den will have a coherent platform-level policy for model-driven work outside the foreground agent loop.
- Feature code can depend on stable task-oriented interfaces rather than direct provider calls.
- Cost, latency, validation, and fallback behavior can be tuned per task class.
- Observability becomes much clearer because model usage is categorized by purpose.
- Den can choose cheaper or more specialized models for utility tasks without weakening primary-agent quality.
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

- the precise API shape of the model tasks layer,
- the storage mechanism for task policies,
- exact provider selection defaults,
- or detailed queue and worker architecture for async task execution.

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

## Rationale

This decision reflects a practical architectural split:

- the **primary agent loop** is foreground interactive cognition,
- while **model tasks** are platform-level support capabilities with different policies.

The model tasks framing is preferable because it is concrete and operational. It makes it clear that Den is not merely cataloging abstract capabilities; it is executing distinct classes of work with distinct requirements.

This decision also complements the Den context compaction architecture. Compaction is one example of a model task that likely requires:

- a different model than the primary agent loop,
- a different prompt contract,
- different validation rules,
- and different fallback behavior.

## Implementation guidance

Implementation planning should assume:

- an explicit task taxonomy in code,
- task profiles that encode policy dimensions such as cost sensitivity, latency budget, output contract, and fallback behavior,
- routing built on top of the model registry,
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
