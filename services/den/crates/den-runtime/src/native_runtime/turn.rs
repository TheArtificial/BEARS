use den_core::config::Config;
use std::sync::{Arc, LazyLock};

use futures::StreamExt;
use sqlx::PgPool;
use uuid::Uuid;

use super::web_chat_loop::{NativeWebChatLoopRuntime, NativeWebChatLoopStream};

use crate::{
    {
        acp_turn_runner::{materialize_acp_runtime_conversation_if_needed, AcpTurnContinueRequest, AcpTurnStartRequest},
        agent_loop::{
            agent_loop_session_key, assemble_native_turn_for_bear, run_agent_step_stream,
            record_approval_decision, AgentLoopSession, AgentLoopSessionStore, AssembleTurnContext,
            NativeToolDispatchMode, SessionTrackingStream,
        },
        bears::BearProfile,
        conversation_persistence,
        llm::{ChatMessage, LlmClient},
        memory::MemoryStoreManager,
        native_runtime::{
            profile::NativeCapabilityProfile,
            tools::merge_den_and_client_tools,
        },
        runtime_contracts::{
            ContinueTurnRequest, RuntimeContinuation, RuntimeConversationBackend,
            RuntimeConversationRef, RuntimeEventStream, RuntimeHistoryPage, RuntimeHistoryRecord,
            RuntimeSemanticEvent, RuntimeStreamContinuation, RuntimeStreamEvent,
            RoleRuntimeBinding, StartTurnRequest,
        },
    },
};
use den_core::DenError;

static SESSION_STORE: LazyLock<AgentLoopSessionStore> = LazyLock::new(AgentLoopSessionStore::new);

/// Shared dependencies for internal native profile turns (no full `ApiState` required).
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
                    pool,
                    bear_id,
                    None,
                    &id,
                    None,
                    None,
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
        let rows = conversation_persistence::list_messages_page(pool, canonical.id, None, 100)
            .await?;
        let records = rows
            .into_iter()
            .rev()
            .filter_map(|row| {
                if row.message_type != "visible_message" {
                    return None;
                }
                let role = row.role?;
                if role != "user" && role != "assistant" {
                    return None;
                }
                Some(RuntimeHistoryRecord {
                    message_id: row.provider_message_id,
                    role,
                    content: row.content_text,
                    created_at: Some(row.created_at.to_string()),
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
    pool: PgPool,
    bear_id: Uuid,
    user_id: Option<i32>,
    conversation_id: &str,
    acp_session_id: &str,
    request_id: Option<String>,
) -> RuntimeEventStream {
    Box::pin(SessionTrackingStream::new(
        stream,
        session,
        SESSION_STORE.clone(),
        pool,
        bear_id,
        user_id,
        conversation_id.to_string(),
        acp_session_id.to_string(),
        request_id,
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
    stream_tokens: bool,
    tool_messages: Vec<ChatMessage>,
) -> Result<AgentLoopSession, DenError> {
    let llm = LlmClient::new(deps.config);
    let bear = crate::bears::db::get_bear(deps.pool, bear_id)
        .await?
        .ok_or_else(|| DenError::NotFound("bear not found".to_string()))?;
    let include_prompt_memory =
        profile.include_prompt_memory && runtime_context.is_none();
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
    let tools = merge_den_and_client_tools(
        deps.config,
        profile.profile,
        client_tools,
        human_message,
    )?;
    let session_key = agent_loop_session_key(conversation_id, acp_session_id);
    let model = llm.resolve_model(bear.default_model.as_deref());
    let session = AgentLoopSession {
        session_key: session_key.clone(),
        bear_id,
        conversation_id: conversation_id.to_string(),
        messages,
        tools,
        model,
        step: 0,
        max_steps: profile.max_steps,
        strategy: profile.strategy,
        stream_tokens,
        key_memory_projection_cache_key,
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
        false,
        Vec::new(),
    )
    .await?;
    let llm = LlmClient::new(deps.config);
    let mut stream = run_agent_step_stream(&llm, &session).await?;
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        if let RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::AssistantTextDelta { text: delta }) = item? {
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
        true,
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
    let stream = run_agent_step_stream(&llm, &session).await?;
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
    request: AcpTurnStartRequest<'_>,
) -> Result<RuntimeEventStream, DenError> {
    start_native_profile_turn_event_stream(request, BearProfile::Pair).await
}

pub async fn start_native_profile_turn_event_stream(
    request: AcpTurnStartRequest<'_>,
    role: BearProfile,
) -> Result<RuntimeEventStream, DenError> {
    let profile = NativeCapabilityProfile::for_profile(role);
    let runtime_conversations =
        NativeRuntimeConversationBackend::with_pool(request.sqlx_pool.clone());
    let materialized =
        materialize_acp_runtime_conversation_if_needed(&runtime_conversations, &request).await?;
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
        request.stream_tokens,
        Vec::new(),
    )
    .await?;
    let llm = LlmClient::new(request.config);
    let stream = run_agent_step_stream(&llm, &session).await?;
    let stream = wrap_session_stream(
        stream,
        &session,
        request.sqlx_pool.clone(),
        request.bear_id,
        Some(request.user_id),
        &conversation_id,
        acp_session_id,
        Some(request.request_id.to_string()),
    );
    let _ = StartTurnRequest {
        conversation: RuntimeConversationRef {
            id: conversation_id,
        },
        binding: request.binding.clone(),
        human_message: request.prompt.to_string(),
        runtime_context: request.runtime_context.map(str::to_string),
        acp_session_id: Some(acp_session_id.to_string()),
        client_tools: request.client_tools.clone(),
        stream_tokens: request.stream_tokens,
    };
    Ok(stream)
}

pub async fn continue_native_profile_turn_event_stream(
    request: AcpTurnContinueRequest<'_>,
    _role: BearProfile,
) -> Result<(RuntimeStreamContinuation, RuntimeEventStream), DenError> {
    continue_native_acp_turn_event_stream(request).await
}

pub async fn continue_native_acp_turn_event_stream(
    request: AcpTurnContinueRequest<'_>,
) -> Result<(RuntimeStreamContinuation, RuntimeEventStream), DenError> {
    let acp_session_id = request.acp_session_id;
    let conversation_id = request.conversation.id.clone();
    let session_key = agent_loop_session_key(&conversation_id, acp_session_id);
    let mut tool_messages = Vec::new();
    match &request.continuation {
        RuntimeContinuation::ToolResult {
            tool_call_id,
            status,
            content,
            ..
        } => {
            tool_messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(content.clone()),
                tool_call_id: Some(tool_call_id.clone()),
                name: None,
                tool_calls: None,
            });
            let _ = status;
        }
        RuntimeContinuation::ApprovalDecision {
            approval_request_id,
            tool_call_id,
            decision,
            reason,
        } => {
            let approve = matches!(
                decision,
                crate::runtime_contracts::RuntimeApprovalDecision::Approve
            );
            record_approval_decision(
                request.sqlx_pool,
                approval_request_id,
                approve,
                reason.as_deref(),
            )
            .await?;
            let content = reason.clone().unwrap_or_else(|| {
                if approve {
                    "approved".to_string()
                } else {
                    "denied".to_string()
                }
            });
            tool_messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(content),
                tool_call_id: tool_call_id.clone(),
                name: None,
                tool_calls: None,
            });
        }
    }
    SESSION_STORE.update(&session_key, |session| {
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
    let stream = run_agent_step_stream(&llm, &session).await?;
    let stream = wrap_session_stream(
        stream,
        &session,
        request.sqlx_pool.clone(),
        session.bear_id,
        None,
        &conversation_id,
        acp_session_id,
        Some(request.request_id.to_string()),
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
            binding_id: "agent-letta-123".to_string(),
            compatibility_backend: Some("runtime:letta".to_string()),
        };
        assert_eq!(bear_id_from_native_binding(&binding), None);
    }
}
