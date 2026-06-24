# Prompt Fragment Registry — Implementation Plan

**Status:** Proposed

**Decision source of truth:** [ADR-0046 — File-backed prompt fragments and compiled runtime prompts](../decisions/adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md)

**Architecture reference:** [Prompt Fragment Registry](../architecture/prompt-fragment-registry.md)

## Goal

Extract long-lived prompt and instruction prose from Rust source into file-backed prompt fragments while preserving Den's runtime invariant that turns consume compiled prompt material rather than raw source.

## Non-goals

- Replacing Den's existing dynamic turn-layer append model with one monolithic template.
- Allowing arbitrary turn-time templating in Bear Admin-authored content.
- Moving every generated string out of Rust; small structural runtime text may stay code-derived.

## Constraints

- Runtime-authored prompt content must remain compile-time-only.
- Repository-authored fragments may use controlled turn-time templating.
- The agent loop hot path must not read prompt files from disk or parse frontmatter.

## Phases

### Phase 1 — Registry foundation

- Add a prompt source tree for repository-authored fragments and bundle manifests.
- Build a startup-time prompt registry:
  - file discovery,
  - frontmatter parsing,
  - metadata validation,
  - MiniJinja template registration,
  - prompt source version hashing.
- Add typed schemas for fragments and bundles.

### Phase 2 — Compile-time templating contract

- Define compile-time vs turn-time variable surfaces.
- Enforce that runtime-authored fragments may use compile-time variables only.
- Add validation errors for unknown variables and illegal templating phases.

### Phase 3 — Managed prompt compilation integration

- Integrate repository fragments into managed prompt compilation.
- Update `compile_and_store_managed_config_for_bear` to normalize repo-authored and runtime-authored prompt sources into compiled per-role output.
- Preserve `bear_compiled_configs` as the hot-path contract.

### Phase 4 — Initial prompt migration

- Move shared role/policy/base instruction prose from Rust into repository fragments.
- Keep dynamic runtime supplements unchanged.
- Validate no behavior drift in compiled outputs where possible.

### Phase 5 — Runtime-owned prompt authoring boundaries

- Document which Bear Admin fields map to runtime-authored fragments.
- Ensure writes/reconciles trigger recompilation.
- Add operator-visible validation errors for bad MiniJinja usage in admin-entered content.

### Phase 6 — Turn-time repo fragments

- Add explicitly repo-only turn-time prompt fragments for controlled dynamic prose such as:
  - date/time reminders,
  - budget reminders,
  - other tightly-scoped runtime notices.
- Keep turn-time rendering bounded and append-oriented.

### Phase 7 — Cleanup and consolidation

- Remove migrated hardcoded prompt prose from Rust.
- Keep only structural/generated helpers in code.
- Reconcile duplicate prompt composition seams across service/runtime crates.

## Suggested crate usage

- `minijinja` — prompt rendering
- `serde` / `serde_yaml` — frontmatter and bundle manifests
- `include_dir` — embed repository prompt trees for production
- optional dev-only `notify` — reload support for prompt authoring iteration

Frontmatter splitting and assembly rules remain Den-owned logic.

## Verification strategy

- Snapshot tests for compiled per-role prompt output.
- Validation tests for allowed variable surfaces.
- Continuation/compaction-sensitive tests to ensure layer ordering remains intact.
- Drift checks that compiled prompt hashes change when repository prompt sources change.

## Migration watchpoints

- `den-service` and `den-runtime` currently mirror some Bear prompt compilation seams; avoid introducing source-of-truth ambiguity.
- Prompt-memory blocks, key memory projection, derived recall, and compaction must remain visibly separate layers after prompt extraction.
- Bear Admin-authored text must fail clearly on invalid template input rather than becoming a runtime surprise.
