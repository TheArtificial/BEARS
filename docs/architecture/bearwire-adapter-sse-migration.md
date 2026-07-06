# BearWire adapter-SSE migration map

Status: draft  
Date: 2026-06-18  
Related: [BearWire armature wire implementation plan](../roadmap/BEARWIRE_ARMATURE_WIRE_IMPLEMENTATION_PLAN.md), [BearWire JSON specification](bearwire-json-spec.md)

This table locks the semantic mapping from the legacy Den → armature adapter-SSE payloads to BearWire v1 events. During the parallel-operation period, Den may still emit adapter-SSE on `/acp/**`, but BearWire projection tests assert the same runtime semantic events have equivalent BearWire outcomes.

| Adapter-SSE `type` | BearWire `type` | Notes |
| --- | --- | --- |
| `assistant_text_delta` | `message.delta` | Payload carries `data.delta`; message/run ids are attached by the edge when known. |
| `reasoning_text_delta` | `message.reasoning.delta` | Provider reasoning/thinking delta. Display as ACP thought chunk; not assistant answer content, model replay, conversation history, or task state. |
| `status_text` | `run.progress` | `data.kind = "status_text"`, `data.text` is the display status. |
| `tool_request` with `approval.required = false` | `tool_call.requested` | `data.tool_call_id`, `data.tool_name`, `data.arguments`; armature executes or translates locally. |
| `tool_request` with `approval.required = true` | `client.waiting` | Canonical v1 armature-actionable wait. Payload includes `data.obligation_id`, `data.expected_client_method = "client.permission.result"`, nested `data.tool_call`, and nested `data.permission`. Legacy `tool_call.blocked` may be accepted during migration only when answerable. |
| `permission_request` | `client.waiting` | Permission mediation is an answerable client obligation, not just a display event. Den must persist the obligation before streaming this event. |
| `turn_complete` | `run.completed` | `data.outcome = "ok"`. |
| `turn_result` with ok/recovered status | `run.completed` | Diagnostics remain in `data`; edge may coalesce with `turn_complete` during migration. |
| `turn_result` with error/cancel/timeout status | `run.failed` or `run.cancelled` | `data.reason`, `data.retryable`, and diagnostics are preserved where available. |
| `error` | `run.failed` | `data.message`, `data.detail`, `data.error_type`, `data.context`. Non-terminal warnings may later become `run.warning` or `diagnostic.reported`. |
| `plan_update` | `resource.updated` | `resource_refs` should include a plan resource. |
| `mode_update` | `session.state` | Session mode/write-policy state. |
| `conversation_resolved` | `session.bound` | `data.binding.conversation_id`; resource ref kind `session`. |
| `session_info_update` | `session.state` | Title/meta updates for the session binding. |

## Test coverage

`services/den/crates/den-runtime/src/runtime/bearwire_projection/golden_traces_tests.rs` now includes migration tests that assert the core `RuntimeSemanticEvent` fixtures project to both the legacy adapter-SSE types and the BearWire event types above.
