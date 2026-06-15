use bytes::Bytes;
use futures::StreamExt;

use den_runtime::{
    gateway_events::gateway_event_to_adapter_sse,
    native_runtime::openai_byte_stream_to_event_stream,
    runtime::bearwire_projection::{
        runtime_semantic_event_to_bearwire_gateway_events, runtime_stream_event_to_bearwire_sse,
    },
    runtime_contracts::{
        RuntimeConversationRef, RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent,
    },
};

fn adapter_sse_event_types(sse_bytes: &[Bytes]) -> Vec<String> {
    sse_bytes
        .iter()
        .filter_map(|chunk| {
            let raw = std::str::from_utf8(chunk.as_ref()).ok()?;
            let json_str = raw.strip_prefix("data: ")?.trim();
            let value: serde_json::Value = serde_json::from_str(json_str).ok()?;
            value
                .get("type")
                .and_then(|item| item.as_str())
                .map(str::to_string)
        })
        .collect()
}

fn semantic_events_to_adapter_types(events: Vec<RuntimeStreamEvent>) -> Vec<String> {
    events
        .into_iter()
        .flat_map(runtime_stream_event_to_bearwire_sse)
        .flat_map(|chunk| adapter_sse_event_types(&[chunk]))
        .collect()
}

async fn collect_adapter_types_from_openai_sse(frames: &str) -> Vec<String> {
    let source = futures::stream::iter(vec![Ok::<Bytes, den_core::DenError>(
        Bytes::from(frames.to_string()),
    )]);
    let mut stream = openai_byte_stream_to_event_stream(source);
    let mut types = Vec::new();
    while let Some(item) = stream.next().await {
        let event = item.expect("openai byte stream event");
        types.extend(semantic_events_to_adapter_types(vec![event]));
    }
    types
}

#[tokio::test]
async fn golden_openai_sse_text_and_tool_call_projects_ordered_adapter_types() {
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Reviewing proposals.\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_golden_1\",\"function\":{\"name\":\"memory_read\",\"arguments\":\"{\\\"path\\\":\\\"pair/notes/demo.md\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    );
    let types = collect_adapter_types_from_openai_sse(frames).await;
    assert_eq!(
        types,
        vec![
            "assistant_text_delta".to_string(),
            "tool_request".to_string(),
        ],
        "tool_calls finish must not synthesize turn_complete"
    );
}

#[tokio::test]
async fn golden_openai_sse_stop_finish_projects_text_and_turn_complete() {
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Done.\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    );
    let types = collect_adapter_types_from_openai_sse(frames).await;
    assert_eq!(
        types,
        vec![
            "assistant_text_delta".to_string(),
            "turn_complete".to_string(),
        ]
    );
}

#[test]
fn golden_semantic_lifecycle_projects_ordered_adapter_types() {
    let types = semantic_events_to_adapter_types(vec![
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta {
            text: "Reviewing proposals.".to_string(),
        }),
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: "call-golden-1".to_string(),
            tool_name: "memory_read".to_string(),
            title: Some("Read memory".to_string()),
            kind: Some("read".to_string()),
            arguments: serde_json::json!({"path": "pair/notes/demo.md"}),
            approval_request_id: None,
            approval_required: false,
            approval_reason: None,
            run_id: None,
        }),
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { turn: None }),
    ]);
    assert_eq!(
        types,
        vec![
            "assistant_text_delta".to_string(),
            "tool_request".to_string(),
            "turn_complete".to_string(),
        ]
    );
}

#[test]
fn golden_semantic_status_text_projects_to_adapter_sse() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(RuntimeSemanticEvent::StatusText {
        text: "Indexing memory.".to_string(),
    });
    let types = mapped
        .into_iter()
        .map(gateway_event_to_adapter_sse)
        .flat_map(|chunk| adapter_sse_event_types(&[chunk]))
        .collect::<Vec<_>>();
    assert_eq!(types, vec!["status_text".to_string()]);
}

#[test]
fn golden_semantic_conversation_resolved_projects_to_adapter_sse() {
    let types = semantic_events_to_adapter_types(vec![RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::ConversationResolved {
            conversation: RuntimeConversationRef {
                id: "den-conv-golden-1".to_string(),
            },
        },
    )]);
    assert_eq!(types, vec!["conversation_resolved".to_string()]);
}

#[test]
fn golden_semantic_run_paused_projects_to_status_text_adapter_sse() {
    let types = semantic_events_to_adapter_types(vec![RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::RunPaused {
            reason: "awaiting_approval".to_string(),
            resume_token: None,
            expires_at: None,
        },
    )]);
    assert_eq!(types, vec!["status_text".to_string()]);
}

#[test]
fn golden_semantic_error_projects_to_adapter_sse() {
    let types = semantic_events_to_adapter_types(vec![RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::Error {
            message: "provider unavailable".to_string(),
            detail: Some("upstream reset".to_string()),
            error_type: Some("runtime_unavailable".to_string()),
            request_id: Some("req-golden-1".to_string()),
            context: None,
        },
    )]);
    assert_eq!(types, vec!["error".to_string()]);
}

#[test]
fn golden_semantic_turn_failed_projects_to_adapter_sse() {
    let types = semantic_events_to_adapter_types(vec![RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: RuntimeErrorCategory::Timeout,
            message: "runtime timed out".to_string(),
        },
    )]);
    assert_eq!(types, vec!["error".to_string()]);
}
