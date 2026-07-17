# ADR-0056: Docket-driven turn routing

**Status:** Proposed  
**Date:** 2026-07-17  
**Amends:** [ADR-0034: Jobs and Tasks Work-Management Model](adr-0034-jobs-and-tasks-work-management.md), [ADR-0043: ACP as edge adapter over a protocol-agnostic core](adr-0043-acp-as-edge-adapter-protocol-agnostic-core.md), [ADR-0045: Session task lists as Docket checkouts and working projections](adr-0045-session-task-lists-and-docket-checkout.md), [ADR-0053: Stance-scoped delegated runs](adr-0053-stance-scoped-delegated-runs.md)

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

Users often want to navigate this tree conversationally:

```text
continue
go deeper
try the next one
summarize this and go back
work on the blocked child
```

If all work happens in one long-running conversation, the transcript becomes noisy and poorly scoped. Child investigations, tool-heavy execution, and side explorations can overwhelm the parent coordination context.

At the same time, exposing every internal conversation split directly to users would create unnecessary UX burden. Some clients, such as ACP or other chat-like APIs, want to present the illusion of one continuous running interaction. Other clients will be custom Den applications that show the Docket task tree explicitly in a GUI and need robust APIs for task navigation, routing, execution state, and conversation binding.

We need a model that supports both:

1. **chat-like routed sessions**, where multiple underlying conversations may appear as one continuous interaction; and
2. **explicit task-tree GUI applications**, where the user sees the Docket hierarchy and can open, route, delegate, resume, or inspect task-scoped conversations directly.

We also need a way to distinguish tasks that are worth a separate scoped execution context from tasks that should continue in the current conversation. This may correlate with model selection, tool access, autonomy level, or subagent behavior. Therefore the concept should not be framed only as "forking a conversation". It should be modeled as a routing and execution decision over a Docket task.

## Decision

Den will model focused task work as **Docket-driven turn routing**.

A routed interaction is composed of:

```text
Client session / UI surface
  presents an interaction model to the user

Docket cursor
  identifies the current position in a Docket job/task tree

Turn router
  resolves each turn to a target conversation and task focus

Conversation
  owns durable transcript containment for a scoped line of work

Docket
  owns durable job/task identity, hierarchy, status, metadata, and result summaries
```

"Focus" is not a durable mode of a long-running conversation. Instead, task focus is derived from the current Docket cursor and resolved Docket state at turn time.

The system will support multiple UI projections over the same underlying model:

- ACP and chat-like sessions may present one continuous stream while internally routing turns across multiple conversations.
- GUI applications may show the Docket task tree explicitly and allow users to navigate, inspect, and control routing decisions.
- Debugging and audit tools may expose the full mapping between sessions, tasks, conversations, runs, and tool calls.

## Terminology

### Docket cursor

A **Docket cursor** is navigational state pointing at a location in a Docket task tree.

At minimum:

```text
job_id
task_id?
```

The cursor answers:

> Where in the Docket tree is this interaction currently pointed?

It does not by itself grant authority to continue work. Before each turn, Den resolves the cursor against current Docket task state, conversation state, and runtime policy.

### Docket-driven turn routing

**Docket-driven turn routing** is the process of mapping an incoming turn to:

```text
target conversation
resolved job/task focus
execution/routing policy
visible response stream
post-turn cursor update
```

The router is responsible for deciding whether a turn should continue in the current conversation, resume an existing task-scoped conversation, create a new scoped conversation, or return to a parent coordination conversation.

### Routed session

A **routed session** is a live client or adapter session that presents an interaction over one or more durable conversations.

For ACP or chat-like clients, this may create the illusion of one continuous chat session.

For GUI clients, the session may instead expose the routing structure explicitly.

### Conversation binding

A **conversation binding** associates a Docket task with a preferred or existing conversation.

Example:

```text
task_id -> conversation_id
```

This allows a child investigation or task-specific execution context to be resumed later.

### Scoped conversation

A **scoped conversation** is a durable conversation used for a particular line of work, usually associated with a Docket task.

A scoped conversation may be:

- the parent coordination conversation;
- a child task investigation conversation;
- a tool-heavy execution conversation;
- a subagent work conversation;
- a review or summarization conversation.

## Routing model

Each incoming turn follows this conceptual flow:

```text
1. Receive user input from a client session.
2. Read the session's Docket cursor.
3. Resolve the cursor against current Docket state.
4. Determine the target task and routing policy.
5. Select or create the target conversation.
6. Execute the turn with resolved task focus.
7. Persist outputs to the conversation and relevant Docket task/run state.
8. Update cursor and task/conversation bindings as needed.
9. Emit a response through the client session's UI projection.
```

The important invariant is:

```text
Cursor is navigation.
Docket is task authority.
Conversation is transcript containment.
Router is placement policy.
UI is projection.
```

## Conversation placement rules

Den should prefer boring default behavior.

Initial routing heuristics:

```text
If the selected task already has a bound conversation:
  route to that conversation.

Else if the selected task is a simple sibling task:
  continue in the current conversation.

Else if the selected task is marked as requiring scoped execution:
  create a scoped conversation and bind it to the task.

Else if the selected task is a child task and likely non-trivial:
  create or offer a scoped conversation.

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

However, child tasks should not always force a new conversation. Creating a scoped conversation for every child would create clutter. Routing must be guided by task metadata, user intent, and execution policy.

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
- `delegated` means the task should be handled as a delegated or subagent-style work unit, likely with its own conversation, model choice, tool constraints, run lifecycle, and result summary.
- `auto` means the router may choose based on task kind, difficulty, expected noise, model policy, tool use, or user/client preference.

Additional fields may refine this:

```text
execution_profile_id?
model_hint?
tool_policy_id?
autonomy_level?
expected_context_size?
result_rollup_policy?
```

Example:

```json
{
  "task_id": "task-A1",
  "title": "Investigate token expiry behavior",
  "kind": "investigation",
  "difficulty": "moderate",
  "routing_strategy": "delegated",
  "execution_profile_id": "code-investigation",
  "result_rollup_policy": "summary_to_parent"
}
```

This allows the same Docket task model to support:

- same-conversation execution;
- separate scoped conversations;
- delegated work;
- model-specific subagents;
- GUI-visible task routing;
- ACP-style chat illusions.

## Delegated and subagent framing

For many tasks, "forking a conversation" is less important than selecting an execution lane.

A delegated or subagent-style task may imply:

```text
separate scoped conversation
different model selection
different tool permissions
different prompt context
bounded task objective
explicit completion criteria
result summary returned to parent
```

The user-facing concept can be:

> This task is worth assigning to a subagent.

Internally, that may create a new scoped conversation. But the durable Docket concept should be the execution profile and routing strategy, not the mechanical conversation fork.

This avoids overfitting the API to transcript topology.

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

The illusion of one running session is purely a UI projection. It is not a claim that the agent loop is running inside one durable conversation.

## GUI application behavior

Custom Den applications should be able to show the Docket tree explicitly.

A GUI may expose:

- current cursor position;
- job/task hierarchy;
- task status;
- task routing strategy;
- task conversation bindings;
- child scoped conversations;
- delegated or subagent runs;
- result summaries;
- parent/child rollups;
- routing transitions;
- model/execution profile choices.

Example UI structure:

```text
Job: Improve auth reliability

▸ Task A: Fix flaky auth tests
  Status: in progress
  Conversation: parent coordination

  ▸ Task A1: Investigate token expiry behavior
    Status: done
    Routing: delegated
    Conversation: child investigation
    Result: Token expiry mock used wall clock instead of test clock.

  ▸ Task A2: Patch test helper
    Status: ready
    Routing: scoped

  ▸ Task A3: Verify CI behavior
    Status: pending
    Routing: inline
```

The GUI should not need to reverse-engineer this from transcripts. Den should provide first-class APIs for Docket cursors, routing decisions, task/conversation bindings, and result rollups.

## Den API requirements

Den needs a robust API layer that treats Docket-driven routing as a first-class capability.

Minimum conceptual API resources:

```text
DocketJob
DocketTask
DocketCursor
Conversation
ConversationBinding
RoutingDecision
ExecutionProfile
TaskRun
ResultRollup
```

Possible API operations:

```text
getJob(job_id)
getTask(task_id)
listTaskTree(job_id)

getCursor(session_id)
setCursor(session_id, job_id, task_id)
moveCursor(session_id, direction)

previewTurnRoute(session_id, input)
routeTurn(session_id, input)

getTaskConversation(task_id)
bindTaskConversation(task_id, conversation_id)
createScopedConversation(task_id, parent_conversation_id)

getRoutingStrategy(task_id)
setRoutingStrategy(task_id, strategy)

listExecutionProfiles()
assignExecutionProfile(task_id, profile_id)

completeTask(task_id, result_summary)
rollupTaskResult(task_id, parent_task_id)
```

The API should distinguish between:

- navigation state;
- durable task state;
- transcript containment;
- execution/routing policy;
- UI projection.

It should not require every client to understand ACP-specific behavior.

## Routing decision shape

The router should produce an inspectable decision object.

Example:

```json
{
  "session_id": "session-123",
  "cursor_before": {
    "job_id": "job-456",
    "task_id": "task-A"
  },
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
    "execution_profile_id": "code-investigation",
    "model_hint": "larger-context-code-model"
  },
  "cursor_after_policy": "remain_on_target",
  "reason": "Child investigation task is marked delegated and has no existing scoped conversation."
}
```

This object should be available to GUI clients, logs, audit tools, and debugging surfaces.

ACP clients may ignore most of it.

## Result rollup

When a scoped child task completes, Den should persist a concise result on the Docket task and make it available to the parent.

Example:

```text
Task A1 result_summary:
Token expiry tests were flaky because the helper used wall-clock time
instead of the injected test clock. Patch should update the helper to use
the test clock consistently.
```

Then the parent coordination context can show:

```text
A1 complete. Root cause found. Ready for A2: patch helper.
```

This avoids requiring the parent conversation to include every detailed child transcript.

## Staleness and authority

A cursor, session, or client-side projection must not resurrect stale work.

Before each turn, Den must resolve:

```text
cursor × Docket task state × conversation state × policy
```

If the cursor points to a completed, cancelled, deleted, blocked, or otherwise unavailable task, the router must degrade safely.

Example behaviors:

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

Continuation authority belongs to resolved task state and runtime policy, not to the UI cursor alone.

## Consequences

### Benefits

- Keeps conversations scoped and readable.
- Supports both chat-like and GUI task-tree interfaces.
- Allows ACP to present one continuous session without forcing one durable conversation.
- Provides first-class API concepts for routing, task navigation, and conversation binding.
- Enables subagent-style task execution without overloading conversation split semantics.
- Makes result rollup explicit instead of relying on giant transcripts.
- Avoids treating focus as a long-running conversation mode.

### Costs

- Requires a router layer between sessions and conversations.
- Requires durable cursor and conversation-binding state.
- Requires APIs for inspecting and controlling routing decisions.
- Introduces more concepts for client authors.
- Requires careful handling of stale cursors and task state changes.
- Requires UI affordances so the chat illusion does not become confusing.

### Risks

#### Illusion leakage

A chat-like UI may hide too much. Users may be confused if a decision happened in a hidden child conversation.

Mitigations:

- expose lightweight task transition messages;
- include collapsible child transcript summaries;
- provide metadata for trace/debug clients;
- persist result summaries on Docket tasks.

#### Over-scoping

If every child task creates a new conversation, the system will create clutter.

Mitigations:

- use `routing_strategy`;
- default simple tasks to `inline`;
- reserve `scoped` or `delegated` for noisy, long-running, or meaningfully independent work.

#### Under-scoping

If too much work remains in one conversation, parent context becomes noisy.

Mitigations:

- mark investigations, tool-heavy work, and autonomous execution as `scoped` or `delegated`;
- allow router to promote `auto` tasks into scoped conversations based on observed complexity.

#### API overfitting to ACP

ACP is only one UI projection.

Mitigations:

- design APIs around Den concepts: Docket, cursor, routing decision, conversation binding, and execution profile;
- treat the ACP chat illusion as an adapter behavior, not the core architecture.

## Alternatives considered

### Alternative 1: Focus mode on a conversation

Treat a conversation as being "in focus mode" for a task or job.

Rejected.

This conflates durable transcript containment with task authority and UI state. It also makes it harder to route child tasks into scoped conversations while preserving a coherent parent coordination context.

### Alternative 2: One conversation per job

Store the entire job execution in one long conversation.

Rejected.

This is simple but produces noisy transcripts and poor boundaries for child investigations, delegated work, and tool-heavy execution.

### Alternative 3: One conversation per task, always

Create a separate conversation for every Docket task.

Rejected.

This gives clean boundaries but creates excessive conversation clutter and makes simple sibling tasks unnecessarily fragmented.

### Alternative 4: Expose conversation splits directly as the primary API

Make clients explicitly create and manage conversation splits.

Rejected as the core model.

Conversation topology is an implementation detail of routing. Some clients should see it, but task-oriented clients should primarily reason in terms of Docket tasks, cursors, execution profiles, and routing strategies.

## Open questions

1. What should the exact `routing_strategy` enum be?
2. Should `delegated` be a routing strategy, an execution profile, or both?
3. Should task routing hints be user-editable in GUI clients?
4. How much routing metadata should ACP expose by default?
5. Should cursor state be per session, per user, per conversation, or explicitly scoped?
6. How should result rollups be versioned when child conversations continue after an earlier summary?
7. What policy determines when an `auto` task is promoted from `inline` to `scoped`?
8. How should multiple users or clients sharing the same Docket job coordinate cursors?
9. Should conversation bindings be one-to-one, one-to-many, or task-to-preferred-conversation with historical runs?
10. How should model selection and execution profile constraints be surfaced in the Docket API?

## Summary

Den will support focused task work through **Docket-driven turn routing**.

A client session may present a single continuous interaction, but Den routes each turn through a Docket cursor to an appropriate scoped conversation. ACP and chat-like APIs may use this to create a "one running session" illusion. GUI applications may instead expose the Docket tree, cursor, routing decisions, scoped conversations, and delegated runs directly.

Tasks will carry routing and execution metadata indicating whether they should be handled inline, in a scoped conversation, by a delegated/subagent-style execution profile, or automatically by router policy.

The core principle is:

```text
Docket owns work structure.
Cursor owns navigation.
Router owns turn placement.
Conversation owns transcript scope.
UI owns presentation.
```
