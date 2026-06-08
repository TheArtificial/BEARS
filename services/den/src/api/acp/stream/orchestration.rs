use axum::{
    body::Body,
    http::{header, HeaderName, HeaderValue, StatusCode},
    response::Response,
};
use bytes::Bytes;
use uuid::Uuid;

/// How long to eagerly drive a freshly-started ACP turn before returning the response, so a
/// tool obligation is registered up front. Bounds the wait when the turn parks on a tool
/// result or slow upstream; the remainder of the turn is driven lazily by the response body.
const ACP_EAGER_PREFIX_DRIVE_TIMEOUT_MS: u64 = 50;

use crate::{
    api::{
        acp::{
            acp_error_status_message, acp_stream_tokens_enabled,
            history::pending_session_title_update_event, AcpGatewayEvent, AcpStreamContext,
        },
        auth::ApiError,
        service::ApiState,
    },
    core::{
        acp_runtime::AcpConversationResolution,
        acp_tools::AcpResolvedSessionPolicy,
        bears::Bear,
        role_runtime::{AcpTurnLifecycleContext, AcpTurnLifecycleRuntime},
        runtime_provider::RoleRuntimeBinding,
        user,
    },
    errors::CustomError,
};

use super::sse_stream::AcpRuntimeSseStream;

pub(in crate::api::acp) struct AcpStreamSetup {
    pub(in crate::api::acp) initial_events: Vec<AcpGatewayEvent>,
    pub(in crate::api::acp) session_info_event_sent: bool,
    pub(in crate::api::acp) workspace_roots: Vec<String>,
    pub(in crate::api::acp) stream_tokens: bool,
    pub(in crate::api::acp) turn_runtime_context: String,
    pub(in crate::api::acp) prompt_memory_diagnostic: serde_json::Value,
}

pub(in crate::api::acp) async fn build_acp_stream_setup(
    state: &ApiState,
    user_id: i32,
    bear: &Bear,
    session_id: &str,
    cwd: &str,
    client_context: &serde_json::Value,
    conversation_resolution: &AcpConversationResolution,
    current_activity_plan: &Option<crate::core::work_plans::WorkPlanProjection>,
    plan_mode_context: &str,
    activity_context: &str,
    tool_prompt_context: &str,
    prompt_memory_diagnostic: serde_json::Value,
    prompt: &str,
    request_id: Uuid,
) -> Result<AcpStreamSetup, ApiError> {
    let mut initial_events = Vec::new();
    let mut session_info_event_sent = false;
    if let Some(conversation) = conversation_resolution.resolved_conversation.clone() {
        initial_events.push(AcpGatewayEvent::ConversationResolved {
            conversation_id: conversation.id,
        });
    }
    if let Some(title_event) = pending_session_title_update_event(
        &state.sqlx_pool,
        user_id,
        bear.id,
        &bear.slug,
        session_id,
    )
    .await
    .map_err(|err| {
        let (status, code, message) = acp_error_status_message(&err);
        ApiError::new(status, code, message)
    })? {
        session_info_event_sent = true;
        initial_events.push(title_event);
    }
    if let Some(plan_event) = current_activity_plan.clone().map(AcpGatewayEvent::PlanUpdate) {
        initial_events.push(plan_event);
    }
    let turn_runtime_context =
        format!("{plan_mode_context}{activity_context}{tool_prompt_context}");
    tracing::info!(
        %request_id,
        acp_session_id = %session_id,
        upstream_user_prompt_len = prompt.len(),
        turn_runtime_context_len = turn_runtime_context.len(),
        turn_runtime_context_has_trusted_mode_suffix =
            turn_runtime_context.contains("Trusted ACP session mode this turn:"),
        turn_runtime_context_has_system_reminder =
            turn_runtime_context.contains("<system-reminder>"),
        runtime_context_sent_as_user_content = false,
        "ACP final upstream prompt assembly"
    );
    let workspace_roots = client_context
        .get("workspace_roots")
        .or_else(|| client_context.get("workspaceRoots"))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec![cwd.to_string()]);
    let stream_tokens = acp_stream_tokens_enabled();

    Ok(AcpStreamSetup {
        initial_events,
        session_info_event_sent,
        workspace_roots,
        stream_tokens,
        turn_runtime_context,
        prompt_memory_diagnostic,
    })
}

pub(in crate::api::acp) async fn build_acp_sse_response(
    state: ApiState,
    user_id: i32,
    request_id: Uuid,
    session_id: &str,
    bear: &Bear,
    client: &str,
    prompt: &str,
    pair_runtime_binding: &RoleRuntimeBinding,
    conversation_resolution: &AcpConversationResolution,
    synthetic_session: &crate::core::acp_sessions::AcpSessionRow,
    resolved_policy: &AcpResolvedSessionPolicy,
    current_activity_plan: &Option<crate::core::work_plans::WorkPlanProjection>,
    merged_client_tool_descriptors: Option<serde_json::Value>,
    setup: AcpStreamSetup,
) -> Result<Result<Response, CustomError>, ApiError> {
    let client_tool_descriptors = merged_client_tool_descriptors.clone();
    let turn_lifecycle = AcpTurnLifecycleRuntime::new(
        state.acp_tool_turns.clone(),
        state.acp_turn_cancellations.clone(),
    );
    let lifecycle_lease = match turn_lifecycle.acquire_pair_turn(
        AcpTurnLifecycleContext {
            bear_id: bear.id,
            acp_session_id: session_id.to_string(),
            resolved_conversation_id: synthetic_session
                .resolved_conversation_id
                .clone()
                .or_else(|| {
                    conversation_resolution
                        .resolved_conversation
                        .as_ref()
                        .map(|conversation| conversation.id.clone())
                }),
        },
        request_id,
    ) {
        Ok(lease) => lease,
        Err(err) => return Ok(Err(err)),
    };
    let role_runtime = lifecycle_lease.role_runtime.clone();
    let turn_scope = lifecycle_lease.turn_scope.clone();
    let active_turn_guard = lifecycle_lease.active_turn_guard;
    let cancel_handle = lifecycle_lease.cancel_handle;
    let cancel_rx = lifecycle_lease.cancel_rx;

    let (upstream, parser) = match crate::core::acp_turn_runner::start_acp_turn_stream_with_retries(
        crate::core::acp_turn_runner::AcpTurnStartRequest {
            state: &state,
            request_id,
            user_id,
            session_id,
            bear_id: bear.id,
            bear_slug: &bear.slug,
            client,
            cwd: None,
            binding: pair_runtime_binding,
            conversation_selection: &conversation_resolution.session_selection,
            upstream_target: &conversation_resolution.upstream_target,
            prompt,
            client_tools: client_tool_descriptors.clone(),
            runtime_context_len: setup.turn_runtime_context.len(),
            stream_tokens: setup.stream_tokens,
        },
    )
    .await
    {
        Ok(upstream) => upstream,
        Err(err) => return Ok(Err(err)),
    };

    let session_policy = resolved_policy.to_json();
    let activity = current_activity_plan.as_ref().map(|plan| serde_json::json!(plan));
    let stream = AcpRuntimeSseStream::new(
        crate::core::acp_turn_runner::runtime_byte_stream_to_event_stream(upstream, parser),
        AcpStreamContext {
            pool: state.sqlx_pool.clone(),
            tool_turns: state.acp_tool_turns.clone(),
            user_id,
            user_profile: user::user_by_id(&state.sqlx_pool, user_id).await.ok(),
            bear_id: bear.id,
            bear_slug: bear.slug.clone(),
            acp_session_id: session_id.to_string(),
            client: client.to_string(),
            conversation_id: conversation_resolution
                .history_target
                .as_ref()
                .map(|conversation| conversation.id.clone())
                .unwrap_or_else(|| conversation_resolution.session_selection.clone()),
            conversation_selection: conversation_resolution
                .session_selection
                .clone(),
            resolved_conversation_id: synthetic_session
                .resolved_conversation_id
                .clone()
                .or_else(|| {
                    conversation_resolution
                        .resolved_conversation
                        .as_ref()
                        .map(|conversation| conversation.id.clone())
                }),
            upstream_target: conversation_resolution.upstream_target.clone(),
            workspace_roots: setup.workspace_roots.clone(),
            session_policy: Some(session_policy),
            activity,
            request_id,
            pair_agent_id: pair_runtime_binding.binding_id.clone(),
            config: state.config.clone(),
            role_runtime,
            turn_scope,
            prompt_memory_diagnostic: setup.prompt_memory_diagnostic.clone(),
        },
        setup.initial_events,
        setup.session_info_event_sent,
        active_turn_guard,
    )
    .with_cancel_registration(cancel_handle, cancel_rx);
    let request_id_header = HeaderValue::from_str(&request_id.to_string()).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_request_id",
            "invalid request id for response header",
        )
    })?;

    // Eagerly drive the turn just far enough to register any tool obligation *before*
    // returning the response, then hand the remainder to the normal lazy body. Adapter-local
    // tool obligations must exist in the shared registry as soon as the prompt response is
    // returned, so a `/tool-results` POST is accepted even if the client posts before it
    // starts reading the stream (and so a reconnect can resume the turn). Driving here on the
    // handler's own task — bounded, not a detached concurrent driver — avoids both the lazy
    // registration gap and the connection/guard lifecycle problems of a spawned turn task.
    let mut stream = Box::pin(stream);
    let mut prefix: Vec<Result<Bytes, std::io::Error>> = Vec::new();
    loop {
        // Stop as soon as the obligation is registered: we only need eager progress up to
        // that point, never past it into the result-wait.
        if !state
            .acp_tool_turns
            .pending_for_session(session_id)
            .is_empty()
        {
            break;
        }
        match tokio::time::timeout(
            std::time::Duration::from_millis(ACP_EAGER_PREFIX_DRIVE_TIMEOUT_MS),
            futures::StreamExt::next(&mut stream),
        )
        .await
        {
            // A frame was ready; buffer it and keep draining what is immediately available.
            Ok(Some(item)) => prefix.push(item),
            // Stream ended (e.g. a turn with no tool request); nothing left to defer.
            Ok(None) => break,
            // The stream is waiting on external input (a tool result) or slow upstream; stop
            // eager draining and let the body drive the rest lazily.
            Err(_) => break,
        }
    }
    let body_stream = futures::StreamExt::chain(futures::stream::iter(prefix), stream);

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .header(HeaderName::from_static("x-request-id"), request_id_header)
        .body(Body::from_stream(body_stream))
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "response_build",
                format!("response build: {e}"),
            )
        })
        .map(Ok)
}
