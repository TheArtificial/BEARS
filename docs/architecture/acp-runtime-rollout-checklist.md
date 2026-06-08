# ACP Runtime Migration Rollout Checklist

> **Direction changed (2026-06).** Steps that assume Letta JSON stream translation (`letta_runtime_stream_parser.rs`) are superseded; the target emits Den-native semantic events directly. Canonical target: [Den-Native Runtime](den-native-runtime.md) ([migration plan](../roadmap/DEN_NATIVE_RUNTIME_PLAN.md)).

Use this checklist during deployment and smoke testing for the Den-owned ACP runtime migration.

## What changed

The ACP runtime path now has clearer Den-owned seams:

- Letta JSON stream payloads are translated into `RuntimeSemanticEvent`
- ACP/BearWire output projection is separated from provider parsing
- raw provider fallback is now explicit as `UntranslatedProviderEvent`
- start-turn conversation creation now goes through the runtime conversation contract
- continuation SSE bytes are adapted through a reusable runtime byte-stream -> event-stream helper
- untranslated provider fallback classes are now counted in ACP stream diagnostics

## Primary rollout signals to inspect

### 1. ACP stream summary logs

Look for the ACP stream summary log emitted from stream diagnostics.

Important fields:

- `unmapped_events`
- `untranslated_event_classes`
- `native_message_types`
- `native_event_types`
- `adapter_event_types`
- `unmapped_event_samples`
- `run_ids`

Healthy expectation:

- `unmapped_events` remains low
- `untranslated_event_classes` is empty or contains only a small number of low-frequency fallback classes
- expected adapter events appear for assistant/status/tool/terminal flows

### 2. Empty-turn error payloads

If a turn ends without visible output, inspect the emitted ACP error context.

Look for:

- `unmapped_event_samples`
- `untranslated_event_classes`
- `run_ids`

This is the fastest way to understand whether a provider payload class needs promotion into a first-class semantic event.

### 3. Continuation/resume flows

Smoke test:

- tool result continuation
- approval continuation
- paused turn resumption
- terminal completion after continuation

Healthy expectation:

- continuation emits semantic events and a clean terminal completion
- no unexpected empty-turn errors
- no repeated untranslated fallback class growth during normal continuation flows

## Most likely fallback classes to watch

These are the most likely categories to require follow-up semantic promotion if they appear frequently:

- `message_type:tool_return_message`
- provider approval lifecycle messages
- provider run-progress/status variants not yet normalized
- provider-specific terminal/error variants
- any `type:*` class that repeatedly appears in successful turns

## Recommended tester workflow

1. Run a normal ACP prompt turn
2. Run a tool-using turn
3. Run an approval-gated turn if available
4. Resume a continued turn
5. Check logs for:
   - `untranslated_event_classes`
   - `unmapped_event_samples`
6. If any class repeats, record:
   - class name
   - sample payload
   - whether the user-visible behavior was correct, degraded, or broken

## How to interpret results

### Good / no-action result

- user-visible behavior is correct
- `untranslated_event_classes` is empty or very low volume
- no empty-turn errors

### Medium-priority follow-up

- user-visible behavior is correct
- one untranslated class repeats often

Action:

- add parser coverage for that class
- promote it to `RuntimeSemanticEvent`
- remove it from fallback handling

### High-priority follow-up

- missing visible output
- empty-turn errors appear
- approval/tool lifecycle becomes inconsistent
- terminal events are absent or duplicated

Action:

- inspect `unmapped_event_samples`
- inspect `untranslated_event_classes`
- patch parser/projection before broader rollout

## Suggested next patch targets after rollout data

Promote frequent fallback classes in this order:

1. tool return/result payloads
2. approval lifecycle payloads
3. progress/status payloads
4. terminal/error payload variants

## Relevant code locations

- `services/den/src/core/letta_runtime_stream_parser.rs`
- `services/den/src/core/runtime_contracts.rs`
- `services/den/src/core/runtime_bearwire_projection.rs`
- `services/den/src/core/acp_turn_runner.rs`
- `services/den/src/api/acp/stream/mapping.rs`
- `services/den/src/api/acp/stream/support.rs`

## Existing focused tests

- semantic parser tests
- semantic -> ACP seed projection tests
- semantic -> BearWire projection tests
- continuation byte-stream adapter test
- untranslated event observability tests

This checklist is intended to make rollout review fast: identify frequent fallback classes, promote them into semantic events, and keep shrinking `UntranslatedProviderEvent` toward true unknown-only usage.
