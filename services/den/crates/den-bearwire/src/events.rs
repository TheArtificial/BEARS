use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use bearwire_protocol::wire::{bearwire_event_to_json_rpc_notification, BearWireEvent};
use bytes::Bytes;
use den_http::errors::CustomError;
use den_runtime::{bearwire_events, turn_obligations};
use den_service::{client_sessions, DenState};
use serde_json::{json, Value};

pub(crate) use bearwire_protocol::methods::EventStreamQuery;

use crate::auth::authenticate_for_bear_slug;
use crate::methods::run::persist_run_failed;

pub(crate) fn events_sse_body(
    session_id: &str,
    events: Vec<bearwire_events::BearWireEventRow>,
    emit_initial_state: bool,
) -> Result<String, CustomError> {
    let mut frame = String::new();
    if events.is_empty() && emit_initial_state {
        let event = BearWireEvent::ephemeral(
            "session.state",
            json!({
                "session_id": session_id,
                "status": "connected",
                "note": "No persisted BearWire events for this session yet."
            }),
        );
        let notification = bearwire_event_to_json_rpc_notification(event);
        let payload = serde_json::to_string(&notification).map_err(|err| {
            CustomError::System(format!("serialize BearWire event failed: {err}"))
        })?;
        frame.push_str(&format!("data: {payload}\n\n"));
    } else if !events.is_empty() {
        for row in events {
            let notification = bearwire_event_to_json_rpc_notification(row.event);
            let payload = serde_json::to_string(&notification).map_err(|err| {
                CustomError::System(format!("serialize BearWire event failed: {err}"))
            })?;
            frame.push_str(&format!("id: {}\ndata: {payload}\n\n", row.sequence_no));
        }
    }
    Ok(frame)
}

async fn expire_session_client_obligations(
    state: &DenState,
    session: &client_sessions::ClientSessionRow,
) -> Result<(), CustomError> {
    let expired = turn_obligations::expire_open_client_obligations_for_session(
        &state.sqlx_pool,
        &session.client_session_id,
    )
    .await?;
    if expired.is_empty() {
        return Ok(());
    }

    let mut by_run = std::collections::BTreeMap::<String, Vec<Value>>::new();
    for obligation in expired {
        by_run
            .entry(obligation.run_id.clone())
            .or_default()
            .push(json!({
                "obligation_id": obligation.id,
                "kind": obligation.kind,
                "expected_responder_action": obligation.expected_responder_action,
                "tool_call_id": obligation.tool_call_id,
                "permission_id": obligation.permission_id,
                "timeout_ms": obligation.timeout_ms(),
                "created_at": obligation.created_at,
                "expires_at": obligation.expires_at(),
            }));
    }

    for (run_id, expired_obligations) in by_run {
        persist_run_failed(
            &state.sqlx_pool,
            &session.client_session_id,
            &run_id,
            session.bear_id,
            session.user_id,
            "client_obligation_timeout",
            "A required client obligation timed out before the armature/client responded."
                .to_string(),
            Some(json!({
                "expired_obligations": expired_obligations,
            })),
        )
        .await;
    }

    Ok(())
}

pub(crate) fn last_event_id(headers: &HeaderMap) -> Option<i64> {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
}

pub(crate) async fn events(
    State(state): State<DenState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<EventStreamQuery>,
) -> Result<Response, CustomError> {
    let user_id = authenticate_for_bear_slug(&state, &headers, &query.bear_slug).await?;
    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &query.bear_slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    expire_session_client_obligations(&state, &session).await?;
    let after = query.after.or_else(|| last_event_id(&headers));
    let events = bearwire_events::list_bearwire_events_after(
        &state.sqlx_pool,
        &session.client_session_id,
        after,
        100,
    )
    .await?;
    let frame = events_sse_body(&session.client_session_id, events, after.is_none())?;

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
    use axum::extract::{Path, Query, State};

    fn test_state(pool: sqlx::PgPool) -> DenState {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        DenState::new(
            pool,
            config.clone(),
            std::sync::Arc::new(den_service::bifrost::BifrostClient::new(config.as_ref())),
            den_memory::MemoryStoreManager::new(config.as_ref()),
        )
    }

    #[test]
    fn last_event_id_header_is_parsed_as_sequence_cursor() {
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", "42".parse().unwrap());
        assert_eq!(last_event_id(&headers), Some(42));
    }

    #[tokio::test]
    async fn events_endpoint_requires_bearer_token_for_bear_session() {
        let err = events(
            State(test_state(
                sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            )),
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
        let text = events_sse_body("session-test", Vec::new(), true).unwrap();
        assert!(text.starts_with("data: "));
        assert!(text.contains("\"method\":\"event\""));
        assert!(text.contains("\"type\":\"session.state\""));
    }

    #[tokio::test]
    async fn events_endpoint_does_not_emit_synthetic_state_for_empty_incremental_poll() {
        let text = events_sse_body("session-test", Vec::new(), false).unwrap();
        assert_eq!(text, "");
    }
}
