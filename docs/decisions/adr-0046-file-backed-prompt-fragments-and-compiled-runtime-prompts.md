# ADR-0046 — File-Backed Prompt Fragments and Compiled Runtime Prompts

**Status:** Accepted

## Context

Den assembles turn context from several layers: compiled profile prompt, key memory projection, derived recall, prompt-memory blocks, tool-surface guidance, and compaction/runtime supplements.

Much of the instruction text used across these layers is still hardcoded in Rust source. That has several problems:

- prompt prose is difficult to review and iterate on independently of code,
- runtime-authored prompt content from Bear Admin must coexist with repository-authored defaults,
- the hot path risks growing ad hoc template parsing/rendering if file-backed prompts are introduced without a clear contract,
- prompt source provenance and cache invalidation are not explicit enough for long-lived Den-native runtime behavior.

Den already has an important runtime invariant: the agent loop should read compiled prompt material from `bear_compiled_configs`, not recompose raw source text during each turn.

## Decision

Den will adopt a **hybrid prompt source model**:

1. **Repository-authored prompt fragments** live as **Markdown files with YAML frontmatter**.
2. **Runtime-authored prompt content** (for example Bear Admin-entered content) remains data-backed in Postgres.
3. Both source classes normalize into a common fragment model and are compiled into `bear_compiled_configs` for hot-path use.
4. **MiniJinja** is the prompt templating engine.

### Source classes

#### Repository-authored fragments

- Stored in Git as Markdown with YAML frontmatter.
- May be embedded into the binary for production use.
- May use:
  - **compile-time variables**,
  - and, for explicitly approved fragments, **turn-time variables**.

#### Runtime-authored fragments

- Stored in Den Postgres.
- May use **compile-time variables only**.
- Must be parsed, validated, and compiled before turns.
- Must not introduce turn-time templating into the hot path.

### Templating phases

Den will maintain two distinct variable surfaces:

- **Compile-time variables** — stable Bear/config/profile data available during managed prompt compilation.
- **Turn-time variables** — runtime/session/turn state such as current date, budget reminders, or channel/runtime supplements.

Only **repository-authored fragments** may use turn-time variables.

### Hot-path invariant

The runtime turn assembler must:

- read the compiled per-role prompt base from `bear_compiled_configs`,
- append dynamic turn-local layers separately,
- never read prompt files from disk during a turn,
- never parse frontmatter during a turn,
- never compile MiniJinja templates during a turn,
- never render runtime-authored database templates during a turn.

### Compilation model

Prompt compilation becomes a first-class pipeline:

- repository fragments are loaded and registered at startup,
- runtime-authored fragments are validated on write/reconcile,
- managed Bear prompt recomputation writes updated compiled output into `bear_compiled_configs`,
- the turn assembler consumes compiled output plus turn-local dynamic layers.

## Consequences

### Positive

- Prompt prose becomes reviewable and versioned outside Rust source.
- Bear Admin may continue to author prompt content without compromising turn performance.
- Prompt source provenance becomes explicit: repo-authored vs runtime-authored vs turn-local dynamic supplements.
- Cache invalidation can key on prompt source version and compiled config hashes instead of ad hoc string generation.
- Runtime behavior remains deterministic and explainable because turn-time templating is restricted to repository-owned fragments.

### Negative

- Den needs a prompt registry/loader, source validation, and compilation pipeline instead of ad hoc string composition.
- Prompt authoring rules become stricter: runtime-authored content cannot use arbitrary runtime/session variables.
- Migration takes sequencing work because current hardcoded text must be moved incrementally.

### Constraints

- Markdown is an authoring format, not an HTML-rendering contract for prompt compilation.
- YAML frontmatter must remain small, typed, and validation-friendly.
- MiniJinja use should stay interpolation-oriented, not become a general-purpose control-flow layer.
- Prompt text should not be hardcoded in Rust source. It should be represented as repository-authored fragments or runtime-configured data with explicit defaults.
- Prompt rendering should choose deterministic instructions before inference whenever Den already knows the relevant state; prompts should not ask the model to branch on known modes or policy inputs if code or MiniJinja can render the applicable instruction directly.

## Implementation notes

- Preferred supporting crates: `minijinja`, `serde`, `serde_yaml`, and an embedded file-tree helper such as `include_dir`.
- Frontmatter extraction and prompt assembly rules remain Den-owned logic.
- The architecture and rollout plan live in:
  - [`../architecture/prompt-fragment-registry.md`](../architecture/prompt-fragment-registry.md)
  - [`../roadmap/PROMPT_FRAGMENT_REGISTRY_IMPLEMENTATION_PLAN.md`](../roadmap/PROMPT_FRAGMENT_REGISTRY_IMPLEMENTATION_PLAN.md)
