use den_core::config::Config;
use den_core::tools::{
    arguments::DenToolChannelContext,
    constants::DEN_WEB_FETCH,
    context::DenToolInvocationContext,
    descriptor::builtin_den_tool_descriptor_for_provider_name,
    result_compaction::{compact_client_tool_result, ClientToolResultInput, ToolResultStatus},
};
use std::sync::{Arc, LazyLock};

use den_memory::MemoryStoreManager;
use den_protocol::{
    ContinueTurnRequest, RoleRuntimeBinding, RuntimeContinuation, RuntimeConversationBackend,
    RuntimeConversationRef, RuntimeErrorCategory, RuntimeEventStream, RuntimeHistoryPage,
    RuntimeHistoryRecord, RuntimeSemanticEvent, RuntimeStreamContinuation, RuntimeStreamEvent,
    RuntimeToolResultStatus, StartTurnRequest,
};
use den_service::{
    bears::BearProfile,
    conversation::{
        events::{
            canonical_persistence_context, canonical_persistence_enabled_for_conversation,
            persist_canonical_conversation_record, CanonicalConversationRecord,
            CanonicalToolRequestRecord, CanonicalToolResultRecord, ConversationEventProvenance,
        },
        persistence as conversation_persistence,
    },
};
use futures::{stream, StreamExt};
use sqlx::PgPool;
use uuid::Uuid;

use super::web_chat_loop::{NativeWebChatLoopRuntime, NativeWebChatLoopStream};

use crate::{
    agent_loop::{
        agent_loop_session_key, assemble_native_turn_for_bear, classify_tool_budget_class,
        evaluate_checkpoint_trigger, evaluate_turn_budget, projected_memory_session_diagnostic,
        recalled_memory_session_diagnostic, record_approval_decision, record_checkpoint_request,
        resolve_agent_loop_control, run_agent_step_stream, tool_result_content_indicates_error,
        tool_signature_from_call, AgentLoopControlResolutionInput, AgentLoopSession,
        AgentLoopSessionStore, AgentStepOverflowContext, AssembleTurnContext,
        CheckpointArtifactInput, CheckpointField, CheckpointReplayPolicy, CheckpointTaskContext,
        CheckpointTrigger, CheckpointVisibility, NativeToolDispatchMode,
        RuntimeCheckpointRequest, SessionTrackingStream, ToolContinuationObservation,
        TurnBudgetStopReason, TurnBudgetWarning,
    },
    llm::{ChatMessage, ChatToolCall, LlmClient},
    native_runtime::{profile::NativeCapabilityProfile, tools::merge_den_and_client_tools},
    turn_runner::{
        materialize_runtime_conversation_if_needed, TurnContinueRequest, TurnStartRequest,
    },
};
use den_core::DenError;
use den_service::conversation::persistence::PersistedTranscriptRecord;

static SESSION_STORE: LazyLock<AgentLoopSessionStore> = LazyLock::new(AgentLoopSessionStore::new);

fn render_host_context_for_model(prompt_context: Option<&serde_json::Value>) -> Option<String> {
    let host_context = prompt_context?.get("host_context")?;
    if let Some(body_text) = host_context
        .get("body_text")
        .and_then(serde_json::Value::as_str)
    {
        let body_text = body_text.trim();
        if !body_text.is_empty() {
            let kind = host_context
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("referenced_resources");
            let delivery = host_context
                .get("delivery")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("reference_only");
            let persistence = host_context
                .get("persistence")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("not_human_message");
            return Some(format!(
                "<host_context kind=\"{kind}\" delivery=\"{delivery}\" persistence=\"{persistence}\">\n{body_text}\n</host_context>"
            ));
        }
    }

    let resources = host_context
        .get("resources")
        .and_then(serde_json::Value::as_array)?;
    if resources.is_empty() {
        return None;
    }

    let kind = host_context
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("referenced_resources");
    let delivery = host_context
        .get("delivery")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("reference_only");
    let persistence = host_context
        .get("persistence")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("not_human_message");
    let mut lines = vec![
        format!(
            "<host_context kind=\"{kind}\" delivery=\"{delivery}\" persistence=\"{persistence}\">"
        ),
        "The ACP client referenced these resources. They are not human-authored instructions."
            .to_string(),
        "Use available file/content tools for authoritative contents before quoting, editing, or relying on them.".to_string(),
        String::new(),
        "Resources:".to_string(),
    ];
    for (index, resource) in resources.iter().enumerate() {
        let label = resource
            .get("label")
            .or_else(|| resource.get("name"))
            .or_else(|| resource.get("uri"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unnamed resource");
        lines.push(format!("- resource {}: {}", index + 1, label));
        if let Some(uri) = resource.get("uri").and_then(serde_json::Value::as_str) {
            lines.push(format!("  uri: {uri}"));
        }
        if let Some(mime_type) = resource
            .get("mime_type")
            .and_then(serde_json::Value::as_str)
        {
            lines.push(format!("  mime_type: {mime_type}"));
        }
        if let Some(text_bytes) = resource
            .get("embedded_text_bytes")
            .and_then(serde_json::Value::as_u64)
        {
            lines.push(format!(
                "  embedded_text_bytes: {text_bytes} (body omitted; use tools for contents)"
            ));
        }
    }
    if let Some(omitted) = host_context
        .get("omitted_references")
        .and_then(serde_json::Value::as_u64)
    {
        if omitted > 0 {
            lines.push(format!("- omitted_references: {omitted}"));
        }
    }
    lines.push("</host_context>".to_string());
    Some(lines.join("\n"))
}

fn prompt_for_model(prompt: &str, prompt_context: Option<&serde_json::Value>) -> String {
    let Some(host_context) = render_host_context_for_model(prompt_context) else {
        return prompt.to_string();
    };
    format!("{host_context}\n\n<user_message>\n{prompt}\n</user_message>")
}

/// Returns whether this turn recovered from context overflow via emergency compaction.
/// Clears the session flag after reading (for client terminal turn_result mapping).
pub fn take_session_overflow_compaction_recovered(
    conversation_id: &str,
    client_session_id: &str,
) -> bool {
    let key = agent_loop_session_key(conversation_id, client_session_id);
    SESSION_STORE.take_overflow_compaction_recovered(&key)
}

pub fn native_client_session_exists(conversation_id: &str, client_session_id: &str) -> bool {
    let key = agent_loop_session_key(conversation_id, client_session_id);
    SESSION_STORE.get(&key).is_some()
}

async fn persisted_tool_call_exists(
    pool: &PgPool,
    bear_id: Uuid,
    conversation_id: &str,
    tool_call_id: &str,
) -> Result<bool, DenError> {
    let exists = sqlx::query_scalar::<_, bool>(
        r"
        SELECT EXISTS (
            SELECT 1
            FROM conversation_messages
            WHERE conversation_id = (
                SELECT id FROM conversations
                WHERE external_conversation_id = $1 AND bear_id = $2
                LIMIT 1
            )
              AND message_type = 'tool_call'
              AND tool_call_id = $3
        )
        ",
    )
    .bind(conversation_id)
    .bind(bear_id)
    .bind(tool_call_id)
    .fetch_one(pool)
    .await
    .map_err(|err| DenError::Database(format!("check persisted tool_call: {err}")))?;
    Ok(exists)
}

pub async fn record_native_client_tool_result(
    pool: &PgPool,
    conversation_id: &str,
    client_session_id: &str,
    request_id: &str,
    run_id: Option<&str>,
    tool_call_id: &str,
    approval_request_id: Option<&str>,
    status: RuntimeToolResultStatus,
    content: String,
) -> Result<(), DenError> {
    if let Some(approval_request_id) = approval_request_id {
        let approve = matches!(status, RuntimeToolResultStatus::Ok);
        record_approval_decision(
            pool,
            approval_request_id,
            approve,
            Some(if approve {
                "tool_result_delivered"
            } else {
                "tool_result_failed"
            }),
        )
        .await?;
    }

    let session_key = agent_loop_session_key(conversation_id, client_session_id);
    SESSION_STORE.update(&session_key, |session| {
        session.request_id = Some(request_id.to_string());
        session.run_id = run_id
            .map(str::to_string)
            .or_else(|| session.run_id.clone());
        session.messages.push(ChatMessage {
            role: "tool".to_string(),
            content: Some(content.clone()),
            tool_call_id: Some(tool_call_id.to_string()),
            name: None,
            tool_calls: None,
        });
    });
    let Some(session) = SESSION_STORE.get(&session_key) else {
        return Err(DenError::System(
            "native agent loop session not found".to_string(),
        ));
    };
    let matching_call = session
        .messages
        .iter()
        .rev()
        .filter(|message| message.role == "assistant")
        .filter_map(|message| message.tool_calls.as_ref())
        .flat_map(|calls| calls.iter())
        .find(|call| call.id == tool_call_id)
        .cloned();
    let tool_name = matching_call
        .as_ref()
        .map(|call| call.function.name.clone());
    if canonical_persistence_enabled_for_conversation(conversation_id) {
        let persistence_context = canonical_persistence_context(
            pool.clone(),
            session.bear_id,
            session.user_id,
            conversation_id.to_string(),
            Some(client_session_id.to_string()),
            Some(request_id.to_string()),
            client_session_id.to_string(),
            false,
        );
        let provenance = ConversationEventProvenance::client_session(client_session_id.to_string());
        if let Some(call) = matching_call.as_ref() {
            if !persisted_tool_call_exists(pool, session.bear_id, conversation_id, tool_call_id)
                .await?
            {
                let args = serde_json::from_str(&call.function.arguments)
                    .unwrap_or_else(|_| serde_json::Value::String(call.function.arguments.clone()));
                persist_canonical_conversation_record(
                    &persistence_context,
                    &CanonicalConversationRecord::tool_request(
                        CanonicalToolRequestRecord::new(
                            call.function.name.clone(),
                            call.id.clone(),
                            request_id.to_string(),
                            approval_request_id.map(str::to_string),
                            args,
                            approval_request_id.is_some(),
                            if approval_request_id.is_some() {
                                Some("native runtime policy".to_string())
                            } else {
                                None
                            },
                            "native_runtime_backfill",
                        ),
                        &provenance,
                    ),
                )
                .await?;
            }
        }

        let status_label = match status {
            RuntimeToolResultStatus::Ok => ToolResultStatus::Ok,
            RuntimeToolResultStatus::Timeout => ToolResultStatus::Timeout,
            RuntimeToolResultStatus::Error => ToolResultStatus::Error,
        };
        let compacted = compact_client_tool_result(&ClientToolResultInput::new(
            tool_call_id.to_string(),
            tool_name.clone(),
            status_label,
            Some(content),
            serde_json::Value::Null,
            serde_json::Value::Null,
        ));
        persist_canonical_conversation_record(
            &persistence_context,
            &CanonicalConversationRecord::tool_result(
                CanonicalToolResultRecord::new(
                    tool_name,
                    tool_call_id.to_string(),
                    approval_request_id.map(str::to_string),
                    status_label,
                    compacted
                        .payload
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    compacted.payload.clone(),
                    serde_json::json!({
                        "component": "den.native_runtime",
                        "phase": "client_tool_result_recorded",
                    }),
                    Some(request_id.to_string()),
                ),
                &provenance,
            ),
        )
        .await?;
    }
    Ok(())
}

fn overflow_context(
    pool: PgPool,
    config: Arc<Config>,
    profile: BearProfile,
) -> AgentStepOverflowContext {
    AgentStepOverflowContext {
        pool,
        config,
        profile,
        session_store: SESSION_STORE.clone(),
    }
}

/// Shared dependencies for internal native profile turns (no full `DenState` required).
pub struct NativeRuntimeDeps<'a> {
    pub pool: &'a PgPool,
    pub config: &'a Config,
    pub stores: &'a MemoryStoreManager,
}

fn bear_id_from_native_binding(binding: &RoleRuntimeBinding) -> Option<Uuid> {
    let rest = binding.binding_id.strip_prefix("den-native:")?;
    let bear_id_str = rest.split(':').next()?;
    Uuid::parse_str(bear_id_str).ok()
}

pub struct NativeRuntimeConversationBackend {
    pool: Option<PgPool>,
}

impl Default for NativeRuntimeConversationBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeRuntimeConversationBackend {
    pub fn new() -> Self {
        Self { pool: None }
    }

    pub fn with_pool(pool: PgPool) -> Self {
        Self { pool: Some(pool) }
    }
}

#[allow(async_fn_in_trait)]
impl RuntimeConversationBackend for NativeRuntimeConversationBackend {
    async fn create_conversation(
        &self,
        binding: &RoleRuntimeBinding,
    ) -> Result<RuntimeConversationRef, DenError> {
        let id = format!("den-conv-{}", Uuid::new_v4().simple());
        if let Some(pool) = &self.pool {
            if let Some(bear_id) = bear_id_from_native_binding(binding) {
                conversation_persistence::ensure_conversation_for_external_id(
                    pool, bear_id, None, &id, None, None,
                )
                .await?;
            }
        }
        Ok(RuntimeConversationRef { id })
    }

    async fn verify_conversation_belongs_to_binding(
        &self,
        binding: &RoleRuntimeBinding,
        conversation_id: &str,
    ) -> Result<(), DenError> {
        let Some(pool) = &self.pool else {
            return Ok(());
        };
        let Some(bear_id) = bear_id_from_native_binding(binding) else {
            return Ok(());
        };
        let found = conversation_persistence::get_conversation_for_external_id(
            pool,
            bear_id,
            conversation_id,
        )
        .await?;
        if found.is_none() {
            return Err(DenError::ValidationError(format!(
                "conversation {conversation_id} does not belong to bear"
            )));
        }
        Ok(())
    }

    async fn load_history(
        &self,
        binding: &RoleRuntimeBinding,
        conversation: &RuntimeConversationRef,
    ) -> Result<RuntimeHistoryPage, DenError> {
        let Some(pool) = &self.pool else {
            return Ok(RuntimeHistoryPage {
                records: Vec::new(),
                raw_payload: None,
            });
        };
        let Some(bear_id) = bear_id_from_native_binding(binding) else {
            return Ok(RuntimeHistoryPage {
                records: Vec::new(),
                raw_payload: None,
            });
        };
        let Some(canonical) = conversation_persistence::get_conversation_for_external_id(
            pool,
            bear_id,
            &conversation.id,
        )
        .await?
        else {
            return Ok(RuntimeHistoryPage {
                records: Vec::new(),
                raw_payload: None,
            });
        };
        let rows =
            conversation_persistence::list_messages_page(pool, canonical.id, None, 100).await?;
        let mut records = Vec::new();
        for row in rows.into_iter().rev() {
            match row.to_model_transcript_record() {
                Some(PersistedTranscriptRecord::Message(message)) => {
                    records.push(RuntimeHistoryRecord::Message {
                        message_id: message.message_id,
                        role: message.role,
                        content: message.content,
                        created_at: Some(message.created_at.to_string()),
                    });
                }
                Some(PersistedTranscriptRecord::ToolCall {
                    tool_call_id,
                    tool_name,
                    arguments,
                    created_at,
                    ..
                }) => {
                    records.push(RuntimeHistoryRecord::ToolCall {
                        message_id: Some(tool_call_id.clone()),
                        tool_call_id,
                        tool_name,
                        arguments,
                        created_at: Some(created_at.to_string()),
                    });
                }
                Some(PersistedTranscriptRecord::ToolResult {
                    tool_call_id,
                    tool_name,
                    status,
                    content,
                    structured_content,
                    created_at,
                    ..
                }) => {
                    records.push(RuntimeHistoryRecord::ToolResult {
                        message_id: tool_call_id.clone(),
                        tool_call_id,
                        tool_name,
                        status,
                        content,
                        structured_content,
                        created_at: Some(created_at.to_string()),
                    });
                }
                None => {}
            }
        }
        Ok(RuntimeHistoryPage {
            records,
            raw_payload: None,
        })
    }
}

fn wrap_session_stream(
    stream: RuntimeEventStream,
    session: &AgentLoopSession,
    config: Arc<Config>,
    profile: BearProfile,
    pool: PgPool,
    bear_id: Uuid,
    user_id: Option<i32>,
    conversation_id: &str,
    client_session_id: &str,
    request_id: Option<String>,
    stores: MemoryStoreManager,
) -> RuntimeEventStream {
    Box::pin(SessionTrackingStream::new(
        stream,
        session,
        SESSION_STORE.clone(),
        pool,
        bear_id,
        session.bear_slug.clone(),
        user_id,
        conversation_id.to_string(),
        client_session_id.to_string(),
        request_id,
        config,
        stores,
        profile,
        NativeToolDispatchMode::DeferToClient,
    ))
}

async fn build_session(
    deps: &NativeRuntimeDeps<'_>,
    profile: NativeCapabilityProfile,
    bear_id: Uuid,
    conversation_id: &str,
    client_session_id: &str,
    human_message: Option<&str>,
    runtime_context: Option<&str>,
    session_id: Option<&str>,
    workspace_roots: Option<&[String]>,
    runtime_target: Option<&str>,
    conversation_selection: Option<&str>,
    user_id: Option<i32>,
    client_context: Option<&serde_json::Value>,
    client_tools: Option<&serde_json::Value>,
    request_id: Option<Uuid>,
    run_id: Option<&str>,
    stream_tokens: bool,
    api_style: Option<crate::llm::LlmApiStyle>,
    tool_messages: Vec<ChatMessage>,
) -> Result<AgentLoopSession, DenError> {
    let llm = LlmClient::new(deps.config);
    let bear = den_service::bears::db::get_bear(deps.pool, bear_id)
        .await?
        .ok_or_else(|| DenError::NotFound("bear not found".to_string()))?;
    let include_prompt_memory = profile.include_prompt_memory && runtime_context.is_none();
    let assembled = assemble_native_turn_for_bear(
        AssembleTurnContext {
            pool: deps.pool,
            config: deps.config,
            stores: deps.stores,
            bear_id,
            profile: profile.profile,
            conversation_id,
            turn_runtime_context: runtime_context,
            human_message,
            tool_messages: &tool_messages,
            session_id,
            workspace_roots,
            runtime_target,
            conversation_selection,
            user_id,
            client_context,
            include_prompt_memory,
            key_memory_cache: None,
            native_runtime: true,
        },
        &bear,
    )
    .await?;
    let key_memory_projection_cache_key = assembled
        .key_memory_projection
        .as_ref()
        .map(|projection| projection.cache_key.clone());
    let latest_projected_memory = assembled
        .key_memory_projection
        .as_ref()
        .map(projected_memory_session_diagnostic);
    let latest_recalled_memory = Some(recalled_memory_session_diagnostic(
        assembled.recall_diagnostic.as_ref(),
    ));
    let messages = assembled.messages;
    let budget_components = assembled.budget_components;
    let active_activity_plan = assembled.active_activity_plan;
    if profile.profile == BearProfile::Work && active_activity_plan.is_none() {
        return Err(DenError::ValidationError(
            "Work stance requires an active task list before execution can continue".to_string(),
        ));
    }
    let tools =
        merge_den_and_client_tools(deps.config, profile.profile, client_tools, human_message)?;
    let session_key = agent_loop_session_key(conversation_id, client_session_id);
    let conversation_model = match conversation_persistence::get_conversation_for_external_id(
        deps.pool,
        bear.id,
        conversation_id,
    )
    .await?
    {
        Some(conversation) => {
            conversation_persistence::resolve_conversation_selected_model(
                deps.pool,
                conversation.id,
            )
            .await?
        }
        None => None,
    };
    let model = if let Some(model) = conversation_model {
        model
    } else {
        den_service::bears::db::resolve_model_for_profile(
            deps.pool,
            &bear,
            profile.profile,
            llm.default_model(),
        )
        .await?
    };
    let model = llm.resolve_model(Some(&model));
    let model_option =
        den_service::model_selection::resolve_model_option(deps.pool, &model).await?;
    let bifrost_virtual_key = den_service::bears::db::bifrost_virtual_key_for_inference(
        deps.pool,
        bear.id,
        &deps.config.den_secret_encryption_key,
    )
    .await?;
    let (bear_loop_control_override, stance_loop_control_override) =
        den_service::bears::db::agent_loop_control_overrides_for_profile(
            deps.pool,
            bear.id,
            profile.profile,
        )
        .await?;
    let agent_loop_control = resolve_agent_loop_control(AgentLoopControlResolutionInput {
        model_handle: Some(&model),
        model_default: None,
        bear_override: bear_loop_control_override,
        stance_override: stance_loop_control_override,
        task_escalation: None,
    });
    let agent_loop_control = crate::agent_loop::ResolvedAgentLoopControl {
        profile: agent_loop_control.profile.with_budget(profile.turn_budget),
        ..agent_loop_control
    };
    tracing::info!(
        bear_id = %bear_id,
        profile = %profile.profile.as_str(),
        conversation_id,
        client_session_id,
        model = %model,
        agent_loop_control_level = %agent_loop_control.level.as_str(),
        agent_loop_control_source = ?agent_loop_control.source,
        "resolved native agent loop control profile"
    );
    let session = AgentLoopSession {
        session_key,
        bear_id,
        bear_slug: bear.slug.clone(),
        user_id,
        conversation_id: conversation_id.to_string(),
        client_session_id: client_session_id.to_string(),
        workspace_roots: workspace_roots
            .map(|items| items.to_vec())
            .unwrap_or_default(),
        request_id: request_id.map(|id| id.to_string()),
        run_id: run_id.map(str::to_string),
        messages,
        tools,
        budget_components,
        model,
        model_context_window: model_option
            .as_ref()
            .and_then(|option| option.context_window),
        model_max_output_tokens: model_option
            .as_ref()
            .and_then(|option| option.max_output_tokens),
        bifrost_virtual_key,
        api_style,
        step: 0,
        turn_budget: agent_loop_control.profile.budget,
        turn_budget_state: Default::default(),
        agent_loop_control,
        checkpoint_state: Default::default(),
        pending_checkpoint_request: None,
        pending_checkpoint_task_action: None,
        strategy: profile.strategy,
        stream_tokens,
        key_memory_projection_cache_key,
        latest_context_budget: None,
        latest_projected_memory,
        latest_recalled_memory,
        active_activity_plan,
        profile: profile.profile,
        overflow_retry_attempted: false,
        overflow_compaction_recovered: false,
    };
    SESSION_STORE.insert(session.clone());
    Ok(session)
}

pub async fn run_native_profile_turn_collect_assistant_text(
    deps: &NativeRuntimeDeps<'_>,
    bear_id: Uuid,
    role: BearProfile,
    conversation_id: &str,
    session_id: &str,
    prompt: &str,
) -> Result<String, DenError> {
    let profile = NativeCapabilityProfile::for_profile(role);
    let session = build_session(
        deps,
        profile,
        bear_id,
        conversation_id,
        session_id,
        Some(prompt),
        None,
        Some(session_id),
        None,
        Some(conversation_id),
        None,
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        Vec::new(),
    )
    .await?;
    let llm = LlmClient::new(deps.config);
    let overflow = overflow_context(deps.pool.clone(), Arc::new(deps.config.clone()), role);
    let mut stream = run_agent_step_stream(&llm, &session, Some(overflow)).await?;
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        if let RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta {
            text: delta,
        }) = item?
        {
            text.push_str(&delta);
        }
    }
    Ok(text)
}

pub struct NativeWebChatTurnParams<'a> {
    pub deps: &'a NativeRuntimeDeps<'a>,
    pub bear_id: Uuid,
    pub bear_slug: &'a str,
    pub chat_binding_id: &'a str,
    pub user_id: i32,
    pub username: Option<&'a str>,
    pub membership_role: Option<&'a str>,
    pub conversation_id: &'a str,
    pub session_id: &'a str,
    pub prompt: &'a str,
    pub request_id: Uuid,
    /// Concrete builtin-tool dispatcher injected by the `den` binary (the
    /// `DenToolContext` aggregate lives there, not in `den-runtime`).
    pub tool_invoker: Arc<dyn super::RuntimeToolInvoker>,
}

/// Browser web chat turn (`BearProfile::Chat`) over the native in-process loop.
pub async fn start_native_web_chat_turn_event_stream(
    params: NativeWebChatTurnParams<'_>,
) -> Result<RuntimeEventStream, DenError> {
    let assembly_started = std::time::Instant::now();
    let profile = NativeCapabilityProfile::for_profile(BearProfile::Chat);
    let session = build_session(
        params.deps,
        profile,
        params.bear_id,
        params.conversation_id,
        params.session_id,
        Some(params.prompt),
        None,
        Some(params.session_id),
        None,
        Some(params.conversation_id),
        Some(params.conversation_id),
        Some(params.user_id),
        None,
        None,
        Some(params.request_id),
        None,
        true,
        None,
        Vec::new(),
    )
    .await?;
    tracing::info!(
        request_id = %params.request_id,
        bear_id = %params.bear_id,
        conversation_id = %params.conversation_id,
        message_count = session.messages.len(),
        tool_count = session.tools.len(),
        assembly_ms = assembly_started.elapsed().as_millis(),
        "native web chat turn assembled"
    );
    let llm = LlmClient::new(params.deps.config);
    let overflow = overflow_context(
        params.deps.pool.clone(),
        Arc::new(params.deps.config.clone()),
        BearProfile::Chat,
    );
    let stream = run_agent_step_stream(&llm, &session, Some(overflow)).await?;
    let runtime = NativeWebChatLoopRuntime {
        pool: params.deps.pool.clone(),
        config: Arc::new(params.deps.config.clone()),
        stores: params.deps.stores.clone(),
        llm,
        session_key: session.session_key.clone(),
        bear_id: params.bear_id,
        bear_slug: params.bear_slug.to_string(),
        chat_binding_id: params.chat_binding_id.to_string(),
        user_id: params.user_id,
        username: params.username.map(str::to_string),
        membership_role: params.membership_role.map(str::to_string),
        conversation_id: params.conversation_id.to_string(),
        session_id: params.session_id.to_string(),
        request_id: params.request_id.to_string(),
        session_store: SESSION_STORE.clone(),
        tool_invoker: params.tool_invoker.clone(),
    };
    let step_stream = NativeWebChatLoopStream::wrap_step_stream(&runtime, stream, &session);
    let turn_start_message_len = session.messages.len();
    Ok(Box::pin(NativeWebChatLoopStream::new(
        runtime,
        step_stream,
        turn_start_message_len,
    )))
}

pub async fn start_native_client_turn_event_stream(
    request: TurnStartRequest<'_>,
) -> Result<RuntimeEventStream, DenError> {
    start_native_profile_turn_event_stream(request, BearProfile::Pair).await
}

pub async fn start_native_profile_turn_event_stream(
    request: TurnStartRequest<'_>,
    role: BearProfile,
) -> Result<RuntimeEventStream, DenError> {
    let profile = NativeCapabilityProfile::for_profile(role);
    let runtime_conversations =
        NativeRuntimeConversationBackend::with_pool(request.sqlx_pool.clone());
    let materialized =
        materialize_runtime_conversation_if_needed(&runtime_conversations, &request).await?;
    let conversation_id = materialized.conversation_id;
    let client_session_id = request.session_id;
    let workspace_roots = request
        .workspace_roots
        .map(|roots| roots.to_vec())
        .filter(|roots| !roots.is_empty())
        .or_else(|| request.cwd.map(|cwd| vec![cwd.to_string()]));
    let prompt_for_model = prompt_for_model(request.prompt, request.prompt_context.as_ref());
    let session = build_session(
        &NativeRuntimeDeps {
            pool: request.sqlx_pool,
            config: request.config,
            stores: request.memory_stores,
        },
        profile,
        request.bear_id,
        &conversation_id,
        client_session_id,
        Some(prompt_for_model.as_str()),
        request.runtime_context,
        Some(client_session_id),
        workspace_roots.as_deref(),
        Some(request.upstream_target),
        Some(request.conversation_selection),
        Some(request.user_id),
        None,
        request.client_tools.as_ref(),
        Some(request.request_id),
        request.run_id,
        request.stream_tokens,
        request.api_style,
        Vec::new(),
    )
    .await?;
    if request.runtime_context.is_none() {
        let provenance = ConversationEventProvenance::client_session(client_session_id.to_string());
        let mut content_json = provenance.as_content_json("user_prompt");
        content_json["role"] = serde_json::json!("user");
        content_json["client_session_id"] = serde_json::json!(client_session_id);
        content_json["client"] = serde_json::json!(request.client);
        content_json["request_id"] = serde_json::json!(request.request_id.to_string());
        if let Some(prompt_context) = request.prompt_context.clone() {
            content_json["prompt_context"] = prompt_context.clone();
            if let Some(host_context) = prompt_context.get("host_context") {
                content_json["host_context"] = host_context.clone();
            }
        }
        let record =
            CanonicalConversationRecord::visible_user_message(request.prompt, content_json, None);
        persist_canonical_conversation_record(
            &canonical_persistence_context(
                request.sqlx_pool.clone(),
                request.bear_id,
                Some(request.user_id),
                conversation_id.clone(),
                Some(client_session_id.to_string()),
                Some(request.request_id.to_string()),
                client_session_id.to_string(),
                false,
            ),
            &record,
        )
        .await?;
    }
    let llm = LlmClient::new(request.config);
    let overflow = overflow_context(
        request.sqlx_pool.clone(),
        Arc::new(request.config.clone()),
        role,
    );
    let stream = run_agent_step_stream(&llm, &session, Some(overflow)).await?;
    let stream = wrap_session_stream(
        stream,
        &session,
        Arc::new(request.config.clone()),
        role,
        request.sqlx_pool.clone(),
        request.bear_id,
        Some(request.user_id),
        &conversation_id,
        client_session_id,
        Some(request.request_id.to_string()),
        request.memory_stores.clone(),
    );
    let _ = StartTurnRequest {
        conversation: RuntimeConversationRef {
            id: conversation_id,
        },
        binding: request.binding.clone(),
        human_message: request.prompt.to_string(),
        runtime_context: request.runtime_context.map(str::to_string),
        client_session_id: Some(client_session_id.to_string()),
        client_tools: request.client_tools.clone(),
        stream_tokens: request.stream_tokens,
    };
    Ok(stream)
}

pub async fn continue_native_profile_turn_event_stream(
    request: TurnContinueRequest<'_>,
    role: BearProfile,
) -> Result<(RuntimeStreamContinuation, RuntimeEventStream), DenError> {
    continue_native_client_turn_event_stream(request, role).await
}

fn find_pending_tool_call(session: &AgentLoopSession, tool_call_id: &str) -> Option<ChatToolCall> {
    session
        .messages
        .iter()
        .rev()
        .filter_map(|message| message.tool_calls.as_ref())
        .flatten()
        .find(|call| call.id == tool_call_id)
        .cloned()
}

fn tool_observation_from_call(
    call: &ChatToolCall,
    content: Option<&str>,
) -> ToolContinuationObservation {
    ToolContinuationObservation {
        tool_name: call.function.name.clone(),
        signature: tool_signature_from_call(call),
        class: classify_tool_budget_class(&call.function.name),
        failed: tool_result_content_indicates_error(content),
    }
}

fn continuation_budget_stop(
    reason: TurnBudgetStopReason,
) -> (RuntimeStreamContinuation, RuntimeEventStream) {
    let stream: RuntimeEventStream = Box::pin(stream::iter(vec![Ok(
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed {
            turn: None,
            category: RuntimeErrorCategory::Internal,
            message: reason.user_message(),
        }),
    )]));
    (RuntimeStreamContinuation::Deferred, stream)
}

const BUDGET_WARNING_PREFIX: &str = "Budget advisory:";
const CHECKPOINT_NUDGE_PREFIX: &str = "Runtime checkpoint required:";

fn budget_warning_runtime_event(warning: &TurnBudgetWarning) -> RuntimeStreamEvent {
    RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress {
        kind: "turn_budget_warning".to_string(),
        text: Some(warning.model_message().to_string()),
        phase: Some("budget".to_string()),
        detail: Some(serde_json::json!({
            "code": warning.code,
        })),
    })
}

fn checkpoint_trigger_runtime_event(trigger: &CheckpointTrigger) -> RuntimeStreamEvent {
    RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress {
        kind: "runtime_checkpoint_would_trigger".to_string(),
        text: Some(trigger.message.clone()),
        phase: Some("agent_loop_control".to_string()),
        detail: Some(serde_json::json!({
            "reason": trigger.reason,
            "mode": "observe_only",
        })),
    })
}

fn runtime_checkpoint_request_for_trigger(
    session: &AgentLoopSession,
    trigger: &CheckpointTrigger,
) -> Option<RuntimeCheckpointRequest> {
    let run_id = session.run_id.clone()?;
    Some(RuntimeCheckpointRequest {
        checkpoint_id: format!(
            "ckpt-{}-{}",
            session.step.saturating_add(1),
            trigger.reason.as_str()
        ),
        run_id,
        reason: trigger.reason,
        control_level: session.agent_loop_control.level,
        active_objective: active_checkpoint_objective(session),
        task_context: checkpoint_task_context(session),
        evidence_refs: Vec::new(),
        required_fields: vec![
            CheckpointField::ActiveObjective,
            CheckpointField::Learned,
            CheckpointField::NextAction,
        ],
    })
}

fn active_checkpoint_objective(session: &AgentLoopSession) -> Option<String> {
    session
        .active_activity_plan
        .as_ref()
        .and_then(|plan| {
            plan.current_item.as_ref().or_else(|| {
                plan.items.iter().find(|item| {
                    matches!(
                        item.status,
                        den_docket::TaskListItemStatus::Pending
                            | den_docket::TaskListItemStatus::InProgress
                    )
                })
            })
        })
        .map(|item| item.title.clone())
}

fn checkpoint_task_context(session: &AgentLoopSession) -> Option<CheckpointTaskContext> {
    let plan = session.active_activity_plan.as_ref()?;
    let active_item = plan.current_item.as_ref().or_else(|| {
        plan.items.iter().find(|item| {
            matches!(
                item.status,
                den_docket::TaskListItemStatus::Pending
                    | den_docket::TaskListItemStatus::InProgress
            )
        })
    });
    Some(CheckpointTaskContext {
        task_list_id: Some(plan.id.to_string()),
        task_list_version: None,
        active_item_id: active_item.map(|item| item.id.to_string()),
        active_item_title: active_item.map(|item| item.title.clone()),
        docket_job_id: plan.source_ref.docket_job_id.clone(),
        docket_task_id: active_item.and_then(|item| item.source_ref.docket_task_id.clone()),
    })
}

fn agent_loop_control_observe_enabled(config: &Config) -> bool {
    matches!(
        config.agent_loop_control_mode.as_str(),
        "observe" | "enforce"
    )
}

fn agent_loop_control_enforce_enabled(config: &Config) -> bool {
    config.agent_loop_control_mode == "enforce"
}

fn checkpoint_audit_enabled_for_session(config: &Config, session: &AgentLoopSession) -> bool {
    match config.checkpoint_audit_mode.as_str() {
        "all" => true,
        "work" => session.profile == BearProfile::Work,
        _ => false,
    }
}

fn render_checkpoint_nudge(request: &RuntimeCheckpointRequest, trigger: &CheckpointTrigger) -> String {
    let request_json = serde_json::to_string_pretty(request)
        .unwrap_or_else(|_| "{\"error\":\"checkpoint_request_unavailable\"}".to_string());
    format!(
        "{CHECKPOINT_NUDGE_PREFIX} {}\n\nReturn ONLY a JSON checkpoint response before more exploratory or risky tool use. Do not put prose in `next_action`. `next_action` must be one of: `call_tool`, `edit`, `validate`, `update_task_list`, `sync_task_list`, `request_handoff`, `final_if_gate_allows`, or `stop_blocked`. If you choose `call_tool`, you may use an object like {{\"action\":\"call_tool\",\"tool_name\":\"memory_read\"}}.\n\nMinimum response shape:\n```json\n{{\n  \"checkpoint_id\": \"<copy from request>\",\n  \"active_objective\": \"one sentence\",\n  \"learned\": [\"fact with evidence\"],\n  \"remaining_uncertainty\": [],\n  \"more_exploration_justified\": false,\n  \"next_action\": \"edit\",\n  \"task_state_change_needed\": null,\n  \"evidence_refs\": [],\n  \"confidence\": \"medium\"\n}}\n```\n\nIf task state should change, call the appropriate task-management tool after the checkpoint; do not treat checkpoint prose as task state.\n\nCheckpoint request:\n```json\n{request_json}\n```",
        trigger.message
    )
}

fn apply_checkpoint_nudge(
    session: &mut AgentLoopSession,
    request: &RuntimeCheckpointRequest,
    trigger: &CheckpointTrigger,
) -> bool {
    let message = render_checkpoint_nudge(request, trigger);
    if session.messages.last().is_some_and(|message| {
        message.role == "system"
            && message
                .content
                .as_deref()
                .is_some_and(|content| content.starts_with(CHECKPOINT_NUDGE_PREFIX))
    }) {
        session.messages.pop();
    }
    session.pending_checkpoint_request = Some(request.clone());
    session.messages.push(ChatMessage {
        role: "system".to_string(),
        content: Some(message),
        tool_call_id: None,
        name: None,
        tool_calls: None,
    });
    true
}

async fn record_checkpoint_request_if_audited(
    pool: &PgPool,
    config: &Config,
    session: &AgentLoopSession,
    trigger: &CheckpointTrigger,
) {
    if !checkpoint_audit_enabled_for_session(config, session) {
        return;
    }
    let Some(request) = runtime_checkpoint_request_for_trigger(session, trigger) else {
        tracing::debug!(
            session_id = %session.client_session_id,
            reason = %trigger.reason.as_str(),
            "skipping work checkpoint artifact because run id is unavailable"
        );
        return;
    };
    if let Err(err) = record_checkpoint_request(
        pool,
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
            session_id = %session.client_session_id,
            run_id = session.run_id.as_deref().unwrap_or("<none>"),
            reason = %trigger.reason.as_str(),
            "failed to record observe-only checkpoint artifact"
        );
    }
}

fn apply_budget_warning(session: &mut AgentLoopSession, warning: &TurnBudgetWarning) -> bool {
    if session.messages.last().is_some_and(|message| {
        message.role == "system" && message.content.as_deref() == Some(warning.model_message())
    }) {
        return false;
    }
    if session.messages.last().is_some_and(|message| {
        message.role == "system"
            && message
                .content
                .as_deref()
                .is_some_and(|content| content.starts_with(BUDGET_WARNING_PREFIX))
    }) {
        session.messages.pop();
    }
    session.messages.push(ChatMessage {
        role: "system".to_string(),
        content: Some(warning.model_message().to_string()),
        tool_call_id: None,
        name: None,
        tool_calls: None,
    });
    true
}

fn call_is_den_web_fetch(call: &ChatToolCall) -> bool {
    builtin_den_tool_descriptor_for_provider_name(&call.function.name)
        .is_some_and(|descriptor| descriptor.name == DEN_WEB_FETCH)
}

fn normalize_approved_web_url(raw: &str) -> Result<String, DenError> {
    let mut url = url::Url::parse(raw.trim())
        .map_err(|err| DenError::ValidationError(format!("web_fetch url is invalid: {err}")))?;
    match url.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(DenError::ValidationError(
                "web_fetch url scheme must be http or https".to_string(),
            ));
        }
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

async fn record_web_fetch_url_approval(
    pool: &PgPool,
    bear_id: Uuid,
    user_id: Option<i32>,
    call: &ChatToolCall,
) -> Result<(), DenError> {
    let args: serde_json::Value = serde_json::from_str(&call.function.arguments)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
    let raw_url = args
        .get("url")
        .and_then(|value| value.as_str())
        .ok_or_else(|| DenError::ValidationError("web_fetch args missing url".to_string()))?;
    let normalized_url = normalize_approved_web_url(raw_url)?;
    sqlx::query(
        r"
        INSERT INTO bear_web_approvals (bear_id, scope_kind, scope_value, approved_by_user_id, source, expires_at)
        VALUES ($1, 'url', $2, $3, 'acp', now() + interval '1 hour')
        ON CONFLICT (bear_id, scope_kind, scope_value) WHERE revoked_at IS NULL
        DO UPDATE SET approved_by_user_id = EXCLUDED.approved_by_user_id,
                      source = EXCLUDED.source,
                      expires_at = EXCLUDED.expires_at
        ",
    )
    .bind(bear_id)
    .bind(normalized_url)
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|err| DenError::Database(format!("record web_fetch approval: {err}")))?;
    Ok(())
}

async fn execute_approved_den_tool_for_session(
    request: &TurnContinueRequest<'_>,
    session: &AgentLoopSession,
    call: &ChatToolCall,
    profile: BearProfile,
) -> Result<ChatMessage, DenError> {
    if call_is_den_web_fetch(call) {
        record_web_fetch_url_approval(request.sqlx_pool, session.bear_id, session.user_id, call)
            .await?;
    }
    let Some(invoker) = super::tool_invoker() else {
        return Err(DenError::System(
            "builtin Den tool runtime is not initialized".to_string(),
        ));
    };
    let canonical = builtin_den_tool_descriptor_for_provider_name(&call.function.name)
        .map(|descriptor| descriptor.name.to_string())
        .unwrap_or_else(|| call.function.name.clone());
    let args = serde_json::from_str(&call.function.arguments)
        .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
    let context = DenToolInvocationContext {
        bear_id: session.bear_id,
        bear_slug: session.bear_slug.clone(),
        binding_id: request.binding.binding_id.clone(),
        profile: Some(profile),
        user_id: session.user_id.unwrap_or_default(),
        username: None,
        membership_role: None,
        conversation_id: session.conversation_id.clone(),
        session_id: session.client_session_id.clone(),
        client_session_id: Some(session.client_session_id.clone()),
        conversation_selection: Some(session.conversation_id.clone()),
        runtime_target: Some(session.conversation_id.clone()),
        workspace_roots: session.workspace_roots.clone(),
        session_policy: None,
        activity: session
            .active_activity_plan
            .as_ref()
            .and_then(|plan| serde_json::to_value(plan).ok()),
        runtime: Some(session.session_info_runtime_snapshot()),
        context_budget: session
            .latest_context_budget
            .as_ref()
            .and_then(|budget| serde_json::to_value(budget).ok()),
        projected_memory: session.latest_projected_memory.clone(),
        recalled_memory: session.latest_recalled_memory.clone(),
        request_id: Some(request.request_id.to_string()),
        channel: DenToolChannelContext {
            family: Some("armature".to_string()),
            client: Some("bearwire".to_string()),
            protocol: Some("bearwire".to_string()),
        },
    };
    let content = match invoker
        .invoke(
            request.sqlx_pool,
            request.config,
            request.memory_stores,
            &canonical,
            args,
            context,
        )
        .await
    {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
        Err(err) => format!("error: {err}"),
    };
    Ok(ChatMessage {
        role: "tool".to_string(),
        content: Some(content),
        tool_call_id: Some(call.id.clone()),
        name: Some(call.function.name.clone()),
        tool_calls: None,
    })
}

pub async fn continue_native_client_turn_event_stream(
    request: TurnContinueRequest<'_>,
    profile: BearProfile,
) -> Result<(RuntimeStreamContinuation, RuntimeEventStream), DenError> {
    let client_session_id = request.client_session_id;
    let conversation_id = request.conversation.id.clone();
    let session_key = agent_loop_session_key(&conversation_id, client_session_id);
    let existing_session = SESSION_STORE.get(&session_key);
    let prior_session = existing_session
        .clone()
        .ok_or_else(|| DenError::System("native agent loop session not found".to_string()))?;
    let mut tool_messages = Vec::new();
    let mut observations = Vec::new();
    match &request.continuation {
        RuntimeContinuation::ToolResult {
            tool_call_id,
            approval_request_id,
            status,
            content,
        } => {
            if let Some(approval_request_id) = approval_request_id.as_deref() {
                let approve = matches!(status, den_protocol::RuntimeToolResultStatus::Ok);
                record_approval_decision(
                    request.sqlx_pool,
                    approval_request_id,
                    approve,
                    Some(if approve {
                        "tool_result_delivered"
                    } else {
                        "tool_result_failed"
                    }),
                )
                .await?;
            }
            let pending_call = find_pending_tool_call(&prior_session, tool_call_id);
            if let Some(call) = pending_call.as_ref() {
                observations.push(tool_observation_from_call(call, Some(content)));
            }
            tool_messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(content.clone()),
                tool_call_id: Some(tool_call_id.clone()),
                name: pending_call.map(|call| call.function.name),
                tool_calls: None,
            });
        }
        RuntimeContinuation::ApprovalDecision {
            approval_request_id,
            tool_call_id,
            decision,
            reason,
        } => {
            let approve = matches!(decision, den_protocol::RuntimeApprovalDecision::Approve);
            record_approval_decision(
                request.sqlx_pool,
                approval_request_id,
                approve,
                reason.as_deref(),
            )
            .await?;
            // Approval is control-plane state. Armature-local approvals wait for the
            // armature to execute the tool and send RuntimeContinuation::ToolResult;
            // Den-hosted web_fetch approvals execute server-side immediately.
            if approve {
                if let Some(session) = existing_session.as_ref() {
                    if let Some(tool_call_id) = tool_call_id.as_deref() {
                        if let Some(call) = find_pending_tool_call(session, tool_call_id) {
                            if call_is_den_web_fetch(&call) {
                                let tool_message = execute_approved_den_tool_for_session(
                                    &request, session, &call, profile,
                                )
                                .await?;
                                observations.push(tool_observation_from_call(
                                    &call,
                                    tool_message.content.as_deref(),
                                ));
                                tool_messages.push(tool_message);
                            }
                        }
                    }
                }
            } else {
                let content = reason.clone().unwrap_or_else(|| "denied".to_string());
                let pending_call = tool_call_id
                    .as_deref()
                    .and_then(|id| find_pending_tool_call(&prior_session, id));
                if let Some(call) = pending_call.as_ref() {
                    observations.push(tool_observation_from_call(call, Some(&content)));
                }
                tool_messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(content),
                    tool_call_id: tool_call_id.clone(),
                    name: pending_call.map(|call| call.function.name),
                    tool_calls: None,
                });
            }
        }
    }
    let evaluation = evaluate_turn_budget(
        prior_session.turn_budget,
        prior_session.step,
        prior_session
            .turn_budget_state
            .started_at
            .elapsed()
            .as_millis() as u64,
        &prior_session.turn_budget_state,
        &observations,
    );
    let checkpoint_evaluation = evaluate_checkpoint_trigger(
        &prior_session.agent_loop_control.profile,
        &prior_session.checkpoint_state,
        &observations,
        evaluation.warning.is_some(),
    );
    let mut warning_applied = false;
    SESSION_STORE.update(&session_key, |session| {
        session.request_id = Some(request.request_id.to_string());
        session.run_id = request
            .run_id
            .map(str::to_string)
            .or_else(|| session.run_id.clone());
        session.messages.extend(tool_messages.clone());
        session.turn_budget_state = evaluation.next_state.clone();
        session.checkpoint_state = checkpoint_evaluation.next_state.clone();
        if let Some(warning) = evaluation.warning.as_ref() {
            warning_applied = apply_budget_warning(session, warning);
        }
    });
    let mut session = SESSION_STORE
        .get(&session_key)
        .ok_or_else(|| DenError::System("native agent loop session not found".to_string()))?;
    if let Some(trigger) = checkpoint_evaluation.trigger.as_ref() {
        record_checkpoint_request_if_audited(request.sqlx_pool, request.config, &session, trigger)
            .await;
        if agent_loop_control_enforce_enabled(request.config) {
            if let Some(checkpoint_request) = runtime_checkpoint_request_for_trigger(&session, trigger)
            {
                SESSION_STORE.update(&session_key, |session| {
                    apply_checkpoint_nudge(session, &checkpoint_request, trigger);
                });
                session = SESSION_STORE.get(&session_key).ok_or_else(|| {
                    DenError::System("native agent loop session not found after checkpoint nudge".to_string())
                })?;
            }
        }
    }
    if let Some(reason) = evaluation.stop_reason {
        return Ok(continuation_budget_stop(reason));
    }
    let llm = LlmClient::new(request.config);
    let overflow = overflow_context(
        request.sqlx_pool.clone(),
        Arc::new(request.config.clone()),
        profile,
    );
    let stream = run_agent_step_stream(&llm, &session, Some(overflow)).await?;
    let mut prefix_events = Vec::new();
    if warning_applied {
        let warning_event = evaluation
            .warning
            .as_ref()
            .map(budget_warning_runtime_event)
            .expect("warning_applied requires warning payload");
        prefix_events.push(Ok(warning_event));
    }
    if agent_loop_control_observe_enabled(request.config) {
        if let Some(trigger) = checkpoint_evaluation.trigger.as_ref() {
            prefix_events.push(Ok(checkpoint_trigger_runtime_event(trigger)));
        }
    }
    let stream = if prefix_events.is_empty() {
        stream
    } else {
        Box::pin(stream::iter(prefix_events).chain(stream)) as RuntimeEventStream
    };
    let stream = wrap_session_stream(
        stream,
        &session,
        Arc::new(request.config.clone()),
        profile,
        request.sqlx_pool.clone(),
        session.bear_id,
        session.user_id,
        &conversation_id,
        client_session_id,
        Some(request.request_id.to_string()),
        request.memory_stores.clone(),
    );
    let _ = ContinueTurnRequest {
        conversation: request.conversation,
        turn: None,
        binding: request.binding.clone(),
        continuation: request.continuation,
    };
    Ok((RuntimeStreamContinuation::Deferred, stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_loop::{
        resolve_agent_loop_control, AgentLoopControlResolutionInput,
        PostMutationVerificationWindow, StrategyProfile, ToolCallBudgetLimits, TurnBudgetPolicy,
    };

    fn sample_budget_warning(message: &str) -> TurnBudgetWarning {
        TurnBudgetWarning {
            code: "tool_class_budget_warning",
            message: message.to_string(),
        }
    }

    fn pair_turn_budget() -> TurnBudgetPolicy {
        TurnBudgetPolicy {
            max_wall_clock_ms: 60_000,
            emergency_hard_steps: 8,
            tool_call_limits: ToolCallBudgetLimits {
                total: 8,
                read: 8,
                search: 8,
                fetch: 8,
                execute: 8,
                write: 8,
                destructive: 2,
                other: 8,
            },
            max_consecutive_tool_failures: 3,
            max_same_tool_signature_repeats: 2,
            post_mutation_verification_window: Some(PostMutationVerificationWindow {
                replenish_read: 2,
                replenish_search: 1,
            }),
        }
    }

    fn test_agent_loop_control() -> crate::agent_loop::ResolvedAgentLoopControl {
        resolve_agent_loop_control(AgentLoopControlResolutionInput {
            model_handle: Some("openai/test"),
            model_default: None,
            bear_override: None,
            stance_override: None,
            task_escalation: None,
        })
    }

    #[test]
    fn bear_id_from_native_binding_parses_den_native_format() {
        let bear_id = Uuid::new_v4();
        let binding = RoleRuntimeBinding {
            binding_id: format!("den-native:{bear_id}:pair"),
            compatibility_backend: Some("runtime:native".to_string()),
        };
        assert_eq!(bear_id_from_native_binding(&binding), Some(bear_id));
    }

    #[test]
    fn bear_id_from_native_binding_rejects_non_native_bindings() {
        let binding = RoleRuntimeBinding {
            binding_id: "legacy-provider-123".to_string(),
            compatibility_backend: Some("runtime:legacy".to_string()),
        };
        assert_eq!(bear_id_from_native_binding(&binding), None);
    }

    #[test]
    fn apply_budget_warning_reports_when_message_changes() {
        let mut session = AgentLoopSession {
            session_key: "session".to_string(),
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            user_id: Some(1),
            conversation_id: "conv".to_string(),
            client_session_id: "session".to_string(),
            workspace_roots: vec![],
            request_id: None,
            run_id: None,
            messages: vec![],
            tools: vec![],
            budget_components: Default::default(),
            model: "openai/test".to_string(),
            model_context_window: None,
            model_max_output_tokens: None,
            bifrost_virtual_key: None,
            api_style: None,
            step: 0,
            turn_budget: pair_turn_budget(),
            turn_budget_state: Default::default(),
            agent_loop_control: test_agent_loop_control(),
            checkpoint_state: Default::default(),
            pending_checkpoint_request: None,
            pending_checkpoint_task_action: None,
            strategy: StrategyProfile::plain_react(),
            stream_tokens: false,
            key_memory_projection_cache_key: None,
            latest_context_budget: None,
            latest_projected_memory: None,
            latest_recalled_memory: None,
            active_activity_plan: None,
            profile: BearProfile::Pair,
            overflow_retry_attempted: false,
            overflow_compaction_recovered: false,
        };

        let warning = sample_budget_warning("Budget advisory: close to read budget.");
        assert!(apply_budget_warning(&mut session, &warning));
        assert!(!apply_budget_warning(&mut session, &warning));

        let replacement = sample_budget_warning("Budget advisory: next read will stop the turn.");
        assert!(apply_budget_warning(&mut session, &replacement));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.as_deref(),
            Some(replacement.model_message())
        );
    }

    #[test]
    fn checkpoint_nudge_renders_structured_request_without_task_state_mutation() {
        let request = RuntimeCheckpointRequest {
            checkpoint_id: "ckpt-1".to_string(),
            run_id: "run-1".to_string(),
            reason: crate::agent_loop::CheckpointReason::OverExploration,
            control_level: den_core::AgentLoopControlLevel::Careful,
            active_objective: Some("Inspect routing".to_string()),
            task_context: None,
            evidence_refs: Vec::new(),
            required_fields: vec![CheckpointField::Learned, CheckpointField::NextAction],
        };
        let trigger = CheckpointTrigger {
            reason: crate::agent_loop::CheckpointReason::OverExploration,
            message: "summarize before more reads".to_string(),
        };

        let rendered = render_checkpoint_nudge(&request, &trigger);

        assert!(rendered.starts_with(CHECKPOINT_NUDGE_PREFIX));
        assert!(rendered.contains("Return ONLY a JSON checkpoint response"));
        assert!(rendered.contains("\"checkpoint_id\": \"ckpt-1\""));
        assert!(rendered.contains("do not treat checkpoint prose as task state"));
        assert!(rendered.contains("`next_action` must be one of"));
    }

    #[test]
    fn budget_warning_runtime_event_is_user_visible_progress_not_error() {
        let warning = sample_budget_warning("Budget advisory: next read will stop the turn.");
        let event = budget_warning_runtime_event(&warning);

        match event {
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::RunProgress {
                kind,
                text,
                phase,
                detail: Some(detail),
            }) => {
                assert_eq!(kind, "turn_budget_warning");
                assert_eq!(text.as_deref(), Some(warning.model_message()));
                assert_eq!(phase.as_deref(), Some("budget"));
                assert_eq!(detail["code"].as_str(), Some(warning.code));
            }
            other => panic!("expected RunProgress budget warning, got {other:?}"),
        }
    }

    #[test]
    fn prompt_for_model_wraps_human_prompt_with_structured_host_context() {
        let prompt = prompt_for_model(
            "Please inspect this.",
            Some(&serde_json::json!({
                "format": "acp_prompt_context.v1",
                "host_context": {
                    "kind": "referenced_resources",
                    "delivery": "reference_only",
                    "persistence": "not_human_message",
                    "resources": [
                        {
                            "label": "src/lib.rs",
                            "uri": "file:///workspace/src/lib.rs",
                            "mime_type": "text/rust",
                            "embedded_text_bytes": 128
                        }
                    ]
                }
            })),
        );

        assert!(prompt.contains("<host_context kind=\"referenced_resources\""));
        assert!(prompt.contains("file:///workspace/src/lib.rs"));
        assert!(prompt.contains("embedded_text_bytes: 128 (body omitted"));
        assert!(prompt.contains("<user_message>\nPlease inspect this.\n</user_message>"));
    }

    #[tokio::test]
    async fn hard_step_continuation_returns_terminal_event_not_error() {
        let conversation_id = format!("transient-{}", Uuid::new_v4().simple());
        let client_session_id = format!("session-{}", Uuid::new_v4().simple());
        let session_key = agent_loop_session_key(&conversation_id, &client_session_id);
        let config = Config::test_stub();
        let stores = MemoryStoreManager::new(&config);
        let pool = PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/unused")
            .expect("lazy pool");
        SESSION_STORE.insert(AgentLoopSession {
            session_key: session_key.clone(),
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            user_id: Some(1),
            conversation_id: conversation_id.clone(),
            client_session_id: client_session_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            request_id: None,
            run_id: Some("run-max-step".to_string()),
            messages: Vec::new(),
            tools: Vec::new(),
            budget_components: Default::default(),
            model: "openai/test".to_string(),
            model_context_window: None,
            model_max_output_tokens: None,
            bifrost_virtual_key: None,
            api_style: None,
            step: 8,
            turn_budget: pair_turn_budget(),
            turn_budget_state: Default::default(),
            agent_loop_control: test_agent_loop_control(),
            checkpoint_state: Default::default(),
            pending_checkpoint_request: None,
            pending_checkpoint_task_action: None,
            strategy: StrategyProfile::plain_react(),
            stream_tokens: false,
            key_memory_projection_cache_key: None,
            latest_context_budget: None,
            latest_projected_memory: None,
            latest_recalled_memory: None,
            active_activity_plan: None,
            profile: BearProfile::Pair,
            overflow_retry_attempted: false,
            overflow_compaction_recovered: false,
        });

        let (_continuation, mut stream) = continue_native_client_turn_event_stream(
            TurnContinueRequest {
                sqlx_pool: &pool,
                config: &config,
                memory_stores: &stores,
                request_id: Uuid::new_v4(),
                run_id: Some("run-max-step"),
                client_session_id: &client_session_id,
                conversation: RuntimeConversationRef {
                    id: conversation_id.clone(),
                },
                binding: &RoleRuntimeBinding {
                    binding_id: format!("den-native:{}:pair", Uuid::new_v4()),
                    compatibility_backend: Some("native".to_string()),
                },
                continuation: RuntimeContinuation::ToolResult {
                    tool_call_id: "call-max".to_string(),
                    approval_request_id: None,
                    status: RuntimeToolResultStatus::Ok,
                    content: "{}".to_string(),
                },
                stream_context: crate::turn_runner::default_tool_continue_stream_context(),
            },
            BearProfile::Pair,
        )
        .await
        .expect("max-step continuation should return terminal stream");

        let event = stream
            .next()
            .await
            .expect("terminal event")
            .expect("event ok");
        match event {
            RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed { message, .. }) => {
                assert!(message.contains("emergency continuation fuse"));
                assert!(message.contains("step=8/emergency_hard_steps=8"));
            }
            other => panic!("expected TurnFailed, got {other:?}"),
        }
        SESSION_STORE.remove(&session_key);
    }

    #[tokio::test]
    async fn recorded_client_tool_result_remains_visible_in_live_continuation_transcript() {
        let conversation_id = format!("transient-{}", Uuid::new_v4().simple());
        let client_session_id = format!("session-{}", Uuid::new_v4().simple());
        let session_key = agent_loop_session_key(&conversation_id, &client_session_id);
        let tool_call_id = "call-visible-tool";
        SESSION_STORE.insert(AgentLoopSession {
            session_key: session_key.clone(),
            bear_id: Uuid::new_v4(),
            bear_slug: "test-bear".to_string(),
            user_id: Some(1),
            conversation_id: conversation_id.clone(),
            client_session_id: client_session_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            request_id: Some("request-before-tool".to_string()),
            run_id: Some("run-visible-tool".to_string()),
            messages: vec![
                ChatMessage {
                    role: "user".to_string(),
                    content: Some("Inspect the plan".to_string()),
                    tool_call_id: None,
                    name: None,
                    tool_calls: None,
                },
                ChatMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_call_id: None,
                    name: None,
                    tool_calls: Some(vec![ChatToolCall {
                        id: tool_call_id.to_string(),
                        call_type: "function".to_string(),
                        function: crate::llm::ChatToolCallFunction {
                            name: "fs_read_text_file".to_string(),
                            arguments: serde_json::json!({
                                "path": "/workspace/docs/roadmap/PLAN.md",
                                "limit": 120,
                            })
                            .to_string(),
                        },
                    }]),
                },
            ],
            tools: Vec::new(),
            budget_components: Default::default(),
            model: "openai/test".to_string(),
            model_context_window: None,
            model_max_output_tokens: None,
            bifrost_virtual_key: None,
            api_style: None,
            step: 1,
            turn_budget: pair_turn_budget(),
            turn_budget_state: Default::default(),
            agent_loop_control: test_agent_loop_control(),
            checkpoint_state: Default::default(),
            pending_checkpoint_request: None,
            pending_checkpoint_task_action: None,
            strategy: StrategyProfile::plain_react(),
            stream_tokens: false,
            key_memory_projection_cache_key: None,
            latest_context_budget: None,
            latest_projected_memory: None,
            latest_recalled_memory: None,
            active_activity_plan: None,
            profile: BearProfile::Pair,
            overflow_retry_attempted: false,
            overflow_compaction_recovered: false,
        });
        let pool = PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/unused")
            .expect("lazy pool");

        record_native_client_tool_result(
            &pool,
            &conversation_id,
            &client_session_id,
            "request-after-tool",
            Some("run-visible-tool"),
            tool_call_id,
            None,
            RuntimeToolResultStatus::Ok,
            serde_json::json!({
                "tool_name": "fs_read_text_file",
                "status": "ok",
                "structured_content": { "content": "# BEARS roadmap" }
            })
            .to_string(),
        )
        .await
        .expect("record tool result");

        let stored = SESSION_STORE.get(&session_key).expect("stored session");
        assert_eq!(stored.messages.len(), 3);
        assert_eq!(stored.messages[1].role, "assistant");
        assert_eq!(
            stored.messages[1]
                .tool_calls
                .as_ref()
                .and_then(|calls| calls.first())
                .map(|call| (call.id.as_str(), call.function.name.as_str())),
            Some((tool_call_id, "fs_read_text_file"))
        );
        assert_eq!(stored.messages[2].role, "tool");
        assert_eq!(
            stored.messages[2].tool_call_id.as_deref(),
            Some(tool_call_id)
        );
        assert!(stored.messages[2]
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("BEARS roadmap"));

        let repaired = crate::agent_loop::repair_tool_call_message_chain(stored.messages);
        assert_eq!(repaired.len(), 3);
        assert_eq!(repaired[1].role, "assistant");
        assert_eq!(repaired[2].role, "tool");
        SESSION_STORE.remove(&session_key);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn recorded_client_tool_result_replays_from_persisted_history(pool: PgPool) {
        let suffix = Uuid::new_v4().simple().to_string();
        let username = format!("toolhist{}", &suffix[..12]);
        let email = format!("{username}@example.test");
        let (user_id,): (i32,) = sqlx::query_as(
            r"
            INSERT INTO users (email, username, display_name, passhash)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            ",
        )
        .bind(email)
        .bind(&username)
        .bind("Tool History Test")
        .bind("test-passhash")
        .fetch_one(&pool)
        .await
        .expect("create user");
        let bear_id = Uuid::new_v4();
        let bear_slug = format!("tool-history-{}", &suffix[..12]);
        sqlx::query(
            r"
            INSERT INTO bears (id, slug, name)
            VALUES ($1, $2, $3)
            ",
        )
        .bind(bear_id)
        .bind(&bear_slug)
        .bind("Tool History Bear")
        .execute(&pool)
        .await
        .expect("create bear");

        let conversation_id = format!("den-conv-{}", Uuid::new_v4().simple());
        let client_session_id = format!("session-{}", Uuid::new_v4().simple());
        let request_id = Uuid::new_v4().to_string();
        let tool_call_id = "call-persisted-visible";
        let session_key = agent_loop_session_key(&conversation_id, &client_session_id);
        let context = canonical_persistence_context(
            pool.clone(),
            bear_id,
            Some(user_id),
            conversation_id.clone(),
            Some(client_session_id.clone()),
            Some(request_id.clone()),
            client_session_id.clone(),
            false,
        );
        persist_canonical_conversation_record(
            &context,
            &CanonicalConversationRecord::visible_user_message(
                "Read the plan",
                serde_json::json!({ "event": "user_message", "scope_id": client_session_id }),
                None,
            ),
        )
        .await
        .expect("persist user message");

        SESSION_STORE.insert(AgentLoopSession {
            session_key: session_key.clone(),
            bear_id,
            bear_slug,
            user_id: Some(user_id),
            conversation_id: conversation_id.clone(),
            client_session_id: client_session_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            request_id: Some(request_id.clone()),
            run_id: Some("run-persisted-visible".to_string()),
            messages: vec![ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![ChatToolCall {
                    id: tool_call_id.to_string(),
                    call_type: "function".to_string(),
                    function: crate::llm::ChatToolCallFunction {
                        name: "fs_read_text_file".to_string(),
                        arguments: serde_json::json!({ "path": "/workspace/docs/roadmap/PLAN.md" })
                            .to_string(),
                    },
                }]),
            }],
            tools: Vec::new(),
            budget_components: Default::default(),
            model: "openai/test".to_string(),
            model_context_window: None,
            model_max_output_tokens: None,
            bifrost_virtual_key: None,
            api_style: None,
            step: 1,
            turn_budget: pair_turn_budget(),
            turn_budget_state: Default::default(),
            agent_loop_control: test_agent_loop_control(),
            checkpoint_state: Default::default(),
            pending_checkpoint_request: None,
            pending_checkpoint_task_action: None,
            strategy: StrategyProfile::plain_react(),
            stream_tokens: false,
            key_memory_projection_cache_key: None,
            latest_context_budget: None,
            latest_projected_memory: None,
            latest_recalled_memory: None,
            active_activity_plan: None,
            profile: BearProfile::Pair,
            overflow_retry_attempted: false,
            overflow_compaction_recovered: false,
        });

        record_native_client_tool_result(
            &pool,
            &conversation_id,
            &client_session_id,
            &request_id,
            Some("run-persisted-visible"),
            tool_call_id,
            None,
            RuntimeToolResultStatus::Ok,
            serde_json::json!({
                "tool_name": "fs_read_text_file",
                "status": "ok",
                "structured_content": { "content": "# BEARS roadmap" }
            })
            .to_string(),
        )
        .await
        .expect("record tool result");
        SESSION_STORE.remove(&session_key);

        let replayed =
            crate::agent_loop::load_transcript_messages(&pool, bear_id, &conversation_id)
                .await
                .expect("load transcript");
        assert_eq!(replayed.len(), 3, "{replayed:#?}");
        assert_eq!(replayed[1].role, "assistant");
        assert_eq!(
            replayed[1]
                .tool_calls
                .as_ref()
                .and_then(|calls| calls.first())
                .map(|call| (call.id.as_str(), call.function.name.as_str())),
            Some((tool_call_id, "fs_read_text_file"))
        );
        assert_eq!(replayed[2].role, "tool");
        assert_eq!(replayed[2].tool_call_id.as_deref(), Some(tool_call_id));
        assert!(replayed[2]
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("BEARS roadmap"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn load_history_replays_persisted_tool_call_and_result_records(pool: PgPool) {
        let suffix = Uuid::new_v4().simple().to_string();
        let username = format!("toolhistload{}", &suffix[..8]);
        let email = format!("{username}@example.test");
        let (user_id,): (i32,) = sqlx::query_as(
            r"
            INSERT INTO users (email, username, display_name, passhash)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            ",
        )
        .bind(email)
        .bind(&username)
        .bind("Tool Load History Test")
        .bind("test-passhash")
        .fetch_one(&pool)
        .await
        .expect("create user");
        let bear_id = Uuid::new_v4();
        let bear_slug = format!("tool-load-history-{}", &suffix[..8]);
        sqlx::query(
            r"
            INSERT INTO bears (id, slug, name)
            VALUES ($1, $2, $3)
            ",
        )
        .bind(bear_id)
        .bind(&bear_slug)
        .bind("Tool Load History Bear")
        .execute(&pool)
        .await
        .expect("create bear");

        let conversation_id = format!("den-conv-{}", Uuid::new_v4().simple());
        let client_session_id = format!("session-{}", Uuid::new_v4().simple());
        let request_id = Uuid::new_v4().to_string();
        let tool_call_id = "call-load-history";
        let context = canonical_persistence_context(
            pool.clone(),
            bear_id,
            Some(user_id),
            conversation_id.clone(),
            Some(client_session_id.clone()),
            Some(request_id.clone()),
            client_session_id.clone(),
            false,
        );
        persist_canonical_conversation_record(
            &context,
            &CanonicalConversationRecord::visible_user_message(
                "Read the plan",
                serde_json::json!({ "event": "user_message", "scope_id": client_session_id }),
                None,
            ),
        )
        .await
        .expect("persist user message");
        let session_key = agent_loop_session_key(&conversation_id, &client_session_id);
        SESSION_STORE.insert(AgentLoopSession {
            session_key: session_key.clone(),
            bear_id,
            bear_slug,
            user_id: Some(user_id),
            conversation_id: conversation_id.clone(),
            client_session_id: client_session_id.clone(),
            workspace_roots: vec!["/workspace".to_string()],
            request_id: Some(request_id.clone()),
            run_id: Some("run-load-history".to_string()),
            messages: vec![ChatMessage {
                role: "assistant".to_string(),
                content: None,
                tool_call_id: None,
                name: None,
                tool_calls: Some(vec![ChatToolCall {
                    id: tool_call_id.to_string(),
                    call_type: "function".to_string(),
                    function: crate::llm::ChatToolCallFunction {
                        name: "fs_read_text_file".to_string(),
                        arguments: serde_json::json!({ "path": "/workspace/docs/roadmap/PLAN.md" })
                            .to_string(),
                    },
                }]),
            }],
            tools: Vec::new(),
            budget_components: Default::default(),
            model: "openai/test".to_string(),
            model_context_window: None,
            model_max_output_tokens: None,
            bifrost_virtual_key: None,
            api_style: None,
            step: 1,
            turn_budget: pair_turn_budget(),
            turn_budget_state: Default::default(),
            agent_loop_control: test_agent_loop_control(),
            checkpoint_state: Default::default(),
            pending_checkpoint_request: None,
            pending_checkpoint_task_action: None,
            strategy: StrategyProfile::plain_react(),
            stream_tokens: false,
            key_memory_projection_cache_key: None,
            latest_context_budget: None,
            latest_projected_memory: None,
            latest_recalled_memory: None,
            active_activity_plan: None,
            profile: BearProfile::Pair,
            overflow_retry_attempted: false,
            overflow_compaction_recovered: false,
        });

        record_native_client_tool_result(
            &pool,
            &conversation_id,
            &client_session_id,
            &request_id,
            Some("run-load-history"),
            tool_call_id,
            None,
            RuntimeToolResultStatus::Ok,
            serde_json::json!({
                "tool_name": "fs_read_text_file",
                "status": "ok",
                "structured_content": { "content": "# BEARS roadmap" }
            })
            .to_string(),
        )
        .await
        .expect("record tool result");
        SESSION_STORE.remove(&session_key);

        let canonical = conversation_persistence::get_conversation_for_external_id(
            &pool,
            bear_id,
            &conversation_id,
        )
        .await
        .expect("load canonical conversation")
        .expect("canonical conversation exists");
        let rows = conversation_persistence::list_messages_page(&pool, canonical.id, None, 10)
            .await
            .expect("list persisted rows");
        assert!(
            rows.iter().any(|row| row.message_type == "tool_call"),
            "expected persisted tool_call row, got {rows:#?}"
        );
        assert!(
            rows.iter().any(|row| row.message_type == "tool_result"),
            "expected persisted tool_result row, got {rows:#?}"
        );
        let persisted_tool_result = rows
            .iter()
            .find(|row| row.message_type == "tool_result")
            .expect("persisted tool_result row");
        assert_eq!(
            persisted_tool_result.content_json["output_summary"],
            serde_json::json!("Used fs_read_text_file (ok): {\"status\":\"ok\",\"structured_content\":{\"content\":\"# BEARS roadmap\"},\"tool_name\":\"fs_read_text_file\"}")
        );
        assert!(
            rows.iter().any(|row| matches!(
                row.to_model_transcript_record(),
                Some(PersistedTranscriptRecord::ToolCall { .. })
            )),
            "expected tool_call transcript projection, got {rows:#?}"
        );
        assert!(
            rows.iter().any(|row| matches!(
                row.to_model_transcript_record(),
                Some(PersistedTranscriptRecord::ToolResult { .. })
            )),
            "expected tool_result transcript projection, got {rows:#?}"
        );

        let backend = NativeRuntimeConversationBackend::with_pool(pool);
        let binding = RoleRuntimeBinding {
            binding_id: format!("den-native:{bear_id}:pair"),
            compatibility_backend: Some("native".to_string()),
        };
        let history = backend
            .load_history(
                &binding,
                &RuntimeConversationRef {
                    id: conversation_id,
                },
            )
            .await
            .expect("load history");

        assert_eq!(history.records.len(), 3, "{history:#?}");
        assert!(matches!(
            &history.records[0],
            RuntimeHistoryRecord::Message { role, content, .. }
            if role == "user" && content == "Read the plan"
        ));
        assert!(matches!(
            &history.records[1],
            RuntimeHistoryRecord::ToolCall {
                tool_call_id: record_tool_call_id,
                tool_name,
                arguments,
                ..
            }
            if record_tool_call_id == tool_call_id
                && tool_name == "fs_read_text_file"
                && arguments.get("path").and_then(serde_json::Value::as_str) == Some("/workspace/docs/roadmap/PLAN.md")
        ));
        assert!(matches!(
            &history.records[2],
            RuntimeHistoryRecord::ToolResult {
                tool_call_id: Some(record_tool_call_id),
                tool_name: Some(tool_name),
                status: Some(status),
                content: Some(content),
                ..
            }
            if record_tool_call_id == tool_call_id
                && tool_name == "fs_read_text_file"
                && status == "ok"
                && content.contains("BEARS roadmap")
        ));
    }

    #[test]
    fn prompt_for_model_leaves_plain_prompt_unchanged_without_host_context() {
        assert_eq!(
            prompt_for_model("Please continue.", None),
            "Please continue."
        );
    }

    #[test]
    fn work_without_active_task_list_does_not_use_task_list_terminal_gate() {
        assert!(crate::runtime::turn_state::should_allow_terminal_response(
            BearProfile::Work,
            None,
            "What I changed: added one test. Remaining work: more later."
        ));
    }
}
