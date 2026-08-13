use axum::http::HeaderMap;
use den_core::BearProfile;
use den_docket::{
    work_runs::{self, WorkRunListFilter},
    DocketExecutionTaskSettlement, DocketJobExecuteRequest, DocketJobListFilter,
    DocketOutcomeDisposition, DocketService, DocketTaskListFilter, DocketTaskStatus,
    PgDocketService,
};
use serde_json::{json, Value};
use uuid::Uuid;

use bearwire_protocol::methods::{
    DocketJobDiagnosticsRequest, DocketJobsExecuteRequest, DocketJobsListRequest,
    DocketJobsSettleTaskRequest,
};
use den_http::errors::CustomError;
use den_service::{
    artifacts::{self, ArtifactAccessContext, DocketArtifactTargetKind},
    client_sessions, DenState,
};

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
                include_archived: request.include_archived.unwrap_or(false),
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

pub async fn docket_job_diagnostics_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let request: DocketJobDiagnosticsRequest = parse_params(params)?;
    let job_id = Uuid::parse_str(&request.job_id)
        .map_err(|err| CustomError::ValidationError(format!("invalid job_id: {err}")))?;
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let service = PgDocketService::from_pool(&state.sqlx_pool);
    let job = service
        .get_job(bear.id, job_id)
        .await?
        .ok_or_else(|| CustomError::NotFound(format!("Docket job {job_id} not found")))?;
    let tasks = service
        .list_tasks(
            bear.id,
            DocketTaskListFilter {
                job_id: Some(job_id),
                include_descendants: true,
                limit: 500,
                ..DocketTaskListFilter::default()
            },
        )
        .await?;
    let context = ArtifactAccessContext {
        bear_id: bear.id,
        user_id: Some(user_id),
        profile: BearProfile::Pair,
    };
    let mut task_citations = Vec::with_capacity(tasks.len());
    for task in &tasks {
        let citations = artifacts::list_docket_artifact_citations(
            &state.sqlx_pool,
            bear.id,
            DocketArtifactTargetKind::Task,
            task.task.id,
            context.clone(),
        )
        .await?;
        task_citations.push(json!({ "task_id": task.task.id, "citations": citations }));
    }
    let runs = work_runs::list_work_runs(
        &state.sqlx_pool,
        WorkRunListFilter {
            bear_id: Some(bear.id),
            job_id: Some(job_id),
            limit: 200,
            ..WorkRunListFilter::default()
        },
    )
    .await?;
    let mut run_citations = Vec::with_capacity(runs.len());
    for run in &runs {
        let citations = artifacts::list_docket_artifact_citations(
            &state.sqlx_pool,
            bear.id,
            DocketArtifactTargetKind::Run,
            run.id,
            context.clone(),
        )
        .await?;
        run_citations.push(json!({ "run_id": run.id, "citations": citations }));
    }

    let run_diagnostics: Vec<Value> = runs.iter().map(work_run_diagnostic).collect();

    Ok(json!({
        "job": job,
        "tasks": tasks,
        "runs": run_diagnostics,
        "artifact_citations": {
            "tasks": task_citations,
            "runs": run_citations,
        },
    }))
}

fn work_run_diagnostic(run: &work_runs::WorkRunRow) -> Value {
    json!({
        "id": run.id,
        "job_run_id": run.job_run_id,
        "executing_task_id": run.executing_task_id,
        "attempt": run.attempt,
        "state": run.state,
        "result_summary": run.result_summary,
        "error": run.error,
        "queued_at": run.queued_at.to_string(),
        "started_at": run.started_at.map(|value| value.to_string()),
        "finished_at": run.finished_at.map(|value| value.to_string()),
    })
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
        .execute_job(execution_request(bear.id, user_id, job_id, &request))
        .await?;

    execution_result(outcome)
}

/// Repairs an explicitly reported stale Docket execution focus. This is separate
/// from execute so callers do not mistake recovery for a retry-safe dispatch.
pub async fn docket_jobs_reconcile_result(
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
        .reconcile_execution(execution_request(bear.id, user_id, job_id, &request))
        .await?;

    execution_result(outcome)
}

/// Settles Docket-owned work and returns its successor control result. Generic
/// Pair task settlement deliberately remains outside this job-scoped RPC.
pub async fn docket_jobs_settle_task_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let request: DocketJobsSettleTaskRequest = parse_params(params)?;
    let job_id = parse_uuid("job_id", &request.job_id)?;
    let task_id = parse_uuid("task_id", &request.task_id)?;
    let status = parse_docket_task_status(&request.status)?;
    let outcome_disposition = request
        .outcome_disposition
        .as_deref()
        .map(parse_outcome_disposition)
        .transpose()?;
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let service = PgDocketService::from_pool(&state.sqlx_pool);
    let outcome = service
        .settle_execution_task(DocketExecutionTaskSettlement {
            execution: DocketJobExecuteRequest {
                bear_id: bear.id,
                job_id,
                actor_role: BearProfile::Pair,
                actor_user_id: Some(user_id),
                actor_agent_id: None,
                session_id: request.session_id,
                source_conversation_id: request.conversation_id,
                source_client_session_id: request.source_client_session_id,
            },
            task_id,
            status,
            outcome_disposition,
            result_refs: request.result_refs,
            result_summary: request.result_summary,
        })
        .await?;
    execution_result(outcome)
}

fn parse_uuid(name: &str, value: &str) -> Result<Uuid, CustomError> {
    Uuid::parse_str(value)
        .map_err(|err| CustomError::ValidationError(format!("invalid {name}: {err}")))
}

fn parse_docket_task_status(value: &str) -> Result<DocketTaskStatus, CustomError> {
    match value {
        "done" => Ok(DocketTaskStatus::Done),
        "blocked" => Ok(DocketTaskStatus::Blocked),
        "cancelled" => Ok(DocketTaskStatus::Cancelled),
        _ => Err(CustomError::ValidationError(
            "Docket execution settlement status must be done, blocked, or cancelled".to_string(),
        )),
    }
}

fn parse_outcome_disposition(value: &str) -> Result<DocketOutcomeDisposition, CustomError> {
    match value {
        "completed" => Ok(DocketOutcomeDisposition::Completed),
        "no_change" => Ok(DocketOutcomeDisposition::NoChange),
        "delegated" => Ok(DocketOutcomeDisposition::Delegated),
        "blocked" => Ok(DocketOutcomeDisposition::Blocked),
        "failed" => Ok(DocketOutcomeDisposition::Failed),
        "cancelled" => Ok(DocketOutcomeDisposition::Cancelled),
        _ => Err(CustomError::ValidationError(format!(
            "invalid Docket outcome_disposition: {value}"
        ))),
    }
}

fn execution_request(
    bear_id: Uuid,
    user_id: i32,
    job_id: Uuid,
    request: &DocketJobsExecuteRequest,
) -> DocketJobExecuteRequest {
    DocketJobExecuteRequest {
        bear_id,
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
    }
}

fn execution_result(outcome: den_docket::DocketJobExecuteOutcome) -> Result<Value, CustomError> {
    let run = outcome.job.current_run.as_ref();
    let execution_state = run
        .map(|run| run.state.to_string())
        .unwrap_or("not_started".to_owned());

    Ok(json!({
        "action": "execution_requested",
        "execution": {
            "requested": true,
            "state": execution_state,
            "run_id": run.map(|run| run.id),
        },
        "outcome": outcome,
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
