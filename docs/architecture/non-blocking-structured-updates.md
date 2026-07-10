# Non-blocking structured updates

Status: design direction  
Date: 2026-07-10

## Purpose

Some model/runtime outputs update user-visible or control-plane state but do not provide information the model needs before it can continue. Treating these as ordinary tool calls makes trivial metadata writes look like blocking work and can trigger unnecessary continuation cycles.

This document defines the design boundary between blocking tool exchanges, client obligations, non-blocking structured updates, and ephemeral progress.

## Principle

Model-facing action names should describe intent in simple operational terms. Runtime descriptors, not the model, decide whether an action is blocking, advisory, durable, replayable, or surface-only.

The model does not need to understand the phrase "non-blocking". It needs clear actions such as `report_progress`, `set_conversation_title`, or `update_task_status`; Den decides whether those actions require model-visible results or are handled as structured state updates.

## Taxonomy

| Category | Blocks model continuation? | Model needs result? | Durable/replayable? | Examples |
| --- | ---: | ---: | ---: | --- |
| Blocking tool exchange | Yes | Yes | Yes | `fs_read_text_file`, `git_diff`, `web_fetch` |
| Client obligation | Yes | Yes/decision | Yes | approval, local tool result, human/resource wait |
| Non-blocking structured update | No | Usually no | Usually yes | conversation title, task in-flight status, plan label |
| Ephemeral progress | No | No | Usually no | "Searching files…", phase/progress message |

## Invariant

Non-blocking structured updates may be persisted and projected immediately, but they must not by themselves create open client obligations, require tool-result continuation, or trigger model continuation.

If an update changes state that the model must observe before safely continuing, represent it as a blocking tool exchange or client obligation instead, or inject it into a later model context snapshot through Den-owned context assembly.

## Model-facing affordance

Prefer concrete action names over protocol names:

- Good: `report_progress`, `set_conversation_title`, `update_task_status`, `mark_task_blocked`, `request_handoff`.
- Avoid: `emit_metadata`, `surface_update`, `session_state_patch`.

The model-facing mental model should be function/action syntax, not raw JSON objects or protocol event names. Provider-native tool/function calling is the primary transport when available; Den receives typed parameters even if the model conceptually sees `update_task_status(status=..., summary=...)`.

If a provider or model path lacks native tool calling, the fallback text grammar should be a small function-call DSL:

```text
report_progress(summary="Found likely config mismatch")
set_conversation_title(title="Fix Coolify sandbox roots")
update_task_status(status="in_progress", summary="Reading deployment config")
```

Rules for the fallback grammar:

- one action per line;
- fixed action names from descriptors;
- named parameters only;
- quoted strings with bounded length;
- enum values from the schema;
- no nested objects unless the action genuinely needs them;
- no Markdown fences, XML blocks, or arbitrary JSON blobs in prose.

Keep schemas narrow and enum-backed for smaller models. Dangerous or terminal state changes should be gated or advisory rather than directly authoritative. For example, an in-flight task status update can be non-blocking, while task completion may require validation or a blocking handoff/checkpoint.

## Runtime descriptor semantics

Descriptors should carry action semantics distinct from model-facing names:

```rust
enum ModelActionSemantics {
    BlockingTool,
    ClientObligation,
    NonBlockingUpdate,
    EphemeralProgress,
}
```

Other descriptor metadata can include replay policy, model visibility, durability, rate limits, and whether terminal/gated state changes require validation.

## BearWire projection

Suggested event families:

```text
session.metadata.updated     non-blocking session metadata, e.g. title
work.progress.updated        durable or semi-durable work progress
task.status.updated          Docket/task status projection
run.progress                 ephemeral runtime progress
```

These events are notifications, not client waits. They may be replayed to UI/history if their replay policy says so, but they do not require a `client.tool.result` or `client.permission.result` response.

## Migration guidance

1. Keep existing model-facing tools/actions where they are legible, such as `set_conversation_title`.
2. Prefer provider-native tool/function calling for model → harness communication.
3. Where native tool calling is unavailable, parse only the small function-call DSL above; do not accept free-form JSON/Markdown control blocks.
4. Add descriptor semantics so Den can treat metadata-style actions as non-blocking internally.
5. Project successful metadata actions as typed `session.metadata.updated` / `task.status.updated` events.
6. Avoid making every task/status mutation cause a model continuation nudge. Only gates, approvals, handoffs, and data dependencies should block.
7. Coalesce or rate-limit noisy progress updates from weaker models.

## Industry precedents

This follows established protocol separation:

- JSON-RPC requests require responses; notifications are non-blocking events.
- LSP uses requests for data/actions, notifications for diagnostics and document/config changes, and `$/progress` for out-of-band progress.
- MCP uses `notifications/progress` for long-running operation status.
- CloudEvents standardizes event envelopes for state changes across systems.
- Provider tool-calling APIs make named function calls the de-facto model-facing action format, while transporting parameters as JSON-schema-shaped data. Metadata-only updates may use that same legible affordance, but Den should not semantically treat them as ordinary data-dependent tools when it can avoid it.
