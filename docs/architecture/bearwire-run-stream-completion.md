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

Use the existing BearWire event store and deployment controls; do not add a second
telemetry pipeline for this change. For each rollout window, query the persisted
`initial_stream_interrupted` progress/visible-marker events and terminal run events,
scoped to the deployment window and traffic cohort. The release owner records these
four rates against the pre-rollout baseline for the same traffic class:

1. **Incomplete streams:** interrupted runs / started runs.
2. **Recovery success:** interrupted runs that subsequently reach a terminal state
   without a second tool request / interrupted runs.
3. **Forced session termination:** sessions with an interruption followed by a
   terminal session closure without a user cancellation / interrupted sessions.
4. **Internal-error exposure:** user-visible messages matching internal diagnostic
   vocabulary (`BearWire`, frame/event counts, server version, or git SHA) / visible
   interruption markers. This must be zero.

### Staged rollout and rollback

1. Deploy to the existing smallest production cohort. Run one controlled
   tool-bearing interruption and verify that its pending obligation remains answerable
   and that a retry resumes the same run without a duplicate tool request or assistant
   message.
2. After at least 30 minutes and 20 BearWire runs (whichever is later), expand only
   if internal-error exposure and forced session termination are both zero, and neither
   incomplete-stream rate nor failed-recovery rate exceeds its baseline by more than
   1 percentage point. If the baseline has fewer than 20 samples, keep the cohort in
   place until it does.
3. Repeat that check at the existing broader cohort and again after full rollout.
   Keep the event query and the controlled-run IDs with the release record.

Halt expansion and use the existing deployment rollback control to restore the prior
image if either zero-tolerance signal is non-zero, a rate crosses the 1-point limit,
or the controlled verification duplicates output/tool execution or leaves a pending
tool call unanswerable. Do not delete the affected run or conversation during
rollback: preserve it for reconciliation and incident review.

Post-release, sample every interrupted run from the first hour (or the first 20, if
more) and confirm its pending tool-call state, subsequent terminal state, and absence
of duplicate persisted assistant content. The event records intentionally contain tool
IDs/states but not tool arguments, so this verification remains safe for operator use.
