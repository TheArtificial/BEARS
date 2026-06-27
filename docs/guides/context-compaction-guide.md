# Context Compaction Guide

This guide explains how Den context compaction works at a practical level and how to think about it when building, testing, documenting, or later describing Bear Den behavior externally.

It complements:

- [ADR-0032: Den Context Compaction Architecture](../decisions/adr-0032-den-context-compaction-architecture.md)
- [Den Context Compaction Contract](../architecture/den-context-compaction-contract.md)
- [Den Context Compaction Schema Direction](../architecture/den-context-compaction-schema.md)
- [Den Context Compaction Implementation Plan](../roadmap/DEN_CONTEXT_COMPACTION_IMPLEMENTATION_PLAN.md)
- [Den-Native Runtime Plan](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md)

## Why compaction exists

Bear Den supports long-running sessions that can include:

- user messages,
- assistant replies,
- tool calls and tool results,
- approval requests and approval decisions,
- workplan and workflow state,
- artifact references,
- and other runtime events.

If Den simply keeps replaying the entire transcript forever, context windows eventually become too large, prompts become slower and more expensive, and continuation quality becomes fragile.

Context compaction is Den's answer to that problem.

The key idea is:

- keep the **canonical transcript**,
- preserve the **active working set** directly,
- and convert older eligible history into **derived compacted state** that is still usable for continuation.

This is not the same thing as deleting history, and it is not the same thing as promoting content into durable memory.

## The basic mental model

Den treats long-lived runtime context as three distinct layers:

### 1. Canonical transcript

The canonical transcript is the source-of-truth session history.

It contains the actual ordered runtime record: user turns, assistant replies, tool events, approval events, workflow changes, and related system/runtime events.

This is the durable history of what happened.

### 2. Active working set

The active working set is the portion of context that must remain directly present for safe continuation.

Examples include:

- current instructions,
- active workflow/workplan state,
- unresolved tool interactions,
- unresolved approval requests,
- recent constraints and decisions,
- active artifact references,
- and recent dialogue needed for the next step.

This is the part Den should not summarize away prematurely.

### 3. Derived compacted state

Derived compacted state is prompt-ready context created from older transcript history.

Examples include:

- iterative summaries,
- collapsed tool bundles,
- structured summaries of older workflow spans.

This state helps Den continue coherently without replaying every historical event in full.

## What compaction is not

Compaction is **not**:

- durable memory promotion,
- transcript deletion,
- a lossy sliding-window policy by default,
- or a hidden mutation of history.

Den's architecture intentionally keeps these concerns separate.

## Why Den uses semantic groups

Den does not want to compact arbitrary individual messages in isolation.

Instead, it groups runtime history into **semantic groups** such as:

- user turns,
- assistant replies,
- tool interaction bundles,
- approval interaction bundles,
- workflow/plan updates,
- artifact/reference updates,
- prior compaction artifacts.

This matters because the unit of continuation is often larger than a single message.

For example:

- a tool call and its result belong together,
- an approval request and its decision belong together,
- a workflow update may need to stay intact,
- an artifact reference may be more important than the surrounding prose.

Compaction becomes safer when it operates on these grouped units instead of raw message lines.

## Protected spans and safety floors

Some runtime history must not be compacted across.

Den calls these boundaries **safety floors** or **protected spans**.

Important examples:

- unresolved tool interactions,
- unresolved approval interactions,
- active workflow/workplan state,
- recent decisions and constraints still governing the session,
- artifact references that are still needed for continuation.

This is especially important for `pair`, where ongoing tool use and approvals make context safety stricter than simple chat.

## Prompt assembly after compaction

After compaction, Den should still assemble prompt context explicitly from separate layers:

- instructions and role/runtime policy,
- active workflow/workplan state,
- recent uncompacted semantic groups,
- derived compacted state,
- and any separately governed memory or retrieval inputs.

Under [ADR-0046](../decisions/adr-0046-file-backed-prompt-fragments-and-compiled-runtime-prompts.md), the instruction/policy layer may come from repository-authored prompt fragments and runtime-authored compile-time prompt content, but compaction must still operate over a visibly layered assembly rather than a single opaque template blob.

This is a major architectural point.

Den should not flatten everything into an indistinguishable blob if that would erase provenance, reduce explainability, or make recovery/debugging difficult.

## Continuation evaluation scaffolding

One of the most important implementation ideas is **continuation evaluation**.

This does **not** mean model-judged evaluation in the first phase.

It means building regression tests that ask:

> After compaction, does Den still preserve the information needed to continue correctly?

### What continuation evaluation should test

For representative sessions, tests should verify preservation of:

- active user goals,
- important constraints,
- decisions already made,
- artifact references,
- workflow/workplan continuity,
- unresolved follow-ups,
- unresolved tool/approval awareness,
- and next-step continuity.

### What first-phase scaffolding looks like

The first version should be deterministic and CI-friendly.

It should include:

1. **Representative session fixtures**
   - tool-heavy `pair` sessions
   - long `chat` sessions
   - workflow/plan-heavy sessions

2. **Expected continuity assertions**
   - what goal must still be present
   - what constraint must still be present
   - what artifact/workflow reference must still be present
   - what unresolved state must still be visible

3. **Pre/post compaction comparisons**
   - compare the prompt-assembly view before compaction and after compaction
   - ensure the compacted form still preserves the critical continuity signals

4. **Probe helpers**
   - small deterministic helper assertions such as:
     - preserve constraint
     - preserve artifact ref
     - preserve workflow state
     - do not hide unresolved approval

### Why this matters

Without continuation evaluation, compaction can look successful while actually degrading the runtime.

For example, a summary might save tokens but accidentally drop:

- the user's real goal,
- a key constraint,
- the file being edited,
- or the fact that an approval is still unresolved.

That would be a regression even if context usage improved.

## Why Den owns compaction

Context compaction is core Den runtime behavior—not an adapter or sidecar concern. Den must own:

- transcript behavior,
- prompt continuity behavior,
- compaction semantics,
- operator visibility,
- and regression safety.

Compaction is therefore part of the core runtime surface, not a cosmetic optimization.

## Why this matters for future storytelling

This guide is also useful groundwork for later external explanation.

A marketing-style explanation of Bear Den should eventually be able to say something like:

- Bear Den remembers the right things without replaying everything forever.
- It preserves active work, tools, approvals, and decisions while compressing older history intelligently.
- It keeps durable memory, active session context, and compacted session summaries as distinct layers.
- It is built to stay explainable and testable, not just smaller.

Those claims should be backed by the architecture and regression tests described here.

## Current implementation status

As of the `test` branch (commit `79b1ee62` and follow-on work):

- Phases A–H are implemented in `den-runtime`: semantic grouping from transcript, compaction service/events, artifact persistence, compaction-aware assembly with transcript cutoff, token-pressure triggers, deterministic summarization, continuation eval probes, manual compaction, and async post-turn compaction via the `context_compact` worker.
- Reactive overflow recovery classifies LLM context-length errors, runs emergency compaction with `ModelSafetyMargin`, reassembles the prompt, retries the LLM step once, and surfaces ACP outcomes as `CompactedRetry` when recovery succeeds.
- Operator visibility is available from `/bear/{slug}/conversations` and `/bear/{slug}/conversations/{conversation_id}`. The list view shows whether compaction events exist; the detail view shows event history, trigger/policy provenance, source spans, diagnostics, and persisted artifact JSON.

Still open: LLM-backed summarization and `archive_harvest` mining of compaction artifacts into memory proposals.

## Reactive overflow recovery

When the LLM provider rejects a prompt for exceeding its context window, Den can recover automatically **if `COMPACTION_MODE=active`**.

1. **Detect** — the agent step classifies handshake errors whose message matches common context-length patterns (for example `context_length_exceeded`, "maximum context", "too many tokens").
2. **Compact** — sync emergency compaction runs with trigger `ModelSafetyMargin`, perserving protected spans and producing/updating an iterative summary artifact plus a transcript sequence cutoff.
3. **Reassemble** — the system prompt compaction block and transcript messages are rebuilt from persisted state; in-session tool results appended during the current step are preserved.
4. **Retry once** — the LLM handshake is retried with the smaller prompt. Only one overflow retry is attempted per agent-loop session step.
5. **Outcome** — on ACP/`pair`, a successful retry emits terminal turn result `status=recovered`, `reason=compacted_retry`. Web chat benefits from the same retry path but does not persist ACP-style turn outcomes.

In `observe` mode, compaction events may still be recorded for telemetry, but the prompt is not cut and overflow retry is skipped — use `active` mode where emergency shrink is required.

## Rollout Checklist

Use this checklist when enabling Den-owned compaction on a new environment or surface.

1. **Confirm prerequisites**
   - Database migrations are applied through `conversation_compaction_artifacts` and `runtime_compaction_events`.
   - Canonical conversation persistence is active for the target surface.
   - Operators can access `/bear/{slug}/conversations` for the target Bear.
   - Focused tests pass: `cargo test --manifest-path services/den/Cargo.toml -p den-runtime compaction` and `cargo test --manifest-path services/den/Cargo.toml -p den-web source_templates_parse`.

2. **Start in observe mode**
   - Set `COMPACTION_MODE=observe`.
   - Keep `COMPACTION_TIMING=async` unless debugging a synchronous turn-start path.
   - Run representative `pair` and/or `chat` sessions long enough to trigger event recording.
   - Inspect the conversation detail page for event trigger, policy version, selected source spans, diagnostics, and artifact JSON.

3. **Compare continuation signals**
   - Confirm recent uncompacted transcript still contains active tool/approval spans.
   - Confirm the latest artifact preserves active goals, constraints, decisions, artifact refs, workflow state, and unresolved follow-ups.
   - Confirm `Skipped` events explain safety-floor or eligibility reasons rather than silently dropping context.

4. **Enable active mode on a bounded surface**
   - Set `COMPACTION_MODE=active` for an internal/test Bear or a limited `pair`/`chat` population first.
   - Leave `COMPACTION_TIMING=async` for normal rollout; use sync only for targeted debugging or emergency behavior validation.
   - Exercise manual compaction and a long session that naturally crosses token/group pressure.
   - For overflow recovery, verify an ACP/`pair` recovered turn reports `status=recovered`, `reason=compacted_retry` when the retry succeeds.

5. **Production gate**
   - No active protected span is compacted across in operator-visible source ranges.
   - Continuation probes or manual continuation checks preserve active constraints and next-step state.
   - Operators can find the latest artifact and event history for impacted sessions.
   - Disabling active mode has been tested in the target environment.

## Rollback Runbook

Compaction rollback is execution rollback, not transcript rollback. Canonical transcript rows remain the source of truth.

1. **Disable authoritative compaction**
   - Set `COMPACTION_MODE=observe` to keep telemetry without cutting prompt history.
   - Set `COMPACTION_MODE=off` if event generation itself is suspected of causing trouble.
   - Restart the affected Den service after changing environment configuration.

2. **Ignore derived artifacts**
   - Do not delete `conversation_compaction_artifacts` during ordinary rollback.
   - In `observe` or `off`, prompt assembly must continue from canonical transcript and active runtime inputs rather than making new artifacts authoritative.
   - Use the operator conversation detail page to identify which artifact/policy version was active when the issue happened.

3. **Recover the session**
   - Retry the turn after disabling active compaction.
   - If the issue involved unresolved approvals or tools, resolve those through the appropriate operator recovery path; compaction is not an approval repair mechanism.
   - If a session remains too large without active compaction, start a new session with a human-authored handoff from the latest trusted transcript/artifact view.

4. **Rebuild artifacts when safe**
   - Keep existing artifacts for audit until an explicit cleanup/backfill job exists.
   - When a future policy version changes summary semantics, rebuild derived artifacts from canonical transcript rows rather than mutating transcript history.
   - Validate rebuilt artifacts with continuation checks before re-enabling active mode.
