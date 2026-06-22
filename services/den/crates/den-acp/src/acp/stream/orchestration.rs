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
const ACP_EAGER_PREFIX_DRIVE_TIMEOUT_MS: u64 = 3000;

use crate::{
    acp::{
        acp_error_status_message, acp_stream_tokens_enabled,
        history::pending_session_title_update_event, GatewayEvent, AcpStreamContext,
    },
    service::DenState,
    core::{
        acp_runtime::{is_acp_history_target, AcpConversationResolution},
        user,
    },
};
use den_http::errors::CustomError;
use den_oauth::auth::ApiError;
use den_runtime::{
    acp_sessions,
    client_tools::ResolvedSessionPolicy,
    bears::Bear,
    conversation_events::{
            persist_canonical_conversation_record, CanonicalConversationRecord,
            ConversationEventProvenance, ConversationPersistenceContext,
        },
    role_runtime::{AcpTurnLifecycleContext, AcpTurnLifecycleRuntime},
    runtime_provider::RoleRuntimeBinding,
};

use super::sse_stream::AcpRuntimeSseStream;

pub(in crate::acp) struct AcpStreamSetup {
    pub(in crate::acp) initial_events: Vec<GatewayEvent>,
    pub(in crate::acp) session_info_event_sent: bool,
    pub(in crate::acp) workspace_roots: Vec<String>,
    pub(in crate::acp) stream_tokens: bool,
    pub(in crate::acp) turn_runtime_context: String,
    pub(in crate::acp) prompt_memory_diagnostic: serde_json::Value,
}

pub(in crate::acp) async fn build_acp_stream_setup(
    state: &DenState,
    user_id: i32,
    bear: &Bear,
    session_id: &str,
    cwd: &str,
    client_context: &serde_json::Value,
    conversation_resolution: &AcpConversationResolution,
    current_activity_plan: &Option<den_docket::WorkPlanProjection>,
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
        initial_events.push(GatewayEvent::ConversationResolved {
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
    if let Some(plan_event) = current_activity_plan.clone().map(GatewayEvent::PlanUpdate) {
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

pub(in crate::acp) async fn build_acp_sse_response(
    state: DenState,
    user_id: i32,
    request_id: Uuid,
    session_id: &str,
    bear: &Bear,
    client: &str,
    prompt: &str,
    pair_runtime_binding: &RoleRuntimeBinding,
    conversation_resolution: &AcpConversationResolution,
    synthetic_session: &den_runtime::acp_sessions::AcpSessionRow,
    resolved_policy: &ResolvedSessionPolicy,
    current_activity_plan: &Option<den_docket::WorkPlanProjection>,
    merged_client_tool_descriptors: Option<serde_json::Value>,
    setup: AcpStreamSetup,
) -> Result<Result<Response, CustomError>, ApiError> {
    let client_tool_descriptors = merged_client_tool_descriptors.clone();
    let turn_lifecycle = AcpTurnLifecycleRuntime::new(
        state.tool_turns.clone(),
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
        Err(err) => return Ok(Err(err.into())),
    };
    let role_runtime = lifecycle_lease.role_runtime.clone();
    let turn_scope = lifecycle_lease.turn_scope.clone();
    let active_turn_guard = lifecycle_lease.active_turn_guard;
    let cancel_handle = lifecycle_lease.cancel_handle;
    let cancel_rx = lifecycle_lease.cancel_rx;

    let event_upstream = match crate::core::acp_turn_runner::start_acp_turn_event_stream_with_retries(
        crate::core::acp_turn_runner::TurnStartRequest {
            sqlx_pool: &state.sqlx_pool,
            config: &state.config,
            memory_stores: &state.memory_stores,
            request_id,
            run_id: None,
            user_id,
            session_id,
            bear_id: bear.id,
            bear_slug: &bear.slug,
            client,
            cwd: synthetic_session.cwd.as_deref(),
            binding: pair_runtime_binding,
            conversation_selection: &conversation_resolution.session_selection,
            upstream_target: &conversation_resolution.upstream_target,
            prompt,
            client_tools: client_tool_descriptors.clone(),
            runtime_context: Some(setup.turn_runtime_context.as_str()),
            runtime_context_len: setup.turn_runtime_context.len(),
            stream_tokens: setup.stream_tokens,
            api_style: None,
        },
    )
    .await
    {
        Ok(upstream) => upstream,
        Err(err) => return Ok(Err(err.into())),
    };

    let materialized_session = acp_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        session_id,
    )
    .await
    .map_err(|err| {
        ApiError::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "database",
            err.to_string(),
        )
    })?;
    let resolved_conversation_id = materialized_session
        .as_ref()
        .and_then(|session| session.resolved_conversation_id.clone())
        .or_else(|| {
            synthetic_session
                .resolved_conversation_id
                .clone()
                .or_else(|| {
                    conversation_resolution
                        .resolved_conversation
                        .as_ref()
                        .map(|conversation| conversation.id.clone())
                })
        });
    let canonical_conversation_id = resolved_conversation_id
        .as_deref()
        .filter(|id| is_acp_history_target(id))
        .map(str::to_string)
        .or_else(|| {
            conversation_resolution
                .history_target
                .as_ref()
                .map(|conversation| conversation.id.clone())
        })
        .unwrap_or_else(|| conversation_resolution.session_selection.clone());

    if is_acp_history_target(canonical_conversation_id.as_str()) {
        let provenance = ConversationEventProvenance {
            source: "acp_prompt".to_string(),
            scope_id: session_id.to_string(),
        };
        let mut content_json = provenance.as_content_json("user_prompt");
        content_json["role"] = serde_json::json!("user");
        content_json["acp_session_id"] = serde_json::json!(session_id);
        content_json["client"] = serde_json::json!(client);
        content_json["request_id"] = serde_json::json!(request_id.to_string());
        persist_canonical_conversation_record(
            &ConversationPersistenceContext {
                pool: state.sqlx_pool.clone(),
                bear_id: bear.id,
                user_id: Some(user_id),
                external_conversation_id: canonical_conversation_id.clone(),
                source_session_id: Some(session_id.to_string()),
                request_id: Some(request_id.to_string()),
                persistence_scope_id: session_id.to_string(),
                skip_persistence: false,
            },
            &CanonicalConversationRecord::visible_user_message(prompt, content_json, None),
        )
        .await
        .map_err(|err| {
            ApiError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "database",
                err.to_string(),
            )
        })?;
    }

    let session_policy = resolved_policy.to_json();
    let activity = current_activity_plan.as_ref().map(|plan| serde_json::json!(plan));
    let event_upstream = futures::StreamExt::map(event_upstream, |item| {
        item.map_err(den_http::errors::CustomError::from)
    });
    let stream = AcpRuntimeSseStream::new(
        event_upstream,
        AcpStreamContext {
            pool: state.sqlx_pool.clone(),
            tool_turns: state.tool_turns.clone(),
            user_id,
            user_profile: user::user_by_id(&state.sqlx_pool, user_id).await.ok(),
            bear_id: bear.id,
            bear_slug: bear.slug.clone(),
            acp_session_id: session_id.to_string(),
            client: client.to_string(),
            conversation_id: canonical_conversation_id,
            conversation_selection: conversation_resolution
                .session_selection
                .clone(),
            resolved_conversation_id,
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
            memory_stores: state.memory_stores.clone(),
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
            .tool_turns
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
