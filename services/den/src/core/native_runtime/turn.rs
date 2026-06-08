use std::sync::LazyLock;

use uuid::Uuid;

use crate::{
    api::service::ApiState,
    core::{
        acp_turn_runner::{materialize_acp_runtime_conversation_if_needed, AcpTurnContinueRequest, AcpTurnStartRequest},
        agent_loop::{
            agent_loop_session_key, assemble_agent_messages, run_agent_step_stream,
            AgentLoopSession, AgentLoopSessionStore, StrategyProfile,
        },
        bears::BearAgentRole,
        llm::{ChatMessage, LlmClient, LlmToolDefinition},
        runtime_contracts::{
            ContinueTurnRequest, RuntimeContinuation, RuntimeConversationBackend,
            RuntimeConversationRef, RuntimeEventStream, RuntimeHistoryPage, RuntimeHistoryRecord,
            RuntimeStreamContinuation, RoleRuntimeBinding, StartTurnRequest,
        },
        tools::descriptor::builtin_den_tool_descriptors_for_role,
    },
    errors::CustomError,
};

static SESSION_STORE: LazyLock<AgentLoopSessionStore> = LazyLock::new(AgentLoopSessionStore::new);

pub struct NativeRuntimeConversationBackend;

impl NativeRuntimeConversationBackend {
    pub fn new() -> Self {
        Self
    }
}

#[allow(async_fn_in_trait)]
impl RuntimeConversationBackend for NativeRuntimeConversationBackend {
    async fn create_conversation(
        &self,
        binding: &RoleRuntimeBinding,
    ) -> Result<RuntimeConversationRef, CustomError> {
        let id = format!(
            "den-conv-{}",
            Uuid::new_v4().simple()
        );
        let _ = binding;
        Ok(RuntimeConversationRef { id })
    }

    async fn verify_conversation_belongs_to_binding(
        &self,
        _binding: &RoleRuntimeBinding,
        _conversation_id: &str,
    ) -> Result<(), CustomError> {
        Ok(())
    }

    async fn load_history(
        &self,
        _binding: &RoleRuntimeBinding,
        conversation: &RuntimeConversationRef,
    ) -> Result<RuntimeHistoryPage, CustomError> {
        Ok(RuntimeHistoryPage {
            records: vec![RuntimeHistoryRecord {
                message_id: None,
                role: "system".to_string(),
                content: format!("native conversation {}", conversation.id),
                created_at: None,
            }],
            raw_payload: None,
        })
    }
}

fn role_tools_to_llm(role: BearAgentRole) -> Vec<LlmToolDefinition> {
    builtin_den_tool_descriptors_for_role(role)
        .into_iter()
        .map(|d| LlmToolDefinition {
            name: d.provider_name.to_string(),
            description: Some(d.description.to_string()),
            parameters: d.input_schema.clone(),
        })
        .collect()
}

async fn build_session(
    state: &ApiState,
    bear_id: Uuid,
    conversation_id: &str,
    acp_session_id: &str,
    human_message: Option<&str>,
    runtime_context: Option<&str>,
    stream_tokens: bool,
    max_steps: u32,
    tool_messages: Vec<ChatMessage>,
) -> Result<AgentLoopSession, CustomError> {
    let llm = LlmClient::new(state.config.as_ref());
    let messages = assemble_agent_messages(
        &state.sqlx_pool,
        bear_id,
        conversation_id,
        runtime_context,
        human_message,
        &tool_messages,
    )
    .await?;
    let session_key = agent_loop_session_key(conversation_id, acp_session_id);
    let session = AgentLoopSession {
        session_key: session_key.clone(),
        messages,
        tools: role_tools_to_llm(BearAgentRole::Pair),
        model: llm.default_model().to_string(),
        step: 0,
        max_steps,
        strategy: StrategyProfile::plain_react(),
        stream_tokens,
    };
    SESSION_STORE.insert(session.clone());
    Ok(session)
}

pub async fn start_native_acp_turn_event_stream(
    request: AcpTurnStartRequest<'_>,
) -> Result<RuntimeEventStream, CustomError> {
    if !request.state.config.uses_native_agent_runtime() {
        return Err(CustomError::System(
            "native runtime requested but AGENT_RUNTIME is not native".to_string(),
        ));
    }
    let runtime_conversations = NativeRuntimeConversationBackend::new();
    let materialized = materialize_acp_runtime_conversation_if_needed(
        &runtime_conversations,
        &request,
    )
    .await?;
    let conversation_id = materialized.conversation_id;
    let acp_session_id = request.session_id;
    let session = build_session(
        request.state,
        request.bear_id,
        &conversation_id,
        acp_session_id,
        Some(request.prompt),
        None,
        request.stream_tokens,
        8,
        Vec::new(),
    )
    .await?;
    let llm = LlmClient::new(request.state.config.as_ref());
    let stream = run_agent_step_stream(&llm, &session).await?;
    let _ = StartTurnRequest {
        conversation: RuntimeConversationRef {
            id: conversation_id,
        },
        binding: request.binding.clone(),
        human_message: request.prompt.to_string(),
        runtime_context: None,
        acp_session_id: Some(acp_session_id.to_string()),
        client_tools: request.client_tools.clone(),
        stream_tokens: request.stream_tokens,
    };
    Ok(stream)
}

pub async fn continue_native_acp_turn_event_stream(
    request: AcpTurnContinueRequest<'_>,
) -> Result<(RuntimeStreamContinuation, RuntimeEventStream), CustomError> {
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
            });
            let _ = status;
        }
        RuntimeContinuation::ApprovalDecision {
            approval_request_id,
            tool_call_id,
            decision,
            reason,
        } => {
            let content = reason.clone().unwrap_or_else(|| match decision {
                crate::core::runtime_contracts::RuntimeApprovalDecision::Approve => {
                    "approved".to_string()
                }
                crate::core::runtime_contracts::RuntimeApprovalDecision::Deny => {
                    "denied".to_string()
                }
            });
            tool_messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(content),
                tool_call_id: tool_call_id.clone(),
                name: None,
            });
            let _ = approval_request_id;
        }
    }
    SESSION_STORE.update(&session_key, |session| {
        session.messages.extend(tool_messages.clone());
        session.step += 1;
    });
    let session = SESSION_STORE
        .get(&session_key)
        .ok_or_else(|| CustomError::System("native agent loop session not found".to_string()))?;
    let llm = LlmClient::new(request.state.config.as_ref());
    let stream = run_agent_step_stream(&llm, &session).await?;
    let _ = ContinueTurnRequest {
        conversation: request.conversation,
        turn: None,
        binding: request.binding.clone(),
        continuation: request.continuation,
    };
    Ok((RuntimeStreamContinuation::Deferred, stream))
}
