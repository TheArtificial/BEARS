# Den state-authority risk retirement roadmap

## Status

Completed implementation plan. Last updated: 2026-07-18.

This roadmap converts the state-machine inventory into a risk-first sequence of authority retirements. It prioritizes disagreements that can cause unauthorized side effects, stuck runs, lost terminal outcomes, or cross-turn corruption before conceptual cleanup.

Canonical inventory: [Den state machine inventory](../architecture/den-state-machine-inventory.md).

Related contracts:

- [ACP runtime contract](../architecture/acp-runtime-contract.md)
- [BearWire JSON specification](../architecture/bearwire-json-spec.md)
- [ACP lifecycle reset plan](ACP_LIFECYCLE_RESET_PLAN.md) — historical/broader context

## Rules of execution

Each milestone must:

1. retire or narrow an authority rather than add a peer state source;
2. have a disjoint, reviewable scope;
3. add or update focused regression tests;
4. pass focused tests before broader tests;
5. update this progress ledger;
6. be committed separately before the next milestone starts.

Avoid new caches, reconciliation loops, or string-based routing. Prefer typed owners, cancellation handles, monotonic generations, and derivation from canonical state.

## Risk order

| Priority | Risk | Failure mode | Retirement target |
| --- | --- | --- | --- |
| Foundation | Terminal-state/event disagreement | terminal DB row without terminal event; stuck ACP working state | one atomic run finisher |
| 1 | Cancellation vs external execution | local mutation continues after cancel; stale result reaches newer turn | registry-owned durable cancellation |
| 2 | Execution-owner disagreement | Den tool executes locally or client tool is never answered | one descriptor-owned execution resolver |
| 3 | Liveness-owner disagreement | healthy continuation falsely fails or dead run never ends | Den-owned typed provider/runtime liveness |
| 4 | Current-turn vs stale projection | older run overwrites title, binding, usage, or plan state | turn-token or monotonic-version gating |
| 5 | Observational cache authority | stale IDs suppress valid updates; unbounded memory | current-turn scope or bounded eviction |
| 6 | Run wait state vs obligations | impossible waiting combinations; orphaned waits | derive blocking reason from obligations |
| 7 | Workflow/plan labels vs permissions | projection labels accidentally expand mutation authority | compile once into `TurnAuthority` |
| 8 | Governance label overlap | presence, mutation posture, and emergency state disagree | orthogonal typed governance inputs |
| 9 | Projection axes mixed with authority | prompt/UI/cache state becomes accidental truth | grouped derived-view boundary |
| 10 | Completion decision treated as state | continue/complete labels drift from durable run outcome | pure decision plus atomic finish only |

## Progress ledger

### Foundation — atomic terminal authority

Status: **completed**

Delivered:

- shared descriptor-owned tool execution resolver;
- one initial/continuation stream-boundary classifier;
- atomic run finish transaction covering run state, obligations, active steps, and terminal BearWire event;
- runtime completion/failure/cancellation and explicit session cancellation/supersession migrated to the atomic finisher;
- ordinary `transition_run` made nonterminal-only;
- redundant public obligation/step terminal settlement helpers removed;
- ACP requires a durable run terminal event before prompt completion;
- state inventory, ACP contract, and BearWire specification updated.

Acceptance checks:

- completion persists a terminal event;
- cancellation reports authoritative settlement counts and event sequence;
- expiry/failure is idempotent;
- terminal runs cannot be reopened or overwritten;
- late client results are ignored after terminal state.

### Milestone 1 — durable local-tool cancellation

Status: **completed**

Goal: cancellation must be durable across registration/subscription races and must stop external execution before registry eviction.

Delivered:

- cancellation receivers are subscribed before local tool tasks are spawned or registered, eliminating the broadcast registration/subscription gap;
- cancellation eviction remains session/turn scoped;
- task phase and input mutations are update-only and cannot recreate cancelled records;
- added race coverage for cancellation sent before the execution wait begins;
- added registry coverage proving post-cancel phase/input updates do not resurrect tasks.

Acceptance checks:

- cancelled local execution does not reach its side-effect body;
- cancelled task cannot post a normal success result;
- registry remains empty after cancellation;
- unrelated session/turn tasks continue;
- duplicate cancellation is idempotent.

### Milestone 2 — complete execution-owner consolidation

Status: **completed**

Goal: every live, continuation, persistence, recovery, and armature route consumes one typed execution owner.

Audit result:

- Den routing, wait persistence, initial/continuation boundaries, and BearWire projection consume `resolve_tool_execution_owner`;
- checkpoint-name checks remaining in the agent loop implement checkpoint semantics rather than routing ownership;
- armature live and `run.state` recovery paths consume canonical BearWire `execution_target` and project Den-owned calls display-only;
- unknown Den-side ownership fails descriptor resolution before an ambiguous client obligation is created.

Acceptance checks:

- no direct checkpoint-name or `execution_target == "den"` routing outside the resolver/projection boundary;
- unknown owner fails closed;
- run-state recovery never spawns Den-owned local execution;
- Den-owned approval waits remain typed client obligations without changing execution owner.

### Milestone 3 — Den-owned typed liveness

Status: **completed**

Goal: distinguish handshake, provider activity, semantic activity, and legitimate client waits.

Proposed type:

```text
RunLiveness =
  handshake_started_at
  + last_provider_activity_at
  + last_semantic_event_at
  + current_wait_kind
```

Progress:

- completed: web-chat keepalive and absolute turn timeout now own polled Tokio sleep deadlines, so permanently pending model/tool futures still wake for liveness and timeout handling;
- completed: raw OpenAI/Responses provider bytes emit process-local typed `ProviderActivity`; continuation watchdogs distinguish handshake silence, provider inactivity before first semantic output, and later semantic/provider inactivity;
- completed: provider activity is explicitly ignored by BearWire/SSE/transcript projection and does not count as a semantic runtime event;
- completed: initial runs and continuations share the same configured handshake and provider-inactivity deadlines; both consume typed provider activity and only Den decides terminal timeout failure.

Acceptance checks:

- provider chunks prevent false semantic-idle failure;
- no provider activity fails initial and continuation paths consistently;
- client waits do not trigger provider watchdogs;
- web-chat keepalive and wall-clock deadlines schedule their own wakes;
- armature does not decide semantic run liveness.

### Milestone 4 — stale-turn mutation gating

Status: **completed**

Goal: old runs cannot mutate current session/conversation projection state.

Delivered:

- session metadata/title, runtime, and context-budget projections require the current prompt turn token;
- conversation binding and environment publication require the current prompt turn token;
- existing status, message, reasoning, plan, and tool projections retain current-turn gating;
- added a regression proving stale session metadata and conversation binding cannot overwrite current turn state.

Acceptance checks:

- stale turn A updates cannot overwrite current turn B;
- intentional session-global updates use a monotonic version rather than bypassing ordering;
- replay remains deterministic.

### Milestone 5 — observational cache retirement

Status: **completed**

Goal: observational caches cannot become durable state or grow without bound.

Delivered:

- removed the process-global surface tool status registry;
- scoped live BearWire tool-card monotonic dedupe to one adapter instance and current prompt turns;
- session cancel/close clears the session's surface status observations;
- direct local/replay rendering is stateless and does not depend on the live dedupe cache;
- added coverage for session isolation, cleanup, and reused tool-call IDs.

Acceptance checks:

- terminal tool status is evicted or bounded;
- session close/cancel clears session-scoped observations;
- reused tool-call IDs cannot be suppressed by stale process-global state.

### Milestone 6 — obligation-derived blocking reason

Status: **completed**

Goal: obligations become the sole authority for why a run is blocked.

Progress:

1. completed: added typed `BlockingReason` derived from open obligation responder actions;
2. completed: `run.state` exposes the derived blocking reason;
3. completed: new permission/tool waits and coordinator re-waits persist only generic `waiting_for_client`;
4. completed: migration rewrites legacy specialized waits to `waiting_for_client`; removed specialized variants from `TurnRunState`, parser, active-state SQL, and database constraints/indexes.

Acceptance checks:

- every client wait has a matching open obligation;
- no open obligation means no client-wait boundary;
- run-state projection and obligation projection cannot disagree.

### Milestone 7 — workflow/plan authority compilation

Status: **completed**

Audit result:

- production BearWire tool advertisement and read-only prompt envelope consume one `TurnAuthority` compiled from mode/plan inputs;
- no production consumer calls `resolve_session_policy_for_mode` directly;
- removed the dead BearWire convenience wrapper that recomputed authority and made tests construct `TurnAuthority` explicitly;
- existing `den-core` tests preserve submitted/drafting plan lock behavior and prove projection labels cannot expand authority.

### Milestone 8 — governance/permission decoupling

Status: **completed**

Audit result and change:

- governance remains a run-supervision context used by autonomous continuation and runtime projection;
- governance was not used to derive tool permissions, but was stored in `TurnAuthority` as a misleading authority input;
- removed governance from `TurnAuthority` and its constructor, so mutation authority now compiles only stance plus mode/plan policy;
- no new governance axes were added because active runtime behavior does not require them.

### Milestone 9 — projection-only axes

Status: **completed**

Audit result and change:

- model request, prompt/context, memory, compaction, and UI/surface cache state are grouped under one explicitly non-authoritative `DerivedViews` boundary;
- `TurnAuthority` contains only stance and resolved session policy; projection state is not an authority input;
- derived views have no input into terminal lifecycle or focus resolution and cannot manufacture permissions, obligations, focus, or completion;
- the existing compile-time seam regression protects prompt/context/compaction state from becoming a `TurnAuthority` input.

### Milestone 10 — completion as a pure decision

Status: **completed**

Audit result and change:

- `TurnCompletionDecision` is returned only by the pure `decide_turn_completion` policy and consumed immediately by `SessionTrackingStream::evaluate_final_gate_or_complete`;
- no completion decision or policy reason is persisted as peer lifecycle state;
- an accepted completion emits an internal semantic event, while persisted lifecycle completion remains exclusively owned by `finish_run_with_bearwire_event` and its matching durable BearWire terminal event;
- the inventory and ACP contract now distinguish ephemeral completion policy from durable run completion.

## Exit criteria

The roadmap is complete when:

- cancellation, execution ownership, liveness, terminal state, and blocking reason each have one authority;
- armature bookkeeping is observational and current-turn/session bounded;
- no client infers run completion from tool state, assistant text, errors, or EOF;
- projection-only axes cannot expand permissions, create focus, or mutate lifecycle;
- the state inventory names fewer peer authorities than at roadmap start;
- each milestone has a separate validated commit recorded below.

## Commit ledger

| Milestone | Commit | Validation |
| --- | --- | --- |
| Foundation — atomic terminal authority | existing commits through `3bd566be` plus current follow-up commit | bundled-Postgres completion/cancel/expiry/late-result tests; offline checks |
| 1 — durable local-tool cancellation | `1a169ccc` | armature compile; pre-wait cancellation race; matching/unrelated cancellation; session cancel/close; registry eviction/non-resurrection |
| 2 — execution-owner consolidation | `49e60dd4` | resolver usage audit; continuation ownership matrix; Den-owned run-state recovery test; BearWire projection tests |
| 3a — timer-backed web-chat liveness | `f5eafb02` | offline runtime compile; 43/44 native-runtime tests (one unrelated SQLx pool exhaustion) |
| 3b — typed provider activity | `183b111f` | raw-byte activity ordering; projection omission; OpenAI stream detach/tool tests; continuation watchdog tests |
| 3c — initial/continuation liveness parity | `5fba42b3` | initial run completion; shared watchdog configuration tests; OpenAI activity/stream tests; offline BearWire compile |
| 4 — stale-turn mutation gating | `9bcc701d` | stale session-info/binding regression; Den-owned tool turn gate; title roundtrip; armature compile |
| 5 — observational cache retirement | `8db7cac1` | session-scoped cache cleanup/reuse; terminal-card monotonicity; cancel/close cleanup; armature compile |
| 6a — obligation-derived blocking behavior | `6c734959` | blocking derivation unit test; tool/permission wait persistence; coordinator contracts; `run.state` blocking reason; offline compile |
| 6b — retire specialized waiting states | `e5d9090b` | migration/schema/type tests; 14 obligation regressions; 10 coordinator contracts; run-state projection; offline compile |
| 7 — workflow/plan authority compilation | `9af60b1a` | direct-consumer audit; tool surface tests; `TurnAuthority` plan lock/projection tests; warning-free BearWire compile |
| 8 — governance/permission decoupling | `da04bb98` | governance usage audit; 14 client-tool authority tests; BearWire tool surface tests; offline compile |
| 9 — projection-only axes | `6c2c6944` | derived-view authority audit; `den-core` client-tool authority tests; formatting and diff checks |
| 10 — completion as a pure decision | `97e03036` | completion-policy tests; BearWire completed/cancelled terminal tests; formatting and diff checks |
