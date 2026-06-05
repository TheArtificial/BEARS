use crate::core::{
    acp_letta_events::AcpGatewayEvent,
    runtime_bearwire_projection::{
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
fn semantic_tool_call_requested_projects_to_tool_request_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            title: Some("Read text file".to_string()),
            kind: Some("read".to_string()),
            arguments: serde_json::json!({"path":"/workspace/README.md"}),
            approval_request_id: Some("approval-1".to_string()),
            approval_required: true,
            approval_reason: Some("workspace read".to_string()),
        },
    );

    assert!(matches!(
        mapped.as_slice(),
        [AcpGatewayEvent::ToolRequest { tool_call_id, tool_name, approval_required, .. }]
            if tool_call_id == "call-1" && tool_name == "fs_read_text_file" && *approval_required
    ));
}

#[test]
fn semantic_turn_failed_projects_to_error_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: RuntimeErrorCategory::Timeout,
            message: "timed out".to_string(),
        },
    );

    assert!(matches!(
        mapped.as_slice(),
        [AcpGatewayEvent::Error { message, error_type, .. }]
            if message == "timed out" && error_type.as_deref() == Some("runtime_timeout")
    ));
}

#[test]
fn semantic_turn_cancelled_projects_to_error_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::TurnCancelled { turn: None },
    );

    assert!(matches!(
        mapped.as_slice(),
        [AcpGatewayEvent::Error { error_type, .. }]
            if error_type.as_deref() == Some("runtime_turn_cancelled")
    ));
}

#[test]
fn untranslated_provider_event_does_not_project_to_bearwire_sse() {
    let mapped = runtime_stream_event_to_bearwire_sse(
        RuntimeStreamEvent::UntranslatedProviderEvent {
            value: serde_json::json!({"message_type":"tool_return_message"}),
        },
    );
    assert!(mapped.is_empty());
}

#[test]
fn semantic_conversation_resolved_projects_to_bearwire_sse() {
    let bytes = runtime_stream_event_to_bearwire_sse(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::ConversationResolved {
            conversation: RuntimeConversationRef {
                id: "conv-test".to_string(),
            },
        },
    ));
    assert_eq!(bytes.len(), 1);
    let text = String::from_utf8(bytes[0].to_vec()).expect("utf8 sse");
    assert!(text.contains("conversation_resolved"));
    assert!(text.contains("conv-test"));
}
