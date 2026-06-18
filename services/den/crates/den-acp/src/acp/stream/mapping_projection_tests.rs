use crate::acp::stream::mapping::runtime_stream_event_to_acp_seed_value;
use den_runtime::runtime_provider::{
    RuntimeConversationRef, RuntimeSemanticEvent, RuntimeStreamEvent,
};

#[test]
fn semantic_conversation_resolved_projects_to_seed_value() {
    let value = runtime_stream_event_to_acp_seed_value(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::ConversationResolved {
            conversation: RuntimeConversationRef {
                id: "conv-test".to_string(),
            },
        },
    ))
    .expect("projection should succeed");

    assert_eq!(value.get("type").and_then(|v| v.as_str()), Some("conversation_resolved"));
    assert_eq!(
        value.get("conversation_id").and_then(|v| v.as_str()),
        Some("conv-test")
    );
}

#[test]
fn semantic_turn_completed_projects_to_stop_reason_seed_value() {
    let value = runtime_stream_event_to_acp_seed_value(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::TurnCompleted { turn: None },
    ))
    .expect("projection should succeed");

    assert_eq!(value.get("message_type").and_then(|v| v.as_str()), Some("stop_reason"));
    assert_eq!(value.get("stop_reason").and_then(|v| v.as_str()), Some("end_turn"));
}

#[test]
fn semantic_tool_call_requested_projects_to_tool_call_seed_value() {
    let value = runtime_stream_event_to_acp_seed_value(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            title: Some("Read text file".to_string()),
            kind: Some("read".to_string()),
            arguments: serde_json::json!({"path":"/workspace/README.md"}),
            approval_request_id: None,
            approval_required: false,
            approval_reason: None,
            run_id: Some("run-1".to_string()),
        },
    ))
    .expect("projection should succeed");

    assert_eq!(value.get("message_type").and_then(|v| v.as_str()), Some("tool_call_message"));
    assert_eq!(value.get("tool_call_id").and_then(|v| v.as_str()), Some("call-1"));
    assert_eq!(value.get("tool_name").and_then(|v| v.as_str()), Some("fs_read_text_file"));
    assert_eq!(value.get("run_id").and_then(|v| v.as_str()), Some("run-1"));
}

#[test]
fn semantic_run_paused_projects_to_requires_approval_stop_reason() {
    let value = runtime_stream_event_to_acp_seed_value(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::RunPaused {
            reason: "awaiting_approval".to_string(),
            resume_token: None,
            expires_at: None,
        },
    ))
    .expect("projection should succeed");

    assert_eq!(value.get("message_type").and_then(|v| v.as_str()), Some("stop_reason"));
    assert_eq!(
        value.get("stop_reason").and_then(|v| v.as_str()),
        Some("requires_approval")
    );
}

#[test]
fn semantic_assistant_text_projects_to_assistant_message_seed_value() {
    let value = runtime_stream_event_to_acp_seed_value(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::AssistantTextDelta {
            text: "hello".to_string(),
        },
    ))
    .expect("projection should succeed");

    assert_eq!(value.get("message_type").and_then(|v| v.as_str()), Some("assistant_message"));
    assert_eq!(value.get("content").and_then(|v| v.as_str()), Some("hello"));
}

#[test]
fn semantic_status_text_projects_to_reasoning_message_seed_value() {
    let value = runtime_stream_event_to_acp_seed_value(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::StatusText {
            text: "thinking".to_string(),
        },
    ))
    .expect("projection should succeed");

    assert_eq!(value.get("message_type").and_then(|v| v.as_str()), Some("reasoning_message"));
    assert_eq!(value.get("reasoning").and_then(|v| v.as_str()), Some("thinking"));
}

#[test]
fn semantic_error_projects_to_error_seed_value() {
    let value = runtime_stream_event_to_acp_seed_value(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::Error {
            message: "runtime error".to_string(),
            detail: Some("detail".to_string()),
            error_type: Some("runtime_error".to_string()),
            request_id: Some("req-1".to_string()),
            context: Some(serde_json::json!({"component":"den.acp"})),
        },
    ))
    .expect("projection should succeed");

    assert_eq!(value.get("message_type").and_then(|v| v.as_str()), Some("error_message"));
    assert_eq!(value.get("message").and_then(|v| v.as_str()), Some("runtime error"));
    assert_eq!(value.get("detail").and_then(|v| v.as_str()), Some("detail"));
    assert_eq!(value.get("error_type").and_then(|v| v.as_str()), Some("runtime_error"));
}
