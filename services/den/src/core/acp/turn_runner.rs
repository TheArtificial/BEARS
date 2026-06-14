//! ACP turn dispatch (edge orchestration) — native in-process loop only.
//!
//! The runtime-side turn *contracts* (request inputs, stream context, conversation
//! materialization) live in `den-runtime`; this module re-exports them and keeps the
//! edge-only wrappers that wire `ApiState`/tool-turn coordination into the native runtime.

use uuid::Uuid;

use crate::{
    api::service::ApiState,
    core::{
        acp_tool_turns::AcpToolTurnCoordinator,
        runtime_contracts::{RuntimeEventStream, RuntimeStreamContinuation},
    },
    errors::DenError,
};

pub use den_runtime::acp_turn_runner::{
    default_acp_tool_continue_stream_context, looks_like_runtime_waiting_for_approval_error,
    materialize_acp_runtime_conversation_if_needed, AcpRuntimeMaterializationResult,
    AcpTurnContinueRequest, AcpTurnStartRequest, AcpTurnStreamContext,
    ACP_STALE_APPROVAL_RECOVERY_DENIAL_REASON,
};

pub struct AcpStaleRuntimeCleanupParams {
    pub state: ApiState,
    pub tool_turns: AcpToolTurnCoordinator,
    pub acp_session_id: String,
    pub bear_id: Uuid,
    pub pair_agent_id: String,
    /// Empty under native runtime (in-process cancel).
    pub run_ids: Vec<String>,
    pub reason: &'static str,
    pub request_id: Uuid,
}

pub async fn start_acp_turn_event_stream_with_retries(
    request: AcpTurnStartRequest<'_>,
) -> Result<RuntimeEventStream, DenError> {
    crate::core::native_runtime::start_native_acp_turn_event_stream(request).await
}

pub async fn continue_acp_turn_with_runtime(
    request: AcpTurnContinueRequest<'_>,
) -> Result<(RuntimeStreamContinuation, RuntimeEventStream), DenError> {
    crate::core::native_runtime::continue_native_acp_turn_event_stream(request).await
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
    let _ = state;
    serde_json::json!({
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
    })
}
