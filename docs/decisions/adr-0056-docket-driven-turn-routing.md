# ADR-0056: Docket-driven turn routing

**Status:** Accepted; Phases 0–3 implemented
**Date:** 2026-07-17 (revised 2026-07-20)  
**Amends:** [ADR-0034: Jobs and Tasks Work-Management Model](adr-0034-jobs-and-tasks-work-management.md), [ADR-0043: ACP as edge adapter over a protocol-agnostic core](adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md), [ADR-0045: Session task lists as Docket checkouts and working projections](adr-0045-session-task-lists-and-docket-checkout.md), [ADR-0053: Stance-scoped delegated runs](adr-0053-stance-scoped-delegated-runs.md)  
**Related:** [ADR-0033: Model tasks layer](adr-0033-model-tasks-layer.md), [ADR-0050: Agent loop control, adaptive budgets, and runtime checkpoints](adr-0050-agent-loop-control-adaptive-budgets-and-runtime-checkpoints.md), [ADR-0051: Reflection performance assessments](adr-0051-reflection-performance-assessments.md)

> **State-inventory maintenance.** This ADR changes focus authority: it splits today's conversation-scoped "focus" mechanism into durable conversation bindings and volatile per-session cursors, and makes run state the sole authority for execution position. Implementation must update the [Den state machine inventory](../architecture/den-state-machine-inventory.md) and include tests that stale cursors and stale projections cannot force continuation, per ADR-0045's standing obligation.

## Context

Den currently treats a **conversation** as the durable user-visible chat container and a **session** as a live client or adapter binding to a conversation. This works for ordinary chat, but it is a poor fit for focused task execution over a Docket task tree.

A Docket job may contain a hierarchy of tasks:

```text
Job
├── Task A
│   ├── Task A1
│   └── Task A2
└── Task B
```

The **leading case for this ADR is autonomous job completion**: a `work`-stance background run (and a focused `pair` session driven by the ADR-0045 continuation gate) walking the task tree to completion without per-turn user input. The interactive case matters too — users want to navigate the tree conversationally:

```text
continue
go deeper
try the next one
summarize this and go back
work on the blocked child
```

But these are two points on one axis: **who originates each turn**. A focused `pair` session under the continuation gate is already semi-autonomous; a fully autonomous `work` run is the same machinery with no user steering. The design must serve turn origination generally, not privilege the interactive case.

If all work happens in one long-running conversation, the transcript becomes noisy and poorly scoped. Child investigations, tool-heavy execution, and side explorations can overwhelm the parent coordination context.

At the same time, exposing every internal conversation split directly to users would create unnecessary UX burden. Some clients, such as ACP or other chat-like APIs, want to present the illusion of one continuous running interaction. Other clients will be custom Den applications that show the Docket task tree explicitly in a GUI and need robust APIs for task navigation, routing, execution state, conversation binding, and transcript review.

We need a model that supports all three:

1. **autonomous execution**, where a dispatcher advances through the tree and every placement decision is recorded and browsable after the fact;
2. **chat-like routed sessions**, where multiple underlying conversations may appear as one continuous interaction; and
3. **explicit task-tree GUI applications**, where the user sees the Docket hierarchy and can open, route, delegate, resume, or inspect task-scoped conversations directly — including while an autonomous run is executing.

We also need a way to distinguish tasks that are worth a separate scoped execution context from tasks that should continue in the current conversation. This correlates with model selection, tool access, autonomy level, and subagent behavior — but those concerns have existing owners (ADR-0033's model-tasks layer, ADR-0053's delegation broker). The concept here is a routing and execution decision over a Docket task, not a fork of a conversation and not a new model-selection mechanism.

## Decision

Den will model focused task work as **Docket-driven turn routing**.

A routed interaction is composed of:

```text
Turn source
  originates each turn: a user, the continuation gate, the dispatcher,
  or a rollup trigger

Client session / UI surface
  presents an interaction model to the user (absent for pure dispatch)

Docket cursor
  per-session attention viewport over a Docket job/task tree

Turn router
  resolves each turn intent to a target conversation and task focus

Conversation
  owns durable transcript containment for a scoped line of work

Docket
  owns durable job/task identity, hierarchy, status, metadata,
  and run-scoped execution state
```

"Focus" is not a durable mode of a long-running conversation. Task focus is derived at turn time from the turn intent, the originating cursor (if any), and resolved Docket state.

The system will support multiple projections over the same underlying model:

- The autonomous dispatcher is a **peer client of the router**: a `work` run advancing to the next task is a routing decision, recorded identically to a user's navigation.
- ACP and chat-like sessions may present one continuous stream while internally routing turns across multiple conversations.
- GUI applications may show the Docket task tree explicitly and allow users to navigate, inspect, and control routing decisions — including watching an autonomous run's position without disturbing it.
- Debugging and audit tools may expose the full mapping between sessions, tasks, conversations, runs, and tool calls.

## Terminology

### Turn intent

A **turn intent** is the router's input: a request to execute one turn somewhere in a Docket tree.

```text
turn_intent:
  source: user | continuation | dispatch | rollup
  session_id?          -- present for user/continuation sources
  cursor?              -- the originating session's viewport, if any
  input?               -- user text, when source = user
  job_id / task_id?    -- explicit target, when source = dispatch
```

- `user` — a human sent input through a client session.
- `continuation` — the ADR-0045 continuation gate is driving a focused session forward without user input.
- `dispatch` — the Docket dispatcher (today, the work-run worker) is advancing a job.
- `rollup` — a child task reached a terminal state and its result must be summarized and surfaced to the parent context.

Every turn, autonomous or interactive, enters the router as a turn intent. There is exactly one placement mechanism; the dispatcher does not have a private one.

### Execution surface

The armature is Den's universal tool-execution harness — sandboxed work runs execute through a Den-provisioned armature in an isolated workspace, and `pair` executes through the user's live armature. A run's **execution surface** is which armature/workspace attachment its tool calls land on:

```text
sandbox   Den-provisioned armature, isolated workspace, egress-gated (ADR-0037)
armature  a user's live armature session attached to their local work surface
```

This is a third independent axis alongside ADR-0053's two:

```text
stance  = authority domain          (chat | pair | work | curate | watch)
mode    = who originates turns      (foreground | background)
surface = where tools execute       (armature | sandbox)
```

Stance never encodes substrate: **`work` on an armature is still `work`.** There is no new stance for armature-attached execution; the surface is an explicit authorization dimension.

Surface selection rules:

- From `pair`, the default is always the session's attached work surface via its own armature. Sandbox dispatch happens only by explicit user choice.
- From `chat` or UI, dispatch defaults to the sandbox; an armature target must be named explicitly.
- The surface is selected only from a typed API/UI field, an already authorized session attachment, or the documented source default — **never inferred from prose**. `pair` therefore does not re-confirm its existing attached armature on every turn. If a request explicitly asks to change surface without naming one, the model resolves that ambiguity through the single elicitation tool (ADR-0050), because changing the surface changes the risk envelope.
- The resolved surface is recorded on every routing decision.

**Armature-attached dispatch** (unattended `work` on a user's armature) carries these semantics:

- The run acts on the **live working tree**; a dirty tree produces a non-blocking warning at dispatch, not a refusal.
- The armature's local permission profile remains authoritative for command execution, and the job's `commit_policy`/autonomy governs writes. Any action that would interactively prompt instead **blocks the run** for review — unattended runs never answer their own prompts.
- Armature disconnect **auto-pauses** the run; a bounded timeout converts the pause to a failed run with a failure rollup.

### Docket cursor

A **Docket cursor** is a per-session **attention viewport** over a Docket task tree:

```text
job_id
task_id?
```

The cursor answers:

> What is this session currently looking at?

It does **not** answer "what task is underway" — that is run-scoped execution state (`in_progress` per ADR-0034), which remains unique per run and serialized per job. Cursors are plural: any number of sessions may hold cursors over the same tree simultaneously, and none of them contends with the others or with an executing run. A cursor resting on a completed, blocked, or otherwise terminal task is a normal browsing state, not an error.

A cursor grants no authority. Before each turn, Den resolves the intent against current Docket task state, conversation state, and runtime policy.

For a purely autonomous run with no observers, no durable cursor is required at all: the run's execution position *is* its viewport. Cursors earn their keep the moment a second point of attention exists — a user browsing, a `pair` session coordinating at the parent while a child executes, an auditor reading history.

### Docket-driven turn routing

**Docket-driven turn routing** is the process of mapping a turn intent to:

```text
target conversation
resolved job/task focus
execution/routing policy
visible response stream (when a session is attached)
post-turn cursor update (for the originating session only)
```

The router is responsible for deciding whether a turn should continue in the current conversation, resume an existing task-scoped conversation, create a new scoped conversation, or return to a parent coordination conversation.

### Routed session

A **routed session** is a live client or adapter session that presents an interaction over one or more durable conversations.

For ACP or chat-like clients, this may create the illusion of one continuous chat session. For GUI clients, the session may instead expose the routing structure explicitly.

### Conversation binding

A **conversation binding** associates a Docket task with a preferred conversation, plus the historical record of which conversation each run used:

```text
task_id -> preferred conversation_id
run_id  -> conversation_id            (historical, per run)
```

This allows a child investigation or task-specific execution context to be resumed later, and allows the GUI to list every conversation that ever touched a task.

The durable half of today's `docket_execution_sessions` record — the (job, run, task) ↔ conversation linkage — becomes the conversation binding. See **Migration from the current focus mechanism**.

### Scoped conversation

A **scoped conversation** is a durable conversation used for a particular line of work, usually associated with a Docket task.

A scoped conversation may be:

- the parent coordination conversation;
- a child task investigation conversation;
- a tool-heavy execution conversation;
- a delegated work conversation;
- a review or summarization conversation.

## Routing model

Each turn follows this conceptual flow:

```text
1. Receive a turn intent (user input, continuation, dispatch, or rollup).
2. Read the originating cursor, if a session is attached.
3. Resolve the intent against current Docket state and run state.
4. Determine the target task and routing policy.
5. Select or create the target conversation.
6. Execute the turn with resolved task focus.
7. Persist outputs to the conversation and relevant Docket task/run state.
8. Update the originating session's cursor and task/conversation bindings.
9. Emit a response through the client session's UI projection, if any.
```

The important invariant is:

```text
Cursor is attention.
Run state is execution position.
Docket is task authority.
Conversation is transcript containment.
Router is placement policy.
UI is projection.
```

The router respects ADR-0034's dispatch invariants: at most one `in_progress` task per run, sibling execution serialized by `sibling_order`, and no intra-job fan-out in v1. Conversation placement never changes execution sequencing; scoping a task's transcript does not license executing it concurrently with its siblings.

### Claim and commit contract

Routing is not a read-then-act race. For a dispatch or continuation turn, Den must atomically compare the expected job/run version, claim the eligible task for that run, and reserve a stable turn idempotency key before model or tool execution begins. The claim is run-scoped, has one owner, and carries a renewable lease so a crashed worker cannot hold it forever. A competing worker that loses the compare-and-swap observes the winner's state and does not execute the turn.

Conversation creation and binding use the same idempotency key. A retry therefore resumes the reserved conversation/turn rather than creating a second scoped conversation. External side effects remain subject to each tool's own idempotency and permission contract; the router does not claim that arbitrary tool calls are transactionally reversible.

The durable sequence is:

```text
resolve candidate from current Docket state
  -> atomically claim task/run position and reserve turn key
  -> persist immutable routing decision
  -> execute the turn
  -> persist transcript and outcome
  -> atomically finalize task/run state and append rollup/outbox events
  -> release claim and select again only at the next boundary
```

Task completion must compare the run id, task id, attempt, claim owner, and expected state version. A late result from an earlier attempt is retained as attempt history but cannot complete the current attempt.

## Conversation placement rules

Den should prefer boring default behavior. For `dispatch` and `continuation` intents, placement is a deterministic policy over task metadata — there is nobody to negotiate with. For `user` intents, interactive forks (reopen a completed task, choose an execution surface) resolve through the single elicitation tool (ADR-0050) rather than a negotiation protocol.

Initial routing heuristics:

```text
If the selected task already has a bound conversation:
  route to that conversation.

Else if the selected task is a sibling with difficulty = trivial:
  continue in the current conversation.

Else if the selected task is marked as requiring scoped execution:
  create a scoped conversation and bind it to the task.

Else if the selected task is a child task and likely non-trivial:
  create a scoped conversation (dispatch/continuation)
  or offer one (user).

Else:
  continue in the current conversation.
```

Sibling tasks usually remain in the same conversation because they often share context:

```text
Task A: Fix validation copy
Task B: Add loading state
Task C: Update tests
```

Child tasks are candidates for scoped conversations because they often represent narrower or noisier work:

```text
Task A: Fix flaky auth tests
  Task A1: Investigate token expiry behavior
  Task A2: Patch helper
  Task A3: Verify CI behavior
```

However, child tasks should not always force a new conversation. Creating a scoped conversation for every child would create clutter. Routing must be guided by task metadata, turn source, and execution policy.

## Task routing metadata

Tasks need metadata indicating whether they are worth separate scoped handling.

This should not be modeled narrowly as:

```text
fork: true
```

"Fork" is an implementation detail of one routing strategy. The durable task metadata should instead describe the desired execution shape.

Recommended shape:

```text
routing_strategy:
  inline
  scoped
  delegated
  auto
```

Where:

- `inline` means the task should usually continue in the current conversation.
- `scoped` means the task should usually get its own scoped conversation.
- `delegated` means the task should be handled as a stance-scoped delegated run per ADR-0053: the router submits a delegation request; the broker resolves stance and mints capabilities. Until the broker exists, `delegated` resolves to the work-run lane (see **Delegated framing** below).
- `auto` means the router may choose based on task kind, difficulty, expected noise, turn source, or user/client preference.

For v1, `auto` is deliberately not a policy engine. It is a typed, deterministic rule table: an existing valid binding is resumed; trivial siblings are `inline`; non-trivial investigations and child tasks are `scoped`; tasks assigned to the work lane are `delegated`; all other tasks are `inline`. Explicit `inline | scoped | delegated` metadata wins. The decision records the matched rule. Configurable scoring, learned heuristics, and `auto` sub-policy hints are deferred.

Additional advisory fields may refine placement and rollup:

```text
expected_context_size?
result_rollup_policy?
```

Tasks already carry the advisory descriptors the router and model-tasks layer need: `kind`, `difficulty`, and `effort_hint` (ADR-0034). **No model or execution-profile identifier is stored on task rows.** ADR-0034 is explicit that model names do not live on tasks, and this ADR upholds that: execution profiles are resolved at dispatch time by the ADR-0033 model-tasks layer and recorded on the routing decision and the run, never on the task. This keeps model economics centrally tunable — a newly approved cheap model becomes eligible without touching a single task — and keeps every model choice auditable per run.

Example:

```json
{
  "task_id": "task-A1",
  "title": "Investigate token expiry behavior",
  "kind": "investigation",
  "difficulty": "moderate",
  "routing_strategy": "delegated",
  "result_rollup_policy": "summary_to_parent"
}
```

This allows the same Docket task model to support:

- same-conversation execution;
- separate scoped conversations;
- delegated work;
- autonomous dispatch;
- GUI-visible task routing;
- ACP-style chat illusions.

## Execution profiles and model economy

Scoped routing is not only a transcript-hygiene measure. **Scoping is the cost strategy.**

The reason a cheaper model can complete a delegated task is that its context is small: a bead-shaped, self-contained task body (ADR-0034), explicitly filtered context (ADR-0053), and a rollup contract instead of the parent transcript. A task routed `inline` inherits the parent conversation's accumulated context and therefore effectively its model tier. Only `scoped` and `delegated` tasks are cheap-model candidates. Routing strategy and model economy are coupled, and the router is where that coupling is exercised.

Resolution pipeline, at dispatch/routing time:

```text
task descriptors (kind, difficulty, effort_hint, expected_context_size)
  × stance
  × Bear model library
  × model registry
  -> ModelRequestProfile   (ADR-0033)
```

The resolved profile is recorded on the routing decision and the run. Policy rules:

1. **Cheap by default, escalate on typed signals.** Each attempt starts at the cheapest profile the task's difficulty allows. Escalation is driven by signals that already exist or are planned: task-gate rejection loops (ADR-0045), ko/loop-health checkpoints (ADR-0050), and terminal `Failed`/`TimedOut` work runs with the existing `attempt` counter. "Attempt N+1 resolves one tier higher" is router policy, not model judgment.
2. **Price the control overhead, not just the tokens.** ADR-0050 mirrors control levels to model capability: weaker models run `careful`, with earlier checkpoints and stronger nudges. The true cost of a cheap delegated run is model price × expected turns × checkpoint overhead. Escalating one tier is sometimes cheaper than three careful failed attempts. The escalation policy lives where both signals are visible — loop control plus the model-tasks layer — which is a further reason profiles do not live on Docket rows.
3. **Models may recommend, policy decides.** Per ADR-0033, a capable controller model may recommend bounded delegation of a subtask to a cheaper approved model, and a weaker model may request escalation; runtime/model-task policy validates both directions, and routing targets are symbolic model refs only.
4. **Close the loop with recorded decisions.** Persisted routing decisions plus ADR-0034's chronic-vs-anomalous-effort observability form the dataset for tuning the difficulty → profile mapping. ADR-0051 reflection assessments may grade rollup quality per profile. Rollup generation itself is a cheap, schema-validated model task.

Delivery is staged: router v1 begins with a small typed profile enum and deterministic registry mapping. It records the symbolic profile and attribution from the first turn, but bounded automatic escalation is enabled only after claim/recovery behavior and attribution are verified. There is no general profile DSL in v1.

## Delegated framing

For many tasks, "forking a conversation" is less important than selecting an execution lane.

A `delegated` task resolves to a **stance-scoped delegated run** (ADR-0053): the router submits a delegation request; the broker resolves the narrowest capable stance, authorizes from first principles, and mints scoped capabilities. Router placement must never become a bypass around delegation authorization — the router decides *where the transcript lives*, the broker decides *what the run may do*.

A delegated task implies:

```text
separate scoped conversation
execution profile resolved by the model-tasks layer
capability bundle minted by the delegation broker
bounded task objective
explicit completion criteria
result rollup returned to parent
```

The user-facing concept can be:

> This task is worth assigning to a subagent.

Internally, that creates a delegated run with a scoped conversation bound to the task. The durable Docket concept is the routing strategy and the run — not the mechanical conversation fork.

**v1 realization:** the delegation broker does not yet exist. The one delegated lane in production is the `work` work-run pipeline (enqueue → claim → provision → checkout → turn → report → finalize, serialized per job). In v1, `routing_strategy: delegated` on a `work`-assignable task means "enqueue a work run"; the broker generalizes this later without changing task metadata.

## ACP and chat-like UI behavior

ACP and similar chat-like APIs may expose a single apparent session:

```text
send_message(session_id, text)
receive_events(session_id)
```

Internally, Den may route turns like this:

```text
Turn 1 -> conversation C_parent, task A
Turn 2 -> conversation C_child_1, task A1
Turn 3 -> conversation C_child_1, task A1
Turn 4 -> conversation C_parent, task A
Turn 5 -> conversation C_parent, task B
```

The user sees a coherent stream:

```text
Assistant: Let's investigate token expiry.
User: ok
Assistant: Found the issue.
User: summarize and go back.
Assistant: Back at the parent task. A1 is complete. Next is A2.
```

The ACP adapter may hide conversation routing by default, while still exposing metadata for traceability:

```json
{
  "session_id": "session-123",
  "job_id": "job-456",
  "task_id": "task-A1",
  "conversation_id": "conv-child-789",
  "parent_conversation_id": "conv-parent-111",
  "cursor": {
    "job_id": "job-456",
    "task_id": "task-A1"
  }
}
```

The illusion of one running session is purely a UI projection, implemented in the ACP adapter over the core router per ADR-0043. It is not a claim that the agent loop is running inside one durable conversation. The illusion is **not v1 scope** (see Scope below); the current one-session-one-conversation binding remains valid for chat-like clients until the adapter projection is built.

## GUI application behavior

Custom Den applications should be able to show the Docket tree explicitly — including while an autonomous run executes. Because cursors are per-session viewports, a browsing user never contends with the run: their cursor is a camera; the run's `in_progress` task is the truth.

A GUI may expose:

- this session's cursor position, and optionally other sessions' cursors;
- the executing run's position (from run state, not from any cursor);
- job/task hierarchy and status;
- task routing strategy;
- task conversation bindings and per-run conversation history;
- child scoped conversations and their transcripts;
- delegated runs;
- result rollups;
- routing decisions and transitions;
- resolved execution profiles per run.

Example UI structure:

```text
Job: Improve auth reliability

▸ Task A: Fix flaky auth tests
  Status: in progress        ← run position (authoritative)
  Conversation: parent coordination

  ▸ Task A1: Investigate token expiry behavior
    Status: done
    Routing: delegated
    Conversation: child investigation      ← your cursor is here
    Result: Token expiry mock used wall clock instead of test clock.

  ▸ Task A2: Patch test helper
    Status: ready
    Routing: scoped

  ▸ Task A3: Verify CI behavior
    Status: pending
    Routing: inline
```

Human vocabulary is a strict subset of the model's. Client copy should surface **job, task, task list, run status, results, and approvals** — the vocabulary the Phase-1 UX surfaces already standardize. Conversation bindings, other sessions' cursors, routing decisions, and turn intents are trace/debug tier: available to every client, shown by default in none.

The GUI should not need to reverse-engineer this from transcripts. Den should provide first-class APIs for Docket cursors, routing decisions, task/conversation bindings, transcripts, and result rollups. Transcript review rests on ADR-0043's fenced requirement that tool activity is core replay state: every model-relevant tool call and result is persisted in a replayable Den transcript shape, so a log-review UI reads core state, not adapter decoration.

## Den API requirements

Den needs a robust API layer that treats Docket-driven routing as a first-class capability.

Minimum conceptual API resources:

```text
DocketJob
DocketTask
DocketCursor            (per session; many per tree)
Conversation
ConversationBinding
RoutingDecision
JobRun                  (bear_job_runs)
TaskRunState            (bear_task_run_state)
WorkRun                 (bear_work_runs)
ResultRollup            (run-scoped task events)
```

Note the three run-shaped records — turn runs (BearWire), job runs, and work runs — are named honestly here rather than collapsed into a fictional unified `TaskRun`. The router sits at their seam; unifying them is acknowledged debt, not something this ADR pretends is done.

Possible API operations:

```text
getJob(job_id)
getTask(task_id)
listTaskTree(job_id)

getCursor(session_id)
setCursor(session_id, job_id, task_id)
listCursors(job_id)                      -- all sessions' viewports

routeTurn(turn_intent)

pauseRun(run_id)
resumeRun(run_id)
stopRun(run_id)                          -- terminal for the run; job stays resumable

getTaskConversations(task_id)            -- preferred binding + per-run history
bindTaskConversation(task_id, conversation_id)
createScopedConversation(task_id, parent_conversation_id)
getConversationTranscript(conversation_id, page)

getRoutingStrategy(task_id)
setRoutingStrategy(task_id, strategy)

listRoutingDecisions(job_id | task_id | run_id)
streamJobEvents(job_id)                  -- task/run/rollup/routing events, live

completeTask(run_id, task_id, attempt, expected_state_version,
             result_summary, completion_evidence)  -- finalizes attempt + records rollup
```

The API should distinguish between:

- navigation state (cursors — plural, unprivileged);
- durable task state;
- run-scoped execution state;
- transcript containment;
- execution/routing policy;
- UI projection.

It should not require every client to understand ACP-specific behavior.

## Routing decision shape

The router should produce an inspectable decision object for **every** routed turn, autonomous or interactive. The routing decision is immutable placement intent: it records the resolved inputs, policy version/rule, target, binding, surface, profile, attempt, and stable turn idempotency key. It is never rewritten to imply that execution succeeded.

Execution progress is a separate lifecycle keyed to the decision:

```text
reserved -> executing -> completed | failed | abandoned
```

A crash may therefore leave an inspectable `reserved` or `executing` attempt without falsifying the placement record. Recovery either resumes the same idempotent turn or marks that attempt `abandoned` before a new attempt is claimed. Transcript/run outcome events carry the actual result.

Example:

```json
{
  "turn_source": "dispatch",
  "session_id": null,
  "cursor_before": null,
  "target": {
    "job_id": "job-456",
    "task_id": "task-A1"
  },
  "conversation": {
    "strategy": "create_scoped",
    "conversation_id": "conv-child-789",
    "parent_conversation_id": "conv-parent-111"
  },
  "execution": {
    "routing_strategy": "delegated",
    "surface": "sandbox",
    "resolved_profile": "code-investigation/tier-1",
    "attempt": 1
  },
  "cursor_after_policy": "none",
  "reason": "Child investigation task is marked delegated and has no existing scoped conversation."
}
```

The `conversation.strategy` vocabulary is deliberately small — `continue_current | resume_bound | create_scoped`. Interactive forks that need user input (reopen a completed task, choose an execution surface) go through the single elicitation tool (ADR-0050), not a negotiation vocabulary in the decision object. This object should be available to GUI clients, logs, audit tools, and debugging surfaces. ACP clients may ignore most of it.

## Watching and steering

Observation and steering of a running job follow **bear rights — jobs introduce no new ACL.** Anyone whose current authorization grants access to the Bear **and to the referenced conversation/work surface** may watch the permitted conversation projection and job event stream live from any surface (`pair`, `chat`, GUI). Bear membership is not a shortcut around conversation, workspace, tool-output, or secret-redaction policy. APIs must authorize each transcript/resource read at request time; clients receive redacted metadata when they may see the job but not a bound transcript.

Watching is pull; **attention is push**. Blocked, decision-needed, completed, and failed events must reach the user as notifications on their active surface, with a link back to the job — not wait in a pull-only queue. Silent stalling is autonomy's primary failure mode: an unattended run that blocks invisibly has not "completed autonomously," it has quietly stopped.

Docket owns the durable attention event; it does not deliver UI notifications directly. Finalization appends the event and a notification-outbox record in the same transaction. Session/adapter notification workers own presentation, delivery attempts, and acknowledgements. The stable deduplication key is `(recipient, job_id, run_id, event_sequence, channel)`, so recovery is at-least-once without presenting the same event twice on one channel. An unacknowledged delivery remains retryable; an active surface may coalesce events but must preserve the link to each durable event.

Steering is deliberately small:

```text
run control:  start | pause | resume | stop
task tree:    ordinary audited Docket mutations
```

There is no parallel command channel. Revising an objective is editing the job goal, criteria, or task bodies. Answering a question is updating the blocked `decision` task. Adding context is editing the relevant task body or adding a child task. Narrowing scope is cancelling tasks. All of these flow through the existing task/job edit paths with their existing audit events and policy — which is what keeps cross-stance steering authentic: a `chat` session steering a `work` run never touches the run's tools; it mutates shared Docket state and issues run-control verbs, nothing more. (This narrows ADR-0053's `ParentCommand` vocabulary for Docket-driven runs.)

Pickup semantics:

- Tree edits land in Docket immediately and take effect at the next turn/task boundary — steering never yanks a task mid-flight. Mutations compare the current tree/state version; stale edits are rejected for refresh rather than silently rebased.
- Editing, reparenting, or deleting the currently in-progress task requires pausing the run first. Pending-task edits may land while execution continues and are re-resolved at the next boundary.
- `pause` first records `pause_requested`. The active model stream or tool call is allowed to reach the next safe boundary; only then does the run become `paused`. Den does not report `paused` while a side effect is in flight.
- `stop` is terminal for the run; the job remains resumable via a new run. Like pause, stop is honored at a safe boundary unless the active operation supports safe cancellation.
- `pause` and `pause_requested` are durable run state, not session state — the pausing client may disconnect.

## Result rollup

When a scoped child task reaches a terminal state, Den records a **rollup event**: an append-only, run-scoped `bear_task_events` entry carrying a concise result. This upholds ADR-0034's invariant that task rows hold no results — run-scoped `result_summary` fields remain the raw material; the rollup event is the durable, parent-visible record. Terminal means `completed | failed | cancelled | timed_out | stopped`; every terminal child produces a rollup, not only successful children. Failure-shaped rollups state what happened, what evidence or partial output is usable, whether retry is safe, and what human action is required.

Example:

```text
Task A1 rollup (run r-77):
Token expiry tests were flaky because the helper used wall-clock time
instead of the injected test clock. Patch should update the helper to use
the test clock consistently.
```

The parent context reads the **latest rollup per child**, ordered by the canonical per-run task-event sequence assigned transactionally by Docket — never by wall-clock timestamp. A child conversation that continues after an earlier summary simply appends a new rollup event. The API may maintain a replaceable latest-per-child read projection for efficiency, but event history remains canonical and the projection is rebuildable.

For autonomous execution the rollup is the *entire product* of a child task: no human is present to synthesize across transcripts mid-run. The rollup contract is therefore strict — schema-validated, tied to completion-criteria evidence — and rollup generation is itself a cheap, validated model task (ADR-0033).

Then the parent coordination context can show:

```text
A1 complete. Root cause found. Ready for A2: patch helper.
```

This avoids requiring the parent conversation to include every detailed child transcript.

## Recovery contract

Recovery is deterministic and uses durable state, not worker memory:

- An unexpired claim remains owned; another worker does not steal it. After lease expiry, recovery inspects persisted transcript/outcome state before reclaiming.
- A reserved decision with no execution may be resumed with the same turn key or abandoned. It is never treated as completed.
- If transcript execution completed but finalization did not, the same idempotent finalization compares attempt/version and appends at most one terminal rollup and outbox event.
- A stale worker cannot finalize after its lease or attempt was superseded.
- `pause_requested` survives restart and prevents selection of another turn; recovery advances it to `paused` once no operation is in flight.
- Notifications are retried from the durable outbox and deduplicated by their stable delivery key.
- On recovery, a job selects a new task only after the prior attempt is durably terminal or abandoned.

The smallest required recovery checks cover duplicate workers racing for one task, a crash between transcript persistence and finalization, a stale late result, and notification replay.

## Staleness and authority

A cursor, session, or client-side projection must not resurrect stale work.

Before each turn, Den must resolve:

```text
turn intent × Docket task state × run state × conversation state × policy
```

A cursor resting on a completed, cancelled, or blocked task is a normal **browsing** state — inspection needs no authority. Staleness only matters when a turn intent tries to *act* there. In that case the router must degrade safely:

```text
If task is completed:
  ask whether to reopen, inspect, or move to next task.

If task is blocked:
  show blocker and offer parent/sibling navigation.

If task no longer exists:
  move cursor to nearest valid ancestor or clear focus.

If bound conversation is unavailable:
  create a new scoped conversation only if policy allows.
```

Continuation authority belongs to resolved task state, run state, and runtime policy — never to a cursor.

## Migration from the current focus mechanism

The codebase today implements conversation-scoped "focus": `docket_execution_sessions` binds (conversation, client session) to (job, run, task); `execute_job` emits `focus_selected` task events; the conversation title carries a `⌖` focus prefix; and mode changes clear focus. This is precisely the "focus mode on a conversation" shape that Alternative 1 below rejects — it conflates the two halves this ADR separates.

Migration splits it along the attention/authority seam:

1. **`docket_execution_sessions` → ConversationBinding.** The durable (job, run, task) ↔ conversation linkage it already records *is* the conversation binding. It gains the task-to-preferred-conversation role and per-run history; it loses any claim to being "where the session is pointed."
2. **Conversation focus → cursor projection.** The `⌖` title, `conversation_has_active_focus`, and `clear_focus_for_mode_change` become projections and behaviors of the originating session's cursor, not durable conversation state. `focus_selected` events remain as the audit trail of routing decisions selecting a task, and are subsumed by the richer RoutingDecision record.
3. **Run state keeps execution authority.** `RuntimeFocusContext` resolution continues to derive from durable Docket execution state — this ADR narrows what the volatile session cache may claim, exactly as the existing `ponytail` note in `focus_context.rs` anticipated ("upgrade to an explicit conversation focus record if focus needs history, labels, or multi-job stacks").

No behavior regresses for the single-actor case: when the only attention on a tree is the executing run itself, binding + run state reproduce today's focus semantics without a cursor record at all.

## Delivery sequence

V1 is a release made of small vertical slices, not one schema-and-UI change:

1. Add strong routing/surface/decision types and persistence.
2. Route one existing dispatcher turn through the router, including atomic claim and idempotent conversation binding.
3. Add recovery and stale-worker checks before unattended continuation.
4. Add success and failure rollups, deterministic latest-per-child projection, and evidence-checked completion.
5. Continue serially at clean boundaries until the job completes or blocks.
6. Add durable pause/resume/stop and version-checked boundary-safe edits.
7. Add `pair` cursors, browsing, transcript access, and event streaming.
8. Add notification outbox delivery and acknowledgement.
9. Enable bounded profile escalation after attribution data and recovery checks exist.

No slice introduces parallel task execution, arbitrary mid-operation mutation, a policy DSL, or a generalized event-sourcing framework.

## Scope (v1)

Autonomous execution **leads**. In scope for v1:

- turn intents and the router as the single placement mechanism, with the work-run dispatcher as its first client;
- deterministic placement policy over `routing_strategy` and task descriptors;
- conversation bindings (migrated from `docket_execution_sessions`) and scoped-conversation creation;
- persisted routing decisions;
- rollup events and the parent read path;
- execution-profile resolution via a small typed model-tasks mapping, with the resolved profile and cost/latency attribution recorded from the first turn; bounded automatic escalation follows after recovery and attribution are verified;
- **`pair` visibility and task-tree mutation**: per-session cursors, tree/transcript/browsing APIs, live job-event streaming, and `pair`-side task-tree edits through the existing checkout/sync surface (ADR-0045) plus user-editable `routing_strategy`;
- **run steering**: pause/resume/stop plus tree-mutation-as-steering with boundary pickup, rights keyed to bear rights.

Explicitly deferred beyond v1:

- configurable/learned escalation or routing policy beyond the fixed typed tiers and three-attempt supervisor ceiling;
- a configurable/learned `auto` policy beyond the fixed v1 rule table;

- armature-attached dispatch (unattended `work` on a user's armature; semantics fixed in **Execution surface** above, build deferred);
- the ACP one-continuous-session illusion (adapter projection; the core router makes it possible per ADR-0043, but no chat client requires it yet);
- the ADR-0053 delegation broker (`delegated` resolves to the work-run lane until then);
- intra-job fan-out (ADR-0034 deferral stands);
- `auto` promotion heuristics beyond a trivial rule table.

## Consequences

### Benefits

- Autonomous job completion and interactive navigation share one placement mechanism and one audit record.
- Keeps conversations scoped and readable.
- Supports chat-like, GUI task-tree, and headless-dispatch interfaces over the same model.
- Any number of observers can browse a tree while a run executes, without coordination.
- Provides first-class API concepts for routing, task navigation, conversation binding, and transcript review.
- Couples scoped routing to model economy: small scoped contexts are what make cheap-model delegation viable, and every model choice is recorded and tunable.
- Makes result rollup explicit, append-only, and run-scoped instead of relying on giant transcripts or mutable task rows.
- Steering needs no new protocol or ACL: run control plus ordinary Docket mutation, under existing bear rights and audit — so watching and steering a live run works identically from `pair`, `chat`, or a GUI.
- Avoids treating focus as a long-running conversation mode, and defines the migration off the existing focus mechanism rather than leaving it ambiguous.

### Costs

- Requires a router layer between turn sources and conversations.
- Requires durable conversation-binding state and (for interactive sessions) cursor state.
- Requires APIs for inspecting and controlling routing decisions.
- Introduces more concepts for client authors.
- Requires careful handling of stale cursors and task state changes.
- Requires UI affordances so the chat illusion does not become confusing.
- The three run-shaped records (turn runs, job runs, work runs) remain un-unified under the router; that debt is named but not paid here.

### Risks

#### Illusion leakage

A chat-like UI may hide too much. Users may be confused if a decision happened in a hidden child conversation.

Mitigations:

- expose lightweight task transition messages;
- include collapsible child transcript summaries;
- provide metadata for trace/debug clients;
- persist rollup events on Docket task history.

#### Over-scoping

If every child task creates a new conversation, the system will create clutter — and autonomous dispatch will happily create conversations nobody asked for.

Mitigations:

- use `routing_strategy`;
- default trivial tasks to `inline`;
- reserve `scoped` or `delegated` for noisy, long-running, or meaningfully independent work;
- clutter is less user-facing under dispatch (the GUI browses the tree, not a conversation list), but placement policy should still be conservative.

#### Under-scoping

If too much work remains in one conversation, parent context becomes noisy — and cheap-model delegation becomes impossible, because inline tasks inherit the parent's context size.

Mitigations:

- mark investigations, tool-heavy work, and autonomous execution as `scoped` or `delegated`;
- allow router to promote `auto` tasks into scoped conversations based on observed complexity.

#### Runaway model spend under autonomy

Every dispatch turn is machine-initiated; cost multiplies with no human brake.

Mitigations:

- cheap-by-default profile resolution with typed escalation only;
- ADR-0050 budgets and control levels remain authoritative over continuation;
- routing decisions record resolved profile and attempt, so spend is attributable per task and tunable.

#### API overfitting to ACP

ACP is only one UI projection.

Mitigations:

- design APIs around Den concepts: Docket, turn intent, cursor, routing decision, conversation binding;
- treat the ACP chat illusion as an adapter behavior, not the core architecture (ADR-0043).

## Alternatives considered

### Alternative 1: Focus mode on a conversation

Treat a conversation as being "in focus mode" for a task or job.

Rejected — **and this is the shape the current code implements** (`docket_execution_sessions` + conversation focus title + clear-on-mode-change). It conflates durable transcript containment with task authority and UI state, makes multiple observers impossible without contention, and makes it harder to route child tasks into scoped conversations while preserving a coherent parent coordination context. See **Migration from the current focus mechanism** for how the existing records split into bindings and cursors rather than being discarded.

### Alternative 2: One conversation per job

Store the entire job execution in one long conversation.

Rejected. This is simple but produces noisy transcripts and poor boundaries for child investigations, delegated work, and tool-heavy execution — and it forecloses cheap-model delegation, since every task inherits the whole job's context.

### Alternative 3: One conversation per task, always

Create a separate conversation for every Docket task.

Rejected. This gives clean boundaries but creates excessive conversation clutter and makes simple sibling tasks unnecessarily fragmented.

### Alternative 4: Expose conversation splits directly as the primary API

Make clients explicitly create and manage conversation splits.

Rejected as the core model. Conversation topology is an implementation detail of routing. Some clients should see it, but task-oriented clients should primarily reason in terms of Docket tasks, cursors, and routing strategies.

### Alternative 5: Cursor as execution pointer

Define the cursor as "the task currently underway," which a single work surface's serial-mutation demand makes well-defined.

Rejected. That quantity already exists — it is run-scoped execution state, unique per run under ADR-0034 — and needs no new concept. Defining the cursor as the execution pointer forecloses the multi-observer case: browsing a done task's rollup while a run executes would either be inexpressible or would yank execution; cursor-on-terminal-task becomes a contradiction instead of a normal browsing state; and multiple clients sharing a job becomes a locking problem instead of a non-event. Serial mutation does not imply serial attention. The cursor wanders; run state stays serial.

## Resolved questions

Previously open questions this revision resolves:

1. **Is `delegated` a routing strategy or an execution profile?** A routing strategy that resolves to an ADR-0053 brokered delegation (v1: the work-run lane). Execution profiles are a separate, model-tasks-layer concern resolved at dispatch and recorded on the decision and run.
2. **Cursor scope?** Per session. Cursors are plural, unprivileged viewports; execution position is run state.
3. **Rollup versioning when a child continues after a summary?** Rollups are append-only run-scoped task events; the parent reads latest-per-child. History, not versions.
4. **Multi-client cursor coordination?** None needed. Cursors never contend; only run state is exclusive, and it already serializes per job.
5. **Conversation-binding cardinality?** Task-to-preferred-conversation, plus per-run historical conversation refs.
6. **How is model selection surfaced in the Docket API?** It isn't stored there. Tasks carry advisory descriptors (`kind`, `difficulty`, `effort_hint`, `expected_context_size`); the model-tasks layer resolves symbolic profiles at dispatch; routing decisions and runs record the resolution.
7. **Should routing hints be user-editable in GUI clients?** Yes — `routing_strategy` is durable task metadata editable through the same authorized task-edit path as other definition fields.
8. **What are steering rights, and who may see cursors and transcripts?** Existing Bear rights remain the job-level authority; each referenced conversation/work-surface resource is also authorized and redacted at read time. Jobs add no ACL and do not broaden transcript access.
9. **What is the steering vocabulary?** Run control (`start | pause | resume | stop`) plus ordinary audited, version-checked Docket tree mutation, picked up at safe turn/task boundaries; editing the in-progress task requires a confirmed pause. No parallel command channel.
10. **How is the execution surface chosen?** From a typed API/UI field, an authorized existing session attachment, or a documented source default — never from prose. `pair` uses its attached armature without repetitive confirmation; changing surface ambiguously uses elicitation.
11. **Is armature-attached work a new stance?** No. Surface is an authorization axis, not a stance; `work` via armature is `work` — indeed sandboxed work also executes through a (Den-provisioned) armature.
12. **How are duplicate workers and crashes handled?** An atomic version-checked leased claim plus stable turn key serializes execution. Decisions are immutable intent; attempt lifecycle records outcomes; idempotent finalization and an outbox recover rollups and notifications.
13. **What does a terminal rollup cover?** Success, failure, cancellation, timeout, and stop. Docket event sequence, not timestamps, defines latest-per-child.
14. **Who delivers notifications?** Docket transactionally records the durable attention event and outbox entry; adapters/sessions deliver and acknowledge it with a stable per-channel deduplication key.
15. **What is `auto` in v1?** The four-value enum is `inline | scoped | delegated | auto`; `auto` is a fixed typed rule table, not a policy engine.

## Open questions

1. How much routing metadata should ACP expose by default when the illusion projection is built?
2. What evidence and operational data should justify replacing the fixed `auto` rule table or enabling bounded profile escalation?
3. When should the three run-shaped records (turn runs, job runs, work runs) be unified, and under what name?

## Summary

Den will support focused task work through **Docket-driven turn routing**.

Every turn — user-originated, continuation-driven, or dispatched — enters one router as a turn intent and is placed into an appropriate scoped conversation, with the decision recorded. Autonomous `work` execution is the leading client of the router; `pair` sessions gain visibility and task-tree mutation over the same machinery; ACP-style chat illusions are a deferred adapter projection.

Cursors are per-session attention viewports — plural, unprivileged, free to rest anywhere in the tree — while execution position belongs to run state alone. Tasks carry routing and advisory metadata; execution profiles are resolved by the model-tasks layer at dispatch, cheap-by-default with typed escalation, so scoped routing doubles as the cost-efficiency mechanism.

Watching and steering follow bear rights: observation is a transcript/event subscription; steering is run control (`start | pause | resume | stop`) plus ordinary audited task-tree mutation with boundary pickup. Execution surface (armature or sandbox) is an explicit, recorded choice — `pair` acts on its own attached work surface by default, and dispatch to the sandbox or to an armature is always deliberate.

The core principle is:

```text
Docket owns work structure.
Run state owns execution position.
Cursors own attention — one per session, many per tree.
Router owns turn placement, for every turn source.
Execution surface is explicit — armature or sandbox, never inferred.
Conversation owns transcript scope.
UI owns presentation.
```
