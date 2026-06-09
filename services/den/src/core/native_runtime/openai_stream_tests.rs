use bytes::Bytes;
use futures::StreamExt;

use crate::core::{
    native_runtime::openai_byte_stream_to_event_stream,
    runtime_contracts::{RuntimeSemanticEvent, RuntimeStreamEvent},
};

#[tokio::test]
async fn tool_call_finish_does_not_synthesize_turn_completed() {
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"memory_read\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    );
    let source = futures::stream::iter(vec![Ok::<Bytes, crate::errors::CustomError>(
        Bytes::from_static(frames.as_bytes()),
    )]);
    let mut stream = openai_byte_stream_to_event_stream(source);

    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("ok event"));
    }

    assert!(
        events.iter().any(|event| matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested { .. })
        )),
        "expected tool call event, got {events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. })
        )),
        "must not synthesize TurnCompleted after tool_calls finish: {events:?}"
    );
}
