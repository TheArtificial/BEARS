use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::Stream;
use tokio::time::timeout;

use crate::{
    core::{
        agent_loop::{context::repair_tool_call_message_chain, AgentLoopSession},
        llm::{byte_stream_with_idle_timeout, ChatCompletionRequest, LlmClient},
        native_runtime::openai_byte_stream_to_event_stream,
        runtime_contracts::{RuntimeEventStream, RuntimeStreamEvent},
    },
    errors::CustomError,
};

/// Max wait for Bifrost to accept `POST /chat/completions` and return response headers.
const NATIVE_LLM_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Max silence between upstream SSE byte chunks after the handshake.
const NATIVE_LLM_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

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
    fn new(llm: LlmClient, request: ChatCompletionRequest, session_key: String) -> Self {
        let model = request.model.clone();
        let message_count = request.messages.len();
        let tool_count = request.tools.len();
        let fut = Box::pin(async move {
            let started = Instant::now();
            tracing::info!(
                session_key = %session_key,
                model = %model,
                message_count,
                tool_count,
                handshake_timeout_secs = NATIVE_LLM_HANDSHAKE_TIMEOUT.as_secs(),
                "LLM chat/completions handshake starting"
            );
            let handshake = timeout(
                NATIVE_LLM_HANDSHAKE_TIMEOUT,
                llm.chat_completions_byte_stream(&request),
            )
            .await;
            match handshake {
                Err(_) => {
                    tracing::warn!(
                        session_key = %session_key,
                        model = %model,
                        duration_ms = started.elapsed().as_millis(),
                        handshake_timeout_secs = NATIVE_LLM_HANDSHAKE_TIMEOUT.as_secs(),
                        "LLM chat/completions handshake timed out"
                    );
                    Err(CustomError::System(format!(
                        "LLM chat/completions handshake timed out after {}s",
                        NATIVE_LLM_HANDSHAKE_TIMEOUT.as_secs()
                    )))
                }
                Ok(Err(err)) => {
                    tracing::warn!(
                        session_key = %session_key,
                        model = %model,
                        duration_ms = started.elapsed().as_millis(),
                        error = %err,
                        "LLM chat/completions handshake failed"
                    );
                    Err(err)
                }
                Ok(Ok(byte_stream)) => {
                    tracing::info!(
                        session_key = %session_key,
                        model = %model,
                        duration_ms = started.elapsed().as_millis(),
                        idle_timeout_secs = NATIVE_LLM_STREAM_IDLE_TIMEOUT.as_secs(),
                        "LLM chat/completions handshake connected"
                    );
                    let byte_stream = byte_stream_with_idle_timeout(
                        byte_stream,
                        NATIVE_LLM_STREAM_IDLE_TIMEOUT,
                    );
                    Ok(openai_byte_stream_to_event_stream(byte_stream))
                }
            }
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
    tracing::info!(
        session_key = %session.session_key,
        model = %session.model,
        message_count = messages.len(),
        tool_count = session.tools.len(),
        step = session.step,
        "native agent step starting LLM stream"
    );
    let request = ChatCompletionRequest {
        model: session.model.clone(),
        messages,
        tools: session.tools.clone(),
        stream: true,
        tool_choice: None,
        temperature: None,
        max_tokens: None,
    };
    Ok(Box::pin(LazyAgentStepStream::new(
        llm.clone(),
        request,
        session.session_key.clone(),
    )))
}
