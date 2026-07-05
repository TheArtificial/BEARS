# Runtime Error UX Policy

This policy defines how Den should project runtime failures for a named bear across three audiences:

- the model in a future turn;
- the human user in the armature/client UI;
- operators debugging Den, Bifrost, or armature logs.

The goal is to preserve continuity without leaking implementation detail into normal user copy.

## Principles

1. The user-facing error should explain the situation at a high level and say whether a retry/fresh turn is appropriate.
2. The model-facing continuity note should be explicit about what happened and what state can be trusted, but should not ask the model to diagnose infrastructure unless that is the user's task.
3. Diagnostic detail belongs in structured Den events/logs, armature stderr, or both, depending on where the failure was observed.
4. Do not put raw provider errors, stack traces, chunk framing errors, internal run-state strings, or full context JSON in normal user-visible error text.
5. Do not hide operational failures from future model turns. A later turn should know that the previous turn did not complete and should know whether recent tool results were preserved.

## Output Channels

### User-Facing Armature Message

The user-facing message should be short and product-level.

It should include:

- what happened in plain language;
- whether work/results were preserved when known;
- whether the user should retry, continue, or wait;
- the run id only when useful for support or logs.

It should not include:

- raw Den reason codes as the main phrase;
- provider JSON bodies;
- chunk parser details;
- Rust/internal enum names;
- full diagnostic context.

Example for budget/runtime interruption:

```text
Builder Bear stopped this turn after it ran too long. Recent tool results were preserved, but no final answer was delivered. Start a fresh turn to continue safely.
```

Example for restart/orphaned continuation:

```text
Builder Bear lost the active runtime state for this turn, likely because the Den service restarted. Recent persisted history is still available. Start a fresh turn to continue.
```

Example for provider stream transport failure:

```text
The model stream was interrupted before Builder Bear finished the turn. Recent tool results were preserved. Retrying or continuing in a fresh turn is safe.
```

### Model-Facing Continuity Note

Den should persist a hidden, model-visible operational outcome when a turn fails after work may have happened.

The note should tell the future model:

- the previous turn did not complete;
- whether recent tool results were preserved;
- whether it should continue from latest successful state;
- whether there is nothing for it to repair.

For infrastructure-only events where no model action is required, say that explicitly.

Example for budget exhaustion:

```text
Previous turn stopped after exhausting its wall-clock budget before delivering a final answer. Recent tool results were preserved. There is no infrastructure repair action for the model; continue from the latest successful state if the user asks to proceed.
```

Example for Den restart / missing in-memory continuation:

```text
Previous turn could not continue because in-memory runtime state was lost, likely due to a Den restart. Persisted conversation and tool results remain available, but no final answer was delivered. There is no repair action for the model; continue from persisted state in this fresh turn.
```

### Chat-Surface And History Marker

When Den sends a model-visible operational note, recoverable error note, task-focus warning, budget warning, or continuation warning, Den should also emit a concise human-visible marker to the active chat surface and user-visible conversation history.

This is a visibility parity rule: if the model is told that runtime state changed, the human should see a compact explanation that the runtime changed course.

This rule is surface-agnostic. It applies to ACP armatures, web chat, desktop/mobile chat, Slack/WhatsApp-style channels, and future Bear conversation surfaces. Armatures may expose richer event cards and diagnostics because they have a trusted work-surface boundary. Chat-only surfaces will usually project fewer event types, but they should still show concise markers for behavior-changing runtime notes.

The marker should:

- be one or two short sentences;
- say what changed at product level;
- avoid raw reason codes and implementation internals;
- be safe to show in normal conversation history;
- correlate to the detailed model-visible note through structured metadata, not by duplicating all detail.

The marker should not include:

- elapsed/limit counters unless needed for user action;
- provider JSON or raw stream/parser errors;
- hidden task-gate internals;
- full diagnostic context.

Examples:

```text
Builder Bear was warned about the number of tool calls for this turn.
```

```text
Builder Bear was kept on the current task focus and asked to continue the next incomplete item.
```

```text
Recent tool results were preserved after an interrupted continuation.
```

```text
Builder Bear stopped this turn after it ran too long. Start a fresh turn to continue safely.
```

Exceptions:

- Pure diagnostics that do not affect model behavior may remain stderr/log-only.
- Security-sensitive or secret-bearing diagnostics must not be mirrored into user history.
- Very frequent warnings should be coalesced or deduplicated so history does not become noisy.

### Operator Diagnostics

Detailed failure context should be structured and searchable.

Den logs should include:

- `run_id`, `session_id`, `bear_id`, `user_id`;
- normalized `reason`, `kind`, `subsystem`, and retryability;
- elapsed/limit values for budget failures;
- provider/model/routing metadata for stream failures;
- restart/orphan indicators for missing continuation state;
- server version/git SHA.

Armature stderr should include:

- the concise user-facing message;
- structured diagnostic context when present;
- Den server version/git SHA when available;
- any adapter-side stream diagnostics such as last event kind, replay counts, or missing first event.

Diagnostic context must not include secrets, full file contents, full prompts, bearer tokens, or unbounded provider payloads.

## Failure Taxonomy

| Class | User Copy | Model Continuity | Diagnostic Home |
|---|---|---|---|
| Turn budget exhausted | “stopped after it ran too long” | Continue only if user asks; latest tool results preserved | Den logs + hidden operational outcome |
| Den restart / missing continuation state | “lost active runtime state, likely restart” | No repair action; continue from persisted state | Den logs + armature stderr |
| Continuation watchdog timeout | “runtime did not resume after tool result” | Continue from latest successful state | Den logs + armature stderr context |
| Provider stream transport error | “model stream was interrupted” | Retry/continue safely; do not assume completion | Bifrost/Den logs + armature stderr |
| Tool execution failure | Tool-specific user message | Use tool result/error semantics | Tool result raw output + Den/armature logs |
| User cancellation | “request was cancelled” | Do not continue unless user asks | Armature/Den logs |

## Warning And Recovery Taxonomy

| Class | Chat/History Marker | Model Note | Diagnostic Home |
|---|---|---|---|
| Near-budget warning | “close to this turn's budget” | Prefer concise wrap-up or ask for fresh turn | Den event/log context budget details |
| Task-focus warning | “kept task focus active” | Continue next incomplete/unblocked item | Den task-focus state |
| Recoverable continuation warning | “recovered from interrupted continuation” | Recent results preserved; continue from latest state | Den logs + armature stderr if adapter-observed |
| Budget replenishment after mutation | “allowed a short verification pass” when user-relevant | Verification-oriented continuation only | Den budget state |

## Retryability Vocabulary

Use retryability to guide behavior, not to expose implementation details.

- `retryable=true`: user copy may say “retrying or continuing is safe.”
- `retryable=false`: user copy should say “start a fresh turn” or “narrow the request,” depending on class.
- `model_action=none`: model continuity note should say there is no infrastructure repair action.
- `model_action=continue_from_persisted_state`: model should proceed from latest persisted transcript/tool state if the user asks.
- `model_action=do_not_assume_completion`: model should not claim the previous task completed.

## Implementation Guidance

1. Den should normalize operational failures into structured outcomes before they reach armatures.
2. BearWire `run.failed` should carry concise user copy plus structured diagnostic context, not one overloaded message string.
3. The armature should render concise user copy and write diagnostic context to stderr.
4. Hidden operational outcome messages should remain model-visible but user-hidden.
5. Concise chat/history markers should accompany model-visible warnings and recovery notes when they affect behavior.
6. If an error is entirely infrastructure-level and no model action is needed, both Den's model note and the user copy should say so in different levels of detail.

## Current Gap To Close

The current `runtime_internal` budget-exhaustion message is too implementation-shaped for users:

```text
I stopped because this turn exhausted its wall-clock budget (elapsed=252985ms/limit=240000ms)...
```

Desired split:

- user message: concise high-level timeout copy;
- hidden model note: preserved results, no final answer, continue only if user asks;
- Den/armature diagnostics: elapsed, limit, run id, version, exact reason.
