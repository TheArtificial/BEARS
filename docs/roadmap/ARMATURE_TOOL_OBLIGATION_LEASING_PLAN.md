# Armature tool-obligation leasing implementation plan

Status: implemented; guarded UAT pending

Date: 2026-07-30

Implemented by: `8ff45a51`, `fbe42c75`, `226b130b`

Related ADRs: [ADR-0048](../decisions/adr-0048-core-turn-client-obligation-coordinator.md), [ADR-0034](../decisions/adr-0034-bearwire-as-den-armature-wire.md), [ADR-0030](../decisions/adr-0030-bearwire-resource-oriented-event-model.md)

## Goal

Make long-running armature-local commands survive transient BearWire event-poll failures without allowing concurrent or automatic re-execution. Den owns a durable, narrowly fenced, renewable execution claim; event delivery remains stateless and cursor-based.

## Non-goals

- No long polling, SSE, WebSockets, or server-held waiter registry.
- No tool-name-specific handling for `cargo`, shells, or command tools.
- No automatic retry after an execution claim has an unknown outcome.
- No durable event row for each lease renewal.
- No attempt-token exposure in state, history, events, logs, or user diagnostics.

## Lease contract

A lease applies only to an armature-local `ToolResult` obligation.

```text
waiting --claim--> claimed/running
claimed/running --renew--> claimed/running
claimed/running --result--> result received
claimed/running --expiry--> failed/outcome_unknown
```

All mutating operations are conditional on authenticated responder plus:

```text
run_id + session_id + obligation_id + tool_call_id + attempt_token + open state
```

`turn_step_id` joins the fence where already available. Den's database clock owns expiry. The initial policy is a 30-second lease with renewal requested after 10 seconds, returned by Den rather than hard-coded by tool name.

## Phase 1: persistence and coordinator transitions

Add the minimum obligation fields needed for claimed execution:

- attempt-token hash (never store the plaintext token if comparison can use a hash);
- `claimed_at`;
- `lease_expires_at`.

Implement coordinator operations for claim and renewal. Settlement, cancellation, renewal, and expiry must use compare-and-set updates so one transition wins. Update expiry scanning to distinguish:

- unclaimed wait timeout → `client_obligation_timeout`;
- claimed lease expiry → `outcome_unknown`, no automatic retry, inspect `run.state` and workspace.

Keep permission obligations and Den-owned tools unchanged.

Done when:

- two concurrent claims yield exactly one execution owner;
- stale/wrong tokens cannot renew or settle;
- renewal cannot revive terminal state;
- result-versus-expiry and renewal-versus-expiry races have one canonical winner;
- claimed expiry persists the normalized unknown-outcome recovery evidence.

## Phase 2: BearWire protocol and state projection

Add typed `client.tool.claim` and `client.tool.renew` requests/responses to the shared BearWire protocol types and HTTP/RPC dispatch. Require the attempt token on `client.tool.result` for claimed obligations.

Expose only safe lease observations in `run.state`:

```text
status: waiting | claimed/running
lease_expires_at: optional timestamp
```

Do not project the token or token hash. Preserve identical-result idempotency and reject conflicting duplicates.

Done when:

- protocol round-trip tests cover claim, renew, stale token, and redaction;
- auth and identity checks include run, session, obligation, tool call, and responder;
- old unclaimed result submission is either migrated atomically with the adapter or rejected explicitly—no silent compatibility path that can execute twice.

## Phase 3: armature claim and heartbeat

At the existing spawned ACP tool-task ownership boundary:

1. receive the armature-local tool obligation;
2. claim it before invoking ACP;
3. invoke ACP only after a successful claim;
4. await tool completion, renewal timer, and cancellation in one `tokio::select!` loop;
5. submit the final result with the attempt token;
6. stop renewal after settlement or loss of ownership.

A failed or ambiguous claim/renewal triggers `run.state` reconciliation, never replacement execution. The local task registry remains a deduplication optimization only.

Done when:

- a slow fake ACP command renews beyond the original obligation deadline;
- two armatures racing one request invoke the fake command once;
- cancellation and process loss stop renewal;
- failed result delivery produces a qualified unknown-outcome recovery message.

## Phase 4: obligation-aware stateless polling

Keep short-lived event-page requests and the authoritative cursor. Replace the universal five-consecutive-errors abort with elapsed-time/backoff policy:

- exponential backoff with jitter for event fetch failures;
- query `run.state` before any synthesized terminal transport error;
- Den's canonical terminal state always wins;
- while this armature owns a current command lease, event-fetch failures do not terminate the prompt or transfer execution ownership;
- if both event polling and state reconciliation are unavailable, continue waiting on the local command and renewing when possible; if Den cannot accept renewal/result, report that contact was lost while a command may still be running.

For runs without a locally owned execution lease, retain a bounded elapsed-time failure policy. Do not infer Den unreachability from a generic Reqwest error; describe the failed operation and preserve its source.

Done when:

- a slow command survives beyond the old five-fetch window;
- repeated event failures plus a reachable terminal `run.state` display Den's canonical message;
- repeated event failures without an active lease still terminate after the configured elapsed grace period;
- concurrent sessions retain independent cursors, leases, and failures.

## Phase 5: end-to-end recovery and diagnostics

Add one end-to-end scenario covering:

1. command starts and holds a lease;
2. event fetching fails transiently;
3. lease renewal proves the armature is alive;
4. either result settlement continues normally, or armature disappearance lets the lease expire;
5. conversation output, `run.state`, and run/log diagnostics render the same normalized outcome and recovery evidence.

Inspect logs and structured state to ensure attempt tokens are absent. Verify workspace/process inspection guidance includes the run ID and never recommends retry before reconciliation.

## Implementation status

All five phases are implemented locally:

- `8ff45a51` added the migration, coordinator claim/renew operations, token-fenced settlement, BearWire methods, expiry semantics, and coordinator/projection checks.
- `fbe42c75` made Armature claim before ACP execution, renew while the tool future is pending, stop on definitive lease loss, and carry the attempt token through settlement.
- `226b130b` removed the universal five-fetch abort, added backoff and `run.state` reconciliation, and kept polling failures from terminating an actively leased command.

Validated during implementation:

- coordinator contract: 10 tests passed;
- surface projection contract: 3 tests passed;
- Armature binary suite: 223 tests passed;
- formatting and compilation checks passed at each checkpoint.

The remaining work is guarded end-to-end UAT against a jointly updated Den server and Armature. No compatibility fallback was added for older armatures because accepting unfenced results would weaken the execution-ownership guarantee.

## Delivery record

Implemented as three reviewable commits:

1. `8ff45a51 Add fenced tool obligation leases`
2. `fbe42c75 Lease Armature tool execution`
3. `226b130b Make BearWire polling obligation aware`

The server enforcement and matching armature claim path must be deployed together unless a future explicit capability negotiation preserves the same fencing guarantees.
