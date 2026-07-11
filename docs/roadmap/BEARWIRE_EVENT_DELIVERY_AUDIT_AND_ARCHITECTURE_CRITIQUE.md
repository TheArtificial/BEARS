# BearWire Event Delivery Audit and Architecture Critique

Status: findings from a 2026-07-11 code review into recurring `client_obligation_timeout`
failures where Den persists a `tool_call.requested` event and armature-local
`tool_result` obligation, but the active armature prompt loop never processes the event.
This document records root causes, architectural weaknesses, and the recommended direction.
Immediate mitigations landed after the review include same-session BearWire append
serialization, armature run-id filtering for run-scoped events, stale-run cancellation on
supersession, SSE parser hardening, and read-only obligation recovery from `run.state`.

Related: [BEARWIRE_V1_PROTOCOL_REFINEMENT_ROADMAP.md](BEARWIRE_V1_PROTOCOL_REFINEMENT_ROADMAP.md),
[BEARWIRE_TURN_COORDINATOR_REFACTOR_PLAN.md](BEARWIRE_TURN_COORDINATOR_REFACTOR_PLAN.md),
[DEN_BEARWIRE_ARMATURE_ACP_HARDENING_PLAN.md](DEN_BEARWIRE_ARMATURE_ACP_HARDENING_PLAN.md).

## Failure signature

```text
Den:
  tool_waiting_for_result tool_name=fs_search_files
  obligation_id=… event_sequence=N        # tool_call.requested persisted + obligation open

Armature:
  no "BearWire tool_call.requested received"
  no "local tool task spawned"
  eventually receives run.failed(client_obligation_timeout)
```

The armature keeps polling the correct session (it receives the eventual `run.failed`),
yet event `N` is never delivered. A recovery guardrail in
`tools/bear-armature/src/bearwire.rs` reconstructs safe read-only tool requests from
`run.state`; it is a mitigation, not the root fix.

## Root causes, ranked

### 1. Out-of-order sequence visibility in the Den event store (best fit)

- `bearwire_events.sequence_no` is a `BIGSERIAL`
  (`services/den/migrations/20260618130000_bearwire_events.up.sql`): sequence values are
  assigned at INSERT time, not commit time.
- `persist_bearwire_tool_call_wait_transactionally`
  (`services/den/crates/den-runtime/src/turn_waits.rs`) appends `tool_call.requested`
  inside a multi-statement transaction; every other appender on the same session stream
  is autocommit and instantly visible (`run.progress`, the `tool_call.finished` echo in
  `methods/client.rs`, `run.failed` from poll-side expiry, `session.*` appends).
- The armature advances its cursor to the max sequence seen per poll
  (`tools/bear-armature/src/bearwire.rs`, `handle_prompt`). If a poll lands while the
  tool-wait transaction is open and an autocommit event with a higher sequence is already
  visible, the cursor passes the still-invisible tool event. `sequence_no > after` then
  never returns it — a permanent, silent skip.
- Realistic concurrent writers: parallel tool-call result echoes racing the next
  tool-wait transaction, and superseded runs still streaming (cause 2).

### 2. Superseded runs are never stopped; armature ignores `run_id` on terminal events

- `run.start` supersedes the active run by updating DB rows only
  (`methods/run.rs`); the old run's spawned task keeps streaming and appending session
  events, and no terminal BearWire event is emitted for the superseded run.
- The armature poll loop treats `run.completed` / `run.failed` / `run.cancelled` as
  terminal for the current turn **without comparing the event's `run_id`**. A late
  terminal event from an old run kills the new prompt's loop; the new run's next
  obligation then has no poller and times out.
- Inverse direction: the superseded prompt loop has no cancellation check and keeps
  acting on the new run's events with a stale turn token (including spawning duplicate
  tool tasks — the normal `tool_call.requested` path does not dedup, unlike the
  recovery path).

### 3. Poll-side expiry cascades across runs

`expire_session_client_obligations` (`den-bearwire/src/events.rs`) runs on every events
poll, is session-scoped, and appends `run.failed(client_obligation_timeout)` for any run
with a stale open obligation. Combined with the missing `run_id` filter, one orphaned
obligation from an earlier run kills the current prompt loop on its next poll, orphaning
the current run's obligations too — a self-perpetuating chain.

### 4. Hard `?` aborts leave obligations open

In the armature poll loop, a transient `fetch_events` HTTP error, malformed SSE JSON
(`parse_event_frame`), or a `tool_call.completed` whose `data` fails to parse aborts the
entire prompt with no retry while Den keeps waiting.

### 5. Silent drop of malformed tool requests (latent)

`spawn_tool_request_task` (`tools/bear-armature/src/main.rs`) drops a received
`tool_call.requested` with only an stderr line when `data.tool_call` fails to
deserialize; the obligation stays open. Den's `tool_call_wire` always emits `id`/`name`
today, so this is latent.

## Architecture critique

The bugs above are downstream of three design decisions.

### A. Correctness-critical delivery rides on a channel designed for UX streaming

The event stream carries traffic with two very different loss tolerances:

- cosmetic (token deltas, `run.progress`) — a dropped item is harmless;
- obligations (`tool_call.requested`, `client.waiting`) — a dropped item deadlocks
  the run.

The architecture treats them identically. The system already owns the right primitive for
the second category: `turn_obligations` is durable, idempotent, keyed by run, and
queryable. The recovery guardrail that polls `run.state` is the architecturally correct
delivery mechanism demoted to a fallback and artificially restricted to read-only tools.

**Recommended direction (highest leverage):** make obligations authoritative for
actionable work while keeping events as ordered projection/replay. The armature's primary
loop for *work* should be able to discover and satisfy open obligations directly
(self-healing, cursorless, at-least-once by construction; the tool-result coordinator's
duplicate handling already provides the idempotency this needs). The event stream remains
valuable for live UI, replay, and ordering, but a missed event must degrade projection
rather than runtime correctness. This demotes exactly-once event delivery over Postgres
polling from a correctness requirement to a projection-quality concern.

### B. The event log is a hand-rolled queue with the classic Postgres pitfall

A client cursor over an insert-time `BIGSERIAL` yields at-most-once delivery under
concurrent writers, because sequence order ≠ commit-visibility order. The design makes
the race likely rather than incidental: appends happen both transactionally and
autocommit, and the client invents its cursor semantics (`after = max sequence seen`)
instead of the server attesting a safe resume point.

If the log stays authoritative for anything actionable:

- serialize same-session appends (e.g. `pg_advisory_xact_lock(hashtext(session_id))`
  inside every append) so commit order matches sequence order, or add a visibility
  watermark to the read path;
- return an explicit `next_after` from the events endpoint instead of letting clients
  infer it.

Related smell: events are INSERTed then UPDATEd to embed their own `sequence`/`event_id`
in the payload. Records are not immutable, envelope metadata duplicates column data, and
there is a torn-read window on the autocommit path. Envelope fields should live in
columns and be composed at read time.

### C. Run lifecycle has no single owner

A run's state lives in three places — the `turn_runs` row, the spawned tokio task, and
the event log — and transitions update some but not all. Supersession flips the DB row,
but the task keeps streaming and no terminal event is appended, so every observer gets a
different story. Symmetrically, armature cancellation gates UI updates only; a superseded
prompt loop keeps polling and doing real work with a stale token.

Both sides need the same conceptual fix: tie task lifetime to run state (an abort handle
owned by whatever supersedes the run; a loop exit condition owned by whatever registers
the new turn), and make emitting a terminal event part of the state transition, not
optional.

### Smaller items

- **The events GET mutates state.** Obligation expiry and run failure inside the poll
  handler makes read frequency semantically load-bearing and lets one poller inject
  `run.failed` into another run's consumer. Expiry belongs in a background sweeper.
- **Session-scoped stream, run-scoped consumers, no enforced scoping.** `run_id` is on
  every event but nothing requires consumers to filter by it, so it is decorative until
  someone forgets (which is exactly what happened with the terminal-event handlers).
  Either make run filtering a stated protocol obligation or let subscriptions take a
  `run_id`.
- **SSE cosplay.** The endpoint returns a fully buffered body in SSE framing; the client
  hand-parses it with a partial SSE parser (last-`data:`-line-wins, `[DONE]` handling for
  a server that never emits it). Either make it a real long-lived stream or return a
  plain JSON page with a cursor.
- **All-or-nothing error handling in the delivery loop.** Per-event fault isolation and
  designed-for duplicate delivery (dedup on `tool_call_id` in the normal spawn path) are
  prerequisites for any at-least-once story.

## Implementation plan

### Phase 0 — Containment already landed

Status: implemented immediately after this review.

- Serialize same-session BearWire appends with a transaction-scoped advisory lock so
  sequence cursor order matches commit visibility.
- Wrap autocommit event appenders in explicit transactions that also take that lock.
- Filter run-scoped armature events by the active prompt `run_id`.
- Cancel registered session turn/tool-turn state when a new run supersedes an active run.
- Harden armature SSE parsing for multi-line `data:` frames and no-data frames.
- Keep read-only `run.state` obligation recovery as a temporary safety net.

Exit criteria:

- The known cursor-skip race is covered by regression tests.
- Foreign terminal run events no longer terminate the active armature prompt loop.

### Phase 1 — Server-owned cursor pages

Status: partially implemented. Den now exposes `/bearwire/v1/sessions/{session_id}/events/page`, advertises it from `initialize`, and bear-armature prefers it while retaining SSE fallback. The armature now advances from Den's `next_after` for JSON pages instead of computing `max(sequence seen)` itself. Remaining work is the broader integration coverage listed below.

Replace client-inferred cursor advancement with server-attested paging.

Target response shape, even if still delivered through the existing endpoint initially:

```json
{
  "events": [],
  "next_after": 234820,
  "has_more": false
}
```

Rules:

- Den, not the client, decides the safe cursor to persist/advance.
- Empty incremental polls return `next_after = previous_after`.
- Frames/pages with no event data cannot advance the cursor.
- The current SSE-shaped buffered response may be kept as a compatibility projection, but
  a plain JSON page should become the canonical polling contract if the endpoint remains
  request/response rather than real streaming.

Exit criteria:

- Armature no longer computes `after = max(sequence seen)` for the canonical JSON page path.
- Unit tests cover empty JSON pages and server-owned `next_after` semantics.
- Remaining: integration tests cover missing event ids and immediate post-`run.start` tool events across the HTTP endpoint.

### Phase 2 — Obligations as authoritative actionable work

Status: partially implemented for safe, approval-free, armature-local read-only tools. Bear-armature now polls `run.state` as a regular obligation sync loop and services eligible open obligations even if their projection event was missed. Mutating, approval-required, Den-owned, and human-input obligations remain fail-closed until their idempotency and safety policy is explicit.

Promote `run.state` / open-obligation polling from recovery guardrail to a first-class
armature work loop.

Rules:

- `tool_call.requested` / `client.waiting` events remain the fast projection path.
- Open obligations are the authoritative source for work the armature must perform.
- The armature may execute an obligation at least once; Den's coordinator owns duplicate
  and conflict handling.
- Recovery should expand beyond read-only only after each obligation kind has explicit
  idempotency, approval, and safety semantics.

Exit criteria:

- Safe read-only path implemented: missing an event should not cause an armature-local read-only tool obligation to time out.
- Mutating, approval-required, Den-owned, and human-input obligations fail closed unless their idempotency/safety policy is explicit.
- Normal event-driven and `run.state`-serviced tool execution share the same atomic dedup path by `session_id` + `tool_call_id`.
- Remaining: promote obligation identity/idempotency policy into descriptors before expanding beyond the read-only allowlist.

### Phase 3 — Run lifecycle ownership

Status: partially implemented. BearWire run supersession and explicit cancellation now share a lifecycle helper that cancels registered active stream/tool work, transitions the active run to `cancelled`, settles open obligations, transitions active steps, records work-run cancellation, and emits a run-scoped `run.cancelled` event. BearWire background run tasks register with the cancellation registry and stop when superseded/cancelled, while terminal `turn_runs` can no longer be reopened or overwritten by stale runtime events. Remaining work is to move all terminal transitions into this owner and make continuation tasks use the same lifecycle path.

Introduce a single run lifecycle owner that coordinates DB state, task cancellation,
obligation settlement, and terminal event emission.

Supersession should be one conceptual operation:

1. cancel active runtime task / registered turn work;
2. settle or fail outstanding obligations;
3. transition the `turn_runs` row;
4. emit a terminal `run.cancelled` or `run.failed`/`run.superseded` event;
5. prevent future stale appends for that run.

Exit criteria:

- Implemented for BearWire start supersession: registered background tasks observe cancellation, and terminal run rows reject stale transitions.
- Implemented for explicit `run.cancel`: cancellation emits a run-scoped `run.cancelled` event and settles obligations.
- Remaining: route continuation-task terminal handling through the same lifecycle owner instead of direct `persist_run_failed` calls.
- Remaining: every terminal state transition has a corresponding terminal projection event from the owner.
- Remaining: Armature and Den agree on why every loop exited across start, continuation, cancellation, and timeout paths.

### Phase 4 — Move expiry out of `GET /events`

Status: implemented for BearWire API processes. `GET /events` and `/events/page` are read-only with respect to obligation/run state, and the API composition root starts a BearWire client-obligation expiry loop that marks timed-out obligations and persists run-scoped `run.failed(client_obligation_timeout)` events.

Move client-obligation expiration from the events polling handler into a Den-owned sweeper
or coordinator-owned timeout task.

Rules:

- Read endpoints do not mutate run/obligation state.
- Expiry cadence is owned by Den runtime policy, not by whichever client happens to poll.
- Expiry events remain run-scoped and do not affect unrelated active runs.

Exit criteria:

- Implemented: `GET /events` is side-effect-free except for auth/observability.
- Implemented: timeout mutation is owned by the BearWire client-obligation expiry loop.
- Remaining: tests prove an old orphaned obligation cannot inject a terminal event into a current run's consumer across a live multi-run session.

### Phase 5 — Fault-isolated armature event processing

Status: partially implemented. Bear-armature now retries transient event-fetch failures before aborting, services safe `run.state` obligations during fetch-error windows, skips malformed SSE frames while preserving valid frames, skips/logs non-terminal event handler failures, and posts a structured error result for malformed `tool_call.requested` events when `run_id` and `tool_call_id` are available. `run.failed` remains intentionally fatal to the prompt loop.

Make event handling at-least-once friendly.

Rules:

- Transient `fetch_events` failures are retried with bounded backoff while the run is
  non-terminal.
- Malformed or unknown non-actionable events are logged and skipped without killing the
  prompt loop.
- Malformed actionable events are converted into structured error results where possible,
  so obligations close rather than timing out.
- Normal `tool_call.requested` spawning dedups by `tool_call_id` / `obligation_id`, matching
  the recovery path.

Exit criteria:

- Implemented: one malformed non-terminal event should not stop unrelated event processing or safe obligation servicing.
- Implemented: malformed tool requests with enough identity are answered with structured error results instead of silently timing out.
- Implemented: duplicate delivery of a tool request cannot spawn duplicate local tool execution.
- Remaining: broader integration tests for malformed actionable permission waits and repeated transient HTTP failures.

### Phase 6 — Choose real streaming or explicit JSON polling

Retire the current buffered-SSE ambiguity.

Options:

- real long-lived SSE/WebSocket stream for live projection, plus obligation polling for
  actionable work; or
- explicit JSON event pages with `next_after`, plus optional separate push channel later.

Exit criteria:

- Protocol docs describe one canonical delivery contract.
- Armature implementation no longer hand-parses buffered SSE pages as if they were a true
  stream.

## Suggested instrumentation (until all phases are complete)

- Den: after committing a tool wait, log `sequence_no`, transaction begin→commit
  duration, and commit timestamp. In the events endpoint, log per poll:
  `session_id, after, min/max sequence returned, count`. A skipped event is then provable
  from logs: some poll returned `max ≥ N` at a wall-clock time earlier than `N`'s commit.
- Armature: promote cursor advancement (event types + sequences per poll) and loop exit
  reason (terminal event type + `run_id`, error, timeout) to lifecycle-level logs so
  "loop died" and "event never arrived" are distinguishable in production.

## Regression tests to add

1. **Out-of-order visibility:** hold the tool-wait transaction open on one connection,
   autocommit-append a `run.progress` on another, poll `after=N-1`, and assert the
   response cannot advance a client past the uncommitted event (documents the bug until
   the append/read path is fixed).
2. **run.start cursor + immediate tool request:** first poll with
   `after = event_sequence - 1` must return both `run.accepted` and a tool request
   appended before the first poll.
3. **Empty poll then tool event:** empty incremental poll leaves the cursor unchanged
   (no synthetic `session.state`); an event appended afterwards is delivered.
4. **Concurrent session prompts:** after supersession, the old run appends no further
   events (or gets a terminal marker); armature-side, a terminal event whose `run_id`
   differs from the current run must not set `saw_done` or error the loop.
5. **Frame-id handling:** frame without `id:` → event processed, cursor not advanced,
   redelivery tolerated; `id:`-only or `[DONE]` frames have explicit cursor policy;
   malformed JSON does not abort the prompt loop.
6. **Loop liveness with open obligations:** transient `fetch_events` failure and an
   unparseable `tool_call.completed` mid-run are survived, and the pending obligation is
   still serviced.

## Non-goals

- Redesigning tool execution or removing approval gating.
- Treating the read-only recovery guardrail as the root fix.
- Routing armature-local tools through Den to avoid the missing event.
