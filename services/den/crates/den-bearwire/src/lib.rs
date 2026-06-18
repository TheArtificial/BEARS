use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use den_http::{acp_tokens, errors::CustomError};
use den_runtime::{acp_sessions, bearwire_events, bears::db as bears_db, DenState};
use den_runtime::runtime::bearwire_projection::wire::{
    bearwire_event_to_json_rpc_notification, BearWireEvent,
};

pub fn router() -> Router<DenState> {
    Router::new()
        .route("/v1/rpc", post(rpc))
        .route("/v1/sessions/{session_id}/events", get(events))
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
    let cancelled = stream_cancel.is_some() || active_turn.is_some();
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
        "active_turn": active_turn.map(|turn| turn.diagnostic()),
        "event_sequence": persisted.sequence_no,
    }))
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
        "run.start"
        | "client.tool.result"
        | "client.permission.result" => JsonRpcResponse::error(
            request.id,
            -32004,
            format!("BearWire method not implemented yet: {}", request.method),
            Some(json!({
                "method": request.method,
                "params": request.params,
                "legacy": "Use /acp/** during the BearWire parallel-operation period."
            })),
        ),
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

async fn events(
    State(state): State<DenState>,
    Path(session_id): Path<String>,
) -> Result<Response, CustomError> {
    let events = bearwire_events::list_bearwire_events_after(
        &state.sqlx_pool,
        &session_id,
        None,
        100,
    )
    .await?;
    let frame = events_sse_body(&session_id, events)?;

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

    #[tokio::test]
    async fn events_endpoint_emits_json_rpc_event_notification() {
        let text = events_sse_body("session-test", Vec::new()).unwrap();
        assert!(text.starts_with("data: "));
        assert!(text.contains("\"method\":\"event\""));
        assert!(text.contains("\"type\":\"session.state\""));
    }
}
