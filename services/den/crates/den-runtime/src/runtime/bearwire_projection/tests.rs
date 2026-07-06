use crate::{
    gateway_events::GatewayEvent,
    runtime::bearwire_projection::{
        project_runtime_event_lossy, runtime_semantic_event_to_bearwire_gateway_events,
        runtime_stream_event_to_bearwire_sse, wire::runtime_stream_event_to_bearwire_events,
        RuntimeEventProjectionOutcome,
    },
};
use den_protocol::{
    RuntimeConversationRef, RuntimeErrorCategory, RuntimeSemanticEvent, RuntimeStreamEvent,
    ToolCallFinishStatus,
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
        [GatewayEvent::AssistantTextDelta { text }] if text == "hello"
    ));
}

#[test]
fn semantic_reasoning_text_projects_to_reasoning_gateway_event() {
    let mapped = runtime_semantic_event_to_bearwire_gateway_events(
        RuntimeSemanticEvent::ReasoningTextDelta {
            text: "thinking".to_string(),
        },
    );

    assert!(matches!(
        mapped.as_slice(),
        [GatewayEvent::ReasoningTextDelta { text }] if text == "thinking"
    ));
}

#[test]
fn semantic_turn_completed_projects_to_bearwire_gateway_event() {
    let mapped =
        runtime_semantic_event_to_bearwire_gateway_events(RuntimeSemanticEvent::TurnCompleted {
            turn: None,
        });

    assert!(matches!(
        mapped.as_slice(),
        [GatewayEvent::TurnComplete { outcome }] if outcome == "ok"
    ));
}

#[test]
fn semantic_reasoning_text_projects_to_dedicated_bearwire_event() {
    let mapped = runtime_stream_event_to_bearwire_sse(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::ReasoningTextDelta {
            text: "thinking".to_string(),
        },
    ));

    let text = std::str::from_utf8(mapped[0].as_ref()).expect("valid utf8 sse");
    assert!(text.contains("\"type\":\"reasoning_text_delta\""), "{text}");
    assert!(text.contains("\"text\":\"thinking\""), "{text}");
}

#[test]
fn semantic_run_paused_projects_to_status_text_gateway_event() {
    let mapped =
        runtime_semantic_event_to_bearwire_gateway_events(RuntimeSemanticEvent::RunPaused {
            reason: "awaiting_approval".to_string(),
            resume_token: None,
            expires_at: None,
        });

    assert!(matches!(
        mapped.as_slice(),
        [GatewayEvent::StatusText { text }] if text == "Waiting for approval."
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
        [GatewayEvent::ToolRequest {
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
fn tool_call_requested_bearwire_event_has_canonical_tool_call_payload() {
    let mapped = runtime_stream_event_to_bearwire_events(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            title: Some("Read file".to_string()),
            kind: Some("read".to_string()),
            arguments: serde_json::json!({"path":"/tmp/demo"}),
            approval_request_id: None,
            approval_required: false,
            approval_reason: None,
            run_id: None,
        },
    ));

    let event = mapped
        .iter()
        .find(|event| event.event_type == "tool_call.requested")
        .expect("tool_call.requested event");
    assert_eq!(event.data["tool_call_id"], "call-1");
    assert_eq!(event.data["tool_name"], "fs_read_text_file");
    assert_eq!(event.data["arguments"]["path"], "/tmp/demo");
    assert_eq!(event.data["tool_call"]["id"], "call-1");
    assert_eq!(event.data["tool_call"]["name"], "fs_read_text_file");
    assert_eq!(event.data["tool_call"]["arguments"]["path"], "/tmp/demo");
    assert!(event.data["tool_call"]["display"].is_object());
}

#[test]
fn tool_call_finished_bearwire_event_has_stable_finish_envelope() {
    let mapped = runtime_stream_event_to_bearwire_events(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::ToolCallFinished {
            tool_call_id: "call-1".to_string(),
            tool_name: "fs_read_text_file".to_string(),
            status: ToolCallFinishStatus::Ok,
            summary: Some("Read 12 lines".to_string()),
            error_message: None,
        },
    ));

    let event = mapped
        .iter()
        .find(|event| event.event_type == "tool_call.completed")
        .expect("tool_call.completed event");
    assert_eq!(event.data["tool_call_id"], "call-1");
    assert_eq!(event.data["tool_name"], "fs_read_text_file");
    assert_eq!(event.data["tool_call"]["id"], "call-1");
    assert_eq!(event.data["tool_call"]["name"], "fs_read_text_file");
    assert_eq!(event.data["status"], "ok");
    assert_eq!(event.data["summary"], "Read 12 lines");
    assert!(event.data.get("compacted").is_some());
}

#[test]
fn semantic_turn_failed_projects_to_error_gateway_event() {
    let mapped =
        runtime_semantic_event_to_bearwire_gateway_events(RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: RuntimeErrorCategory::Timeout,
            message: "runtime timed out".to_string(),
        });

    assert!(matches!(
        mapped.as_slice(),
        [GatewayEvent::Error {
            message,
            error_type,
            ..
        }] if message == "runtime timed out"
            && error_type.as_deref() == Some("runtime_timeout")
    ));
}

#[test]
fn semantic_turn_cancelled_projects_to_cancelled_error_gateway_event() {
    let mapped =
        runtime_semantic_event_to_bearwire_gateway_events(RuntimeSemanticEvent::TurnCancelled {
            turn: None,
        });

    assert!(matches!(
        mapped.as_slice(),
        [GatewayEvent::Error {
            error_type,
            ..
        }] if error_type.as_deref() == Some("runtime_turn_cancelled")
    ));
}

#[test]
fn untranslated_provider_event_does_not_project_to_bearwire_sse() {
    let mapped =
        runtime_stream_event_to_bearwire_sse(RuntimeStreamEvent::UntranslatedProviderEvent {
            value: serde_json::json!({"message_type":"provider_only"}),
        });

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

#[test]
fn lossy_projection_handles_progress_events_without_failure() {
    let outcome = project_runtime_event_lossy(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::RunProgress {
            kind: "plan_update".to_string(),
            text: None,
            phase: Some("tool_result".to_string()),
            detail: Some(serde_json::json!({ "entries": [] })),
        },
    ));

    assert!(matches!(outcome, RuntimeEventProjectionOutcome::Events(events) if !events.is_empty()));
}

#[test]
fn session_info_update_progress_projects_to_session_info_gateway_event() {
    let mapped =
        runtime_semantic_event_to_bearwire_gateway_events(RuntimeSemanticEvent::RunProgress {
            kind: "session_info_update".to_string(),
            text: None,
            phase: Some("tool_result".to_string()),
            detail: Some(serde_json::json!({
                "title": "New title"
            })),
        });

    assert!(matches!(
        mapped.as_slice(),
        [GatewayEvent::SessionInfoUpdate { title, updated_at, .. }]
            if title.as_deref() == Some("New title") && updated_at.is_none()
    ));
}

#[test]
fn budget_warning_progress_projects_to_status_text_gateway_event() {
    let mapped =
        runtime_semantic_event_to_bearwire_gateway_events(RuntimeSemanticEvent::RunProgress {
            kind: "turn_budget_warning".to_string(),
            text: Some("Budget advisory: next read will stop the turn.".to_string()),
            phase: Some("budget".to_string()),
            detail: Some(serde_json::json!({
                "code": "tool_class_budget_warning"
            })),
        });

    assert!(matches!(
        mapped.as_slice(),
        [GatewayEvent::StatusText { text }]
            if text == "Budget advisory: next read will stop the turn."
    ));
}

#[test]
fn session_info_update_progress_projects_to_dedicated_bearwire_event() {
    let mapped = runtime_stream_event_to_bearwire_sse(RuntimeStreamEvent::Semantic(
        RuntimeSemanticEvent::RunProgress {
            kind: "session_info_update".to_string(),
            text: None,
            phase: Some("tool_result".to_string()),
            detail: Some(serde_json::json!({
                "title": "New title",
                "updated_at": "2026-07-05T12:00:00Z"
            })),
        },
    ));

    let text = std::str::from_utf8(mapped[0].as_ref()).expect("valid utf8 sse");
    assert!(text.contains("\"type\":\"session_info_update\""), "{text}");
    assert!(text.contains("\"title\":\"New title\""), "{text}");
    assert!(
        text.contains("\"updated_at\":\"2026-07-05T12:00:00Z\""),
        "{text}"
    );
}

#[test]
fn lossy_projection_covers_core_semantic_variants() {
    let events = vec![
        RuntimeSemanticEvent::AssistantTextDelta {
            text: "hi".to_string(),
        },
        RuntimeSemanticEvent::StatusText {
            text: "working".to_string(),
        },
        RuntimeSemanticEvent::ConversationResolved {
            conversation: RuntimeConversationRef {
                id: "conv".to_string(),
            },
        },
        RuntimeSemanticEvent::TurnCompleted { turn: None },
        RuntimeSemanticEvent::RunPaused {
            reason: "awaiting_approval".to_string(),
            resume_token: None,
            expires_at: None,
        },
        RuntimeSemanticEvent::Error {
            message: "err".to_string(),
            detail: None,
            error_type: None,
            request_id: None,
            context: None,
        },
        RuntimeSemanticEvent::TurnFailed {
            category: RuntimeErrorCategory::Internal,
            message: "failed".to_string(),
            turn: None,
        },
        RuntimeSemanticEvent::TurnCancelled { turn: None },
        RuntimeSemanticEvent::RunProgress {
            kind: "status_text".to_string(),
            text: Some("status".to_string()),
            phase: None,
            detail: None,
        },
        RuntimeSemanticEvent::ToolCallFinished {
            tool_call_id: "call".to_string(),
            tool_name: "tool".to_string(),
            status: ToolCallFinishStatus::Ok,
            summary: Some("done".to_string()),
            error_message: None,
        },
    ];

    for event in events {
        let outcome = project_runtime_event_lossy(RuntimeStreamEvent::Semantic(event));
        assert!(matches!(outcome, RuntimeEventProjectionOutcome::Events(_)));
    }
}
