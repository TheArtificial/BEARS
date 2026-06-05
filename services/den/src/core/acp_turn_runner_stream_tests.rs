use bytes::Bytes;
use futures::StreamExt;

use crate::core::{
    acp_turn_runner::runtime_byte_stream_to_event_stream,
    letta_runtime_stream_parser::runtime_stream_event_from_letta_json,
    runtime_contracts::{RuntimeEventParser, RuntimeStreamEvent},
};

#[tokio::test]
async fn continuation_byte_stream_adapter_emits_semantic_event_and_terminal_completion() {
    let frame = b"data: {\"message_type\":\"assistant_message\",\"content\":\"hello\"}\n\n";
    let source = futures::stream::iter(vec![Ok::<Bytes, crate::errors::CustomError>(
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
        RuntimeStreamEvent::Semantic(crate::core::runtime_contracts::RuntimeSemanticEvent::AssistantTextDelta { text })
            if text == "hello"
    ));

    let second = stream.next().await.expect("terminal event").expect("ok event");
    assert!(matches!(
        second,
        RuntimeStreamEvent::Semantic(crate::core::runtime_contracts::RuntimeSemanticEvent::TurnCompleted { .. })
    ));
}
