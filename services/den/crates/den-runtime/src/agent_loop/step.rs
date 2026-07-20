use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use den_core::{config::Config, profile::BearProfile, DenError, ThinkingEffort};
use den_protocol::{RuntimeEventStream, RuntimeSemanticEvent, RuntimeStreamEvent};
use futures::{stream, Stream, StreamExt, TryStreamExt};
use sqlx::PgPool;
use tokio::time::timeout;

use crate::{
    agent_loop::{
        context::repair_tool_call_message_chain,
        evaluate_turn_context_budget,
        overflow_retry::compact_session_messages_for_overflow,
        record_context_budget_pressure_decision,
        session_store::{AgentLoopSession, AgentLoopSessionStore},
    },
    context_budget::estimate_context_budget,
    llm::{
        bifrost_key_selection_error, byte_stream_with_idle_timeout,
        execution_fallback_model_handles, preferred_api_style_for_model, ChatCompletionRequest,
        LlmApiStyle, LlmClient,
    },
    native_runtime::{
        openai_byte_stream_to_event_stream_with_telemetry,
        responses_byte_stream_to_event_stream_with_telemetry,
    },
    runtime_compaction::{den_error_indicates_context_overflow, CompactionMode},
};

/// Default max wait for Bifrost/upstream model to accept a streaming request and return
/// response headers. Idle providers can be cold after a long session pause; keep this
/// comfortably above the old 30s watchdog while still bounded.
const DEFAULT_NATIVE_LLM_HANDSHAKE_TIMEOUT: Duration = Duration::from_mins(2);
const MIN_NATIVE_LLM_HANDSHAKE_TIMEOUT_SECS: u64 = 15;
const MAX_NATIVE_LLM_HANDSHAKE_TIMEOUT_SECS: u64 = 900;
/// Max silence between upstream SSE byte chunks after the handshake.
const NATIVE_LLM_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub fn native_llm_handshake_timeout() -> Duration {
    native_llm_handshake_timeout_from_raw(
        std::env::var("BEARS_LLM_HANDSHAKE_TIMEOUT_SECS")
            .ok()
            .as_deref(),
    )
}

fn native_llm_handshake_timeout_from_raw(raw: Option<&str>) -> Duration {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .map(|secs| {
            secs.clamp(
                MIN_NATIVE_LLM_HANDSHAKE_TIMEOUT_SECS,
                MAX_NATIVE_LLM_HANDSHAKE_TIMEOUT_SECS,
            )
        })
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_NATIVE_LLM_HANDSHAKE_TIMEOUT)
}

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
    async fn connect_request_stream(
        llm: &LlmClient,
        request: &ChatCompletionRequest,
        session_key: &str,
        model: &str,
        api_style: LlmApiStyle,
        started: Instant,
    ) -> Result<RuntimeEventStream, DenError> {
        match api_style {
            LlmApiStyle::ChatCompletionsStream => {
                let byte_stream = llm.chat_completions_byte_stream(request).await?;
                Self::connect_byte_stream(
                    session_key.to_string(),
                    model.to_string(),
                    api_style,
                    started,
                    byte_stream,
                    request.telemetry.clone(),
                )
            }
            LlmApiStyle::ResponsesStream => match llm.responses_byte_stream(request).await {
                Ok(byte_stream) => Self::connect_byte_stream(
                    session_key.to_string(),
                    model.to_string(),
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
                    let byte_stream = llm.chat_completions_byte_stream(request).await?;
                    Self::connect_byte_stream(
                        session_key.to_string(),
                        model.to_string(),
                        LlmApiStyle::ChatCompletionsStream,
                        started,
                        byte_stream,
                        request.telemetry.clone(),
                    )
                }
                Err(err) => Err(err),
            },
        }
    }

    async fn retry_with_fallback_models(
        llm: &LlmClient,
        request: &ChatCompletionRequest,
        session_key: &str,
        api_style_override: Option<LlmApiStyle>,
    ) -> Result<Option<RuntimeEventStream>, DenError> {
        let fallback_models = execution_fallback_model_handles(&request.model);
        if fallback_models.is_empty() {
            return Ok(None);
        }
        for fallback_model in fallback_models {
            let mut fallback_request = request.clone();
            fallback_request.model = (*fallback_model).to_string();
            let fallback_style =
                api_style_override.unwrap_or_else(|| preferred_api_style_for_model(fallback_model));
            tracing::warn!(
                session_key = %session_key,
                requested_model = %request.model,
                fallback_model,
                api_style = %fallback_style.as_str(),
                "retrying LLM stream with fallback model after Bifrost key-selection error"
            );
            match timeout(
                native_llm_handshake_timeout(),
                Self::connect_request_stream(
                    llm,
                    &fallback_request,
                    session_key,
                    fallback_model,
                    fallback_style,
                    Instant::now(),
                ),
            )
            .await
            {
                Ok(Ok(stream)) => {
                    tracing::info!(
                        session_key = %session_key,
                        requested_model = %request.model,
                        fallback_model,
                        api_style = %fallback_style.as_str(),
                        "LLM stream fallback model handshake succeeded"
                    );
                    return Ok(Some(stream));
                }
                Ok(Err(err)) if bifrost_key_selection_error(&err.to_string()) => {
                    tracing::warn!(
                        session_key = %session_key,
                        requested_model = %request.model,
                        fallback_model,
                        error = %err,
                        "fallback model also hit Bifrost key-selection error"
                    );
                }
                Ok(Err(err)) => return Err(err),
                Err(_) => {
                    tracing::warn!(
                        session_key = %session_key,
                        requested_model = %request.model,
                        fallback_model,
                        handshake_timeout_secs = native_llm_handshake_timeout().as_secs(),
                        "fallback model handshake timed out"
                    );
                }
            }
        }
        Ok(None)
    }

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
            let handshake_timeout = native_llm_handshake_timeout();
            tracing::info!(
                session_key = %session_key,
                model = %model,
                api_style = %api_style.as_str(),
                message_count,
                tool_count,
                handshake_timeout_secs = handshake_timeout.as_secs(),
                handshake_timeout_source = "BEARS_LLM_HANDSHAKE_TIMEOUT_SECS",
                "LLM stream handshake starting"
            );
            let handshake = timeout(
                handshake_timeout,
                Self::connect_request_stream(
                    &llm,
                    &request,
                    &session_key,
                    &model,
                    api_style,
                    started,
                ),
            )
            .await;
            match handshake {
                Err(_) => {
                    tracing::warn!(
                        session_key = %session_key,
                        model = %model,
                        duration_ms = started.elapsed().as_millis(),
                        handshake_timeout_secs = handshake_timeout.as_secs(),
                        api_style = %api_style.as_str(),
                        "LLM stream handshake timed out"
                    );
                    Err(DenError::System(format!(
                        "LLM stream handshake timed out after {}s (set BEARS_LLM_HANDSHAKE_TIMEOUT_SECS to tune cold/idle upstream startup tolerance)",
                        handshake_timeout.as_secs()
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
                    if bifrost_key_selection_error(&err.to_string()) {
                        if let Some(stream) = Self::retry_with_fallback_models(
                            &llm,
                            &request,
                            &session_key,
                            api_style_override,
                        )
                        .await?
                        {
                            return Ok(stream);
                        }
                    }
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
            LlmApiStyle::ResponsesStream => {
                responses_byte_stream_to_event_stream_with_telemetry(byte_stream, telemetry)
            }
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
            s.messages.clone_from(&new_messages);
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
            thinking_effort: request.thinking_effort,
            telemetry: request.telemetry,
        };

        tracing::info!(
            session_key = %session_key,
            model = %model,
            api_style = %api_style.as_str(),
            message_count = retry_request.messages.len(),
            "retrying LLM stream after emergency compaction"
        );

        let handshake_timeout = native_llm_handshake_timeout();
        let handshake = timeout(handshake_timeout, async {
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
                handshake_timeout.as_secs()
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
/// request; deferring the LLM call until the stream is polled avoids wedging client streams
/// that wait on `POST /prompt` with no timeout.
pub const RUNTIME_CHECKPOINT_TOOL_NAME: &str = "checkpoint";

fn api_compatible_thinking_effort(
    api_style: Option<LlmApiStyle>,
    has_function_tools: bool,
    thinking_effort: Option<ThinkingEffort>,
) -> Option<ThinkingEffort> {
    if api_style == Some(LlmApiStyle::ChatCompletionsStream) && has_function_tools {
        None
    } else {
        thinking_effort
    }
}

fn checkpoint_thinking_effort_for_session(
    session: &AgentLoopSession,
    has_function_tools: bool,
) -> Option<ThinkingEffort> {
    session.checkpoint_state.last_checkpoint_reason?;
    let policy = session.agent_loop_control.profile.thinking;
    let configured_effort = policy
        .enabled
        .then_some(policy.checkpoint_turn_effort)
        .flatten();
    let compatible_effort =
        api_compatible_thinking_effort(session.api_style, has_function_tools, configured_effort);
    if configured_effort.is_some() && compatible_effort.is_none() {
        tracing::warn!(
            session_key = %session.session_key,
            model = %session.model,
            api_style = LlmApiStyle::ChatCompletionsStream.as_str(),
            "omitting checkpoint reasoning effort because Chat Completions with function tools is incompatible"
        );
    }
    compatible_effort
}

fn checkpoint_tool_definition() -> crate::llm::LlmToolDefinition {
    crate::llm::LlmToolDefinition {
        name: RUNTIME_CHECKPOINT_TOOL_NAME.to_string(),
        description: Some(
            "Report a runtime checkpoint decision before continuing. Use this tool instead of replying with checkpoint JSON in assistant text. The fields are advisory/audit only; budgets and task tools remain authoritative.".to_string(),
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "checkpoint_id": { "type": "string" },
                "active_objective": { "type": "string" },
                "summary": { "type": "string", "description": "Short prose synthesis of learned facts, uncertainty, and rationale. Keep it concise." },
                "more_exploration_justified": { "type": "boolean" },
                "next_action": {
                    "type": "string",
                    "enum": [
                        "call_tool",
                        "edit",
                        "validate",
                        "update_task_list",
                        "sync_task_list",
                        "request_handoff",
                        "final_if_gate_allows",
                        "stop_blocked"
                    ]
                },
                "task_state_change_needed": {
                    "type": ["object", "null"],
                    "properties": {
                        "target_state": { "type": "string" },
                        "reason": { "type": "string" },
                        "evidence_refs": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "kind": { "type": "string" },
                                    "id": { "type": "string" },
                                    "summary": { "type": ["string", "null"] }
                                },
                                "required": ["kind", "id"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["target_state", "reason", "evidence_refs"],
                    "additionalProperties": false
                },
                "evidence_refs": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string" },
                            "id": { "type": "string" },
                            "summary": { "type": ["string", "null"] }
                        },
                        "required": ["kind", "id"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["checkpoint_id", "active_objective", "more_exploration_justified", "next_action"],
            "additionalProperties": false
        }),
    }
}

fn should_try_preflight_context_compaction(
    stop_requested: bool,
    recovered_this_step: bool,
    retry_already_attempted: bool,
    compaction_mode: &str,
) -> bool {
    stop_requested
        && !recovered_this_step
        && !retry_already_attempted
        && CompactionMode::parse(compaction_mode) == CompactionMode::Active
}

fn tools_with_checkpoint_tool(session: &AgentLoopSession) -> Vec<crate::llm::LlmToolDefinition> {
    let mut tools = session.tools.clone();
    if session.pending_checkpoint_request.is_some()
        && !tools
            .iter()
            .any(|tool| tool.name == RUNTIME_CHECKPOINT_TOOL_NAME)
    {
        tools.push(checkpoint_tool_definition());
    }
    tools
}

pub async fn run_agent_step_stream(
    llm: &LlmClient,
    session: &AgentLoopSession,
    overflow: Option<AgentStepOverflowContext>,
) -> Result<RuntimeEventStream, DenError> {
    let mut session = session.clone();
    let mut recovered_from_preflight_context_budget = false;
    let mut messages = repair_tool_call_message_chain(session.messages.clone());
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
    let (request, budget, context_budget_evaluation) = loop {
        let tools = tools_with_checkpoint_tool(&session);
        let thinking_effort = checkpoint_thinking_effort_for_session(&session, !tools.is_empty());
        let request = ChatCompletionRequest {
            model: session.model.clone(),
            messages,
            tools,
            stream: true,
            tool_choice: None,
            temperature: None,
            max_tokens: None,
            thinking_effort,
            telemetry: Some(session.llm_telemetry()),
        };
        let budget = estimate_context_budget(
            &request,
            &session.budget_components,
            session.model_context_window,
            session.model_max_output_tokens,
        );
        let context_budget_evaluation =
            evaluate_turn_context_budget(&session.turn_budget_state, budget);
        let budget = context_budget_evaluation
            .next_state
            .latest_context_budget
            .clone()
            .expect("context budget evaluation stores the latest report");
        if let Some(overflow) = overflow.as_ref() {
            overflow
                .session_store
                .update(&session.session_key, |stored| {
                    stored.latest_context_budget = Some(budget.clone());
                    stored.turn_budget_state = context_budget_evaluation.next_state.clone();
                });
            let _ = den_service::conversation::persistence::update_latest_context_budget(
                &overflow.pool,
                session.bear_id,
                &session.conversation_id,
                Some(&session.client_session_id),
                &budget,
            )
            .await;
            if budget.near_budget || budget.over_budget {
                if let Some(run_id) = session.run_id.as_deref() {
                    let _ = record_context_budget_pressure_decision(
                        &overflow.pool,
                        run_id,
                        None,
                        Some(session.objective_orientation.kind().to_string()),
                        &budget,
                    )
                    .await;
                }
            }
        }
        if overflow.as_ref().is_none_or(|overflow| {
            !should_try_preflight_context_compaction(
                context_budget_evaluation.stop_reason.is_some(),
                recovered_from_preflight_context_budget,
                session.overflow_retry_attempted,
                &overflow.config.compaction_mode,
            )
        }) {
            break (request, budget, context_budget_evaluation);
        }
        let Some(overflow) = overflow.as_ref() else {
            break (request, budget, context_budget_evaluation);
        };
        tracing::info!(
            session_key = %session.session_key,
            conversation_id = %session.conversation_id,
            profile = %overflow.profile.as_str(),
            "context budget exceeded before LLM call; running emergency compaction"
        );
        let (new_messages, recovered) = compact_session_messages_for_overflow(
            &overflow.pool,
            &overflow.config,
            &session,
            overflow.profile,
        )
        .await?;
        overflow
            .session_store
            .update(&session.session_key, |stored| {
                stored.messages.clone_from(&new_messages);
                stored.overflow_retry_attempted = true;
                stored.overflow_compaction_recovered = recovered;
            });
        if !recovered {
            break (request, budget, context_budget_evaluation);
        }
        session.messages = new_messages;
        session.overflow_retry_attempted = true;
        session.overflow_compaction_recovered = true;
        session.turn_budget_state = context_budget_evaluation.next_state.clone();
        messages = repair_tool_call_message_chain(session.messages.clone());
        recovered_from_preflight_context_budget = true;
    };
    if let Some(warning) = context_budget_evaluation.warning.as_ref() {
        tracing::warn!(
            session_key = %session.session_key,
            model = %budget.model,
            context_window = budget.context_window,
            estimated_input_tokens = budget.estimated_input_tokens,
            reserved_output_tokens = budget.reserved_output_tokens,
            estimated_total_tokens = budget.estimated_total_tokens,
            warning_code = warning.code,
            "context budget is near model limit"
        );
    }
    if let Some(reason) = context_budget_evaluation.stop_reason.as_ref() {
        return Err(DenError::ValidationError(reason.user_message()));
    }
    let context_budget_pressure_event = context_budget_evaluation.warning.as_ref().map(|warning| {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress {
            kind: "context_budget_pressure".to_string(),
            text: Some(warning.model_message().to_string()),
            phase: Some("agent_loop_control".to_string()),
            detail: Some(serde_json::json!({
                "model": budget.model,
                "context_window": budget.context_window,
                "estimated_input_tokens": budget.estimated_input_tokens,
                "reserved_output_tokens": budget.reserved_output_tokens,
                "estimated_total_tokens": budget.estimated_total_tokens,
                "near_budget": budget.near_budget,
                "over_budget": budget.over_budget,
                "action": "prefer_checkpoint_before_more_context_growth",
            })),
        })
    });
    let checkpoint_thinking_event = request.thinking_effort.map(|effort| {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress {
            kind: "checkpoint_thinking_override_applied".to_string(),
            text: Some(format!(
                "Applied checkpoint thinking effort `{}` for this model call.",
                effort.as_str()
            )),
            phase: Some("agent_loop_control".to_string()),
            detail: Some(serde_json::json!({
                "effort": effort.as_str(),
                "reason": session.checkpoint_state.last_checkpoint_reason,
                "model": session.model,
                "control_level": session.agent_loop_control.level,
            })),
        })
    });
    let base_stream = Box::pin(LazyAgentStepStream::new(
        llm.clone(),
        request,
        session.session_key.clone(),
        session.api_style,
        overflow,
    )) as RuntimeEventStream;
    let prefix_events = [context_budget_pressure_event, checkpoint_thinking_event]
        .into_iter()
        .flatten()
        .map(Ok)
        .collect::<Vec<_>>();
    if prefix_events.is_empty() {
        Ok(base_stream)
    } else {
        Ok(Box::pin(stream::iter(prefix_events).chain(base_stream)) as RuntimeEventStream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_effort_is_omitted_for_chat_completions_with_function_tools() {
        let effort = Some(ThinkingEffort::Medium);

        assert_eq!(
            api_compatible_thinking_effort(Some(LlmApiStyle::ChatCompletionsStream), true, effort),
            None
        );
        assert_eq!(
            api_compatible_thinking_effort(Some(LlmApiStyle::ChatCompletionsStream), false, effort),
            effort
        );
        assert_eq!(
            api_compatible_thinking_effort(Some(LlmApiStyle::ResponsesStream), true, effort),
            effort
        );
        assert_eq!(api_compatible_thinking_effort(None, true, effort), effort);
    }

    #[test]
    fn preflight_context_compaction_only_runs_once_for_active_overflow_recovery() {
        assert!(should_try_preflight_context_compaction(
            true, false, false, "active"
        ));
        assert!(!should_try_preflight_context_compaction(
            false, false, false, "active"
        ));
        assert!(!should_try_preflight_context_compaction(
            true, true, false, "active"
        ));
        assert!(!should_try_preflight_context_compaction(
            true, false, true, "active"
        ));
        assert!(!should_try_preflight_context_compaction(
            true, false, false, "off"
        ));
    }

    #[test]
    fn native_llm_handshake_timeout_defaults_to_cold_start_tolerant_value() {
        assert_eq!(
            native_llm_handshake_timeout_from_raw(None),
            DEFAULT_NATIVE_LLM_HANDSHAKE_TIMEOUT
        );
    }

    #[test]
    fn chat_completions_with_tools_omits_incompatible_thinking_effort() {
        assert_eq!(
            api_compatible_thinking_effort(
                Some(LlmApiStyle::ChatCompletionsStream),
                true,
                Some(ThinkingEffort::Low),
            ),
            None
        );
        assert_eq!(
            api_compatible_thinking_effort(
                Some(LlmApiStyle::ChatCompletionsStream),
                false,
                Some(ThinkingEffort::Low),
            ),
            Some(ThinkingEffort::Low)
        );
        assert_eq!(
            api_compatible_thinking_effort(
                Some(LlmApiStyle::ResponsesStream),
                true,
                Some(ThinkingEffort::Low),
            ),
            Some(ThinkingEffort::Low)
        );
    }

    #[test]
    fn native_llm_handshake_timeout_parses_and_clamps_env_value() {
        assert_eq!(
            native_llm_handshake_timeout_from_raw(Some("90")),
            Duration::from_secs(90)
        );
        assert_eq!(
            native_llm_handshake_timeout_from_raw(Some("1")),
            Duration::from_secs(MIN_NATIVE_LLM_HANDSHAKE_TIMEOUT_SECS)
        );
        assert_eq!(
            native_llm_handshake_timeout_from_raw(Some("9999")),
            Duration::from_secs(MAX_NATIVE_LLM_HANDSHAKE_TIMEOUT_SECS)
        );
        assert_eq!(
            native_llm_handshake_timeout_from_raw(Some("not-a-number")),
            DEFAULT_NATIVE_LLM_HANDSHAKE_TIMEOUT
        );
    }
}
