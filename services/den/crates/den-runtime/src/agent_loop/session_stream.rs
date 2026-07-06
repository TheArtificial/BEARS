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
        record_checkpoint_request, record_checkpoint_response,
        run_agent_step_stream,
        session_store::AgentLoopSessionStore,
        tool_call_finished_event_for_content,
        tool_policy::{
            maybe_pause_for_tool_approval, provider_tool_requires_approval,
            provider_tool_supports_unilateral_execution,
        },
        task_gate_checkpoint_trigger, validate_checkpoint_response, AgentStepOverflowContext,
        CheckpointArtifactInput, CheckpointField, CheckpointReplayPolicy, CheckpointResponseInput,
        CheckpointTaskContext, CheckpointTrigger, CheckpointValidationStatus, CheckpointVisibility,
        RuntimeCheckpointRequest, RuntimeCheckpointResponse,
    },
    llm::{ChatMessage, ChatToolCall, LlmClient},
    runtime_compaction::enqueue_compaction_after_turn,
    tool_output_artifacts::{create_tool_output_artifact, ToolOutputArtifactInput},
};
use den_core::tools::{
    arguments::DenToolChannelContext,
    constants::{
        DEN_TASK_LISTS_REQUEST_HANDOFF_LEGACY_PROVIDER, DEN_TASK_LISTS_REQUEST_HANDOFF_PROVIDER,
        DEN_TASK_LISTS_UPDATE_LEGACY_PROVIDER, DEN_TASK_LISTS_UPDATE_PROVIDER,
        DEN_TASK_LIST_SYNC_PROVIDER, DEN_TASK_UPDATE_CURRENT_STATUS_PROVIDER, DEN_TOOL_OUTPUT_READ,
        DEN_WEB_FETCH,
    },
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
            .or_else(|| value.get("task_list").and_then(|plan| plan.get("items")))
            .or_else(|| {
                value
                    .get("sync")
                    .and_then(|sync| sync.get("task_list"))
                    .and_then(|plan| plan.get("items"))
            })
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

    fn observe_checkpoint_triggers(&self) -> bool {
        matches!(
            self.config.agent_loop_control_mode.as_str(),
            "observe" | "enforce"
        )
    }

    fn enforce_checkpoint_responses(&self) -> bool {
        self.config.agent_loop_control_mode == "enforce"
    }

    fn pending_checkpoint_request(&self) -> Option<RuntimeCheckpointRequest> {
        self.store
            .get(&self.session_key)
            .and_then(|session| session.pending_checkpoint_request)
    }

    fn checkpoint_audit_enabled(&self) -> bool {
        match self.config.checkpoint_audit_mode.as_str() {
            "all" => true,
            "work" => self.profile == BearProfile::Work,
            _ => false,
        }
    }

    fn task_gate_checkpoint_request(
        &self,
        trigger: &CheckpointTrigger,
        next_task: &str,
    ) -> Option<RuntimeCheckpointRequest> {
        let session = self.store.get(&self.session_key)?;
        let run_id = session.run_id.clone()?;
        let task_context = session.active_activity_plan.as_ref().map(|plan| {
            let active_item = plan.current_item.as_ref().or_else(|| {
                plan.items.iter().find(|item| {
                    matches!(
                        item.status,
                        den_docket::TaskListItemStatus::Pending
                            | den_docket::TaskListItemStatus::InProgress
                    )
                })
            });
            CheckpointTaskContext {
                task_list_id: Some(plan.id.to_string()),
                task_list_version: None,
                active_item_id: active_item.map(|item| item.id.to_string()),
                active_item_title: active_item.map(|item| item.title.clone()),
                docket_job_id: plan.source_ref.docket_job_id.clone(),
                docket_task_id: active_item.and_then(|item| item.source_ref.docket_task_id.clone()),
            }
        });
        Some(RuntimeCheckpointRequest {
            checkpoint_id: format!(
                "ckpt-{}-{}",
                session.step.saturating_add(1),
                trigger.reason.as_str()
            ),
            run_id,
            reason: trigger.reason,
            control_level: session.agent_loop_control.level,
            active_objective: Some(next_task.to_string()),
            task_context,
            evidence_refs: Vec::new(),
            required_fields: vec![
                CheckpointField::ActiveObjective,
                CheckpointField::Learned,
                CheckpointField::NextAction,
                CheckpointField::TaskStateChangeNeeded,
            ],
        })
    }

    fn checkpoint_nudge_text(request: &RuntimeCheckpointRequest, trigger: &CheckpointTrigger) -> String {
        let request_json = serde_json::to_string_pretty(request)
            .unwrap_or_else(|_| "{\"error\":\"checkpoint_request_unavailable\"}".to_string());
        format!(
            "Runtime checkpoint required: {}\n\nReturn a structured checkpoint response before continuing. If the active task is done, blocked, not applicable, waived, cancelled, unsafe, or permission-gated, the checkpoint response may say so, but task state must still be updated through task tools with evidence.\n\nCheckpoint request:\n```json\n{request_json}\n```",
            trigger.message
        )
    }

    fn apply_checkpoint_nudge(
        &self,
        request: RuntimeCheckpointRequest,
        trigger: &CheckpointTrigger,
    ) {
        let message = Self::checkpoint_nudge_text(&request, trigger);
        self.store.update(&self.session_key, |session| {
            if session.messages.last().is_some_and(|message| {
                message.role == "system"
                    && message
                        .content
                        .as_deref()
                        .is_some_and(|content| content.starts_with("Runtime checkpoint required:"))
            }) {
                session.messages.pop();
            }
            session.pending_checkpoint_request = Some(request.clone());
            session.messages.push(ChatMessage {
                role: "system".to_string(),
                content: Some(message.clone()),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            });
        });
    }

    fn record_checkpoint_request_if_audited(&self, request: RuntimeCheckpointRequest) {
        if !self.checkpoint_audit_enabled() {
            return;
        }
        let pool = self.pool.clone();
        let session_id = self.client_session_id.clone();
        tokio::spawn(async move {
            if let Err(err) = record_checkpoint_request(
                &pool,
                CheckpointArtifactInput {
                    run_id: request.run_id.clone(),
                    turn_step_id: None,
                    request,
                    visibility: CheckpointVisibility::AuditOnly,
                    replay_policy: CheckpointReplayPolicy::None,
                },
            )
            .await
            {
                tracing::warn!(
                    error = %err,
                    session_id = %session_id,
                    "failed to record task-gate checkpoint request artifact"
                );
            }
        });
    }

    fn parse_checkpoint_response_text(text: &str) -> Result<RuntimeCheckpointResponse, DenError> {
        let trimmed = text.trim();
        let json_text = if let Some(rest) = trimmed.strip_prefix("```json") {
            rest.trim()
                .strip_suffix("```")
                .map(str::trim)
                .unwrap_or(rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix("```") {
            rest.trim()
                .strip_suffix("```")
                .map(str::trim)
                .unwrap_or(rest.trim())
        } else {
            trimmed
        };
        serde_json::from_str(json_text)
            .map_err(|err| DenError::Parsing(format!("invalid checkpoint response JSON: {err}")))
    }

    fn checkpoint_failure_event(message: String) -> RuntimeStreamEvent {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: den_protocol::RuntimeErrorCategory::BackendProtocol,
            message,
        })
    }

    fn required_task_action_satisfied(action: &crate::agent_loop::CheckpointNextAction, tool_name: &str) -> bool {
        match action {
            crate::agent_loop::CheckpointNextAction::UpdateTaskList => matches!(
                tool_name,
                DEN_TASK_LISTS_UPDATE_PROVIDER
                    | DEN_TASK_LISTS_UPDATE_LEGACY_PROVIDER
                    | DEN_TASK_UPDATE_CURRENT_STATUS_PROVIDER
            ),
            crate::agent_loop::CheckpointNextAction::SyncTaskList => {
                tool_name == DEN_TASK_LIST_SYNC_PROVIDER
            }
            crate::agent_loop::CheckpointNextAction::RequestHandoff => matches!(
                tool_name,
                DEN_TASK_LISTS_REQUEST_HANDOFF_PROVIDER | DEN_TASK_LISTS_REQUEST_HANDOFF_LEGACY_PROVIDER
            ),
            _ => false,
        }
    }

    fn checkpoint_next_action_can_satisfy_task_state_change(
        action: &crate::agent_loop::CheckpointNextAction,
    ) -> bool {
        matches!(
            action,
            crate::agent_loop::CheckpointNextAction::UpdateTaskList
                | crate::agent_loop::CheckpointNextAction::SyncTaskList
                | crate::agent_loop::CheckpointNextAction::RequestHandoff
        )
    }

    fn pending_checkpoint_task_action(&self) -> Option<crate::agent_loop::CheckpointNextAction> {
        self.store
            .get(&self.session_key)
            .and_then(|session| session.pending_checkpoint_task_action)
    }

    fn enforce_required_checkpoint_task_action(
        &self,
        tool_name: &str,
    ) -> Result<(), RuntimeStreamEvent> {
        let Some(action) = self.pending_checkpoint_task_action() else {
            return Ok(());
        };
        if Self::required_task_action_satisfied(&action, tool_name) {
            self.store.update(&self.session_key, |session| {
                session.pending_checkpoint_task_action = None;
            });
            return Ok(());
        }
        Err(Self::checkpoint_failure_event(format!(
            "Runtime checkpoint required task-state follow-through with `{action:?}`, but the assistant next called `{tool_name}`. Use the task-management tool indicated by the checkpoint response before other actions."
        )))
    }

    fn fail_if_checkpoint_task_action_pending(&self) -> Result<(), RuntimeStreamEvent> {
        if let Some(action) = self.pending_checkpoint_task_action() {
            return Err(Self::checkpoint_failure_event(format!(
                "Runtime checkpoint required task-state follow-through with `{action:?}`, but the assistant attempted to stop before using the required task-management tool."
            )));
        }
        Ok(())
    }

    fn validate_pending_checkpoint_response(&mut self) -> Result<bool, RuntimeStreamEvent> {
        if !self.enforce_checkpoint_responses() {
            return Ok(false);
        }
        let Some(request) = self.pending_checkpoint_request() else {
            return Ok(false);
        };
        let response = Self::parse_checkpoint_response_text(&self.assistant_text).map_err(|err| {
            Self::checkpoint_failure_event(format!(
                "Runtime checkpoint response was required before continuation, but the assistant did not return valid checkpoint JSON: {err}"
            ))
        })?;
        validate_checkpoint_response(&request, &response).map_err(|err| {
            Self::checkpoint_failure_event(format!(
                "Runtime checkpoint response failed validation: {err:?}"
            ))
        })?;
        let required_task_action = if response.task_state_change_needed.is_some() {
            if !Self::checkpoint_next_action_can_satisfy_task_state_change(&response.next_action) {
                return Err(Self::checkpoint_failure_event(
                    "Runtime checkpoint response declared task_state_change_needed, but next_action was not a task-management action. Use update_task_list, sync_task_list, or request_task_list_handoff.".to_string(),
                ));
            }
            Some(response.next_action.clone())
        } else {
            None
        };

        self.store.update(&self.session_key, |session| {
            session.pending_checkpoint_request = None;
            session.pending_checkpoint_task_action = required_task_action.clone();
            session.checkpoint_state.last_checkpoint_reason = None;
        });
        self.record_checkpoint_response_if_audited(request, response);
        self.assistant_text.clear();
        Ok(true)
    }

    fn record_checkpoint_response_if_audited(
        &self,
        request: RuntimeCheckpointRequest,
        response: RuntimeCheckpointResponse,
    ) {
        if !self.checkpoint_audit_enabled() {
            return;
        }
        let pool = self.pool.clone();
        let session_id = self.client_session_id.clone();
        tokio::spawn(async move {
            if let Err(err) = record_checkpoint_response(
                &pool,
                CheckpointResponseInput {
                    run_id: request.run_id,
                    checkpoint_id: request.checkpoint_id,
                    response,
                    validation_status: CheckpointValidationStatus::Valid,
                },
            )
            .await
            {
                tracing::warn!(
                    error = %err,
                    session_id = %session_id,
                    "failed to record validated checkpoint response artifact"
                );
            }
        });
    }

    fn begin_checkpoint_continuation(&mut self) {
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
                if let Err(event) = self.validate_pending_checkpoint_response() {
                    self.finished = true;
                    return Poll::Ready(Some(Ok(event)));
                }
                if let Err(event) = self.enforce_required_checkpoint_task_action(&tool_name) {
                    self.finished = true;
                    return Poll::Ready(Some(Ok(event)));
                }
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
                            tool_call_id,
                            tool_name,
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
                        id: tool_call_id,
                        call_type: "function".to_string(),
                        function: crate::llm::ChatToolCallFunction {
                            name: tool_name,
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
                match self.validate_pending_checkpoint_response() {
                    Ok(true) => {
                        self.begin_checkpoint_continuation();
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    Ok(false) => {}
                    Err(event) => {
                        self.finished = true;
                        return Poll::Ready(Some(Ok(event)));
                    }
                }
                if let Err(event) = self.fail_if_checkpoint_task_action_pending() {
                    self.finished = true;
                    return Poll::Ready(Some(Ok(event)));
                }
                if self.assistant_text.trim().is_empty() {
                    let fallback = "BEARS completed the turn without assistant output.".to_string();
                    self.assistant_text.clone_from(&fallback);
                    self.persist_assistant_tool_step();
                    self.pending_pause_after_tool =
                        Some(RuntimeSemanticEvent::TurnCompleted { turn: None });
                    return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::AssistantTextDelta { text: fallback },
                    ))));
                }
                self.persist_assistant_tool_step();
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
                                Some(plan),
                                crate::runtime::turn_state::classify_autonomous_final_response(
                                    &self.assistant_text,
                                ),
                            )
                            .next_incomplete_task_title
                        })
                        .unwrap_or_else(|| "the next incomplete task".to_string());
                    if let Some(session) = self.store.get(&self.session_key) {
                        if let Some(trigger) = task_gate_checkpoint_trigger(&session.agent_loop_control.profile) {
                            if self.observe_checkpoint_triggers() {
                                if let Some(request) = self.task_gate_checkpoint_request(&trigger, &next_task) {
                                    self.record_checkpoint_request_if_audited(request.clone());
                                    if self.enforce_checkpoint_responses() {
                                        self.apply_checkpoint_nudge(request, &trigger);
                                        self.begin_checkpoint_continuation();
                                        self.pending_pause_after_tool = Some(RuntimeSemanticEvent::RunProgress {
                                            kind: "runtime_checkpoint_required".to_string(),
                                            text: Some(trigger.message.clone()),
                                            phase: Some("agent_loop_control".to_string()),
                                            detail: Some(serde_json::json!({
                                                "reason": trigger.reason,
                                                "mode": "enforce",
                                                "next_task": next_task.clone(),
                                            })),
                                        });
                                        return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                                            RuntimeSemanticEvent::StatusText {
                                                text: "Required a structured checkpoint before continuing the active task gate.".to_string(),
                                            },
                                        ))));
                                    }
                                    self.begin_final_gate_continuation(&next_task);
                                    self.pending_pause_after_tool = Some(RuntimeSemanticEvent::RunProgress {
                                        kind: "runtime_checkpoint_would_trigger".to_string(),
                                        text: Some(trigger.message.clone()),
                                        phase: Some("agent_loop_control".to_string()),
                                        detail: Some(serde_json::json!({
                                            "reason": trigger.reason,
                                            "mode": "observe_only",
                                            "next_task": next_task.clone(),
                                        })),
                                    });
                                    return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(
                                        RuntimeSemanticEvent::StatusText {
                                            text: format!(
                                                "Observed a task-gate checkpoint trigger; continuing with {next_task}."
                                            ),
                                        },
                                    ))));
                                }
                            }
                        }
                    }
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
            resolve_agent_loop_control, AgentLoopControlResolutionInput, NativeToolDispatchMode,
            PostMutationVerificationWindow, StrategyProfile, ToolCallBudgetLimits, TurnBudgetPolicy,
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
            agent_loop_control: resolve_agent_loop_control(AgentLoopControlResolutionInput {
                model_handle: Some("openai/test"),
                model_default: None,
                bear_override: None,
                stance_override: None,
                task_escalation: None,
            }),
            checkpoint_state: Default::default(),
            pending_checkpoint_request: None,
            pending_checkpoint_task_action: None,
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

    #[test]
    fn parses_plain_and_fenced_checkpoint_response_json() {
        let raw = serde_json::json!({
            "checkpoint_id": "ckpt-1",
            "active_objective": "Inspect routing",
            "learned": ["The projector owns the mapping."],
            "remaining_uncertainty": [],
            "more_exploration_justified": false,
            "next_action": "validate",
            "task_state_change_needed": null,
            "evidence_refs": [],
            "confidence": "medium"
        })
        .to_string();

        let parsed = SessionTrackingStream::parse_checkpoint_response_text(&raw)
            .expect("plain checkpoint response parses");
        assert_eq!(parsed.checkpoint_id, "ckpt-1");

        let fenced = format!("```json\n{raw}\n```");
        let parsed = SessionTrackingStream::parse_checkpoint_response_text(&fenced)
            .expect("fenced checkpoint response parses");
        assert_eq!(parsed.next_action, crate::agent_loop::CheckpointNextAction::Validate);
    }

    #[test]
    fn checkpoint_task_state_change_requires_task_management_next_action() {
        assert!(SessionTrackingStream::checkpoint_next_action_can_satisfy_task_state_change(
            &crate::agent_loop::CheckpointNextAction::UpdateTaskList
        ));
        assert!(SessionTrackingStream::checkpoint_next_action_can_satisfy_task_state_change(
            &crate::agent_loop::CheckpointNextAction::SyncTaskList
        ));
        assert!(SessionTrackingStream::checkpoint_next_action_can_satisfy_task_state_change(
            &crate::agent_loop::CheckpointNextAction::RequestHandoff
        ));
        assert!(!SessionTrackingStream::checkpoint_next_action_can_satisfy_task_state_change(
            &crate::agent_loop::CheckpointNextAction::CallTool { tool_name: None }
        ));
    }

    #[test]
    fn required_checkpoint_task_action_matches_only_task_management_tools() {
        assert!(SessionTrackingStream::required_task_action_satisfied(
            &crate::agent_loop::CheckpointNextAction::UpdateTaskList,
            DEN_TASK_LISTS_UPDATE_PROVIDER,
        ));
        assert!(SessionTrackingStream::required_task_action_satisfied(
            &crate::agent_loop::CheckpointNextAction::UpdateTaskList,
            DEN_TASK_UPDATE_CURRENT_STATUS_PROVIDER,
        ));
        assert!(SessionTrackingStream::required_task_action_satisfied(
            &crate::agent_loop::CheckpointNextAction::SyncTaskList,
            DEN_TASK_LIST_SYNC_PROVIDER,
        ));
        assert!(SessionTrackingStream::required_task_action_satisfied(
            &crate::agent_loop::CheckpointNextAction::RequestHandoff,
            DEN_TASK_LISTS_REQUEST_HANDOFF_PROVIDER,
        ));
        assert!(!SessionTrackingStream::required_task_action_satisfied(
            &crate::agent_loop::CheckpointNextAction::SyncTaskList,
            "memory_read",
        ));
    }

    #[test]
    fn rejects_non_json_checkpoint_response() {
        let err = SessionTrackingStream::parse_checkpoint_response_text("I will keep reading")
            .expect_err("non-json checkpoint response should fail");
        assert!(err.to_string().contains("invalid checkpoint response JSON"));
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
