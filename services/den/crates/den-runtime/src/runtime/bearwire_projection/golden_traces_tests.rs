//! Golden adapter-SSE traces (ADR-0043 step-0 safety net).
//!
//! These lock the *wire output* of the runtime → BearWire → adapter-SSE
//! projection so the ADR-0043 rename/relocation of `client_* ` symbols cannot
//! silently change client-visible behavior. They are written entirely against
//! protocol-neutral entry points — `openai_byte_stream_to_event_stream`,
//! `runtime_stream_event_to_bearwire_sse`, and `RuntimeSemanticEvent` — and
//! assert on the serialized SSE payloads, so they remain valid as the
//! `GatewayEvent` overlay moves into the adapter.

use bytes::Bytes;
use futures::StreamExt;

use crate::{
    native_runtime::openai_byte_stream_to_event_stream,
    runtime::bearwire_projection::{
        runtime_stream_event_to_bearwire_sse,
        wire::{
            runtime_semantic_event_to_bearwire_events,
            runtime_stream_event_to_bearwire_notifications,
        },
    },
};
use den_protocol::{
    RuntimeConversationRef, RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent,
    ToolCallFinishStatus,
};

/// Parse one `data: {json}\n\n` adapter-SSE frame into its JSON value.
fn sse_to_json(chunk: &Bytes) -> serde_json::Value {
    let raw = std::str::from_utf8(chunk.as_ref()).expect("utf8 sse");
    let json_str = raw
        .strip_prefix("data: ")
        .expect("adapter sse has `data: ` prefix")
        .trim();
    serde_json::from_str(json_str).expect("adapter sse payload is json")
}

/// Project a single semantic event to its ordered adapter-SSE JSON payloads.
fn project_semantic(event: RuntimeSemanticEvent) -> Vec<serde_json::Value> {
    runtime_stream_event_to_bearwire_sse(RuntimeStreamEvent::Semantic(event))
        .iter()
        .map(sse_to_json)
        .collect()
}

/// Drive raw OpenAI SSE frames through the full pipeline to adapter-SSE JSON.
async fn project_openai_frames(frames: &str) -> Vec<serde_json::Value> {
    let source = futures::stream::iter(vec![Ok::<Bytes, den_core::DenError>(Bytes::from(
        frames.to_string(),
    ))]);
    let mut stream = openai_byte_stream_to_event_stream(source);
    let mut payloads = Vec::new();
    while let Some(item) = stream.next().await {
        let event = item.expect("openai byte stream event");
        payloads.extend(
            runtime_stream_event_to_bearwire_sse(event)
                .iter()
                .map(sse_to_json),
        );
    }
    payloads
}

fn types(payloads: &[serde_json::Value]) -> Vec<&str> {
    payloads
        .iter()
        .map(|p| p["type"].as_str().expect("payload has type"))
        .collect()
}

fn bearwire_types(event: RuntimeSemanticEvent) -> Vec<String> {
    runtime_semantic_event_to_bearwire_events(event)
        .into_iter()
        .map(|event| event.event_type)
        .collect()
}

fn assert_type_mapping(
    event: RuntimeSemanticEvent,
    adapter_types: &[&str],
    bearwire_types_expected: &[&str],
) {
    let adapter_payloads = project_semantic(event.clone());
    assert_eq!(types(&adapter_payloads), adapter_types);
    assert_eq!(bearwire_types(event), bearwire_types_expected);
}

// --- End-to-end OpenAI SSE → adapter SSE ------------------------------------

#[tokio::test]
async fn golden_trace_text_then_tool_call() {
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Reviewing proposals.\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_golden_1\",\"function\":{\"name\":\"memory_read\",\"arguments\":\"{\\\"path\\\":\\\"pair/notes/demo.md\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    );
    let payloads = project_openai_frames(frames).await;

    // A `tool_calls` finish must NOT synthesize a turn_complete.
    assert_eq!(
        types(&payloads),
        vec!["assistant_text_delta", "tool_request"]
    );

    assert_eq!(
        payloads[0],
        serde_json::json!({"type": "assistant_text_delta", "text": "Reviewing proposals."})
    );

    // Lock the stable wire contract of a tool_request (ids/name/args/approval/transport).
    let tool = &payloads[1];
    assert_eq!(tool["type"], "tool_request");
    assert_eq!(tool["tool_call_id"], "call_golden_1");
    assert_eq!(tool["tool_name"], "memory_read");
    assert_eq!(
        tool["args"],
        serde_json::json!({"path": "pair/notes/demo.md"})
    );
    assert_eq!(tool["approval"]["required"], false);
    assert_eq!(tool["diagnostic"]["transport_version"], 4);
}

#[tokio::test]
async fn golden_trace_text_then_stop_completes_turn() {
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Done.\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    );
    let payloads = project_openai_frames(frames).await;
    assert_eq!(
        payloads,
        vec![
            serde_json::json!({"type": "assistant_text_delta", "text": "Done."}),
            serde_json::json!({"type": "turn_complete", "outcome": "ok"}),
        ]
    );
}

#[tokio::test]
async fn golden_trace_multi_text_delta_preserves_each_chunk() {
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello \"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"world.\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    );
    let payloads = project_openai_frames(frames).await;
    assert_eq!(
        payloads,
        vec![
            serde_json::json!({"type": "assistant_text_delta", "text": "Hello "}),
            serde_json::json!({"type": "assistant_text_delta", "text": "world."}),
            serde_json::json!({"type": "turn_complete", "outcome": "ok"}),
        ]
    );
}

// --- Adapter-SSE → BearWire migration type parity ---------------------------

#[test]
fn reasoning_delta_bearwire_event_is_display_only() {
    let events = runtime_semantic_event_to_bearwire_events(
        RuntimeSemanticEvent::ReasoningTextDelta {
            text: "thinking".to_string(),
        },
    );

    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.event_type, "message.reasoning.delta");
    assert_eq!(event.data["delta"], "thinking");
    assert_eq!(event.data["source"], "provider_reasoning");
    assert_eq!(event.data["replay_policy"], "none");
}

#[test]
fn migration_type_mapping_covers_core_semantic_events() {
    assert_type_mapping(
        RuntimeSemanticEvent::AssistantTextDelta {
            text: "Hello".to_string(),
        },
        &["assistant_text_delta"],
        &["message.delta"],
    );
    assert_type_mapping(
        RuntimeSemanticEvent::ReasoningTextDelta {
            text: "Thinking".to_string(),
        },
        &["reasoning_text_delta"],
        &["message.reasoning.delta"],
    );
    assert_type_mapping(
        RuntimeSemanticEvent::StatusText {
            text: "Thinking".to_string(),
        },
        &["status_text"],
        &["run.progress"],
    );
    assert_type_mapping(
        RuntimeSemanticEvent::ConversationResolved {
            conversation: RuntimeConversationRef {
                id: "conv-123".to_string(),
            },
        },
        &["conversation_resolved"],
        &["session.bound"],
    );
    assert_type_mapping(
        RuntimeSemanticEvent::TurnCompleted { turn: None },
        &["turn_complete"],
        &["run.completed"],
    );
    assert_type_mapping(
        RuntimeSemanticEvent::RunProgress {
            kind: "indexing".to_string(),
            text: Some("Indexing".to_string()),
            phase: None,
            detail: None,
        },
        &["status_text"],
        &["run.progress"],
    );
}

#[test]
fn migration_type_mapping_covers_tool_and_error_semantics() {
    assert_type_mapping(
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: "call-1".to_string(),
            tool_name: "memory_read".to_string(),
            title: Some("Read memory".to_string()),
            kind: Some("function".to_string()),
            arguments: serde_json::json!({"path": "pair/notes/demo.md"}),
            approval_request_id: None,
            approval_required: false,
            approval_reason: None,
            run_id: Some("run-1".to_string()),
        },
        &["tool_request"],
        &["tool_call.requested"],
    );
    assert_type_mapping(
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: "call-2".to_string(),
            tool_name: "web_fetch".to_string(),
            title: None,
            kind: None,
            arguments: serde_json::json!({"url": "https://example.com"}),
            approval_request_id: Some("perm-1".to_string()),
            approval_required: true,
            approval_reason: Some("network".to_string()),
            run_id: Some("run-2".to_string()),
        },
        &["tool_request"],
        &["client.waiting"],
    );
    let waiting_events =
        runtime_semantic_event_to_bearwire_events(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: "call-wait".to_string(),
            tool_name: "fs_edit_file".to_string(),
            title: Some("Edit file".to_string()),
            kind: Some("function".to_string()),
            arguments: serde_json::json!({"path": "/workspace/README.md"}),
            approval_request_id: Some("perm-wait".to_string()),
            approval_required: true,
            approval_reason: Some("permission required".to_string()),
            run_id: Some("run-wait".to_string()),
        });
    assert_eq!(waiting_events.len(), 1);
    let waiting = &waiting_events[0];
    assert_eq!(waiting.event_type, "client.waiting");
    assert_eq!(
        waiting.data["expected_client_method"],
        "client.permission.result"
    );
    assert_eq!(waiting.data["tool_call"]["id"], "call-wait");
    assert_eq!(waiting.data["tool_call"]["name"], "fs_edit_file");
    assert_eq!(
        waiting.data["tool_call"]["arguments"]["path"],
        "/workspace/README.md"
    );
    assert_eq!(waiting.data["permission"]["id"], "perm-wait");
    assert_eq!(waiting.data["permission"]["reason"], "permission required");

    assert_type_mapping(
        RuntimeSemanticEvent::ToolCallFinished {
            tool_call_id: "call-3".to_string(),
            tool_name: "memory_read".to_string(),
            status: ToolCallFinishStatus::Ok,
            summary: Some("done".to_string()),
            error_message: None,
        },
        &["status_text"],
        &["tool_call.completed"],
    );
    assert_type_mapping(
        RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: RuntimeErrorCategory::Timeout,
            message: "timed out".to_string(),
        },
        &["error"],
        &["run.failed"],
    );
}

#[test]
fn bearwire_projection_serializes_as_json_rpc_event_notification() {
    let notifications = runtime_stream_event_to_bearwire_notifications(
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta {
            text: "Hello".to_string(),
        }),
    );
    assert_eq!(notifications.len(), 1);
    let value = serde_json::to_value(&notifications[0]).expect("serialize notification");
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["method"], "event");
    assert_eq!(value["params"]["type"], "message.delta");
    assert_eq!(value["params"]["data"]["delta"], "Hello");
}

// --- Semantic event → adapter SSE (full payloads) ---------------------------

#[test]
fn golden_assistant_text_delta_payload() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::AssistantTextDelta {
            text: "Reviewing proposals.".to_string(),
        }),
        vec![serde_json::json!({"type": "assistant_text_delta", "text": "Reviewing proposals."})]
    );
}

#[test]
fn golden_status_text_payload() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::StatusText {
            text: "Indexing memory.".to_string(),
        }),
        vec![serde_json::json!({"type": "status_text", "text": "Indexing memory."})]
    );
}

#[test]
fn golden_run_progress_prefers_text_then_kind() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::RunProgress {
            kind: "indexing".to_string(),
            text: Some("Indexing 5 files".to_string()),
            phase: None,
            detail: None,
        }),
        vec![serde_json::json!({"type": "status_text", "text": "Indexing 5 files"})]
    );
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::RunProgress {
            kind: "indexing".to_string(),
            text: None,
            phase: None,
            detail: None,
        }),
        vec![serde_json::json!({"type": "status_text", "text": "indexing"})]
    );
}

#[test]
fn golden_run_paused_awaiting_approval_payload() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::RunPaused {
            reason: "awaiting_approval".to_string(),
            resume_token: None,
            expires_at: None,
        }),
        vec![serde_json::json!({"type": "status_text", "text": "Waiting for approval."})]
    );
}

#[test]
fn golden_conversation_resolved_payload() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::ConversationResolved {
            conversation: RuntimeConversationRef {
                id: "den-conv-golden-1".to_string(),
            },
        }),
        vec![serde_json::json!({
            "type": "conversation_resolved",
            "conversation_id": "den-conv-golden-1",
        })]
    );
}

#[test]
fn golden_turn_completed_payload() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::TurnCompleted { turn: None }),
        vec![serde_json::json!({"type": "turn_complete", "outcome": "ok"})]
    );
}

#[test]
fn golden_error_payload_full_shape() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::Error {
            message: "provider unavailable".to_string(),
            detail: Some("upstream reset".to_string()),
            error_type: Some("runtime_unavailable".to_string()),
            request_id: Some("req-golden-1".to_string()),
            context: None,
        }),
        vec![serde_json::json!({
            "type": "error",
            "message": "provider unavailable",
            "detail": "upstream reset",
            "error_type": "runtime_unavailable",
            "request_id": "req-golden-1",
        })]
    );
}

#[test]
fn golden_turn_failed_maps_category_to_error_type() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: RuntimeErrorCategory::Timeout,
            message: "runtime timed out".to_string(),
        }),
        vec![serde_json::json!({
            "type": "error",
            "message": "runtime timed out",
            "detail": null,
            "error_type": "runtime_timeout",
        })]
    );
}

#[test]
fn golden_turn_cancelled_maps_to_cancelled_error() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::TurnCancelled { turn: None }),
        vec![serde_json::json!({
            "type": "error",
            "message": "Runtime continuation was cancelled.",
            "detail": null,
            "error_type": "runtime_turn_cancelled",
        })]
    );
}

#[test]
fn golden_tool_call_finished_ok_is_status_text() {
    assert_eq!(
        project_semantic(RuntimeSemanticEvent::ToolCallFinished {
            tool_call_id: "call-golden-2".to_string(),
            tool_name: "memory_read".to_string(),
            status: ToolCallFinishStatus::Ok,
            summary: Some("Read 3 entries".to_string()),
            error_message: None,
        }),
        vec![serde_json::json!({"type": "status_text", "text": "Read 3 entries"})]
    );
}

#[test]
fn golden_tool_call_finished_error_emits_status_then_error() {
    let payloads = project_semantic(RuntimeSemanticEvent::ToolCallFinished {
        tool_call_id: "call-golden-3".to_string(),
        tool_name: "memory_write".to_string(),
        status: ToolCallFinishStatus::Error,
        summary: None,
        error_message: Some("disk full".to_string()),
    });
    assert_eq!(types(&payloads), vec!["status_text", "error"]);
    assert_eq!(
        payloads[0],
        serde_json::json!({"type": "status_text", "text": "disk full"})
    );
    assert_eq!(
        payloads[1],
        serde_json::json!({
            "type": "error",
            "message": "disk full",
            "detail": "Tool `memory_write` returned an error.",
            "error_type": "tool_execution_error",
        })
    );
}

#[test]
fn golden_untranslated_provider_event_yields_no_sse() {
    let mapped =
        runtime_stream_event_to_bearwire_sse(RuntimeStreamEvent::UntranslatedProviderEvent {
            value: serde_json::json!({"message_type": "provider_only"}),
        });
    assert!(mapped.is_empty());
}
