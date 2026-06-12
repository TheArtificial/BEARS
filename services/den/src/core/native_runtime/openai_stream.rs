use std::collections::VecDeque;
use std::pin::Pin;
use std::task::Poll;

use futures::Stream;

use crate::{
    core::{
        letta_runtime_stream_parser::{
            find_sse_frame_end, strip_trailing_sse_delimiter_owned,
        },
        llm::{openai_sse_frame_to_runtime_events, OpenAiStreamAccumulator},
        runtime_contracts::{RuntimeByteStream, RuntimeEventStream, RuntimeSemanticEvent, RuntimeStreamEvent},
    },
    errors::CustomError,
};

pub fn openai_byte_stream_to_event_stream(
    parsed: impl Stream<Item = Result<bytes::Bytes, CustomError>> + Send + Unpin + 'static,
) -> RuntimeEventStream {
    let mut buffer = Vec::new();
    let mut queued_events: VecDeque<Result<RuntimeStreamEvent, CustomError>> = VecDeque::new();
    let mut finished = false;
    let mut saw_terminal_or_pause = false;
    let mut accumulator = OpenAiStreamAccumulator::default();
    let mut pinned = Box::pin(parsed);
    let stream = futures::stream::poll_fn(move |cx| loop {
        if let Some(item) = queued_events.pop_front() {
            return Poll::Ready(Some(item));
        }
        if finished {
            return Poll::Ready(None);
        }
        match Pin::new(&mut pinned).poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                buffer.extend_from_slice(&bytes);
                while let Some(end) = find_sse_frame_end(&buffer) {
                    let raw: Vec<u8> = buffer.drain(..end).collect();
                    let frame_body = strip_trailing_sse_delimiter_owned(raw);
                    match openai_sse_frame_to_runtime_events(&mut accumulator, &frame_body) {
                        Ok(events) => {
                            for event in events {
                                // Tool-call finishes are not turn terminals: the ACP layer must
                                // park for tool results and continue. A synthetic TurnCompleted
                                // here preempts that continuation (same class of bug as Letta
                                // requires_approval pauses in acp_turn_runner_stream_tests).
                                if matches!(
                                    &event,
                                    RuntimeStreamEvent::Semantic(
                                        RuntimeSemanticEvent::RunPaused { .. }
                                            | RuntimeSemanticEvent::ToolCallRequested { .. }
                                            | RuntimeSemanticEvent::TurnCompleted { .. }
                                            | RuntimeSemanticEvent::TurnFailed { .. }
                                            | RuntimeSemanticEvent::TurnCancelled { .. }
                                            | RuntimeSemanticEvent::Error { .. }
                                    )
                                ) {
                                    saw_terminal_or_pause = true;
                                }
                                queued_events.push_back(Ok(event));
                            }
                        }
                        Err(err) => queued_events.push_back(Err(err)),
                    }
                }
                if accumulator.should_detach_upstream() {
                    finished = true;
                }
            }
            Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
            Poll::Ready(None) => {
                finished = true;
                if buffer.is_empty() && !saw_terminal_or_pause {
                    for event in accumulator.flush_end_of_stream() {
                        if matches!(
                            &event,
                            RuntimeStreamEvent::Semantic(
                                RuntimeSemanticEvent::RunPaused { .. }
                                    | RuntimeSemanticEvent::ToolCallRequested { .. }
                                    | RuntimeSemanticEvent::TurnCompleted { .. }
                                    | RuntimeSemanticEvent::TurnFailed { .. }
                                    | RuntimeSemanticEvent::TurnCancelled { .. }
                                    | RuntimeSemanticEvent::Error { .. }
                            )
                        ) {
                            saw_terminal_or_pause = true;
                        }
                        queued_events.push_back(Ok(event));
                    }
                    if !saw_terminal_or_pause {
                        tracing::warn!(
                            "OpenAI SSE stream ended with no frames; synthesizing TurnCompleted"
                        );
                        queued_events.push_back(Ok(RuntimeStreamEvent::Semantic(
                            RuntimeSemanticEvent::TurnCompleted { turn: None },
                        )));
                    }
                } else if !buffer.is_empty() {
                    queued_events.push_back(Err(CustomError::System(format!(
                        "OpenAI SSE stream ended with incomplete frame ({} bytes)",
                        buffer.len()
                    ))));
                }
            }
            Poll::Pending => return Poll::Pending,
        }
    });
    Box::pin(stream)
}

pub fn openai_byte_stream_to_event_stream_from_box(
    parsed: RuntimeByteStream,
) -> RuntimeEventStream {
    openai_byte_stream_to_event_stream(parsed)
}
