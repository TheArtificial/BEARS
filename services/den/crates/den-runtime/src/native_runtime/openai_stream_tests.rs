use den_core::DenError;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;

use crate::native_runtime::openai_byte_stream_to_event_stream;
use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};

#[tokio::test]
async fn tool_call_finish_does_not_synthesize_turn_completed() {
    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"memory_read\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    );
    let source = futures::stream::iter(vec![Ok::<Bytes, den_core::DenError>(Bytes::from_static(
        frames.as_bytes(),
    ))]);
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

#[tokio::test]
async fn stop_finish_detaches_without_waiting_for_upstream_close() {
    static UPSTREAM_POLLED_AFTER_STOP: AtomicBool = AtomicBool::new(false);
    struct HangAfterFirst {
        chunk: Bytes,
        emitted: bool,
    }
    impl Stream for HangAfterFirst {
        type Item = Result<Bytes, DenError>;
        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if self.emitted {
                UPSTREAM_POLLED_AFTER_STOP.store(true, Ordering::SeqCst);
                Poll::Pending
            } else {
                self.emitted = true;
                Poll::Ready(Some(Ok(self.chunk.clone())))
            }
        }
    }

    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    );
    let source = HangAfterFirst {
        chunk: Bytes::from_static(frames.as_bytes()),
        emitted: false,
    };
    let mut stream = openai_byte_stream_to_event_stream(source);

    let mut events = Vec::new();
    while let Some(item) = stream.next().await {
        events.push(item.expect("ok event"));
    }

    assert!(
        events.iter().any(|event| matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { .. })
        )),
        "expected assistant text: {events:?}"
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. })
        )),
        "expected TurnCompleted after stop finish: {events:?}"
    );
    assert!(
        !UPSTREAM_POLLED_AFTER_STOP.load(Ordering::SeqCst),
        "must not block on upstream close after stop finish"
    );
}

#[tokio::test]
async fn tool_calls_finish_detaches_without_waiting_for_upstream_close() {
    static UPSTREAM_POLLED_AFTER_TOOL_FINISH: AtomicBool = AtomicBool::new(false);
    struct HangAfterFirst {
        chunk: Bytes,
        emitted: bool,
    }
    impl Stream for HangAfterFirst {
        type Item = Result<Bytes, DenError>;
        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            if self.emitted {
                UPSTREAM_POLLED_AFTER_TOOL_FINISH.store(true, Ordering::SeqCst);
                Poll::Pending
            } else {
                self.emitted = true;
                Poll::Ready(Some(Ok(self.chunk.clone())))
            }
        }
    }

    let frames = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"memory_read\",\"arguments\":\"{}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    );
    let source = HangAfterFirst {
        chunk: Bytes::from_static(frames.as_bytes()),
        emitted: false,
    };
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
        "expected tool call event: {events:?}"
    );
    assert!(
        !events.iter().any(|event| matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. })
        )),
        "must not synthesize TurnCompleted after tool_calls finish: {events:?}"
    );
    assert!(
        !UPSTREAM_POLLED_AFTER_TOOL_FINISH.load(Ordering::SeqCst),
        "must not block on upstream close after tool_calls finish"
    );
}
