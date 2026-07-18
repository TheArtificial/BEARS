use super::super::chat_proxy_stream::EPHEMERAL_PROGRESS_STATUSES;
use super::*;

#[test]
fn provider_activity_is_not_projected_to_web_chat() {
    assert!(
        runtime_stream_event_to_bear_channel_bytes(RuntimeStreamEvent::ProviderActivity, None,)
            .is_empty()
    );
}

#[test]
fn maps_assistant_text_to_assistant_delta() {
    let events = runtime_semantic_to_bear_channel_events(
        RuntimeSemanticEvent::AssistantTextDelta {
            text: "Hello".to_string(),
        },
        None,
    );
    assert_eq!(events[0]["type"], "assistant_delta");
    assert_eq!(events[0]["text"], "Hello");
}

#[test]
fn maps_ephemeral_run_progress_to_status_progress() {
    for status in EPHEMERAL_PROGRESS_STATUSES {
        let events = runtime_semantic_to_bear_channel_events(
            RuntimeSemanticEvent::RunProgress {
                kind: "status".to_string(),
                text: Some((*status).to_string()),
                phase: None,
                detail: None,
            },
            None,
        );
        assert_eq!(events[0]["type"], "status_progress", "status={status}");
        assert_eq!(events[0]["text"], *status);
    }
}

#[test]
fn maps_non_ephemeral_run_progress_to_reasoning_delta() {
    let events = runtime_semantic_to_bear_channel_events(
        RuntimeSemanticEvent::RunProgress {
            kind: "status".to_string(),
            text: Some("Indexing documents".to_string()),
            phase: None,
            detail: None,
        },
        None,
    );
    assert_eq!(events[0]["type"], "reasoning_delta");
}

#[test]
fn maps_status_text_to_reasoning_delta() {
    let events = runtime_semantic_to_bear_channel_events(
        RuntimeSemanticEvent::StatusText {
            text: "Indexing".to_string(),
        },
        None,
    );
    assert_eq!(events[0]["type"], "reasoning_delta");
}

#[test]
fn maps_turn_completed_to_done() {
    let events = runtime_semantic_to_bear_channel_events(
        RuntimeSemanticEvent::TurnCompleted { turn: None },
        None,
    );
    assert_eq!(events[0]["type"], "done");
}

#[test]
fn maps_tool_call_finished_to_status_and_single_error() {
    let events = runtime_semantic_to_bear_channel_events(
        RuntimeSemanticEvent::ToolCallFinished {
            tool_call_id: "call_1".to_string(),
            tool_name: "memory_read".to_string(),
            status: ToolCallFinishStatus::Error,
            summary: Some("memory_read failed: not found".to_string()),
            error_message: Some("memory_read failed: not found".to_string()),
        },
        Some("req-1"),
    );
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["type"], "server_tool_finished");
    assert_eq!(events[0]["tool"], "memory_read");
    assert_eq!(events[1]["type"], "error");
    assert_eq!(events[1]["error_type"], "tool_execution_error");
}

#[test]
fn maps_tool_call_requested_to_server_tool_started() {
    let events = runtime_semantic_to_bear_channel_events(
        RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id: "call_1".to_string(),
            tool_name: "den_capabilities_list_self".to_string(),
            title: Some("den_capabilities_list_self".to_string()),
            kind: Some("function".to_string()),
            arguments: serde_json::json!({}),
            approval_request_id: None,
            approval_required: false,
            approval_reason: None,
            run_id: None,
        },
        None,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "server_tool_started");
    assert_eq!(events[0]["tool"], "den_capabilities_list_self");
}

#[test]
fn turn_failed_includes_request_id() {
    let events = runtime_semantic_to_bear_channel_events(
        RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: RuntimeErrorCategory::Timeout,
            message: "timed out".to_string(),
        },
        Some("req-123"),
    );
    assert_eq!(events[0]["request_id"], "req-123");
    assert_eq!(events[0]["error_type"], "runtime_timeout");
}
