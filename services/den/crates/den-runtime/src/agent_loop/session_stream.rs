use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use den_memory::MemoryStoreManager;
use den_protocol::{RuntimeEventStream, RuntimeSemanticEvent, RuntimeStreamEvent};
use futures::Stream;

use crate::runtime::turn_state::{
    autonomous_execution_gate_for_task_list, detect_task_focus_loop,
    should_allow_terminal_response_for_task_list,
};
use crate::{
    agent_loop::{
        approvals::create_native_approval,
        run_agent_step_stream,
        session_store::AgentLoopSessionStore,
        tool_call_finished_event_for_content,
        tool_policy::{
            maybe_pause_for_tool_approval, provider_tool_requires_approval,
            provider_tool_supports_unilateral_execution,
        },
        AgentStepOverflowContext,
    },
    llm::{ChatMessage, ChatToolCall, LlmClient},
    runtime_compaction::enqueue_compaction_after_turn,
    tool_output_artifacts::{create_tool_output_artifact, ToolOutputArtifactInput},
};
use den_core::tools::{
    arguments::DenToolChannelContext,
    constants::{DEN_TOOL_OUTPUT_READ, DEN_WEB_FETCH},
    context::DenToolInvocationContext,
    descriptor::builtin_den_tool_descriptor_for_provider_name,
    result_compaction::{compact_json_tool_result, compact_json_tool_result_with_artifact},
};
use den_core::{config::Config, profile::BearProfile, DenError};

use super::session_store::AgentLoopSession;
use super::transcript::{
    spawn_persist_incomplete_acp_tool_results, spawn_persist_native_agent_step,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativeToolDispatchMode {
    /// armature and similar clients execute tools and continue via `/tool-results`.
    #[default]
    DeferToClient,
    /// Browser web chat executes Den server tools in-process and continues the loop.
    ServerSideInProcess,
}

type ApprovalPauseFuture =
    Pin<Box<dyn Future<Output = Result<Option<RuntimeSemanticEvent>, DenError>> + Send>>;
type ServerToolFuture = Pin<
    Box<
        dyn Future<Output = Result<(ChatToolCall, ChatMessage, RuntimeEventStream), DenError>>
            + Send,
    >,
>;
type FinalGateContinuationFuture =
    Pin<Box<dyn Future<Output = Result<RuntimeEventStream, DenError>> + Send>>;

async fn tool_output_read_result(
    pool: &sqlx::PgPool,
    bear_id: uuid::Uuid,
    session_id: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, DenError> {
    let artifact_ref = args
        .get("artifact_ref")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DenError::ValidationError("artifact_ref is required".to_string()))?;
    let offset = args
        .get("offset")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let limit_chars = args
        .get("limit_chars")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(12_000)
        .clamp(1, 24_000) as usize;
    let read = crate::tool_output_artifacts::read_tool_output_artifact(
        pool,
        bear_id,
        session_id,
        artifact_ref,
        offset,
        limit_chars,
    )
    .await?;
    Ok(serde_json::json!({
        "artifact_ref": read.artifact_ref,
        "tool_call_id": read.tool_call_id,
        "tool_name": read.tool_name,
        "source": read.source,
        "offset": read.offset,
        "limit_chars": read.limit_chars,
        "total_chars": read.total_chars,
        "truncated": read.truncated,
        "content": read.content,
        "metadata": read.metadata,
    }))
}

pub struct SessionTrackingStream {
    inner: Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, DenError>> + Send>>,
    session_key: String,
    store: AgentLoopSessionStore,
    assistant_text: String,
    tool_calls: HashMap<String, (String, String)>,
    pool: sqlx::PgPool,
    bear_id: uuid::Uuid,
    bear_slug: String,
    user_id: Option<i32>,
    conversation_id: String,
    client_session_id: String,
    request_id: Option<String>,
    finished: bool,
    assistant_synced_to_session: bool,
    pending_approval: Option<ApprovalPauseFuture>,
    pending_tool_event: Option<RuntimeStreamEvent>,
    pending_pause_after_tool: Option<RuntimeSemanticEvent>,
    pending_server_tool: Option<ServerToolFuture>,
    pending_server_tool_continuation: Option<String>,
    pending_final_gate_continuation: Option<FinalGateContinuationFuture>,
    dispatch_mode: NativeToolDispatchMode,
    config: Arc<Config>,
    stores: MemoryStoreManager,
    profile: BearProfile,
}

impl SessionTrackingStream {
    pub fn new(
        inner: Pin<Box<dyn Stream<Item = Result<RuntimeStreamEvent, DenError>> + Send>>,
        session: &AgentLoopSession,
        store: AgentLoopSessionStore,
        pool: sqlx::PgPool,
        bear_id: uuid::Uuid,
        bear_slug: String,
        user_id: Option<i32>,
        conversation_id: String,
        client_session_id: String,
        request_id: Option<String>,
        config: Arc<Config>,
        stores: MemoryStoreManager,
        profile: BearProfile,
        dispatch_mode: NativeToolDispatchMode,
    ) -> Self {
        Self {
            inner,
            session_key: session.session_key.clone(),
            store,
            assistant_text: String::new(),
            tool_calls: HashMap::new(),
            pool,
            bear_id,
            bear_slug,
            user_id,
            conversation_id,
            client_session_id,
            request_id,
            finished: false,
            assistant_synced_to_session: false,
            pending_approval: None,
            pending_tool_event: None,
            pending_pause_after_tool: None,
            pending_server_tool: None,
            pending_server_tool_continuation: None,
            pending_final_gate_continuation: None,
            dispatch_mode,
            config,
            stores,
            profile,
        }
    }

    fn accumulated_tool_calls(&self) -> Vec<ChatToolCall> {
        self.tool_calls
            .iter()
            .map(|(id, (name, args))| ChatToolCall {
                id: id.clone(),
                call_type: "function".to_string(),
                function: crate::llm::ChatToolCallFunction {
                    name: name.clone(),
                    arguments: args.clone(),
                },
            })
            .collect()
    }

    fn assistant_content(&self) -> Option<String> {
        if self.assistant_text.is_empty() {
            None
        } else {
            Some(self.assistant_text.clone())
        }
    }

    fn sync_assistant_tool_step_to_session(&mut self) {
        if self.tool_calls.is_empty() {
            return;
        }
        let calls = self.accumulated_tool_calls();
        let content = self.assistant_content();
        let already_synced = self.assistant_synced_to_session;
        self.store.update(&self.session_key, |session| {
            upsert_assistant_tool_step_in_messages(
                &mut session.messages,
                content.clone(),
                &calls,
                already_synced,
            );
        });
        self.assistant_synced_to_session = true;
    }

    fn remove_recent_server_tool_chain_from_session(&self, tool_call_id: &str) {
        self.store.update(&self.session_key, |session| {
            let Some(last) = session.messages.last() else {
                return;
            };
            if last.role != "tool" || last.tool_call_id.as_deref() != Some(tool_call_id) {
                return;
            }
            let tool_index = session.messages.len() - 1;
            if tool_index == 0 {
                session.messages.pop();
                return;
            }
            let assistant_index = tool_index - 1;
            let assistant_matches = session.messages[assistant_index].role == "assistant"
                && session.messages[assistant_index]
                    .tool_calls
                    .as_ref()
                    .is_some_and(|calls| calls.iter().any(|call| call.id == tool_call_id));
            if assistant_matches {
                session.messages.truncate(assistant_index);
            } else {
                session.messages.pop();
            }
        });
    }

    fn persist_outstanding_tools_as_incomplete(&self, reason: &str) {
        if self.tool_calls.is_empty() {
            return;
        }
        let calls = self.accumulated_tool_calls();
        spawn_persist_incomplete_acp_tool_results(
            self.pool.clone(),
            self.bear_id,
            self.user_id,
            self.conversation_id.clone(),
            self.client_session_id.clone(),
            self.request_id.clone(),
            &calls,
            reason,
        );
    }

    fn server_tool_context(&self) -> DenToolInvocationContext {
        let workspace_roots = self
            .store
            .get(&self.session_key)
            .map(|session| session.workspace_roots)
            .unwrap_or_default();
        let context_budget = self
            .store
            .get(&self.session_key)
            .and_then(|session| session.latest_context_budget)
            .and_then(|report| serde_json::to_value(report).ok());
        let projected_memory = self
            .store
            .get(&self.session_key)
            .and_then(|session| session.latest_projected_memory);
        let recalled_memory = self
            .store
            .get(&self.session_key)
            .and_then(|session| session.latest_recalled_memory);
        DenToolInvocationContext {
            bear_id: self.bear_id,
            bear_slug: self.bear_slug.clone(),
            binding_id: format!("den-native:{}:{}", self.bear_id, self.profile.as_str()),
            profile: Some(self.profile),
            user_id: self.user_id.unwrap_or_default(),
            username: None,
            membership_role: None,
            conversation_id: self.conversation_id.clone(),
            session_id: self.client_session_id.clone(),
            client_session_id: Some(self.client_session_id.clone()),
            conversation_selection: Some(self.conversation_id.clone()),
            runtime_target: Some(self.conversation_id.clone()),
            workspace_roots,
            session_policy: None,
            activity: None,
            runtime: None,
            context_budget,
            projected_memory,
            recalled_memory,
            request_id: self.request_id.clone(),
            channel: DenToolChannelContext {
                family: Some("armature".to_string()),
                client: Some("bearwire".to_string()),
                protocol: Some("bearwire".to_string()),
            },
        }
    }

    fn should_request_den_tool_permission(&self, tool_name: &str) -> bool {
        self.dispatch_mode == NativeToolDispatchMode::DeferToClient
            && builtin_den_tool_descriptor_for_provider_name(tool_name)
                .is_some_and(|descriptor| descriptor.name == DEN_WEB_FETCH)
    }

    fn should_execute_den_tool_server_side(&self, tool_name: &str) -> bool {
        self.dispatch_mode == NativeToolDispatchMode::DeferToClient
            && builtin_den_tool_descriptor_for_provider_name(tool_name).is_some()
            && provider_tool_supports_unilateral_execution(tool_name)
            && !self.should_request_den_tool_permission(tool_name)
    }

    fn web_fetch_permission_target(arguments: &serde_json::Value) -> serde_json::Value {
        let mut target = arguments.clone();
        if !target.is_object() {
            target = serde_json::json!({});
        }
        target["kind"] = serde_json::json!("web_fetch");
        if let Some(url) = target.get("url").and_then(|value| value.as_str()) {
            if let Ok(parsed) = url::Url::parse(url.trim()) {
                if let Some(host) = parsed.host_str() {
                    let host = match parsed.port() {
                        Some(port)
                            if !((parsed.scheme() == "https" && port == 443)
                                || (parsed.scheme() == "http" && port == 80)) =>
                        {
                            format!("{}:{port}", host.trim_end_matches('.').to_ascii_lowercase())
                        }
                        _ => host.trim_end_matches('.').to_ascii_lowercase(),
                    };
                    target["host"] = serde_json::json!(host);
                }
            }
        }
        target
    }

    fn plan_update_event_from_tool_message(message: &ChatMessage) -> Option<RuntimeSemanticEvent> {
        let content = message.content.as_deref()?;
        let value: serde_json::Value = serde_json::from_str(content).ok()?;
        let entries = value
            .get("plan")
            .and_then(|plan| plan.get("items"))
            .and_then(|items| items.as_array())?
            .clone();
        if entries.is_empty() {
            return None;
        }
        Some(RuntimeSemanticEvent::RunProgress {
            kind: "plan_update".to_string(),
            text: None,
            phase: Some("tool_result".to_string()),
            detail: Some(serde_json::json!({ "entries": entries })),
        })
    }

    fn session_info_update_event_from_tool_message(
        message: &ChatMessage,
    ) -> Option<RuntimeSemanticEvent> {
        if message.name.as_deref()? != "set_conversation_title" {
            return None;
        }
        let content = message.content.as_deref()?;
        let value: serde_json::Value = serde_json::from_str(content).ok()?;
        let title = value
            .get("title")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())?
            .to_string();
        Some(RuntimeSemanticEvent::RunProgress {
            kind: "session_info_update".to_string(),
            text: None,
            phase: Some("tool_result".to_string()),
            detail: Some(serde_json::json!({ "title": title })),
        })
    }

    fn begin_server_tool_execution(&mut self, call: ChatToolCall) {
        let Some(invoker) = crate::native_runtime::tool_invoker() else {
            let tool_name = call.function.name.clone();
            self.pending_server_tool = Some(Box::pin(async move {
                Err(DenError::System(format!(
                    "builtin Den tool runtime is not initialized for {tool_name}"
                )))
            }));
            return;
        };
        let provider_name = call.function.name.clone();
        let canonical = builtin_den_tool_descriptor_for_provider_name(&provider_name)
            .map(|descriptor| descriptor.name.to_string())
            .unwrap_or_else(|| provider_name.clone());
        let args = serde_json::from_str(&call.function.arguments)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
        let context = self.server_tool_context();
        let pool = self.pool.clone();
        let config = self.config.clone();
        let stores = self.stores.clone();
        let store = self.store.clone();
        let session_key = self.session_key.clone();
        let profile = self.profile;
        let bear_id = self.bear_id;
        let user_id = self.user_id;
        let conversation_id = self.conversation_id.clone();
        let client_session_id = self.client_session_id.clone();
        self.pending_server_tool = Some(Box::pin(async move {
            let result = if canonical == DEN_TOOL_OUTPUT_READ {
                tool_output_read_result(&pool, bear_id, &client_session_id, args).await
            } else {
                invoker
                    .invoke(&pool, config.as_ref(), &stores, &canonical, args, context)
                    .await
            };
            let content = match result {
                Ok(value) => {
                    let compacted = compact_json_tool_result(value.clone());
                    if compacted.truncated {
                        match create_tool_output_artifact(
                            &pool,
                            ToolOutputArtifactInput {
                                bear_id,
                                user_id,
                                session_id: client_session_id.clone(),
                                conversation_id: Some(conversation_id.clone()),
                                run_id: None,
                                tool_call_id: call.id.clone(),
                                tool_name: Some(provider_name.clone()),
                                source: "den_hosted",
                                content_text: None,
                                content_json: Some(value.clone()),
                                metadata: serde_json::json!({ "canonical_tool": canonical }),
                            },
                        )
                        .await
                        {
                            Ok(artifact) => {
                                compact_json_tool_result_with_artifact(
                                    value,
                                    Some(&artifact.artifact_ref),
                                )
                                .content
                            }
                            Err(_) => compacted.content,
                        }
                    } else {
                        compacted.content
                    }
                }
                Err(error) => format!("error: {error}"),
            };
            let message = ChatMessage {
                role: "tool".to_string(),
                content: Some(content),
                tool_call_id: Some(call.id.clone()),
                name: Some(provider_name),
                tool_calls: None,
            };
            store.update(&session_key, |session| {
                session.messages.push(message.clone());
            });
            let session = store.get(&session_key).ok_or_else(|| {
                DenError::System("native agent loop session not found".to_string())
            })?;
            let llm = LlmClient::new(config.as_ref());
            let overflow = AgentStepOverflowContext {
                pool: pool.clone(),
                config: config.clone(),
                profile,
                session_store: store.clone(),
            };
            let stream = run_agent_step_stream(&llm, &session, Some(overflow)).await?;
            Ok((call, message, stream))
        }));
    }

    fn persist_assistant_tool_step(&self) {
        let calls = self.accumulated_tool_calls();
        if self.dispatch_mode != NativeToolDispatchMode::ServerSideInProcess {
            spawn_persist_native_agent_step(
                self.pool.clone(),
                self.bear_id,
                self.user_id,
                self.conversation_id.clone(),
                self.client_session_id.clone(),
                self.request_id.clone(),
                self.assistant_text.clone(),
                &calls,
            );
        }
        if !self.tool_calls.is_empty() {
            self.store.update(&self.session_key, |session| {
                session.step += 1;
            });
            return;
        }
        if self.assistant_text.trim().is_empty() {
            return;
        }
        self.store.update(&self.session_key, |session| {
            session.messages.push(crate::llm::ChatMessage {
                role: "assistant".to_string(),
                content: self.assistant_content(),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            });
            session.step += 1;
        });
    }

    fn begin_final_gate_continuation(&mut self, next_task: &str) {
        let model_message = format!(
            "You are in autonomous implementation mode. The active task list still has incomplete, unblocked work. Do not final-answer yet. Continue with: {next_task}."
        );
        self.store.update(&self.session_key, |session| {
            session.messages.push(ChatMessage {
                role: "system".to_string(),
                content: Some(model_message),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            });
        });

        let store = self.store.clone();
        let session_key = self.session_key.clone();
        let config = self.config.clone();
        let pool = self.pool.clone();
        let profile = self.profile;
        self.pending_final_gate_continuation = Some(Box::pin(async move {
            let session = store.get(&session_key).ok_or_else(|| {
                DenError::System("native agent loop session not found".to_string())
            })?;
            let llm = LlmClient::new(config.as_ref());
            let overflow = AgentStepOverflowContext {
                pool,
                config,
                profile,
                session_store: store,
            };
            run_agent_step_stream(&llm, &session, Some(overflow)).await
        }));
    }
}

fn upsert_assistant_tool_step_in_messages(
    messages: &mut Vec<crate::llm::ChatMessage>,
    content: Option<String>,
    calls: &[ChatToolCall],
    already_synced: bool,
) {
    let assistant = crate::llm::ChatMessage {
        role: "assistant".to_string(),
        content,
        tool_call_id: None,
        name: None,
        tool_calls: Some(calls.to_vec()),
    };
    if already_synced {
        if let Some(last) = messages.last_mut() {
            if last.role == "assistant" && last.tool_calls.is_some() {
                *last = assistant;
                return;
            }
        }
    }
    messages.push(assistant);
}

impl Stream for SessionTrackingStream {
    type Item = Result<RuntimeStreamEvent, DenError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }

        if let Some(pause) = self.pending_pause_after_tool.take() {
            if matches!(
                pause,
                RuntimeSemanticEvent::RunPaused { .. }
                    | RuntimeSemanticEvent::TurnCompleted { .. }
                    | RuntimeSemanticEvent::TurnFailed { .. }
                    | RuntimeSemanticEvent::TurnCancelled { .. }
            ) {
                self.finished = true;
            }
            return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(pause))));
        }

        if let Some(fut) = self.pending_server_tool.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok((call, message, stream))) => {
                    self.pending_server_tool = None;
                    self.tool_calls.remove(&call.id);
                    self.inner = stream;
                    self.pending_pause_after_tool =
                        Self::plan_update_event_from_tool_message(&message).or_else(|| {
                            Self::session_info_update_event_from_tool_message(&message)
                        });
                    self.pending_server_tool_continuation = Some(call.id.clone());
                    let finished =
                        tool_call_finished_event_for_content(&call, message.content.as_deref());
                    return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(finished))));
                }
                Poll::Ready(Err(error)) => {
                    self.pending_server_tool = None;
                    return Poll::Ready(Some(Err(error)));
                }
                Poll::Pending => return Poll::Pending,
            }
        }

        if let Some(fut) = self.pending_final_gate_continuation.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok(stream)) => {
                    self.pending_final_gate_continuation = None;
                    self.inner = stream;
                    self.assistant_text.clear();
                    self.tool_calls.clear();
                    self.assistant_synced_to_session = false;
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(Err(error)) => {
                    self.pending_final_gate_continuation = None;
                    return Poll::Ready(Some(Err(error)));
                }
                Poll::Pending => return Poll::Pending,
            }
        }

        if let Some(fut) = self.pending_approval.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok(Some(pause))) => {
                    self.pending_approval = None;
                    self.persist_assistant_tool_step();
                    if let RuntimeSemanticEvent::RunPaused {
                        resume_token: Some(approval_id),
                        ..
                    } = &pause
                    {
                        if let Some(RuntimeStreamEvent::Semantic(
                            RuntimeSemanticEvent::ToolCallRequested {
                                approval_request_id,
                                ..
                            },
                        )) = self.pending_tool_event.as_mut()
                        {
                            *approval_request_id = Some(approval_id.clone());
                        }
                    }
                    if let Some(event) = self.pending_tool_event.take() {
                        self.pending_pause_after_tool = Some(pause);
                        return Poll::Ready(Some(Ok(event)));
                    }
                    self.finished = true;
                    return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(pause))));
                }
                Poll::Ready(Ok(None)) => {
                    self.pending_approval = None;
                    if let Some(event) = self.pending_tool_event.take() {
                        return Poll::Ready(Some(Ok(event)));
                    }
                }
                Poll::Ready(Err(error)) => {
                    self.pending_approval = None;
                    self.pending_tool_event = None;
                    return Poll::Ready(Some(Err(error)));
                }
                Poll::Pending => return Poll::Pending,
            }
        }

        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::AssistantTextDelta { text },
            )))) => {
                self.pending_server_tool_continuation = None;
                self.assistant_text.push_str(&text);
                if self.assistant_synced_to_session && !self.tool_calls.is_empty() {
                    self.sync_assistant_tool_step_to_session();
                }
                Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                    RuntimeSemanticEvent::AssistantTextDelta { text },
                ))))
            }
            Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::ToolCallRequested {
                    tool_call_id,
                    tool_name,
                    arguments,
                    ..
                },
            )))) => {
                self.pending_server_tool_continuation = None;
                self.tool_calls.insert(
                    tool_call_id.clone(),
                    (tool_name.clone(), arguments.to_string()),
                );
                self.sync_assistant_tool_step_to_session();
                let approval_required = provider_tool_requires_approval(&tool_name);
                let event = RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    title: None,
                    kind: Some("function".to_string()),
                    arguments: arguments.clone(),
                    approval_request_id: None,
                    approval_required,
                    approval_reason: if approval_required {
                        Some("native runtime policy".to_string())
                    } else {
                        None
                    },
                    run_id: None,
                });
                if self.should_request_den_tool_permission(&tool_name) {
                    let pool = self.pool.clone();
                    let bear_id = self.bear_id;
                    let conversation_id = self.conversation_id.clone();
                    let client_session_id = self.client_session_id.clone();
                    let permission_target = Self::web_fetch_permission_target(&arguments);
                    let arguments_value = permission_target.clone();
                    let approval_tool_call_id = tool_call_id.clone();
                    let approval_tool_name = tool_name.clone();
                    self.pending_tool_event = Some(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::ToolCallRequested {
                            tool_call_id: tool_call_id.clone(),
                            tool_name: tool_name.clone(),
                            title: None,
                            kind: Some("function".to_string()),
                            arguments: permission_target,
                            approval_request_id: None,
                            approval_required: true,
                            approval_reason: Some(
                                "web_fetch requires approval for this URL".to_string(),
                            ),
                            run_id: None,
                        },
                    ));
                    self.pending_approval = Some(Box::pin(async move {
                        let approval_id = create_native_approval(
                            &pool,
                            bear_id,
                            &conversation_id,
                            &client_session_id,
                            &approval_tool_call_id,
                            &approval_tool_name,
                            &arguments_value,
                        )
                        .await?;
                        Ok(Some(RuntimeSemanticEvent::RunPaused {
                            reason: "requires_approval".to_string(),
                            resume_token: Some(approval_id),
                            expires_at: None,
                        }))
                    }));
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                if self.should_execute_den_tool_server_side(&tool_name) {
                    self.persist_assistant_tool_step();
                    let call = ChatToolCall {
                        id: tool_call_id.clone(),
                        call_type: "function".to_string(),
                        function: crate::llm::ChatToolCallFunction {
                            name: tool_name.clone(),
                            arguments: arguments.to_string(),
                        },
                    };
                    self.begin_server_tool_execution(call);
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                if approval_required && self.dispatch_mode == NativeToolDispatchMode::DeferToClient
                {
                    let pool = self.pool.clone();
                    let bear_id = self.bear_id;
                    let conversation_id = self.conversation_id.clone();
                    let client_session_id = self.client_session_id.clone();
                    let arguments_value = arguments;
                    self.pending_tool_event = Some(event);
                    self.pending_approval = Some(Box::pin(async move {
                        maybe_pause_for_tool_approval(
                            &pool,
                            bear_id,
                            &conversation_id,
                            &client_session_id,
                            &tool_call_id,
                            &tool_name,
                            &arguments_value,
                        )
                        .await
                    }));
                    // We just installed a new future after polling the inner stream. If we
                    // return Pending without waking, the approval future has not been polled
                    // yet and no waker is registered for it; the tool request can remain
                    // parked until an unrelated upstream wake. Schedule an immediate re-poll
                    // so the future can start and register its own wake source.
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::TurnCompleted { .. },
            )))) => {
                self.pending_server_tool_continuation = None;
                if !self.tool_calls.is_empty() {
                    // Tool-call finishes must not emit TurnCompleted: client stream parks for adapter-local
                    // tool results and continues via /tool-results (same class of bug as
                    // openai_stream synthetic TurnCompleted).
                    self.persist_assistant_tool_step();
                    self.persist_outstanding_tools_as_incomplete("turn_ended_before_tool_results");
                    self.finished = true;
                    tracing::debug!(
                        client_session_id = %self.client_session_id,
                        tool_call_count = self.tool_calls.len(),
                        "native runtime suppressing TurnCompleted while tool calls are outstanding"
                    );
                    return Poll::Ready(None);
                }
                if self.assistant_text.trim().is_empty() {
                    let fallback = "BEARS completed the turn without assistant output.".to_string();
                    self.assistant_text = fallback.clone();
                    self.persist_assistant_tool_step();
                    self.pending_pause_after_tool =
                        Some(RuntimeSemanticEvent::TurnCompleted { turn: None });
                    return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::AssistantTextDelta { text: fallback },
                    ))));
                } else {
                    self.persist_assistant_tool_step();
                }
                let active_activity_plan = self
                    .store
                    .get(&self.session_key)
                    .and_then(|session| session.active_activity_plan);
                if !should_allow_terminal_response_for_task_list(
                    self.profile,
                    active_activity_plan.as_ref(),
                    &self.assistant_text,
                ) {
                    let recent_texts = self
                        .store
                        .get(&self.session_key)
                        .map(|session| {
                            session
                                .messages
                                .iter()
                                .rev()
                                .filter_map(|message| message.content.as_deref())
                                .take(6)
                                .map(str::to_string)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let loop_detection = detect_task_focus_loop(&recent_texts);
                    if loop_detection.detected {
                        self.finished = true;
                        tracing::warn!(
                            client_session_id = %self.client_session_id,
                            profile = %self.profile.as_str(),
                            terminal_objections = loop_detection.terminal_objections,
                            continuation_nudges = loop_detection.continuation_nudges,
                            repeated_objection_kind = ?loop_detection.repeated_objection_kind,
                            "native runtime task-focus loop detected; accepting terminal objection instead of issuing another continuation nudge"
                        );
                        return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                            RuntimeSemanticEvent::TurnCompleted { turn: None },
                        ))));
                    }
                    let next_task = active_activity_plan
                        .as_ref()
                        .and_then(|plan| {
                            autonomous_execution_gate_for_task_list(
                                self.profile,
                                Some(&plan),
                                crate::runtime::turn_state::classify_autonomous_final_response(
                                    &self.assistant_text,
                                ),
                            )
                            .next_incomplete_task_title
                        })
                        .unwrap_or_else(|| "the next incomplete task".to_string());
                    tracing::info!(
                        client_session_id = %self.client_session_id,
                        profile = %self.profile.as_str(),
                        next_task = %next_task,
                        "native runtime converted premature terminal response into continuation nudge"
                    );
                    self.begin_final_gate_continuation(&next_task);
                    self.pending_pause_after_tool = Some(RuntimeSemanticEvent::RunProgress {
                        kind: "autonomous_continuation_gate".to_string(),
                        text: Some(format!(
                            "Active task-list work remains; continuing with {next_task} instead of stopping on a progress-only summary."
                        )),
                        phase: Some("continuation".to_string()),
                        detail: Some(serde_json::json!({
                            "next_task": next_task,
                            "profile": self.profile.as_str(),
                            "terminal_response_suppressed": true,
                        })),
                    });
                    return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::StatusText {
                            text: format!(
                                "Warned the bear that task focus is still active; continuing with {next_task}."
                            ),
                        },
                    ))));
                }
                let pool = self.pool.clone();
                let config = self.config.clone();
                let bear_id = self.bear_id;
                let conversation_id = self.conversation_id.clone();
                let profile = self.profile;
                tokio::spawn(async move {
                    enqueue_compaction_after_turn(
                        &pool,
                        &config,
                        bear_id,
                        &conversation_id,
                        profile,
                    )
                    .await;
                });
                self.finished = true;
                Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                    RuntimeSemanticEvent::TurnCompleted { turn: None },
                ))))
            }
            Poll::Ready(Some(Err(error))) => {
                if let Some(tool_call_id) = self.pending_server_tool_continuation.take() {
                    tracing::warn!(
                        client_session_id = %self.client_session_id,
                        tool_call_id = %tool_call_id,
                        error = %error,
                        "native runtime server-tool continuation failed; removing recent tool chain from in-memory session"
                    );
                    self.remove_recent_server_tool_chain_from_session(&tool_call_id);
                }
                self.finished = true;
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(other) => {
                if other.is_none() && !self.tool_calls.is_empty() {
                    self.persist_assistant_tool_step();
                    self.persist_outstanding_tools_as_incomplete(
                        "llm_stream_ended_before_tool_results",
                    );
                    self.finished = true;
                    tracing::debug!(
                        client_session_id = %self.client_session_id,
                        tool_call_count = self.tool_calls.len(),
                        "native runtime ended LLM stream with outstanding tool calls; deferring TurnCompleted"
                    );
                    return Poll::Ready(None);
                }
                let failed_or_empty_continuation = matches!(
                    &other,
                    Some(Ok(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::TurnFailed { .. }
                            | RuntimeSemanticEvent::TurnCancelled { .. }
                            | RuntimeSemanticEvent::Error { .. }
                    ))) | None
                );
                if failed_or_empty_continuation {
                    if let Some(tool_call_id) = self.pending_server_tool_continuation.take() {
                        tracing::warn!(
                            client_session_id = %self.client_session_id,
                            tool_call_id = %tool_call_id,
                            "native runtime server-tool continuation ended unsuccessfully; removing recent tool chain from in-memory session"
                        );
                        self.remove_recent_server_tool_chain_from_session(&tool_call_id);
                    }
                    self.finished = true;
                } else if other.is_some() {
                    self.pending_server_tool_continuation = None;
                }
                Poll::Ready(other)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        task::{RawWaker, RawWakerVTable, Waker},
    };

    use crate::{
        agent_loop::{
            NativeToolDispatchMode, PostMutationVerificationWindow, StrategyProfile,
            ToolCallBudgetLimits, TurnBudgetPolicy,
        },
        llm::{ChatMessage, ChatToolCall, ChatToolCallFunction},
    };
    use den_core::config::Config;
    use den_memory::MemoryStoreManager;
    use den_protocol::RuntimeStreamEvent;
    use den_service::bears::BearProfile;
    use futures::StreamExt;

    fn counting_waker(counter: Arc<AtomicUsize>) -> Waker {
        unsafe fn clone(data: *const ()) -> RawWaker {
            let arc = Arc::<AtomicUsize>::from_raw(data.cast());
            let cloned = arc.clone();
            std::mem::forget(arc);
            RawWaker::new(Arc::into_raw(cloned).cast(), &VTABLE)
        }
        unsafe fn wake(data: *const ()) {
            let arc = Arc::<AtomicUsize>::from_raw(data.cast());
            arc.fetch_add(1, Ordering::SeqCst);
        }
        unsafe fn wake_by_ref(data: *const ()) {
            let arc = Arc::<AtomicUsize>::from_raw(data.cast());
            arc.fetch_add(1, Ordering::SeqCst);
            std::mem::forget(arc);
        }
        unsafe fn drop(data: *const ()) {
            std::mem::drop(Arc::<AtomicUsize>::from_raw(data.cast()));
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop);
        let raw = RawWaker::new(Arc::into_raw(counter).cast(), &VTABLE);
        unsafe { Waker::from_raw(raw) }
    }

    fn test_session(session_key: &str, bear_id: uuid::Uuid) -> AgentLoopSession {
        AgentLoopSession {
            session_key: session_key.to_string(),
            bear_id,
            bear_slug: "test-bear".to_string(),
            user_id: Some(7),
            conversation_id: "den-conv-test".to_string(),
            client_session_id: "client-test".to_string(),
            workspace_roots: vec!["/workspace".to_string()],
            request_id: Some("request-test".to_string()),
            run_id: Some("run-test".to_string()),
            messages: Vec::new(),
            tools: Vec::new(),
            budget_components: Default::default(),
            model: "openai/test".to_string(),
            model_context_window: None,
            model_max_output_tokens: None,
            bifrost_virtual_key: None,
            api_style: None,
            step: 0,
            turn_budget: TurnBudgetPolicy {
                max_wall_clock_ms: 60_000,
                emergency_hard_steps: 16,
                tool_call_limits: ToolCallBudgetLimits {
                    total: 8,
                    read: 6,
                    search: 4,
                    fetch: 2,
                    execute: 2,
                    write: 2,
                    destructive: 1,
                    other: 2,
                },
                max_consecutive_tool_failures: 2,
                max_same_tool_signature_repeats: 1,
                post_mutation_verification_window: Some(PostMutationVerificationWindow {
                    replenish_read: 2,
                    replenish_search: 1,
                }),
            },
            turn_budget_state: Default::default(),
            strategy: StrategyProfile::plain_react(),
            stream_tokens: true,
            key_memory_projection_cache_key: None,
            latest_context_budget: None,
            latest_projected_memory: None,
            latest_recalled_memory: None,
            active_activity_plan: None,
            profile: BearProfile::Pair,
            overflow_retry_attempted: false,
            overflow_compaction_recovered: false,
        }
    }

    fn sample_tool_call(id: &str) -> ChatToolCall {
        ChatToolCall {
            id: id.to_string(),
            call_type: "function".to_string(),
            function: ChatToolCallFunction {
                name: "memory_read".to_string(),
                arguments: "{}".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn empty_turn_completion_emits_fallback_assistant_output_first() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::new();
        store.insert(session.clone());
        let mut stream = SessionTrackingStream::new(
            Box::pin(futures::stream::iter(vec![Ok(
                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { turn: None }),
            )])),
            &session,
            store,
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop")
                .expect("lazy test pool"),
            bear_id,
            "test-bear".to_string(),
            Some(7),
            "den-conv-test".to_string(),
            "client-test".to_string(),
            Some("request-test".to_string()),
            Arc::new(den_core::config::Config::test_stub()),
            MemoryStoreManager::new(&den_core::config::Config::test_stub()),
            BearProfile::Pair,
            NativeToolDispatchMode::DeferToClient,
        );

        let first = stream.next().await.expect("fallback event").expect("ok");
        assert!(matches!(
            first,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { ref text })
                if text.contains("completed the turn without assistant output")
        ));
        let second = stream.next().await.expect("completion event").expect("ok");
        assert!(matches!(
            second,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. })
        ));
        assert!(stream.next().await.is_none());
    }

    #[test]
    fn upsert_assistant_tool_step_pushes_then_merges_tool_calls() {
        let mut messages = Vec::new();
        upsert_assistant_tool_step_in_messages(
            &mut messages,
            None,
            &[sample_tool_call("call_1")],
            false,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tool_calls.as_ref().map(|c| c.len()), Some(1));

        upsert_assistant_tool_step_in_messages(
            &mut messages,
            Some("checking".to_string()),
            &[sample_tool_call("call_1"), sample_tool_call("call_2")],
            true,
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_deref(), Some("checking"));
        assert_eq!(messages[0].tool_calls.as_ref().map(|c| c.len()), Some(2));
    }

    #[test]
    fn session_info_update_event_is_derived_from_set_conversation_title_tool_result() {
        let message = ChatMessage {
            role: "tool".to_string(),
            content: Some(
                serde_json::json!({
                    "ok": true,
                    "title": "Useful title"
                })
                .to_string(),
            ),
            tool_call_id: Some("call-1".to_string()),
            name: Some("set_conversation_title".to_string()),
            tool_calls: None,
        };

        let event = SessionTrackingStream::session_info_update_event_from_tool_message(&message)
            .expect("session info update event");

        assert!(matches!(
            event,
            RuntimeSemanticEvent::RunProgress { kind, detail: Some(detail), .. }
                if kind == "session_info_update"
                    && detail["title"].as_str() == Some("Useful title")
        ));
    }

    #[tokio::test]
    async fn server_tool_context_inherits_workspace_roots_from_session() {
        let bear_id = uuid::Uuid::new_v4();
        let session_key = "den-conv-test:client-test";
        let mut session = test_session(session_key, bear_id);
        session.workspace_roots = vec!["/workspace/project".to_string()];
        let store = AgentLoopSessionStore::new();
        store.insert(session.clone());
        let stream = SessionTrackingStream::new(
            Box::pin(futures::stream::empty()),
            &session,
            store,
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/unused")
                .expect("lazy pool"),
            bear_id,
            session.bear_slug.clone(),
            session.user_id,
            session.conversation_id.clone(),
            session.client_session_id.clone(),
            session.request_id.clone(),
            Arc::new(Config::test_stub()),
            MemoryStoreManager::new(&Config::test_stub()),
            BearProfile::Pair,
            NativeToolDispatchMode::DeferToClient,
        );

        let context = stream.server_tool_context();
        assert_eq!(
            context.workspace_roots,
            vec!["/workspace/project".to_string()]
        );
    }

    #[tokio::test]
    async fn server_tool_continuation_cleanup_removes_recent_tool_chain() {
        let bear_id = uuid::Uuid::new_v4();
        let mut session = test_session("den-conv-test:client-test", bear_id);
        session.messages.push(ChatMessage {
            role: "user".to_string(),
            content: Some("continue".to_string()),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        });
        session.messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_call_id: None,
            name: None,
            tool_calls: Some(vec![sample_tool_call("call_1")]),
        });
        session.messages.push(ChatMessage {
            role: "tool".to_string(),
            content: Some("{}".to_string()),
            tool_call_id: Some("call_1".to_string()),
            name: Some("memory_read".to_string()),
            tool_calls: None,
        });
        let store = AgentLoopSessionStore::new();
        store.insert(session.clone());
        let stream = SessionTrackingStream::new(
            Box::pin(futures::stream::empty()),
            &session,
            store.clone(),
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop")
                .expect("lazy test pool"),
            bear_id,
            "test-bear".to_string(),
            Some(7),
            "den-conv-test".to_string(),
            "client-test".to_string(),
            Some("request-test".to_string()),
            Arc::new(den_core::config::Config::test_stub()),
            MemoryStoreManager::new(&den_core::config::Config::test_stub()),
            BearProfile::Pair,
            NativeToolDispatchMode::DeferToClient,
        );

        stream.remove_recent_server_tool_chain_from_session("call_1");

        let repaired = store.get(&session.session_key).expect("session");
        assert_eq!(repaired.messages.len(), 1);
        assert_eq!(repaired.messages[0].role, "user");
    }

    #[tokio::test]
    async fn den_tools_route_server_side_but_client_tools_do_not() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::new();
        store.insert(session.clone());
        let stream = SessionTrackingStream::new(
            Box::pin(futures::stream::empty()),
            &session,
            store,
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop")
                .expect("lazy test pool"),
            bear_id,
            "test-bear".to_string(),
            Some(7),
            "den-conv-test".to_string(),
            "client-test".to_string(),
            Some("request-test".to_string()),
            Arc::new(den_core::config::Config::test_stub()),
            MemoryStoreManager::new(&den_core::config::Config::test_stub()),
            BearProfile::Pair,
            NativeToolDispatchMode::DeferToClient,
        );

        assert!(stream.should_execute_den_tool_server_side("list_task_lists"));
        assert!(stream.should_execute_den_tool_server_side("list_plans"));
        assert!(stream.should_execute_den_tool_server_side("session_info"));
        assert!(stream.should_request_den_tool_permission("web_fetch"));
        assert!(!stream.should_execute_den_tool_server_side("web_fetch"));
        assert!(!stream.should_execute_den_tool_server_side("fs_read_text_file"));
        assert!(!stream.should_execute_den_tool_server_side("mcp__chrome_devtools_custom__click"));
    }

    #[tokio::test]
    async fn approval_required_tool_call_wakes_after_installing_pending_future() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::new();
        store.insert(session.clone());
        let inner = futures::stream::iter(vec![Ok(RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::ToolCallRequested {
                tool_call_id: "call-read".to_string(),
                tool_name: "fs_read_text_file".to_string(),
                title: None,
                kind: Some("function".to_string()),
                arguments: serde_json::json!({"path":"README.md"}),
                approval_request_id: None,
                approval_required: false,
                approval_reason: None,
                run_id: None,
            },
        ))]);
        let mut stream = SessionTrackingStream::new(
            Box::pin(inner),
            &session,
            store,
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop")
                .expect("lazy test pool"),
            bear_id,
            "test-bear".to_string(),
            Some(7),
            "den-conv-test".to_string(),
            "client-test".to_string(),
            Some("request-test".to_string()),
            Arc::new(den_core::config::Config::test_stub()),
            MemoryStoreManager::new(&den_core::config::Config::test_stub()),
            BearProfile::Pair,
            NativeToolDispatchMode::DeferToClient,
        );
        let wake_count = Arc::new(AtomicUsize::new(0));
        let waker = counting_waker(wake_count.clone());
        let mut cx = Context::from_waker(&waker);

        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Pending
        ));
        assert_eq!(wake_count.load(Ordering::SeqCst), 1);
        assert!(stream.pending_approval.is_some());
        assert!(stream.pending_tool_event.is_some());
    }

    #[tokio::test]
    async fn run_paused_event_finishes_current_runtime_stream_segment() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::new();
        store.insert(session.clone());
        let mut stream = SessionTrackingStream::new(
            Box::pin(futures::stream::iter(vec![Ok(
                RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta {
                    text: "should-not-leak-after-pause".to_string(),
                }),
            )])),
            &session,
            store,
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop")
                .expect("lazy test pool"),
            bear_id,
            "test-bear".to_string(),
            Some(7),
            "den-conv-test".to_string(),
            "client-test".to_string(),
            Some("request-test".to_string()),
            Arc::new(den_core::config::Config::test_stub()),
            MemoryStoreManager::new(&den_core::config::Config::test_stub()),
            BearProfile::Pair,
            NativeToolDispatchMode::DeferToClient,
        );
        stream.pending_pause_after_tool = Some(RuntimeSemanticEvent::RunPaused {
            reason: "requires_approval".to_string(),
            resume_token: Some("perm-test".to_string()),
            expires_at: None,
        });

        let wake_count = Arc::new(AtomicUsize::new(0));
        let waker = counting_waker(wake_count);
        let mut cx = Context::from_waker(&waker);

        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                RuntimeSemanticEvent::RunPaused { .. }
            ))))
        ));
        assert!(matches!(
            Pin::new(&mut stream).poll_next(&mut cx),
            Poll::Ready(None)
        ));
    }

    #[test]
    fn assistant_tool_step_sync_precedes_tool_result_in_message_chain() {
        let mut messages = vec![ChatMessage {
            role: "user".to_string(),
            content: Some("hi".to_string()),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }];
        upsert_assistant_tool_step_in_messages(
            &mut messages,
            None,
            &[sample_tool_call("call_1")],
            false,
        );
        messages.push(ChatMessage {
            role: "tool".to_string(),
            content: Some("ok".to_string()),
            tool_call_id: Some("call_1".to_string()),
            name: None,
            tool_calls: None,
        });

        let repaired = crate::agent_loop::context::repair_tool_call_message_chain(messages);
        assert_eq!(repaired.len(), 3);
        assert_eq!(repaired[1].role, "assistant");
        assert_eq!(repaired[2].role, "tool");
    }
}
