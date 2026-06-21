use super::stream::{
    openai_sse_chunk_to_runtime_events, responses_sse_frame_to_runtime_events,
    OpenAiStreamAccumulator, ResponsesStreamAccumulator,
};
use crate::runtime_contracts::{RuntimeSemanticEvent, RuntimeStreamEvent};

#[test]
fn parses_text_delta_from_delta_text_field() {
    let chunk = br#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"text":"hello"},"finish_reason":null}]}

"#;
    let events = openai_sse_chunk_to_runtime_events(chunk).expect("parse");
    assert!(matches!(
        events.first(),
        Some(RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { text }))
            if text == "hello"
    ));
}

#[test]
fn parses_text_delta_from_recorded_sse() {
    let chunk = br#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}]}

"#;
    let events = openai_sse_chunk_to_runtime_events(chunk).expect("parse");
    assert!(matches!(
        events.first(),
        Some(RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { text }))
            if text == "hello"
    ));
}

#[test]
fn parses_tool_call_finish_from_recorded_sse() {
    let mut acc = OpenAiStreamAccumulator::default();
    let first = serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_abc",
                    "function": {"name": "memory_read", "arguments": "{\"path\":"}
                }]
            }
        }]
    });
    let second = serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "function": {"arguments": "\"pair/note.md\"}"}
                }]
            }
        }]
    });
    let finish = serde_json::json!({
        "choices": [{
            "finish_reason": "tool_calls"
        }]
    });
    acc.ingest_sse_data_line(&first);
    acc.ingest_sse_data_line(&second);
    let parsed = acc.ingest_sse_data_line(&finish);
    assert_eq!(parsed.events.len(), 1);
    match &parsed.events[0] {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            arguments,
            ..
        }) => {
            assert_eq!(tool_call_id, "call_abc");
            assert_eq!(tool_name, "memory_read");
            assert_eq!(arguments["path"], "pair/note.md");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn stop_finish_emits_turn_completed() {
    let chunk = br#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}

"#;
    let events = openai_sse_chunk_to_runtime_events(chunk).expect("parse");
    assert!(events.iter().any(|event| matches!(
        event,
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. })
    )));
}

#[test]
fn responses_text_delta_emits_assistant_text_delta() {
    let mut acc = ResponsesStreamAccumulator::default();
    let frame = br#"data: {"type":"response.output_text.delta","delta":"hello"}

"#;
    let events = responses_sse_frame_to_runtime_events(&mut acc, frame).expect("parse");
    assert!(matches!(
        events.first(),
        Some(RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { text }))
            if text == "hello"
    ));
}

#[test]
fn responses_completed_emits_turn_completed() {
    let mut acc = ResponsesStreamAccumulator::default();
    let frame = br#"data: {"type":"response.completed","response":{"id":"resp_1"}}

"#;
    let events = responses_sse_frame_to_runtime_events(&mut acc, frame).expect("parse");
    assert!(events.iter().any(|event| matches!(
        event,
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. })
    )));
}

#[test]
fn responses_function_call_done_emits_tool_call_requested() {
    let mut acc = ResponsesStreamAccumulator::default();
    let frame = br#"data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"session_info","arguments":"{}"}}

"#;
    let events = responses_sse_frame_to_runtime_events(&mut acc, frame).expect("parse");
    assert_eq!(events.len(), 1);
    match &events[0] {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            arguments,
            ..
        }) => {
            assert_eq!(tool_call_id, "call_1");
            assert_eq!(tool_name, "session_info");
            assert_eq!(arguments, &serde_json::json!({}));
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
