//! ACP turn dispatch — native in-process loop is the default; Letta path is gated behind `AGENT_RUNTIME=letta`.

use uuid::Uuid;

use crate::{
    api::service::ApiState,
    core::{
        acp_tool_turns::AcpToolTurnCoordinator,
        runtime_contracts::{
            RuntimeConversationBackend, RuntimeConversationRef, RuntimeEventStream,
            RuntimeStreamContinuation, RoleRuntimeBinding,
        },
    },
    errors::CustomError,
};

/// Shown to the model when stale-approval recovery auto-denies an expired tool approval.
pub const ACP_STALE_APPROVAL_RECOVERY_DENIAL_REASON: &str = "BEARS closed an expired ACP approval request during stale-approval recovery. This denial applies only to that stale request; it is not a user or web policy block. Retry the tool if it is still needed.";

pub struct AcpTurnStartRequest<'a> {
    pub state: &'a ApiState,
    pub request_id: Uuid,
    pub user_id: i32,
    pub session_id: &'a str,
    pub bear_id: Uuid,
    pub bear_slug: &'a str,
    pub client: &'a str,
    pub cwd: Option<&'a str>,
    pub binding: &'a RoleRuntimeBinding,
    pub conversation_selection: &'a str,
    pub upstream_target: &'a str,
    pub prompt: &'a str,
    pub client_tools: Option<serde_json::Value>,
    pub runtime_context: Option<&'a str>,
    pub runtime_context_len: usize,
    pub stream_tokens: bool,
}

pub struct AcpStaleRuntimeCleanupParams {
    pub state: ApiState,
    pub tool_turns: AcpToolTurnCoordinator,
    pub acp_session_id: String,
    pub bear_id: Uuid,
    pub pair_agent_id: String,
    /// Letta HTTP run ids observed during the turn. Empty under native runtime (in-process cancel).
    pub run_ids: Vec<String>,
    pub reason: &'static str,
    pub request_id: Uuid,
}

pub struct AcpTurnContinueRequest<'a> {
    pub state: &'a ApiState,
    pub request_id: Uuid,
    pub acp_session_id: &'a str,
    pub conversation: RuntimeConversationRef,
    pub binding: &'a RoleRuntimeBinding,
    pub continuation: crate::core::runtime_contracts::RuntimeContinuation,
    pub stream_context: AcpTurnStreamContext,
}

pub fn default_acp_tool_continue_stream_context() -> AcpTurnStreamContext {
    AcpTurnStreamContext {
        client_tools: None,
        stream_tokens: false,
        max_steps: 4,
    }
}

#[derive(Debug, Clone)]
pub struct AcpTurnStreamContext {
    pub client_tools: Option<serde_json::Value>,
    pub stream_tokens: bool,
    pub max_steps: u32,
}

pub fn looks_like_runtime_waiting_for_approval_error(err: &CustomError) -> bool {
    crate::core::runtime_contracts::runtime_error_is_conflict_pending_approval(err)
}

pub struct AcpRuntimeMaterializationResult {
    pub conversation_id: String,
    pub created: bool,
}

/// Materialize a runtime conversation when the client selected a pending `new-*` id.
///
/// Native path: prompt bootstrap usually resolves `upstream_target` to `den-conv-*` before the
/// turn starts; this function then returns early without creating a second conversation.
pub async fn materialize_acp_runtime_conversation_if_needed<B: RuntimeConversationBackend>(
    runtime_conversations: &B,
    request: &AcpTurnStartRequest<'_>,
) -> Result<AcpRuntimeMaterializationResult, CustomError> {
    if request.upstream_target.starts_with("conv-")
        || crate::core::acp_runtime::is_native_runtime_conversation_id(request.upstream_target)
    {
        return Ok(AcpRuntimeMaterializationResult {
            conversation_id: request.upstream_target.to_string(),
            created: false,
        });
    }
    if !request.conversation_selection.starts_with("new-") {
        return Ok(AcpRuntimeMaterializationResult {
            conversation_id: request.upstream_target.to_string(),
            created: false,
        });
    }
    let conv_id = runtime_conversations
        .create_conversation(request.binding)
        .await?
        .id;
    crate::core::acp_sessions::upsert_session(
        &request.state.sqlx_pool,
        crate::core::acp_sessions::UpsertAcpSession {
            user_id: request.user_id,
            bear_id: request.bear_id,
            bear_slug: request.bear_slug.to_string(),
            acp_session_id: request.session_id.to_string(),
            runtime_session_id: format!(
                "acp-api-direct:{}:{}:{}",
                request.client, request.bear_id, request.session_id
            ),
            conversation_id: request.conversation_selection.to_string(),
            resolved_conversation_id: Some(conv_id.clone()),
            client: request.client.to_string(),
            cwd: request.cwd.map(str::to_string),
            current_mode: None,
        },
    )
    .await?;
    Ok(AcpRuntimeMaterializationResult {
        conversation_id: conv_id,
        created: true,
    })
}

pub async fn start_acp_turn_event_stream_with_retries(
    request: AcpTurnStartRequest<'_>,
) -> Result<RuntimeEventStream, CustomError> {
    if request.state.config.uses_native_agent_runtime() {
        return crate::core::native_runtime::start_native_acp_turn_event_stream(request).await;
    }
    crate::core::acp_turn_runner_letta::start_letta_acp_turn_event_stream(request).await
}

pub async fn continue_acp_turn_with_runtime(
    request: AcpTurnContinueRequest<'_>,
) -> Result<(RuntimeStreamContinuation, RuntimeEventStream), CustomError> {
    if request.state.config.uses_native_agent_runtime() {
        return crate::core::native_runtime::continue_native_acp_turn_event_stream(request).await;
    }
    crate::core::acp_turn_runner_letta::continue_letta_acp_turn_event_stream(request).await
}

pub async fn acp_cleanup_stale_runtime_state(
    params: AcpStaleRuntimeCleanupParams,
) -> serde_json::Value {
    let AcpStaleRuntimeCleanupParams {
        state,
        tool_turns,
        acp_session_id,
        bear_id,
        pair_agent_id,
        run_ids,
        reason,
        request_id,
    } = params;
    let tool_turn_cleanup = tool_turns.cleanup_request_tool_turns(&acp_session_id, request_id);
    if state.config.uses_native_agent_runtime() {
        return serde_json::json!({
            "ok": true,
            "reason": reason,
            "run_ids": run_ids,
            "cancel_result": "native:in-process cleanup (no external run ids)",
            "tool_turn_cleanup": {
                "pending_removed": tool_turn_cleanup.pending_removed,
                "settled_removed": tool_turn_cleanup.settled_removed,
            },
            "bear_id": bear_id,
            "pair_agent_id": pair_agent_id,
        });
    }
    crate::core::acp_turn_runner_letta::letta_cleanup_stale_runtime_state(
        &state,
        tool_turns,
        acp_session_id,
        bear_id,
        pair_agent_id,
        run_ids,
        reason,
        request_id,
    )
    .await
}
