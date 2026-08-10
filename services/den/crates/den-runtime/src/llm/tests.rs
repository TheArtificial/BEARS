use super::stream::{
    openai_sse_chunk_to_runtime_events, responses_sse_frame_to_runtime_events,
    OpenAiStreamAccumulator, ResponsesStreamAccumulator,
};
use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};

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
fn openai_usage_chunk_captures_prompt_tokens_without_choices() {
    let mut acc = OpenAiStreamAccumulator::default();
    let usage = serde_json::json!({
        "usage": {"prompt_tokens": 1_234},
        "choices": []
    });

    assert!(acc.ingest_sse_data_line(&usage).events.is_empty());
    assert_eq!(acc.take_observed_prompt_tokens(), Some(1_234));
    assert_eq!(acc.take_observed_prompt_tokens(), None);
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
fn responses_completed_captures_input_tokens() {
    let mut acc = ResponsesStreamAccumulator::default();
    let frame = br#"data: {"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":4321}}}

"#;

    let events = responses_sse_frame_to_runtime_events(&mut acc, frame).expect("parse");
    assert!(events.iter().any(|event| matches!(
        event,
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. })
    )));
    assert_eq!(acc.take_observed_prompt_tokens(), Some(4_321));
}

#[test]
fn responses_reasoning_delta_emits_reasoning_text_delta() {
    let mut acc = ResponsesStreamAccumulator::default();
    let frame = br#"data: {"type":"response.reasoning_text.delta","delta":"private thought"}

"#;
    let events = responses_sse_frame_to_runtime_events(&mut acc, frame).expect("parse");
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ReasoningTextDelta { text })
            if text == "private thought"
    ));
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

#[test]
fn responses_function_call_arguments_done_emits_tool_call_requested() {
    let mut acc = ResponsesStreamAccumulator::default();
    let added = br#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"memory_read","arguments":""}}

"#;
    let args_done = br#"data: {"type":"response.function_call_arguments.done","item_id":"fc_1","output_index":0,"arguments":"{\"path\":\"pair/note.md\"}"}

"#;
    assert!(responses_sse_frame_to_runtime_events(&mut acc, added)
        .expect("parse added")
        .is_empty());
    let events = responses_sse_frame_to_runtime_events(&mut acc, args_done).expect("parse done");
    assert_eq!(events.len(), 1);
    match &events[0] {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            arguments,
            ..
        }) => {
            assert_eq!(tool_call_id, "call_1");
            assert_eq!(tool_name, "memory_read");
            assert_eq!(arguments["path"], "pair/note.md");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn responses_completed_flushes_pending_function_call_without_output_item_done() {
    let mut acc = ResponsesStreamAccumulator::default();
    let added = br#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"memory_read","arguments":""}}

"#;
    let args_delta = br#"data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"{\"path\":"}

"#;
    let completed = br#"data: {"type":"response.completed","response":{"id":"resp_1"}}

"#;
    assert!(responses_sse_frame_to_runtime_events(&mut acc, added)
        .expect("parse added")
        .is_empty());
    assert!(responses_sse_frame_to_runtime_events(&mut acc, args_delta)
        .expect("parse delta")
        .is_empty());
    let events =
        responses_sse_frame_to_runtime_events(&mut acc, completed).expect("parse completed");
    assert_eq!(events.len(), 1);
    match &events[0] {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            tool_name,
            arguments,
            ..
        }) => {
            assert_eq!(tool_call_id, "call_1");
            assert_eq!(tool_name, "memory_read");
            assert_eq!(
                arguments,
                &serde_json::Value::String("{\"path\":".to_string())
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
