use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use den_memory::MemoryStoreManager;
use den_protocol::{RuntimeEventStream, RuntimeSemanticEvent, RuntimeStreamEvent};
use den_service::bears::prompt_fragments::{
    render_turn_fragment, repository_prompt_fragment_registry,
};
use futures::Stream;

use crate::runtime::completion_policy::{
    decide_turn_completion, TurnCompletionCompleteReason, TurnCompletionDecision,
    TurnCompletionPolicyInput,
};
use crate::runtime::focus_context::{
    resolve_runtime_focus_context, RuntimeFocusContext, RuntimeFocusResolveRequest,
};
use crate::runtime_error_ux::{
    checkpoint_follow_through_required_policy, RuntimeIssueDisposition, RuntimeIssueSeverity,
};
use crate::{
    agent_loop::{
        approvals::create_native_approval,
        record_checkpoint_response, run_agent_step_stream,
        session_store::AgentLoopSessionStore,
        step::RUNTIME_CHECKPOINT_TOOL_NAME,
        tool_call_finished_event_for_content,
        tool_policy::{
            maybe_pause_for_tool_approval, provider_tool_is_den_web_fetch,
            provider_tool_requires_approval, provider_tool_supports_unilateral_execution,
        },
        validate_checkpoint_response, AgentStepOverflowContext, CheckpointResponseInput,
        CheckpointValidationStatus, RuntimeCheckpointRequest, RuntimeCheckpointResponse,
    },
    llm::{ChatMessage, ChatToolCall, LlmClient},
    native_runtime::is_task_definition_or_delegation_tool_provider_name,
    runtime_compaction::enqueue_compaction_after_turn,
    tool_output_artifacts::{create_tool_output_artifact, ToolOutputArtifactInput},
};
use den_core::tools::{
    arguments::DenToolChannelContext,
    constants::{
        DEN_TASK_CREATE_PROVIDER, DEN_TASK_LISTS_REQUEST_HANDOFF_PROVIDER,
        DEN_TASK_LISTS_UPDATE_PROVIDER, DEN_TASK_LIST_SYNC_PROVIDER,
        DEN_TASK_UPDATE_CURRENT_STATUS_PROVIDER, DEN_TASK_UPDATE_PROVIDER, DEN_TOOL_OUTPUT_READ,
    },
    context::DenToolInvocationContext,
    descriptor::builtin_den_tool_descriptor_for_provider_name,
    result_compaction::{compact_json_tool_result, compact_json_tool_result_with_artifact},
};
use den_core::{config::Config, governance::Governance, profile::BearProfile, DenError};
use den_docket::TaskListProjection;

use super::transcript::{
    spawn_persist_incomplete_acp_tool_results, spawn_persist_native_agent_step,
};
use super::{session_store::AgentLoopSession, ObjectiveOrientation, OrientationTaskRef};

const MAX_CHECKPOINT_RECOVERY_ATTEMPTS: u8 = 2;

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
type ServerToolContinuationFuture =
    Pin<Box<dyn Future<Output = Result<RuntimeEventStream, DenError>> + Send>>;
type ServerToolFuture = Pin<
    Box<
        dyn Future<
                Output = Result<
                    (ChatToolCall, ChatMessage, ServerToolContinuationFuture),
                    DenError,
                >,
            > + Send,
    >,
>;
type FinalGateContinuationFuture =
    Pin<Box<dyn Future<Output = Result<RuntimeEventStream, DenError>> + Send>>;
type FinalGateFocusFuture = Pin<
    Box<
        dyn Future<Output = Result<crate::runtime::focus_context::RuntimeFocusContext, DenError>>
            + Send,
    >,
>;

fn render_final_gate_continuation_guidance(next_task: &str) -> Result<String, DenError> {
    // Keep reusable final-gate steering in the fragment registry; this helper is
    // only the structured-state adapter for loop-control code.
    let fragments = repository_prompt_fragment_registry()?;
    let fragment = fragments.require("runtime_task_list_final_gate_continuation")?;
    render_turn_fragment(
        fragment,
        &serde_json::json!({
            "gate": {
                "next_task": next_task,
            }
        }),
    )
}

fn recent_tool_result_matches(messages: &[ChatMessage], tool_call_id: &str) -> bool {
    messages.last().is_some_and(|last| {
        last.role == "tool" && last.tool_call_id.as_deref() == Some(tool_call_id)
    })
}

fn recent_tool_exchange_start(messages: &[ChatMessage], tool_call_id: &str) -> Option<usize> {
    if !recent_tool_result_matches(messages, tool_call_id) || messages.len() < 2 {
        return None;
    }
    let assistant_index = messages.len() - 2;
    let assistant = &messages[assistant_index];
    (assistant.role == "assistant"
        && assistant
            .tool_calls
            .as_ref()
            .is_some_and(|calls| calls.iter().any(|call| call.id == tool_call_id)))
    .then_some(assistant_index)
}

fn oriented_child_limit_error(max_children: u8, child_count: i64) -> Option<String> {
    (child_count >= i64::from(max_children)).then(|| {
        format!("oriented task decomposition child limit exceeded; max_children is {max_children}")
    })
}

async fn oriented_child_count_policy_error(
    pool: &sqlx::PgPool,
    bear_id: uuid::Uuid,
    canonical_tool_name: &str,
    args: &serde_json::Value,
    orientation: Option<&ObjectiveOrientation>,
) -> Result<Option<String>, DenError> {
    if !matches!(
        canonical_tool_name,
        DEN_TASK_CREATE_PROVIDER | DEN_TASK_UPDATE_PROVIDER
    ) {
        return Ok(None);
    }
    let Some(ObjectiveOrientation::Oriented { task }) = orientation else {
        return Ok(None);
    };
    let Some(parent_task_id) = args
        .get("parent_task_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(None);
    };
    let OrientationTaskRef::DocketTask {
        task_id: oriented_task_id,
        ..
    } = &task.task_ref
    else {
        return Ok(None);
    };
    if parent_task_id != oriented_task_id || task.child_policy.max_children == 0 {
        return Ok(None);
    }
    let parent_task_id = uuid::Uuid::parse_str(parent_task_id).map_err(|_| {
        DenError::ValidationError("parent_task_id must be a valid UUID".to_string())
    })?;
    let child_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM bear_tasks
        WHERE bear_id = $1
          AND parent_task_id = $2
        "#,
    )
    .bind(bear_id)
    .bind(parent_task_id)
    .fetch_one(pool)
    .await?;
    if let Some(error) = oriented_child_limit_error(task.child_policy.max_children, child_count) {
        return Ok(Some(error));
    }
    Ok(None)
}

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
    pending_checkpoint_thinking: Option<RuntimeSemanticEvent>,
    pending_server_tool: Option<ServerToolFuture>,
    pending_server_tool_stream: Option<ServerToolContinuationFuture>,
    pending_server_tool_continuation: Option<String>,
    pending_final_gate_continuation: Option<FinalGateContinuationFuture>,
    pending_final_gate_focus: Option<FinalGateFocusFuture>,
    dispatch_mode: NativeToolDispatchMode,
    config: Arc<Config>,
    stores: MemoryStoreManager,
    profile: BearProfile,
    may_define_task: bool,
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
        let may_define_task = match &session.objective_orientation {
            crate::agent_loop::ObjectiveOrientation::Freeform { policy } => policy.may_define_task,
            crate::agent_loop::ObjectiveOrientation::Oriented { .. }
            | crate::agent_loop::ObjectiveOrientation::Focused { .. } => true,
        };
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
            pending_checkpoint_thinking: None,
            pending_server_tool: None,
            pending_server_tool_stream: None,
            pending_server_tool_continuation: None,
            pending_final_gate_continuation: None,
            pending_final_gate_focus: None,
            dispatch_mode,
            config,
            stores,
            profile,
            may_define_task,
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
        self.store.update(
            &self.session_key,
            |session| match recent_tool_exchange_start(&session.messages, tool_call_id) {
                Some(assistant_index) => session.messages.truncate(assistant_index),
                None if recent_tool_result_matches(&session.messages, tool_call_id) => {
                    session.messages.pop();
                }
                None => {}
            },
        );
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
        let session = self.store.get(&self.session_key);
        let workspace_roots = session
            .as_ref()
            .map(|session| session.workspace_roots.clone())
            .unwrap_or_default();
        let context_budget = session
            .as_ref()
            .and_then(|session| session.latest_context_budget.clone())
            .and_then(|report| serde_json::to_value(report).ok());
        let projected_memory = session
            .as_ref()
            .and_then(|session| session.latest_projected_memory.clone());
        let recalled_memory = session.and_then(|session| session.latest_recalled_memory);
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
            work_run_id: None,
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
            && provider_tool_is_den_web_fetch(tool_name)
    }

    fn should_execute_den_tool_server_side(&self, tool_name: &str) -> bool {
        self.dispatch_mode == NativeToolDispatchMode::DeferToClient
            && builtin_den_tool_descriptor_for_provider_name(tool_name).is_some()
            && provider_tool_supports_unilateral_execution(tool_name)
            && !self.should_request_den_tool_permission(tool_name)
            && (self.may_define_task
                || !is_task_definition_or_delegation_tool_provider_name(tool_name))
    }

    fn task_definition_policy_error(
        &self,
        canonical_tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<String> {
        if !matches!(
            canonical_tool_name,
            DEN_TASK_CREATE_PROVIDER | DEN_TASK_UPDATE_PROVIDER
        ) {
            return None;
        }
        let orientation = self
            .store
            .get(&self.session_key)
            .map(|session| session.objective_orientation)?;
        match orientation {
            ObjectiveOrientation::Focused { job } if !job.mutable => Some(
                "objective orientation is immutable focused; task decomposition is not allowed"
                    .to_string(),
            ),
            ObjectiveOrientation::Oriented { task } => {
                let parent_task_id = args
                    .get("parent_task_id")
                    .and_then(serde_json::Value::as_str);
                let Some(parent_task_id) = parent_task_id else {
                    return None;
                };
                let OrientationTaskRef::DocketTask {
                    task_id: oriented_task_id,
                    ..
                } = task.task_ref
                else {
                    return Some(
                        "oriented task decomposition requires a Docket task parent".to_string(),
                    );
                };
                if parent_task_id != oriented_task_id {
                    return Some(format!(
                        "oriented task decomposition depth limit exceeded; parent_task_id must be the oriented task {}",
                        oriented_task_id
                    ));
                }
                if task.child_policy.max_children == 0 {
                    return Some(
                        "oriented task decomposition child limit exceeded; max_children is 0"
                            .to_string(),
                    );
                }
                if task.child_policy.max_depth_below_oriented_task == 0 {
                    return Some(
                        "oriented task decomposition depth limit exceeded; max depth is 0"
                            .to_string(),
                    );
                }
                None
            }
            _ => None,
        }
    }

    fn started_tool_title(tool_name: &str) -> Option<String> {
        builtin_den_tool_descriptor_for_provider_name(tool_name)
            .map(|descriptor| descriptor.label.to_string())
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
        let entries = Self::plan_update_entries_from_tool_result(&value)?;
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

    fn plan_update_entries_from_tool_result(
        value: &serde_json::Value,
    ) -> Option<Vec<serde_json::Value>> {
        value
            .get("plan")
            .and_then(|plan| plan.get("items"))
            .or_else(|| value.get("task_list").and_then(|plan| plan.get("items")))
            .or_else(|| {
                value
                    .get("sync")
                    .and_then(|sync| sync.get("task_list"))
                    .and_then(|plan| plan.get("items"))
            })
            .and_then(|items| items.as_array())
            .cloned()
            .or_else(|| Self::docket_tool_plan_entries(value))
    }

    fn docket_tool_plan_entries(value: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
        let domain = value.get("domain").and_then(serde_json::Value::as_str);
        if domain != Some("docket") {
            return None;
        }
        if let Some(tasks) = value
            .get("job")
            .and_then(|job| job.get("tasks"))
            .and_then(serde_json::Value::as_array)
        {
            return Some(tasks.iter().map(Self::docket_task_plan_entry).collect());
        }
        if let Some(tasks) = value.get("tasks").and_then(serde_json::Value::as_array) {
            return Some(tasks.iter().map(Self::docket_task_plan_entry).collect());
        }
        value
            .get("task")
            .map(|task| vec![Self::docket_task_plan_entry(task)])
    }

    fn docket_task_plan_entry(task: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": task.get("id"),
            "title": task.get("title"),
            "summary": task.get("body").or_else(|| task.get("summary")),
            "status": task.get("status"),
            "source_ref": {
                "kind": "docket_task",
                "docket_job_id": task.get("job_id"),
                "docket_task_id": task.get("id"),
            },
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
        if let Some(error) = self.task_definition_policy_error(&canonical, &args) {
            self.pending_server_tool =
                Some(Box::pin(
                    async move { Err(DenError::ValidationError(error)) },
                ));
            return;
        }
        let objective_orientation = self
            .store
            .get(&self.session_key)
            .map(|session| session.objective_orientation);
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
            if let Some(error) = oriented_child_count_policy_error(
                &pool,
                bear_id,
                &canonical,
                &args,
                objective_orientation.as_ref(),
            )
            .await?
            {
                return Err(DenError::ValidationError(error));
            }
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
            let continuation = Box::pin(async move {
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
            });
            Ok((call, message, continuation as ServerToolContinuationFuture))
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

    fn checkpoint_failure_event(message: String) -> RuntimeStreamEvent {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: den_protocol::RuntimeErrorCategory::BackendProtocol,
            message,
        })
    }

    fn checkpoint_recovery_event(message: String) -> RuntimeStreamEvent {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress {
            kind: "recoverable_tool_rejection".to_string(),
            text: Some(message),
            phase: Some("runtime_checkpoint".to_string()),
            detail: Some(serde_json::json!({
                "code": "checkpoint_follow_through_required",
                "severity": "recoverable",
                "disposition": "steer_model_and_continue",
                "side_effects": "blocked_before_execution",
                "required_next_tool": RUNTIME_CHECKPOINT_TOOL_NAME,
                "ui": {
                    "presentation": "tool_card",
                    "title": "Checkpoint needed",
                    "intent": "warning"
                },
            })),
        })
    }

    fn checkpoint_recovery_message(
        request: &RuntimeCheckpointRequest,
        attempted_action: &str,
    ) -> String {
        format!(
            "Your attempted action `{attempted_action}` was blocked before execution because Den checkpoint `{}` is pending. Call the `{}` tool before any other tool.",
            request.checkpoint_id, RUNTIME_CHECKPOINT_TOOL_NAME
        )
    }

    fn push_checkpoint_recovery_guidance(
        &self,
        request: &RuntimeCheckpointRequest,
        attempted_action: &str,
    ) {
        let policy = checkpoint_follow_through_required_policy(RUNTIME_CHECKPOINT_TOOL_NAME);
        debug_assert_eq!(policy.severity, RuntimeIssueSeverity::Recoverable);
        debug_assert_eq!(
            policy.disposition,
            RuntimeIssueDisposition::SteerModelAndContinue
        );
        let message = Self::checkpoint_recovery_message(request, attempted_action);
        self.store.update(&self.session_key, |session| {
            session.messages.push(ChatMessage {
                role: "system".to_string(),
                content: Some(message),
                tool_call_id: None,
                name: None,
                tool_calls: None,
            });
        });
    }

    fn required_task_action_satisfied(
        action: &crate::agent_loop::CheckpointNextAction,
        tool_name: &str,
    ) -> bool {
        match action {
            crate::agent_loop::CheckpointNextAction::UpdateTaskList => matches!(
                tool_name,
                DEN_TASK_LISTS_UPDATE_PROVIDER | DEN_TASK_UPDATE_CURRENT_STATUS_PROVIDER
            ),
            crate::agent_loop::CheckpointNextAction::SyncTaskList => {
                tool_name == DEN_TASK_LIST_SYNC_PROVIDER
            }
            crate::agent_loop::CheckpointNextAction::RequestHandoff => {
                tool_name == DEN_TASK_LISTS_REQUEST_HANDOFF_PROVIDER
            }
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

    fn checkpoint_has_task_followthrough_context(request: &RuntimeCheckpointRequest) -> bool {
        request.task_context.as_ref().is_some_and(|context| {
            context.task_list_id.is_some()
                || context.active_item_id.is_some()
                || context.docket_job_id.is_some()
                || context.docket_task_id.is_some()
        })
    }

    fn pending_checkpoint_task_action(&self) -> Option<crate::agent_loop::CheckpointNextAction> {
        self.store
            .get(&self.session_key)
            .and_then(|session| session.pending_checkpoint_task_action)
    }

    #[allow(clippy::result_large_err)]
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
                session.pending_checkpoint_recovery_attempts = 0;
            });
            return Ok(());
        }
        Err(Self::checkpoint_failure_event(format!(
            "Den needs task-state follow-through with `{action:?}` before continuing, but the assistant next called `{tool_name}`. Use the task-management tool indicated by the checkpoint response before other actions. No blocked tool was executed."
        )))
    }

    #[allow(clippy::result_large_err)]
    fn fail_if_checkpoint_task_action_pending(&self) -> Result<(), RuntimeStreamEvent> {
        if let Some(action) = self.pending_checkpoint_task_action() {
            return Err(Self::checkpoint_failure_event(format!(
                "Den needs task-state follow-through with `{action:?}` before stopping, but the assistant attempted to stop before using the required task-management tool. No blocked tool was executed."
            )));
        }
        Ok(())
    }

    fn apply_valid_checkpoint_response(
        &mut self,
        request: RuntimeCheckpointRequest,
        response: RuntimeCheckpointResponse,
    ) {
        let checkpoint_summary = response
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(str::to_string);
        let required_task_action = Self::checkpoint_has_task_followthrough_context(&request)
            .then(|| {
                response.task_state_change_needed.as_ref().and_then(|_| {
                    Self::checkpoint_next_action_can_satisfy_task_state_change(
                        &response.next_action,
                    )
                    .then_some(response.next_action.clone())
                })
            })
            .flatten();
        self.store.update(&self.session_key, |session| {
            session.pending_checkpoint_request = None;
            session.pending_checkpoint_recovery_attempts = 0;
            session
                .pending_checkpoint_task_action
                .clone_from(&required_task_action);
            session.checkpoint_state.reset_after_checkpoint_report();
        });
        self.pending_checkpoint_thinking =
            checkpoint_summary.map(|text| RuntimeSemanticEvent::ReasoningTextDelta {
                text: format!("{text}\n"),
            });
        self.record_checkpoint_response_if_audited(request, response);
    }

    fn degrade_invalid_checkpoint_response(&mut self, reason: String) {
        tracing::warn!(
            session_id = %self.client_session_id,
            reason = %reason,
            "degrading invalid checkpoint response without failing turn"
        );
        self.store.update(&self.session_key, |session| {
            session.pending_checkpoint_request = None;
            session.pending_checkpoint_recovery_attempts = 0;
            session.checkpoint_state.reset_after_checkpoint_report();
        });
    }

    fn checkpoint_tool_result_content(ok: bool, message: &str) -> String {
        serde_json::json!({
            "ok": ok,
            "message": message,
            "source": "den.runtime.checkpoint",
        })
        .to_string()
    }

    fn handle_checkpoint_tool_call(
        &mut self,
        tool_call_id: String,
        arguments: serde_json::Value,
    ) -> RuntimeStreamEvent {
        let call = ChatToolCall {
            id: tool_call_id.clone(),
            call_type: "function".to_string(),
            function: crate::llm::ChatToolCallFunction {
                name: RUNTIME_CHECKPOINT_TOOL_NAME.to_string(),
                arguments: arguments.to_string(),
            },
        };
        self.tool_calls.insert(
            tool_call_id.clone(),
            (
                RUNTIME_CHECKPOINT_TOOL_NAME.to_string(),
                arguments.to_string(),
            ),
        );
        self.sync_assistant_tool_step_to_session();
        self.persist_assistant_tool_step();

        let content = match self.pending_checkpoint_request() {
            Some(request) => match serde_json::from_value::<RuntimeCheckpointResponse>(arguments) {
                Ok(response) => match validate_checkpoint_response(&request, &response) {
                    Ok(()) => {
                        self.apply_valid_checkpoint_response(request, response);
                        Self::checkpoint_tool_result_content(true, "checkpoint recorded")
                    }
                    Err(err) => {
                        self.degrade_invalid_checkpoint_response(format!(
                            "checkpoint tool arguments failed validation: {err:?}"
                        ));
                        Self::checkpoint_tool_result_content(
                            false,
                            "checkpoint arguments were not valid; continuing with deterministic runtime signals",
                        )
                    }
                },
                Err(err) => {
                    self.degrade_invalid_checkpoint_response(format!(
                        "checkpoint tool arguments were not parseable: {err}"
                    ));
                    Self::checkpoint_tool_result_content(
                        false,
                        "checkpoint arguments were not parseable; continuing with deterministic runtime signals",
                    )
                }
            },
            None => Self::checkpoint_tool_result_content(false, "no checkpoint was pending"),
        };

        self.tool_calls.remove(&tool_call_id);
        self.store.update(&self.session_key, |session| {
            session.messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(content.clone()),
                tool_call_id: Some(tool_call_id),
                name: Some(RUNTIME_CHECKPOINT_TOOL_NAME.to_string()),
                tool_calls: None,
            });
        });
        self.begin_checkpoint_continuation();
        RuntimeStreamEvent::Semantic(tool_call_finished_event_for_content(&call, Some(&content)))
    }

    fn block_or_recover_if_checkpoint_pending(
        &mut self,
        attempted_action: &str,
    ) -> Result<(), RuntimeStreamEvent> {
        if !self.enforce_checkpoint_responses() {
            return Ok(());
        }
        let Some(request) = self.pending_checkpoint_request() else {
            return Ok(());
        };
        let mut attempts = MAX_CHECKPOINT_RECOVERY_ATTEMPTS.saturating_add(1);
        self.store.update(&self.session_key, |session| {
            session.pending_checkpoint_recovery_attempts = session
                .pending_checkpoint_recovery_attempts
                .saturating_add(1);
            attempts = session.pending_checkpoint_recovery_attempts;
        });

        if attempts > MAX_CHECKPOINT_RECOVERY_ATTEMPTS {
            return Err(Self::checkpoint_failure_event(format!(
                "Den got stuck satisfying checkpoint `{}` after {MAX_CHECKPOINT_RECOVERY_ATTEMPTS} recoverable correction attempts; the assistant attempted `{attempted_action}` before calling the `{}` tool. No blocked tool was executed.",
                request.checkpoint_id, RUNTIME_CHECKPOINT_TOOL_NAME
            )));
        }

        let message = Self::checkpoint_recovery_message(&request, attempted_action);
        self.push_checkpoint_recovery_guidance(&request, attempted_action);
        self.begin_checkpoint_continuation();
        Err(Self::checkpoint_recovery_event(message))
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

    fn prepare_autonomous_final_gate(&mut self, focus: RuntimeFocusContext) {
        let focused_task_list = focus.active_activity_plan().cloned();
        self.store.update(&self.session_key, |session| {
            session.cached_activity_plan_projection = focused_task_list.clone();
        });
        self.evaluate_final_gate_or_complete(focused_task_list);
    }

    fn evaluate_final_gate_or_complete(
        &mut self,
        cached_activity_plan_projection: Option<TaskListProjection>,
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
        let decision = decide_turn_completion(TurnCompletionPolicyInput {
            profile: self.profile,
            focused_task_list: cached_activity_plan_projection.as_ref(),
            assistant_text: &self.assistant_text,
            recent_texts: &recent_texts,
        });

        match decision {
            TurnCompletionDecision::Continue {
                reason,
                next_task,
                final_response_kind,
                ..
            } => {
                tracing::info!(
                    client_session_id = %self.client_session_id,
                    profile = %self.profile.as_str(),
                    next_task = %next_task,
                    reason = ?reason,
                    final_response_kind = ?final_response_kind,
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
                        "reason": format!("{reason:?}"),
                        "final_response_kind": format!("{final_response_kind:?}"),
                    })),
                });
                return;
            }
            TurnCompletionDecision::Complete {
                reason: TurnCompletionCompleteReason::RepeatedTerminalObjection,
                loop_detection: Some(loop_detection),
                ..
            } => {
                self.finished = true;
                tracing::warn!(
                    client_session_id = %self.client_session_id,
                    profile = %self.profile.as_str(),
                    terminal_objections = loop_detection.terminal_objections,
                    continuation_nudges = loop_detection.continuation_nudges,
                    repeated_objection_kind = ?loop_detection.repeated_objection_kind,
                    "native runtime task-focus loop detected; accepting terminal objection instead of issuing another continuation nudge"
                );
                self.pending_pause_after_tool =
                    Some(RuntimeSemanticEvent::TurnCompleted { turn: None });
                return;
            }
            TurnCompletionDecision::Complete {
                reason:
                    TurnCompletionCompleteReason::NoActiveFocusedTask
                    | TurnCompletionCompleteReason::FocusedWorkCompleteFinalizationDrain,
                ..
            } => {
                self.store.update(&self.session_key, |session| {
                    session.governance = Governance::Interactive;
                    session.cached_activity_plan_projection = None;
                });
            }
            TurnCompletionDecision::Complete { .. } => {}
        }

        let pool = self.pool.clone();
        let config = self.config.clone();
        let bear_id = self.bear_id;
        let conversation_id = self.conversation_id.clone();
        let profile = self.profile;
        tokio::spawn(async move {
            enqueue_compaction_after_turn(&pool, &config, bear_id, &conversation_id, profile).await;
        });
        self.finished = true;
        self.pending_pause_after_tool = Some(RuntimeSemanticEvent::TurnCompleted { turn: None });
    }

    fn begin_final_gate_focus_resolution(&mut self) {
        // Final-answer gating is a behavior boundary: resolve durable focus here
        // instead of trusting the session projection cache.
        let pool = self.pool.clone();
        let bear_id = self.bear_id;
        let profile = self.profile;
        let user_id = self.user_id;
        let conversation_id = self.conversation_id.clone();
        let client_session_id = self.client_session_id.clone();
        let cached_activity_plan_projection = self
            .store
            .get(&self.session_key)
            .and_then(|session| session.cached_activity_plan_projection);
        self.pending_final_gate_focus = Some(Box::pin(async move {
            resolve_runtime_focus_context(
                &pool,
                RuntimeFocusResolveRequest {
                    bear_id,
                    profile,
                    user_id,
                    conversation_id,
                    client_session_id,
                    cached_activity_plan_projection,
                },
            )
            .await
        }));
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
        let model_message = render_final_gate_continuation_guidance(next_task).unwrap_or_else(|err| {
            tracing::warn!(
                error = %err,
                "failed to render final-gate continuation fragment; using fallback guidance"
            );
            format!(
                "You are in autonomous implementation mode. The active task list still has incomplete, unblocked work. Do not final-answer yet. Continue with: {next_task}."
            )
        });
        self.store.update(&self.session_key, |session| {
            session.governance = Governance::AutonomousContinuation;
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
        // Terminal events are queued before some paths mark the stream finished. Drain the
        // queued event first so BearWire observes the authoritative turn outcome before EOF.
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

        if let Some(event) = self.pending_checkpoint_thinking.take() {
            return Poll::Ready(Some(Ok(RuntimeStreamEvent::Semantic(event))));
        }

        if self.finished {
            return Poll::Ready(None);
        }

        if let Some(fut) = self.pending_server_tool.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok((call, message, continuation))) => {
                    self.pending_server_tool = None;
                    self.tool_calls.remove(&call.id);
                    self.pending_server_tool_stream = Some(continuation);
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

        if let Some(fut) = self.pending_server_tool_stream.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok(stream)) => {
                    self.pending_server_tool_stream = None;
                    self.inner = stream;
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(Err(error)) => {
                    self.pending_server_tool_stream = None;
                    if let Some(tool_call_id) = self.pending_server_tool_continuation.take() {
                        self.remove_recent_server_tool_chain_from_session(&tool_call_id);
                    }
                    return Poll::Ready(Some(Err(error)));
                }
                Poll::Pending => return Poll::Pending,
            }
        }

        if let Some(fut) = self.pending_final_gate_focus.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Ready(Ok(focus)) => {
                    self.pending_final_gate_focus = None;
                    self.prepare_autonomous_final_gate(focus);
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Poll::Ready(Err(error)) => {
                    self.pending_final_gate_focus = None;
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
                if tool_name == RUNTIME_CHECKPOINT_TOOL_NAME {
                    let started =
                        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
                            tool_call_id: tool_call_id.clone(),
                            tool_name,
                            title: Some("Runtime checkpoint".to_string()),
                            kind: Some("function".to_string()),
                            arguments: arguments.clone(),
                            approval_request_id: None,
                            approval_required: false,
                            approval_reason: None,
                            run_id: None,
                        });
                    let finished = self.handle_checkpoint_tool_call(tool_call_id, arguments);
                    if let RuntimeStreamEvent::Semantic(event) = finished {
                        self.pending_pause_after_tool = Some(event);
                    }
                    return Poll::Ready(Some(Ok(started)));
                }
                if let Err(event) =
                    self.block_or_recover_if_checkpoint_pending(&format!("tool_call:{tool_name}"))
                {
                    if matches!(
                        event,
                        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed { .. })
                    ) {
                        self.finished = true;
                    }
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
                            tool_name: tool_name.clone(),
                            title: Self::started_tool_title(&tool_name),
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
                let approval_required = provider_tool_requires_approval(&tool_name);
                let event = RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    title: Self::started_tool_title(&tool_name),
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
                    return Poll::Ready(Some(Ok(event)));
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
                if let Err(event) = self.block_or_recover_if_checkpoint_pending("final_answer") {
                    if matches!(
                        event,
                        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed { .. })
                    ) {
                        self.finished = true;
                    }
                    return Poll::Ready(Some(Ok(event)));
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
                self.begin_final_gate_focus_resolution();
                cx.waker().wake_by_ref();
                Poll::Pending
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
                if other.is_none() {
                    if let Some(tool_call_id) = self.pending_server_tool_continuation.take() {
                        tracing::warn!(
                            client_session_id = %self.client_session_id,
                            tool_call_id = %tool_call_id,
                            "native runtime server-tool continuation ended without a terminal event; removing recent tool chain from in-memory session"
                        );
                        self.remove_recent_server_tool_chain_from_session(&tool_call_id);
                        self.finished = true;
                        return Poll::Ready(Some(Ok(Self::checkpoint_failure_event(
                            "Server-tool continuation stream ended without a terminal runtime event."
                                .to_string(),
                        ))));
                    }
                    if self.assistant_text.trim().is_empty() {
                        self.finished = true;
                        return Poll::Ready(Some(Ok(Self::checkpoint_failure_event(
                            "Model stream ended without assistant output or a terminal runtime event."
                                .to_string(),
                        ))));
                    }
                    // ponytail: some provider/client streams can EOF after assistant text without
                    // emitting an explicit TurnCompleted. Treat that as a terminal answer candidate
                    // and send it through the same focused-work gate; upgrade path is making every
                    // provider adapter emit an explicit terminal event before EOF.
                    self.persist_assistant_tool_step();
                    self.begin_final_gate_focus_resolution();
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                let failed_continuation = matches!(
                    &other,
                    Some(Ok(RuntimeStreamEvent::Semantic(
                        RuntimeSemanticEvent::TurnFailed { .. }
                            | RuntimeSemanticEvent::TurnCancelled { .. }
                            | RuntimeSemanticEvent::Error { .. }
                    )))
                );
                if failed_continuation {
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
            PostMutationVerificationWindow, StrategyProfile, ToolCallBudgetLimits,
            TurnBudgetPolicy,
        },
        llm::{ChatMessage, ChatToolCall, ChatToolCallFunction},
    };
    use den_core::config::Config;
    use den_memory::MemoryStoreManager;
    use den_protocol::{RuntimeSemanticEvent, RuntimeStreamEvent};
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
                stance: Some(BearProfile::Pair),
                objective_orientation: None,
                pre_risk: false,
            }),
            governance: den_core::governance::Governance::Interactive,
            objective_orientation: crate::agent_loop::ObjectiveOrientation::Freeform {
                policy: crate::agent_loop::FreeformPolicy::closed(),
            },
            checkpoint_state: Default::default(),
            pending_checkpoint_request: None,
            pending_checkpoint_task_action: None,
            pending_checkpoint_recovery_attempts: 0,
            strategy: StrategyProfile::plain_react(),
            stream_tokens: true,
            key_memory_projection_cache_key: None,
            latest_context_budget: None,
            latest_projected_memory: None,
            latest_recalled_memory: None,
            cached_activity_plan_projection: None,
            profile: BearProfile::Pair,
            overflow_retry_attempted: false,
            overflow_compaction_recovered: false,
        }
    }

    #[tokio::test]
    async fn immutable_focused_orientation_denies_task_decomposition() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.objective_orientation = crate::agent_loop::ObjectiveOrientation::Focused {
            job: crate::agent_loop::JobOrientation {
                job_id: uuid::Uuid::new_v4().to_string(),
                active_task_ref: None,
                mutable: false,
            },
        };
        let stream = test_tracking_stream_with_session(&session);

        let error = stream
            .task_definition_policy_error(
                "create_task",
                &serde_json::json!({ "parent_task_id": uuid::Uuid::new_v4() }),
            )
            .expect("immutable focused task creation is rejected");

        assert!(error.contains("immutable focused"));
    }

    #[tokio::test]
    async fn oriented_orientation_rejects_deeper_child_parent() {
        let oriented_task_id = uuid::Uuid::new_v4().to_string();
        let nested_parent_id = uuid::Uuid::new_v4().to_string();
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.objective_orientation = crate::agent_loop::ObjectiveOrientation::Oriented {
            task: crate::agent_loop::TaskOrientation {
                task_ref: crate::agent_loop::OrientationTaskRef::DocketTask {
                    job_id: Some(uuid::Uuid::new_v4().to_string()),
                    task_id: oriented_task_id.clone(),
                    title: Some("Oriented task".to_string()),
                },
                child_policy: crate::agent_loop::OrientedChildTaskPolicy {
                    max_children: 6,
                    max_depth_below_oriented_task: 1,
                },
            },
        };
        let stream = test_tracking_stream_with_session(&session);

        let error = stream
            .task_definition_policy_error(
                "create_task",
                &serde_json::json!({ "parent_task_id": nested_parent_id }),
            )
            .expect("nested decomposition is rejected");
        assert!(error.contains("depth limit exceeded"));

        assert!(stream
            .task_definition_policy_error(
                "create_task",
                &serde_json::json!({ "parent_task_id": oriented_task_id }),
            )
            .is_none());
    }

    #[tokio::test]
    async fn oriented_orientation_rejects_zero_child_cap() {
        let oriented_task_id = uuid::Uuid::new_v4().to_string();
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.objective_orientation = crate::agent_loop::ObjectiveOrientation::Oriented {
            task: crate::agent_loop::TaskOrientation {
                task_ref: crate::agent_loop::OrientationTaskRef::DocketTask {
                    job_id: Some(uuid::Uuid::new_v4().to_string()),
                    task_id: oriented_task_id.clone(),
                    title: None,
                },
                child_policy: crate::agent_loop::OrientedChildTaskPolicy {
                    max_children: 0,
                    max_depth_below_oriented_task: 1,
                },
            },
        };
        let stream = test_tracking_stream_with_session(&session);

        let error = stream
            .task_definition_policy_error(
                "create_task",
                &serde_json::json!({ "parent_task_id": oriented_task_id }),
            )
            .expect("zero child cap is rejected");
        assert!(error.contains("child limit exceeded"));
    }

    #[test]
    fn oriented_child_limit_rejects_at_cap() {
        assert!(oriented_child_limit_error(6, 5).is_none());
        let error = oriented_child_limit_error(6, 6).expect("cap is enforced");
        assert!(error.contains("max_children is 6"));
    }

    #[tokio::test]
    async fn pending_checkpoint_blocks_non_checkpoint_actions_in_enforce_mode() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.pending_checkpoint_request = Some(checkpoint_request("ckpt-required"));
        let mut stream = test_tracking_stream_with_session(&session);

        let tool_err = stream
            .block_or_recover_if_checkpoint_pending("tool_call:memory_read")
            .expect_err("non-checkpoint tool call is blocked and steered");
        assert!(matches!(
            tool_err,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress { ref kind, ref text, ref detail, .. })
                if kind == "recoverable_tool_rejection"
                    && text.as_deref().is_some_and(|text| text.contains("ckpt-required"))
                    && detail
                        .as_ref()
                        .and_then(|detail| detail.pointer("/ui/presentation"))
                        .and_then(serde_json::Value::as_str)
                        == Some("tool_card")
        ));

        let final_err = stream
            .block_or_recover_if_checkpoint_pending("final_answer")
            .expect_err("final answer is blocked and steered while retry budget remains");
        assert!(matches!(
            final_err,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress { ref kind, ref text, .. })
                if kind == "recoverable_tool_rejection"
                    && text.as_deref().is_some_and(|text| text.contains("final_answer"))
        ));
    }

    #[tokio::test]
    async fn pending_checkpoint_rejects_wrong_next_tool_recoverably_before_escalation() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.pending_checkpoint_request = Some(checkpoint_request("ckpt-recoverable"));
        let mut stream = test_tracking_stream_with_session(&session);

        let event = stream
            .block_or_recover_if_checkpoint_pending("tool_call:run_command")
            .expect_err("wrong next tool is blocked and steered");
        assert!(matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress {
                ref kind,
                ref text,
                ..
            }) if kind == "recoverable_tool_rejection"
                && text.as_deref().is_some_and(|text| text.contains("blocked before execution"))
        ));
        let stored = stream
            .store
            .get(&stream.session_key)
            .expect("stored session");
        assert_eq!(stored.pending_checkpoint_recovery_attempts, 1);
        assert!(stored.messages.last().is_some_and(|message| {
            message.role == "system"
                && message
                    .content
                    .as_deref()
                    .is_some_and(|content| content.contains("Call the `checkpoint` tool"))
        }));
        assert!(stream.pending_final_gate_continuation.is_some());
    }

    #[tokio::test]
    async fn pending_checkpoint_repeated_wrong_next_tool_escalates() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.pending_checkpoint_request = Some(checkpoint_request("ckpt-escalate"));
        session.pending_checkpoint_recovery_attempts = MAX_CHECKPOINT_RECOVERY_ATTEMPTS;
        let mut stream = test_tracking_stream_with_session(&session);

        let event = stream
            .block_or_recover_if_checkpoint_pending("tool_call:run_command")
            .expect_err("retry budget escalates to terminal failure");
        assert!(matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed { ref message, .. })
                if message.contains("ckpt-escalate") && message.contains("No blocked tool was executed")
        ));
    }

    fn checkpoint_request(checkpoint_id: &str) -> RuntimeCheckpointRequest {
        RuntimeCheckpointRequest {
            checkpoint_id: checkpoint_id.to_string(),
            run_id: "run-test".to_string(),
            reason: crate::agent_loop::CheckpointReason::OverExploration,
            control_level: den_core::AgentLoopControlLevel::Careful,
            profile_fingerprint: None,
            active_objective: Some("Update task state".to_string()),
            task_context: None,
            evidence_refs: Vec::new(),
            required_fields: vec![
                crate::agent_loop::CheckpointField::ActiveObjective,
                crate::agent_loop::CheckpointField::MoreExplorationJustified,
                crate::agent_loop::CheckpointField::NextAction,
            ],
        }
    }

    fn task_checkpoint_request(checkpoint_id: &str) -> RuntimeCheckpointRequest {
        RuntimeCheckpointRequest {
            task_context: Some(crate::agent_loop::CheckpointTaskContext {
                task_list_id: Some("task-list-test".to_string()),
                task_list_version: Some("1".to_string()),
                active_item_id: Some("task-item-test".to_string()),
                active_item_title: Some("Update task state".to_string()),
                docket_job_id: None,
                docket_task_id: None,
            }),
            ..checkpoint_request(checkpoint_id)
        }
    }

    fn checkpoint_response_json(
        checkpoint_id: &str,
        next_action: serde_json::Value,
        task_state_change_needed: serde_json::Value,
    ) -> String {
        serde_json::json!({
            "checkpoint_id": checkpoint_id,
            "active_objective": "Update task state",
            "summary": "Checkpoint says the run should reconcile task state before continuing.",
            "learned": ["The remaining item needs explicit state reconciliation."],
            "remaining_uncertainty": [],
            "more_exploration_justified": false,
            "next_action": next_action,
            "task_state_change_needed": task_state_change_needed,
            "evidence_refs": [],
            "confidence": "medium"
        })
        .to_string()
    }

    fn test_tracking_stream_with_session(session: &AgentLoopSession) -> SessionTrackingStream {
        let store = AgentLoopSessionStore::default();
        store.insert(session.clone());
        SessionTrackingStream::new(
            Box::pin(futures::stream::empty()),
            session,
            store,
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop")
                .expect("lazy test pool"),
            session.bear_id,
            session.bear_slug.clone(),
            session.user_id,
            session.conversation_id.clone(),
            session.client_session_id.clone(),
            session.request_id.clone(),
            Arc::new(Config::test_stub()),
            MemoryStoreManager::new(&Config::test_stub()),
            session.profile,
            NativeToolDispatchMode::DeferToClient,
        )
    }

    #[tokio::test]
    async fn final_gate_continuation_marks_session_autonomous() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.governance = Governance::Interactive;
        let mut stream = test_tracking_stream_with_session(&session);

        stream.begin_final_gate_continuation("finish the task");

        let stored = stream
            .store
            .get(&stream.session_key)
            .expect("stored session after continuation");
        assert_eq!(stored.governance, Governance::AutonomousContinuation);
        assert!(stored
            .messages
            .last()
            .and_then(|message| message.content.as_deref())
            .is_some_and(|content| content.contains("autonomous implementation mode")));
    }

    fn completed_task_list_projection() -> den_docket::TaskListProjection {
        den_docket::TaskListProjection {
            id: uuid::Uuid::nil(),
            bear_id: uuid::Uuid::nil(),
            title: "Focused work".to_string(),
            summary: "Acceptance criteria".to_string(),
            owner_profile: "pair".to_string(),
            visibility: "bear_visible".to_string(),
            status: "completed".to_string(),
            version: 1,
            source_ref: den_docket::TaskListSourceRef::local(Vec::new()),
            current_item: None,
            items: vec![den_docket::TaskListItem {
                id: "done".to_string(),
                title: "Done task".to_string(),
                summary: None,
                status: den_docket::TaskListItemStatus::Completed,
                blocked_reason: None,
                source_ref: den_docket::TaskListSourceRef::local(Vec::new()),
                sync_state: den_docket::TaskListSyncState::Clean,
            }],
            source_conversation_id: None,
            source_client_session_id: None,
            handoff_intent_path: None,
            handoff_task_id: None,
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            updated_at: time::OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[tokio::test]
    async fn final_gate_completion_drains_completed_focus_state() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.governance = Governance::AutonomousContinuation;
        session.cached_activity_plan_projection = Some(completed_task_list_projection());
        let mut stream = test_tracking_stream_with_session(&session);

        stream.evaluate_final_gate_or_complete(Some(completed_task_list_projection()));

        let stored = stream
            .store
            .get(&stream.session_key)
            .expect("stored session after final gate");
        assert!(stream.finished);
        assert_eq!(stored.governance, Governance::Interactive);
        assert!(stored.cached_activity_plan_projection.is_none());
    }

    #[tokio::test]
    async fn final_gate_ignores_and_clears_cache_without_durable_focus() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.governance = Governance::AutonomousContinuation;
        session.cached_activity_plan_projection = Some(completed_task_list_projection());
        let mut stream = test_tracking_stream_with_session(&session);

        stream.prepare_autonomous_final_gate(RuntimeFocusContext {
            source: crate::runtime::focus_context::RuntimeFocusSource::None,
            cached_activity_plan_projection: Some(completed_task_list_projection()),
        });

        let stored = stream
            .store
            .get(&stream.session_key)
            .expect("stored session after cache-only final gate");
        assert!(stream.finished);
        assert_eq!(stored.governance, Governance::Interactive);
        assert!(stored.cached_activity_plan_projection.is_none());
        assert!(stream.pending_final_gate_continuation.is_none());
    }

    #[test]
    fn checkpoint_task_state_change_requires_task_management_next_action() {
        assert!(
            SessionTrackingStream::checkpoint_next_action_can_satisfy_task_state_change(
                &crate::agent_loop::CheckpointNextAction::UpdateTaskList
            )
        );
        assert!(
            SessionTrackingStream::checkpoint_next_action_can_satisfy_task_state_change(
                &crate::agent_loop::CheckpointNextAction::SyncTaskList
            )
        );
        assert!(
            SessionTrackingStream::checkpoint_next_action_can_satisfy_task_state_change(
                &crate::agent_loop::CheckpointNextAction::RequestHandoff
            )
        );
        assert!(
            !SessionTrackingStream::checkpoint_next_action_can_satisfy_task_state_change(
                &crate::agent_loop::CheckpointNextAction::CallTool { tool_name: None }
            )
        );
    }

    #[tokio::test]
    async fn validated_checkpoint_response_sets_required_task_action_follow_through() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.pending_checkpoint_request = Some(task_checkpoint_request("ckpt-follow-through"));
        session.checkpoint_state.read_search_since_mutation = 5;
        session.checkpoint_state.consecutive_failures = 2;
        session.checkpoint_state.same_signature_repeat_count = 2;
        session.checkpoint_state.last_signature = Some("memory_read:{path=a}".to_string());
        let mut stream = test_tracking_stream_with_session(&session);
        let arguments: serde_json::Value = serde_json::from_str(&checkpoint_response_json(
            "ckpt-follow-through",
            serde_json::json!("update_task_list"),
            serde_json::json!({
                "target_state": "blocked",
                "reason": "Missing deployment credential",
                "evidence_refs": []
            }),
        ))
        .expect("checkpoint response json");

        let event = stream.handle_checkpoint_tool_call("call-checkpoint".to_string(), arguments);
        assert!(matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallFinished { .. })
        ));
        assert!(matches!(
            stream.pending_checkpoint_task_action(),
            Some(crate::agent_loop::CheckpointNextAction::UpdateTaskList)
        ));
        assert!(stream.pending_checkpoint_thinking.is_some());
        assert!(matches!(
            stream.pending_checkpoint_thinking,
            Some(RuntimeSemanticEvent::ReasoningTextDelta { ref text })
                if text.contains("Checkpoint says the run should reconcile task state")
        ));
        assert!(stream
            .enforce_required_checkpoint_task_action("memory_read")
            .is_err());
        let stored = stream
            .store
            .get(&stream.session_key)
            .expect("stored checkpoint session");
        assert_eq!(stored.checkpoint_state.read_search_since_mutation, 0);
        assert_eq!(stored.checkpoint_state.consecutive_failures, 0);
        assert_eq!(stored.checkpoint_state.same_signature_repeat_count, 0);
        assert_eq!(stored.checkpoint_state.last_signature, None);
        assert!(stream
            .enforce_required_checkpoint_task_action(DEN_TASK_LISTS_UPDATE_PROVIDER)
            .is_ok());
        assert!(stream.pending_checkpoint_task_action().is_none());
    }

    #[tokio::test]
    async fn checkpoint_task_state_change_without_task_context_does_not_require_follow_through() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.pending_checkpoint_request = Some(checkpoint_request("ckpt-no-task-context"));
        let mut stream = test_tracking_stream_with_session(&session);
        let arguments: serde_json::Value = serde_json::from_str(&checkpoint_response_json(
            "ckpt-no-task-context",
            serde_json::json!("update_task_list"),
            serde_json::json!({
                "target_state": "blocked",
                "reason": "No active task context should not force task follow-through",
                "evidence_refs": []
            }),
        ))
        .expect("checkpoint response json");

        let event = stream.handle_checkpoint_tool_call("call-checkpoint".to_string(), arguments);
        assert!(matches!(
            event,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallFinished { .. })
        ));
        assert!(stream.pending_checkpoint_request().is_none());
        assert!(stream.pending_checkpoint_task_action().is_none());
        assert!(stream.fail_if_checkpoint_task_action_pending().is_ok());
    }

    #[tokio::test]
    async fn checkpoint_task_state_change_with_non_task_next_action_degrades_without_failure() {
        let mut session = test_session("den-conv-test:client-test", uuid::Uuid::new_v4());
        session.pending_checkpoint_request = Some(checkpoint_request("ckpt-advisory-action"));
        let mut stream = test_tracking_stream_with_session(&session);
        let arguments: serde_json::Value = serde_json::from_str(&checkpoint_response_json(
            "ckpt-advisory-action",
            serde_json::json!("validate"),
            serde_json::json!({
                "target_state": "blocked",
                "reason": "Missing deployment credential",
                "evidence_refs": []
            }),
        ))
        .expect("checkpoint response json");

        let _ = stream.handle_checkpoint_tool_call("call-checkpoint".to_string(), arguments);
        assert!(stream.pending_checkpoint_request().is_none());
        assert!(stream.pending_checkpoint_task_action().is_none());
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
        let store = AgentLoopSessionStore::default();
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
    fn plan_update_event_is_derived_from_task_list_tool_result() {
        let message = ChatMessage {
            role: "tool".to_string(),
            content: Some(
                serde_json::json!({
                    "task_list": {
                        "items": [
                            {"id": "task-1", "title": "First", "status": "pending"}
                        ]
                    }
                })
                .to_string(),
            ),
            tool_call_id: Some("call-1".to_string()),
            name: Some("update_task_list".to_string()),
            tool_calls: None,
        };

        let event = SessionTrackingStream::plan_update_event_from_tool_message(&message)
            .expect("plan update event");

        assert!(matches!(
            event,
            RuntimeSemanticEvent::RunProgress { kind, detail: Some(detail), .. }
                if kind == "plan_update"
                    && detail["entries"][0]["id"].as_str() == Some("task-1")
        ));
    }

    #[test]
    fn plan_update_event_is_derived_from_docket_task_tool_result() {
        let message = ChatMessage {
            role: "tool".to_string(),
            content: Some(
                serde_json::json!({
                    "domain": "docket",
                    "task": {
                        "id": "docket-task-1",
                        "job_id": "job-1",
                        "title": "Add task",
                        "body": "Make the ACP plan visible.",
                        "status": "pending"
                    }
                })
                .to_string(),
            ),
            tool_call_id: Some("call-1".to_string()),
            name: Some("create_task".to_string()),
            tool_calls: None,
        };

        let event = SessionTrackingStream::plan_update_event_from_tool_message(&message)
            .expect("plan update event");

        assert!(matches!(
            event,
            RuntimeSemanticEvent::RunProgress { kind, detail: Some(detail), .. }
                if kind == "plan_update"
                    && detail["entries"][0]["id"].as_str() == Some("docket-task-1")
                    && detail["entries"][0]["title"].as_str() == Some("Add task")
                    && detail["entries"][0]["source_ref"]["kind"].as_str() == Some("docket_task")
        ));
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
        let store = AgentLoopSessionStore::default();
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
        let store = AgentLoopSessionStore::default();
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
    async fn queued_terminal_event_is_delivered_before_finished_stream_eof() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::default();
        store.insert(session.clone());
        let mut stream = SessionTrackingStream::new(
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
        stream.finished = true;
        stream.pending_pause_after_tool = Some(RuntimeSemanticEvent::TurnCompleted { turn: None });

        let terminal = stream
            .next()
            .await
            .expect("queued terminal event")
            .expect("ok");
        assert!(matches!(
            terminal,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. })
        ));
        assert!(stream.next().await.is_none(), "EOF follows terminal event");
    }

    #[tokio::test]
    async fn server_tool_completion_is_not_blocked_by_stalled_model_continuation() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::default();
        store.insert(session.clone());
        let mut stream = SessionTrackingStream::new(
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
        let call = ChatToolCall {
            id: "call-list-jobs".to_string(),
            call_type: "function".to_string(),
            function: crate::llm::ChatToolCallFunction {
                name: "list_jobs".to_string(),
                arguments: "{}".to_string(),
            },
        };
        let message = ChatMessage {
            role: "tool".to_string(),
            content: Some("{\"jobs\":[]}".to_string()),
            tool_call_id: Some(call.id.clone()),
            name: Some(call.function.name.clone()),
            tool_calls: None,
        };
        let stalled: ServerToolContinuationFuture = Box::pin(futures::future::pending());
        stream.pending_server_tool = Some(Box::pin(async move { Ok((call, message, stalled)) }));

        let completed = stream.next().await.expect("tool completion").expect("ok");
        assert!(matches!(
            completed,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallFinished {
                ref tool_call_id,
                ref tool_name,
                ..
            }) if tool_call_id == "call-list-jobs" && tool_name == "list_jobs"
        ));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), stream.next())
                .await
                .is_err(),
            "the synthetic continuation should remain stalled after completion is delivered"
        );
    }

    #[tokio::test]
    async fn server_side_den_tool_emits_started_event_before_execution_result() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::default();
        store.insert(session.clone());
        let inner = futures::stream::iter(vec![Ok(RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::ToolCallRequested {
                tool_call_id: "call-list".to_string(),
                tool_name: "list_task_lists".to_string(),
                title: None,
                kind: Some("function".to_string()),
                arguments: serde_json::json!({}),
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

        let first = stream.next().await.expect("started event").expect("ok");
        assert!(matches!(
            first,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
                ref tool_call_id,
                ref tool_name,
                ref arguments,
                ..
            }) if tool_call_id == "call-list" && tool_name == "list_task_lists" && arguments == &serde_json::json!({})
        ));
        assert!(
            stream.pending_server_tool.is_some(),
            "server-side execution should be queued after the visible started event"
        );
    }

    #[tokio::test]
    async fn set_conversation_title_started_event_has_descriptor_title_and_arguments() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::default();
        store.insert(session.clone());
        let inner = futures::stream::iter(vec![Ok(RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::ToolCallRequested {
                tool_call_id: "call-title".to_string(),
                tool_name: "set_conversation_title".to_string(),
                title: None,
                kind: Some("function".to_string()),
                arguments: serde_json::json!({"title":"Roadmap replay hardening"}),
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

        let first = stream.next().await.expect("started event").expect("ok");
        assert!(matches!(
            first,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
                ref tool_call_id,
                ref tool_name,
                ref title,
                ref arguments,
                ..
            }) if tool_call_id == "call-title"
                && tool_name == "set_conversation_title"
                && title.as_deref() == Some("Set conversation title")
                && arguments == &serde_json::json!({"title":"Roadmap replay hardening"})
        ));
        assert!(
            stream.pending_server_tool.is_some(),
            "Den-hosted tool execution should be queued after a full started event"
        );
    }

    #[tokio::test]
    async fn checkpoint_tool_emits_started_event_before_finished_event() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::default();
        store.insert(session.clone());
        let inner = futures::stream::iter(vec![Ok(RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::ToolCallRequested {
                tool_call_id: "call-checkpoint".to_string(),
                tool_name: RUNTIME_CHECKPOINT_TOOL_NAME.to_string(),
                title: None,
                kind: Some("function".to_string()),
                arguments: serde_json::json!({"checkpoint_id":"ckpt-test"}),
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

        let first = stream.next().await.expect("started event").expect("ok");
        assert!(matches!(
            first,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
                ref tool_call_id,
                ref tool_name,
                ref title,
                ..
            }) if tool_call_id == "call-checkpoint"
                && tool_name == RUNTIME_CHECKPOINT_TOOL_NAME
                && title.as_deref() == Some("Runtime checkpoint")
        ));

        let second = stream.next().await.expect("finished event").expect("ok");
        assert!(matches!(
            second,
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallFinished {
                ref tool_call_id,
                ref tool_name,
                ..
            }) if tool_call_id == "call-checkpoint" && tool_name == RUNTIME_CHECKPOINT_TOOL_NAME
        ));
    }

    #[tokio::test]
    async fn den_tools_route_server_side_but_client_tools_do_not() {
        let bear_id = uuid::Uuid::new_v4();
        let session = test_session("den-conv-test:client-test", bear_id);
        let store = AgentLoopSessionStore::default();
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
        assert!(stream.should_execute_den_tool_server_side("create_job"));
        assert!(!stream.should_execute_den_tool_server_side("list_plans"));
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
        let store = AgentLoopSessionStore::default();
        store.insert(session.clone());
        let inner = futures::stream::iter(vec![Ok(RuntimeStreamEvent::Semantic(
            RuntimeSemanticEvent::ToolCallRequested {
                tool_call_id: "call-edit".to_string(),
                tool_name: "fs_edit_file".to_string(),
                title: None,
                kind: Some("function".to_string()),
                arguments: serde_json::json!({"path":"README.md","old_text":"a","new_text":"b"}),
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
        let store = AgentLoopSessionStore::default();
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
