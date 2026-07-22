# BearWire run stream completion

This policy applies when the server-side runtime stream for a BearWire run ends.
It distinguishes the lifetime of the stream from the lifetime of the run: an EOF is
not, by itself, a terminal run outcome.

| Observed boundary | EOF policy | Durable run/session result |
| --- | --- | --- |
| `turn.completed`, `turn.failed`, `turn.cancelled`, or `error` | Finish normally. | Persist the matching terminal run state. |
| `run.paused` or answerable `client.waiting` | Stop consuming this stream; do not fail it. | Preserve the run and its obligation as resumable. |
| EOF after non-terminal activity, including tool activity | Make one bounded reconciliation attempt using the persisted run/event state. | On success, continue from the persisted cursor without replaying tool calls or assistant text. On failure, retain an interrupted, retryable run and its conversation. |
| EOF before activity | Make one bounded reconciliation attempt when a durable run exists. | If it cannot reconcile, retain an interrupted, retryable run; do not terminate the conversation session. |

## EOF invariants

- A terminal runtime event is the only normal completion signal.
- A client-wait boundary is a successful pause, not a failed completion.
- Reconciliation is bounded to one attempt per stream EOF and reads persisted state before resuming, so it cannot re-execute a completed tool call or duplicate already persisted assistant output.
- Failure to reconcile is an operational interruption. It may end the active stream, but it must not delete or terminate the conversation session.

## Telemetry vocabulary

Use these terms independently:

- **user-visible event**: any event or persisted marker rendered to the user, including progress and waiting state.
- **assistant content**: an assistant text delta or persisted assistant message.
- **final assistant message**: the completed assistant response associated with a terminal run outcome.
- **terminal run event**: `turn.completed`, `turn.failed`, `turn.cancelled`, or `error`.
- **client-wait boundary**: `run.paused` or a tool request that transfers control to the client.

Do not infer assistant content from a user-visible event. Diagnostics for incomplete streams must report each signal separately, plus final observed boundary, stream cursor, outstanding tool-call IDs/states, and reconciliation outcome. Tool arguments and credentials are excluded from those diagnostics.

## Release gate

Use existing telemetry and deployment controls. Stage this change and halt/roll back if incomplete-stream frequency, failed reconciliation frequency, forced session termination, or user-visible internal-error exposure regresses. Verify during rollout that pending tool calls remain answerable and resumed runs do not duplicate output or tool execution.
