// ROUTES: When modifying routes in this file, update /src/ROUTES.md.
//! Operator UI for autonomous work: Docket jobs with work tasks, work runs,
//! and their sandbox execution results. Read-mostly; the only controls are
//! the safe trio (dispatch, cancel, retry), all of which flow through the
//! same durable state the dispatch worker uses — this UI never touches the
//! sandbox host directly.

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use minijinja::context;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    auth_backend::AuthSession,
    errors::CustomError,
    web::{self, AppState},
};
use den_docket::work_runs::{self, WorkRunListFilter, WorkRunRow};
use den_docket::{DocketJobListFilter, DocketService, PgDocketService};
use den_service::bears::db as bears_db;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/work", get(index))
        .route("/work/jobs/{job_id}", get(job_detail))
        .route("/work/runs/{run_id}", get(run_detail))
        .route("/work/tasks/{task_id}/dispatch", post(dispatch_task))
        .route("/work/runs/{run_id}/cancel", post(cancel_run))
        .route("/work/runs/{run_id}/retry", post(retry_run))
}

#[derive(Serialize)]
struct RunView {
    id: String,
    state: String,
    is_active: bool,
    attempt: i32,
    bear_slug: String,
    job_id: String,
    task_id: String,
    root: Option<String>,
    sandbox_type: Option<String>,
    sandbox_strength: Option<String>,
    result_summary: Option<String>,
    error: Option<String>,
    cancel_requested: bool,
    queued_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    duration_secs: Option<i64>,
    cleanup_failed: bool,
}

fn ts(value: time::OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn run_view(run: &WorkRunRow, bear_slug: &str) -> RunView {
    let is_active = run
        .state_enum()
        .is_some_and(|state| !state.is_terminal());
    let duration_secs = run.started_at.map(|started| {
        let end = run.finished_at.unwrap_or_else(time::OffsetDateTime::now_utc);
        (end - started).whole_seconds()
    });
    let cleanup_failed = run
        .result_refs
        .as_ref()
        .and_then(|refs| refs.get("cleanup"))
        .and_then(serde_json::Value::as_str)
        == Some("failed");
    RunView {
        id: run.id.to_string(),
        state: run.state.clone(),
        is_active,
        attempt: run.attempt,
        bear_slug: bear_slug.to_string(),
        job_id: run.job_id.to_string(),
        task_id: run.task_id.to_string(),
        root: run.root_name.clone(),
        sandbox_type: run.sandbox_type.clone(),
        sandbox_strength: run.sandbox_strength.clone(),
        result_summary: run.result_summary.clone(),
        error: run.error.clone(),
        cancel_requested: run.cancel_requested,
        queued_at: ts(run.queued_at),
        started_at: run.started_at.map(ts),
        finished_at: run.finished_at.map(ts),
        duration_secs,
        cleanup_failed,
    }
}

fn require_user(auth_session: &AuthSession) -> Result<i32, CustomError> {
    auth_session
        .user
        .as_ref()
        .map(|user| user.id)
        .ok_or_else(|| CustomError::Authentication("login required".to_string()))
}

/// Bears the user is a member of, as (id → slug).
async fn member_bears(
    state: &AppState,
    user_id: i32,
) -> Result<std::collections::HashMap<Uuid, String>, CustomError> {
    let rows = bears_db::list_bears_for_user(state.sqlx_pool(), user_id).await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.bear.id, row.bear.slug))
        .collect())
}

async fn index(
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let user_id = require_user(&auth_session)?;
    let bears = member_bears(&state, user_id).await?;

    let mut runs: Vec<RunView> = Vec::new();
    let mut jobs_with_work: Vec<serde_json::Value> = Vec::new();
    for (bear_id, bear_slug) in &bears {
        let bear_runs = work_runs::list_work_runs(
            state.sqlx_pool(),
            WorkRunListFilter {
                bear_id: Some(*bear_id),
                limit: 50,
                ..WorkRunListFilter::default()
            },
        )
        .await?;
        runs.extend(bear_runs.iter().map(|run| run_view(run, bear_slug)));

        let service = PgDocketService::from_pool(state.sqlx_pool());
        let jobs = service
            .list_jobs(*bear_id, DocketJobListFilter::default())
            .await?;
        for job in jobs {
            jobs_with_work.push(serde_json::json!({
                "id": job.id.to_string(),
                "bear_slug": bear_slug,
                "goal": job.goal,
                "status": job.status,
                "work_surface_ref": job.work_surface_ref,
            }));
        }
    }
    runs.sort_by(|a, b| b.queued_at.cmp(&a.queued_at));
    let active: Vec<&RunView> = runs.iter().filter(|run| run.is_active).collect();

    web::render_template(
        &state,
        "work/index.html",
        auth_session,
        context! {
            title => "Work",
            active_runs => active,
            runs => runs,
            jobs => jobs_with_work,
        },
    )
    .await
}

async fn job_detail(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(job_id): Path<Uuid>,
) -> Result<Response, CustomError> {
    let user_id = require_user(&auth_session)?;
    let bears = member_bears(&state, user_id).await?;

    // Resolve which member bear owns this job.
    let owner: Option<(Uuid, String)> = {
        let row: Option<(Uuid,)> =
            sqlx::query_as("SELECT bear_id FROM bear_jobs WHERE id = $1")
                .bind(job_id)
                .fetch_optional(state.sqlx_pool())
                .await
                .map_err(den_core::DenError::from)?;
        row.and_then(|(bear_id,)| bears.get(&bear_id).map(|slug| (bear_id, slug.clone())))
    };
    let Some((bear_id, bear_slug)) = owner else {
        return Err(CustomError::NotFound("job not found".to_string()));
    };

    let service = PgDocketService::from_pool(state.sqlx_pool());
    let projection = service
        .get_job(bear_id, job_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("job not found".to_string()))?;
    let runs = work_runs::list_work_runs(
        state.sqlx_pool(),
        WorkRunListFilter {
            bear_id: Some(bear_id),
            job_id: Some(job_id),
            limit: 100,
            ..WorkRunListFilter::default()
        },
    )
    .await?;
    let run_views: Vec<RunView> = runs.iter().map(|run| run_view(run, &bear_slug)).collect();

    let task_states: std::collections::HashMap<Uuid, &str> = projection
        .task_states
        .iter()
        .map(|state| (state.task_id, state.status.as_str()))
        .collect();
    let tasks: Vec<serde_json::Value> = projection
        .tasks
        .iter()
        .map(|task| {
            let status = task_states.get(&task.id).copied().unwrap_or("pending");
            serde_json::json!({
                "id": task.id.to_string(),
                "title": task.title,
                "kind": task.kind,
                "assigned_to_role": task.assigned_to_role,
                "status": status,
                "is_work": task.assigned_to_role.as_deref() == Some("work"),
                "completion_criteria": task.completion_criteria.0,
            })
        })
        .collect();

    web::render_template(
        &state,
        "work/job.html",
        auth_session,
        context! {
            title => "Work job",
            bear_slug => bear_slug,
            job_id => job_id.to_string(),
            goal => projection.job.goal,
            status => projection.job.status,
            work_surface_ref => projection.job.work_surface_ref,
            tasks => tasks,
            runs => run_views,
        },
    )
    .await
}

async fn run_detail(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(run_id): Path<Uuid>,
) -> Result<Response, CustomError> {
    let user_id = require_user(&auth_session)?;
    let bears = member_bears(&state, user_id).await?;
    let run = work_runs::get_work_run(state.sqlx_pool(), run_id)
        .await?
        .filter(|run| bears.contains_key(&run.bear_id))
        .ok_or_else(|| CustomError::NotFound("work run not found".to_string()))?;
    let bear_slug = bears
        .get(&run.bear_id)
        .cloned()
        .unwrap_or_default();

    let dispatch_context =
        work_runs::get_work_run_dispatch_context(state.sqlx_pool(), run.id).await?;
    let refs = run.result_refs.clone().unwrap_or(serde_json::Value::Null);
    let log_tail = refs
        .get("log_tail")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let diff_patch = refs
        .get("diff_patch")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let changed_files: Vec<serde_json::Value> = refs
        .get("changed_files")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let armature_report = refs.get("armature_report").cloned();
    let turn_outcome = refs.get("turn_outcome").cloned();
    let work_surface = run.work_surface.clone();
    let usage = run.usage.clone();
    let view = run_view(&run, &bear_slug);
    let can_retry = !view.is_active;

    web::render_template(
        &state,
        "work/run.html",
        auth_session,
        context! {
            title => "Work run",
            run => view,
            job_goal => dispatch_context.job_goal,
            log_tail => log_tail,
            diff_patch => diff_patch,
            changed_files => changed_files,
            armature_report => armature_report,
            turn_outcome => turn_outcome,
            work_surface => work_surface,
            usage => usage,
            can_retry => can_retry,
        },
    )
    .await
}

async fn dispatch_task(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(task_id): Path<Uuid>,
) -> Result<Response, CustomError> {
    let user_id = require_user(&auth_session)?;
    let bears = member_bears(&state, user_id).await?;
    let row: Option<(Uuid, Option<Uuid>)> =
        sqlx::query_as("SELECT bear_id, job_id FROM bear_tasks WHERE id = $1")
            .bind(task_id)
            .fetch_optional(state.sqlx_pool())
            .await
            .map_err(den_core::DenError::from)?;
    let Some((bear_id, job_id)) = row.filter(|(bear_id, _)| bears.contains_key(bear_id)) else {
        return Err(CustomError::NotFound("task not found".to_string()));
    };
    work_runs::enqueue_work_run(
        state.sqlx_pool(),
        work_runs::WorkRunEnqueue {
            bear_id,
            task_id,
            root_name: None,
            git_ref: None,
            requested_by_user_id: Some(user_id),
        },
    )
    .await?;
    let destination = job_id.map_or_else(
        || "/work".to_string(),
        |job_id| format!("/work/jobs/{job_id}"),
    );
    Ok(Redirect::to(&destination).into_response())
}

async fn cancel_run(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(run_id): Path<Uuid>,
) -> Result<Response, CustomError> {
    let user_id = require_user(&auth_session)?;
    let bears = member_bears(&state, user_id).await?;
    let run = work_runs::get_work_run(state.sqlx_pool(), run_id)
        .await?
        .filter(|run| bears.contains_key(&run.bear_id))
        .ok_or_else(|| CustomError::NotFound("work run not found".to_string()))?;
    work_runs::request_work_run_cancel(state.sqlx_pool(), run.id, run.bear_id).await?;
    Ok(Redirect::to(&format!("/work/runs/{run_id}")).into_response())
}

async fn retry_run(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(run_id): Path<Uuid>,
) -> Result<Response, CustomError> {
    let user_id = require_user(&auth_session)?;
    let bears = member_bears(&state, user_id).await?;
    let run = work_runs::get_work_run(state.sqlx_pool(), run_id)
        .await?
        .filter(|run| bears.contains_key(&run.bear_id))
        .ok_or_else(|| CustomError::NotFound("work run not found".to_string()))?;
    let retry = work_runs::enqueue_work_run(
        state.sqlx_pool(),
        work_runs::WorkRunEnqueue {
            bear_id: run.bear_id,
            task_id: run.task_id,
            root_name: run.root_name.clone(),
            git_ref: run.git_ref.clone(),
            requested_by_user_id: Some(user_id),
        },
    )
    .await?;
    Ok(Redirect::to(&format!("/work/runs/{}", retry.id)).into_response())
}
