use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use den_core::{config::Config, profile::BearProfile, DenError};
use futures::{Stream, TryStreamExt};
use sqlx::PgPool;
use tokio::time::timeout;

use crate::{
    agent_loop::{
        context::repair_tool_call_message_chain,
        overflow_retry::compact_session_messages_for_overflow,
        session_store::{AgentLoopSession, AgentLoopSessionStore},
    },
    llm::{
        bifrost_key_selection_error, byte_stream_with_idle_timeout, preferred_api_style_for_model,
        ChatCompletionRequest, LlmApiStyle, LlmClient,
    },
    native_runtime::{
        openai_byte_stream_to_event_stream_with_telemetry, responses_byte_stream_to_event_stream,
    },
    runtime_compaction::{den_error_indicates_context_overflow, CompactionMode},
    runtime_contracts::{RuntimeEventStream, RuntimeStreamEvent},
};

/// Max wait for Bifrost to accept `POST /chat/completions` and return response headers.
const NATIVE_LLM_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Max silence between upstream SSE byte chunks after the handshake.
const NATIVE_LLM_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Dependencies for one-shot context-overflow recovery during an agent step.
#[derive(Clone)]
pub struct AgentStepOverflowContext {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub profile: BearProfile,
    pub session_store: AgentLoopSessionStore,
}

enum LazyAgentStepState {
    Init {
        fut: Pin<Box<dyn Future<Output = Result<RuntimeEventStream, DenError>> + Send>>,
    },
    Streaming(RuntimeEventStream),
}

struct LazyAgentStepStream {
    state: Option<LazyAgentStepState>,
}

impl LazyAgentStepStream {
    fn new(
        llm: LlmClient,
        request: ChatCompletionRequest,
        session_key: String,
        api_style_override: Option<LlmApiStyle>,
        overflow: Option<AgentStepOverflowContext>,
    ) -> Self {
        let model = request.model.clone();
        let message_count = request.messages.len();
        let tool_count = request.tools.len();
        let fut = Self::handshake_future(
            llm,
            request,
            session_key,
            model,
            message_count,
            tool_count,
            api_style_override,
            overflow,
        );
        Self {
            state: Some(LazyAgentStepState::Init { fut }),
        }
    }

    fn handshake_future(
        llm: LlmClient,
        request: ChatCompletionRequest,
        session_key: String,
        model: String,
        message_count: usize,
        tool_count: usize,
        api_style_override: Option<LlmApiStyle>,
        overflow: Option<AgentStepOverflowContext>,
    ) -> Pin<Box<dyn Future<Output = Result<RuntimeEventStream, DenError>> + Send>> {
        Box::pin(async move {
            let started = Instant::now();
            let api_style =
                api_style_override.unwrap_or_else(|| preferred_api_style_for_model(&model));
            tracing::info!(
                session_key = %session_key,
                model = %model,
                api_style = %api_style.as_str(),
                message_count,
                tool_count,
                handshake_timeout_secs = NATIVE_LLM_HANDSHAKE_TIMEOUT.as_secs(),
                "LLM stream handshake starting"
            );
            let handshake = timeout(NATIVE_LLM_HANDSHAKE_TIMEOUT, async {
                match api_style {
                    LlmApiStyle::ChatCompletionsStream => {
                        let byte_stream = llm.chat_completions_byte_stream(&request).await?;
                        Self::connect_byte_stream(
                            session_key.clone(),
                            model.clone(),
                            api_style,
                            started,
                            byte_stream,
                            request.telemetry.clone(),
                        )
                    }
                    LlmApiStyle::ResponsesStream => match llm.responses_byte_stream(&request).await {
                        Ok(byte_stream) => Self::connect_byte_stream(
                            session_key.clone(),
                            model.clone(),
                            api_style,
                            started,
                            byte_stream,
                            request.telemetry.clone(),
                        ),
                        Err(err) if bifrost_key_selection_error(&err.to_string()) => {
                            tracing::warn!(
                                session_key = %session_key,
                                model = %model,
                                api_style = %api_style.as_str(),
                                error = %err,
                                "LLM responses stream hit Bifrost key-selection error; retrying via chat/completions stream"
                            );
                            let byte_stream = llm.chat_completions_byte_stream(&request).await?;
                            Self::connect_byte_stream(
                                session_key.clone(),
                                model.clone(),
                                LlmApiStyle::ChatCompletionsStream,
                                started,
                                byte_stream,
                                request.telemetry.clone(),
                            )
                        }
                        Err(err) => Err(err),
                    }
                }
            })
            .await;
            match handshake {
                Err(_) => {
                    tracing::warn!(
                        session_key = %session_key,
                        model = %model,
                        duration_ms = started.elapsed().as_millis(),
                        handshake_timeout_secs = NATIVE_LLM_HANDSHAKE_TIMEOUT.as_secs(),
                        api_style = %api_style.as_str(),
                        "LLM stream handshake timed out"
                    );
                    Err(DenError::System(format!(
                        "LLM stream handshake timed out after {}s",
                        NATIVE_LLM_HANDSHAKE_TIMEOUT.as_secs()
                    )))
                }
                Ok(Err(err)) => {
                    tracing::warn!(
                        session_key = %session_key,
                        model = %model,
                        duration_ms = started.elapsed().as_millis(),
                        error = %err,
                        api_style = %api_style.as_str(),
                        "LLM stream handshake failed"
                    );
                    if let Some(ctx) = overflow {
                        if den_error_indicates_context_overflow(&err) {
                            return Self::recover_from_overflow_and_retry(
                                ctx,
                                llm,
                                request,
                                session_key,
                                model,
                                api_style,
                                started,
                            )
                            .await;
                        }
                    }
                    Err(err)
                }
                Ok(Ok(stream)) => Ok(stream),
            }
        })
    }

    fn connect_byte_stream(
        session_key: String,
        model: String,
        api_style: LlmApiStyle,
        started: Instant,
        byte_stream: impl Stream<Item = Result<bytes::Bytes, DenError>> + Send + Unpin + 'static,
        telemetry: Option<crate::llm::LlmRequestTelemetry>,
    ) -> Result<RuntimeEventStream, DenError> {
        tracing::info!(
            session_key = %session_key,
            model = %model,
            api_style = %api_style.as_str(),
            duration_ms = started.elapsed().as_millis(),
            idle_timeout_secs = NATIVE_LLM_STREAM_IDLE_TIMEOUT.as_secs(),
            "LLM stream handshake connected"
        );
        let byte_stream =
            byte_stream_with_idle_timeout(byte_stream, NATIVE_LLM_STREAM_IDLE_TIMEOUT)
                .map_err(DenError::from);
        Ok(match api_style {
            LlmApiStyle::ChatCompletionsStream => {
                openai_byte_stream_to_event_stream_with_telemetry(byte_stream, telemetry)
            }
            LlmApiStyle::ResponsesStream => responses_byte_stream_to_event_stream(byte_stream),
        })
    }

    async fn recover_from_overflow_and_retry(
        ctx: AgentStepOverflowContext,
        llm: LlmClient,
        request: ChatCompletionRequest,
        session_key: String,
        model: String,
        api_style: LlmApiStyle,
        started: Instant,
    ) -> Result<RuntimeEventStream, DenError> {
        let session = ctx.session_store.get(&session_key).ok_or_else(|| {
            DenError::System("agent loop session not found for overflow recovery".into())
        })?;
        if session.overflow_retry_attempted {
            return Err(DenError::System(
                "LLM context overflow persists after emergency compaction retry".into(),
            ));
        }
        if CompactionMode::parse(&ctx.config.compaction_mode) != CompactionMode::Active {
            tracing::warn!(
                session_key = %session_key,
                compaction_mode = %ctx.config.compaction_mode,
                "context overflow detected but COMPACTION_MODE is not active; skipping retry"
            );
            return Err(DenError::System(
                "LLM context overflow; enable COMPACTION_MODE=active for emergency recovery".into(),
            ));
        }

        tracing::info!(
            session_key = %session_key,
            conversation_id = %session.conversation_id,
            profile = %ctx.profile.as_str(),
            "context overflow detected; running emergency compaction"
        );

        let (new_messages, recovered) =
            compact_session_messages_for_overflow(&ctx.pool, &ctx.config, &session, ctx.profile)
                .await?;

        ctx.session_store.update(&session_key, |s| {
            s.messages = new_messages.clone();
            s.overflow_retry_attempted = true;
            s.overflow_compaction_recovered = recovered;
        });

        if !recovered {
            return Err(DenError::System(
                "LLM context overflow; emergency compaction did not shrink prompt".into(),
            ));
        }

        let retry_request = ChatCompletionRequest {
            model: request.model.clone(),
            messages: repair_tool_call_message_chain(new_messages),
            tools: request.tools,
            stream: request.stream,
            tool_choice: request.tool_choice,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            telemetry: request.telemetry,
        };

        tracing::info!(
            session_key = %session_key,
            model = %model,
            api_style = %api_style.as_str(),
            message_count = retry_request.messages.len(),
            "retrying LLM stream after emergency compaction"
        );

        let handshake = timeout(NATIVE_LLM_HANDSHAKE_TIMEOUT, async {
            match api_style {
                LlmApiStyle::ChatCompletionsStream => {
                    let byte_stream = llm.chat_completions_byte_stream(&retry_request).await?;
                    Self::connect_byte_stream(
                        session_key.clone(),
                        model.clone(),
                        api_style,
                        started,
                        byte_stream,
                        retry_request.telemetry.clone(),
                    )
                }
                LlmApiStyle::ResponsesStream => {
                    let byte_stream = llm.responses_byte_stream(&retry_request).await?;
                    Self::connect_byte_stream(
                        session_key.clone(),
                        model.clone(),
                        api_style,
                        started,
                        byte_stream,
                        retry_request.telemetry.clone(),
                    )
                }
            }
        })
        .await;

        match handshake {
            Err(_) => Err(DenError::System(format!(
                "LLM stream retry timed out after {}s",
                NATIVE_LLM_HANDSHAKE_TIMEOUT.as_secs()
            ))),
            Ok(Err(err)) => {
                tracing::warn!(
                    session_key = %session_key,
                    model = %model,
                    api_style = %api_style.as_str(),
                    duration_ms = started.elapsed().as_millis(),
                    error = %err,
                    "LLM stream retry failed after emergency compaction"
                );
                Err(err)
            }
            Ok(Ok(stream)) => Ok(stream),
        }
    }
}

impl Stream for LazyAgentStepStream {
    type Item = Result<RuntimeStreamEvent, DenError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let Some(state) = self.state.take() else {
                return Poll::Ready(None);
            };
            match state {
                LazyAgentStepState::Init { mut fut } => match fut.as_mut().poll(cx) {
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
                },
                LazyAgentStepState::Streaming(mut stream) => {
                    match Pin::new(&mut stream).poll_next(cx) {
                        Poll::Ready(Some(item)) => {
                            self.state = Some(LazyAgentStepState::Streaming(stream));
                            return Poll::Ready(Some(item));
                        }
                        Poll::Ready(None) => return Poll::Ready(None),
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
    overflow: Option<AgentStepOverflowContext>,
) -> Result<RuntimeEventStream, DenError> {
    let messages = repair_tool_call_message_chain(session.messages.clone());
    tracing::info!(
        session_key = %session.session_key,
        model = %session.model,
        message_count = messages.len(),
        tool_count = session.tools.len(),
        api_style_override = session.api_style.map(|style| style.as_str()),
        step = session.step,
        overflow_recovery = overflow.is_some(),
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
        telemetry: Some(session.llm_telemetry()),
    };
    Ok(Box::pin(LazyAgentStepStream::new(
        llm.clone(),
        request,
        session.session_key.clone(),
        session.api_style,
        overflow,
    )))
}
