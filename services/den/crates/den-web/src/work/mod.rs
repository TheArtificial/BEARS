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
use axum_extra::extract::Form;
use minijinja::context;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth_backend::AuthSession,
    errors::CustomError,
    web::{self, AppState},
};
use den_docket::work_runs::{self, WorkRunListFilter, WorkRunRow, SCRATCH_ROOT_NAME};
use den_docket::{
    DocketCommitPolicy, DocketEffortHint, DocketJobCreate, DocketJobCriterionInput,
    DocketJobListFilter, DocketJobStatus, DocketService, DocketTaskDifficulty, DocketTaskInput,
    DocketTaskKind, DocketTaskScope, PgDocketService, TaskListVisibility,
};
use den_sandbox::protocol::CatalogResponse;
use den_sandbox::SandboxClient;
use den_service::bears::db as bears_db;
use den_service::bears::BearProfile;

pub mod surfaces;

#[cfg(test)]
mod tests;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/work", get(index))
        .route("/work/new", get(new_job_form).post(create_job))
        .route("/work/jobs/{job_id}", get(job_detail))
        .route("/work/jobs/{job_id}/duplicate", post(duplicate_job))
        .route("/work/runs/{run_id}", get(run_detail))
        .route("/work/tasks/{task_id}/dispatch", post(dispatch_task))
        .route("/work/runs/{run_id}/cancel", post(cancel_run))
        .route("/work/runs/{run_id}/retry", post(retry_run))
        .merge(surfaces::router())
}

/// Best-effort fetch of the sandbox provider's root/image catalog for form
/// selects. `None` (provider unconfigured or down) degrades the forms to free
/// text inputs rather than failing the page.
async fn provider_catalog(state: &AppState) -> Option<CatalogResponse> {
    let url = state.config.sandbox_server_url.as_deref()?.trim();
    if url.is_empty() {
        return None;
    }
    let client = SandboxClient::new(url, &state.config.sandbox_server_token);
    match client.catalog().await {
        Ok(catalog) => Some(catalog),
        Err(err) => {
            tracing::warn!(error = %err, "work UI: sandbox catalog fetch failed");
            None
        }
    }
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
    git_ref: Option<String>,
    image: Option<String>,
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
    /// Publish outcome from result_refs: branch/commit pushed upstream.
    published_branch: Option<String>,
    published_commit: Option<String>,
    publish_failed: Option<String>,
    /// For queued runs: 1-based position in the job's queue (runs serialize
    /// per job) and the in-flight run this one waits behind.
    queue_position: Option<i64>,
    waiting_on_run_id: Option<String>,
}

fn ts(value: time::OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn run_view(run: &WorkRunRow, bear_slug: &str) -> RunView {
    let is_active = run.state_enum().is_some_and(|state| !state.is_terminal());
    let duration_secs = run.started_at.map(|started| {
        let end = run
            .finished_at
            .unwrap_or_else(time::OffsetDateTime::now_utc);
        (end - started).whole_seconds()
    });
    let cleanup_failed = run
        .result_refs
        .as_ref()
        .and_then(|refs| refs.get("cleanup"))
        .and_then(serde_json::Value::as_str)
        == Some("failed");
    let ref_str = |pointer: &str| {
        run.result_refs
            .as_ref()
            .and_then(|refs| refs.pointer(pointer))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    };
    RunView {
        id: run.id.to_string(),
        state: run.state.clone(),
        is_active,
        attempt: run.attempt,
        bear_slug: bear_slug.to_string(),
        job_id: run.job_id.to_string(),
        task_id: run.task_id.to_string(),
        root: run.root_name.clone(),
        git_ref: run.git_ref.clone(),
        image: run.image_name.clone(),
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
        published_branch: ref_str("/published/branch"),
        published_commit: ref_str("/published/commit"),
        publish_failed: ref_str("/publish_failed"),
        queue_position: None,
        waiting_on_run_id: None,
    }
}

/// Annotate queued runs with their queue placement (one batch query).
async fn attach_queue_info(
    state: &AppState,
    runs: &[WorkRunRow],
    views: &mut [RunView],
) -> Result<(), CustomError> {
    let queued_ids: Vec<Uuid> = runs
        .iter()
        .filter(|run| run.state == "queued")
        .map(|run| run.id)
        .collect();
    if queued_ids.is_empty() {
        return Ok(());
    }
    let infos = work_runs::queued_run_positions(state.sqlx_pool(), &queued_ids).await?;
    let by_id: std::collections::HashMap<String, &work_runs::WorkRunQueueInfo> = infos
        .iter()
        .map(|info| (info.run_id.to_string(), info))
        .collect();
    for view in views.iter_mut() {
        if let Some(info) = by_id.get(&view.id) {
            view.queue_position = Some(info.position);
            view.waiting_on_run_id = info.waiting_on_run_id.map(|id| id.to_string());
        }
    }
    Ok(())
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
    let mut run_rows: Vec<WorkRunRow> = Vec::new();
    let mut jobs_with_work: Vec<serde_json::Value> = Vec::new();
    let mut attention: Vec<serde_json::Value> = Vec::new();
    let mut awaiting_completion: Vec<serde_json::Value> = Vec::new();
    for (bear_id, bear_slug) in &bears {
        for run in work_runs::attention_work_runs(state.sqlx_pool(), *bear_id, None, 20).await? {
            attention.push(serde_json::json!({
                "run_id": run.run_id.to_string(),
                "job_id": run.job_id.to_string(),
                "bear_slug": bear_slug,
                "task_title": run.task_title,
                "job_goal": run.job_goal,
                "state": run.state,
                "reason": run.result_summary.or(run.error),
            }));
        }
        for job in work_runs::jobs_awaiting_completion(state.sqlx_pool(), *bear_id).await? {
            awaiting_completion.push(serde_json::json!({
                "id": job.id.to_string(),
                "bear_slug": bear_slug,
                "goal": job.goal,
                "status": job.status,
            }));
        }
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
        run_rows.extend(bear_runs);

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
    attach_queue_info(&state, &run_rows, &mut runs).await?;
    runs.sort_by(|a, b| b.queued_at.cmp(&a.queued_at));
    let active: Vec<&RunView> = runs.iter().filter(|run| run.is_active).collect();

    // Dispatch-path status so "why is my queued run not starting?" is
    // answerable from this page: is a provider configured, and is it
    // reachable? (The dispatch worker itself runs in the workers process;
    // its liveness is visible in that process's logs.)
    let sandbox_server_url = state
        .config
        .sandbox_server_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_string);
    let provider_status = match &sandbox_server_url {
        None => serde_json::json!({ "configured": false }),
        Some(url) => {
            let client = SandboxClient::new(url, &state.config.sandbox_server_token);
            match client.health().await {
                Ok(health) => serde_json::json!({
                    "configured": true,
                    "url": url,
                    "reachable": true,
                    "backend_available": health.backend_available,
                    "active_sandboxes": health.active_sandboxes,
                }),
                Err(err) => serde_json::json!({
                    "configured": true,
                    "url": url,
                    "reachable": false,
                    "error": err.to_string(),
                }),
            }
        }
    };

    web::render_template(
        &state,
        "work/index.html",
        auth_session,
        context! {
            title => "Work",
            active_runs => active,
            runs => runs,
            jobs => jobs_with_work,
            attention => attention,
            awaiting_completion => awaiting_completion,
            provider_status => provider_status,
        },
    )
    .await
}

async fn new_job_form(
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let user_id = require_user(&auth_session)?;
    let bears = member_bears(&state, user_id).await?;
    let mut bear_slugs: Vec<(String, String)> = bears
        .iter()
        .map(|(id, slug)| (id.to_string(), slug.clone()))
        .collect();
    bear_slugs.sort_by(|a, b| a.1.cmp(&b.1));
    let catalog = provider_catalog(&state).await;

    // Managed surfaces available to any of the user's bears; the create
    // handler re-checks the selected bear's assignment server-side.
    let bear_ids: Vec<Uuid> = bears.keys().copied().collect();
    let mut surfaces: Vec<serde_json::Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for surface in
        den_service::work_surfaces::list_surfaces_for_bears(state.sqlx_pool(), &bear_ids).await?
    {
        if seen.insert(surface.id) {
            surfaces.push(serde_json::json!({
                "id": surface.id.to_string(),
                "name": surface.name,
                "default_ref": surface.default_ref,
            }));
        }
    }

    web::render_template(
        &state,
        "work/new.html",
        auth_session,
        context! {
            title => "New work job",
            bears => bear_slugs,
            catalog => catalog,
            surfaces => surfaces,
        },
    )
    .await
}

/// One task row per repeated `task_title[]` input; criteria are
/// semicolon-separated within the row. Blank rows are skipped.
#[derive(Debug, Deserialize)]
struct NewJobForm {
    bear_id: Uuid,
    goal: String,
    /// Managed work surface id (preferred; from the surface select).
    #[serde(default)]
    surface_id: String,
    /// Legacy free-text provider root name.
    #[serde(default)]
    root: String,
    #[serde(default)]
    commit_policy: String,
    #[serde(default)]
    work_branch: String,
    #[serde(default)]
    task_title: Vec<String>,
    #[serde(default)]
    task_criteria: Vec<String>,
}

async fn create_job(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Form(form): Form<NewJobForm>,
) -> Result<Response, CustomError> {
    let user_id = require_user(&auth_session)?;
    let bears = member_bears(&state, user_id).await?;
    if !bears.contains_key(&form.bear_id) {
        return Err(CustomError::NotFound("bear not found".to_string()));
    }

    let commit_policy = match form.commit_policy.trim() {
        "" => None,
        raw => Some(
            serde_json::from_value::<DocketCommitPolicy>(serde_json::json!(raw)).map_err(|_| {
                CustomError::ValidationError(format!("invalid commit policy '{raw}'"))
            })?,
        ),
    };

    let mut tasks = Vec::new();
    for (index, title) in form.task_title.iter().enumerate() {
        let title = title.trim();
        if title.is_empty() {
            continue;
        }
        let criteria: Vec<String> = form
            .task_criteria
            .get(index)
            .map(|raw| {
                raw.split(';')
                    .map(str::trim)
                    .filter(|criterion| !criterion.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if criteria.is_empty() {
            return Err(CustomError::ValidationError(format!(
                "task '{title}' needs at least one completion criterion (semicolon-separated)"
            )));
        }
        tasks.push(DocketTaskInput {
            client_key: Some(format!("ui-task-{index}")),
            parent_client_key: None,
            parent_task_id: None,
            sibling_order: i32::try_from(index).unwrap_or(0),
            kind: DocketTaskKind::Execution,
            scope: DocketTaskScope::Template,
            title: title.to_string(),
            // Docket requires a non-empty body; the quick-create form only
            // collects titles, so the title doubles as the body.
            body: title.to_string(),
            completion_criteria: criteria,
            difficulty: None,
            effort_hint: None,
            assigned_to_role: Some(BearProfile::Work),
        });
    }
    if tasks.is_empty() {
        return Err(CustomError::ValidationError(
            "a work job needs at least one task".to_string(),
        ));
    }

    // Managed surface select wins over the legacy free-text root. Either
    // way, a name matching a managed surface requires the bear's assignment.
    let (work_surface_ref, work_surface_id) = if let Ok(surface_id) =
        form.surface_id.trim().parse::<Uuid>()
    {
        let surface = den_service::work_surfaces::surface_by_id(state.sqlx_pool(), surface_id)
            .await?
            .ok_or_else(|| CustomError::NotFound("work surface not found".to_string()))?;
        if !den_service::work_surfaces::bear_may_use_surface(
            state.sqlx_pool(),
            form.bear_id,
            surface.id,
        )
        .await?
        {
            return Err(CustomError::ValidationError(format!(
                "bear is not assigned to work surface '{}'",
                surface.name
            )));
        }
        (Some(surface.name), Some(surface.id))
    } else if let Some(root) = Some(form.root.trim().to_string()).filter(|root| !root.is_empty()) {
        match den_service::work_surfaces::surface_by_name(state.sqlx_pool(), &root).await? {
            Some(surface) => {
                if !den_service::work_surfaces::bear_may_use_surface(
                    state.sqlx_pool(),
                    form.bear_id,
                    surface.id,
                )
                .await?
                {
                    return Err(CustomError::ValidationError(format!(
                        "bear is not assigned to work surface '{}'",
                        surface.name
                    )));
                }
                (Some(surface.name), Some(surface.id))
            }
            None => (Some(root), None),
        }
    } else {
        (None, None)
    };

    let job = PgDocketService::from_pool(state.sqlx_pool())
        .create_job(DocketJobCreate {
            bear_id: form.bear_id,
            created_by_user_id: user_id,
            created_by_role: "ui".to_string(),
            goal: form.goal,
            work_surface_ref,
            work_surface_id,
            commit_policy,
            work_branch: Some(form.work_branch.trim().to_string())
                .filter(|branch| !branch.is_empty()),
            status: DocketJobStatus::Ready,
            visibility: TaskListVisibility::SameUser,
            source_conversation_id: None,
            objective_kind: None,
            criteria: vec![DocketJobCriterionInput {
                kind: den_docket::DocketCriterionKind::Narrative,
                description: "All tasks completed to their criteria".to_string(),
                spec: None,
                sibling_order: 0,
            }],
            tasks,
        })
        .await?;
    Ok(Redirect::to(&format!("/work/jobs/{}", job.job.id)).into_response())
}

fn parse_docket_enum<T: serde::de::DeserializeOwned>(
    field: &str,
    value: &str,
) -> Result<T, CustomError> {
    serde_json::from_value(serde_json::json!(value)).map_err(|_| {
        CustomError::ValidationError(format!("job contains invalid {field} value '{value}'"))
    })
}

async fn duplicate_job(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(job_id): Path<Uuid>,
) -> Result<Response, CustomError> {
    let user_id = require_user(&auth_session)?;
    let bears = member_bears(&state, user_id).await?;
    let owner: Option<(Uuid,)> = sqlx::query_as("SELECT bear_id FROM bear_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(state.sqlx_pool())
        .await
        .map_err(den_core::DenError::from)?;
    let Some((bear_id,)) = owner.filter(|(bear_id,)| bears.contains_key(bear_id)) else {
        return Err(CustomError::NotFound("job not found".to_string()));
    };

    let source = PgDocketService::from_pool(state.sqlx_pool())
        .get_job(bear_id, job_id)
        .await?
        .ok_or_else(|| CustomError::NotFound("job not found".to_string()))?;
    let task_keys: std::collections::HashMap<Uuid, String> = source
        .tasks
        .iter()
        .map(|task| (task.id, format!("duplicate-{}", task.id.simple())))
        .collect();
    let tasks = source
        .tasks
        .iter()
        .map(|task| {
            Ok(DocketTaskInput {
                client_key: task_keys.get(&task.id).cloned(),
                parent_client_key: task
                    .parent_task_id
                    .and_then(|parent_id| task_keys.get(&parent_id).cloned()),
                parent_task_id: None,
                sibling_order: task.sibling_order,
                kind: parse_docket_enum("task kind", &task.kind)?,
                scope: parse_docket_enum("task scope", &task.scope)?,
                title: task.title.clone(),
                body: task.body.clone(),
                completion_criteria: task.completion_criteria.0.clone(),
                difficulty: task
                    .difficulty
                    .as_deref()
                    .map(|value| {
                        parse_docket_enum::<DocketTaskDifficulty>("task difficulty", value)
                    })
                    .transpose()?,
                effort_hint: task
                    .effort_hint
                    .as_deref()
                    .map(|value| parse_docket_enum::<DocketEffortHint>("task effort", value))
                    .transpose()?,
                assigned_to_role: task
                    .assigned_to_role
                    .as_deref()
                    .map(|value| parse_docket_enum::<BearProfile>("assigned role", value))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>, CustomError>>()?;
    let criteria = source
        .criteria
        .iter()
        .map(|criterion| {
            Ok(DocketJobCriterionInput {
                kind: parse_docket_enum("criterion kind", &criterion.kind)?,
                description: criterion.description.clone(),
                spec: criterion.spec.as_ref().map(|spec| spec.0.clone()),
                sibling_order: criterion.sibling_order,
            })
        })
        .collect::<Result<Vec<_>, CustomError>>()?;
    let commit_policy = source
        .job
        .commit_policy
        .as_deref()
        .map(|value| parse_docket_enum::<DocketCommitPolicy>("commit policy", value))
        .transpose()?;
    let visibility = parse_docket_enum::<TaskListVisibility>("visibility", &source.job.visibility)?;

    let duplicate = PgDocketService::from_pool(state.sqlx_pool())
        .create_job(DocketJobCreate {
            bear_id,
            created_by_user_id: user_id,
            created_by_role: "ui".to_string(),
            goal: format!("{} (copy)", source.job.goal),
            work_surface_ref: source.job.work_surface_ref,
            work_surface_id: source.job.work_surface_id,
            commit_policy,
            work_branch: None,
            status: DocketJobStatus::Ready,
            visibility,
            source_conversation_id: None,
            objective_kind: source.job.objective_kind,
            criteria,
            tasks,
        })
        .await?;
    Ok(Redirect::to(&format!("/work/jobs/{}", duplicate.job.id)).into_response())
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
        let row: Option<(Uuid,)> = sqlx::query_as("SELECT bear_id FROM bear_jobs WHERE id = $1")
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
    let mut run_views: Vec<RunView> = runs.iter().map(|run| run_view(run, &bear_slug)).collect();
    attach_queue_info(&state, &runs, &mut run_views).await?;

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

    let status_report = den_docket::docket_job_status_report(&projection);
    let catalog = provider_catalog(&state).await;
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
            commit_policy => projection.job.commit_policy,
            work_branch => projection.job.work_branch,
            tasks => tasks,
            runs => run_views,
            catalog => catalog,
            tasks_complete => status_report.tasks_complete,
            criteria_complete => status_report.criteria_complete,
            next_action => status_report.next_action,
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
    let bear_slug = bears.get(&run.bear_id).cloned().unwrap_or_default();

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
    let conversation_id: Option<String> = match run.bearwire_session_id.as_deref() {
        Some(session_id) => sqlx::query_scalar(
            "SELECT COALESCE(NULLIF(resolved_conversation_id, ''), conversation_id) \
             FROM client_sessions WHERE client_session_id = $1 AND bear_id = $2 \
             ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(session_id)
        .bind(run.bear_id)
        .fetch_optional(state.sqlx_pool())
        .await
        .map_err(den_core::DenError::from)?,
        None => None,
    };
    let work_surface = run.work_surface.clone();
    let usage = run.usage.clone();
    let mut views = vec![run_view(&run, &bear_slug)];
    attach_queue_info(&state, std::slice::from_ref(&run), &mut views).await?;
    let view = views.remove(0);
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
            conversation_id => conversation_id,
            work_surface => work_surface,
            usage => usage,
            can_retry => can_retry,
        },
    )
    .await
}

#[derive(Debug, Default, Deserialize)]
struct DispatchForm {
    #[serde(default)]
    root: String,
    #[serde(default)]
    scratch: bool,
    #[serde(default)]
    image: String,
    #[serde(default)]
    git_ref: String,
}

fn clean_form_field(raw: &str) -> Option<String> {
    Some(raw.trim().to_string()).filter(|value| !value.is_empty())
}

async fn dispatch_task(
    State(state): State<AppState>,
    auth_session: AuthSession,
    Path(task_id): Path<Uuid>,
    Form(form): Form<DispatchForm>,
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
    let root_name = if form.scratch {
        Some(SCRATCH_ROOT_NAME.to_string())
    } else {
        clean_form_field(&form.root)
    };
    work_runs::enqueue_work_run(
        state.sqlx_pool(),
        work_runs::WorkRunEnqueue {
            bear_id,
            task_id,
            root_name,
            git_ref: clean_form_field(&form.git_ref),
            image_name: clean_form_field(&form.image),
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
            image_name: run.image_name.clone(),
            requested_by_user_id: Some(user_id),
        },
    )
    .await?;
    Ok(Redirect::to(&format!("/work/runs/{}", retry.id)).into_response())
}
