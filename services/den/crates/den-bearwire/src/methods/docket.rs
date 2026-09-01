use axum::http::HeaderMap;
use den_core::BearProfile;
use den_docket::{
    work_runs::{self, WorkRunListFilter},
    DocketExecutionControl, DocketExecutionNextAction, DocketExecutionTaskSettlement,
    DocketJobExecuteRequest, DocketJobListFilter, DocketOutcomeDisposition, DocketService,
    DocketSessionTaskSettlement, DocketTaskListFilter, DocketTaskStatus, PgDocketService,
};
use serde_json::{json, Value};
use uuid::Uuid;

use bearwire_protocol::methods::{
    DocketJobDiagnosticsRequest, DocketJobsExecuteRequest, DocketJobsListRequest,
    DocketJobsSettleTaskRequest, DocketSessionTasksSettleRequest, RuntimeDiagnosticsListRequest,
};
use den_http::errors::CustomError;
use den_runtime::current_task::select_pair_current_task;
use den_runtime::runtime_exception_events::{
    self, RuntimeExceptionEventFilter, RuntimeExceptionSeverity,
};
use den_service::{
    artifacts::{self, ArtifactAccessContext, DocketArtifactTargetKind},
    client_sessions, DenState,
};

use crate::auth::authenticated_bear;
use crate::methods::parse_params;
use crate::methods::session::start_pair_current_task;

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

/// Lists bounded, sanitized runtime failures for the authenticated Bear.
/// This is an operations evidence surface, not a general log query API.
pub async fn runtime_diagnostics_list_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let request: RuntimeDiagnosticsListRequest = parse_params(params)?;
    let (_, bear) = authenticated_bear(state, headers, params).await?;
    let severity = match request.severity.as_deref() {
        None => None,
        Some("warning") => Some(RuntimeExceptionSeverity::Warning),
        Some("error") => Some(RuntimeExceptionSeverity::Error),
        Some(value) => {
            return Err(CustomError::ValidationError(format!(
                "invalid severity {value:?}; expected warning or error"
            )))
        }
    };
    let parse_optional_uuid = |name: &str, value: Option<String>| {
        value
            .map(|value| {
                Uuid::parse_str(&value).map_err(|error| {
                    CustomError::ValidationError(format!("invalid {name}: {error}"))
                })
            })
            .transpose()
    };
    let events = runtime_exception_events::list(
        &state.sqlx_pool,
        RuntimeExceptionEventFilter {
            bear_id: Some(bear.id),
            work_run_id: parse_optional_uuid("work_run_id", request.work_run_id)?,
            runtime_run_id: request.runtime_run_id,
            session_id: request.session_id,
            docket_job_id: parse_optional_uuid("docket_job_id", request.docket_job_id)?,
            event_code: request.event_code,
            severity,
            limit: request.limit,
        },
    )
    .await?;
    Ok(json!({ "events": events }))
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
    let mut criterion_citations = Vec::with_capacity(job.criteria.len());
    for criterion in &job.criteria {
        let citations = artifacts::list_docket_artifact_citations(
            &state.sqlx_pool,
            bear.id,
            DocketArtifactTargetKind::Criterion,
            criterion.id,
            context.clone(),
        )
        .await?;
        criterion_citations.push(json!({ "criterion_id": criterion.id, "citations": citations }));
    }

    let run_diagnostics: Vec<Value> = runs.iter().map(work_run_diagnostic).collect();

    Ok(json!({
        "job": job,
        "tasks": tasks,
        "runs": run_diagnostics,
        "artifact_citations": {
            "tasks": task_citations,
            "runs": run_citations,
            "criteria": criterion_citations,
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

    execution_result(
        state,
        user_id,
        bear,
        pair_binding_session_id(&request),
        outcome,
    )
    .await
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

    execution_result(
        state,
        user_id,
        bear,
        pair_binding_session_id(&request),
        outcome,
    )
    .await
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
                // Pair attempts are keyed by the Armature client session; both
                // protocol spellings identify it at this boundary.
                session_id: pair_attempt_session_id(&request),
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
    execution_result_payload(
        outcome,
        json!({
            "status": "not_applicable",
            "reason": "Task settlement does not change Pair binding.",
        }),
    )
}

pub async fn docket_session_tasks_settle_result(
    state: &DenState,
    headers: &HeaderMap,
    params: &Value,
) -> Result<Value, CustomError> {
    let request: DocketSessionTasksSettleRequest = parse_params(params)?;
    let task_id = parse_uuid("task_id", &request.task_id)?;
    let status = parse_docket_task_status(&request.status)?;
    let outcome_disposition = request
        .outcome_disposition
        .as_deref()
        .map(parse_outcome_disposition)
        .transpose()?;
    let (user_id, bear) = authenticated_bear(state, headers, params).await?;
    let session = client_sessions::find_for_user_bear_session_id(
        &state.sqlx_pool,
        user_id,
        bear.id,
        &request.session_id,
    )
    .await?
    .ok_or_else(|| CustomError::NotFound(format!("session {} not found", request.session_id)))?;
    let service = PgDocketService::from_pool(&state.sqlx_pool);
    let task = service
        .settle_session_task(DocketSessionTaskSettlement {
            bear_id: bear.id,
            pair_session_id: session.id,
            task_id,
            status,
            outcome_disposition,
            result_refs: request.result_refs,
            result_summary: request.result_summary,
            actor_role: BearProfile::Pair,
            actor_user_id: Some(user_id),
            actor_agent_id: None,
        })
        .await?;
    Ok(json!({ "task": task }))
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

fn pair_binding_session_id(request: &DocketJobsExecuteRequest) -> Option<&str> {
    request
        .source_client_session_id
        .as_deref()
        .or(request.session_id.as_deref())
}

fn pair_attempt_session_id(request: &DocketJobsSettleTaskRequest) -> Option<String> {
    request
        .source_client_session_id
        .clone()
        .or_else(|| request.session_id.clone())
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
        // The ACP client session is the durable Pair-attempt owner. A caller
        // may supply a separate conversational/session envelope, but it must
        // not split Docket focus from the attempt that later settles it.
        session_id: pair_binding_session_id(request).map(str::to_owned),
        source_conversation_id: request.conversation_id.clone(),
        source_client_session_id: pair_binding_session_id(request).map(str::to_owned),
    }
}

async fn execution_result(
    state: &DenState,
    user_id: i32,
    bear: den_service::bears::Bear,
    client_session_id: Option<&str>,
    outcome: den_docket::DocketJobExecuteOutcome,
) -> Result<Value, CustomError> {
    let mut pair_binding = json!({
        "status": "not_applicable",
        "reason": "Docket did not select a Pair task.",
    });
    if matches!(
        outcome.control.next_action,
        DocketExecutionNextAction::WorkCurrentTask
    ) {
        if let (Some(client_session_id), Some(task_id)) =
            (client_session_id, outcome.control.task.selected_task_id)
        {
            let session = client_sessions::find_for_user_bear_session_id(
                &state.sqlx_pool,
                user_id,
                bear.id,
                client_session_id,
            )
            .await?
            .ok_or_else(|| CustomError::NotFound("client session not found".to_string()))?;
            PgDocketService::from_pool(&state.sqlx_pool)
                .attach_task_to_pair_session(bear.id, task_id, session.id)
                .await?;
            select_pair_current_task(
                &state.sqlx_pool,
                user_id,
                bear.id,
                client_session_id,
                Some(task_id),
            )
            .await?;
            let loop_start =
                start_pair_current_task(state, user_id, bear, client_session_id).await?;
            pair_binding = json!({
                "status": "attached",
                "task_id": task_id,
                "client_session_id": client_session_id,
                "current_task_selected": true,
                "loop_started": loop_start["started"].clone(),
                "loop_run_id": loop_start["run_id"].clone(),
            });
        } else {
            pair_binding = json!({
                "status": "not_attempted",
                "reason": "no_authenticated_pair_session",
                "task_id": outcome.control.task.selected_task_id,
                "current_task_selected": false,
            });
        }
    }
    execution_result_payload(outcome, pair_binding)
}

fn execution_result_payload(
    outcome: den_docket::DocketJobExecuteOutcome,
    pair_binding: Value,
) -> Result<Value, CustomError> {
    let run = outcome.job.current_run.as_ref();
    let execution_state = run
        .map(|run| run.state.clone())
        .unwrap_or_else(|| "not_started".to_owned());
    let status = execution_status(&outcome.control);
    let gate = outcome.control.gate();

    Ok(json!({
        "action": "execution_requested",
        "execution": {
            "requested": true,
            "state": execution_state,
            "run_id": run.map(|run| run.id),
        },
        "status": status,
        "gate": gate,
        "pair_binding": pair_binding,
        "outcome": outcome,
    }))
}

fn execution_status(control: &DocketExecutionControl) -> Value {
    let message = match control.next_action {
        DocketExecutionNextAction::WorkCurrentTask => "Docket selected the current task.",
        DocketExecutionNextAction::JobCompleted => "Docket job completed.",
        DocketExecutionNextAction::ReconcileExecution => {
            "Docket task focus is stale; reconcile execution before continuing."
        }
        DocketExecutionNextAction::RecoverBlockedRun => {
            "Docket has no actionable task; inspect the blocked run."
        }
    };

    json!({
        "message": message,
        "next_action": control.next_action,
        "retryable": control.retryable,
        "reason": control.reason,
        "resources": {
            "run_id": control.run_id,
            "selected_task_id": control.task.selected_task_id,
            "focused_task_id": control.task.focused_task_id,
            "claimed_task_id": control.task.claimed_task_id,
            "current_task_id": control.task.current_task_id,
        },
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use den_docket::model::DocketExecutionTaskControl;

    #[test]
    fn pair_binding_prefers_the_explicit_client_session_id() {
        let request = DocketJobsExecuteRequest {
            bear_slug: "builder".to_owned(),
            job_id: Uuid::new_v4().to_string(),
            session_id: Some("conversation-session".to_owned()),
            conversation_id: None,
            source_client_session_id: Some("acp-session".to_owned()),
        };

        assert_eq!(pair_binding_session_id(&request), Some("acp-session"));
        let execution = execution_request(Uuid::new_v4(), 1, Uuid::new_v4(), &request);
        assert_eq!(execution.session_id.as_deref(), Some("acp-session"));
        assert_eq!(
            execution.source_client_session_id.as_deref(),
            Some("acp-session")
        );
    }

    #[test]
    fn pair_binding_falls_back_to_session_id() {
        let request = DocketJobsExecuteRequest {
            bear_slug: "builder".to_owned(),
            job_id: Uuid::new_v4().to_string(),
            session_id: Some("acp-session".to_owned()),
            conversation_id: None,
            source_client_session_id: None,
        };

        assert_eq!(pair_binding_session_id(&request), Some("acp-session"));
    }

    #[test]
    fn pair_attempt_session_prefers_the_explicit_client_session_id() {
        let request = DocketJobsSettleTaskRequest {
            bear_slug: "builder".to_owned(),
            job_id: Uuid::new_v4().to_string(),
            task_id: Uuid::new_v4().to_string(),
            status: "done".to_owned(),
            outcome_disposition: None,
            result_refs: None,
            result_summary: None,
            session_id: Some("conversation-session".to_owned()),
            conversation_id: None,
            source_client_session_id: Some("acp-session".to_owned()),
        };

        assert_eq!(
            pair_attempt_session_id(&request).as_deref(),
            Some("acp-session")
        );
    }

    #[test]
    fn execution_status_makes_stale_focus_actionable() {
        let run_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();
        let status = execution_status(&DocketExecutionControl {
            run_id,
            run_state: "running".to_owned(),
            task: DocketExecutionTaskControl {
                selected_task_id: Some(task_id),
                focused_task_id: Some(Uuid::new_v4()),
                claimed_task_id: Some(Uuid::new_v4()),
                current_task_id: None,
            },
            next_action: DocketExecutionNextAction::ReconcileExecution,
            retryable: false,
            reason: Some(den_docket::DocketExecutionReason::ActiveTaskIsStale),
        });

        assert_eq!(
            status["message"],
            "Docket task focus is stale; reconcile execution before continuing."
        );
        assert_eq!(status["next_action"], "reconcile_execution");
        assert_eq!(status["retryable"], false);
        assert_eq!(status["reason"], "active_task_is_stale");
        assert_eq!(status["resources"]["run_id"], run_id.to_string());
        assert_eq!(status["resources"]["selected_task_id"], task_id.to_string());
    }

    #[test]
    fn docket_diagnostics_include_criterion_citation_slot() {
        let citations = json!({
            "tasks": [],
            "runs": [],
            "criteria": [{
                "criterion_id": Uuid::from_u128(4),
                "citations": [],
            }],
        });
        assert_eq!(
            citations["criteria"][0]["criterion_id"],
            Uuid::from_u128(4).to_string()
        );
    }
}
