use crate::runtime_contracts::{RuntimeSemanticEvent, RuntimeStreamEvent};
use crate::agent_assist::runtime_stream_parser::runtime_stream_event_from_provider_json;

#[test]
fn reasoning_message_maps_to_semantic_status_text() {
    let event = serde_json::json!({
        "message_type": "reasoning_message",
        "reasoning": "thinking"
    });
    let mapped = runtime_stream_event_from_provider_json(&event).expect("mapped event");
    match mapped {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::StatusText { text }) => {
            assert_eq!(text, "thinking");
        }
        other => panic!("unexpected mapping: {other:?}"),
    }
}

#[test]
fn assistant_message_maps_to_semantic_assistant_text() {
    let event = serde_json::json!({
        "message_type": "assistant_message",
        "content": "hello"
    });
    let mapped = runtime_stream_event_from_provider_json(&event).expect("mapped event");
    match mapped {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { text }) => {
            assert_eq!(text, "hello");
        }
        other => panic!("unexpected mapping: {other:?}"),
    }
}

#[test]
fn stop_reason_requires_approval_maps_to_semantic_pause() {
    let event = serde_json::json!({
        "message_type": "stop_reason",
        "stop_reason": "requires_approval"
    });
    let mapped = runtime_stream_event_from_provider_json(&event).expect("mapped event");
    match mapped {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunPaused { reason, .. }) => {
            assert_eq!(reason, "awaiting_approval");
        }
        other => panic!("unexpected mapping: {other:?}"),
    }
}

#[test]
fn approval_request_with_nested_tool_call_preserves_identity_and_args() {
    // provider nests identity and arguments under `tool_call`, with the approval id in the
    // top-level `id`. Earlier parsing only read top-level `tool_call_id`/`args`, dropping
    // them — which left adapter-local tool obligations unmatchable and `/tool-results`
    // posts rejected as `late_result_ignored`.
    let event = serde_json::json!({
        "id": "approval-call-read-e2e",
        "message_type": "approval_request_message",
        "tool_call": {
            "name": "fs_read_text_file",
            "tool_call_id": "call-read-e2e",
            "arguments": "{\"limit\":10,\"line\":1,\"path\":\"/tmp/acp-workspace/README.md\"}"
        }
    });
    let mapped = runtime_stream_event_from_provider_json(&event).expect("mapped event");
    match mapped {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            arguments,
            approval_request_id,
            approval_required,
            ..
        }) => {
            assert_eq!(tool_call_id, "call-read-e2e");
            assert_eq!(tool_name, "fs_read_text_file");
            assert_eq!(approval_request_id.as_deref(), Some("approval-call-read-e2e"));
            assert!(approval_required);
            // Arguments are carried as the raw JSON-string fragment provider emits; downstream
            // accumulation parses them. Assert the path is present rather than an empty object.
            let args_str = arguments.as_str().unwrap_or_default();
            assert!(
                args_str.contains("/tmp/acp-workspace/README.md"),
                "arguments fragment lost: {arguments:?}"
            );
        }
        other => panic!("unexpected mapping: {other:?}"),
    }
}

#[test]
fn tool_call_with_top_level_fields_still_parses() {
    // Compatibility: providers that put identity/args at the top level must still work.
    let event = serde_json::json!({
        "message_type": "tool_call_message",
        "tool_call_id": "call-top",
        "tool_name": "fs_list_directory",
        "args": { "path": "/tmp" }
    });
    let mapped = runtime_stream_event_from_provider_json(&event).expect("mapped event");
    match mapped {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            approval_required,
            ..
        }) => {
            assert_eq!(tool_call_id, "call-top");
            assert_eq!(tool_name, "fs_list_directory");
            // `tool_call_message` (not `approval_request_message`) is not approval-gated.
            assert!(!approval_required);
        }
        other => panic!("unexpected mapping: {other:?}"),
    }
}

#[test]
fn end_turn_stop_reason_maps_to_semantic_turn_completed() {
    let event = serde_json::json!({
        "message_type": "stop_reason",
        "stop_reason": "end_turn"
    });
    let mapped = runtime_stream_event_from_provider_json(&event).expect("mapped event");
    match mapped {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. }) => {}
        other => panic!("unexpected mapping: {other:?}"),
    }
}

#[test]
fn unknown_stop_reason_maps_to_semantic_turn_failed() {
    let event = serde_json::json!({
        "message_type": "stop_reason",
        "stop_reason": "max_steps_exceeded"
    });
    let mapped = runtime_stream_event_from_provider_json(&event).expect("mapped event");
    match mapped {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed { message, .. }) => {
            assert!(message.contains("max_steps_exceeded"));
        }
        other => panic!("unexpected mapping: {other:?}"),
    }
}

#[test]
fn conversation_resolved_event_maps_to_semantic_conversation_ref() {
    let event = serde_json::json!({
        "message_type": "conversation_resolved",
        "conversation_id": "conv-resolved-789"
    });
    let mapped = runtime_stream_event_from_provider_json(&event).expect("mapped event");
    match mapped {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ConversationResolved { conversation }) => {
            assert_eq!(conversation.id, "conv-resolved-789");
        }
        other => panic!("unexpected mapping: {other:?}"),
    }
}
