use axum::{extract::State, response::Response, routing::get, Router};
use minijinja::context;
use serde::Serialize;

use crate::auth_backend::AuthSession;
use crate::errors::CustomError;
use crate::web::{self, AppState};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(index))
}

#[derive(Debug, Serialize)]
struct WorkerHealthRow {
    worker: String,
    total: i64,
    pending: i64,
    running: i64,
    succeeded: i64,
    failed: i64,
    blocked: i64,
    most_recent_at: Option<String>,
}

pub async fn index(
    State(state): State<AppState>,
    auth_session: AuthSession,
) -> Result<Response, CustomError> {
    let mut rows = Vec::new();
    rows.push(reflection_runs_row(state.sqlx_pool()).await?);
    rows.push(compaction_events_row(state.sqlx_pool()).await?);
    rows.push(work_runs_row(state.sqlx_pool()).await?);

    web::render_template(
        &state,
        "admin/workers/index.html",
        auth_session,
        context! {
            rows,
            native_runtime => true,
        },
    )
    .await
}

async fn reflection_runs_row(pool: &sqlx::PgPool) -> Result<WorkerHealthRow, CustomError> {
    let row = sqlx::query!(
        r"
        SELECT
            count(*) AS total,
            0::BIGINT AS pending,
            count(*) FILTER (WHERE status = 'started') AS running,
            count(*) FILTER (WHERE status = 'completed') AS succeeded,
            count(*) FILTER (WHERE status = 'failed') AS failed,
            count(*) FILTER (WHERE status = 'skipped') AS blocked,
            max(coalesce(completed_at, created_at)) AS most_recent_at
        FROM pair_reflection_runs
        ",
    )
    .fetch_one(pool)
    .await?;

    Ok(WorkerHealthRow {
        worker: "Reflection runs".to_string(),
        total: row.total.unwrap_or_default(),
        pending: row.pending.unwrap_or_default(),
        running: row.running.unwrap_or_default(),
        succeeded: row.succeeded.unwrap_or_default(),
        failed: row.failed.unwrap_or_default(),
        blocked: row.blocked.unwrap_or_default(),
        most_recent_at: row.most_recent_at.map(|value| value.to_string()),
    })
}

async fn compaction_events_row(pool: &sqlx::PgPool) -> Result<WorkerHealthRow, CustomError> {
    let row = sqlx::query!(
        r"
        SELECT
            count(*) AS total,
            0::BIGINT AS pending,
            0::BIGINT AS running,
            count(*) FILTER (WHERE status = 'Applied') AS succeeded,
            count(*) FILTER (WHERE status = 'Failed') AS failed,
            count(*) FILTER (WHERE status = 'Skipped') AS blocked,
            max(created_at) AS most_recent_at
        FROM runtime_compaction_events
        ",
    )
    .fetch_one(pool)
    .await?;

    Ok(WorkerHealthRow {
        worker: "Compaction events".to_string(),
        total: row.total.unwrap_or_default(),
        pending: row.pending.unwrap_or_default(),
        running: row.running.unwrap_or_default(),
        succeeded: row.succeeded.unwrap_or_default(),
        failed: row.failed.unwrap_or_default(),
        blocked: row.blocked.unwrap_or_default(),
        most_recent_at: row.most_recent_at.map(|value| value.to_string()),
    })
}

async fn work_runs_row(pool: &sqlx::PgPool) -> Result<WorkerHealthRow, CustomError> {
    let row = sqlx::query!(
        r"
        SELECT
            count(*) AS total,
            count(*) FILTER (WHERE state = 'queued') AS pending,
            count(*) FILTER (WHERE state IN ('claimed', 'provisioning', 'running', 'reporting')) AS running,
            count(*) FILTER (WHERE state = 'succeeded') AS succeeded,
            count(*) FILTER (WHERE state IN ('failed', 'timed_out')) AS failed,
            count(*) FILTER (WHERE state IN ('blocked', 'cancelled')) AS blocked,
            max(updated_at) AS most_recent_at
        FROM bear_work_runs
        ",
    )
    .fetch_one(pool)
    .await?;

    Ok(WorkerHealthRow {
        worker: "Work runs".to_string(),
        total: row.total.unwrap_or_default(),
        pending: row.pending.unwrap_or_default(),
        running: row.running.unwrap_or_default(),
        succeeded: row.succeeded.unwrap_or_default(),
        failed: row.failed.unwrap_or_default(),
        blocked: row.blocked.unwrap_or_default(),
        most_recent_at: row.most_recent_at.map(|value| value.to_string()),
    })
}
