use crate::core::runtime_contracts::{RuntimeSemanticEvent, RuntimeStreamEvent};
use crate::core::letta_runtime_stream_parser::runtime_stream_event_from_letta_json;

#[test]
fn reasoning_message_maps_to_semantic_status_text() {
    let event = serde_json::json!({
        "message_type": "reasoning_message",
        "reasoning": "thinking"
    });
    let mapped = runtime_stream_event_from_letta_json(&event).expect("mapped event");
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
    let mapped = runtime_stream_event_from_letta_json(&event).expect("mapped event");
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
    let mapped = runtime_stream_event_from_letta_json(&event).expect("mapped event");
    match mapped {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunPaused { reason, .. }) => {
            assert_eq!(reason, "awaiting_approval");
        }
        other => panic!("unexpected mapping: {other:?}"),
    }
}
