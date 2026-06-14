//! Minimal Agent Client Protocol (ACP) gateway for adapter clients.
//!
//! This is the Phase 7 basic-chat slice: Den authenticates, authorizes the selected bear,
//! injects trusted context, and maps text prompts to the Bear's API-direct `pair` Letta agent.
//! Client-tool relay and full ACP stdio transport live in later slices / an external adapter.

pub(super) mod client;
pub(super) mod compat;
pub(super) mod config;
pub(super) mod handlers;
pub(super) mod history;
pub(super) mod http_types;
pub(super) mod runtime_support;
pub(super) mod pair_reflection_support;
pub(super) mod paths;
pub(super) mod prompt_context;
pub(super) mod prompt_guidance;
pub(super) mod responses;
pub(super) mod routing;
pub(super) mod sessions;
pub(super) mod stream;
pub(super) mod tool_result_diagnostics;
pub(super) mod tool_results;
pub(super) mod types;
pub(super) mod workflow;
pub(super) mod workflow_guidance;
#[cfg(test)]
mod tests;

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use tracing::Instrument;
use uuid::Uuid;

use crate::{
    api::{
        acp::{
            compat::{
                acp_compatibility_error_response, check_adapter_contract,
            },
            stream::{
                mapping::map_runtime_stream_event_to_acp_adapter_events_with_persistence,
                plan::{
                    mode_from_den_tool_result, plan_approval_fallback_payload,
                    plan_update_from_den_tool_result,
                },
                prompt_flow::run_prompt_flow,
                runtime::{invoke_acp_den_tool, persist_stream_event_side_effects},
            },
            tool_results::default_unavailable_context_budget,
        },
        service::ApiState,
    },
    core::{
        acp_events::AcpGatewayEvent,
        acp_tools::{acp_provider_tool_names_for_client_context, resolve_session_policy_for_mode},
        acp_turn_controller::AcpActiveTurnCancelHandle,
        acp_turn_runner::{
            acp_cleanup_stale_runtime_state, continue_acp_turn_with_runtime,
            default_acp_tool_continue_stream_context, AcpStaleRuntimeCleanupParams,
            AcpTurnContinueRequest,
        },
        runtime_provider::RoleRuntimeBinding,
    },
};
use self::{
    responses::acp_error_status_message,
    types::{
        format_acp_session_timestamp, AcpPendingFuture, AcpResolvedToolResult,
        AcpResolvedTurnContext, AcpSessionHttp, AcpStreamContext, AdapterContract,
        ToolExecutionRoute,
    },
};

const ACP_SESSIONS_PAGE_SIZE: i64 = 50;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/bears/{slug}/sessions", get(list_acp_sessions))
        .route("/bears/{slug}/sessions/{session_id}", get(get_acp_session))
        .route(
            "/bears/{slug}/sessions/{session_id}/prompt-memory",
            get(get_acp_session_prompt_memory),
        )
        .route(
            "/bears/{slug}/sessions/{session_id}/runtime",
            get(get_acp_session_runtime),
        )
        .route(
            "/bears/{slug}/sessions/{session_id}/mode",
            post(set_session_mode),
        )
        .route(
            "/bears/{slug}/sessions/{session_id}/adapter-environment",
            post(post_adapter_environment),
        )
        .route("/bears/{slug}/sessions/{session_id}/prompt", post(prompt))
        .route(
            "/bears/{slug}/sessions/{session_id}/tool-results/{tool_call_id}",
            post(tool_result),
        )
        .route(
            "/bears/{slug}/sessions/{session_id}/permissions/{permission_id}",
            post(permission_result),
        )
        .route(
            "/bears/{slug}/sessions/{session_id}/close",
            post(close_session),
        )
        .route(
            "/bears/{slug}/sessions/{session_id}/cancel",
            post(cancel_session),
        )
        .route(
            "/bears/{slug}/sessions/{session_id}/compact",
            post(compact_session),
        )
        .route("/bears/{slug}/conversations", get(conversations))
        .route(
            "/bears/{slug}/conversations/{conversation_id}/history",
            get(conversation_history),
        )
        .route("/bears/{slug}/auth-check", get(auth_check))
}

pub(crate) use self::client::{
    acp_pair_den_tool_descriptors, merge_acp_pair_tool_descriptors, new_acp_conversation_id,
    normalize_acp_client, requested_mode_from_prompt, tools_enabled_for_client,
};
pub(crate) use self::config::{
    acp_debug_event_sample_chars, acp_debug_ui_enabled, acp_stream_tokens_enabled,
    acp_text_chunk_chars, acp_tool_timeout_ms_for_provider,
};
pub(crate) use self::http_types::{
    AcpCompactionStatusResponse, AcpConversationHistoryMessage, AcpConversationRow,
    AcpErrorResponse, AcpPromptRequest, AcpToolResultResponse,
};
use self::http_types::{
    AcpAdapterEnvironmentRequest, AcpCloseSessionResponse, AcpConversationHistoryQuery,
    AcpConversationHistoryResponse, AcpConversationsQuery, AcpConversationsResponse,
    AcpPermissionDecisionRequest, AcpPermissionDecisionResponse, AcpPromptMemoryQuery,
    AcpPromptMemoryResponse, AcpSessionsListHttpResponse, AcpSessionsListQuery,
    AcpSetModeRequest, AcpSetModeResponse,
};
use self::config::pending_web_fetch_approvals;
use self::config::PendingWebFetchApproval;
pub(crate) use self::history::normalize_acp_conversation_id;
pub(crate) use self::routing::{
    acp_archive_target_for_session, acp_den_provider_to_canonical_tool_name,
};
use self::routing::tool_execution_route;
pub(crate) use self::runtime_support::{
    cancel_runtime_runs_by_id_or_skip, looks_like_runtime_waiting_for_approval_error,
};
pub(crate) use self::pair_reflection_support::run_pair_reflection_summary;
pub(crate) use self::sessions::{acp_session_row_to_http_with_modes, resolve_acp_turn_context};
pub(crate) use self::workflow::{workflow_state_json, workflow_state_json_from_sources};

use self::{
    handlers::{
        auth::{auth_check, authenticate_acp_code_token_with_auth},
        conversations::{conversation_history, conversations},
        permissions::permission_result,
        session_lifecycle::{cancel_session, close_session, compact_session},
        sessions::{
            get_acp_session, get_acp_session_prompt_memory, get_acp_session_runtime,
            list_acp_sessions, post_adapter_environment, set_session_mode,
        },
        tool_results::tool_result,
    },
    responses::{acp_error_response, api_auth_error_response},
    sessions::{decode_acp_sessions_cursor, encode_acp_sessions_cursor},
};

async fn prompt(
    State(state): State<ApiState>,
    Path((slug, session_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<AcpPromptRequest>,
) -> impl IntoResponse {
    let request_id = Uuid::new_v4();
    if let Err(err) = check_adapter_contract(body.adapter_contract.as_ref()) {
        return acp_compatibility_error_response(err, request_id);
    }
    let result = async { prompt_inner(state, slug, session_id, headers, body, request_id).await }
        .instrument(tracing::info_span!("acp_prompt", request_id = %request_id))
        .await;
    match result {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => acp_error_response(err, request_id),
        Err(err) => api_auth_error_response(err, request_id),
    }
}


async fn prompt_inner(
    state: ApiState,
    slug: String,
    session_id: String,
    headers: HeaderMap,
    body: AcpPromptRequest,
    request_id: Uuid,
) -> types::AcpPromptInnerResult {
    run_prompt_flow(state, slug, session_id, headers, body, request_id).await
}



