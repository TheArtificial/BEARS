use bytes::Bytes;
use futures::StreamExt;

use den_runtime::{
    runtime_stream_parser::runtime_byte_stream_to_event_stream,
    runtime_stream_parser::runtime_stream_event_from_letta_json,
    runtime_contracts::{RuntimeEventParser, RuntimeStreamEvent},
};

#[tokio::test]
async fn continuation_byte_stream_adapter_emits_semantic_event_and_terminal_completion() {
    let frame = b"data: {\"message_type\":\"assistant_message\",\"content\":\"hello\"}\n\n";
    let source = futures::stream::iter(vec![Ok::<Bytes, crate::errors::DenError>(
        Bytes::from_static(frame),
    )]);
    let mut stream = runtime_byte_stream_to_event_stream(
        Box::pin(source),
        RuntimeEventParser {
            parse_json_event: runtime_stream_event_from_letta_json,
        },
    );

    let first = stream.next().await.expect("first event").expect("ok event");
    assert!(matches!(
        first,
        RuntimeStreamEvent::Semantic(den_runtime::runtime_contracts::RuntimeSemanticEvent::AssistantTextDelta { text })
            if text == "hello"
    ));

    let second = stream.next().await.expect("terminal event").expect("ok event");
    assert!(matches!(
        second,
        RuntimeStreamEvent::Semantic(den_runtime::runtime_contracts::RuntimeSemanticEvent::TurnCompleted { .. })
    ));
}

#[tokio::test]
async fn requires_approval_pause_does_not_synthesize_turn_completion() {
    use den_runtime::runtime_contracts::RuntimeSemanticEvent;

    // A turn that pauses for approval (tool request + `requires_approval` stop) then closes
    // its byte stream must NOT get a synthetic `TurnCompleted`. The turn is awaiting a tool
    // result and a continuation; a spurious completion drives premature terminal emission and
    // preempts that continuation (so the tool result never gets a follow-up turn).
    let frames = concat!(
        "data: {\"id\":\"approval-call-1\",\"message_type\":\"approval_request_message\",",
        "\"tool_call\":{\"name\":\"fs_read_text_file\",\"tool_call_id\":\"call-1\",\"arguments\":\"{}\"}}\n\n",
        "data: {\"message_type\":\"stop_reason\",\"stop_reason\":\"requires_approval\"}\n\n",
    );
    let source = futures::stream::iter(vec![Ok::<Bytes, crate::errors::DenError>(
        Bytes::from_static(frames.as_bytes()),
    )]);
    let mut stream = runtime_byte_stream_to_event_stream(
        Box::pin(source),
        RuntimeEventParser {
            parse_json_event: runtime_stream_event_from_letta_json,
        },
    );

    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("ok event"));
    }

    assert!(
        matches!(
            events.first(),
            Some(RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested { .. }))
        ),
        "expected first event to be the tool request, got {events:?}"
    );
    assert!(
        matches!(
            events.last(),
            Some(RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunPaused { .. }))
        ),
        "stream must end on the pause, with no synthetic TurnCompleted: {events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. })
        )),
        "no TurnCompleted should be synthesized after a requires_approval pause: {events:?}"
    );
}
