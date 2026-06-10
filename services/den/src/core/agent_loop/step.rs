use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;

use crate::{
    core::{
        agent_loop::{context::repair_tool_call_message_chain, AgentLoopSession},
        llm::{ChatCompletionRequest, LlmClient},
        native_runtime::openai_byte_stream_to_event_stream,
        runtime_contracts::{RuntimeEventStream, RuntimeStreamEvent},
    },
    errors::CustomError,
};

enum LazyAgentStepState {
    Init {
        fut: Pin<Box<dyn Future<Output = Result<RuntimeEventStream, CustomError>> + Send>>,
    },
    Streaming(RuntimeEventStream),
}

struct LazyAgentStepStream {
    state: Option<LazyAgentStepState>,
}

impl LazyAgentStepStream {
    fn new(llm: LlmClient, request: ChatCompletionRequest) -> Self {
        let fut = Box::pin(async move {
            let byte_stream = llm.chat_completions_byte_stream(&request).await?;
            Ok(openai_byte_stream_to_event_stream(byte_stream))
        });
        Self {
            state: Some(LazyAgentStepState::Init { fut }),
        }
    }
}

impl Stream for LazyAgentStepStream {
    type Item = Result<RuntimeStreamEvent, CustomError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let Some(state) = self.state.take() else {
                return Poll::Ready(None);
            };
            match state {
                LazyAgentStepState::Init { mut fut } => {
                    match fut.as_mut().poll(cx) {
                        Poll::Ready(Ok(stream)) => {
                            self.state = Some(LazyAgentStepState::Streaming(stream));
                        }
                        Poll::Ready(Err(err)) => {
                            self.state = None;
                            return Poll::Ready(Some(Err(err)));
                        }
                        Poll::Pending => {
                            self.state = Some(LazyAgentStepState::Init { fut });
                            return Poll::Pending;
                        }
                    }
                }
                LazyAgentStepState::Streaming(mut stream) => {
                    match Pin::new(&mut stream).poll_next(cx) {
                        Poll::Ready(Some(item)) => {
                            self.state = Some(LazyAgentStepState::Streaming(stream));
                            return Poll::Ready(Some(item));
                        }
                        Poll::Ready(None) => {
                            return Poll::Ready(None);
                        }
                        Poll::Pending => {
                            self.state = Some(LazyAgentStepState::Streaming(stream));
                            return Poll::Pending;
                        }
                    }
                }
            }
        }
    }
}

/// Starts an agent step stream without blocking on the upstream LLM HTTP handshake.
///
/// The prompt handler must return SSE headers before Bifrost accepts the chat/completions
/// request; deferring the LLM call until the stream is polled avoids wedging ACP clients
/// that wait on `POST /prompt` with no timeout.
pub async fn run_agent_step_stream(
    llm: &LlmClient,
    session: &AgentLoopSession,
) -> Result<RuntimeEventStream, CustomError> {
    let messages = repair_tool_call_message_chain(session.messages.clone());
    let request = ChatCompletionRequest {
        model: session.model.clone(),
        messages,
        tools: session.tools.clone(),
        stream: true,
        tool_choice: None,
        temperature: None,
        max_tokens: None,
    };
    Ok(Box::pin(LazyAgentStepStream::new(llm.clone(), request)))
}
