# Pair `/focus` continuation-freeze investigation

**Status:** Investigation complete; implementation follow-up is tracked in Docket job `22dad768-3462-4d3c-b60a-cca09f9d0e22`.

## Evidence

Two production traces establish that a Pair session can appear frozen while a `/focus`-initiated run is waiting for a server-side tool continuation:

1. On 2026-09-07 at 17:31 UTC, `run_4e7c…` was interrupted with
   `server_restart_interrupted`. An LLM handshake which began before that interruption later completed and its continuation was fenced because `active_run_id=None`.
2. On 2026-09-07 at 18:11 UTC, without a restart or supersession, one run
   (`run_738a…`) completed continuations at steps 1 and 2 and then logged only
   `LLM stream handshake starting` for step 2. The trace contains no later
   handshake-connected, terminal tool/turn event, failure, or client completion.

The second trace proves restart/supersession is not necessary for the symptom.
The 30-second idle timeout recorded in the handshake logs must be followed through
in the implementation; the captured lifecycle has no corresponding terminal event.

## Runtime path and ownership points

The relevant server-side path is
`den-runtime/src/agent_loop/session_stream.rs`:

1. `SessionTrackingStream` receives a model `ToolCallRequested` event and queues
   the Den-hosted tool (`pending_server_tool`).
2. The tool executes and its result is persisted to the session transcript.
   A visible `ToolCallFinished` is emitted before the model continuation starts.
3. The continuation refreshes task context and asks
   `turn_runs::active_run_for_session` for the authoritative active run.
4. If the session run differs from the authoritative run, it emits
   `native_superseded_run_continuation_fenced` and returns the explicit
   cancellation stream. This is the authoritative ownership fence. A restart or
   run invalidation makes `active_run_id` absent at this point.
5. Otherwise the continuation invokes `run_agent_step_stream`, which begins the
   next provider handshake/stream. A provider/driver continuation that remains
   pending at this point has no independent terminal event in the client stream.

Thus the direct no-restart failure boundary is **after tool completion and before
(or during) the next model continuation's terminal event**, not focus task
selection or active-run fencing. Restart/supersession is a separate path that can
strand the same continuation after it has begun.

## Deterministic regression probe

`server_tool_completion_is_not_blocked_by_stalled_model_continuation` is the
minimal existing runnable reproduction in
`services/den/crates/den-runtime/src/agent_loop/session_stream.rs`.

It supplies a perpetually pending `ServerToolContinuationFuture`, verifies the
client receives `ToolCallFinished`, then verifies the next stream poll times out.
That exactly models the externally visible freeze: the tool appears complete but
no terminal turn result ever follows. It is intentionally a characterization test
of the current failure mode; later implementation tasks should invert its final
expectation to require a bounded, retryable terminal event.

Run it with:

```bash
cd services/den
cargo test -p den-runtime --lib server_tool_completion_is_not_blocked_by_stalled_model_continuation -- --exact
```

## Required repair boundary

The next implementation must give a pending server-tool continuation a bounded,
client-visible outcome (or safe resumption), and must cancel it when the run loses
ownership. It must not treat `ToolCallFinished` as completion of the user turn.
