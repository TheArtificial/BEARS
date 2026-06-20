# ADR-0044: Runtime stream state machines must make progress explicit

**Status:** Proposed  
**Date:** 2026-06-20  
**Deciders:** Hans

**Related:**
- [ADR-0035: Den-native in-process agent runtime](adr-0035-den-native-in-process-agent-runtime.md)
- [ADR-0043: ACP Is an Edge Adapter; the Den Runtime Is Protocol-Agnostic](adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md)
- [BearWire armature wire implementation plan](../roadmap/BEARWIRE_ARMATURE_WIRE_IMPLEMENTATION_PLAN.md)

## Context

Den's native runtime uses custom `Stream` state machines to bridge model streams, semantic runtime events, tool calls, approval pauses, continuations, and edge projections such as BearWire and ACP. These streams are performance-critical and correctness-critical: a missed poll, wake, terminal event, or queued output can look like model latency, broken approval UI, lost tool calls, or missing conversation history.

A production BearWire/ACP delay exposed a subtle bug in this class. The model emitted an OpenAI tool call quickly, and Den's parser produced `ToolCallRequested` quickly, but the approval request surfaced much later. The issue was not model latency, Bifrost latency, BearWire polling, or Zed approval handling.

The problematic sequence was:

```text
poll SessionTrackingStream
  poll inner model stream
    receives ToolCallRequested
  create pending_approval future
  store pending_tool_event
  return Poll::Pending
```

The newly-created `pending_approval` future had not yet been polled, so it had not registered the current task's waker. Returning `Poll::Pending` violated the async contract in practice: the stream had not arranged to be woken when progress was possible. It could remain parked until some unrelated wake happened.

This was not a one-off lesson about approvals. It is a general rule for custom runtime stream state machines.

## Decision

Den runtime stream state machines must make progress explicit. A custom `Stream` implementation must satisfy these invariants:

### 1. `Poll::Pending` must have a wake path

If a stream returns `Poll::Pending`, at least one of these must be true:

- it just polled an existing child future/stream that registered the current waker;
- it explicitly arranged a wake with `cx.waker().wake_by_ref()`;
- it is waiting on an external cancellation/notification primitive that has already been polled or otherwise registered.

It is not valid to create new internal async work and return `Poll::Pending` before that work has been polled or woken.

### 2. Installing internal async work must poll or wake

Whenever a stream installs new internal work during `poll_next` and does not immediately poll that work in the same call, it must wake itself:

```rust
self.pending_work = Some(Box::pin(async move { /* ... */ }));
cx.waker().wake_by_ref();
return Poll::Pending;
```

Equivalently, a state machine may loop back and poll the newly-installed future in the same `poll_next` call. The requirement is not the exact mechanism; the requirement is that the new work is not left wakerless.

This applies to internal futures, queued tool executions, queued continuations, pending approval registration, and queued next-step construction.

### 3. Queued output should usually be emitted before polling more input

A stream with a `pending_out`, `queued_*`, or `pending_*_event` buffer should drain queued output before polling upstream input again. This prevents later input from overtaking already-derived semantic events.

Recommended shape:

```rust
if let Some(event) = self.pending_out.pop_front() {
    return Poll::Ready(Some(Ok(event)));
}
```

### 4. State transitions should be explicit and small

Prefer explicit phases such as:

```rust
enum Phase {
    PollingModel,
    WaitingForApproval,
    WaitingForToolResult,
    ExecutingServerTool,
    Continuing,
    Terminal,
}
```

Avoid implicit state encoded only by scattered `Option<Pin<Box<...>>>` fields unless the invariants are locally obvious and tested.

### 5. Terminal and cancellation states must drain or settle obligations

Before a stream becomes permanently finished, it must either:

- emit/persist the terminal event;
- settle outstanding tool/permission obligations;
- intentionally persist incomplete tool results; or
- document why no outstanding work exists.

Terminal suppression is allowed only when another state machine owns the terminal emission and this ownership is explicit.

### 6. Instrument important boundaries with `tracing`

Runtime stream state machines should log low-volume lifecycle milestones at `info` or `debug`, and high-volume parser/stream internals at `trace` under targeted names. Do not add bespoke envvars when `tracing` target/level filtering is sufficient.

Examples:

```rust
tracing::debug!(target: "den.llm.stream", ...);
tracing::trace!(target: "den.llm.stream", ...);
```

### 7. Regression tests should poll manually when wake behavior matters

Async stream wake bugs often disappear under `.next().await` because a runtime may provide incidental wakes. For wake-sensitive behavior, tests should manually call `poll_next` with a custom counting waker and assert the expected wake behavior.

## Consequences

### Positive

- Prevents latent stalls where model/tool events exist but the runtime stream is parked.
- Makes BearWire and ACP responsiveness less dependent on accidental external wakes.
- Creates a shared review vocabulary for custom stream state machines.
- Makes future runtime bugs easier to localize because progress points and ownership boundaries are explicit.

### Negative / tradeoffs

- Extra self-wakes can cause an additional poll. This is acceptable for infrequent state transitions like approval setup, queued tool output, or next-step creation.
- Overuse of `wake_by_ref` can obscure state-machine structure. Prefer polling the newly-created child immediately when that keeps the code clearer.
- More explicit phases can add boilerplate. Use them where state interactions are non-trivial; do not over-engineer simple stream adapters.

## Implementation notes

The initial bug fix is in `services/den/crates/den-runtime/src/agent_loop/session_stream.rs`:

```rust
self.pending_approval = Some(Box::pin(async move {
    maybe_pause_for_tool_approval(/* ... */).await
}));
cx.waker().wake_by_ref();
return Poll::Pending;
```

Regression coverage lives in:

```text
services/den/crates/den-runtime/src/agent_loop/session_stream.rs
```

Test:

```text
approval_required_tool_call_wakes_after_installing_pending_future
```

The test constructs a `SessionTrackingStream`, feeds it an approval-required `ToolCallRequested`, polls once with a counting waker, and asserts that the stream returns `Pending` while scheduling exactly one wake and storing the pending approval/tool event.

## Review checklist

When reviewing custom `Stream` implementations in `den-runtime`:

1. Identify all state fields: `pending_*`, `queued_*`, `active`, `phase`, `finished`, `terminal_*`, and child streams/futures.
2. Find every `return Poll::Pending`.
3. For each `Pending`, ask: what will wake this task?
4. Find assignments like `pending_* = Some(...)`, `active = Some(...)`, `queued_* = Some(...)`, or phase transitions inside `poll_next`.
5. If newly-assigned work needs another poll to progress, require either immediate polling or `cx.waker().wake_by_ref()`.
6. Check that queued output is drained before upstream input is polled again.
7. Check that terminal/cancel paths settle obligations or persist incomplete work deliberately.
8. Check that high-volume instrumentation uses `tracing::trace!`/`debug!` targets, not custom env toggles.
9. Add a manual-poll regression test when wake behavior is part of the invariant.

A targeted review after the BearWire delay found:

- `SessionTrackingStream` needed the wake and now has it.
- `LazyAgentStepStream` returns `Pending` only after polling an existing future/stream.
- `openai_byte_stream_to_event_stream` returns `Pending` only after polling the underlying byte stream.
- `NativeWebChatLoopStream` already wakes after queueing output or advancing internal tool execution state.
