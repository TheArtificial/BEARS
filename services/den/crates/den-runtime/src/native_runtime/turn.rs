use den_core::config::Config;
use den_core::tools::{
    arguments::DenToolChannelContext, constants::DEN_WEB_FETCH, context::DenToolInvocationContext,
    descriptor::builtin_den_tool_descriptor_for_provider_name,
};
use std::sync::{Arc, LazyLock};

use futures::StreamExt;
use den_memory::MemoryStoreManager;
use den_protocol::{
    ContinueTurnRequest, RoleRuntimeBinding, RuntimeContinuation, RuntimeConversationBackend,
    RuntimeConversationRef, RuntimeEventStream, RuntimeHistoryPage, RuntimeHistoryRecord,
    RuntimeSemanticEvent, RuntimeStreamContinuation, RuntimeStreamEvent, StartTurnRequest,
};
use den_service::{
    bears::BearProfile,
    conversation::{
        events::{
            canonical_persistence_context, persist_canonical_conversation_record,
            CanonicalConversationRecord, ConversationEventProvenance,
        },
        persistence as conversation_persistence,
    },
};
use sqlx::PgPool;
use uuid::Uuid;

use super::web_chat_loop::{NativeWebChatLoopRuntime, NativeWebChatLoopStream};

use crate::{
    agent_loop::{
        agent_loop_session_key, assemble_native_turn_for_bear, record_approval_decision,
        run_agent_step_stream, AgentLoopSession, AgentLoopSessionStore, AgentStepOverflowContext,
        AssembleTurnContext, NativeToolDispatchMode, SessionTrackingStream,
    },
    llm::{ChatMessage, ChatToolCall, LlmClient},
    native_runtime::{profile::NativeCapabilityProfile, tools::merge_den_and_client_tools},
    turn_runner::{
        materialize_runtime_conversation_if_needed, TurnContinueRequest, TurnStartRequest,
    },
};
use den_core::DenError;

static SESSION_STORE: LazyLock<AgentLoopSessionStore> = LazyLock::new(AgentLoopSessionStore::new);

/// Returns whether this turn recovered from context overflow via emergency compaction.
/// Clears the session flag after reading (for ACP terminal turn_result mapping).
pub fn take_session_overflow_compaction_recovered(
    conversation_id: &str,
    acp_session_id: &str,
) -> bool {
    let key = agent_loop_session_key(conversation_id, acp_session_id);
    SESSION_STORE.take_overflow_compaction_recovered(&key)
}

pub fn native_acp_session_exists(conversation_id: &str, acp_session_id: &str) -> bool {
    let key = agent_loop_session_key(conversation_id, acp_session_id);
    SESSION_STORE.get(&key).is_some()
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
        let records = rows
            .into_iter()
            .rev()
            .filter_map(|row| {
                row.to_model_transcript_message()
                    .map(|message| RuntimeHistoryRecord {
                        message_id: message.message_id,
                        role: message.role,
                        content: message.content,
                        created_at: Some(message.created_at.to_string()),
                    })
            })
            .collect();
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
    acp_session_id: &str,
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
        acp_session_id.to_string(),
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
    acp_session_id: &str,
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
    let messages = assembled.messages;
    let tools =
        merge_den_and_client_tools(deps.config, profile.profile, client_tools, human_message)?;
    let session_key = agent_loop_session_key(conversation_id, acp_session_id);
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
    let bifrost_virtual_key = den_service::bears::db::bifrost_virtual_key_for_inference(
        deps.pool,
        bear.id,
        &deps.config.den_secret_encryption_key,
    )
    .await?;
    let session = AgentLoopSession {
        session_key,
        bear_id,
        bear_slug: bear.slug.clone(),
        user_id,
        conversation_id: conversation_id.to_string(),
        acp_session_id: acp_session_id.to_string(),
        request_id: request_id.map(|id| id.to_string()),
        run_id: run_id.map(str::to_string),
        messages,
        tools,
        model,
        bifrost_virtual_key,
        api_style,
        step: 0,
        max_steps: profile.max_steps,
        strategy: profile.strategy,
        stream_tokens,
        key_memory_projection_cache_key,
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

pub async fn start_native_acp_turn_event_stream(
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
    let acp_session_id = request.session_id;
    let workspace_roots = request.cwd.map(|cwd| vec![cwd.to_string()]);
    let session = build_session(
        &NativeRuntimeDeps {
            pool: request.sqlx_pool,
            config: request.config,
            stores: request.memory_stores,
        },
        profile,
        request.bear_id,
        &conversation_id,
        acp_session_id,
        Some(request.prompt),
        request.runtime_context,
        Some(acp_session_id),
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
        let provenance = ConversationEventProvenance::acp_session(acp_session_id.to_string());
        let mut content_json = provenance.as_content_json("user_prompt");
        content_json["role"] = serde_json::json!("user");
        content_json["acp_session_id"] = serde_json::json!(acp_session_id);
        content_json["client"] = serde_json::json!(request.client);
        content_json["request_id"] = serde_json::json!(request.request_id.to_string());
        let record =
            CanonicalConversationRecord::visible_user_message(request.prompt, content_json, None);
        persist_canonical_conversation_record(
            &canonical_persistence_context(
                request.sqlx_pool.clone(),
                request.bear_id,
                Some(request.user_id),
                conversation_id.clone(),
                Some(acp_session_id.to_string()),
                Some(request.request_id.to_string()),
                acp_session_id.to_string(),
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
        acp_session_id,
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
        client_session_id: Some(acp_session_id.to_string()),
        client_tools: request.client_tools.clone(),
        stream_tokens: request.stream_tokens,
    };
    Ok(stream)
}

pub async fn continue_native_profile_turn_event_stream(
    request: TurnContinueRequest<'_>,
    role: BearProfile,
) -> Result<(RuntimeStreamContinuation, RuntimeEventStream), DenError> {
    continue_native_acp_turn_event_stream(request, role).await
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
        r#"
        INSERT INTO bear_web_approvals (bear_id, scope_kind, scope_value, approved_by_user_id, source, expires_at)
        VALUES ($1, 'url', $2, $3, 'acp', now() + interval '1 hour')
        ON CONFLICT (bear_id, scope_kind, scope_value) WHERE revoked_at IS NULL
        DO UPDATE SET approved_by_user_id = EXCLUDED.approved_by_user_id,
                      source = EXCLUDED.source,
                      expires_at = EXCLUDED.expires_at
        "#,
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
        session_id: session.acp_session_id.clone(),
        acp_session_id: Some(session.acp_session_id.clone()),
        conversation_selection: Some(session.conversation_id.clone()),
        runtime_target: Some(session.conversation_id.clone()),
        workspace_roots: Vec::new(),
        session_policy: None,
        activity: None,
        runtime: None,
        context_budget: None,
        request_id: Some(request.request_id.to_string()),
        channel: DenToolChannelContext {
            family: Some("acp".to_string()),
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

pub async fn continue_native_acp_turn_event_stream(
    request: TurnContinueRequest<'_>,
    profile: BearProfile,
) -> Result<(RuntimeStreamContinuation, RuntimeEventStream), DenError> {
    let acp_session_id = request.acp_session_id;
    let conversation_id = request.conversation.id.clone();
    let session_key = agent_loop_session_key(&conversation_id, acp_session_id);
    let existing_session = SESSION_STORE.get(&session_key);
    let mut tool_messages = Vec::new();
    match &request.continuation {
        RuntimeContinuation::ToolResult {
            tool_call_id,
            approval_request_id,
            status,
            content,
        } => {
            if let Some(approval_request_id) = approval_request_id.as_deref() {
                let approve = matches!(
                    status,
                    den_protocol::RuntimeToolResultStatus::Ok
                );
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
            tool_messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(content.clone()),
                tool_call_id: Some(tool_call_id.clone()),
                name: None,
                tool_calls: None,
            });
        }
        RuntimeContinuation::ApprovalDecision {
            approval_request_id,
            tool_call_id,
            decision,
            reason,
        } => {
            let approve = matches!(
                decision,
                den_protocol::RuntimeApprovalDecision::Approve
            );
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
                                tool_messages.push(
                                    execute_approved_den_tool_for_session(
                                        &request, session, &call, profile,
                                    )
                                    .await?,
                                );
                            }
                        }
                    }
                }
            } else {
                let content = reason.clone().unwrap_or_else(|| "denied".to_string());
                tool_messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(content),
                    tool_call_id: tool_call_id.clone(),
                    name: None,
                    tool_calls: None,
                });
            }
        }
    }
    SESSION_STORE.update(&session_key, |session| {
        session.request_id = Some(request.request_id.to_string());
        session.run_id = request
            .run_id
            .map(str::to_string)
            .or_else(|| session.run_id.clone());
        session.messages.extend(tool_messages.clone());
    });
    let session = SESSION_STORE
        .get(&session_key)
        .ok_or_else(|| DenError::System("native agent loop session not found".to_string()))?;
    if session.step >= session.max_steps {
        return Err(DenError::System(
            "native agent loop reached max steps".to_string(),
        ));
    }
    let llm = LlmClient::new(request.config);
    let overflow = overflow_context(
        request.sqlx_pool.clone(),
        Arc::new(request.config.clone()),
        profile,
    );
    let stream = run_agent_step_stream(&llm, &session, Some(overflow)).await?;
    let stream = wrap_session_stream(
        stream,
        &session,
        Arc::new(request.config.clone()),
        profile,
        request.sqlx_pool.clone(),
        session.bear_id,
        session.user_id,
        &conversation_id,
        acp_session_id,
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
}
