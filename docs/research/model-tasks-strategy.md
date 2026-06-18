# Model Tasks Strategy

**Status:** Draft research note
**Date:** 2026-05-30
**Audience:** Hans, Builder Bear contributors, Den architecture work
**Related ADR:** [adr-0033-model-tasks-layer.md](../decisions/adr-0033-model-tasks-layer.md)

## Executive summary

Den will need model-driven functionality beyond the foreground agent loop itself. Examples include:

- embeddings for retrieval and vector search
- agent-scoped compaction summarization and related context-reduction work
- short constrained generation such as conversation and job titles
- labeling and classification
- memory extraction and distillation
- reflection and evaluation
- structured extraction and reranking

If these capabilities are implemented as ad hoc provider calls in individual services, Den will likely accumulate inconsistent model usage, duplicated prompt patterns, unclear cost behavior, weak observability, and brittle fallback behavior.

The recommendation in this note is:

> Den should introduce a first-class **model tasks** layer that treats non-agent-loop model usage as a platform capability with explicit task classes, routing policy, output contracts, validation, fallback behavior, and observability.

This layer should sit alongside the model registry. The registry describes what models are available. The model tasks layer decides which model to use for a given task and how to run that task safely.

## Problem statement

The foreground agent loop is only one kind of model use in Den. Many other features require model-driven behavior but have different constraints.

Examples include:

- generating embeddings for indexed content
- compacting conversation state into summaries
- naming conversations, jobs, or sessions
- classifying text into constrained categories
- extracting structured fields from freeform text
- generating memory candidates from runtime events
- evaluating summaries or outputs using an LLM judge

These workloads differ from primary agent inference in important ways:

- they often do not need tool use
- they may have stricter schema or contract requirements
- many should optimize for low cost over maximum reasoning quality
- some can be asynchronous or batched
- some need deterministic or strongly validated outputs

Without an explicit strategy, these tasks tend to be implemented as local conveniences. That creates platform drift.

## Why a dedicated layer is needed

A first-class model tasks layer helps Den avoid:

- scattered provider selection logic
- inconsistent retry and fallback behavior
- model calls that bypass observability and metering
- local prompt conventions that diverge over time
- weak guarantees around structured outputs
- accidental overuse of expensive primary-agent models for cheap utility tasks

It also creates room for deliberate policy decisions about:

- latency
- cost
- quality
- determinism
- validation
- sync versus async execution

## Distinguish agent inference from model tasks

Den should make an architectural distinction between:

## 1. Primary agent inference

This is the foreground model path used for:

- user conversation
- tool selection
- multi-step reasoning
- task execution in the live loop

This path is typically:

- latency-sensitive
- quality-sensitive
- tool-aware
- stateful with respect to live prompt assembly

## 2. Model tasks

These are supporting or secondary model workloads such as:

- embeddings
- agent-scoped compaction summaries
- short generation
- classification
- reranking
- reflection
- evaluation
- memory extraction
- structured extraction

These paths are often:

- more schema-constrained
- more cost-sensitive
- less interactive
- sometimes asynchronous
- easier to validate mechanically

This distinction is important because a single model-selection policy will not fit both classes of work.

## Initial model task taxonomy

A useful initial taxonomy for Den is:

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

The taxonomy should be treated as platform vocabulary, not an arbitrary string list created independently by callers.

## Requirements-category grounding

The taxonomy above is not just intuition. It lines up with how external evaluation and model ecosystems separate meaningful task classes.

### 1. Embedding ecosystems already distinguish retrieval, reranking, similarity, classification, clustering, and summarization

The strongest evidence comes from text-embedding benchmarks.

The Massive Text Embedding Benchmark (MTEB) organizes embedding evaluation into distinct task families including:

- retrieval
- reranking
- classification
- clustering
- pair classification
- semantic textual similarity
- summarization
- bitext mining

That matters for Den because it shows that tasks which may look superficially similar at the API level are meaningfully different in evaluation practice. A model that is strong for retrieval is not automatically strong for reranking. A model that works for semantic similarity is not automatically ideal for constrained classification. This supports keeping at least the following Den task classes distinct:

- `embedding`
- `rerank`
- `classification`

It also suggests Den should avoid collapsing all secondary model work into a generic `embedding_or_search` or `utility_llm` bucket.

### 2. Retrieval is a first-class problem, not just a subcase of generation

BEIR treats retrieval as its own benchmark family spanning heterogeneous retrieval tasks and domains. That is useful architectural evidence for Den.

In Den, retrieval-adjacent work can involve:

- vector generation for indexing
- recall-oriented search
- candidate reranking
- query transformation or query understanding

These should not all be represented by one generic task type. At minimum, retrieval infrastructure justifies keeping vector generation and reranking distinct. Den does not necessarily need a separate `retrieval_query_transform` class yet, but the taxonomy should leave room for it if retrieval flows become more sophisticated.

### 3. Holistic benchmark design supports taxonomy-by-use-case, not only taxonomy-by-model

HELM is useful here because it explicitly frames evaluation around scenarios or use cases rather than only around model internals. Its public framing emphasizes a taxonomy over scenarios and metrics, with tasks including things like question answering, information retrieval, summarization, and classification.

This supports a Den architecture principle:

> model tasks should be defined by the work the system is asking a model to do, plus the contract and evaluation criteria for that work, rather than by raw provider capability names alone.

That is one reason `agent_compaction` deserves to remain separate from the more general idea of summarization. In Den, compaction is not just "produce a shorter text." It is a runtime state-management task with specific retention goals, schema expectations, and failure consequences.

### 4. Product ecosystems use pragmatic task categories that map well to platform APIs

The Hugging Face task taxonomy is not a research benchmark, but it is useful evidence for practical API design. It separates categories such as:

- summarization
- feature extraction
- question answering
- text classification
- sentence similarity
- text ranking
- text retrieval
- zero-shot classification

This reinforces the idea that externally visible platform interfaces tend to work better when they expose stable task categories rather than one undifferentiated generation endpoint. For Den, that supports task-oriented routing surfaces such as `classification`, `structured_extraction`, `embedding`, and `rerank` instead of asking every caller to decide prompts and providers directly.

## Implications for Den taxonomy design

The desk-research signal suggests a few concrete decisions.

### Keep these classes distinct now

- `embedding`: vector production with strict provider-family and dimension consistency requirements
- `rerank`: candidate scoring and ordering, often on the synchronous path of retrieval
- `classification`: constrained label assignment with enum validation and deterministic output preferences
- `structured_extraction`: schema-shaped extraction where field validity matters more than prose quality
- `short_generation`: short constrained text generation where output length, style, and cheap fallback matter more than deep reasoning
- `agent_compaction`: runtime context-state reduction with compaction-specific quality and validation criteria

### Keep these separate from the primary agent loop

- `agent_primary` should remain distinct from all support-task classes
- `agent_compaction` should remain distinct from `agent_primary` even when both use language generation

This is especially important because benchmark taxonomies and production APIs both show that task differences tend to dominate model-selection policy.

### Keep these as pragmatic platform classes, even if they are not benchmark categories

Some Den task classes are product-motivated rather than benchmark-native.

- `short_generation`
- `memory_extraction`
- `reflection`
- `evaluation`

These still make sense because they correspond to distinct runtime contracts and operational policies.

For example:

- `short_generation` covers low-cost, highly constrained generation such as conversation titles, short labels, and similar naming tasks, and often allows heuristic fallback
- `memory_extraction` prioritizes precision, reviewability, and structured candidate output
- `reflection` is synthesis or critique over prior work, often nearline or asynchronous
- `evaluation` is rubric-based judging or scoring, often batchable and provenance-sensitive

Even where benchmarks do not expose these exact names, Den should keep them separate if routing, validation, and observability policy differ.

### Do not overfit the first taxonomy

The research signal also suggests restraint.

Den should not create many speculative task classes before there is runtime need. For example, it may eventually make sense to add:

- `query_rewrite`
- `similarity_scoring`
- `safety_classification`
- `translation`

But those should probably emerge when Den has clear product requirements or materially different execution policy, not simply because an external benchmark has such a category.

## Suggested task profiles

Each model task class should have an explicit profile.

## `agent_primary`

Purpose:

- live foreground agent reasoning and response generation

Typical characteristics:

- highest quality requirement
- tool-aware
- latency-sensitive
- medium to high cost tolerance compared to utility tasks

## `agent_compaction`

Purpose:

- compaction summaries and related runtime context-state reduction for agent execution

Typical characteristics:

- text-in text-out
- no tool use required
- optimize for retention per token, not eloquence
- usually cheaper than the primary agent model
- may require validation against required sections or fields
- semantically tied to agent runtime state, not generic user-facing summarization

## `embedding`

Purpose:

- generate vectors for indexed retrieval and similarity search

Typical characteristics:

- requires provider and dimension consistency
- strongly cost-sensitive at scale
- frequently batchable or asynchronous
- output contract is numeric vector shape, not natural language quality

## `rerank`

Purpose:

- improve retrieval quality by scoring candidate results

Typical characteristics:

- ranking quality matters more than generative style
- often latency-sensitive within retrieval flows
- output contract is scored ordering or relevance score

## `memory_extraction`

Purpose:

- extract durable memory candidates from runtime material

Typical characteristics:

- favors precision over creativity
- likely schema-constrained
- may run inline or nearline depending on flow
- should support review-friendly structured output

## `short_generation`

Purpose:

- generate short constrained text such as conversation titles, job names, labels, and similar naming-oriented outputs

Typical characteristics:

- should be very cheap
- often can be asynchronous
- should have strong length and style constraints
- should have heuristic fallback when the model is unavailable
- differs from `structured_extraction` because success is judged primarily by concise wording and format constraints rather than schema completeness

## `classification`

Purpose:

- assign constrained labels, route decisions, or finite states

Typical characteristics:

- values determinism and schema validity
- often cheaper than general generation
- requires enum or constrained output validation

## `structured_extraction`

Purpose:

- extract structured fields or objects from natural language or runtime traces

Typical characteristics:

- schema-first
- validation-heavy
- often suitable for retries with tighter prompts
- differs from `short_generation` because the primary contract is field correctness and schema validity, not concise phrasing

## `reflection`

Purpose:

- critique, synthesis, or self-review of prior outputs or state

Typical characteristics:

- moderate quality requirement
- often asynchronous or nearline
- may write candidate artifacts rather than user-facing answers

## `evaluation`

Purpose:

- judge outputs, summaries, or behaviors against a rubric

Typical characteristics:

- often offline or batchable
- quality and rubric fidelity matter more than latency
- should preserve provenance and score metadata

## Model tasks layer responsibilities

The model tasks layer should be responsible for:

- mapping task class to eligible provider or model choices
- applying execution policy and defaults
- validating outputs against task-specific contracts
- handling retries and fallbacks
- recording observability and cost metadata
- separating sync, async, and batch execution modes
- exposing stable task-oriented APIs to application code

This layer should not merely proxy raw provider APIs.

## Relationship to the model registry

The model registry and the model tasks layer solve different problems.

### Model registry

Answers questions like:

- what models exist?
- which providers are configured?
- what capabilities does each model advertise?
- what metadata is known about cost, limits, or modality?

### Model tasks layer

Answers questions like:

- which model should be used for this task class?
- what output contract applies?
- what validation should be performed?
- what fallback should happen if the preferred model fails?
- should this task run synchronously, asynchronously, or in batch?

Den should keep these concepts separate.

## Execution policy dimensions

Each task class should define policy along dimensions such as:

- latency budget
- cost sensitivity
- quality floor
- determinism preference
- modality
- output schema or contract
- sync versus async eligibility
- fallback behavior
- caching eligibility
- observability tags

## Example policy patterns

### Short generation

Recommended pattern:

- cheap model by default
- strong length and style constraints
- best-effort asynchronous generation allowed
- heuristic fallback if unavailable
- suitable for titles, short labels, and similar naming tasks

### Embeddings

Recommended pattern:

- dedicated embedding model
- strict vector shape and provider consistency
- batch support
- reindex jobs separated from user-interactive flows

### Compaction

Recommended pattern:

- keep the task class explicitly separate from primary agent inference
- keep the name `agent_compaction` rather than collapsing it to generic `compaction` unless Den later introduces non-agent compaction workloads with materially different contracts
- cheaper summarization-capable model preferred by default
- structured summary contract
- escalation or retry path if validation fails

### Classification and structured extraction

Recommended pattern:

- schema-first prompts and validation
- deterministic or constrained output settings where available
- enum and JSON contract enforcement

## Validation and fallback

Many model tasks benefit from strict validation. Examples:

- embeddings: verify vector dimension and provider-family compatibility
- short generation: verify length, emptiness, and formatting rules
- classification: verify enum membership
- structured extraction: verify schema validity
- agent compaction: verify required sections or fields
- evaluation: verify rubric output shape and provenance fields

Fallback behavior should also be task-specific. Examples:

- short generation failure -> heuristic title or label
- agent compaction failure -> retry with stronger model or larger budget
- evaluation unavailable -> mark deferred instead of blocking user flow
- embedding provider unavailable -> queue retry or fail closed depending on operation type

## Observability and governance

Model tasks should be metered and observable separately from the primary agent loop.

At minimum, Den should record:

- task class
- provider
- model
- caller or service
- latency
- input and output size where appropriate
- retry count
- validation result
- estimated cost
- work surface, conversation, or job reference where applicable

This matters because background model tasks can become a major share of platform cost without being obvious in foreground user metrics.

## Sync, async, and batch execution

Den should not assume all model tasks are synchronous.

A useful classification is:

### Foreground synchronous

Needed immediately for the user flow:

- primary agent response
- inline compaction when context is over budget
- structured extraction needed for the next step

### Background nearline

Useful soon, but not blocking:

- short generation for titles or labels
- memory candidate extraction
- labels and tags
- recap generation

### Offline or batch

Can be delayed:

- embeddings backfill
- eval runs
- reindexing
- summary refresh jobs
- large-scale distillation

The model tasks layer should make these modes explicit.

## Interaction with the context compaction architecture

The model tasks strategy is closely related to context compaction.

Compaction is a strong example of why Den needs this layer:

- compaction likely should not use the same model as the primary agent loop
- compaction needs a different prompt contract
- compaction has different cost and latency tradeoffs
- compaction may require validation and escalation behavior

This supports the broader principle that model use in Den should be organized by task class rather than by raw provider calls in feature code.

## Recommendations

Den should adopt the following principles:

1. Treat model-driven support work as a first-class **model tasks** layer.
2. Use explicit task classes rather than ad hoc service-local provider calls.
3. Keep `agent_compaction` explicitly agent-scoped unless Den later introduces non-agent compaction workloads that need a different contract.
4. Separate model registry concerns from model task routing and policy concerns.
5. Validate outputs against task-appropriate contracts wherever possible.
6. Define task-specific fallback and retry behavior.
7. Meter and observe model task usage independently from the primary agent loop.
8. Support synchronous, asynchronous, and batch task execution modes.

## References and citation notes

The taxonomy guidance in this note is informed by the following external sources and publicly visible taxonomy patterns:

- **MTEB: Massive Text Embedding Benchmark** — supports separating embedding-related task families such as retrieval, reranking, classification, clustering, semantic similarity, and summarization
  - ACL Anthology: https://aclanthology.org/2023.eacl-main.148/
  - Project repo: https://github.com/embeddings-benchmark/mteb/
- **BEIR: A Heterogeneous Benchmark for Zero-shot Evaluation of Information Retrieval Models** — supports treating retrieval as its own architectural concern rather than a generic generation subcase
  - OpenReview: https://openreview.net/forum?id=wCu6T5xFjeJ
  - arXiv: https://arxiv.org/abs/2104.08663
- **HELM: Holistic Evaluation of Language Models** — supports organizing around use-case and scenario taxonomy rather than only provider capability labels
  - Stanford CRFM overview: https://crfm.stanford.edu/2022/11/17/helm.html
  - arXiv: https://arxiv.org/abs/2211.09110
- **Hugging Face task taxonomy** — useful as evidence of pragmatic product-facing task segmentation in public model ecosystems
  - Tasks index: https://huggingface.co/tasks
  - Text ranking: https://huggingface.co/tasks/text-ranking
  - Summarization: https://huggingface.co/tasks/summarization
  - Feature extraction: https://huggingface.co/tasks/feature-extraction
  - Text classification: https://huggingface.co/tasks/text-classification

These references are being used here to justify task-category boundaries, not to claim that Den should mirror any external taxonomy exactly.

## Proposed next steps

- Adopt an ADR for the model tasks layer.
- Define the initial task taxonomy in code and docs.
- Add task-oriented routing policy on top of the model registry.
- Define output contracts for the first few classes:
  - agent compaction
  - embeddings
  - short generation
  - structured extraction
- Add observability tags and usage accounting by model task class.

## Open questions

Important implementation questions still include:

- whether task profiles should be static config, database-backed policy, or both
- how much per-Bear or per-role override should be allowed
- whether some tasks should support provider-specific implementations rather than generic prompting
- how caching and deduplication should interact with task class semantics
- how task policy should be exposed to runtime services and workers
