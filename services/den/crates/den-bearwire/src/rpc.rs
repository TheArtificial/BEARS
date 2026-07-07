use axum::{extract::State, http::HeaderMap, response::IntoResponse, Json};
use serde_json::{json, Value};

pub(crate) use bearwire_protocol::rpc::JsonRpcRequest;
use bearwire_protocol::rpc::JsonRpcResponse;
use den_http::errors::CustomError;
use den_service::DenState;

use crate::methods;

pub(crate) async fn rpc(
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
        "initialize" => JsonRpcResponse::ok(request.id, methods::initialize_result(&state)),
        "session.open" | "session.resume" => method_response(
            request.id,
            methods::session::session_open_result(&state, &headers, &request.params).await,
            format!("BearWire {} failed", request.method),
        ),
        "session.close" => method_response(
            request.id,
            methods::session::session_close_result(&state, &headers, &request.params).await,
            "BearWire session.close failed",
        ),
        "session.compact" => method_response(
            request.id,
            methods::session::session_compact_result(&state, &headers, &request.params).await,
            "BearWire session.compact failed",
        ),
        "session.state" => method_response(
            request.id,
            methods::session::session_state_result(&state, &headers, &request.params).await,
            "BearWire session.state failed",
        ),
        "session.model.get" => method_response(
            request.id,
            methods::session::session_model_get_result(&state, &headers, &request.params).await,
            "BearWire session.model.get failed",
        ),
        "session.model.set" => method_response(
            request.id,
            methods::session::session_model_set_result(&state, &headers, &request.params).await,
            "BearWire session.model.set failed",
        ),
        "conversation.history" => method_response(
            request.id,
            methods::conversation::conversation_history_result(&state, &headers, &request.params)
                .await,
            "BearWire conversation.history failed",
        ),
        "conversation.surface_history" => method_response(
            request.id,
            methods::conversation::conversation_surface_history_result(
                &state,
                &headers,
                &request.params,
            )
            .await,
            "BearWire conversation.surface_history failed",
        ),
        "run.cancel" => method_response(
            request.id,
            methods::run::run_cancel_result(&state, &headers, &request.params).await,
            "BearWire run.cancel failed",
        ),
        "resource.update" => method_response(
            request.id,
            methods::resource::resource_update_result(&state, &headers, &request.params).await,
            "BearWire resource.update failed",
        ),
        "run.start" => method_response(
            request.id,
            methods::run::run_start_result(&state, &headers, &request.params).await,
            "BearWire run.start failed",
        ),
        "client.tool.result" => method_response(
            request.id,
            methods::client::client_tool_result_result(&state, &headers, &request.params).await,
            "BearWire client.tool.result failed",
        ),
        "client.permission.result" => method_response(
            request.id,
            methods::client::client_permission_result_result(&state, &headers, &request.params)
                .await,
            "BearWire client.permission.result failed",
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

fn method_response(
    id: Option<Value>,
    result: Result<Value, CustomError>,
    message: impl Into<String>,
) -> JsonRpcResponse {
    match result {
        Ok(result) => JsonRpcResponse::ok(id, result),
        Err(err) => JsonRpcResponse::error(
            id,
            -32001,
            message,
            Some(json!({ "error": err.to_string() })),
        ),
    }
}
