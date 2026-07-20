use axum::http::HeaderMap;
use den_core::BearProfile;
use den_docket::{DocketJobExecuteRequest, DocketJobListFilter, DocketService, PgDocketService};
use serde_json::{json, Value};
use uuid::Uuid;

use bearwire_protocol::{
    methods::{DocketJobsExecuteRequest, DocketJobsListRequest},
    wire::BearWireEvent,
};
use den_http::errors::CustomError;
use den_runtime::bearwire_events;
use den_service::{client_sessions, DenState};

use crate::auth::authenticated_bear;
use crate::methods::conversation::project_focus_title;
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

pub async fn docket_jobs_execute_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let request: DocketJobsExecuteRequest = parse_params(params)?;
    let job_id = Uuid::parse_str(&request.job_id)
        .map_err(|err| CustomError::ValidationError(format!("invalid job_id: {err}")))?;
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let service = PgDocketService::from_pool(&state.sqlx_pool);
    let outcome = service
        .execute_job(DocketJobExecuteRequest {
            bear_id: bear.id,
            job_id,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
            session_id: request.session_id.clone(),
            source_conversation_id: request.conversation_id.clone(),
            source_client_session_id: request
                .source_client_session_id
                .clone()
                .or_else(|| request.session_id.clone()),
        })
        .await?;

    if let Some(session_id) = request
        .source_client_session_id
        .as_deref()
        .or(request.session_id.as_deref())
    {
        if let Some(session) = client_sessions::find_for_user_bear_session_id(
            &state.sqlx_pool,
            user_id,
            bear.id,
            session_id,
        )
        .await?
        {
            let title = project_focus_title(session.conversation_title.clone(), true);
            if title.is_some() || session.conversation_title_updated_at.is_some() {
                let mut event = BearWireEvent::ephemeral(
                    "session_info_update",
                    json!({
                        "title": title,
                        "updated_at": session
                            .conversation_title_updated_at
                            .unwrap_or(session.updated_at)
                            .to_string(),
                    }),
                );
                event.bear_id = Some(bear.id.to_string());
                event.human_id = Some(user_id.to_string());
                event.session_id = Some(session.client_session_id.clone());
                let _ = bearwire_events::append_bearwire_event(
                    &state.sqlx_pool,
                    &session.client_session_id,
                    Some(bear.id),
                    Some(user_id),
                    event,
                )
                .await;
            }
        }
    }

    Ok(json!(outcome))
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
