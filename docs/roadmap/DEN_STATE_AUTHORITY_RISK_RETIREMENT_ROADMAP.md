# Den state-authority risk retirement roadmap

## Status

Active implementation plan. Last updated: 2026-07-18.

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

Status: **in progress**

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
- remaining: apply the same provider-aware watchdog policy to initial runs and remove the initial/continuation watchdog asymmetry.

Acceptance checks:

- provider chunks prevent false semantic-idle failure;
- no provider activity fails initial and continuation paths consistently;
- client waits do not trigger provider watchdogs;
- web-chat keepalive and wall-clock deadlines schedule their own wakes;
- armature does not decide semantic run liveness.

### Milestone 4 — stale-turn mutation gating

Status: **not started**

Goal: old runs cannot mutate current session/conversation projection state.

Targets:

- session metadata/title updates;
- conversation binding;
- usage/context updates;
- plan updates;
- environment publication.

Acceptance checks:

- stale turn A updates cannot overwrite current turn B;
- intentional session-global updates use a monotonic version rather than bypassing ordering;
- replay remains deterministic.

### Milestone 5 — observational cache retirement

Status: **not started**

Goal: observational caches cannot become durable state or grow without bound.

Targets:

- surface tool status registry;
- per-session dedupe state;
- stale task/status records.

Acceptance checks:

- terminal tool status is evicted or bounded;
- session close/cancel clears session-scoped observations;
- reused tool-call IDs cannot be suppressed by stale process-global state.

### Milestone 6 — obligation-derived blocking reason

Status: **not started**

Goal: obligations become the sole authority for why a run is blocked.

Migration order:

1. add typed derived `BlockingReason`;
2. switch runtime decisions and API projection to it;
3. stop branching on specialized persisted waiting labels;
4. remove redundant waiting states only after consumers migrate.

Acceptance checks:

- every client wait has a matching open obligation;
- no open obligation means no client-wait boundary;
- run-state projection and obligation projection cannot disagree.

### Milestones 7–10 — policy and projection simplification

Status: **not started**

Execute only after the external-side-effect and liveness risks above are retired:

- compile workflow/plan state once into `TurnAuthority`;
- split governance into orthogonal typed inputs if branches require it;
- group model/prompt/context/cache/UI state under derived views;
- keep completion as a pure decision whose only durable result is atomic run finish.

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
| 3b — typed provider activity | this sub-item commit | raw-byte activity ordering; projection omission; OpenAI stream detach/tool tests; continuation watchdog tests |
