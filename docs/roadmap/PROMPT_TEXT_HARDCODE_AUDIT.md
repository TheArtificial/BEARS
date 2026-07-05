# Prompt Text Hardcode Audit

**Status:** Initial audit

**Related:** [ADR-0046](../decisions/adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md), [Prompt Fragment Registry](../architecture/prompt-fragment-registry.md), [Prompt Fragment Registry implementation plan](PROMPT_FRAGMENT_REGISTRY_IMPLEMENTATION_PLAN.md)

## Scope

This audit identifies hardcoded model-facing prompt or context text that should be migrated into prompt fragments or runtime-configured data where practical.

It intentionally excludes ordinary validation errors, UI labels, database SQL, test fixtures, and logs unless they are directly injected into model context.

## Principles

- Prompt text should not be hardcoded into source code. It should live in repository context fragments, or, when runtime-configured, in the database with explicit defaults alongside the fragment/configuration model.
- Prompts should not ask models to make choices when Den already has the state needed to choose. Use Rust or MiniJinja to render the applicable instruction before inference.

Current narrow exception:

- ACP adapter direct-tool descriptor objects currently carry short model-facing affordance hints for armature-local tools, such as preferring `fs_search_files` over `rg`/`grep` and `fs_replace_text` over targeted `sed` replacements.
- Keep this exception narrow and descriptor-scoped. It exists to improve tool discoverability in the absence of a canonical compiled-context home for adapter-local tool hints.
- Do not generalize this into freeform adapter-side prompt prose.

## High-priority migration candidates

### ACP direct tool/runtime prompt context

Files:

- `services/den/crates/den-acp/src/acp/prompt_context.rs`
- `services/den/crates/den-acp/src/acp/workflow_guidance.rs`

Examples:

- trusted ACP session mode guidance
- bearer-token identity guidance
- plan-mode/system-reminder guidance
- workflow state authority guidance
- runtime compaction prompt-context prose

Why migrate:

- These strings are injected directly into turn context.
- They mix static policy prose with runtime state formatting.
- They should become repository-owned `runtime` fragments with MiniJinja selecting deterministic branches before inference.

### Docket workboard prompt context

File:

- `services/den/crates/den-docket/src/model.rs`

Function:

- `render_workboard_prompt_context`

Why migrate:

- It injects a `<system-reminder>` with Docket/workboard behavioral instructions.
- The dynamic list formatting can remain code-owned, but the standing instruction prose should be a runtime fragment.

### Curate briefing prompt

File:

- `services/den/crates/den-runtime/src/native_runtime/profile_briefing.rs`

Function:

- `compose_curate_briefing_prompt`

Why migrate:

- It is a standalone LLM task prompt.
- The static instruction header should be a repository fragment; proposal list rendering can remain structural Rust output.

### Tool descriptor guidance and model-facing tool descriptions

Files:

- `services/den/crates/den-core/src/tools/tool_descriptor_guidance.rs`
- `services/den/crates/den-core/src/tools/descriptor/mod.rs`
- `services/den/crates/den-acp/src/acp/client_tool_advertisement.rs`

Why migrate carefully:

- Tool descriptions are model-facing prompt text.
- Much of the content is descriptor-owned structural text, so not every string should move immediately.
- The next step should be a descriptor-owned prompt-text registry or data table, not scattered fragments disconnected from tool metadata.

## Medium-priority migration candidates

### Managed stance/default prompt fallbacks

Files:

- `services/den/crates/den-service/src/bears/context_composition.rs`
- `services/den/crates/den-service/src/bears/templates.rs`

Why migrate:

- `den_baseline` and `pair` have started moving to repository fragments.
- `chat`, `curate`, `work`, `watch`, and template emphasis text remain hardcoded.
- These should move stance-by-stance into `services/den/prompts/fragments/stances/` and supporting bundle manifests.

### Session/environment context summaries

File:

- `services/den/crates/den-core/src/tools/environment/payloads.rs`

Why migrate carefully:

- Some fields are structured data and should stay code-owned.
- The prose values, such as context composition notes and agent-context summaries, are model-facing and should move or become descriptor-owned text.

## Low-priority / likely keep in code

- Validation errors and operator diagnostics.
- UI-facing labels and approval summaries unless they are also embedded in model-facing tool descriptors.
- Compact structural formatting of already-selected dynamic records.
- Test-only prompt snippets.

## Suggested next migration slices

1. Move ACP direct tool/runtime prompt context into `runtime` prompt fragments.
2. Move Docket workboard static instruction prose into a runtime prompt fragment, leaving the dynamic plan list in Rust.
3. Move Curate briefing header into a task/stance prompt fragment.
4. Continue stance migration after `pair`: `chat`, then `curate`, `work`, and `watch`.
5. Design a descriptor-owned data format for model-facing tool description/guidance text before migrating `den-core::tools` strings.
