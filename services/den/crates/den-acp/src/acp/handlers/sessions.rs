use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::{
    acp::{
        check_adapter_contract,
        paths::{is_absolute_local_path, optional_absolute_cwd_filter},
        plan_approval_fallback_payload,
        responses::acp_error_response,
        tool_results::default_unavailable_context_budget,
        ACP_SESSIONS_PAGE_SIZE,
    },
    service::DenState,
    core::{
        acp_tokens,
        docket::{DocketService, PgDocketService, WorkPlanLookup},
    },
};
use den_http::errors::CustomError;
use den_oauth::auth;
use den_service::{
    acp_sessions,
    bears::{db as bears_db, BearProfile},
    conversation::persistence::{ensure_conversation_for_external_id, set_conversation_title},
    prompt_memory_block_store::list_prompt_memory_blocks_for_bear_profile,
    prompt_memory_blocks::PromptMemoryBlockState,
};
use den_runtime::{
    plan_mode,
    role_runtime::{RoleRuntime, RoleTurnScope},
};

use crate::acp::{
    acp_session_row_to_http_with_modes, decode_acp_sessions_cursor, encode_acp_sessions_cursor,
    format_acp_session_timestamp, resolve_acp_turn_context, tools_enabled_for_client,
    AcpAdapterEnvironmentRequest, AcpPromptMemoryQuery, AcpPromptMemoryResponse,
    AcpSessionsListHttpResponse, AcpSessionsListQuery, AcpSetModeRequest,
    AcpSetModeResponse,
};

use super::auth::{authenticate_acp_code_token, authenticate_acp_code_token_with_auth};

fn den_canonical_conversation_id(session: &acp_sessions::AcpSessionRow) -> Option<String> {
    let conversation_id = session.conversation_id.trim();
    if conversation_id.is_empty() {
        None
    } else {
        Some(conversation_id.to_string())
    }
}

fn runtime_conversation_id(session: &acp_sessions::AcpSessionRow) -> Option<String> {
    session
        .resolved_conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let conversation_id = session.conversation_id.trim();
            conversation_id
                .starts_with("conv-")
                .then(|| conversation_id.to_string())
        })
}


pub(in crate::acp) async fn get_acp_session_prompt_memory(
    State(state): State<DenState>,
    Path((slug, session_id)): Path<(String, String)>,
    Query(query): Query<AcpPromptMemoryQuery>,
    headers: HeaderMap,
) -> Response {
    let request_id = Uuid::new_v4();
    match get_acp_session_prompt_memory_inner(state, slug, session_id, query, headers).await {
        Ok(response) => response,
        Err(err) => acp_error_response(err, request_id),
    }
}

pub(super) async fn get_acp_session_prompt_memory_inner(
    state: DenState,
    slug: String,
    session_id: String,
    query: AcpPromptMemoryQuery,
    headers: HeaderMap,
) -> Result<Response, CustomError> {
    let user_id = authenticate_acp_code_token(&state, &headers, &slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, slug.trim())
        .await?
        .ok_or_else(|| {
            CustomError::NotFound("bear not found or you do not have access".to_string())
        })?;
    let row = acp_sessions::find_for_user_bear_session(&state.sqlx_pool, user_id, &bear.slug, session_id.trim())
        .await?
        .ok_or_else(|| CustomError::NotFound("ACP session not found".to_string()))?;
    let mut blocks = list_prompt_memory_blocks_for_bear_profile(&state.sqlx_pool, bear.id, BearProfile::Pair.as_str()).await?;
    if !query.include_archived {
        blocks.retain(|block| block.state != PromptMemoryBlockState::Archived);
    }
    if let Some(scope) = query.scope.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        blocks.retain(|block| serde_json::to_value(block.scope).ok().and_then(|v| v.as_str().map(str::to_string)).as_deref() == Some(scope));
    }
    if let Some(block_type) = query.block_type.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        blocks.retain(|block| serde_json::to_value(block.block_type).ok().and_then(|v| v.as_str().map(str::to_string)).as_deref() == Some(block_type));
    }
    if let Some(work_surface) = query.work_surface.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        blocks.retain(|block| block.work_surface.as_deref() == Some(work_surface));
    }
    let effective_session_id = query
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| row.acp_session_id.clone());
    blocks.retain(|block| block.session_id.is_none() || block.session_id.as_deref() == Some(effective_session_id.as_str()));
    let blocks_json = blocks
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| CustomError::Parsing(format!("prompt memory blocks serialize: {err}")))?;
    let prompt_memory_diagnostic = serde_json::json!({
        "status": if blocks_json.is_empty() { "empty" } else { "ok" },
        "source": "prompt_memory_blocks",
        "session_id": effective_session_id,
        "count": blocks_json.len(),
    });
    Ok(Json(AcpPromptMemoryResponse {
        ok: true,
        role: BearProfile::Pair.as_str().to_string(),
        count: blocks_json.len(),
        filters: serde_json::json!({
            "include_archived": query.include_archived,
            "scope": query.scope,
            "block_type": query.block_type,
            "work_surface": query.work_surface,
            "session_id": effective_session_id,
        }),
        prompt_memory_diagnostic,
        blocks: blocks_json,
    })
    .into_response())
}

pub(in crate::acp) async fn list_acp_sessions(
    State(state): State<DenState>,
    Path(slug): Path<String>,
    Query(query): Query<AcpSessionsListQuery>,
    headers: HeaderMap,
) -> Response {
    let request_id = Uuid::new_v4();
    match list_acp_sessions_inner(state, slug, query, headers).await {
        Ok(response) => response,
        Err(err) => acp_error_response(err, request_id),
    }
}

pub(super) async fn list_acp_sessions_inner(
    state: DenState,
    slug: String,
    query: AcpSessionsListQuery,
    headers: HeaderMap,
) -> Result<Response, CustomError> {
    let user_id = authenticate_acp_code_token(&state, &headers, &slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, slug.trim())
        .await?
        .ok_or_else(|| {
            CustomError::NotFound("bear not found or you do not have access".to_string())
        })?;
    let cursor = decode_acp_sessions_cursor(query.cursor.as_deref())?;
    let cwd_filter = optional_absolute_cwd_filter(query.cwd.as_deref())?;
    let fetch_limit = ACP_SESSIONS_PAGE_SIZE + 1;
    let mut rows = acp_sessions::list_for_user_bear(
        &state.sqlx_pool,
        acp_sessions::SessionListParams {
            user_id,
            bear_slug: &bear.slug,
            include_closed: query.include_closed,
            cwd_filter,
            limit: fetch_limit,
            cursor_updated_at: cursor.as_ref().map(|c| c.updated_at),
            cursor_id: cursor.as_ref().map(|c| c.id),
        },
    )
    .await?;
    let has_more = rows.len() > ACP_SESSIONS_PAGE_SIZE as usize;
    rows.truncate(ACP_SESSIONS_PAGE_SIZE as usize);
    let next_cursor = if has_more {
        rows.last().map(encode_acp_sessions_cursor)
    } else {
        None
    };
    let mut sessions = Vec::new();
    for row in rows {
        if row
            .cwd
            .as_deref()
            .map(str::trim)
            .filter(|s| is_absolute_local_path(s))
            .is_none()
        {
            tracing::warn!(
                acp_session_id = %row.acp_session_id,
                bear_slug = %row.bear_slug,
                "omitting ACP session list row with missing or non-absolute cwd"
            );
            continue;
        }
        let plan_mode = plan_mode::active_for_session(
            &state.sqlx_pool,
            user_id,
            bear.id,
            &row.acp_session_id,
        )
        .await?
        .map(serde_json::to_value)
        .transpose()?;
        sessions.push(acp_session_row_to_http_with_modes(&state.sqlx_pool, row, plan_mode).await?);
    }
    Ok(Json(AcpSessionsListHttpResponse {
        sessions,
        next_cursor,
    })
    .into_response())
}

pub(in crate::acp) async fn get_acp_session(
    State(state): State<DenState>,
    Path((slug, session_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let request_id = Uuid::new_v4();
    match get_acp_session_inner(state, slug, session_id, headers).await {
        Ok(response) => response,
        Err(err) => acp_error_response(err, request_id),
    }
}

pub(in crate::acp) async fn get_acp_session_runtime(
    State(state): State<DenState>,
    Path((slug, session_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let request_id = Uuid::new_v4();
    match get_acp_session_runtime_inner(state, slug, session_id, headers).await {
        Ok(response) => response,
        Err(err) => acp_error_response(err, request_id),
    }
}

pub(super) async fn get_acp_session_runtime_inner(
    state: DenState,
    slug: String,
    session_id: String,
    headers: HeaderMap,
) -> Result<Response, CustomError> {
    let user_id = authenticate_acp_code_token(&state, &headers, &slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, slug.trim())
        .await?
        .ok_or_else(|| {
            CustomError::NotFound("bear not found or you do not have access".to_string())
        })?;
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(CustomError::ValidationError(
            "session_id must not be empty".to_string(),
        ));
    }
    let row =
        acp_sessions::find_for_user_bear_session(&state.sqlx_pool, user_id, &bear.slug, session_id)
            .await?
            .ok_or_else(|| CustomError::NotFound("ACP session not found".to_string()))?;
    let plan_mode =
        plan_mode::active_for_session(&state.sqlx_pool, user_id, bear.id, session_id).await?;
    let activity_plan = PgDocketService::from_pool(&state.sqlx_pool)
        .get_visible_work_plan(
            bear.id,
            BearProfile::Pair,
            user_id,
            WorkPlanLookup {
                plan_id: None,
                source_conversation_id: den_canonical_conversation_id(&row),
                source_acp_session_id: Some(session_id.to_string()),
            },
        )
        .await?;
    let turn_context = resolve_acp_turn_context(&row, plan_mode.as_ref(), activity_plan.as_ref());
    let role_scope = RoleTurnScope::acp_pair(
        bear.id,
        session_id.to_string(),
        runtime_conversation_id(&row),
    );
    let role_runtime = RoleRuntime::with_turn_cancellations(
        state.tool_turns.clone(),
        state.acp_turn_cancellations.clone(),
    );
    let runtime = role_runtime.tool_turn_runtime_snapshot(session_id, &state.tool_turns);
    let active_turn = state
        .tool_turns
        .active_turn_for_session(session_id)
        .map(|turn| turn.diagnostic());
    let stream_turn = state
        .acp_turn_cancellations
        .active_for_session(session_id)
        .map(|turn| {
            serde_json::json!({
                "acp_session_id": turn.acp_session_id,
                "request_id": turn.request_id,
                "conversation_id": turn.conversation_id,
                "run_ids": turn.run_ids,
            })
        });
    let pending = state
        .tool_turns
        .pending_for_session(session_id)
        .into_iter()
        .map(|turn| turn.diagnostic())
        .collect::<Vec<_>>();
    let expired = state
        .tool_turns
        .expired_pending_for_session(session_id)
        .into_iter()
        .map(|turn| turn.diagnostic())
        .collect::<Vec<_>>();
    let adapter_environment = if tools_enabled_for_client(&row.client) {
        row.adapter_environment.clone().unwrap_or_else(|| {
            json!({
                "status": "unavailable",
                "note": "ACP adapter has not published an environment snapshot for this session yet.",
            })
        })
    } else {
        serde_json::json!({ "status": "not_applicable" })
    };
    Ok(Json(serde_json::json!({
        "ok": true,
        "bear_id": bear.id,
        "role": "pair",
        "channel_kind": "acp_session",
        "acp_session_id": session_id,
        "title": row.conversation_title,
        "conversation_title_updated_at": row
            .conversation_title_updated_at
            .map(format_acp_session_timestamp),
        "conversation_title_synced_at": row
            .conversation_title_synced_at
            .map(format_acp_session_timestamp),
        "conversation": {
            "conversation_id": den_canonical_conversation_id(&row),
            "session_selection": row.conversation_id,
            "resolved_conversation_id": row.resolved_conversation_id,
            "upstream_target": runtime_conversation_id(&row)
                .unwrap_or_else(|| "unresolved".to_string()),
        },
        "active_turn": {
            "active": active_turn.is_some(),
            "turn": active_turn,
        },
        "stream_turn": {
            "active": stream_turn.is_some(),
            "turn": stream_turn,
        },
        "pending_tools": pending,
        "expired_tools": expired,
        "tool_turns": role_runtime.pending_diagnostics(&role_scope),
        "runtime": runtime,
        "adapter_environment": adapter_environment,
        "context_budget": default_unavailable_context_budget(),
        "turn_state": turn_context.workflow_state,
        "session_policy": turn_context.policy.to_json(),
        "activity": activity_plan,
        "plan_mode": plan_mode,
    }))
    .into_response())
}

pub(in crate::acp) async fn set_session_mode(
    State(state): State<DenState>,
    Path((slug, session_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<AcpSetModeRequest>,
) -> Response {
    let request_id = Uuid::new_v4();
    match set_session_mode_inner(state, slug, session_id, headers, body).await {
        Ok(response) => response,
        Err(err) => acp_error_response(err, request_id),
    }
}

pub(in crate::acp) async fn post_adapter_environment(
    State(state): State<DenState>,
    Path((slug, session_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<AcpAdapterEnvironmentRequest>,
) -> Response {
    let request_id = Uuid::new_v4();
    match post_adapter_environment_inner(state, slug, session_id, headers, body).await {
        Ok(response) => response,
        Err(err) => acp_error_response(err, request_id),
    }
}

pub(super) async fn post_adapter_environment_inner(
    state: DenState,
    slug: String,
    session_id: String,
    headers: HeaderMap,
    body: AcpAdapterEnvironmentRequest,
) -> Result<Response, CustomError> {
    let token = auth::extract_bearer_token(&headers)
        .map_err(|err| CustomError::Authentication(err.message))?;
    let auth = authenticate_acp_code_token_with_auth(&state, &token, &slug).await?;
    if !acp_tokens::scopes_contains(&auth.scopes, acp_tokens::acp_tools_scope()) {
        return Err(CustomError::Authorization(
            "ACP token is missing required acp:tools scope".to_string(),
        ));
    }
    let session = acp_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        auth.user_id,
        &slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("ACP session not found".to_string()))?;
    acp_sessions::update_adapter_environment(
        &state.sqlx_pool,
        auth.user_id,
        session.bear_id,
        &session_id,
        &body.environment,
    )
    .await?;
    let client_title = body
        .conversation_title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            ["thread_title", "conversation_title", "title"]
                .iter()
                .find_map(|key| {
                    body.environment
                        .get(*key)
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                })
        });
    if let Some(client_title) = client_title {
        acp_sessions::update_client_conversation_title(
            &state.sqlx_pool,
            auth.user_id,
            session.bear_id,
            &session_id,
            Some(client_title),
        )
        .await?;
        let external_conversation_id = runtime_conversation_id(&session)
            .filter(|value| value.starts_with("conv-"));
        if let Some(external_conversation_id) = external_conversation_id {
            let _ = ensure_conversation_for_external_id(
                &state.sqlx_pool,
                session.bear_id,
                Some(auth.user_id),
                &external_conversation_id,
                Some(&session_id),
                Some(client_title),
            )
            .await;
            let _ = set_conversation_title(
                &state.sqlx_pool,
                session.bear_id,
                &external_conversation_id,
                client_title,
            )
            .await;
        }
    }
    Ok(Json(serde_json::json!({
        "accepted": true,
        "reason": "stored",
    }))
    .into_response())
}

pub(super) async fn set_session_mode_inner(
    state: DenState,
    slug: String,
    session_id: String,
    headers: HeaderMap,
    body: AcpSetModeRequest,
) -> Result<Response, CustomError> {
    if let Err(err) = check_adapter_contract(body.adapter_contract.as_ref()) {
        return Ok(crate::acp::compat::acp_compatibility_error_response(
            err,
            Uuid::new_v4(),
        ));
    }
    let user_id = authenticate_acp_code_token(&state, &headers, &slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, slug.trim())
        .await?
        .ok_or_else(|| {
            CustomError::NotFound("bear not found or you do not have access".to_string())
        })?;
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(CustomError::ValidationError(
            "session_id must not be empty".to_string(),
        ));
    }
    let requested_mode = body.mode.trim().to_ascii_lowercase();
    if !matches!(requested_mode.as_str(), "ask" | "plan" | "write") {
        return Err(CustomError::ValidationError(
            "mode must be one of ask, plan, write".to_string(),
        ));
    }
    let existing =
        acp_sessions::find_for_user_bear_session(&state.sqlx_pool, user_id, &bear.slug, session_id)
            .await?;
    let Some(_existing) = existing else {
        return Err(CustomError::NotFound("ACP session not found".to_string()));
    };

    let reason = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("User selected ACP session mode");

    let effective_mode;
    let message;
    match requested_mode.as_str() {
        "plan" => {
            plan_mode::enter_plan_mode(
                &state.sqlx_pool,
                plan_mode::EnterPlanModeParams {
                    user_id,
                    bear_id: bear.id,
                    bear_slug: bear.slug.clone(),
                    acp_session_id: session_id.to_string(),
                    reason: reason.to_string(),
                    requested_by: plan_mode::PlanModeRequestedBy::User,
                    previous_permission_mode: Some("ask".to_string()),
                },
            )
            .await?;
            acp_sessions::set_current_mode(&state.sqlx_pool, user_id, bear.id, session_id, "plan")
                .await?;
            effective_mode = "plan".to_string();
            message = "Plan mode entered. Planning is active; concrete tool use remains governed by Den policy and ACP client approval.".to_string();
        }
        "ask" => {
            if let Some(active) =
                plan_mode::active_for_session(&state.sqlx_pool, user_id, bear.id, session_id)
                    .await?
            {
                plan_mode::cancel_plan_mode(
                    &state.sqlx_pool,
                    user_id,
                    bear.id,
                    session_id,
                    Some(active.id),
                )
                .await?;
                message = "Plan mode cancelled; returned to Ask.".to_string();
            } else {
                message = "Returned to Ask according to Den session policy.".to_string();
            }
            acp_sessions::set_current_mode(&state.sqlx_pool, user_id, bear.id, session_id, "ask")
                .await?;
            effective_mode = "ask".to_string();
        }
        "write" => {
            let active_plan =
                plan_mode::active_for_session(&state.sqlx_pool, user_id, bear.id, session_id)
                    .await?;
            if let Some(active) = active_plan.as_ref() {
                match active.state.as_str() {
                    "submitted" => {
                        plan_mode::approve_plan_mode(
                            &state.sqlx_pool,
                            user_id,
                            bear.id,
                            session_id,
                            active.id,
                        )
                        .await?;
                        message = "Write mode enabled by user request; the submitted plan was approved by the authenticated ACP human.".to_string();
                    }
                    "active" => {
                        plan_mode::cancel_plan_mode(
                            &state.sqlx_pool,
                            user_id,
                            bear.id,
                            session_id,
                            Some(active.id),
                        )
                        .await?;
                        message = "Write mode enabled by user request; the unsubmitted plan draft was closed so the mode change could take effect.".to_string();
                    }
                    _ => {
                        message = "Write mode enabled by user request. Concrete tool use remains subject to Den policy and ACP client approval.".to_string();
                    }
                }
            } else {
                message = "Write mode enabled by user request. Concrete tool use remains subject to Den policy and ACP client approval.".to_string();
            }
            acp_sessions::set_current_mode(&state.sqlx_pool, user_id, bear.id, session_id, "write")
                .await?;
            effective_mode = "write".to_string();
            tracing::info!(
                bear_id = %bear.id,
                acp_session_id = %session_id,
                requested_mode = %requested_mode,
                effective_mode = %effective_mode,
                active_plan_id = ?active_plan.as_ref().map(|plan| plan.id),
                active_plan_state = ?active_plan.as_ref().map(|plan| plan.state.as_str()),
                "ACP session mode changed to write by authenticated user request"
            );
        }
        _ => unreachable!(),
    }

    let plan_mode_row =
        plan_mode::active_for_session(&state.sqlx_pool, user_id, bear.id, session_id).await?;
    let plan_mode = plan_mode_row
        .clone()
        .map(serde_json::to_value)
        .transpose()?;
    let synthetic_row = acp_sessions::AcpSessionRow {
        current_mode: effective_mode.clone(),
        ..acp_sessions::find_for_user_bear_session(
            &state.sqlx_pool,
            user_id,
            &bear.slug,
            session_id,
        )
        .await?
        .ok_or_else(|| CustomError::NotFound("ACP session not found".to_string()))?
    };
    let turn_context = resolve_acp_turn_context(&synthetic_row, plan_mode_row.as_ref(), None);
    Ok(Json(AcpSetModeResponse {
        requested_mode,
        effective_mode: turn_context.effective_mode,
        session_policy: turn_context.policy.to_json(),
        workflow_state: turn_context.workflow_state,
        plan_mode,
        message,
    })
    .into_response())
}

pub(super) async fn get_acp_session_inner(
    state: DenState,
    slug: String,
    session_id: String,
    headers: HeaderMap,
) -> Result<Response, CustomError> {
    let user_id = authenticate_acp_code_token(&state, &headers, &slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, slug.trim())
        .await?
        .ok_or_else(|| {
            CustomError::NotFound("bear not found or you do not have access".to_string())
        })?;
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(CustomError::ValidationError(
            "session_id must not be empty".to_string(),
        ));
    }
    let row =
        acp_sessions::find_for_user_bear_session(&state.sqlx_pool, user_id, &bear.slug, session_id)
            .await?
            .ok_or_else(|| CustomError::NotFound("ACP session not found".to_string()))?;
    let plan_mode =
        plan_mode::active_for_session(&state.sqlx_pool, user_id, bear.id, session_id).await?;
    let approval_fallback = plan_mode
        .as_ref()
        .filter(|plan| plan.state == "submitted")
        .map(plan_approval_fallback_payload);
    let mut response = serde_json::to_value(acp_session_row_to_http_with_modes(
        &state.sqlx_pool,
        row,
        plan_mode.map(serde_json::to_value).transpose()?,
    )
    .await?)?;
    if let Some(approval_fallback) = approval_fallback {
        response["approval_fallback"] = approval_fallback;
    }
    Ok(Json(response).into_response())
}
