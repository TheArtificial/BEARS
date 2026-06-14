use crate::{
    acp_events::AcpGatewayEvent,
    runtime::bearwire_projection::{
        runtime_semantic_event_to_bearwire_gateway_events, runtime_stream_event_to_bearwire_sse,
    },
    runtime_contracts::{
        RuntimeConversationRef, RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent,
    },
};

#[test]
fn semantic_assistant_text_projects_to_bearwire_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::AssistantTextDelta {
            text: "hello".to_string(),
        },
    );

    assert!(matches!(
        mapped.as_slice(),
        [AcpGatewayEvent::AssistantTextDelta { text }] if text == "hello"
    ));
}

#[test]
fn semantic_turn_completed_projects_to_bearwire_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::TurnCompleted { turn: None },
    );

    assert!(matches!(
        mapped.as_slice(),
        [AcpGatewayEvent::TurnComplete { outcome }] if outcome == "ok"
    ));
}

#[test]
fn semantic_run_paused_projects_to_status_text_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::RunPaused {
            reason: "awaiting_approval".to_string(),
            resume_token: None,
            expires_at: None,
        },
    );

    assert!(matches!(
        mapped.as_slice(),
        [AcpGatewayEvent::StatusText { text }] if text == "Waiting for approval."
    ));
}

#[test]
fn semantic_tool_call_projects_to_tool_request_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            title: Some("Read file".to_string()),
            kind: Some("read".to_string()),
            arguments: serde_json::json!({"path":"/tmp/demo"}),
            approval_request_id: Some("approval-1".to_string()),
            approval_required: true,
            approval_reason: Some("Need file access".to_string()),
            run_id: None,
        },
    );

    assert!(matches!(
        mapped.as_slice(),
        [AcpGatewayEvent::ToolRequest {
            tool_call_id,
            tool_name,
            approval_required,
            approval_request_id,
            ..
        }] if tool_call_id == "call-1"
            && tool_name == "fs_read_text_file"
            && *approval_required
            && approval_request_id.as_deref() == Some("approval-1")
    ));
}

#[test]
fn semantic_turn_failed_projects_to_error_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: RuntimeErrorCategory::Timeout,
            message: "runtime timed out".to_string(),
        },
    );

    assert!(matches!(
        mapped.as_slice(),
        [AcpGatewayEvent::Error {
            message,
            error_type,
            ..
        }] if message == "runtime timed out"
            && error_type.as_deref() == Some("runtime_timeout")
    ));
}

#[test]
fn semantic_turn_cancelled_projects_to_cancelled_error_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::TurnCancelled { turn: None },
    );

    assert!(matches!(
        mapped.as_slice(),
        [AcpGatewayEvent::Error {
            error_type,
            ..
        }] if error_type.as_deref() == Some("runtime_turn_cancelled")
    ));
}

#[test]
fn untranslated_provider_event_does_not_project_to_bearwire_sse() {
    let mapped = runtime_stream_event_to_bearwire_sse(
        RuntimeStreamEvent::UntranslatedProviderEvent {
            value: serde_json::json!({"message_type":"provider_only"}),
        },
    );

    assert!(mapped.is_empty());
}

#[test]
fn conversation_resolved_projects_to_sse() {
    let mapped = runtime_stream_event_to_bearwire_sse(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::ConversationResolved {
            conversation: RuntimeConversationRef {
                id: "conv-123".to_string(),
            },
        },
    ));

    let text = std::str::from_utf8(mapped[0].as_ref()).expect("valid utf8 sse");
    assert!(text.contains("conversation_resolved"));
    assert!(text.contains("conv-123"));
}
