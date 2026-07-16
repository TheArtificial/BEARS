use axum::http::HeaderMap;
use den_docket::{DocketJobListFilter, DocketService, PgDocketService};
use serde_json::{json, Value};

use bearwire_protocol::methods::DocketJobsListRequest;
use den_http::errors::CustomError;
use den_service::{client_sessions, DenState};

use crate::auth::authenticated_bear;
use crate::methods::parse_params;

pub async fn docket_jobs_list_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let request: DocketJobsListRequest = parse_params(params)?;
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let source_conversation_id = source_conversation_id(state, user_id, bear.id, &request).await?;
    let service = PgDocketService::from_pool(&state.sqlx_pool);
    let jobs = service
        .list_jobs(
            bear.id,
            DocketJobListFilter {
                include_cancelled: request.include_cancelled.unwrap_or(false),
                source_conversation_id,
                limit: request.limit.unwrap_or(50),
                ..DocketJobListFilter::default()
            },
        )
        .await?;

    Ok(json!({
        "jobs": jobs,
    }))
}

async fn source_conversation_id(
    state: &DenState,
    user_id: i32,
    bear_id: uuid::Uuid,
    request: &DocketJobsListRequest,
) -> Result<Option<String>, CustomError> {
    if let Some(conversation_id) = request.conversation_id.as_ref() {
        return Ok(Some(conversation_id.clone()));
    }
    let Some(session_id) = request.session_id.as_ref() else {
        return Ok(None);
    };
    let session = client_sessions::find_for_user_bear_session_id(
        &state.sqlx_pool,
        user_id,
        bear_id,
        session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound(format!("session {session_id} not found")))?;
    Ok(Some(
        session
            .resolved_conversation_id
            .filter(|id| !id.trim().is_empty())
            .unwrap_or(session.conversation_id),
    ))
}
