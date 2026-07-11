use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Json,
};
use bearwire_protocol::wire::bearwire_event_to_json_rpc_notification;
use den_http::errors::CustomError;
use den_runtime::bearwire_events;
use den_service::{client_sessions, DenState};
use serde_json::{json, Value};

pub(crate) use bearwire_protocol::methods::EventPageQuery;

use crate::auth::authenticate_for_bear_slug;

const DEFAULT_EVENT_PAGE_LIMIT: i64 = 100;
const MAX_EVENT_PAGE_LIMIT: i64 = 500;

pub(crate) fn events_page_body(
    session_id: &str,
    after: Option<i64>,
    mut events: Vec<bearwire_events::BearWireEventRow>,
    requested_limit: Option<i64>,
) -> Result<Value, CustomError> {
    let limit = requested_limit
        .unwrap_or(DEFAULT_EVENT_PAGE_LIMIT)
        .clamp(1, MAX_EVENT_PAGE_LIMIT) as usize;
    let has_more = events.len() > limit;
    if has_more {
        events.truncate(limit);
    }
    let next_after = events.last().map(|event| event.sequence_no).or(after);
    let events = events
        .into_iter()
        .map(|row| {
            let notification = bearwire_event_to_json_rpc_notification(row.event);
            Ok(json!({
                "sequence": row.sequence_no,
                "event": notification.params,
            }))
        })
        .collect::<Result<Vec<_>, CustomError>>()?;

    Ok(json!({
        "ok": true,
        "session_id": session_id,
        "events": events,
        "next_after": next_after,
        "has_more": has_more,
    }))
}


pub(crate) async fn events_page(
    State(state): State<DenState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<EventPageQuery>,
) -> Result<Json<Value>, CustomError> {
    let user_id = authenticate_for_bear_slug(&state, &headers, &query.bear_slug).await?;
    let session = client_sessions::find_for_user_bear_session(
        &state.sqlx_pool,
        user_id,
        &query.bear_slug,
        &session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound("BearWire session not found".to_string()))?;
    let requested_limit = query.limit.unwrap_or(DEFAULT_EVENT_PAGE_LIMIT);
    let events = bearwire_events::list_bearwire_events_after(
        &state.sqlx_pool,
        &session.client_session_id,
        query.after,
        requested_limit.saturating_add(1),
    )
    .await?;
    Ok(Json(events_page_body(
        &session.client_session_id,
        query.after,
        events,
        Some(requested_limit),
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, Query, State};
    use bearwire_protocol::wire::BearWireEvent;
    use sqlx::types::time::OffsetDateTime;
    use uuid::Uuid;

    fn test_state(pool: sqlx::PgPool) -> DenState {
        let config = std::sync::Arc::new(den_core::config::Config::test_stub());
        DenState::new(
            pool,
            config.clone(),
            std::sync::Arc::new(den_service::bifrost::BifrostClient::new(config.as_ref())),
            den_memory::MemoryStoreManager::new(config.as_ref()),
        )
    }

    #[tokio::test]
    async fn events_page_endpoint_requires_bearer_token_for_bear_session() {
        let err = events_page(
            State(test_state(
                sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1/noop").unwrap(),
            )),
            HeaderMap::new(),
            Path("session-test".to_string()),
            Query(EventPageQuery {
                bear_slug: "meta".to_string(),
                after: None,
                limit: None,
            }),
        )
        .await
        .expect_err("missing auth should error");
        assert!(err.to_string().contains("missing Authorization"));
    }


    fn event_row(sequence_no: i64, event_type: &str) -> bearwire_events::BearWireEventRow {
        bearwire_events::BearWireEventRow {
            id: Uuid::new_v4(),
            sequence_no,
            session_id: "session-test".to_string(),
            event_type: event_type.to_string(),
            event: BearWireEvent::ephemeral(
                event_type,
                json!({
                    "sequence_no": sequence_no,
                }),
            ),
            created_at: OffsetDateTime::now_utc(),
        }
    }

    #[test]
    fn events_page_body_uses_server_owned_next_after() {
        let body = events_page_body(
            "session-test",
            Some(41),
            vec![
                event_row(42, "run.progress"),
                event_row(43, "message.delta"),
            ],
            Some(100),
        )
        .unwrap();

        assert_eq!(body["ok"], true);
        assert_eq!(body["next_after"], 43);
        assert_eq!(body["has_more"], false);
        assert_eq!(body["events"].as_array().unwrap().len(), 2);
        assert_eq!(body["events"][0]["sequence"], 42);
        assert_eq!(body["events"][0]["event"]["type"], "run.progress");
    }

    #[test]
    fn events_page_body_does_not_advance_empty_page_cursor() {
        let body = events_page_body("session-test", Some(41), Vec::new(), Some(100)).unwrap();

        assert_eq!(body["next_after"], 41);
        assert_eq!(body["has_more"], false);
        assert!(body["events"].as_array().unwrap().is_empty());
    }

    #[test]
    fn events_page_body_reports_has_more_without_advancing_past_returned_events() {
        let body = events_page_body(
            "session-test",
            Some(41),
            vec![
                event_row(42, "run.progress"),
                event_row(43, "message.delta"),
                event_row(44, "message.delta"),
            ],
            Some(2),
        )
        .unwrap();

        assert_eq!(body["has_more"], true);
        assert_eq!(body["next_after"], 43);
        assert_eq!(body["events"].as_array().unwrap().len(), 2);
    }
}
