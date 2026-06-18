//! Runtime-side ACP turn contracts: the start/continue request inputs the native runtime
//! consumes, the stream context, and conversation materialization.
//!
//! The ACP *edge* orchestration (retry wrappers, stale-state cleanup) stays in `den`'s
//! `core::acp::turn_runner`, which re-exports these types for existing call sites.

use sqlx::PgPool;
use uuid::Uuid;

use den_core::{config::Config, DenError};

use crate::{
    acp_sessions,
    conversation_ids::is_native_runtime_conversation_id,
    memory::MemoryStoreManager,
    runtime_contracts::{
        RuntimeContinuation, RuntimeConversationBackend, RuntimeConversationRef,
    },
};

/// Shown to the model when stale-approval recovery auto-denies an expired tool approval.
pub const STALE_APPROVAL_RECOVERY_DENIAL_REASON: &str = "BEARS closed an expired ACP approval request during stale-approval recovery. This denial applies only to that stale request; it is not a user or web policy block. Retry the tool if it is still needed.";

pub struct TurnStartRequest<'a> {
    pub sqlx_pool: &'a PgPool,
    pub config: &'a Config,
    pub memory_stores: &'a MemoryStoreManager,
    pub request_id: Uuid,
    pub user_id: i32,
    pub session_id: &'a str,
    pub bear_id: Uuid,
    pub bear_slug: &'a str,
    pub client: &'a str,
    pub cwd: Option<&'a str>,
    pub binding: &'a crate::runtime_contracts::RoleRuntimeBinding,
    pub conversation_selection: &'a str,
    pub upstream_target: &'a str,
    pub prompt: &'a str,
    pub client_tools: Option<serde_json::Value>,
    pub runtime_context: Option<&'a str>,
    pub runtime_context_len: usize,
    pub stream_tokens: bool,
}

pub struct TurnContinueRequest<'a> {
    pub sqlx_pool: &'a PgPool,
    pub config: &'a Config,
    pub memory_stores: &'a MemoryStoreManager,
    pub request_id: Uuid,
    pub acp_session_id: &'a str,
    pub conversation: RuntimeConversationRef,
    pub binding: &'a crate::runtime_contracts::RoleRuntimeBinding,
    pub continuation: RuntimeContinuation,
    pub stream_context: TurnStreamContext,
}

pub fn default_tool_continue_stream_context() -> TurnStreamContext {
    TurnStreamContext {
        client_tools: None,
        stream_tokens: false,
        max_steps: 4,
    }
}

#[derive(Debug, Clone)]
pub struct TurnStreamContext {
    pub client_tools: Option<serde_json::Value>,
    pub stream_tokens: bool,
    pub max_steps: u32,
}

pub fn looks_like_runtime_waiting_for_approval_error(err: &DenError) -> bool {
    crate::runtime_contracts::runtime_error_is_conflict_pending_approval(err)
}

pub struct RuntimeMaterializationResult {
    pub conversation_id: String,
    pub created: bool,
}

/// Materialize a runtime conversation when the client selected a pending `new-*` id.
///
/// Prompt bootstrap usually resolves `upstream_target` to `den-conv-*` before the
/// turn starts; this function then returns early without creating a second conversation.
pub async fn materialize_runtime_conversation_if_needed<B: RuntimeConversationBackend>(
    runtime_conversations: &B,
    request: &TurnStartRequest<'_>,
) -> Result<RuntimeMaterializationResult, DenError> {
    if request.upstream_target.starts_with("conv-")
        || is_native_runtime_conversation_id(request.upstream_target)
    {
        return Ok(RuntimeMaterializationResult {
            conversation_id: request.upstream_target.to_string(),
            created: false,
        });
    }
    if !request.conversation_selection.starts_with("new-") {
        return Ok(RuntimeMaterializationResult {
            conversation_id: request.upstream_target.to_string(),
            created: false,
        });
    }
    let conv_id = runtime_conversations
        .create_conversation(request.binding)
        .await?
        .id;
    acp_sessions::upsert_session(
        request.sqlx_pool,
        acp_sessions::UpsertAcpSession {
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
    Ok(RuntimeMaterializationResult {
        conversation_id: conv_id,
        created: true,
    })
}
