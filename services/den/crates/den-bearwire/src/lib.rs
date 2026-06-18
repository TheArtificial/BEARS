use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use den_http::{acp_tokens, errors::CustomError};
use den_runtime::{
    acp_sessions, bearwire_events, bearwire_runs,
    bears::{db as bears_db, BearProfile},
    native_runtime::{continue_native_acp_turn_event_stream, start_native_acp_turn_event_stream},
    runtime::bearwire_projection::wire::{
        bearwire_event_to_json_rpc_notification, runtime_stream_event_to_bearwire_events,
        BearWireEvent,
    },
    runtime_contracts::{
        RoleRuntimeBinding, RuntimeApprovalDecision, RuntimeContinuation, RuntimeConversationRef,
        RuntimeToolResultStatus,
    },
    turn_runner::{default_tool_continue_stream_context, TurnContinueRequest, TurnStartRequest},
    DenState,
};

pub fn router() -> Router<DenState> {
    Router::new()
        .route("/v1/rpc", post(rpc))
        .route("/v1/sessions/{session_id}/events", get(events))
}

#[derive(Debug, Deserialize)]
struct EventStreamQuery {
    bear_slug: String,
    after: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn ok(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i64, message: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data,
            }),
        }
    }
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, CustomError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| CustomError::Authentication("missing Authorization bearer token".to_string()))?;
    value
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| CustomError::Authentication("expected Authorization: Bearer <token>".to_string()))
}

async fn authenticate_for_bear_slug(
    state: &DenState,
    headers: &HeaderMap,
    bear_slug: &str,
) -> Result<i32, CustomError> {
    let token = bearer_token(headers)?;
    if !acp_tokens::is_acp_token(token) {
        return Err(CustomError::Authentication(
            "expected a bear-scoped BEARS ACP token".to_string(),
        ));
    }
    acp_tokens::authenticate_for_bear_slug(
        &state.sqlx_pool,
        token,
        bear_slug,
        acp_tokens::acp_chat_scope(),
    )
    .await?
    .ok_or_else(|| CustomError::Authorization("token is not valid for this Bear".to_string()))
}

async fn authenticated_bear(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<(i32, den_runtime::bears::Bear), CustomError> {
    let bear_slug = params
        .get("bear_slug")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CustomError::ValidationError("bear_slug is required".to_string()))?;
    let user_id = authenticate_for_bear_slug(state, headers, bear_slug).await?;
    let bear = bears_db::bear_for_user_by_slug(&state.sqlx_pool, user_id, bear_slug)
        .await?
        .ok_or_else(|| CustomError::NotFound("Bear not found or token lacks access".to_string()))?;
    Ok((user_id, bear))
}

fn param_string(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn required_param_string(params: &Value, key: &str) -> Result<String, CustomError> {
    param_string(params, key)
        .ok_or_else(|| CustomError::ValidationError(format!("{key} is required")))
}

async fn session_open_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let conversation_id = param_string(params, "conversation_id").unwrap_or_else(|| session_id.clone());
    let runtime_session_id = param_string(params, "runtime_session_id")
        .unwrap_or_else(|| format!("bearwire:{}:{}", bear.id, session_id));
    let client = param_string(params, "client").unwrap_or_else(|| "bearwire".to_string());
    let cwd = param_string(params, "cwd");
    let current_mode = param_string(params, "mode");
    acp_sessions::upsert_session(
        &state.sqlx_pool,
        acp_sessions::UpsertAcpSession {
            user_id,
            bear_id: bear.id,
            bear_slug: bear.slug.clone(),
            acp_session_id: session_id.clone(),
            runtime_session_id,
            conversation_id,
            resolved_conversation_id: None,
            client,
            cwd,
            current_mode,
        },
    )
    .await?;
    let session = acp_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await?;
    let mut event = BearWireEvent::ephemeral(
        "session.opened",
        json!({
            "session_id": session_id,
            "bear_slug": bear.slug,
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
    let persisted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await?;
    Ok(json!({
        "ok": true,
        "session": session,
        "event_sequence": persisted.sequence_no,
    }))
}

async fn session_close_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let Some(session) = acp_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await? else {
        return Ok(json!({ "ok": true, "closed": false, "session_id": session_id }));
    };
    acp_sessions::mark_closed(&state.sqlx_pool, session.id).await?;
    let mut event = BearWireEvent::ephemeral(
        "session.closed",
        json!({
            "session_id": session_id,
            "bear_slug": bear.slug,
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
    let persisted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await?;
    Ok(json!({
        "ok": true,
        "closed": true,
        "session_id": session_id,
        "event_sequence": persisted.sequence_no,
    }))
}

async fn persist_runtime_event_as_bearwire(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    runtime_event: den_runtime::runtime_contracts::RuntimeStreamEvent,
    request_id: Uuid,
) {
    update_run_state_for_runtime_event(pool, run_id, &runtime_event, request_id).await;
    for mut event in runtime_stream_event_to_bearwire_events(runtime_event) {
        event.bear_id = Some(bear_id.to_string());
        event.human_id = Some(user_id.to_string());
        event.session_id = Some(session_id.to_string());
        if event.run_id.is_none() {
            event.run_id = Some(run_id.to_string());
        }
        if let Err(err) = bearwire_events::append_bearwire_event(
            pool,
            session_id,
            Some(bear_id),
            Some(user_id),
            event,
        )
        .await
        {
            tracing::warn!(error = %err, session_id = %session_id, "failed to persist BearWire runtime event");
        }
    }
}

async fn persist_run_failed(
    pool: &sqlx::PgPool,
    session_id: &str,
    run_id: &str,
    bear_id: uuid::Uuid,
    user_id: i32,
    request_id: Option<Uuid>,
    reason: &str,
    message: String,
) {
    let _ = bearwire_runs::transition_run(
        pool,
        run_id,
        bearwire_runs::BearWireRunState::Failed,
        None,
        None,
        request_id,
        Some(reason),
    )
    .await;
    let mut event = BearWireEvent::ephemeral(
        "run.failed",
        json!({
            "run_id": run_id,
            "message": message,
            "reason": reason,
        }),
    );
    event.bear_id = Some(bear_id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.to_string());
    event.run_id = Some(run_id.to_string());
    let _ = bearwire_events::append_bearwire_event(
        pool,
        session_id,
        Some(bear_id),
        Some(user_id),
        event,
    )
    .await;
}

async fn update_run_state_for_runtime_event(
    pool: &sqlx::PgPool,
    run_id: &str,
    event: &den_runtime::runtime_contracts::RuntimeStreamEvent,
    request_id: Uuid,
) {
    use den_runtime::runtime_contracts::{RuntimeSemanticEvent, RuntimeStreamEvent};
    match event {
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::ToolCallRequested {
            tool_call_id,
            approval_request_id,
            approval_required,
            ..
        }) => {
            let (state, active_tool, active_permission) = if *approval_required {
                (
                    bearwire_runs::BearWireRunState::WaitingForPermission,
                    None,
                    approval_request_id.as_deref(),
                )
            } else {
                (
                    bearwire_runs::BearWireRunState::WaitingForToolResult,
                    Some(tool_call_id.as_str()),
                    None,
                )
            };
            let _ = bearwire_runs::transition_run(
                pool,
                run_id,
                state,
                active_tool,
                active_permission,
                Some(request_id),
                None,
            )
            .await;
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCompleted { .. }) => {
            let _ = bearwire_runs::transition_run(
                pool,
                run_id,
                bearwire_runs::BearWireRunState::Completed,
                None,
                None,
                Some(request_id),
                Some("completed"),
            )
            .await;
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnFailed { category, .. }) => {
            let reason = format!("{:?}", category);
            let _ = bearwire_runs::transition_run(
                pool,
                run_id,
                bearwire_runs::BearWireRunState::Failed,
                None,
                None,
                Some(request_id),
                Some(&reason),
            )
            .await;
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::TurnCancelled { .. }) => {
            let _ = bearwire_runs::transition_run(
                pool,
                run_id,
                bearwire_runs::BearWireRunState::Cancelled,
                None,
                None,
                Some(request_id),
                Some("cancelled"),
            )
            .await;
        }
        RuntimeStreamEvent::Semantic(RuntimeSemanticEvent::Error { error_type, .. }) => {
            let _ = bearwire_runs::transition_run(
                pool,
                run_id,
                bearwire_runs::BearWireRunState::Failed,
                None,
                None,
                Some(request_id),
                error_type.as_deref().or(Some("error")),
            )
            .await;
        }
        _ => {}
    }
}

async fn run_start_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let prompt = required_param_string(params, "prompt")?;
    let conversation_id = param_string(params, "conversation_id").unwrap_or_else(|| session_id.clone());
    let client = param_string(params, "client").unwrap_or_else(|| "bearwire".to_string());
    let cwd = param_string(params, "cwd");
    let binding_id = bears_db::profile_binding_id(&state.sqlx_pool, bear.id, BearProfile::Pair)
        .await?
        .ok_or_else(|| CustomError::NotFound("Bear pair profile binding not found".to_string()))?;
    let binding = RoleRuntimeBinding {
        binding_id,
        compatibility_backend: Some("native".to_string()),
    };
    acp_sessions::upsert_session(
        &state.sqlx_pool,
        acp_sessions::UpsertAcpSession {
            user_id,
            bear_id: bear.id,
            bear_slug: bear.slug.clone(),
            acp_session_id: session_id.clone(),
            runtime_session_id: format!("bearwire:{}:{}", bear.id, session_id),
            conversation_id: conversation_id.clone(),
            resolved_conversation_id: None,
            client: client.clone(),
            cwd: cwd.clone(),
            current_mode: None,
        },
    )
    .await?;

    let run_id = format!("run_{}", Uuid::new_v4().simple());
    let run = bearwire_runs::create_run(
        &state.sqlx_pool,
        &run_id,
        &session_id,
        bear.id,
        user_id,
    )
    .await?;
    let mut accepted = BearWireEvent::ephemeral(
        "run.accepted",
        json!({
            "run_id": run_id.clone(),
            "session_id": session_id.clone(),
        }),
    );
    accepted.bear_id = Some(bear.id.to_string());
    accepted.human_id = Some(user_id.to_string());
    accepted.session_id = Some(session_id.clone());
    accepted.run_id = Some(run_id.clone());
    let accepted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        accepted,
    )
    .await?;

    let pool = state.sqlx_pool.clone();
    let config = state.config.clone();
    let memory_stores = state.memory_stores.clone();
    let bear_slug = bear.slug.clone();
    let bear_id = bear.id;
    let session_for_task = session_id.clone();
    let conversation_for_task = conversation_id.clone();
    let prompt_for_task = prompt.clone();
    let run_id_for_task = run_id.clone();
    tokio::spawn(async move {
        let request_id = Uuid::new_v4();
        let _ = bearwire_runs::transition_run(
            &pool,
            &run_id_for_task,
            bearwire_runs::BearWireRunState::Running,
            None,
            None,
            Some(request_id),
            None,
        )
        .await;
        let mut started = BearWireEvent::ephemeral(
            "run.started",
            json!({
                "run_id": run_id_for_task.clone(),
                "session_id": session_for_task.clone(),
            }),
        );
        started.bear_id = Some(bear_id.to_string());
        started.human_id = Some(user_id.to_string());
        started.session_id = Some(session_for_task.clone());
        started.run_id = Some(run_id_for_task.clone());
        let _ = bearwire_events::append_bearwire_event(
            &pool,
            &session_for_task,
            Some(bear_id),
            Some(user_id),
            started,
        )
        .await;
        let stream_result = start_native_acp_turn_event_stream(TurnStartRequest {
            sqlx_pool: &pool,
            config: config.as_ref(),
            memory_stores: &memory_stores,
            request_id,
            user_id,
            session_id: &session_for_task,
            bear_id,
            bear_slug: &bear_slug,
            client: &client,
            cwd: cwd.as_deref(),
            binding: &binding,
            conversation_selection: &conversation_for_task,
            upstream_target: &conversation_for_task,
            prompt: &prompt_for_task,
            client_tools: None,
            runtime_context: None,
            runtime_context_len: 0,
            stream_tokens: true,
        })
        .await;

        match stream_result {
            Ok(mut stream) => {
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(runtime_event) => {
                            persist_runtime_event_as_bearwire(
                                &pool,
                                &session_for_task,
                                &run_id_for_task,
                                bear_id,
                                user_id,
                                runtime_event,
                                request_id,
                            )
                            .await;
                        }
                        Err(err) => {
                            persist_run_failed(
                                &pool,
                                &session_for_task,
                                &run_id_for_task,
                                bear_id,
                                user_id,
                                Some(request_id),
                                "stream_error",
                                err.to_string(),
                            )
                            .await;
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                persist_run_failed(
                    &pool,
                    &session_for_task,
                    &run_id_for_task,
                    bear_id,
                    user_id,
                    Some(request_id),
                    "start_failed",
                    err.to_string(),
                )
                .await;
            }
        }
    });

    Ok(json!({
        "ok": true,
        "accepted": true,
        "run_id": run_id,
        "session_id": session_id,
        "event_sequence": accepted.sequence_no,
        "state": run.state,
    }))
}

async fn run_cancel_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let Some(session) = acp_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &bear.slug,
        &session_id,
    )
    .await? else {
        return Ok(json!({
            "ok": true,
            "cancelled": false,
            "session_id": session_id,
            "reason": "session_not_found",
        }));
    };

    let stream_cancel = state.acp_turn_cancellations.cancel_session(&session.acp_session_id);
    let active_turn = state.tool_turns.cancel_active_turn(&session.acp_session_id);
    let active_run = bearwire_runs::active_run_for_session(&state.sqlx_pool, &session_id).await?;
    if let Some(run) = &active_run {
        let _ = bearwire_runs::transition_run(
            &state.sqlx_pool,
            &run.run_id,
            bearwire_runs::BearWireRunState::Cancelled,
            None,
            None,
            None,
            Some("client_requested"),
        )
        .await?;
    }
    let cancelled = stream_cancel.is_some() || active_turn.is_some() || active_run.is_some();
    let run_ids = stream_cancel
        .as_ref()
        .map(|turn| turn.run_ids.clone())
        .unwrap_or_default();

    let mut event = BearWireEvent::ephemeral(
        "run.cancelled",
        json!({
            "session_id": session_id,
            "cancelled": cancelled,
            "run_ids": run_ids,
            "run_id": active_run.as_ref().map(|run| run.run_id.clone()),
            "reason": if cancelled { "client_requested" } else { "no_active_run" },
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
    let persisted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await?;

    Ok(json!({
        "ok": true,
        "cancelled": cancelled,
        "session_id": session_id,
        "run_ids": run_ids,
        "run_id": active_run.as_ref().map(|run| run.run_id.clone()),
        "active_turn": active_turn.map(|turn| turn.diagnostic()),
        "event_sequence": persisted.sequence_no,
    }))
}

fn spawn_continuation_task(
    state: &DenState,
    run: bearwire_runs::BearWireRunRow,
    binding_id: String,
    conversation_id: String,
    continuation: RuntimeContinuation,
) {
    let pool = state.sqlx_pool.clone();
    let config = state.config.clone();
    let memory_stores = state.memory_stores.clone();
    tokio::spawn(async move {
        let request_id = Uuid::new_v4();
        let _ = bearwire_runs::transition_run(
            &pool,
            &run.run_id,
            bearwire_runs::BearWireRunState::Continuing,
            None,
            None,
            Some(request_id),
            None,
        )
        .await;
        let binding = RoleRuntimeBinding {
            binding_id,
            compatibility_backend: Some("native".to_string()),
        };
        let result = continue_native_acp_turn_event_stream(
            TurnContinueRequest {
                sqlx_pool: &pool,
                config: config.as_ref(),
                memory_stores: &memory_stores,
                request_id,
                acp_session_id: &run.session_id,
                conversation: RuntimeConversationRef {
                    id: conversation_id,
                },
                binding: &binding,
                continuation,
                stream_context: default_tool_continue_stream_context(),
            },
            BearProfile::Pair,
        )
        .await;
        match result {
            Ok((_continuation, mut stream)) => {
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(runtime_event) => {
                            persist_runtime_event_as_bearwire(
                                &pool,
                                &run.session_id,
                                &run.run_id,
                                run.bear_id,
                                run.user_id,
                                runtime_event,
                                request_id,
                            )
                            .await;
                        }
                        Err(err) => {
                            persist_run_failed(
                                &pool,
                                &run.session_id,
                                &run.run_id,
                                run.bear_id,
                                run.user_id,
                                Some(request_id),
                                "continuation_stream_error",
                                err.to_string(),
                            )
                            .await;
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                persist_run_failed(
                    &pool,
                    &run.session_id,
                    &run.run_id,
                    run.bear_id,
                    run.user_id,
                    Some(request_id),
                    "continuation_start_failed",
                    err.to_string(),
                )
                .await;
            }
        }
    });
}

async fn client_tool_result_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let run_id = required_param_string(params, "run_id")?;
    let session_id = required_param_string(params, "session_id")?;
    let tool_call_id = required_param_string(params, "tool_call_id")?;
    let status = param_string(params, "status").unwrap_or_else(|| "ok".to_string());
    let Some(run) = bearwire_runs::get_run(&state.sqlx_pool, &run_id).await? else {
        return Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "reason": "run_not_found",
        }));
    };
    if run.bear_id != bear.id || run.user_id != user_id || run.session_id != session_id {
        return Err(CustomError::Authorization(
            "run does not belong to authenticated Bear/session".to_string(),
        ));
    }
    if !matches!(run.state.as_str(), "waiting_for_tool_result") {
        return Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "run_state": run.state,
        }));
    }
    if run.active_tool_call_id.as_deref() != Some(tool_call_id.as_str()) {
        return Err(CustomError::ValidationError(
            "tool_call_id does not match active BearWire run obligation".to_string(),
        ));
    }

    let payload = json!({
        "tool_call_id": tool_call_id,
        "status": status,
        "content": params.get("content").cloned().unwrap_or(Value::Null),
        "structured_content": params.get("structured_content").cloned().unwrap_or(Value::Null),
        "error": params.get("error").cloned().unwrap_or(Value::Null),
    });
    let record = bearwire_runs::record_client_result(
        &state.sqlx_pool,
        &run_id,
        "tool",
        &tool_call_id,
        payload.clone(),
    )
    .await?;
    match record {
        bearwire_runs::BearWireClientResultRecord::DuplicateConflict { existing_hash } => {
            return Err(CustomError::ValidationError(format!(
                "conflicting duplicate tool result for {tool_call_id}; existing hash {existing_hash}"
            )));
        }
        bearwire_runs::BearWireClientResultRecord::DuplicateIdentical { row } => {
            return Ok(json!({
                "ok": true,
                "duplicate": true,
                "result_id": row.id,
                "run_state": run.state,
            }));
        }
        bearwire_runs::BearWireClientResultRecord::Inserted { row } => {
            let event_type = if status == "ok" {
                "tool_call.completed"
            } else {
                "tool_call.failed"
            };
            let mut event = BearWireEvent::ephemeral(event_type, payload);
            event.bear_id = Some(bear.id.to_string());
            event.human_id = Some(user_id.to_string());
            event.session_id = Some(session_id.clone());
            event.run_id = Some(run_id.clone());
            event.subject = Some(format!("resource/tool_call/{tool_call_id}"));
            let persisted = bearwire_events::append_bearwire_event(
                &state.sqlx_pool,
                &session_id,
                Some(bear.id),
                Some(user_id),
                event,
            )
            .await?;
            let transitioned = bearwire_runs::transition_run(
                &state.sqlx_pool,
                &run_id,
                bearwire_runs::BearWireRunState::Continuing,
                None,
                None,
                run.active_request_id,
                None,
            )
            .await?;
            let session = acp_sessions::find_for_user_bear_session(
                &state.sqlx_pool,
                user_id,
                &bear.slug,
                &session_id,
            )
            .await?
            .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
            let binding_id = bears_db::profile_binding_id(&state.sqlx_pool, bear.id, BearProfile::Pair)
                .await?
                .ok_or_else(|| CustomError::NotFound("Bear pair profile binding not found".to_string()))?;
            let content = params
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    params
                        .get("structured_content")
                        .or_else(|| params.get("error"))
                        .map(|v| v.to_string())
                        .unwrap_or_default()
                });
            let continuation_status = match status.as_str() {
                "ok" => RuntimeToolResultStatus::Ok,
                "timeout" | "timed_out" => RuntimeToolResultStatus::Timeout,
                _ => RuntimeToolResultStatus::Error,
            };
            spawn_continuation_task(
                state,
                transitioned.clone().unwrap_or(run.clone()),
                binding_id,
                session
                    .resolved_conversation_id
                    .clone()
                    .unwrap_or(session.conversation_id),
                RuntimeContinuation::ToolResult {
                    tool_call_id: tool_call_id.clone(),
                    approval_request_id: run.active_permission_id.clone(),
                    status: continuation_status,
                    content,
                },
            );
            return Ok(json!({
                "ok": true,
                "duplicate": false,
                "result_id": row.id,
                "event_sequence": persisted.sequence_no,
                "run_state": transitioned.map(|run| run.state).unwrap_or_else(|| "unknown".to_string()),
                "continuation": "started",
            }));
        }
    }
}

async fn client_permission_result_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let run_id = required_param_string(params, "run_id")?;
    let session_id = required_param_string(params, "session_id")?;
    let permission_id = required_param_string(params, "permission_id")?;
    let decision = param_string(params, "decision").unwrap_or_else(|| "denied".to_string());
    let Some(run) = bearwire_runs::get_run(&state.sqlx_pool, &run_id).await? else {
        return Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "reason": "run_not_found",
        }));
    };
    if run.bear_id != bear.id || run.user_id != user_id || run.session_id != session_id {
        return Err(CustomError::Authorization(
            "run does not belong to authenticated Bear/session".to_string(),
        ));
    }
    if !matches!(run.state.as_str(), "waiting_for_permission") {
        return Ok(json!({
            "ok": false,
            "status": "late_result_ignored",
            "run_state": run.state,
        }));
    }
    if run.active_permission_id.as_deref() != Some(permission_id.as_str()) {
        return Err(CustomError::ValidationError(
            "permission_id does not match active BearWire run obligation".to_string(),
        ));
    }

    let normalized_decision = match decision.as_str() {
        "approved" | "approve" | "granted" | "allow" => "granted",
        "denied" | "deny" | "rejected" | "reject" => "denied",
        "timeout" | "timed_out" => "expired",
        other => {
            return Err(CustomError::ValidationError(format!(
                "unsupported permission decision: {other}"
            )));
        }
    };
    let payload = json!({
        "permission_id": permission_id,
        "decision": normalized_decision,
        "reason": params.get("reason").cloned().unwrap_or(Value::Null),
    });
    let record = bearwire_runs::record_client_result(
        &state.sqlx_pool,
        &run_id,
        "permission",
        &permission_id,
        payload.clone(),
    )
    .await?;
    match record {
        bearwire_runs::BearWireClientResultRecord::DuplicateConflict { existing_hash } => {
            return Err(CustomError::ValidationError(format!(
                "conflicting duplicate permission result for {permission_id}; existing hash {existing_hash}"
            )));
        }
        bearwire_runs::BearWireClientResultRecord::DuplicateIdentical { row } => {
            return Ok(json!({
                "ok": true,
                "duplicate": true,
                "result_id": row.id,
                "run_state": run.state,
            }));
        }
        bearwire_runs::BearWireClientResultRecord::Inserted { row } => {
            let event_type = match normalized_decision {
                "granted" => "permission.granted",
                "expired" => "permission.expired",
                _ => "permission.denied",
            };
            let mut event = BearWireEvent::ephemeral(event_type, payload);
            event.bear_id = Some(bear.id.to_string());
            event.human_id = Some(user_id.to_string());
            event.session_id = Some(session_id.clone());
            event.run_id = Some(run_id.clone());
            event.subject = Some(format!("resource/permission_request/{permission_id}"));
            let persisted = bearwire_events::append_bearwire_event(
                &state.sqlx_pool,
                &session_id,
                Some(bear.id),
                Some(user_id),
                event,
            )
            .await?;
            let transitioned = bearwire_runs::transition_run(
                &state.sqlx_pool,
                &run_id,
                bearwire_runs::BearWireRunState::Continuing,
                None,
                None,
                run.active_request_id,
                None,
            )
            .await?;
            let session = acp_sessions::find_for_user_bear_session(
                &state.sqlx_pool,
                user_id,
                &bear.slug,
                &session_id,
            )
            .await?
            .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
            let binding_id = bears_db::profile_binding_id(&state.sqlx_pool, bear.id, BearProfile::Pair)
                .await?
                .ok_or_else(|| CustomError::NotFound("Bear pair profile binding not found".to_string()))?;
            let decision = if normalized_decision == "granted" {
                RuntimeApprovalDecision::Approve
            } else {
                RuntimeApprovalDecision::Deny
            };
            let reason = params
                .get("reason")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            spawn_continuation_task(
                state,
                transitioned.clone().unwrap_or(run.clone()),
                binding_id,
                session
                    .resolved_conversation_id
                    .clone()
                    .unwrap_or(session.conversation_id),
                RuntimeContinuation::ApprovalDecision {
                    approval_request_id: permission_id.clone(),
                    tool_call_id: run.active_tool_call_id.clone(),
                    decision,
                    reason,
                },
            );
            return Ok(json!({
                "ok": true,
                "duplicate": false,
                "result_id": row.id,
                "event_sequence": persisted.sequence_no,
                "run_state": transitioned.map(|run| run.state).unwrap_or_else(|| "unknown".to_string()),
                "continuation": "started",
            }));
        }
    }
}

async fn resource_update_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session_id = required_param_string(params, "session_id")?;
    let resource = params
        .get("resource")
        .cloned()
        .or_else(|| params.get("payload").cloned())
        .unwrap_or_else(|| json!({}));
    let resource_kind = resource
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let mut event = BearWireEvent::ephemeral(
        "resource.updated",
        json!({
            "session_id": session_id,
            "resource": resource,
        }),
    );
    event.bear_id = Some(bear.id.to_string());
    event.human_id = Some(user_id.to_string());
    event.session_id = Some(session_id.clone());
    event.subject = Some(format!("resource/{resource_kind}"));
    let persisted = bearwire_events::append_bearwire_event(
        &state.sqlx_pool,
        &session_id,
        Some(bear.id),
        Some(user_id),
        event,
    )
    .await?;

    Ok(json!({
        "ok": true,
        "event_sequence": persisted.sequence_no,
    }))
}

async fn session_state_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let Some(bear_slug) = params.get("bear_slug").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(json!({
            "status": "available",
            "note": "Provide bear_slug and optional session_id for authenticated BearWire session state.",
            "params": params,
        }));
    };
    let user_id = authenticate_for_bear_slug(state, headers, bear_slug).await?;
    if let Some(session_id) = params.get("session_id").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
        let session = acp_sessions::find_for_user_bear_session(
            &state.sqlx_pool,
            user_id,
            bear_slug,
            session_id,
        )
        .await?;
        return Ok(json!({
            "kind": "single",
            "bear_slug": bear_slug,
            "session": session,
        }));
    }

    let include_closed = params
        .get("include_closed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let limit = params
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(50)
        .clamp(1, 100);
    let sessions = acp_sessions::list_for_user_bear(
        &state.sqlx_pool,
        acp_sessions::SessionListParams {
            user_id,
            bear_slug,
            include_closed,
            cwd_filter: None,
            limit,
            cursor_updated_at: None,
            cursor_id: None,
        },
    )
    .await?;
    Ok(json!({
        "kind": "list",
        "bear_slug": bear_slug,
        "sessions": sessions,
    }))
}

async fn rpc(
    State(state): State<DenState>,
    headers: HeaderMap,
    Json(request): Json<JsonRpcRequest>,
) -> Result<impl IntoResponse, CustomError> {
    if request.jsonrpc.as_deref().unwrap_or("2.0") != "2.0" {
        return Ok(Json(JsonRpcResponse::error(
            request.id,
            -32600,
            "Invalid JSON-RPC version",
            None,
        )));
    }

    let response = match request.method.as_str() {
        "initialize" => JsonRpcResponse::ok(
            request.id,
            json!({
                "protocol": "bearwire",
                "version": 1,
                "server": {
                    "name": "den",
                    "version": den_http::build_info::snapshot().version,
                    "git_sha": den_http::build_info::snapshot().git_sha,
                },
                "bearwire": {
                    "rpc": "/bearwire/v1/rpc",
                    "events": "/bearwire/v1/sessions/{session_id}/events"
                },
                "legacy_acp_enabled": state.config.acp_gateway_enabled,
            }),
        ),
        "session.open" | "session.resume" => match session_open_result(&state, &headers, &request.params).await {
            Ok(result) => JsonRpcResponse::ok(request.id, result),
            Err(err) => JsonRpcResponse::error(
                request.id,
                -32001,
                format!("BearWire {} failed", request.method),
                Some(json!({ "error": err.to_string() })),
            ),
        },
        "session.close" => match session_close_result(&state, &headers, &request.params).await {
            Ok(result) => JsonRpcResponse::ok(request.id, result),
            Err(err) => JsonRpcResponse::error(
                request.id,
                -32001,
                "BearWire session.close failed",
                Some(json!({ "error": err.to_string() })),
            ),
        },
        "session.state" => match session_state_result(&state, &headers, &request.params).await {
            Ok(result) => JsonRpcResponse::ok(request.id, result),
            Err(err) => JsonRpcResponse::error(
                request.id,
                -32001,
                "BearWire session.state failed",
                Some(json!({ "error": err.to_string() })),
            ),
        },
        "run.cancel" => match run_cancel_result(&state, &headers, &request.params).await {
            Ok(result) => JsonRpcResponse::ok(request.id, result),
            Err(err) => JsonRpcResponse::error(
                request.id,
                -32001,
                "BearWire run.cancel failed",
                Some(json!({ "error": err.to_string() })),
            ),
        },
        "resource.update" => match resource_update_result(&state, &headers, &request.params).await {
            Ok(result) => JsonRpcResponse::ok(request.id, result),
            Err(err) => JsonRpcResponse::error(
                request.id,
                -32001,
                "BearWire resource.update failed",
                Some(json!({ "error": err.to_string() })),
            ),
        },
        "run.start" => match run_start_result(&state, &headers, &request.params).await {
            Ok(result) => JsonRpcResponse::ok(request.id, result),
            Err(err) => JsonRpcResponse::error(
                request.id,
                -32001,
                "BearWire run.start failed",
                Some(json!({ "error": err.to_string() })),
            ),
        },
        "client.tool.result" => match client_tool_result_result(&state, &headers, &request.params).await {
            Ok(result) => JsonRpcResponse::ok(request.id, result),
            Err(err) => JsonRpcResponse::error(
                request.id,
                -32001,
                "BearWire client.tool.result failed",
                Some(json!({ "error": err.to_string() })),
            ),
        },
        "client.permission.result" => match client_permission_result_result(&state, &headers, &request.params).await {
            Ok(result) => JsonRpcResponse::ok(request.id, result),
            Err(err) => JsonRpcResponse::error(
                request.id,
                -32001,
                "BearWire client.permission.result failed",
                Some(json!({ "error": err.to_string() })),
            ),
        },
        other => JsonRpcResponse::error(
            request.id,
            -32601,
            format!("Method not found: {other}"),
            Some(json!({ "method": other })),
        ),
    };

    Ok(Json(response))
}

fn events_sse_body(
    session_id: &str,
    events: Vec<bearwire_events::BearWireEventRow>,
) -> Result<String, CustomError> {
    let mut frame = String::new();
    if events.is_empty() {
        let event = BearWireEvent::ephemeral(
            "session.state",
            json!({
                "session_id": session_id,
                "status": "connected",
                "note": "No persisted BearWire events for this session yet."
            }),
        );
        let notification = bearwire_event_to_json_rpc_notification(event);
        let payload = serde_json::to_string(&notification)
            .map_err(|err| CustomError::System(format!("serialize BearWire event failed: {err}")))?;
        frame.push_str(&format!("data: {payload}\n\n"));
    } else {
        for row in events {
            let notification = bearwire_event_to_json_rpc_notification(row.event);
            let payload = serde_json::to_string(&notification)
                .map_err(|err| CustomError::System(format!("serialize BearWire event failed: {err}")))?;
            frame.push_str(&format!("id: {}\ndata: {payload}\n\n", row.sequence_no));
        }
    }
    Ok(frame)
}

fn last_event_id(headers: &HeaderMap) -> Option<i64> {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
}

async fn events(
    State(state): State<DenState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<EventStreamQuery>,
) -> Result<Response, CustomError> {
    let user_id = authenticate_for_bear_slug(&state, &headers, &query.bear_slug).await?;
    let session = acp_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &query.bear_slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let after = query.after.or_else(|| last_event_id(&headers));
    let events = bearwire_events::list_bearwire_events_after(
        &state.sqlx_pool,
        &session.acp_session_id,
        after,
        100,
    )
    .await?;
    let frame = events_sse_body(&session.acp_session_id, events)?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(Bytes::from(frame)))
        .map_err(|err| CustomError::System(format!("build BearWire SSE response failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use den_runtime::bears::{db as bears_db, db::BearParams};

    fn test_state(pool: sqlx::PgPool) -> DenState {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        DenState::new(
            pool,
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        )
    }

    async fn create_test_user(pool: &sqlx::PgPool) -> i32 {
        let suffix = Uuid::new_v4().simple().to_string();
        let username = format!("bw{}", &suffix[..16]);
        let email = format!("{username}@example.test");
        let (user_id,): (i32,) = sqlx::query_as(
            r#"
            INSERT INTO users (email, username, display_name, passhash)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
        )
        .bind(email)
        .bind(&username)
        .bind(format!("BearWire Test {username}"))
        .bind("unused-in-bearwire-tests")
        .fetch_one(pool)
        .await
        .expect("insert test user");
        user_id
    }

    async fn create_test_bear(pool: &sqlx::PgPool) -> (uuid::Uuid, String) {
        let suffix = Uuid::new_v4().simple().to_string();
        let slug = format!("bearwire-test-{}", &suffix[..12]);
        let bear_id = bears_db::create_bear(
            pool,
            BearParams {
                slug: &slug,
                name: "BearWire Test Bear",
                description: "BearWire integration test bear",
                system_prompt: "test",
                default_model: None,
                tools_enabled: None,
                letta_agent_type: None,
                letta_tool_ids: sqlx::types::Json(Vec::<String>::new()),
                context_profile: None,
            },
        )
        .await
        .expect("create Bear");
        (bear_id, slug)
    }

    async fn create_token_for_bear(
        pool: &sqlx::PgPool,
        user_id: i32,
        bear_id: uuid::Uuid,
    ) -> String {
        bears_db::grant_membership(pool, user_id, bear_id, Some(bears_db::BEAR_ROLE_ADMIN))
            .await
            .expect("grant membership");
        acp_tokens::create_for_bear(pool, user_id, bear_id, "BearWire test token")
            .await
            .expect("create token")
            .raw_token
    }

    fn bearer_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().expect("header value"),
        );
        headers
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn session_open_persists_event_and_events_replay(pool: sqlx::PgPool) {
        let user_id = create_test_user(&pool).await;
        let (bear_id, bear_slug) = create_test_bear(&pool).await;
        let token = create_token_for_bear(&pool, user_id, bear_id).await;
        let state = test_state(pool.clone());
        let session_id = format!("session-{}", Uuid::new_v4().simple());

        let response = rpc(
            State(state.clone()),
            bearer_headers(&token),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-open")),
                method: "session.open".to_string(),
                params: json!({
                    "bear_slug": bear_slug,
                    "session_id": session_id,
                    "conversation_id": "conv-bearwire-test",
                    "client": "bearwire-test"
                }),
            }),
        )
        .await
        .expect("session.open response")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await

            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["result"]["ok"], true);
        let sequence = value["result"]["event_sequence"].as_i64().unwrap();

        let replay = events(
            State(state),
            bearer_headers(&token),
            Path(session_id.clone()),
            Query(EventStreamQuery {
                bear_slug: value["result"]["session"]["bear_slug"].as_str().unwrap().to_string(),
                after: None,
            }),
        )
        .await
        .expect("events response");
        let replay_body = axum::body::to_bytes(replay.into_body(), usize::MAX)
            .await
            .unwrap();
        let replay_text = std::str::from_utf8(&replay_body).unwrap();
        assert!(replay_text.contains(&format!("id: {sequence}")), "{replay_text}");
        assert!(replay_text.contains("\"type\":\"session.opened\""), "{replay_text}");

        let replay_after = events(
            State(test_state(pool)),
            bearer_headers(&token),
            Path(session_id),
            Query(EventStreamQuery {
                bear_slug: value["result"]["session"]["bear_slug"].as_str().unwrap().to_string(),
                after: Some(sequence),
            }),
        )
        .await
        .expect("events response after cursor");
        let replay_after_body = axum::body::to_bytes(replay_after.into_body(), usize::MAX)
            .await
            .unwrap();
        let replay_after_text = std::str::from_utf8(&replay_after_body).unwrap();
        assert!(!replay_after_text.contains("session.opened"), "{replay_after_text}");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn client_result_recording_is_idempotent_and_detects_conflicts(pool: sqlx::PgPool) {
        let user_id = create_test_user(&pool).await;
        let (bear_id, _bear_slug) = create_test_bear(&pool).await;
        let run = bearwire_runs::create_run(
            &pool,
            "run-idempotency-test",
            "session-idempotency-test",
            bear_id,
            user_id,
        )
        .await
        .expect("create run");
        assert_eq!(run.state, "accepted");

        let first = bearwire_runs::record_client_result(
            &pool,
            "run-idempotency-test",
            "tool",
            "call-1",
            json!({ "status": "ok", "content": "same" }),
        )
        .await
        .expect("record first result");
        assert!(matches!(
            first,
            bearwire_runs::BearWireClientResultRecord::Inserted { .. }
        ));

        let duplicate = bearwire_runs::record_client_result(
            &pool,
            "run-idempotency-test",
            "tool",
            "call-1",
            json!({ "status": "ok", "content": "same" }),
        )
        .await
        .expect("record duplicate result");
        assert!(matches!(
            duplicate,
            bearwire_runs::BearWireClientResultRecord::DuplicateIdentical { .. }
        ));

        let conflict = bearwire_runs::record_client_result(
            &pool,
            "run-idempotency-test",
            "tool",
            "call-1",
            json!({ "status": "ok", "content": "different" }),
        )
        .await
        .expect("record conflicting result");
        assert!(matches!(
            conflict,
            bearwire_runs::BearWireClientResultRecord::DuplicateConflict { .. }
        ));
    }

    #[tokio::test]
    async fn initialize_returns_bearwire_capabilities() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let response = rpc(
            State(state),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-1")),
                method: "initialize".to_string(),
                params: json!({}),
            }),
        )
        .await
        .expect("initialize ok")
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn planned_v1_methods_are_recognized() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        for method in [
            "session.open",
            "session.resume",
            "session.close",
            "session.state",
            "run.start",
            "run.cancel",
            "client.tool.result",
            "client.permission.result",
            "resource.update",
        ] {
            let response = rpc(
                State(state.clone()),
                HeaderMap::new(),
                Json(JsonRpcRequest {
                    jsonrpc: Some("2.0".to_string()),
                    id: Some(json!(method)),
                    method: method.to_string(),
                    params: json!({ "session_id": "session-test" }),
                }),
            )
            .await
            .expect("rpc ok")
            .into_response();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert_ne!(value.pointer("/error/code"), Some(&json!(-32601)), "{method}");
        }
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let response = rpc(
            State(state),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-unknown")),
                method: "not.real".to_string(),
                params: json!({}),
            }),
        )
        .await
        .expect("rpc ok")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn session_open_with_bear_slug_requires_bearer_token() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let response = rpc(
            State(state),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-open")),
                method: "session.open".to_string(),
                params: json!({ "bear_slug": "meta", "session_id": "session-test" }),
            }),
        )
        .await
        .expect("rpc ok")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], -32001);
        assert!(value["error"]["data"]["error"].as_str().unwrap().contains("missing Authorization"));
    }

    #[tokio::test]
    async fn session_state_with_bear_slug_requires_bearer_token() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let response = rpc(
            State(state),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-state")),
                method: "session.state".to_string(),
                params: json!({ "bear_slug": "meta" }),
            }),
        )
        .await
        .expect("rpc ok")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], -32001);
        assert!(value["error"]["data"]["error"].as_str().unwrap().contains("missing Authorization"));
    }

    #[tokio::test]
    async fn run_start_with_bear_slug_requires_bearer_token() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let response = rpc(
            State(state),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-run")),
                method: "run.start".to_string(),
                params: json!({ "bear_slug": "meta", "session_id": "session-test", "prompt": "hello" }),
            }),
        )
        .await
        .expect("rpc ok")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], -32001);
        assert!(value["error"]["data"]["error"].as_str().unwrap().contains("missing Authorization"));
    }

    #[tokio::test]
    async fn run_cancel_with_bear_slug_requires_bearer_token() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let response = rpc(
            State(state),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-cancel")),
                method: "run.cancel".to_string(),
                params: json!({ "bear_slug": "meta", "session_id": "session-test" }),
            }),
        )
        .await
        .expect("rpc ok")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], -32001);
        assert!(value["error"]["data"]["error"].as_str().unwrap().contains("missing Authorization"));
    }

    #[tokio::test]
    async fn client_tool_result_with_bear_slug_requires_bearer_token() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let response = rpc(
            State(state),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-tool-result")),
                method: "client.tool.result".to_string(),
                params: json!({
                    "bear_slug": "meta",
                    "session_id": "session-test",
                    "run_id": "run-test",
                    "tool_call_id": "call-test",
                    "status": "ok"
                }),
            }),
        )
        .await
        .expect("rpc ok")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], -32001);
        assert!(value["error"]["data"]["error"].as_str().unwrap().contains("missing Authorization"));
    }

    #[tokio::test]
    async fn client_permission_result_with_bear_slug_requires_bearer_token() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let response = rpc(
            State(state),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-permission-result")),
                method: "client.permission.result".to_string(),
                params: json!({
                    "bear_slug": "meta",
                    "session_id": "session-test",
                    "run_id": "run-test",
                    "permission_id": "perm-test",
                    "decision": "approved"
                }),
            }),
        )
        .await
        .expect("rpc ok")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], -32001);
        assert!(value["error"]["data"]["error"].as_str().unwrap().contains("missing Authorization"));
    }

    #[tokio::test]
    async fn resource_update_with_bear_slug_requires_bearer_token() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let response = rpc(
            State(state),
            HeaderMap::new(),
            Json(JsonRpcRequest {
                jsonrpc: Some("2.0".to_string()),
                id: Some(json!("req-resource")),
                method: "resource.update".to_string(),
                params: json!({
                    "bear_slug": "meta",
                    "session_id": "session-test",
                    "resource": { "kind": "acp_adapter", "id": "armature-test" }
                }),
            }),
        )
        .await
        .expect("rpc ok")
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["error"]["code"], -32001);
        assert!(value["error"]["data"]["error"].as_str().unwrap().contains("missing Authorization"));
    }

    #[test]
    fn last_event_id_header_is_parsed_as_sequence_cursor() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", "42".parse().unwrap());
        assert_eq!(last_event_id(&headers), Some(42));
    }

    #[tokio::test]
    async fn events_endpoint_requires_bearer_token_for_bear_session() {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        let state = DenState::new(
            sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            config.clone(),
            std::sync::Arc::new(den_runtime::bifrost::BifrostClient::new(config.as_ref())),
            den_runtime::memory::MemoryStoreManager::new(config.as_ref()),
        );
        let err = events(
            State(state),
            HeaderMap::new(),
            Path("session-test".to_string()),
            Query(EventStreamQuery {
                bear_slug: "meta".to_string(),
                after: None,
            }),
        )
        .await
        .expect_err("missing auth should error");
        assert!(err.to_string().contains("missing Authorization"));
    }

    #[tokio::test]
    async fn events_endpoint_emits_json_rpc_event_notification() {
        let text = events_sse_body("session-test", Vec::new()).unwrap();
        assert!(text.starts_with("data: "));
        assert!(text.contains("\"method\":\"event\""));
        assert!(text.contains("\"type\":\"session.state\""));
    }
}
