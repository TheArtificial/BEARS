//! Multi-step native web chat turn: executes Den server tools in-process and continues the loop.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::Stream;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    config::Config,
    core::{
        agent_loop::{
            pending_tool_calls, run_agent_step_stream, provider_tool_requires_approval,
            AgentLoopSessionStore, NativeToolDispatchMode, SessionTrackingStream,
        },
        bears::BearProfile,
        llm::{ChatMessage, ChatToolCall, LlmClient},
        memory::MemoryStoreManager,
        runtime_contracts::{
            RuntimeErrorCategory, RuntimeEventStream, RuntimeSemanticEvent, RuntimeStreamEvent,
        },
        tools::{
            arguments::DenToolChannelContext,
            descriptor::builtin_den_tool_descriptor_for_provider_name,
            session::{invoke_den_tool, DenToolInvocationContext},
        },
    },
    errors::CustomError,
};

const WEB_CHAT_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const WEB_CHAT_TURN_BUDGET: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct NativeWebChatLoopRuntime {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub stores: MemoryStoreManager,
    pub llm: LlmClient,
    pub session_key: String,
    pub bear_id: Uuid,
    pub bear_slug: String,
    pub chat_binding_id: String,
    pub user_id: i32,
    pub username: Option<String>,
    pub membership_role: Option<String>,
    pub conversation_id: String,
    pub session_id: String,
    pub request_id: String,
    pub session_store: AgentLoopSessionStore,
}

type ToolExecFuture =
    Pin<Box<dyn Future<Output = Result<ChatMessage, CustomError>> + Send>>;
type NextStepFuture =
    Pin<Box<dyn Future<Output = Result<RuntimeEventStream, CustomError>> + Send>>;

enum LoopPhase {
    Streaming(Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>>),
    ExecutingTools {
        calls: Vec<ChatToolCall>,
        index: usize,
        results: Vec<ChatMessage>,
        active: Option<ToolExecFuture>,
    },
    StartingNextStep(NextStepFuture),
    Finished,
}

pub struct NativeWebChatLoopStream {
    runtime: NativeWebChatLoopRuntime,
    phase: LoopPhase,
    pending_out: VecDeque<RuntimeStreamEvent>,
    saw_turn_completed: bool,
    last_outbound: Instant,
    started_at: Instant,
}

fn turn_timeout_event() -> RuntimeStreamEvent {
    RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed {
        turn: None,
        category: RuntimeErrorCategory::Timeout,
        message: "This reply took too long and was stopped. Try a simpler question or retry.".to_string(),
    })
}

fn keepalive_event() -> RuntimeStreamEvent {
    status_event("Still working…")
}

fn status_event(text: impl Into<String>) -> RuntimeStreamEvent {
    RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress {
        kind: "status".to_string(),
        text: Some(text.into()),
        phase: None,
        detail: None,
    })
}

fn tool_started_event(call: &ChatToolCall) -> RuntimeStreamEvent {
    RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
        tool_call_id: call.id.clone(),
        tool_name: call.function.name.clone(),
        title: Some(call.function.name.clone()),
        kind: Some("function".to_string()),
        arguments: serde_json::from_str(&call.function.arguments).unwrap_or_else(|_| Value::Object(Default::default())),
        approval_request_id: None,
        approval_required: provider_tool_requires_approval(&call.function.name),
        approval_reason: None,
        run_id: None,
    })
}

impl NativeWebChatLoopStream {
    pub fn new(runtime: NativeWebChatLoopRuntime, initial: RuntimeEventStream) -> Self {
        Self {
            runtime,
            phase: LoopPhase::Streaming(initial),
            pending_out: VecDeque::from([status_event("Thinking…")]),
            saw_turn_completed: false,
            last_outbound: Instant::now(),
            started_at: Instant::now(),
        }
    }

    fn touch_outbound(&mut self) {
        self.last_outbound = Instant::now();
    }

    fn maybe_keepalive(&mut self) -> Option<RuntimeStreamEvent> {
        if matches!(self.phase, LoopPhase::Finished) {
            return None;
        }
        if self.last_outbound.elapsed() < WEB_CHAT_KEEPALIVE_INTERVAL {
            return None;
        }
        self.touch_outbound();
        Some(keepalive_event())
    }

    pub(crate) fn wrap_step_stream(
        runtime: &NativeWebChatLoopRuntime,
        stream: RuntimeEventStream,
        session: &crate::core::agent_loop::AgentLoopSession,
    ) -> RuntimeEventStream {
        Box::pin(SessionTrackingStream::new(
            stream,
            session,
            runtime.session_store.clone(),
            runtime.pool.clone(),
            runtime.bear_id,
            Some(runtime.user_id),
            runtime.conversation_id.clone(),
            runtime.session_id.clone(),
            Some(runtime.request_id.clone()),
            NativeToolDispatchMode::ServerSideInProcess,
        ))
    }

    fn begin_tool_execution(&mut self, calls: Vec<ChatToolCall>) {
        self.pending_out.push_back(status_event("Running tools…"));
        self.phase = LoopPhase::ExecutingTools {
            calls,
            index: 0,
            results: Vec::new(),
            active: None,
        };
    }

    fn begin_next_step(&mut self) {
        let runtime = self.runtime.clone();
        self.pending_out.push_back(status_event("Continuing…"));
        self.phase = LoopPhase::StartingNextStep(Box::pin(async move {
            let session = runtime
                .session_store
                .get(&runtime.session_key)
                .ok_or_else(|| CustomError::System("native web chat session not found".to_string()))?;
            let raw = run_agent_step_stream(&runtime.llm, &session).await?;
            Ok(NativeWebChatLoopStream::wrap_step_stream(&runtime, raw, &session))
        }));
    }

    fn poll_tool_execution(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<RuntimeStreamEvent, CustomError>>> {
        let LoopPhase::ExecutingTools {
            calls,
            index,
            results,
            active,
        } = &mut self.phase
        else {
            return Poll::Pending;
        };

        if *index >= calls.len() {
            let completed = std::mem::take(results);
            let runtime = self.runtime.clone();
            runtime.session_store.update(&runtime.session_key, |session| {
                session.messages.extend(completed);
            });
            self.begin_next_step();
            return Poll::Pending;
        }

        if active.is_none() {
            let call = calls[*index].clone();
            self.pending_out.push_back(tool_started_event(&call));
            let runtime = self.runtime.clone();
            *active = Some(Box::pin(async move { execute_one_web_chat_den_tool(&runtime, call).await }));
        }

        let Some(fut) = active.as_mut() else {
            return Poll::Pending;
        };
        match fut.as_mut().poll(cx) {
            Poll::Ready(Ok(message)) => {
                results.push(message);
                *index += 1;
                *active = None;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(Err(error)) => {
                self.phase = LoopPhase::Finished;
                Poll::Ready(Some(Err(error)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl Stream for NativeWebChatLoopStream {
    type Item = Result<RuntimeStreamEvent, CustomError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(event) = self.pending_out.pop_front() {
            self.touch_outbound();
            return Poll::Ready(Some(Ok(event)));
        }
        if let Some(event) = self.maybe_keepalive() {
            return Poll::Ready(Some(Ok(event)));
        }
        if self.started_at.elapsed() >= WEB_CHAT_TURN_BUDGET {
            self.phase = LoopPhase::Finished;
            self.touch_outbound();
            return Poll::Ready(Some(Ok(turn_timeout_event())));
        }

        loop {
            match &mut self.phase {
                LoopPhase::Finished => return Poll::Ready(None),
                LoopPhase::ExecutingTools { .. } => return self.poll_tool_execution(cx),
                LoopPhase::StartingNextStep(future) => match future.as_mut().poll(cx) {
                    Poll::Ready(Ok(stream)) => {
                        self.saw_turn_completed = false;
                        self.phase = LoopPhase::Streaming(stream);
                    }
                    Poll::Ready(Err(error)) => {
                        self.phase = LoopPhase::Finished;
                        return Poll::Ready(Some(Err(error)));
                    }
                    Poll::Pending => return Poll::Pending,
                },
                LoopPhase::Streaming(stream) => match stream.as_mut().poll_next(cx) {
                    Poll::Ready(Some(Ok(
                        event @ RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. }),
                    ))) => {
                        self.saw_turn_completed = true;
                        self.touch_outbound();
                        return Poll::Ready(Some(Ok(event)));
                    }
                    Poll::Ready(Some(item)) => {
                        self.touch_outbound();
                        return Poll::Ready(Some(item));
                    }
                    Poll::Ready(None) => {
                        let session = match self.runtime.session_store.get(&self.runtime.session_key) {
                            Some(session) => session,
                            None => {
                                self.phase = LoopPhase::Finished;
                                return Poll::Ready(Some(Err(CustomError::System(
                                    "native web chat session not found".to_string(),
                                ))));
                            }
                        };
                        let pending = pending_tool_calls(&session.messages);
                        if pending.is_empty() {
                            if !self.saw_turn_completed {
                                self.pending_out.push_back(RuntimeStreamEvent::Semantic(
                                    RuntimeSemanticEvent::TurnCompleted { turn: None },
                                ));
                            }
                            self.phase = LoopPhase::Finished;
                            if let Some(event) = self.pending_out.pop_front() {
                                return Poll::Ready(Some(Ok(event)));
                            }
                            return Poll::Ready(None);
                        }
                        if session.step >= session.max_steps {
                            self.phase = LoopPhase::Finished;
                            return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                                RuntimeSemanticEvent::TurnFailed {
                                    turn: None,
                                    category: RuntimeErrorCategory::Internal,
                                    message: "native web chat reached max agent steps".to_string(),
                                },
                            ))));
                        }
                        self.begin_tool_execution(pending);
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                },
            }
        }
    }
}

async fn execute_one_web_chat_den_tool(
    runtime: &NativeWebChatLoopRuntime,
    call: ChatToolCall,
) -> Result<ChatMessage, CustomError> {
    let provider_name = call.function.name.clone();
    let canonical = builtin_den_tool_descriptor_for_provider_name(&provider_name)
        .map(|descriptor| descriptor.name.to_string())
        .unwrap_or_else(|| provider_name.clone());
    let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or_else(|_| Value::Object(Default::default()));
    let content = if provider_tool_requires_approval(&provider_name) {
        serde_json::json!({
            "ok": false,
            "error": "This tool requires interactive approval, which web chat does not support yet."
        })
        .to_string()
    } else if builtin_den_tool_descriptor_for_provider_name(&provider_name).is_none() {
        format!("unsupported server tool: {provider_name}")
    } else {
        let tool_context = DenToolInvocationContext {
            bear_id: runtime.bear_id,
            bear_slug: runtime.bear_slug.clone(),
            binding_id: runtime.chat_binding_id.clone(),
            profile: Some(BearProfile::Chat),
            user_id: runtime.user_id,
            username: runtime.username.clone(),
            membership_role: runtime.membership_role.clone(),
            conversation_id: runtime.conversation_id.clone(),
            session_id: runtime.session_id.clone(),
            acp_session_id: None,
            conversation_selection: Some(runtime.conversation_id.clone()),
            runtime_target: Some(runtime.conversation_id.clone()),
            workspace_roots: Vec::new(),
            session_policy: None,
            activity: None,
            runtime: None,
            context_budget: None,
            request_id: Some(runtime.request_id.clone()),
            channel: DenToolChannelContext {
                family: Some("browser_chat".to_string()),
                client: Some("den_web".to_string()),
                protocol: Some("den_chat".to_string()),
            },
        };
        match invoke_den_tool(
            &runtime.pool,
            runtime.config.as_ref(),
            &runtime.stores,
            &canonical,
            args,
            tool_context,
        )
        .await
        {
            Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
            Err(error) => format!("error: {error}"),
        }
    };
    Ok(ChatMessage {
        role: "tool".to_string(),
        content: Some(content),
        tool_call_id: Some(call.id),
        name: Some(provider_name),
        tool_calls: None,
    })
}
