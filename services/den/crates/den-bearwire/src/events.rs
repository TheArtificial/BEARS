use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use bytes::Bytes;
use serde::Deserialize;
use serde_json::json;

use den_http::errors::CustomError;
use den_service::{acp_sessions, DenState};
use den_runtime::{
    bearwire_events,
    runtime::bearwire_projection::wire::{bearwire_event_to_json_rpc_notification, BearWireEvent},
};

use crate::auth::authenticate_for_bear_slug;

#[derive(Debug, Deserialize)]
pub(crate) struct EventStreamQuery {
    pub(crate) bear_slug: String,
    pub(crate) after: Option<i64>,
}

pub(crate) fn events_sse_body(
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
            State(test_state(sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap())),
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
