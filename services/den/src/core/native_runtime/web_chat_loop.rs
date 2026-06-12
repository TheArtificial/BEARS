//! Multi-step native web chat turn: executes Den server tools in-process and continues the loop.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

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

enum LoopPhase {
    Streaming(Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, CustomError>> + Send>>),
    Driving(Pin<Box<dyn std::future::Future<Output = Result<LoopDriveOutcome, CustomError>> + Send>>),
    Finished,
}

enum LoopDriveOutcome {
    NextStep(RuntimeEventStream),
    TurnComplete,
    Failed(RuntimeStreamEvent),
}

pub struct NativeWebChatLoopStream {
    runtime: NativeWebChatLoopRuntime,
    phase: LoopPhase,
    pending_out: VecDeque<RuntimeStreamEvent>,
    saw_turn_completed: bool,
}

impl NativeWebChatLoopStream {
    pub fn new(runtime: NativeWebChatLoopRuntime, initial: RuntimeEventStream) -> Self {
        Self {
            runtime,
            phase: LoopPhase::Streaming(initial),
            pending_out: VecDeque::new(),
            saw_turn_completed: false,
        }
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
}

impl Stream for NativeWebChatLoopStream {
    type Item = Result<RuntimeStreamEvent, CustomError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(event) = self.pending_out.pop_front() {
            return Poll::Ready(Some(Ok(event)));
        }

        loop {
            match &mut self.phase {
                LoopPhase::Finished => return Poll::Ready(None),
                LoopPhase::Driving(future) => match future.as_mut().poll(cx) {
                    Poll::Ready(Ok(LoopDriveOutcome::NextStep(stream))) => {
                        self.saw_turn_completed = false;
                        self.phase = LoopPhase::Streaming(stream);
                    }
                    Poll::Ready(Ok(LoopDriveOutcome::TurnComplete)) => {
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
                    Poll::Ready(Ok(LoopDriveOutcome::Failed(event))) => {
                        self.phase = LoopPhase::Finished;
                        return Poll::Ready(Some(Ok(event)));
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
                        return Poll::Ready(Some(Ok(event)));
                    }
                    Poll::Ready(Some(item)) => return Poll::Ready(Some(item)),
                    Poll::Ready(None) => {
                        let runtime = self.runtime.clone();
                        self.phase = LoopPhase::Driving(Box::pin(drive_after_step(runtime)));
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                },
            }
        }
    }
}

async fn drive_after_step(runtime: NativeWebChatLoopRuntime) -> Result<LoopDriveOutcome, CustomError> {
    let session = runtime
        .session_store
        .get(&runtime.session_key)
        .ok_or_else(|| CustomError::System("native web chat session not found".to_string()))?;
    let pending = pending_tool_calls(&session.messages);
    if pending.is_empty() {
        return Ok(LoopDriveOutcome::TurnComplete);
    }
    if session.step >= session.max_steps {
        return Ok(LoopDriveOutcome::Failed(RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::TurnFailed {
                turn: None,
                category: RuntimeErrorCategory::Internal,
                message: "native web chat reached max agent steps".to_string(),
            },
        )));
    }

    let tool_messages = execute_web_chat_den_tools(&runtime, pending).await?;
    runtime.session_store.update(&runtime.session_key, |session| {
        session.messages.extend(tool_messages);
    });

    let session = runtime
        .session_store
        .get(&runtime.session_key)
        .ok_or_else(|| CustomError::System("native web chat session not found".to_string()))?;
    let raw = run_agent_step_stream(&runtime.llm, &session).await?;
    let stream = NativeWebChatLoopStream::wrap_step_stream(&runtime, raw, &session);
    Ok(LoopDriveOutcome::NextStep(stream))
}

async fn execute_web_chat_den_tools(
    runtime: &NativeWebChatLoopRuntime,
    calls: Vec<ChatToolCall>,
) -> Result<Vec<ChatMessage>, CustomError> {
    let mut results = Vec::with_capacity(calls.len());
    for call in calls {
        let provider_name = call.function.name.clone();
        let canonical = builtin_den_tool_descriptor_for_provider_name(&provider_name)
            .map(|descriptor| descriptor.name.to_string())
            .unwrap_or_else(|| provider_name.clone());
        let args: Value = serde_json::from_str(&call.function.arguments).unwrap_or_else(|_| {
            serde_json::json!({})
        });
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
        results.push(ChatMessage {
            role: "tool".to_string(),
            content: Some(content),
            tool_call_id: Some(call.id),
            name: Some(provider_name),
            tool_calls: None,
        });
    }
    Ok(results)
}
