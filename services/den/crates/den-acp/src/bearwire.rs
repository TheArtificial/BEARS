use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};


use crate::service::DenState;
use den_http::errors::CustomError;
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

async fn rpc(
    State(state): State<DenState>,
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
        "session.state" => JsonRpcResponse::ok(
            request.id,
            json!({
                "status": "available",
                "note": "BearWire session.state is a v1 shim in progress; legacy /acp session state remains authoritative during migration.",
                "params": request.params,
            }),
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

async fn events(Path(session_id): Path<String>) -> Result<Response, CustomError> {
    let event = BearWireEvent::ephemeral(
        "session.state",
        json!({
            "session_id": session_id,
            "status": "connected",
            "note": "BearWire HTTP+SSE endpoint is available; run events are not yet replayed from this stream."
        }),
    );
    let notification = bearwire_event_to_json_rpc_notification(event);
    let payload = serde_json::to_string(&notification)
        .map_err(|err| CustomError::System(format!("serialize BearWire event failed: {err}")))?;
    let frame = format!("data: {payload}\n\n");

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
    async fn events_endpoint_emits_json_rpc_event_notification() {
        let response = events(Path("session-test".to_string())).await.unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(text.starts_with("data: "));
        assert!(text.contains("\"method\":\"event\""));
        assert!(text.contains("\"type\":\"session.state\""));
    }
}
